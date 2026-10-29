//! Native Rust formatting from the verified, managed language pack.
//!
//! This is deliberately not a dprint process-plugin or arbitrary command runner.
//! Only stable formatting settings enter a private rustfmt config; source is
//! supplied on stdin so rustfmt cannot walk or rewrite sibling module files.
use crate::{backend::FormatRequest, toolchains::ToolchainManager};
use serde_json::{Map, Value};
use std::{
    fs,
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

const OUTPUT_LIMIT: usize = 32 * 1024 * 1024;
pub const OPTIONS: &[&str] = &[
    "edition",
    "style_edition",
    "max_width",
    "hard_tabs",
    "tab_spaces",
    "newline_style",
    "use_small_heuristics",
    "reorder_imports",
    "reorder_modules",
    "use_field_init_shorthand",
    "use_try_shorthand",
    "match_block_trailing_comma",
];

/// Return a validated native configuration. dprint does not interpret this
/// namespace; unresolved inheritance is rejected rather than silently ignored.
pub fn options(config: &str) -> Result<Map<String, Value>, String> {
    let value = jsonc_parser::parse_to_serde_value(config, &jsonc_parser::ParseOptions::default())
        .map_err(|e| format!("Invalid Rust formatting configuration: {e}"))?
        .ok_or("The formatting configuration is empty.")?;
    if !value.is_object() {
        return Err("The formatting configuration must be a JSONC object.".into());
    }
    if value.get("extends").is_some()
        || value
            .get("inherit")
            .is_some_and(|v| v != &Value::Bool(false))
        || value.get("overrides").is_some()
    {
        return Err("Native Rust formatting does not resolve dprint extends, inherit, or overrides. Use an explicit local rustfmt section without these features.".into());
    }
    let mut result = match value.get("rustfmt") {
        None => Map::new(),
        Some(Value::Object(options)) => options.clone(),
        Some(_) => return Err("The rustfmt section must be an object.".into()),
    };
    for (name, value) in &result {
        validate_option(name, value)?;
    }
    result
        .entry("edition")
        .or_insert(Value::String("2021".into()));
    result
        .entry("style_edition")
        .or_insert(Value::String("2021".into()));
    Ok(result)
}

pub fn validate_option(name: &str, value: &Value) -> Result<(), String> {
    let valid = match name {
        "edition" | "style_edition" => value
            .as_str()
            .is_some_and(|v| ["2015", "2018", "2021", "2024"].contains(&v)),
        "max_width" => value.as_u64().is_some_and(|v| (1..=10_000).contains(&v)),
        "tab_spaces" => value.as_u64().is_some_and(|v| (1..=16).contains(&v)),
        "newline_style" => value
            .as_str()
            .is_some_and(|v| ["Auto", "Windows", "Unix", "Native"].contains(&v)),
        "use_small_heuristics" => value
            .as_str()
            .is_some_and(|v| ["Off", "Max", "Default"].contains(&v)),
        "hard_tabs"
        | "reorder_imports"
        | "reorder_modules"
        | "use_field_init_shorthand"
        | "use_try_shorthand"
        | "match_block_trailing_comma" => value.is_boolean(),
        _ => {
            return Err(format!(
                "Unsupported Rust setting {name:?}. Only the supported stable formatting options are allowed; commands, file paths and nightly settings are not accepted."
            ));
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid value for Rust setting {name:?}: {value}"))
    }
}

/// Manager caches hashes and issues a shared lease for each formatting process.
/// Keep the lease until all pipes are closed; never cache a lease across jobs.
#[derive(Clone, Default)]
pub(crate) struct RustfmtRuntime(Arc<Mutex<Option<ToolchainManager>>>);
impl std::fmt::Debug for RustfmtRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RustfmtRuntime(managed)")
    }
}
impl RustfmtRuntime {
    pub(crate) fn with_manager(manager: ToolchainManager) -> Self {
        Self(Arc::new(Mutex::new(Some(manager))))
    }
    pub(crate) fn format(
        &self,
        request: &FormatRequest,
        cancelled: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<String, String> {
        let started = Instant::now();
        let settings = options(request.config.source_text())?;
        if cancelled.load(Ordering::Acquire) {
            return Err("Rust formatting cancelled.".into());
        }
        let runtime = {
            let mut manager = loop {
                if cancelled.load(Ordering::Acquire) {
                    return Err("Rust formatting cancelled.".into());
                }
                if started.elapsed() >= timeout {
                    return Err("Rust formatting timed out waiting for its managed runtime.".into());
                }
                match self.0.try_lock() {
                    Ok(manager) => break manager,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => return Err("Rust runtime manager is unavailable.".into()),
                }
            };
            if manager.is_none() {
                *manager = Some(ToolchainManager::discover()?);
            }
            manager
                .as_ref()
                .expect("initialized manager")
                .resolve_cancellable("rustfmt", &cancelled)
                .map_err(|e| {
                    format!("Install Rust support from Language support before formatting. {e}")
                })?
        };
        let timeout = timeout
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or("Rust formatting timed out verifying its managed runtime.")?;
        format_with_runtime(
            &runtime.executable,
            &runtime.environment,
            &request.source,
            &settings,
            cancelled,
            timeout,
        )
    }
}

/// Portable native rustfmt TOML, not a dprint plugin configuration.
pub fn export_config(snapshot: &crate::config::ConfigSnapshot) -> Result<String, String> {
    let settings = options(snapshot.source_text())?;
    Ok(settings
        .iter()
        .map(|(key, value)| format!("{key} = {value}\n"))
        .collect())
}

fn format_with_runtime(
    executable: &Path,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
    source: &str,
    settings: &Map<String, Value>,
    cancelled: Arc<AtomicBool>,
    timeout: Duration,
) -> Result<String, String> {
    if source.len() > OUTPUT_LIMIT {
        return Err("Rust source exceeds the 32 MiB safety limit.".into());
    }
    let directory =
        tempfile::tempdir().map_err(|e| format!("Cannot create isolated Rust config: {e}"))?;
    let config = directory.path().join("rustfmt.toml");
    let text = settings
        .iter()
        .map(|(key, value)| format!("{key} = {value}\n"))
        .collect::<String>();
    fs::write(&config, text).map_err(|e| format!("Cannot prepare Rust config: {e}"))?;
    let mut command = Command::new(executable);
    command.env_clear();
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .envs(environment.iter().cloned())
        .env("HOME", directory.path())
        .env("TMPDIR", directory.path())
        .env("TMP", directory.path())
        .env("TEMP", directory.path())
        .current_dir(directory.path())
        .arg("--emit")
        .arg("stdout")
        .arg("--color")
        .arg("never")
        .arg("--config-path")
        .arg(&config)
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
        command.creation_flags(0x08000000);
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("Rust formatting cancelled.".into());
    }
    let mut child = command.spawn().map_err(|e| {
        format!("Cannot start the verified Rust formatter: {e}. Reinstall Rust support.")
    })?;
    let mut stdin = child.stdin.take().expect("piped input");
    let output = child.stdout.take().expect("piped output");
    let errors = child.stderr.take().expect("piped errors");
    let source = source.to_owned();
    let writer = thread::spawn(move || stdin.write_all(source.as_bytes()));
    let reader = thread::spawn(move || bounded_read(output));
    let error_reader = thread::spawn(move || bounded_read(errors));
    let started = Instant::now();
    let status = loop {
        if cancelled.load(Ordering::Acquire) {
            break Err("Rust formatting cancelled.".to_owned());
        }
        if started.elapsed() >= timeout {
            break Err("Rust formatting timed out.".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(e) => break Err(format!("Cannot wait for Rust formatter: {e}")),
        }
    };
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // SAFETY: the child was started in its own group; no caller-supplied PID.
        if status.is_err() {
            unsafe {
                kill(-(child.id() as i32), 9);
            }
        }
    }
    if status.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let written = writer.join().map_err(|_| "Rust input writer failed.")?;
    let stdout = reader.join().map_err(|_| "Rust output reader failed.")??;
    let stderr = error_reader
        .join()
        .map_err(|_| "Rust diagnostic reader failed.")??;
    let status = status?;
    if !status.success() || !stderr.is_empty() {
        return Err(format!(
            "Rust formatter {}: {}",
            status,
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    written.map_err(|e| format!("Cannot write source to Rust formatter: {e}"))?;
    if cancelled.load(Ordering::Acquire) {
        return Err("Rust formatting cancelled.".into());
    }
    String::from_utf8(stdout).map_err(|e| format!("Rust formatter returned invalid UTF-8: {e}"))
}

fn bounded_read(mut stream: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let available = OUTPUT_LIMIT.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..n.min(available)]);
        overflow |= n > available;
    }
    if overflow {
        Err("Rust formatter output exceeded the 32 MiB safety limit.".into())
    } else {
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_explicit_stable_scalar_settings_are_accepted() {
        for (key, value) in [
            ("edition", json!("2027")),
            ("style_edition", json!("2027")),
            ("max_width", json!(1.5)),
            ("tab_spaces", json!(0)),
            ("unstable_features", json!(true)),
            ("emit_mode", json!("Files")),
            ("config_path", json!("/tmp/a")),
        ] {
            assert!(validate_option(key, &value).is_err(), "{key}");
        }
        assert_eq!(options("{}").unwrap()["edition"], json!("2021"));
        assert!(options(r#"{"extends":"base.json"}"#).is_err());
        assert!(options(r#"{"rustfmt":{"edition":"2024","hard_tabs":true}}"#).is_ok());
    }
}
