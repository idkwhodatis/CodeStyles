//! One owned worker for language-pack I/O. Never performs network, hashing, or
//! extraction on the UI thread. Progress is coalesced rather than queued forever.
use crate::toolchains::{InstallProgress, ToolchainManager, ToolchainStatus};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackAction {
    Refresh,
    Install,
    InstallLocal(PathBuf),
    Uninstall,
}

#[derive(Clone, Debug)]
pub struct PackSnapshot {
    pub status: ToolchainStatus,
    pub root: PathBuf,
    pub available: bool,
    pub local_available: bool,
    pub publication_note: String,
}

#[derive(Debug)]
pub struct PackCompletion {
    pub action: PackAction,
    pub result: Result<(), String>,
    pub snapshot: Result<PackSnapshot, String>,
}

pub struct ToolchainController {
    sender: Option<mpsc::SyncSender<(PackAction, Arc<AtomicBool>)>>,
    receiver: mpsc::Receiver<PackCompletion>,
    progress: Arc<Mutex<Option<InstallProgress>>>,
    cancelled: Option<Arc<AtomicBool>>,
    worker: Option<JoinHandle<()>>,
    running: bool,
}

impl Default for ToolchainController {
    fn default() -> Self {
        Self::new()
    }
}
impl ToolchainController {
    pub fn new() -> Self {
        // Pin the selected root for this app session. Losing write permission must
        // not silently redirect an uninstall to a different fallback installation.
        let manager = Mutex::new(None);
        Self::with_runner(move |action, cancel, progress| run(&manager, action, cancel, progress))
    }

    fn with_runner<F>(runner: F) -> Self
    where
        F: Fn(PackAction, Arc<AtomicBool>, &dyn Fn(InstallProgress)) -> PackCompletion
            + Send
            + 'static,
    {
        let (sender, requests) = mpsc::sync_channel::<(PackAction, Arc<AtomicBool>)>(1);
        let (results, receiver) = mpsc::channel();
        let progress = Arc::new(Mutex::new(None));
        let updates = progress.clone();
        let worker = thread::spawn(move || {
            while let Ok((action, cancelled)) = requests.recv() {
                let result = runner(action, cancelled, &|value| {
                    *updates.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
                });
                if results.send(result).is_err() {
                    break;
                }
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            progress,
            cancelled: None,
            worker: Some(worker),
            running: false,
        }
    }

    pub fn submit(&mut self, action: PackAction) -> Result<(), String> {
        if self.running {
            return Err("A language-pack operation is still running. Cancel or wait before starting another.".into());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.sender
            .as_ref()
            .ok_or("Language-pack worker stopped.")?
            .try_send((action, cancelled.clone()))
            .map_err(|_| "Language-pack worker unavailable.")?;
        self.cancelled = Some(cancelled);
        self.running = true;
        Ok(())
    }
    pub fn is_running(&self) -> bool {
        self.running
    }
    pub fn cancel(&self) {
        if let Some(cancelled) = &self.cancelled {
            cancelled.store(true, Ordering::Release);
        }
    }
    pub fn progress(&self) -> Option<InstallProgress> {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
    pub fn poll(&mut self) -> Option<PackCompletion> {
        match self.receiver.try_recv() {
            Ok(result) => {
                self.running = false;
                self.cancelled = None;
                Some(result)
            }
            Err(mpsc::TryRecvError::Disconnected) if self.running => {
                self.running = false;
                self.cancelled = None;
                Some(PackCompletion { action: PackAction::Refresh, result: Err("Language-pack worker stopped unexpectedly. Refresh its status before trying again.".into()), snapshot: Err("Status unknown.".into()) })
            }
            Err(_) => None,
        }
    }
}
impl Drop for ToolchainController {
    fn drop(&mut self) {
        self.cancel();
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn snapshot(manager: &ToolchainManager, cancel: &AtomicBool) -> Result<PackSnapshot, String> {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../resources/toolchains.json"))
            .map_err(|e| e.to_string())?;
    let target = crate::resources::target()?;
    let spec = &catalog["toolchains"]["rustfmt"]["targets"][target];
    Ok(PackSnapshot {
        status: manager.status_cancellable("rustfmt", cancel)?,
        root: manager.root().to_path_buf(),
        available: manager.download_available("rustfmt").is_ok(),
        local_available: spec["archives"]
            .as_array()
            .is_some_and(|archives| archives.len() == 1),
        publication_note: spec["reason"]
            .as_str()
            .unwrap_or("Pinned GitHub release pack.")
            .into(),
    })
}
fn run(
    cached_manager: &Mutex<Option<ToolchainManager>>,
    action: PackAction,
    cancel: Arc<AtomicBool>,
    progress: &dyn Fn(InstallProgress),
) -> PackCompletion {
    let mut cached = cached_manager.lock().unwrap_or_else(|e| e.into_inner());
    if cached.is_none() {
        match ToolchainManager::discover() {
            Ok(manager) => *cached = Some(manager),
            Err(error) => {
                return PackCompletion {
                    action,
                    result: Err(error.clone()),
                    snapshot: Err(error),
                };
            }
        }
    }
    let manager = cached.as_ref().unwrap();
    let result = match &action {
        PackAction::Refresh => Ok(()),
        PackAction::Install => manager
            .install("rustfmt", cancel.clone(), progress)
            .map(|_| ()),
        PackAction::InstallLocal(path) => manager
            .install_verified_archive("rustfmt", path, cancel.clone(), progress)
            .map(|_| ()),
        PackAction::Uninstall => manager.uninstall_cancellable("rustfmt", &cancel),
    };
    PackCompletion {
        action,
        result,
        snapshot: snapshot(manager, &cancel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn finish(action: PackAction, result: Result<(), String>) -> PackCompletion {
        PackCompletion {
            action,
            result,
            snapshot: Err("fixture".into()),
        }
    }
    fn wait(controller: &mut ToolchainController) -> PackCompletion {
        let start = Instant::now();
        loop {
            if let Some(result) = controller.poll() {
                return result;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn repeated_actions_are_rejected_until_completion_even_after_cancel() {
        let mut controller = ToolchainController::with_runner(|action, cancel, _| {
            while !cancel.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(2));
            }
            finish(action, Err("cancelled".into()))
        });
        controller.submit(PackAction::Install).unwrap();
        assert!(controller.submit(PackAction::Uninstall).is_err());
        controller.cancel();
        assert!(controller.submit(PackAction::Refresh).is_err());
        assert!(wait(&mut controller).result.is_err());
        assert!(!controller.is_running());
        controller.submit(PackAction::Install).unwrap();
        controller.cancel();
        let _ = wait(&mut controller);
    }
    #[test]
    fn drop_cancels_and_joins_the_operation() {
        let stopped = Arc::new(AtomicBool::new(false));
        let observed = stopped.clone();
        let mut controller = ToolchainController::with_runner(move |action, cancel, _| {
            while !cancel.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(2));
            }
            observed.store(true, Ordering::Release);
            finish(action, Err("cancelled".into()))
        });
        controller.submit(PackAction::Install).unwrap();
        drop(controller);
        assert!(stopped.load(Ordering::Acquire));
    }
    #[test]
    fn progress_is_coalesced_and_completion_preserves_action() {
        let mut controller = ToolchainController::with_runner(|action, _, progress| {
            for byte in 0..1000 {
                progress(InstallProgress {
                    phase: "download",
                    downloaded_bytes: byte,
                    total_bytes: Some(1000),
                });
            }
            finish(action, Ok(()))
        });
        controller.submit(PackAction::Refresh).unwrap();
        let result = wait(&mut controller);
        assert_eq!(result.action, PackAction::Refresh);
        assert_eq!(controller.progress().unwrap().downloaded_bytes, 999);
        assert!(controller.progress().is_none());
    }
}
