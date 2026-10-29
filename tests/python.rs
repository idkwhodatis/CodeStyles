//! Real pinned Ruff WASM integration, including reviewed project writes.
//! These tests require the same checksummed bundle shipped with the application.
use codestyles::{
    backend::{Backend, FormatRequest},
    config::ConfigDocument,
    project::{ProjectAction, ProjectRequest},
    resources,
    schema::PinnedPlugin,
};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
};

fn backend() -> Backend {
    resources::backend_from_root(&Path::new(env!("CARGO_MANIFEST_DIR")).join("resources"))
        .expect("verified native dprint and Ruff WASM must be bundled")
}

fn cancellation(value: bool) -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(value))
}

fn plugin_path(plugin: PinnedPlugin) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/plugins")
        .join(plugin.filename())
}

fn project_config(root: &Path) -> ConfigDocument {
    let path = root.join("dprint.jsonc");
    fs::write(
        &path,
        json!({
            "plugins": [plugin_path(PinnedPlugin::Ruff), plugin_path(PinnedPlugin::TypeScript)],
            "includes": ["**/*.{py,pyi,ts}"],
            "excludes": ["**/excluded.py"],
            "ruff": {"quoteStyle": "single", "lineEnding": "lf"}
        })
        .to_string(),
    )
    .unwrap();
    ConfigDocument::load(&path).unwrap()
}

fn preview(backend: &Backend, document: &ConfigDocument, source: &str, path: &Path) -> String {
    backend
        .format(
            &FormatRequest {
                source: source.into(),
                source_path: path.to_owned(),
                config: document.snapshot(),
            },
            cancellation(false),
        )
        .unwrap()
}

fn direct_cli(backend: &Backend, config: &Path, source: &str, path: &Path) -> String {
    let mut stdin_path = std::ffi::OsString::from("--stdin=");
    stdin_path.push(path);
    let mut child = Command::new(backend.executable())
        .args(["fmt", "--config-discovery=false", "--config"])
        .arg(config)
        .arg(stdin_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn python_and_stubs_preview_match_saved_cli_without_mutating_inputs() {
    let backend = backend();
    let dir = tempfile::tempdir().unwrap();
    let mut document = backend.standalone_config().unwrap();
    document
        .set(&["ruff", "quoteStyle"], json!("single"))
        .unwrap();
    document.set(&["ruff", "lineLength"], json!(40)).unwrap();
    // The saved CLI config uses a local verified plugin so the test is offline;
    // the application preview maps its portable pinned URL to the same WASM.
    let saved = dir.path().join("saved.jsonc");
    fs::write(
        &saved,
        json!({"plugins":[plugin_path(PinnedPlugin::Ruff)],
        "ruff":{"quoteStyle":"single","lineLength":40}})
        .to_string(),
    )
    .unwrap();
    let saved_before = fs::read(&saved).unwrap();
    for (filename, source) in [
        (
            "中文 sample.py",
            "def greet(name:str):\n return {\"message\":\"hello\",\"name\":name,\"items\":[1,2,3]}\n",
        ),
        (
            "--sample.pyi",
            "def greet(name:str, greeting:str=\"hello\")->str: ...\n",
        ),
    ] {
        let path = dir.path().join(filename);
        fs::write(&path, source).unwrap();
        let actual = preview(&backend, &document, source, &path);
        assert_eq!(actual, direct_cli(&backend, &saved, source, &path));
        assert_ne!(actual, source);
        assert!(actual.contains("'hello'"), "{actual}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
    assert_eq!(fs::read(saved).unwrap(), saved_before);
}

#[test]
fn python_settings_round_trip_quotes_width_indent_and_line_endings_without_lint() {
    let backend = backend();
    let mut document = backend.standalone_config().unwrap();
    let source = "import sys\nimport os\n\ndef greet(name):\n return {\"message\":\"hello\",\"name\":name,\"values\":[1,2,3,4]}\n";
    let path = Path::new("sample.py");
    document
        .set(&["ruff", "quoteStyle"], json!("double"))
        .unwrap();
    document.set(&["ruff", "lineLength"], json!(120)).unwrap();
    let baseline = preview(&backend, &document, source, path);
    document
        .set(&["ruff", "quoteStyle"], json!("single"))
        .unwrap();
    document.set(&["ruff", "lineLength"], json!(40)).unwrap();
    document
        .set(&["ruff", "indentStyle"], json!("tab"))
        .unwrap();
    document
        .set(&["ruff", "lineEnding"], json!("cr-lf"))
        .unwrap();
    let changed = preview(&backend, &document, source, path);
    assert_ne!(changed, baseline);
    assert!(changed.contains("'hello'"));
    assert!(changed.contains("\r\n\treturn {\r\n"), "{changed:?}");
    assert!(
        changed.starts_with("import sys\r\nimport os\r\n"),
        "formatting must not remove or sort unused imports: {changed}"
    );
    document
        .set(&["ruff", "quoteStyle"], json!("double"))
        .unwrap();
    document.set(&["ruff", "lineLength"], json!(120)).unwrap();
    document.remove(&["ruff", "indentStyle"]).unwrap();
    document.remove(&["ruff", "lineEnding"]).unwrap();
    assert_eq!(preview(&backend, &document, source, path), baseline);
}

#[test]
fn invalid_python_reports_error_and_preserves_source_and_saved_configuration() {
    let backend = backend();
    let dir = tempfile::tempdir().unwrap();
    let document = project_config(dir.path());
    let path = dir.path().join("broken.py");
    let source = "def broken(:\n return 1\n";
    fs::write(&path, source).unwrap();
    let saved = fs::read(document.path().unwrap()).unwrap();
    let result = backend.format(
        &FormatRequest {
            source: source.into(),
            source_path: path.clone(),
            config: document.snapshot(),
        },
        cancellation(false),
    );
    let error = result.unwrap_err();
    assert!(error.contains("dprint failed"), "{error}");
    let check =
        ProjectRequest::new(dir.path().to_owned(), &document, ProjectAction::Check, true).unwrap();
    assert!(
        backend
            .execute_project(&check, cancellation(false))
            .is_err()
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
    assert_eq!(fs::read(document.path().unwrap()).unwrap(), saved);
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".codestyles-draft-")
    }));
}

#[test]
fn python_mixed_project_review_select_apply_rechecks_and_honors_excludes() {
    let backend = backend();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let document = project_config(root);
    let saved = fs::read(document.path().unwrap()).unwrap();
    let files = [
        ("中文 sample.py", "message=\"hello\"\n"),
        ("types.pyi", "def greet(name:str)->str: ...\n"),
        ("other.ts", "const value={x:1}\n"),
        ("excluded.py", "untouched=\"hello\"\n"),
    ];
    for (name, source) in files {
        fs::write(root.join(name), source).unwrap();
    }
    let check =
        ProjectRequest::new(root.to_owned(), &document, ProjectAction::Check, true).unwrap();
    let report = backend
        .execute_project(&check, cancellation(false))
        .unwrap();
    assert_eq!(report.files.len(), 3);
    assert!(report.files.iter().all(|file| !file.diff.is_empty()));
    for (name, source) in files {
        assert_eq!(fs::read_to_string(root.join(name)).unwrap(), source);
    }
    let selected: Vec<_> = ["中文 sample.py", "types.pyi"]
        .map(|name| root.join(name))
        .to_vec();
    let apply =
        ProjectRequest::from_review(root.to_owned(), &document, selected, &report, true).unwrap();
    let verified = backend
        .execute_project(&apply, cancellation(false))
        .unwrap();
    assert_eq!(verified.files.len(), 1);
    assert_eq!(
        verified.files[0].path,
        fs::canonicalize(root.join("other.ts")).unwrap()
    );
    assert!(verified.summary.contains("Verified"));
    assert_eq!(
        fs::read_to_string(root.join("中文 sample.py")).unwrap(),
        "message = 'hello'\n"
    );
    assert_ne!(
        fs::read_to_string(root.join("types.pyi")).unwrap(),
        files[1].1
    );
    for (name, source) in &files[2..] {
        assert_eq!(fs::read_to_string(root.join(name)).unwrap(), *source);
    }
    assert_eq!(fs::read(document.path().unwrap()).unwrap(), saved);
}

#[test]
fn python_review_rejects_stale_inputs_and_cancelled_writes() {
    for changed in ["source", "config", "cancel"] {
        let backend = backend();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let document = project_config(root);
        let source = root.join("sample.py");
        fs::write(&source, "message=\"hello\"\n").unwrap();
        let check =
            ProjectRequest::new(root.to_owned(), &document, ProjectAction::Check, true).unwrap();
        let report = backend
            .execute_project(&check, cancellation(false))
            .unwrap();
        let apply = ProjectRequest::from_review(
            root.to_owned(),
            &document,
            vec![source.clone()],
            &report,
            true,
        )
        .unwrap();
        match changed {
            "source" => fs::write(&source, "message=\"edited\"\n").unwrap(),
            "config" => fs::write(document.path().unwrap(), "{}").unwrap(),
            _ => {}
        }
        let before = fs::read(&source).unwrap();
        let error = backend
            .execute_project(&apply, cancellation(changed == "cancel"))
            .unwrap_err();
        assert!(!error.contains("may already"), "{error}");
        assert_eq!(fs::read(source).unwrap(), before, "{changed}");
    }
}

#[test]
fn python_nested_configuration_overrides_apply_without_mutating_configs() {
    let backend = backend();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let document = project_config(root);
    let nested = root.join("nested 中文");
    fs::create_dir(&nested).unwrap();
    let nested_config = nested.join("dprint.jsonc");
    let nested_text =
        "{ // preserve this note\n\"inherit\":true,\"ruff\":{\"quoteStyle\":\"double\"}}";
    fs::write(&nested_config, nested_text).unwrap();
    let top_source = root.join("top.py");
    let nested_source = nested.join("nested.py");
    for path in [&top_source, &nested_source] {
        fs::write(path, "message='hello'\n").unwrap();
    }
    let request =
        ProjectRequest::new(root.to_owned(), &document, ProjectAction::Check, true).unwrap();
    let report = backend
        .execute_project(&request, cancellation(false))
        .unwrap();
    assert_eq!(report.files.len(), 2);
    let apply = ProjectRequest::from_review(
        root.to_owned(),
        &document,
        vec![top_source.clone(), nested_source.clone()],
        &report,
        true,
    )
    .unwrap();
    assert!(
        backend
            .execute_project(&apply, cancellation(false))
            .unwrap()
            .files
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(top_source).unwrap(),
        "message = 'hello'\n"
    );
    assert_eq!(
        fs::read_to_string(nested_source).unwrap(),
        "message = \"hello\"\n"
    );
    assert_eq!(fs::read_to_string(nested_config).unwrap(), nested_text);
}
