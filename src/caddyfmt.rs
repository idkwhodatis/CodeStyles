//! Managed, isolated Caddy formatter-only WASM through verified dprint.
//! Exact upstream one-pass formatting; no server, imports or config loading.
use crate::{
    backend::FormatRequest,
    script_source::{self, ScriptLanguage},
    toolchains::ToolchainManager,
};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub fn options(text: &str) -> Result<serde_json::Value, String> {
    crate::config::validate_managed_language_config(text, "caddyfile")
}

#[derive(Clone, Default)]
pub(crate) struct CaddyfmtRuntime(Arc<Mutex<Option<ToolchainManager>>>);
impl std::fmt::Debug for CaddyfmtRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CaddyfmtRuntime(managed)")
    }
}
impl CaddyfmtRuntime {
    pub(crate) fn with_manager(manager: ToolchainManager) -> Self {
        Self(Arc::new(Mutex::new(Some(manager))))
    }
    pub(crate) fn format(
        &self,
        request: &FormatRequest,
        cancel: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<String, String> {
        let started = Instant::now();
        let settings = options(request.config.source_text())?;
        script_source::validate_source(ScriptLanguage::Caddyfile, &request.source, &settings)?;
        let (runtime, engine) = {
            let mut manager = loop {
                if cancel.load(Ordering::Acquire) {
                    return Err("Caddyfile formatting cancelled.".into());
                }
                if started.elapsed() >= timeout {
                    return Err(
                        "Caddyfile formatting timed out waiting for managed runtime.".into(),
                    );
                }
                match self.0.try_lock() {
                    Ok(manager) => break manager,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => return Err("Caddyfile runtime manager is unavailable.".into()),
                }
            };
            if manager.is_none() {
                *manager = Some(ToolchainManager::discover()?);
            }
            let manager = manager.as_ref().unwrap();
            let runtime = manager
                .resolve_cancellable("caddyfmt", &cancel)
                .map_err(|e| {
                    format!(
                        "Install Caddyfile support from Language support before formatting. {e}"
                    )
                })?;
            let engine = manager.resolve_cancellable("dprint", &cancel).map_err(|e| {
                format!("Caddyfile support requires the managed dprint engine. Install or repair dprint in Language support. {e}")
            })?;
            (runtime, engine)
        };
        if !runtime.executable.as_os_str().is_empty() || runtime.wasm_plugins.len() != 1 {
            return Err(
                "Caddyfile support requires the complete verified formatter WASM pack.".into(),
            );
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or("Caddyfile formatting timed out verifying managed runtime.")?;
        // Keep both pack leases alive until all formatter pipes/processes exit.
        let output = format_with_runtime(
            &engine.executable,
            &runtime.wasm_plugins[0],
            &request.source,
            cancel,
            remaining,
        )?;
        script_source::validate_output(ScriptLanguage::Caddyfile, &request.source, &output)?;
        Ok(output)
    }
}

fn format_with_runtime(
    executable: &Path,
    plugin: &Path,
    source: &str,
    cancel: Arc<AtomicBool>,
    timeout: Duration,
) -> Result<String, String> {
    let directory = tempfile::tempdir()
        .map_err(|e| format!("Cannot create isolated Caddyfile workspace: {e}"))?;
    let workspace = directory.path().canonicalize().map_err(|e| e.to_string())?;
    let plugin = plugin
        .canonicalize()
        .map_err(|e| format!("Cannot locate verified Caddy formatter: {e}"))?;
    let plugin = plugin
        .to_str()
        .ok_or("Caddy formatter plugin path must be UTF-8.")?;
    let config = workspace.join("dprint.json");
    // Only the verified Caddy formatter is loaded. Caller plugins, styles,
    // configuration discovery, includes and project metadata cannot affect it.
    std::fs::write(
        &config,
        serde_json::json!({
            "caddyfmt": {}, "plugins": [plugin], "incremental": false,
        })
        .to_string(),
    )
    .map_err(|e| format!("Cannot prepare Caddy formatter config: {e}"))?;
    let mut command = Command::new(executable);
    command
        .env_clear()
        .current_dir(&workspace)
        .args([
            "fmt",
            "--config-discovery=false",
            "--skip-stable-format",
            "--log-level=silent",
            "--stdin=Caddyfile",
            "--config",
        ])
        .arg(&config)
        .env("DPRINT_CACHE_DIR", workspace.join("cache"));
    for key in [
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "TMPDIR",
        "TMP",
        "TEMP",
    ] {
        command.env(key, &workspace);
    }
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Caddyfile formatting cancelled.".into());
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start verified Caddyfile formatter: {e}"))?;
    #[cfg(windows)]
    let process_tree = crate::project::WindowsProcessTree::attach_and_resume(&mut child, &cancel)?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let source = source.to_owned();
    let writer = thread::spawn(move || stdin.write_all(source.as_bytes()));
    let reader = thread::spawn(move || bounded_read(stdout));
    let errors = thread::spawn(move || bounded_read(stderr));
    let started = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Acquire) {
            break Err("Caddyfile formatting cancelled.".to_owned());
        }
        if started.elapsed() >= timeout {
            break Err("Caddyfile formatting timed out.".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => break Err(format!("Cannot wait for Caddyfile formatter: {error}")),
        }
    };
    // Descendants can otherwise retain the pipes after the parent has exited.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    process_tree.terminate();
    if status.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let written = writer
        .join()
        .map_err(|_| "Caddyfile input writer failed.")?;
    let output = reader
        .join()
        .map_err(|_| "Caddyfile output reader failed.")??;
    let diagnostic = errors
        .join()
        .map_err(|_| "Caddyfile diagnostic reader failed.")??;
    let status = status?;
    if !status.success() || !diagnostic.is_empty() {
        return Err(format!(
            "Caddyfile formatter {status}: {}",
            String::from_utf8_lossy(&diagnostic)
                .chars()
                .take(8192)
                .collect::<String>()
                .trim()
        ));
    }
    written.map_err(|e| format!("Cannot send source to Caddyfile formatter: {e}"))?;
    if cancel.load(Ordering::Acquire) {
        return Err("Caddyfile formatting cancelled.".into());
    }
    String::from_utf8(output).map_err(|_| "Caddyfile formatter returned invalid UTF-8.".into())
}

fn bounded_read(mut stream: impl Read) -> Result<Vec<u8>, String> {
    const LIMIT: usize = 32 * 1024 * 1024;
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let count = stream.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        let remaining = LIMIT.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..count.min(remaining)]);
        overflow |= count > remaining;
    }
    if overflow {
        Err("Caddyfile formatter output exceeds the 32 MiB limit.".into())
    } else {
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_output_is_drained_and_limited() {
        assert_eq!(bounded_read(&b"hello"[..]).unwrap(), b"hello");
        assert!(bounded_read(std::io::repeat(b'x').take(32 * 1024 * 1024 + 1)).is_err());
    }
    #[cfg(unix)]
    fn fixture(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("caddy-test");
        std::fs::write(&file, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir, file)
    }
    #[cfg(unix)]
    #[test]
    fn invocation_has_only_fmt_stdin_and_isolated_environment() {
        let (_dir, executable) = fixture(
            "test \"$#\" = 7 && test \"$1\" = fmt && test \"$2\" = --config-discovery=false && test \"$3\" = --skip-stable-format && test \"$4\" = --log-level=silent && test \"$5\" = --stdin=Caddyfile && test \"$6\" = --config || exit 4\ntest \"$HOME\" = \"$PWD\" || exit 5\ntest -z \"$CADDY_TEST_SECRET\" || exit 6\ntest \"$DPRINT_CACHE_DIR\" = \"$PWD/cache\" && test -f \"$7\" || exit 7\n/bin/cat",
        );
        assert_eq!(
            format_with_runtime(
                &executable,
                &executable,
                "localhost {}\n",
                Arc::new(AtomicBool::new(false)),
                Duration::from_secs(2)
            )
            .unwrap(),
            "localhost {}\n"
        );
    }
    #[cfg(unix)]
    #[test]
    fn errors_timeouts_and_cancel_never_become_source_output() {
        let (_dir, executable) = fixture("echo failed >&2; exit 1");
        assert!(
            format_with_runtime(
                &executable,
                &executable,
                "input",
                Arc::new(AtomicBool::new(false)),
                Duration::from_secs(2)
            )
            .is_err()
        );
        let (_dir, executable) = fixture("/bin/sleep 30");
        let started = Instant::now();
        assert!(
            format_with_runtime(
                &executable,
                &executable,
                "input",
                Arc::new(AtomicBool::new(false)),
                Duration::from_millis(30)
            )
            .unwrap_err()
            .contains("timed out")
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            format_with_runtime(
                &executable,
                &executable,
                "input",
                Arc::new(AtomicBool::new(true)),
                Duration::from_secs(2)
            )
            .unwrap_err()
            .contains("cancelled")
        );
    }
}
