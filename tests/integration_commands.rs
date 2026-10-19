//! Runs the copied command text through the native shell and real bundled CLI.
use codestyles::{
    integration::{CommandShell, project_guidance},
    resources,
};
use std::{
    fs,
    process::{Command, Output},
};

fn run(command: &str, cache: &std::path::Path) -> Output {
    let mut process = if cfg!(windows) {
        let mut process = Command::new("powershell.exe");
        process.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        process
    } else {
        let mut process = Command::new("sh");
        process.args(["-c", command]);
        process
    };
    process.env("DPRINT_CACHE_DIR", cache).output().unwrap()
}

#[test]
fn copied_commands_check_format_and_recheck_with_real_bundled_dprint() {
    let resources = resources::resolve_resources().unwrap();
    let backend = resources::backend_from_root(&resources.root).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("Code Styles 格式 O'Brien’s $HOME [x]");
    fs::create_dir(&project).unwrap();
    let executable = project.join(if cfg!(windows) {
        "bundled dprint.exe"
    } else {
        "bundled dprint"
    });
    fs::copy(backend.executable(), &executable).unwrap();
    let source = project.join("source.json");
    let cache = temp.path().join("cache");
    // Without explicit --config this competing standard name would win over
    // dprint.jsonc. Copied commands must use the selected file, just like Project.
    fs::write(
        project.join("dprint.json"),
        "{\"plugins\":[],\"includes\":[]}",
    )
    .unwrap();
    for name in ["dprint.jsonc", "custom O'Brien’s.jsonc"] {
        let config = project.join(name);
        fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({
                "includes": ["source.json"],
                "plugins": [resources.root.join("plugins/json-0.25.2.wasm")],
                "json": {"indentWidth": 2}
            }))
            .unwrap(),
        )
        .unwrap();
        let original = "{\"answer\":42}";
        fs::write(&source, original).unwrap();
        let guide =
            project_guidance(&executable, Some(&config), false, CommandShell::native()).unwrap();
        let check = guide.check_command.unwrap();
        let format = guide.format_command.unwrap();
        let before = run(&check, &cache);
        assert!(
            !before.status.success(),
            "unformatted source must fail check"
        );
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            original,
            "check must not write"
        );
        #[cfg(windows)]
        {
            let restoration_check = format!(
                "$before = (Get-Location).Path; $caught = $false; try {{ {check} }} catch {{ $caught = $true }}; if (!$caught) {{ exit 91 }}; if ((Get-Location).Path -ne $before) {{ exit 92 }}; exit 0"
            );
            let restored = run(&restoration_check, &cache);
            assert!(
                restored.status.success(),
                "failed native command must throw and restore the caller's location"
            );
        }
        let formatted = run(&format, &cache);
        assert!(
            formatted.status.success(),
            "{}",
            String::from_utf8_lossy(&formatted.stderr)
        );
        assert_ne!(fs::read_to_string(&source).unwrap(), original);
        let after = run(&check, &cache);
        assert!(
            after.status.success(),
            "{}",
            String::from_utf8_lossy(&after.stderr)
        );
        fs::remove_file(config).unwrap();
    }
}
