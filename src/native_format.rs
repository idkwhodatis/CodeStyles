//! Fixed, managed Go/Dart formatters. No SDK discovery, shell, PATH fallback,
//! package install, project config search, or implicit language-pack download.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeLanguage {
    Go,
    Dart,
}
impl NativeLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "go" => Some(Self::Go),
            "dart" => Some(Self::Dart),
            _ => None,
        }
    }
    pub fn section(self) -> &'static str {
        match self {
            Self::Go => "gofmt",
            Self::Dart => "dartfmt",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Go => "Go",
            Self::Dart => "Dart",
        }
    }
}

pub fn options(config: &str, language: NativeLanguage) -> Result<Map<String, Value>, String> {
    let value = jsonc_parser::parse_to_serde_value(config, &jsonc_parser::ParseOptions::default())
        .map_err(|e| format!("Invalid native formatting configuration: {e}"))?
        .ok_or("Empty native formatting configuration.")?;
    if !value.is_object() {
        return Err("Native formatting configuration must be a JSONC object.".into());
    }
    if value.get("extends").is_some()
        || value.get("overrides").is_some()
        || value
            .get("inherit")
            .is_some_and(|v| v != &Value::Bool(false))
    {
        return Err("Native formatting does not resolve dprint extends, inherit or overrides. Use explicit local formatter settings.".into());
    }
    let mut result = match value.get(language.section()) {
        None => Map::new(),
        Some(Value::Object(settings)) => settings.clone(),
        Some(_) => {
            return Err(format!(
                "The {} section must be an object.",
                language.section()
            ));
        }
    };
    for (name, value) in &result {
        validate_option(language, name, value)?;
    }
    if language == NativeLanguage::Dart {
        result
            .entry("languageVersion")
            .or_insert(Value::String("3.13".into()));
        result.entry("pageWidth").or_insert(Value::from(80));
        result
            .entry("trailingCommas")
            .or_insert(Value::String("automate".into()));
    }
    if language == NativeLanguage::Dart
        && result["trailingCommas"] == "preserve"
        && result["languageVersion"]
            .as_str()
            .unwrap()
            .split_once('.')
            .unwrap()
            .1
            .parse::<u32>()
            .unwrap()
            < 8
    {
        return Err("Dart trailingCommas=preserve requires languageVersion 3.8 or later.".into());
    }
    Ok(result)
}

pub fn validate_option(language: NativeLanguage, name: &str, value: &Value) -> Result<(), String> {
    let valid = match (language, name) {
        (NativeLanguage::Dart, "languageVersion") => value
            .as_str()
            .is_some_and(|v| (0..=13).any(|minor| v == format!("3.{minor}"))),
        (NativeLanguage::Dart, "pageWidth") => {
            value.as_u64().is_some_and(|v| (1..=10_000).contains(&v))
        }
        (NativeLanguage::Dart, "trailingCommas") => value
            .as_str()
            .is_some_and(|v| matches!(v, "automate" | "preserve")),
        _ => {
            return Err(format!(
                "Unsupported {} setting: {name}. Go uses the fixed official gofmt style.",
                language.label()
            ));
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid {} value for {name}.", language.label()))
    }
}

fn arguments(language: NativeLanguage, settings: &Map<String, Value>) -> Vec<String> {
    match language {
        NativeLanguage::Go => vec![],
        NativeLanguage::Dart => vec![
            "--suppress-analytics".into(),
            "format".into(),
            "--output=show".into(),
            "--summary=none".into(),
            "--show=none".into(),
            format!(
                "--language-version={}",
                settings["languageVersion"].as_str().unwrap()
            ),
            format!("--page-width={}", settings["pageWidth"].as_u64().unwrap()),
            format!(
                "--trailing-commas={}",
                settings["trailingCommas"].as_str().unwrap()
            ),
        ],
    }
}

#[derive(Clone, Default)]
pub(crate) struct NativeRuntime(Arc<Mutex<Option<ToolchainManager>>>);
impl std::fmt::Debug for NativeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeRuntime(managed)")
    }
}
impl NativeRuntime {
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
        let language = NativeLanguage::from_path(&request.source_path)
            .ok_or("Unsupported native formatter extension.")?;
        if request.source.len() > OUTPUT_LIMIT {
            return Err("Native source exceeds the 32 MiB safety limit.".into());
        }
        let settings = options(request.config.source_text(), language)?;
        let runtime = {
            let mut manager = loop {
                if cancel.load(Ordering::Acquire) {
                    return Err("Native formatting cancelled.".into());
                }
                if started.elapsed() >= timeout {
                    return Err("Native formatting timed out waiting for managed runtime.".into());
                }
                match self.0.try_lock() {
                    Ok(manager) => break manager,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => return Err("Native runtime manager is unavailable.".into()),
                }
            };
            if manager.is_none() {
                *manager = Some(ToolchainManager::discover()?);
            }
            manager
                .as_ref()
                .unwrap()
                .resolve_cancellable(language.section(), &cancel)
                .map_err(|e| {
                    format!(
                        "Install {} support from Language support before formatting. {e}",
                        language.label()
                    )
                })?
        };
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|v| !v.is_zero())
            .ok_or("Native formatting timed out verifying managed runtime.")?;
        // The runtime lease remains alive until the child and every pipe close.
        format_with_runtime(
            &runtime.executable,
            &runtime.environment,
            language,
            &request.source,
            &settings,
            cancel,
            remaining,
        )
    }
}

fn format_with_runtime(
    executable: &Path,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
    language: NativeLanguage,
    source: &str,
    settings: &Map<String, Value>,
    cancel: Arc<AtomicBool>,
    timeout: Duration,
) -> Result<String, String> {
    let directory =
        tempfile::tempdir().map_err(|e| format!("Cannot create isolated native workspace: {e}"))?;
    if language == NativeLanguage::Go {
        // Go telemetry uses os.UserConfigDir, not a GOTELEMETRY env override.
        // Write only private mode data, never alter the user's global preference.
        let config = if cfg!(target_os = "macos") {
            directory.path().join("Library/Application Support")
        } else {
            directory.path().to_path_buf()
        };
        let telemetry = config.join("go/telemetry");
        fs::create_dir_all(&telemetry).map_err(|e| e.to_string())?;
        fs::write(telemetry.join("mode"), "off 2026-10-30").map_err(|e| e.to_string())?;
    }
    let mut command = Command::new(executable);
    command.env_clear().envs(environment.iter().cloned());
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    for key in [
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "TMPDIR",
        "TMP",
        "TEMP",
    ] {
        command.env(key, directory.path());
    }
    command
        .current_dir(directory.path())
        .args(arguments(language, settings))
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
        return Err("Native formatting cancelled.".into());
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start verified {} formatter: {e}", language.label()))?;
    #[cfg(windows)]
    let process_tree = crate::project::WindowsProcessTree::attach_and_resume(&mut child, &cancel)?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let errors = child.stderr.take().unwrap();
    let source = source.to_owned();
    let writer = thread::spawn(move || input.write_all(source.as_bytes()));
    let reader = thread::spawn(move || bounded_read(output));
    let error_reader = thread::spawn(move || bounded_read(errors));
    let started = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Acquire) {
            break Err("Native formatting cancelled.".to_owned());
        }
        if started.elapsed() >= timeout {
            break Err("Native formatting timed out.".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(e) => break Err(format!("Cannot wait for native formatter: {e}")),
        }
    };
    // Kill descendants even after a successful launcher exit before joining pipes.
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
    let written = writer.join().map_err(|_| "Native input writer failed.")?;
    let stdout = reader
        .join()
        .map_err(|_| "Native output reader failed.")??;
    let stderr = error_reader
        .join()
        .map_err(|_| "Native diagnostic reader failed.")??;
    let status = status?;
    if !status.success() || !stderr.is_empty() {
        let text = String::from_utf8_lossy(&stderr);
        let diagnostic: String = text.chars().take(8192).collect();
        return Err(format!(
            "{} formatter {status}: {}",
            language.label(),
            diagnostic.trim()
        ));
    }
    written.map_err(|e| format!("Cannot send source to native formatter: {e}"))?;
    if cancel.load(Ordering::Acquire) {
        return Err("Native formatting cancelled.".into());
    }
    String::from_utf8(stdout).map_err(|_| "Native formatter returned invalid UTF-8.".into())
}

fn bounded_read(mut input: impl Read) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        let remaining = OUTPUT_LIMIT.saturating_sub(result.len());
        result.extend_from_slice(&buffer[..count.min(remaining)]);
        overflow |= count > remaining;
    }
    if overflow {
        Err("Native formatter output exceeds the 32 MiB safety limit.".into())
    } else {
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn go_style_is_fixed_and_arbitrary_commands_are_rejected() {
        assert!(options("{}", NativeLanguage::Go).unwrap().is_empty());
        for key in ["simplify", "rewrite", "executable", "command", "args"] {
            assert!(options(&json!({"gofmt":{key:true}}).to_string(), NativeLanguage::Go).is_err());
        }
        assert!(arguments(NativeLanguage::Go, &Map::new()).is_empty());
    }
    #[test]
    fn dart_options_are_explicit_bounded_and_have_no_discovery_path() {
        let settings = options("{}", NativeLanguage::Dart).unwrap();
        let args = arguments(NativeLanguage::Dart, &settings);
        assert!(args.contains(&"--suppress-analytics".to_owned()));
        assert!(args.contains(&"--language-version=3.13".to_owned()));
        assert!(!args.iter().any(|a| a.contains("stdin-name")));
        for (name, value) in [
            ("languageVersion", json!("latest")),
            ("languageVersion", json!("3.14")),
            ("languageVersion", json!("3.013")),
            ("pageWidth", json!(0)),
            ("pageWidth", json!(10001)),
            ("pageWidth", json!(1.5)),
            ("trailingCommas", json!("unsafe")),
            ("output", json!("write")),
        ] {
            assert!(validate_option(NativeLanguage::Dart, name, &value).is_err());
        }
        assert!(validate_option(NativeLanguage::Dart, "languageVersion", &json!("3.0")).is_ok());
    }
    #[test]
    fn inherited_and_wrong_typed_native_configuration_fails_closed() {
        for input in [
            "[]",
            "",
            r#"{"extends":"outside"}"#,
            r#"{"inherit":true}"#,
            r#"{"overrides":[]}"#,
            r#"{"dartfmt":[]}"#,
        ] {
            assert!(options(input, NativeLanguage::Dart).is_err());
        }
        assert!(
            options(
                r#"{/*keep*/"inherit":false,"dartfmt":{"pageWidth":100}}"#,
                NativeLanguage::Dart
            )
            .is_ok()
        );
    }
    #[test]
    fn unsupported_legacy_trailing_comma_combination_is_not_silently_ignored() {
        assert!(
            options(
                r#"{"dartfmt":{"languageVersion":"3.7","trailingCommas":"preserve"}}"#,
                NativeLanguage::Dart
            )
            .is_err()
        );
        assert!(
            options(
                r#"{"dartfmt":{"languageVersion":"3.8","trailingCommas":"preserve"}}"#,
                NativeLanguage::Dart
            )
            .is_ok()
        );
        assert!(
            options(
                r#"{"dartfmt":{"languageVersion":"3.6","pageWidth":20}}"#,
                NativeLanguage::Dart
            )
            .is_ok()
        );
    }
    #[test]
    fn only_exact_supported_extensions_route_native() {
        assert_eq!(
            NativeLanguage::from_path(Path::new("a.go")),
            Some(NativeLanguage::Go)
        );
        assert_eq!(
            NativeLanguage::from_path(Path::new("a.dart")),
            Some(NativeLanguage::Dart)
        );
        assert_eq!(NativeLanguage::from_path(Path::new("a.rs")), None);
    }
}
