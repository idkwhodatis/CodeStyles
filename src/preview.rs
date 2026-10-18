//! One serial worker, bounded latest-request queue, and revision-gated results.
use crate::{
    backend::{Backend, FormatRequest},
    config::ConfigSnapshot,
};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct PreviewResult {
    pub revision: u64,
    /// Draft output. Kept unchanged for callers of the original submit API.
    pub result: Result<String, String>,
    /// Saved-configuration output from the same original source and path.
    pub baseline_result: Option<Result<String, String>>,
}

struct Pending {
    revision: u64,
    request: FormatRequest,
    baseline: Option<ConfigSnapshot>,
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
                // Do not start the second process after cancelled draft work.
                if pending.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                let baseline_result = pending.baseline.map(|config| {
                    // Reuse the exact original source/path. Formatted draft
                    // output must never become input to the comparison side.
                    let request = FormatRequest {
                        config,
                        ..pending.request
                    };
                    format(&request, pending.cancelled.clone())
                });
                if !pending.cancelled.load(Ordering::Acquire) {
                    let _ = sender.send(PreviewResult {
                        revision: pending.revision,
                        result,
                        baseline_result,
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
        self.submit_inner(request, None)
    }

    /// Format saved and draft snapshots as one cancellable revision. Both
    /// sides use the source/path captured by `request`, on the single worker.
    pub fn submit_comparison(&mut self, request: FormatRequest, baseline: ConfigSnapshot) -> u64 {
        self.submit_inner(request, Some(baseline))
    }

    fn submit_inner(&mut self, request: FormatRequest, baseline: Option<ConfigSnapshot>) -> u64 {
        self.cancel();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancellation = Some(cancelled.clone());
        let (lock, wake) = &*self.state;
        lock.lock().unwrap().pending = Some(Pending {
            revision: self.revision,
            request,
            baseline,
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
    fn wait_result(preview: &mut PreviewController) -> PreviewResult {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = preview.poll() {
                return result;
            }
            assert!(Instant::now() < deadline, "preview did not finish");
            thread::sleep(Duration::from_millis(10));
        }
    }

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
        assert!(result.baseline_result.is_none());
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

    #[test]
    fn comparison_uses_immutable_saved_and_draft_snapshots_with_identical_original_input() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dprint.jsonc");
        std::fs::write(&path, "{\"plugins\": [], \"indentWidth\": 2}\n").unwrap();
        let mut document = crate::config::ConfigDocument::load(&path).unwrap();
        let baseline = document.saved_snapshot();
        document
            .set(&["indentWidth"], serde_json::json!(4))
            .unwrap();
        let draft = document.snapshot();
        let source_path = directory.path().join("original.ts");
        let calls = Arc::new(Mutex::new(Vec::<FormatRequest>::new()));
        let captured = calls.clone();
        let mut preview = PreviewController::with_formatter(move |request, _| {
            captured.lock().unwrap().push(request.clone());
            Ok(format!("formatted with {}", request.config.source_text()))
        });
        let revision = preview.submit_comparison(
            FormatRequest {
                source: "const original={untouched:true};\n".into(),
                source_path: source_path.clone(),
                config: draft.clone(),
            },
            baseline.clone(),
        );
        document
            .set(&["indentWidth"], serde_json::json!(8))
            .unwrap();
        // Neither an edit during debounce nor a newer disk file changes the job.
        std::fs::write(&path, "{\"plugins\": [], \"indentWidth\": 16}\n").unwrap();
        let result = wait_result(&mut preview);
        assert_eq!(result.revision, revision);
        assert_eq!(
            result.result.unwrap(),
            format!("formatted with {}", draft.source_text())
        );
        assert_eq!(
            result.baseline_result.unwrap().unwrap(),
            format!("formatted with {}", baseline.source_text())
        );
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].config, draft);
        assert_eq!(calls[1].config, baseline);
        for call in calls.iter() {
            assert_eq!(call.source, "const original={untouched:true};\n");
            assert_eq!(call.source_path, source_path);
            assert_eq!(call.config.original_path(), document.path());
        }
    }

    #[test]
    fn saved_snapshot_advances_only_after_successful_save_and_keeps_old_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dprint.jsonc");
        let mut document = crate::config::ConfigDocument::standalone(&[]).unwrap();
        let initial = document.saved_snapshot();
        document
            .set(&["indentWidth"], serde_json::json!(4))
            .unwrap();
        assert_eq!(document.saved_snapshot(), initial);
        let first_draft = document.draft_text().to_owned();
        document.save_as(&path).unwrap();
        let saved = document.saved_snapshot();
        assert_eq!(saved.source_text(), first_draft);
        assert_eq!(saved.original_path(), document.path());
        assert_eq!(initial.original_path(), None);
        document
            .set(&["indentWidth"], serde_json::json!(8))
            .unwrap();
        assert_eq!(document.saved_snapshot(), saved);
        document.save().unwrap();
        let updated = document.saved_snapshot();
        assert_eq!(updated, document.snapshot());
        assert_ne!(updated, saved);
        assert_eq!(saved.source_text(), first_draft);
        document
            .set(&["indentWidth"], serde_json::json!(2))
            .unwrap();
        std::fs::write(&path, "{\"indentWidth\":16}\n").unwrap();
        assert!(document.save().is_err());
        assert_eq!(document.saved_snapshot(), updated);
    }

    #[test]
    fn comparison_keeps_each_side_error_independent() {
        for fail_draft in [false, true] {
            let document = crate::config::ConfigDocument::standalone(&[]).unwrap();
            let baseline = document.saved_snapshot();
            let mut draft = document.clone();
            draft.set(&["indentWidth"], serde_json::json!(4)).unwrap();
            let mut preview = PreviewController::with_formatter(move |request, _| {
                let is_draft = request.config.source_text().contains("indentWidth");
                if is_draft == fail_draft {
                    Err("side-specific failure".into())
                } else {
                    Ok("successful side".into())
                }
            });
            let mut input = request("original");
            input.config = draft.snapshot();
            preview.submit_comparison(input, baseline);
            let result = wait_result(&mut preview);
            assert_eq!(result.result.is_err(), fail_draft);
            assert_eq!(result.baseline_result.unwrap().is_err(), !fail_draft);
        }
    }

    #[test]
    fn cancelled_comparison_draft_never_starts_the_old_baseline() {
        let (started_tx, started_rx) = mpsc::channel();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let mut preview = PreviewController::with_formatter(move |request, cancelled| {
            captured.lock().unwrap().push(request.source.clone());
            if request.source == "old original" {
                started_tx.send(()).unwrap();
                while !cancelled.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            Ok(request.source.clone())
        });
        let baseline = crate::config::ConfigDocument::standalone(&[])
            .unwrap()
            .saved_snapshot();
        preview.submit_comparison(request("old original"), baseline.clone());
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let revision = preview.submit_comparison(request("new original"), baseline);
        let result = wait_result(&mut preview);
        assert_eq!(result.revision, revision);
        assert_eq!(result.result.unwrap(), "new original");
        assert_eq!(result.baseline_result.unwrap().unwrap(), "new original");
        assert_eq!(
            *calls.lock().unwrap(),
            ["old original", "new original", "new original"]
        );
    }

    #[test]
    fn newer_revision_cancels_baseline_and_drops_the_entire_old_pair() {
        let mut document = crate::config::ConfigDocument::standalone(&[]).unwrap();
        let baseline = document.saved_snapshot();
        document
            .set(&["indentWidth"], serde_json::json!(4))
            .unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let mut preview = PreviewController::with_formatter(move |request, cancelled| {
            if request.source == "old original"
                && !request.config.source_text().contains("indentWidth")
            {
                started_tx.send(()).unwrap();
                while !cancelled.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            Ok(request.source.clone())
        });
        let mut input = request("old original");
        input.config = document.snapshot();
        preview.submit_comparison(input, baseline);
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            preview.poll().is_none(),
            "draft must not publish an incomplete pair"
        );
        let revision = preview.submit(request("new original"));
        let result = wait_result(&mut preview);
        assert_eq!(result.revision, revision);
        assert_eq!(result.result.unwrap(), "new original");
        assert!(result.baseline_result.is_none());
    }

    #[test]
    fn closing_during_comparison_baseline_cancels_and_joins_worker() {
        let mut document = crate::config::ConfigDocument::standalone(&[]).unwrap();
        let baseline = document.saved_snapshot();
        document
            .set(&["indentWidth"], serde_json::json!(4))
            .unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let mut preview = PreviewController::with_formatter(move |request, cancelled| {
            if !request.config.source_text().contains("indentWidth") {
                started_tx.send(()).unwrap();
                while !cancelled.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                worker_stopped.store(true, Ordering::Release);
            }
            Ok("output".into())
        });
        let mut input = request("original");
        input.config = document.snapshot();
        preview.submit_comparison(input, baseline);
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(preview);
        assert!(stopped.load(Ordering::Acquire));
    }
}
