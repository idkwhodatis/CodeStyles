//! A single cancellable project worker. Writes are never silently replaced by
//! a newer request: another submit is rejected until the running job is reaped.
use crate::{
    backend::Backend,
    project::{ProjectReport, ProjectRequest},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

#[derive(Debug)]
pub struct ProjectResult {
    pub revision: u64,
    pub result: Result<ProjectReport, String>,
}
struct Pending {
    revision: u64,
    request: ProjectRequest,
    cancelled: Arc<AtomicBool>,
}

pub struct ProjectController {
    sender: Option<mpsc::SyncSender<Pending>>,
    receiver: mpsc::Receiver<ProjectResult>,
    worker: Option<JoinHandle<()>>,
    cancelled: Option<Arc<AtomicBool>>,
    revision: u64,
    running: bool,
}
impl ProjectController {
    pub fn new(backend: Backend) -> Self {
        Self::with_runner(move |request, cancelled| backend.execute_project(request, cancelled))
    }
    fn with_runner<F>(run: F) -> Self
    where
        F: Fn(&ProjectRequest, Arc<AtomicBool>) -> Result<ProjectReport, String> + Send + 'static,
    {
        let (sender, requests) = mpsc::sync_channel::<Pending>(1);
        let (results, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while let Ok(pending) = requests.recv() {
                let result = run(&pending.request, pending.cancelled);
                if results
                    .send(ProjectResult {
                        revision: pending.revision,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            cancelled: None,
            revision: 0,
            running: false,
        }
    }
    pub fn submit(&mut self, request: ProjectRequest) -> Result<u64, String> {
        if self.running {
            return Err(
                "A project operation is still running. Wait for it to finish or cancel it first."
                    .into(),
            );
        }
        self.revision += 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.sender
            .as_ref()
            .ok_or("The project worker has stopped.")?
            .try_send(Pending {
                revision: self.revision,
                request,
                cancelled: cancelled.clone(),
            })
            .map_err(|_| "The project worker is unavailable.")?;
        self.cancelled = Some(cancelled);
        self.running = true;
        Ok(self.revision)
    }
    /// Cancellation is cooperative at the worker boundary and kills/reaps the
    /// current CLI process tree. A format may already have written some files.
    pub fn cancel(&mut self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.store(true, Ordering::Release);
        }
    }
    pub fn is_running(&self) -> bool {
        self.running
    }
    pub fn poll(&mut self) -> Option<ProjectResult> {
        match self.receiver.try_recv() {
            Ok(result) if result.revision == self.revision => {
                self.running = false;
                self.cancelled = None;
                Some(result)
            }
            Ok(_) | Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) if self.running => {
                self.running = false;
                self.cancelled = None;
                Some(ProjectResult { revision: self.revision, result: Err("The project worker stopped unexpectedly. Formatting may have partially completed; run Check project before continuing.".into()) })
            }
            Err(_) => None,
        }
    }
}
impl Drop for ProjectController {
    fn drop(&mut self) {
        self.cancel();
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::ConfigDocument, project::ProjectAction};
    use std::time::{Duration, Instant};
    fn request(directory: &std::path::Path) -> ProjectRequest {
        let config = directory.join("dprint.json");
        std::fs::write(&config, "{\"plugins\": []}\n").unwrap();
        ProjectRequest::new(
            directory.to_owned(),
            &ConfigDocument::load(&config).unwrap(),
            ProjectAction::Check,
            true,
        )
        .unwrap()
    }
    #[test]
    fn rejects_parallel_work_cancels_and_can_restart_after_reap() {
        let dir = tempfile::tempdir().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let mut controller = ProjectController::with_runner(move |_, cancelled| {
            started_tx.send(()).unwrap();
            while !cancelled.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            Err("cancelled".into())
        });
        let revision = controller.submit(request(dir.path())).unwrap();
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(controller.submit(request(dir.path())).is_err());
        controller.cancel();
        assert!(
            controller.is_running(),
            "cancellation must await process reaping"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = controller.poll() {
                assert_eq!(result.revision, revision);
                assert_eq!(result.result.unwrap_err(), "cancelled");
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!controller.is_running());
        assert!(controller.submit(request(dir.path())).unwrap() > revision);
        controller.cancel();
    }
    #[test]
    fn drop_cancels_and_joins_worker() {
        let dir = tempfile::tempdir().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let mut controller = ProjectController::with_runner(move |_, cancelled| {
            started_tx.send(()).unwrap();
            while !cancelled.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            worker_stopped.store(true, Ordering::Release);
            Err("cancelled".into())
        });
        controller.submit(request(dir.path())).unwrap();
        started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(controller);
        assert!(stopped.load(Ordering::Acquire));
    }
}
