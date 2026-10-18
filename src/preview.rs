//! One serial worker, bounded latest-request queue, and revision-gated results.
use crate::backend::{Backend, FormatRequest};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct PreviewResult {
    pub revision: u64,
    pub result: Result<String, String>,
}

struct Pending {
    revision: u64,
    request: FormatRequest,
    cancelled: Arc<AtomicBool>,
    due: Instant,
}

#[derive(Default)]
struct State {
    pending: Option<Pending>,
    stopped: bool,
}

pub struct PreviewController {
    state: Arc<(Mutex<State>, Condvar)>,
    receiver: mpsc::Receiver<PreviewResult>,
    worker: Option<JoinHandle<()>>,
    cancellation: Option<Arc<AtomicBool>>,
    revision: u64,
}

impl PreviewController {
    pub fn new(backend: Backend) -> Self {
        Self::with_formatter(move |request, cancelled| backend.format(request, cancelled))
    }

    fn with_formatter<F>(format: F) -> Self
    where
        F: Fn(&FormatRequest, Arc<AtomicBool>) -> Result<String, String> + Send + 'static,
    {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker_state = state.clone();
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            loop {
                let (lock, wake) = &*worker_state;
                let mut state = lock.lock().unwrap();
                let pending = loop {
                    if state.stopped {
                        return;
                    }
                    if let Some(pending) = &state.pending {
                        let now = Instant::now();
                        if now >= pending.due {
                            break state.pending.take().unwrap();
                        }
                        let wait = pending.due - now;
                        state = wake.wait_timeout(state, wait).unwrap().0;
                    } else {
                        state = wake.wait(state).unwrap();
                    }
                };
                drop(state);
                let result = format(&pending.request, pending.cancelled.clone());
                if !pending.cancelled.load(Ordering::Acquire) {
                    let _ = sender.send(PreviewResult {
                        revision: pending.revision,
                        result,
                    });
                }
            }
        });
        Self {
            state,
            receiver,
            worker: Some(worker),
            cancellation: None,
            revision: 0,
        }
    }

    /// Invalidate queued and running work when switching documents or revoking trust.
    pub fn cancel(&mut self) -> u64 {
        if let Some(cancelled) = self.cancellation.take() {
            cancelled.store(true, Ordering::Release);
        }
        self.revision += 1;
        let (lock, wake) = &*self.state;
        lock.lock().unwrap().pending = None;
        wake.notify_one();
        self.revision
    }

    pub fn submit(&mut self, request: FormatRequest) -> u64 {
        self.cancel();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancellation = Some(cancelled.clone());
        let (lock, wake) = &*self.state;
        lock.lock().unwrap().pending = Some(Pending {
            revision: self.revision,
            request,
            cancelled,
            due: Instant::now() + Duration::from_millis(150),
        });
        wake.notify_one();
        self.revision
    }

    pub fn poll(&mut self) -> Option<PreviewResult> {
        self.receiver
            .try_iter()
            .filter(|result| result.revision == self.revision)
            .last()
    }
}

impl Drop for PreviewController {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.cancellation {
            cancelled.store(true, Ordering::Release);
        }
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.stopped = true;
        state.pending = None;
        drop(state);
        wake.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(source: &str) -> FormatRequest {
        FormatRequest {
            source: source.into(),
            source_path: "sample.ts".into(),
            config: crate::config::ConfigDocument::standalone(&[])
                .unwrap()
                .snapshot(),
        }
    }
    #[test]
    fn debounce_coalesces_and_returns_only_latest() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let mut preview = PreviewController::with_formatter(move |req, _| {
            captured.lock().unwrap().push(req.source.clone());
            Ok(req.source.clone())
        });
        preview.submit(request("old"));
        let revision = preview.submit(request("new"));
        let deadline = Instant::now() + Duration::from_secs(3);
        let result = loop {
            if let Some(result) = preview.poll() {
                break result;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(result.revision, revision);
        assert_eq!(result.result.unwrap(), "new");
        assert_eq!(*calls.lock().unwrap(), vec!["new"]);
    }
    #[test]
    fn newer_revision_cancels_running_work() {
        let (started_tx, started_rx) = mpsc::channel();
        let mut preview = PreviewController::with_formatter(move |req, cancel| {
            if req.source == "old" {
                started_tx.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            Ok(req.source.clone())
        });
        preview.submit(request("old"));
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        preview.submit(request("new"));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = preview.poll() {
                assert_eq!(result.result.unwrap(), "new");
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn rapid_revisions_only_publish_latest_snapshot() {
        let mut preview =
            PreviewController::with_formatter(|req, _| Ok(req.config.source_text().to_owned()));
        let mut document = crate::config::ConfigDocument::standalone(&[]).unwrap();
        for width in 1..=16 {
            document
                .set(&["indentWidth"], serde_json::json!(width))
                .unwrap();
            let mut input = request("same original source");
            input.config = document.snapshot();
            preview.submit(input);
        }
        let expected = document.draft_text().to_owned();
        document
            .set(&["indentWidth"], serde_json::json!(2))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = preview.poll() {
                assert_eq!(result.revision, 16);
                assert_eq!(result.result.unwrap(), expected);
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn closing_cancels_and_joins_running_work() {
        let (started_tx, started_rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let mut preview = PreviewController::with_formatter(move |_, cancelled| {
            started_tx.send(()).unwrap();
            while !cancelled.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            worker_stopped.store(true, Ordering::Release);
            Err("cancelled".into())
        });
        preview.submit(request("original"));
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(preview);
        assert!(stopped.load(Ordering::Acquire));
    }

    #[test]
    fn cancel_invalidates_already_completed_results_and_queued_jobs() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let captured = calls.clone();
        let (finished_tx, finished_rx) = mpsc::channel();
        let mut preview = PreviewController::with_formatter(move |_, _| {
            captured.fetch_add(1, Ordering::Relaxed);
            finished_tx.send(()).unwrap();
            Ok("old output".into())
        });
        preview.submit(request("old"));
        finished_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        preview.cancel();
        assert!(preview.poll().is_none());
        preview.submit(request("queued"));
        preview.cancel();
        thread::sleep(Duration::from_millis(200));
        assert!(preview.poll().is_none());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
