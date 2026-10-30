//! CLI-only formatting backend. No formatter core is linked into the application.
use crate::config::{ConfigDocument, ConfigSnapshot, DraftConfig};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct FormatRequest {
    pub source: String,
    pub source_path: PathBuf,
    /// Exact draft bytes captured before queuing, with original path semantics.
    pub config: ConfigSnapshot,
}

#[derive(Clone, Debug)]
pub struct Backend {
    executable: PathBuf,
    plugins: Vec<PathBuf>,
    timeout: Duration,
    rustfmt: crate::rustfmt::RustfmtRuntime,
    native: crate::native_format::NativeRuntime,
    managed: Arc<Mutex<Option<crate::toolchains::ToolchainManager>>>,
}

impl Backend {
    pub fn new(executable: PathBuf, plugins: Vec<PathBuf>) -> Self {
        Self {
            executable,
            plugins,
            timeout: Duration::from_secs(30),
            rustfmt: Default::default(),
            native: Default::default(),
            managed: Default::default(),
        }
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Select an already validated managed-pack location, never an arbitrary
    /// executable or catalog. Useful for explicit portable installations.
    pub fn with_toolchain_manager(mut self, manager: crate::toolchains::ToolchainManager) -> Self {
        self.rustfmt = crate::rustfmt::RustfmtRuntime::with_manager(manager.clone());
        self.native = crate::native_format::NativeRuntime::with_manager(manager.clone());
        self.managed = Arc::new(Mutex::new(Some(manager)));
        self
    }

    /// The verified, explicitly selected bundled CLI, never a PATH lookup.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    /// A portable new document referencing the exact bundled plugin versions.
    /// Only preview-time exact pinned references map to verified local assets.
    pub fn standalone_config(&self) -> Result<ConfigDocument, String> {
        ConfigDocument::standalone(&self.plugins)
    }

    pub fn format(
        &self,
        request: &FormatRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("Formatting cancelled.".into());
        }
        if request.source.len() > 32 * 1024 * 1024 {
            return Err("Source exceeds the 32 MiB limit.".into());
        }
        if request
            .source_path
            .extension()
            .is_some_and(|extension| extension == "rs")
        {
            return self.rustfmt.format(request, cancelled, self.timeout);
        }
        let extension = request
            .source_path
            .extension()
            .and_then(|value| value.to_str());
        if matches!(extension, Some("go" | "dart")) {
            return self.native.format(request, cancelled, self.timeout);
        }
        // Retain the shared pack lease until every formatter pipe has exited.
        let _vue_runtime;
        let draft = if extension == Some("vue") {
            crate::vue::validate_source(&request.source)?;
            let manager = {
                let mut slot = self
                    .managed
                    .lock()
                    .map_err(|_| "Managed formatter state failed.")?;
                if slot.is_none() {
                    *slot = Some(crate::toolchains::ToolchainManager::discover()?);
                }
                slot.as_ref().expect("initialized").clone()
            };
            _vue_runtime = Some(manager.resolve_cancellable("vue", &cancelled)?);
            let runtime = _vue_runtime.as_ref().expect("resolved");
            if !runtime.executable.as_os_str().is_empty() || runtime.wasm_plugins.len() != 2 {
                return Err("Vue support requires the complete verified WASM pack.".into());
            }
            let mut plugins = self.plugins.clone();
            plugins.extend(runtime.wasm_plugins.iter().cloned());
            DraftConfig::for_vue(&request.config, &plugins)?
        } else {
            _vue_runtime = None;
            DraftConfig::for_preview(&request.config, &self.plugins)?
        };
        let cache = cache_directory(std::env::consts::OS, |key| std::env::var_os(key))?;
        std::fs::create_dir_all(&cache)
            .map_err(|e| format!("Cannot create dprint cache {}: {e}", cache.display()))?;
        let mut command = Command::new(&self.executable);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        command.env("DPRINT_CACHE_DIR", &cache);
        // Keep the path within one option value so names starting with a dash
        // cannot be interpreted as CLI flags. OsString preserves native paths.
        let mut stdin_path = std::ffi::OsString::from("--stdin=");
        stdin_path.push(&request.source_path);
        command
            .arg("fmt")
            .arg("--config-discovery=false")
            .arg("--config")
            .arg(draft.path())
            .arg(stdin_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Cannot start bundled dprint {}: {e}",
                self.executable.display()
            )
        })?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let source = request.source.clone();
        // All three pipes are serviced concurrently. Large input or diagnostics
        // cannot deadlock the process before cancellation/timeout is checked.
        let writer = thread::spawn(move || stdin.write_all(source.as_bytes()));
        let output_reader = thread::spawn(move || bounded_formatter_output(stdout));
        let error_reader = thread::spawn(move || bounded_formatter_output(stderr));
        let started = Instant::now();
        let mut failure = None;
        let status = loop {
            if cancelled.load(Ordering::Acquire) {
                failure = Some("Formatting cancelled.".to_owned());
                break None;
            }
            if started.elapsed() >= self.timeout {
                failure = Some(format!(
                    "Formatting timed out after {} seconds.",
                    self.timeout.as_secs()
                ));
                break None;
            }
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(error) => {
                    failure = Some(format!("Cannot wait for dprint: {error}"));
                    break None;
                }
            }
        };
        if status.is_none() {
            let _ = child.kill();
        }
        // wait() also covers the race where dprint exits just before kill().
        let _ = child.wait();
        let written = writer.join().map_err(|_| "Input writer failed.")?;
        let output = output_reader
            .join()
            .map_err(|_| "Output reader failed.")?
            .map_err(|e| format!("Cannot read dprint output: {e}"))?;
        let errors = error_reader
            .join()
            .map_err(|_| "Error reader failed.")?
            .map_err(|e| format!("Cannot read dprint diagnostics: {e}"))?;
        if let Some(message) = failure {
            return Err(message);
        }
        if !status.expect("completed status").success() {
            return Err(format!(
                "dprint failed: {}",
                String::from_utf8_lossy(&errors).trim()
            ));
        }
        written.map_err(|e| format!("Cannot send input to dprint: {e}"))?;
        if cancelled.load(Ordering::Acquire) {
            return Err("Formatting cancelled.".into());
        }
        String::from_utf8(output).map_err(|e| format!("dprint returned invalid UTF-8: {e}"))
    }
}

fn bounded_formatter_output(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
    const LIMIT: usize = 32 * 1024 * 1024;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut overflow = false;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = LIMIT.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..count.min(remaining)]);
        overflow |= count > remaining;
    }
    if overflow {
        Err(std::io::Error::other(
            "Formatter output exceeds the 32 MiB limit.",
        ))
    } else {
        Ok(bytes)
    }
}

pub(crate) fn cache_directory(
    os: &str,
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    if let Some(path) = env("DPRINT_CACHE_DIR").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let base = match os {
        "windows" => env("LOCALAPPDATA").map(PathBuf::from),
        "macos" => env("HOME").map(|home| PathBuf::from(home).join("Library/Caches")),
        _ => env("XDG_CACHE_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".cache"))),
    }
    .ok_or("Cannot locate user cache directory; set DPRINT_CACHE_DIR to a writable directory.")?;
    Ok(base.join("CodeStyles").join("dprint"))
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    fn resolve(os: &str, values: &[(&str, &str)]) -> PathBuf {
        cache_directory(os, |name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).into())
        })
        .unwrap()
    }
    #[test]
    fn platform_cache_paths_and_explicit_override() {
        assert_eq!(
            resolve("linux", &[("HOME", "/home/person")]),
            PathBuf::from("/home/person/.cache/CodeStyles/dprint")
        );
        assert_eq!(
            resolve(
                "linux",
                &[("HOME", "/home/person"), ("XDG_CACHE_HOME", "/cache")]
            ),
            PathBuf::from("/cache/CodeStyles/dprint")
        );
        assert_eq!(
            resolve("macos", &[("HOME", "/Users/person")]),
            PathBuf::from("/Users/person/Library/Caches/CodeStyles/dprint")
        );
        assert_eq!(
            resolve("windows", &[("LOCALAPPDATA", "local")]),
            PathBuf::from("local/CodeStyles/dprint")
        );
        assert_eq!(
            resolve(
                "linux",
                &[("HOME", "/home/person"), ("DPRINT_CACHE_DIR", "/override")]
            ),
            PathBuf::from("/override")
        );
        assert!(cache_directory("linux", |_| None).is_err());
    }
}

#[cfg(test)]
mod output_limit_tests {
    use super::*;
    #[test]
    fn formatter_output_is_drained_but_never_kept_above_limit() {
        assert_eq!(bounded_formatter_output(&b"ok"[..]).unwrap(), b"ok");
        assert!(
            bounded_formatter_output(std::io::repeat(b'x').take(32 * 1024 * 1024 + 1)).is_err()
        );
    }
}
