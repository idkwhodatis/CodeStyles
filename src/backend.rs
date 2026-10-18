//! CLI-only formatting backend. No formatter core is linked into the application.
use crate::config::DraftConfig;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct FormatRequest {
    pub source: String,
    pub source_path: PathBuf,
    /// The file is read when this request starts formatting, after debounce.
    /// Source/settings are immutable snapshots; external config edits are not snapshotted.
    pub config_path: Option<PathBuf>,
    pub indent_width: u8,
    pub semi_colons: String,
}

#[derive(Clone, Debug)]
pub struct Backend {
    executable: PathBuf,
    plugins: Vec<PathBuf>,
    timeout: Duration,
}

impl Backend {
    pub fn new(executable: PathBuf, plugins: Vec<PathBuf>) -> Self {
        Self {
            executable,
            plugins,
            timeout: Duration::from_secs(30),
        }
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn format(
        &self,
        request: &FormatRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("Formatting cancelled.".into());
        }
        let draft = DraftConfig::new(
            request.config_path.as_deref(),
            &self.plugins,
            request.indent_width,
            &request.semi_colons,
        )?;
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
        command
            .arg("fmt")
            .arg("--config-discovery=false")
            .arg("--config")
            .arg(draft.path())
            .arg("--stdin")
            .arg(&request.source_path)
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
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let source = request.source.clone();
        // All three pipes are serviced concurrently. Large input or diagnostics
        // cannot deadlock the process before cancellation/timeout is checked.
        let writer = thread::spawn(move || stdin.write_all(source.as_bytes()));
        let output_reader = thread::spawn(move || {
            let mut out = Vec::new();
            stdout.read_to_end(&mut out).map(|_| out)
        });
        let error_reader = thread::spawn(move || {
            let mut out = Vec::new();
            stderr.read_to_end(&mut out).map(|_| out)
        });
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

fn cache_directory(
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
