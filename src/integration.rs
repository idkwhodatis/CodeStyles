//! Read-only editor setup guidance. Callers supply the already-verified bundled
//! executable; generating a guide never discovers, launches, or changes anything.
use std::path::Path;

pub const DPRINT_VERSION: &str = "0.63.0";
pub const EXTENSION_ID: &str = "dprint.dprint";
pub const EXTENSION_URL: &str = "https://marketplace.visualstudio.com/items?itemName=dprint.dprint";
pub const EXTENSION_SETTINGS_URL: &str =
    "https://github.com/dprint/dprint-vscode#extension-settings";
pub const VSCODE_SETTINGS_URL: &str = "https://code.visualstudio.com/docs/configure/settings";
pub const CI_URL: &str = "https://github.com/dprint/check#specific-version";
pub const RELEASE_URL: &str = "https://github.com/dprint/dprint/releases/tag/0.63.0";

/// Portable step for an existing workflow, after checkout. No installation path
/// or machine-local plugin references belong in this snippet.
pub const CI_STEP: &str = "- uses: dprint/check@v2\n  with:\n    dprint-version: \"0.63.0\"\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandShell {
    Posix,
    PowerShell,
}

impl CommandShell {
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::PowerShell
        } else {
            Self::Posix
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Posix => "POSIX shell (sh/bash/zsh)",
            Self::PowerShell => "PowerShell (not Command Prompt)",
        }
    }

    fn quote(self, value: &str) -> String {
        let escaped = match self {
            Self::Posix => value.replace('\'', "'\"'\"'"),
            // PowerShell also treats typographic single quotes as delimiters.
            Self::PowerShell => value
                .chars()
                .flat_map(|ch| {
                    let quote =
                        matches!(ch, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}');
                    std::iter::once(ch).chain(quote.then_some(ch))
                })
                .collect(),
        };
        format!("'{escaped}'")
    }

    fn command(
        self,
        executable: &str,
        root: &str,
        action: &str,
        custom_config: Option<&str>,
    ) -> String {
        let executable = self.quote(executable);
        let root = self.quote(root);
        let config = custom_config
            .map(|path| format!(" --config {}", self.quote(path)))
            .unwrap_or_default();
        match self {
            Self::Posix => format!("(cd {root} && {executable} {action}{config})"),
            Self::PowerShell => format!(
                "& {{ $ErrorActionPreference = 'Stop'; Push-Location -LiteralPath {root}; try {{ & {executable} {action}{config}; if ($LASTEXITCODE -ne 0) {{ throw \"dprint exited with code $LASTEXITCODE\" }} }} finally {{ Pop-Location }} }}"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectIntegration {
    /// A discoverable filename, not a claim that the extension is installed or
    /// that its effective configuration has been checked.
    pub discoverable: bool,
    pub status: String,
    pub check_command: Option<String>,
    pub format_command: Option<String>,
}

/// Generate commands for an already-verified bundled console dprint and the
/// current saved configuration. This function performs no I/O. The caller must
/// include pending/invalid raw edits in `dirty`; draft changes are never used by
/// an external editor, shell command, or CI job.
pub fn project_guidance(
    executable: &Path,
    config: Option<&Path>,
    dirty: bool,
    shell: CommandShell,
) -> Result<ProjectIntegration, String> {
    let executable = command_path(executable)?;
    let mut result = ProjectIntegration {
        discoverable: false,
        status: "Save a project configuration before copying project commands. Editor and CI use saved settings.".into(),
        check_command: None,
        format_command: None,
    };
    let Some(config) = config else {
        return Ok(result);
    };
    let config_text = command_path(config)?;
    let name = config
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Configuration path must name a file.")?;
    result.discoverable = matches!(
        name,
        "dprint.json" | "dprint.jsonc" | ".dprint.json" | ".dprint.jsonc"
    );
    if dirty {
        result.status = "Save or discard configuration edits first. Copied commands, VS Code and CI cannot use the unsaved draft.".into();
        return Ok(result);
    }
    let root = command_path(
        config
            .parent()
            .ok_or("Configuration needs a project directory.")?,
    )?;
    result.status = if result.discoverable {
        "Discoverable configuration name. Commands run from its directory with this saved config explicitly selected; nested discovery stays enabled. Keep only one root config for editor/CI discovery; workspace overrides may affect VS Code. Format writes source files; review the Project check results first.".into()
    } else {
        "Custom configuration name: CLI commands use --config explicitly. VS Code and the generic CI step cannot discover this file. Save under a standard dprint config name for shared integration, or customize CI's config-path. Nested discovery stays enabled. Format writes source files.".into()
    };
    let custom = Some(config_text);
    result.check_command = Some(shell.command(executable, root, "check", custom));
    result.format_command = Some(shell.command(executable, root, "fmt", custom));
    Ok(result)
}

fn command_path(path: &Path) -> Result<&str, String> {
    if !path.is_absolute() {
        return Err("Project commands require absolute executable and configuration paths.".into());
    }
    let text = path
        .to_str()
        .ok_or("Project command paths must be valid Unicode.")?;
    if text.chars().any(char::is_control) {
        return Err("Project command paths cannot contain control characters.".into());
    }
    Ok(text)
}

pub const STABLE_FORMAT_EXPLANATION: &str = "Set dprint.ensureStableFormat to true to repeat formatting until the output stops changing, like dprint fmt and CodeStyles preview. The extension defaults to false; true requires dprint 0.59 or newer and may take longer. This option does not pin the executable or plugin versions.";

pub const EDITOR_STEPS: [&str; 5] = [
    "Install the official dprint.dprint extension in VS Code. CodeStyles does not install or configure the editor for you.",
    "Save the project configuration as dprint.json, dprint.jsonc, .dprint.json or .dprint.jsonc in the project. The extension discovers project configuration; it does not use CodeStyles' unsaved draft or support arbitrary custom config locations.",
    "Open Preferences: Open User Settings (JSON). Merge the supplied entries into existing settings. Keep dprint.path in your user settings: this absolute executable path belongs to this installation, not to the team's repository. Preserve other existing settings.",
    "The snippet selects dprint for JavaScript, JSX, TypeScript, TSX, JSON, JSONC, Markdown and TOML. Format on save is optional; add editor.formatOnSave: true to the desired language settings only if you want it.",
    "Run Dprint: Restart after changing the executable path, then Format Document on a project file. If results differ, check workspace overrides, the saved config, plugin versions and the source path. Move or remove CodeStyles and this path may need updating. Remote editors need a matching executable on their own host.",
];

pub const CI_INSTRUCTIONS: &str = "Use dprint 0.63.0 in CI and the same committed configuration and exact plugin versions as the editor. For new CodeStyles configurations those plugins are TypeScript 0.96.1, JSON 0.25.2, Markdown 0.26.0, TOML 0.9.0 and Ruff 0.11.3 (Python). Imported configurations retain their original plugin references.\n\nAfter your existing repository checkout, an official GitHub Actions step is:\n- uses: dprint/check@v2\n  with:\n    dprint-version: \"0.63.0\"\n\nThe action otherwise selects the latest CLI. Pin the action itself according to your repository's policy. For other CI systems, install the matching 0.63.0 release for the runner and run dprint --version followed by dprint check from the project root. A custom CLI config path can be specified with --config, but the editor needs a discoverable config name. Do not copy this machine's absolute dprint.path into CI.";

pub const OFFLINE_NOTE: &str = "Selecting the bundled executable does not make editor or CI formatting automatically offline. CodeStyles alone remaps its exact pinned plugin URLs to bundled WASM for preview. VS Code and CI resolve the saved config normally and may download plugins or inherited configs. Only trust configurations and process plugins you intend to run.";

pub const PATH_NOTE: &str = "The settings snippet escapes the path as JSON; Copy path gives the raw path. The extension supports path expansion, so prefer installation paths without expansion syntax such as $HOME or ${HOME}, and check the resolved executable if your path contains it.";

pub const LANGUAGES: [&str; 9] = [
    "javascript",
    "javascriptreact",
    "typescript",
    "typescriptreact",
    "json",
    "jsonc",
    "markdown",
    "toml",
    "python",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrationGuide {
    /// Exact native path rendered as Unicode, for a copy-path action. It is not
    /// shell-quoted. The JSON field below is escaped separately.
    pub executable_path: String,
    pub settings_json: String,
    pub editor_steps: String,
    pub ci_instructions: String,
}

/// Build user-level VS Code settings for an absolute, already-verified bundled
/// executable. No filesystem access or editor mutation occurs here. Relative and
/// non-Unicode paths fail rather than yielding an ambiguous or lossy snippet.
pub fn guidance(executable: &Path) -> Result<IntegrationGuide, String> {
    if !executable.is_absolute() {
        return Err("Editor integration needs the absolute bundled dprint executable path.".into());
    }
    let executable_path = executable.to_str().ok_or(
        "The bundled executable path cannot be represented in VS Code's Unicode JSON settings. Move CodeStyles to a Unicode path before configuring the editor.",
    )?;
    Ok(IntegrationGuide {
        executable_path: executable_path.to_owned(),
        settings_json: settings_json(executable_path)?,
        editor_steps: EDITOR_STEPS
            .iter()
            .enumerate()
            .map(|(index, step)| format!("{}. {step}", index + 1))
            .chain([
                STABLE_FORMAT_EXPLANATION.to_owned(),
                OFFLINE_NOTE.to_owned(),
                PATH_NOTE.to_owned(),
                format!("Extension: {EXTENSION_URL}\nExtension settings: {EXTENSION_SETTINGS_URL}\nVS Code settings: {VSCODE_SETTINGS_URL}"),
            ])
            .collect::<Vec<_>>()
            .join("\n\n"),
        ci_instructions: format!("{CI_INSTRUCTIONS}\n\nOfficial action: {CI_URL}\nPinned release: {RELEASE_URL}"),
    })
}

fn settings_json(executable_path: &str) -> Result<String, String> {
    let mut settings = serde_json::json!({
        "dprint.path": executable_path,
        "dprint.ensureStableFormat": true,
    });
    for language in LANGUAGES {
        settings[format!("[{language}]")] = serde_json::json!({
            "editor.defaultFormatter": EXTENSION_ID,
        });
    }
    serde_json::to_string_pretty(&settings)
        .map_err(|error| format!("Cannot build editor settings JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn portable_ci_step_pins_cli_without_local_paths() {
        assert_eq!(
            CI_STEP,
            "- uses: dprint/check@v2\n  with:\n    dprint-version: \"0.63.0\"\n"
        );
        assert!(CI_INSTRUCTIONS.contains(CI_STEP.trim_end()));
    }

    #[test]
    fn project_readiness_distinguishes_unsaved_dirty_and_custom_names() {
        let executable = absolute_path();
        let root = executable.parent().unwrap();
        let missing = project_guidance(&executable, None, false, CommandShell::native()).unwrap();
        assert!(!missing.discoverable);
        assert!(missing.check_command.is_none());
        for name in [
            "dprint.json",
            "dprint.jsonc",
            ".dprint.json",
            ".dprint.jsonc",
        ] {
            let config = root.join(name);
            let guide = project_guidance(&executable, Some(&config), false, CommandShell::native())
                .unwrap();
            assert!(guide.discoverable);
            assert!(guide.check_command.unwrap().contains("--config"));
            assert!(guide.format_command.unwrap().contains(" fmt"));
            let dirty =
                project_guidance(&executable, Some(&config), true, CommandShell::native()).unwrap();
            assert!(dirty.discoverable);
            assert!(dirty.check_command.is_none() && dirty.format_command.is_none());
        }
        let custom = root.join("my style.jsonc");
        let guide =
            project_guidance(&executable, Some(&custom), false, CommandShell::native()).unwrap();
        assert!(!guide.discoverable);
        assert!(guide.status.contains("cannot discover"));
        assert!(guide.check_command.unwrap().contains("--config"));
    }

    #[test]
    fn project_commands_reject_ambiguous_paths_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("dprint");
        for config in [
            PathBuf::from("dprint.json"),
            directory.path().join("bad\nconfig.json"),
            directory.path().join("bad\0config.json"),
        ] {
            assert!(
                project_guidance(&executable, Some(&config), false, CommandShell::native())
                    .is_err()
            );
        }
        assert!(
            project_guidance(Path::new("dprint"), None, false, CommandShell::native()).is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn shell_quoting_keeps_windows_paths_literal() {
        let shell = CommandShell::PowerShell;
        assert_eq!(
            shell.quote(r"C:\Program Files\格式\O'Brien\$HOME`[x]\dprint.exe"),
            r"'C:\Program Files\格式\O''Brien\$HOME`[x]\dprint.exe'"
        );
        assert_eq!(shell.quote("‘a’‚b‛"), "'‘‘a’’‚‚b‛‛'");
        let command = shell.command(
            r"C:\Code Styles\dprint.exe",
            r"C:\projects\[x]",
            "check",
            None,
        );
        assert!(command.contains("Push-Location -LiteralPath 'C:\\projects\\[x]'"));
        assert!(command.contains("& 'C:\\Code Styles\\dprint.exe' check"));
        assert!(command.contains("finally { Pop-Location }"));
        assert!(!command.contains("cmd.exe"));
    }

    #[cfg(unix)]
    #[test]
    fn posix_commands_execute_literal_paths_arguments_and_preserve_exit_status() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("格式 ' \" $HOME ` echo ; [x]");
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("dprint ' \" $HOME");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$@\"\nexit 7\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        for name in ["dprint.jsonc", "style ' \" $HOME.jsonc"] {
            let config = root.join(name);
            let guide =
                project_guidance(&executable, Some(&config), false, CommandShell::Posix).unwrap();
            for (command, action) in [
                (guide.check_command.unwrap(), "check"),
                (guide.format_command.unwrap(), "fmt"),
            ] {
                let output = std::process::Command::new("sh")
                    .args(["-c", &command])
                    .output()
                    .unwrap();
                assert_eq!(output.status.code(), Some(7));
                let mut expected = format!("{}\n{action}\n", root.display());
                expected.push_str(&format!("--config\n{}\n", config.display()));
                assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
                assert!(output.stderr.is_empty());
            }
        }
        let command = CommandShell::Posix.command(
            executable.to_str().unwrap(),
            root.join("missing").to_str().unwrap(),
            "fmt",
            None,
        );
        let output = std::process::Command::new("sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            output.stdout.is_empty(),
            "must not format after a failed directory change"
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_literal_quoting_round_trips_in_real_parser() {
        let value = r"C:\Program Files\格式\O'Brien’s\$HOME`[x]\dprint.exe";
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new(); [Console]::Write({})",
            CommandShell::PowerShell.quote(value)
        );
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap(), value);
    }

    fn absolute_path() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\CodeStyles\resources\dprint.exe")
        } else {
            PathBuf::from("/opt/CodeStyles/resources/dprint")
        }
    }

    #[test]
    fn recommends_verified_extension_and_cli_parity_setting() {
        let guide = guidance(&absolute_path()).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&guide.settings_json).unwrap();
        assert_eq!(settings["dprint.path"], guide.executable_path);
        assert_eq!(settings["dprint.ensureStableFormat"], true);
        for language in LANGUAGES {
            assert_eq!(
                settings[format!("[{language}]")]["editor.defaultFormatter"],
                EXTENSION_ID
            );
        }
        assert!(settings.get("editor.defaultFormatter").is_none());
        assert!(!guide.settings_json.contains("editor.formatOnSave"));
        assert_eq!(settings.as_object().unwrap().len(), LANGUAGES.len() + 2);
    }

    #[test]
    fn json_paths_round_trip_without_interpolation_or_loss() {
        // Serialization is platform independent, so Windows escaping is tested
        // on Linux too. Absolute-path validation itself uses the native OS.
        for path in [
            r"C:\Program Files\CodeStyles\dprint.exe",
            r"\\server\share\CodeStyles\dprint.exe",
            r"\\?\C:\CodeStyles\dprint.exe",
            "/opt/a\"quoted\\name/格式/dprint",
            "/opt/a\nline\tbreak/dprint",
            "/opt/${literal}/dprint",
        ] {
            let json = settings_json(path).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed["dprint.path"].as_str(), Some(path));
        }
    }

    #[test]
    fn relative_empty_and_parent_relative_paths_are_rejected() {
        for path in ["", "dprint", "resources/dprint", "../bin/dprint"] {
            assert!(guidance(Path::new(path)).is_err(), "accepted {path:?}");
        }
    }

    #[test]
    fn guide_generation_does_not_require_or_create_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("not-created").join("dprint");
        let guide = guidance(&absent).unwrap();
        assert_eq!(guide.executable_path, absent.to_str().unwrap());
        assert!(!absent.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_paths_fail_instead_of_being_lossily_copied() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_vec(b"/opt/\xff/dprint".to_vec()));
        assert!(guidance(&path).unwrap_err().contains("Unicode"));
    }

    #[test]
    fn instructions_keep_machine_paths_local_and_versions_consistent() {
        let guide = guidance(&absolute_path()).unwrap();
        assert!(guide.editor_steps.contains("user settings"));
        assert!(!guide.ci_instructions.contains(&guide.executable_path));
        assert!(guide.ci_instructions.contains("dprint-version: \"0.63.0\""));
        assert!(STABLE_FORMAT_EXPLANATION.contains("defaults to false"));
        assert!(OFFLINE_NOTE.contains("may download"));
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../resources/manifest.json")).unwrap();
        assert_eq!(manifest["dprint_version"], DPRINT_VERSION);
        for plugin in ["typescript", "json", "markdown", "toml", "ruff"] {
            assert!(CI_INSTRUCTIONS.contains(manifest["plugins"][plugin].as_str().unwrap()));
        }
    }
}
