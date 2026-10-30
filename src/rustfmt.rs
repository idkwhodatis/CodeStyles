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

/// Read a user-selected, regular UTF-8 rustfmt.toml without following a final
/// symlink/reparse point or blocking on a FIFO swapped in before open. The
/// handle is checked after open and the streaming limit also covers file growth.
pub fn read_import_config(path: &Path) -> Result<String, String> {
    fn regular(metadata: &fs::Metadata) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return false;
            } // FILE_ATTRIBUTE_REPARSE_POINT
        }
        metadata.is_file() && !metadata.file_type().is_symlink()
    }
    let before = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect rustfmt.toml: {error}"))?;
    if !regular(&before) {
        return Err("Rust configuration import requires a regular non-symlink file.".into());
    }
    if before.len() > IMPORT_LIMIT as u64 {
        return Err("Rust configuration import exceeds the 1 MiB limit.".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|error| format!("Cannot open rustfmt.toml safely: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("Cannot inspect opened rustfmt.toml: {error}"))?;
    if !regular(&opened) {
        return Err("Rust configuration import requires a regular non-symlink file.".into());
    }
    let mut bytes = Vec::new();
    file.take(IMPORT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read rustfmt.toml: {error}"))?;
    if bytes.len() > IMPORT_LIMIT {
        return Err("Rust configuration import exceeds the 1 MiB limit.".into());
    }
    String::from_utf8(bytes).map_err(|_| "rustfmt.toml is not valid UTF-8.".into())
}

const IMPORT_LIMIT: usize = 1024 * 1024;

/// Explicitly merge supported settings from a rustfmt.toml into the draft.
/// Only keys present in the TOML overwrite existing Rust settings; absent keys,
/// other sections, and existing JSONC comments are preserved. TOML comments are
/// not copied into JSONC. No disk write, implicit project lookup, or installation
/// occurs. Every parse/validation/CST edit must succeed before the draft changes.
pub fn import_config(
    document: &mut crate::config::ConfigDocument,
    text: &str,
) -> Result<usize, String> {
    import_config_inner(document, text).map_err(|error| bounded_import_message(&error, 2048))
}

fn bounded_import_message(message: &str, limit: usize) -> String {
    if message.len() <= limit {
        return message.to_owned();
    }
    let mut end = limit - "…".len();
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

fn import_config_inner(
    document: &mut crate::config::ConfigDocument,
    text: &str,
) -> Result<usize, String> {
    if text.len() > IMPORT_LIMIT {
        return Err("Rust configuration import exceeds the 1 MiB limit.".into());
    }
    let table = text.parse::<toml::Table>().map_err(|error| {
        // toml::Error Display includes the entire source line and caret.
        // Use its concise message and calculate a Unicode character column
        // ourselves, without copying an untrusted (up to 1 MiB) source line.
        let location = error
            .span()
            .map(|span| {
                let mut offset = span.start.min(text.len());
                while !text.is_char_boundary(offset) {
                    offset -= 1;
                }
                let prefix = &text[..offset];
                let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
                let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                format!(" at line {line}, column {column}")
            })
            .unwrap_or_default();
        format!(
            "Invalid rustfmt.toml{location}: {}",
            bounded_import_message(error.message(), 1024)
        )
    })?;
    if table.is_empty() {
        return Err("rustfmt.toml contains no settings to import.".into());
    }
    let mut settings = Map::new();
    for (name, value) in table {
        // Validate the name before converting so unknown nested tables are not
        // silently flattened or dropped, and nightly/path options remain denied.
        if !OPTIONS.contains(&name.as_str()) {
            let name = bounded_import_message(&name, 160);
            return Err(format!(
                "Unsupported Rust setting {name:?}; import supports only the 12 stable settings exposed by CodeStyles. Nothing was imported."
            ));
        }
        let value = match value {
            toml::Value::String(value) => Value::String(value),
            toml::Value::Integer(value) => Value::from(value),
            toml::Value::Boolean(value) => Value::Bool(value),
            _ => {
                return Err(format!(
                    "Unsupported value type for Rust setting {name:?}; only its documented string, integer, or boolean type is accepted. Nothing was imported."
                ));
            }
        };
        validate_option(&name, &value)?;
        settings.insert(name, value);
    }
    let count = settings.len();
    let mut edited = document.clone();
    for (name, value) in settings {
        edited.set(&["rustfmt", &name], value)?;
    }
    // Apply the same stable/settings and inheritance contract as preview. In
    // particular, do not claim import success for an unusable native section.
    options(edited.draft_text())?;
    document.replace_draft_text(edited.draft_text())?;
    Ok(count)
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
