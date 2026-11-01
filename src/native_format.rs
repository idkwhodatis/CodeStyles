//! Explicit, managed native formatters (Go, Dart, Java, C#, C and C++). No SDK discovery, shell, PATH fallback,
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
const CSHARP_RUNTIME_VERSION: &str = "10.0.14";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeLanguage {
    Go,
    Dart,
    Java,
    CSharp,
    C,
    Cpp,
}
impl NativeLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "go" => Some(Self::Go),
            "dart" => Some(Self::Dart),
            "java" => Some(Self::Java),
            "cs" => Some(Self::CSharp),
            "c" | "h" => Some(Self::C),
            "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "C" | "H" => {
                Some(Self::Cpp)
            }
            _ => None,
        }
    }
    pub fn section(self) -> &'static str {
        match self {
            Self::Go => "gofmt",
            Self::Dart => "dartfmt",
            Self::Java => "googleJavaFormat",
            Self::CSharp => "csharpier",
            Self::C | Self::Cpp => "clangFormat",
        }
    }
    pub fn pack_id(self) -> &'static str {
        match self {
            Self::Java => "google-java-format",
            Self::C | Self::Cpp => "clang-format",
            _ => self.section(),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Go => "Go",
            Self::Dart => "Dart",
            Self::Java => "Java",
            Self::CSharp => "C#",
            Self::C => "C",
            Self::Cpp => "C++",
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
    match language {
        NativeLanguage::Java => {
            result
                .entry("style")
                .or_insert(Value::String("google".into()));
        }
        NativeLanguage::CSharp => {
            result.entry("printWidth").or_insert(Value::from(100));
            result.entry("indentSize").or_insert(Value::from(4));
            result.entry("useTabs").or_insert(Value::Bool(false));
            result
                .entry("endOfLine")
                .or_insert(Value::String("auto".into()));
        }
        NativeLanguage::C | NativeLanguage::Cpp => {
            result
                .entry("BasedOnStyle")
                .or_insert(Value::String("LLVM".into()));
        }
        _ => {}
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
        (NativeLanguage::Java, "style") => value
            .as_str()
            .is_some_and(|v| matches!(v, "google" | "aosp")),
        (NativeLanguage::CSharp, "printWidth") => {
            value.as_u64().is_some_and(|v| (1..=10_000).contains(&v))
        }
        (NativeLanguage::CSharp, "indentSize") => {
            value.as_u64().is_some_and(|v| (1..=16).contains(&v))
        }
        (NativeLanguage::CSharp, "useTabs") => value.is_boolean(),
        (NativeLanguage::CSharp, "endOfLine") => value
            .as_str()
            .is_some_and(|v| matches!(v, "auto" | "lf" | "crlf")),
        (NativeLanguage::C | NativeLanguage::Cpp, "BasedOnStyle") => {
            value.as_str().is_some_and(|v| {
                matches!(
                    v,
                    "LLVM" | "Google" | "Chromium" | "Mozilla" | "WebKit" | "Microsoft" | "GNU"
                )
            })
        }
        (NativeLanguage::C | NativeLanguage::Cpp, "IndentWidth") => {
            value.as_u64().is_some_and(|v| (1..=16).contains(&v))
        }
        (NativeLanguage::C | NativeLanguage::Cpp, "ColumnLimit") => {
            value.as_u64().is_some_and(|v| v <= 10_000)
        }
        (NativeLanguage::C | NativeLanguage::Cpp, "UseTab") => value.as_str().is_some_and(|v| {
            matches!(
                v,
                "Never"
                    | "ForIndentation"
                    | "ForContinuationAndIndentation"
                    | "AlignWithSpaces"
                    | "Always"
            )
        }),
        (NativeLanguage::C | NativeLanguage::Cpp, "BreakBeforeBraces") => {
            value.as_str().is_some_and(|v| {
                matches!(
                    v,
                    "Attach"
                        | "Linux"
                        | "Mozilla"
                        | "Stroustrup"
                        | "Allman"
                        | "Whitesmiths"
                        | "GNU"
                        | "WebKit"
                )
            })
        }
        _ => {
            return Err(format!(
                "Unsupported {} setting: {name}. Only explicit supported formatter settings are accepted.",
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
        NativeLanguage::Java => {
            // Formatting must not silently remove or reorder imports.
            let mut args = vec![
                "--skip-sorting-imports".into(),
                "--skip-removing-unused-imports".into(),
            ];
            if settings.get("style").and_then(Value::as_str) == Some("aosp") {
                args.push("--aosp".into());
            }
            args.push("-".into());
            args
        }
        NativeLanguage::CSharp => vec![
            "format".into(),
            "--no-cache".into(),
            "--no-msbuild-check".into(),
            "--log-level".into(),
            "Error".into(),
        ],
        NativeLanguage::C | NativeLanguage::Cpp => vec![
            format!(
                "--assume-filename=sample.{}",
                if language == NativeLanguage::C {
                    "c"
                } else {
                    "cpp"
                }
            ),
            // JSON is also valid YAML flow syntax. Never use style=file or pass a
            // user path: both would enable implicit parent configuration search.
            format!("--style={}", Value::Object(settings.clone())),
            "--sort-includes=false".into(),
            "--Werror".into(),
            "--fail-on-incomplete-format".into(),
        ],
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
                .resolve_cancellable(language.pack_id(), &cancel)
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
    // Preserve absolute-path semantics even with a relative or noncanonical TMPDIR.
    let workspace = directory
        .path()
        .canonicalize()
        .map_err(|e| format!("Cannot resolve isolated native workspace: {e}"))?;
    if language == NativeLanguage::Go {
        // Go telemetry uses os.UserConfigDir, not a GOTELEMETRY env override.
        // Write only private mode data, never alter the user's global preference.
        let config = if cfg!(target_os = "macos") {
            workspace.as_path().join("Library/Application Support")
        } else {
            workspace.as_path().to_path_buf()
        };
        let telemetry = config.join("go/telemetry");
        fs::create_dir_all(&telemetry).map_err(|e| e.to_string())?;
        fs::write(telemetry.join("mode"), "off 2026-10-30").map_err(|e| e.to_string())?;
    }
    if language == NativeLanguage::CSharp {
        fs::write(workspace.as_path().join("codestyles.csharpierignore"), "")
            .map_err(|e| format!("Cannot write isolated C# ignore file: {e}"))?;
        fs::write(
            workspace.as_path().join("codestyles.csharpier.json"),
            serde_json::to_vec(settings).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("Cannot write isolated C# options: {e}"))?;
    }
    let mut command = Command::new(executable);
    command.env_clear().envs(environment.iter().cloned());
    // A managed CSharpier runtime must not probe an installed SDK/runtime,
    // run first-use setup, or transmit .NET CLI telemetry.
    command
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_MULTILEVEL_LOOKUP", "0")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_HOME", workspace.as_path());
    if language == NativeLanguage::CSharp {
        let dotnet_root = executable
            .parent()
            .ok_or("Managed .NET host has no directory.")?;
        let pack_root = dotnet_root
            .parent()
            .ok_or("Managed .NET host has no pack root.")?;
        // This is a pinned runtime host, NOT an SDK, tool shim or caller DLL.
        // Framework roll-forward is disabled even if upstream runtimeconfig
        // requests it. All DLLs and runtime files are verified by the pack lease.
        command
            .env("DOTNET_ROOT", dotnet_root)
            .args([
                "exec",
                "--fx-version",
                CSHARP_RUNTIME_VERSION,
                "--roll-forward",
                "Disable",
            ])
            .arg(pack_root.join("lib/csharpier/CSharpier.dll"));
    }
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
        command.env(key, workspace.as_path());
    }
    command
        .current_dir(workspace.as_path())
        .args(arguments(language, settings));
    if language == NativeLanguage::CSharp {
        command
            .arg("--config-path")
            .arg(workspace.as_path().join("codestyles.csharpier.json"))
            .arg("--ignore-path")
            .arg(workspace.as_path().join("codestyles.csharpierignore"))
            // CSharpier 1.3.0 throws on bare sample.cs: use a private absolute
            // name, never the user's project path or its parent configuration.
            .arg("--stdin-path")
            .arg(workspace.as_path().join("sample.cs"));
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
    let source_has_content = !source.trim().is_empty();
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
    let output =
        String::from_utf8(stdout).map_err(|_| "Native formatter returned invalid UTF-8.")?;
    if source_has_content && output.trim().is_empty() {
        return Err("Native formatter returned empty output for nonempty source (the source may be excluded or unsupported); refusing to replace it.".into());
    }
    Ok(output)
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

#[cfg(test)]
mod additional_language_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_and_ui_versions_match_fixed_pack_research_manifest() {
        let sources: Value =
            serde_json::from_str(include_str!("../resources/extended-formatter-sources.json"))
                .unwrap();
        assert_eq!(
            sources["tools"]["csharpier"]["managed_runtime"]["version"],
            CSHARP_RUNTIME_VERSION
        );
        for (plugin, id) in [
            (
                crate::schema::PinnedPlugin::GoogleJavaFormat,
                "google-java-format",
            ),
            (crate::schema::PinnedPlugin::CSharpier, "csharpier"),
            (crate::schema::PinnedPlugin::ClangFormat, "clang-format"),
        ] {
            assert_eq!(sources["tools"][id]["version"], plugin.version());
        }
    }

    #[test]
    fn explicit_settings_and_pack_ids_do_not_enable_discovery() {
        for language in [
            NativeLanguage::Java,
            NativeLanguage::CSharp,
            NativeLanguage::C,
            NativeLanguage::Cpp,
        ] {
            let settings = options("{}", language).unwrap();
            let args = arguments(language, &settings);
            assert!(!args.iter().any(|a| a == "--replace"
                || a == "-i"
                || a == "--skip-validation"
                || a == "--style=file"));
            for key in [
                "command",
                "executable",
                "args",
                "config",
                "extends",
                "overrides",
            ] {
                assert!(
                    options(
                        &json!({language.section():{key:"outside"}}).to_string(),
                        language
                    )
                    .is_err()
                );
            }
            for text in [
                r#"{"extends":"outside"}"#,
                r#"{"inherit":true}"#,
                r#"{"overrides":[]}"#,
            ] {
                assert!(options(text, language).is_err());
            }
        }
        assert_eq!(NativeLanguage::Java.pack_id(), "google-java-format");
        assert_eq!(NativeLanguage::Cpp.pack_id(), "clang-format");
        assert_eq!(NativeLanguage::CSharp.pack_id(), "csharpier");
        let java = arguments(
            NativeLanguage::Java,
            &options(
                r#"{"googleJavaFormat":{"style":"aosp"}}"#,
                NativeLanguage::Java,
            )
            .unwrap(),
        );
        assert!(java.contains(&"--aosp".into()));
        assert!(java.contains(&"--skip-removing-unused-imports".into()));
        assert!(java.contains(&"--skip-sorting-imports".into()));
        assert_eq!(java.last().unwrap(), "-");
        let csharp = arguments(
            NativeLanguage::CSharp,
            &options("{}", NativeLanguage::CSharp).unwrap(),
        );
        assert!(csharp.contains(&"--no-msbuild-check".into()));
        assert!(csharp.contains(&"--log-level".into()));
        for (language, suffix) in [(NativeLanguage::C, "c"), (NativeLanguage::Cpp, "cpp")] {
            let args = arguments(language, &options("{}", language).unwrap());
            assert!(args.contains(&format!("--assume-filename=sample.{suffix}")));
            assert!(
                args.iter()
                    .any(|a| a.starts_with("--style={") && a.contains("LLVM"))
            );
        }
    }

    #[test]
    fn new_native_settings_are_bounded_and_reject_inheritance() {
        for (language, name, value) in [
            (NativeLanguage::Java, "style", json!("custom")),
            (NativeLanguage::CSharp, "printWidth", json!(0)),
            (NativeLanguage::CSharp, "printWidth", json!(10001)),
            (NativeLanguage::CSharp, "indentSize", json!(17)),
            (NativeLanguage::CSharp, "useTabs", json!("true")),
            (NativeLanguage::CSharp, "endOfLine", json!("native")),
            (
                NativeLanguage::Cpp,
                "BasedOnStyle",
                json!("InheritParentConfig"),
            ),
            (NativeLanguage::C, "IndentWidth", json!(0)),
            (NativeLanguage::Cpp, "ColumnLimit", json!(-1)),
            (NativeLanguage::Cpp, "ColumnLimit", json!(10001)),
            (NativeLanguage::Cpp, "UseTab", json!(true)),
            (NativeLanguage::Cpp, "BreakBeforeBraces", json!("Custom")),
            (NativeLanguage::Cpp, "IncludeCategories", json!([])),
        ] {
            assert!(
                validate_option(language, name, &value).is_err(),
                "{language:?}: {name}"
            );
        }
        assert!(validate_option(NativeLanguage::C, "ColumnLimit", &json!(0)).is_ok());
        assert!(validate_option(NativeLanguage::CSharp, "useTabs", &json!(true)).is_ok());
    }

    #[test]
    fn new_native_backends_report_missing_pack_without_dprint_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ToolchainManager::at_managed_root(dir.path().join("managed")).unwrap();
        let backend = crate::backend::Backend::new(dir.path().join("never-run-dprint"), vec![])
            .with_toolchain_manager(manager);
        let config = crate::config::ConfigDocument::standalone(&[]).unwrap();
        for extension in ["java", "cs", "c", "h", "cpp", "hpp", "C", "H", "c++", "h++"] {
            let error = backend
                .format(
                    &FormatRequest {
                        source: "sample".into(),
                        source_path: dir.path().join(format!("sample.{extension}")),
                        config: config.snapshot(),
                    },
                    Arc::new(AtomicBool::new(false)),
                )
                .unwrap_err();
            assert!(error.contains("Language support"), "{extension}: {error}");
            assert!(!error.contains("start bundled dprint"));
        }
    }

    #[cfg(unix)]
    fn fixture(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-formatter");
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        (dir, path)
    }

    // A protocol fixture, not evidence that a production formatter is available.
    #[test]
    #[cfg(unix)]
    fn isolated_native_process_keeps_settings_private_and_source_unchanged() {
        let (_dir, executable) = fixture(
            r#"test "$1" = exec || exit 8
 test "$2" = --fx-version && test "$3" = 10.0.14 || exit 9
 test "$4" = --roll-forward && test "$5" = Disable || exit 10
 case "$6" in /*/lib/csharpier/CSharpier.dll) ;; *) exit 11 ;; esac
 shift 6
 test "$1" = format || exit 12
 shift
 while test "$#" -gt 0; do
   case "$1" in
     --config-path) test -f "$2" || exit 13; shift ;;
     --ignore-path) test -f "$2" && test ! -s "$2" || exit 14; shift ;;
     --stdin-path) test "$2" = "$HOME/sample.cs" || exit 15; shift ;;
   esac
   shift
 done
 test "$HOME" = "$PWD" || exit 16
 test "$DOTNET_CLI_TELEMETRY_OPTOUT" = 1 || exit 17
 /bin/cat"#,
        );
        let settings =
            options(r#"{"csharpier":{"printWidth":80}}"#, NativeLanguage::CSharp).unwrap();
        let result = format_with_runtime(
            &executable,
            &[],
            NativeLanguage::CSharp,
            "class Sample {}\n",
            &settings,
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(result, "class Sample {}\n");
    }

    #[test]
    #[cfg(unix)]
    fn native_failures_timeouts_and_cancel_never_return_partial_output() {
        for (script, expected, timeout) in [
            (
                "printf partial; printf 'invalid syntax' >&2; exit 2",
                "invalid syntax",
                Duration::from_secs(5),
            ),
            (
                "printf partial; /bin/sleep 10",
                "timed out",
                Duration::from_millis(80),
            ),
            (
                "/bin/cat >/dev/null",
                "empty output",
                Duration::from_secs(5),
            ),
        ] {
            let (_dir, executable) = fixture(script);
            let error = format_with_runtime(
                &executable,
                &[],
                NativeLanguage::Java,
                "class X {}",
                &options("{}", NativeLanguage::Java).unwrap(),
                Arc::new(AtomicBool::new(false)),
                timeout,
            )
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        let (_dir, executable) = fixture("printf 'must not run'");
        let error = format_with_runtime(
            &executable,
            &[],
            NativeLanguage::Cpp,
            "int x;",
            &options("{}", NativeLanguage::Cpp).unwrap(),
            Arc::new(AtomicBool::new(true)),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.contains("cancelled"));
    }
}
