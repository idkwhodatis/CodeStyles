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

    pub fn submit(&mut self, request: FormatRequest) -> u64 {
        if let Some(cancelled) = self.cancellation.take() {
            cancelled.store(true, Ordering::Release);
        }
        self.revision += 1;
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
            config_path: None,
            indent_width: 2,
            semi_colons: "always".into(),
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
}
