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

pub const STABLE_FORMAT_EXPLANATION: &str = "Set dprint.ensureStableFormat to true to repeat formatting until the output stops changing, like dprint fmt and CodeStyles preview. The extension defaults to false; true requires dprint 0.59 or newer and may take longer. This option does not pin the executable or plugin versions.";

pub const EDITOR_STEPS: [&str; 5] = [
    "Install the official dprint.dprint extension in VS Code. CodeStyles does not install or configure the editor for you.",
    "Save the project configuration as dprint.json, dprint.jsonc, .dprint.json or .dprint.jsonc in the project. The extension discovers project configuration; it does not use CodeStyles' unsaved draft or support arbitrary custom config locations.",
    "Open Preferences: Open User Settings (JSON). Merge the supplied entries into existing settings. Keep dprint.path in your user settings: this absolute executable path belongs to this installation, not to the team's repository. Preserve other existing settings.",
    "The snippet selects dprint for JavaScript, JSX, TypeScript, TSX, JSON, JSONC, Markdown and TOML. Format on save is optional; add editor.formatOnSave: true to the desired language settings only if you want it.",
    "Run Dprint: Restart after changing the executable path, then Format Document on a project file. If results differ, check workspace overrides, the saved config, plugin versions and the source path. Move or remove CodeStyles and this path may need updating. Remote editors need a matching executable on their own host.",
];

pub const CI_INSTRUCTIONS: &str = "Use dprint 0.63.0 in CI and the same committed configuration and exact plugin versions as the editor. For new CodeStyles configurations those plugins are TypeScript 0.96.1, JSON 0.25.2, Markdown 0.26.0 and TOML 0.9.0. Imported configurations retain their original plugin references.\n\nAfter your existing repository checkout, an official GitHub Actions step is:\n- uses: dprint/check@v2\n  with:\n    dprint-version: \"0.63.0\"\n\nThe action otherwise selects the latest CLI. Pin the action itself according to your repository's policy. For other CI systems, install the matching 0.63.0 release for the runner and run dprint --version followed by dprint check from the project root. A custom CLI config path can be specified with --config, but the editor needs a discoverable config name. Do not copy this machine's absolute dprint.path into CI.";

pub const OFFLINE_NOTE: &str = "Selecting the bundled executable does not make editor or CI formatting automatically offline. CodeStyles alone remaps its exact pinned plugin URLs to bundled WASM for preview. VS Code and CI resolve the saved config normally and may download plugins or inherited configs. Only trust configurations and process plugins you intend to run.";

pub const PATH_NOTE: &str = "The settings snippet escapes the path as JSON; Copy path gives the raw path. The extension supports path expansion, so prefer installation paths without expansion syntax such as $HOME or ${HOME}, and check the resolved executable if your path contains it.";

pub const LANGUAGES: [&str; 8] = [
    "javascript",
    "javascriptreact",
    "typescript",
    "typescriptreact",
    "json",
    "jsonc",
    "markdown",
    "toml",
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
        for plugin in ["typescript", "json", "markdown", "toml"] {
            assert!(CI_INSTRUCTIONS.contains(manifest["plugins"][plugin].as_str().unwrap()));
        }
    }
}
