use codestyles::{
    backend::Backend,
    config::ConfigDocument,
    project::{ProjectAction, ProjectRequest},
    resources,
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn backend() -> Backend {
    resources::bundled_backend()
        .unwrap()
        .with_timeout(Duration::from_secs(30))
}
fn config(root: &Path, newline: &str) -> ConfigDocument {
    let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/plugins");
    let plugins: Vec<_> = [
        "typescript-0.96.1.wasm",
        "json-0.25.2.wasm",
        "markdown-0.26.0.wasm",
        "toml-0.9.0.wasm",
    ]
    .into_iter()
    .map(|name| resources.join(name).to_str().unwrap().to_owned())
    .collect();
    fs::create_dir_all(root).unwrap();
    let path = root.join("dprint.jsonc");
    fs::write(&path, serde_json::to_string_pretty(&json!({"plugins":plugins,"includes":["**/*.{ts,json,jsonc,md,toml}"],"newLineKind":newline,"typescript":{"semiColons":"always"}})).unwrap()).unwrap();
    ConfigDocument::load(&path).unwrap()
}
fn request(root: &Path, doc: &ConfigDocument, action: ProjectAction) -> ProjectRequest {
    ProjectRequest::new(root.to_owned(), doc, action, true).unwrap()
}
fn run(
    root: &Path,
    doc: &ConfigDocument,
    action: ProjectAction,
) -> codestyles::project::ProjectReport {
    backend()
        .execute_project(
            &request(root, doc, action),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
}

#[test]
fn checks_and_formats_real_saved_project_with_spaces_unicode_and_line_endings() {
    for ending in ["lf", "crlf"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("项目 with spaces");
        let doc = config(&root, ending);
        let saved = fs::read(doc.path().unwrap()).unwrap();
        for (name, text) in [
            ("汉字 file.ts", "const x={a:1,b:2}\r\n"),
            ("a.md", "# Heading\n\n*   a\n*   b\n"),
            ("a.toml", "name=\"test\"\n"),
        ] {
            fs::write(root.join(name), text).unwrap();
        }
        let before: Vec<_> = ["汉字 file.ts", "a.md", "a.toml"]
            .iter()
            .map(|p| fs::read(root.join(p)).unwrap())
            .collect();
        let checked = run(&root, &doc, ProjectAction::Check);
        assert_eq!(checked.cli_version, "dprint 0.63.0");
        assert_eq!(checked.files.len(), 3);
        assert!(checked.files.iter().all(|file| !file.diff.is_empty()));
        for (name, bytes) in ["汉字 file.ts", "a.md", "a.toml"].iter().zip(before) {
            assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
        }
        let formatted = run(&root, &doc, ProjectAction::FormatProject);
        assert_eq!(formatted.checked_files, 3);
        assert_eq!(fs::read(doc.path().unwrap()).unwrap(), saved);
        assert!(run(&root, &doc, ProjectAction::Check).files.is_empty());
        let text = fs::read_to_string(root.join("汉字 file.ts")).unwrap();
        assert!(text.ends_with(if ending == "crlf" { ";\r\n" } else { ";\n" }));
    }
}

#[test]
fn selected_names_are_literal_and_nested_configuration_is_respected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let nested = root.join("nested space");
    fs::create_dir(&nested).unwrap();
    let child = nested.join("dprint.json");
    fs::write(
        &child,
        "{\"inherit\":true,\"typescript\":{\"semiColons\":\"asi\"}}",
    )
    .unwrap();
    let nested_saved = fs::read(&child).unwrap();
    let selected = root.join("--watch [a]{b}.ts");
    let untouched = root.join("--watch a{b}.ts");
    let inner = nested.join("中文.ts");
    for path in [&selected, &untouched, &inner] {
        fs::write(path, "const x={a:1}\n").unwrap();
    }
    let before = fs::read(&untouched).unwrap();
    run(
        root,
        &doc,
        ProjectAction::FormatSelected(vec![selected.clone(), inner.clone()]),
    );
    assert_eq!(fs::read(untouched).unwrap(), before);
    assert!(fs::read_to_string(selected).unwrap().ends_with(";\n"));
    assert!(!fs::read_to_string(inner).unwrap().ends_with(";\n"));
    assert_eq!(fs::read(child).unwrap(), nested_saved);
}

#[test]
fn refuses_dirty_untrusted_unsaved_changed_config_and_outside_targets() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut doc = config(root, "lf");
    assert!(ProjectRequest::new(root.to_owned(), &doc, ProjectAction::Check, false).is_err());
    doc.set(&["lineWidth"], json!(100)).unwrap();
    assert!(ProjectRequest::new(root.to_owned(), &doc, ProjectAction::Check, true).is_err());
    doc.discard_changes();
    let unsaved = backend().standalone_config().unwrap();
    assert!(ProjectRequest::new(root.to_owned(), &unsaved, ProjectAction::Check, true).is_err());
    let outside = tempfile::NamedTempFile::new().unwrap();
    assert!(
        ProjectRequest::new(
            root.to_owned(),
            &doc,
            ProjectAction::FormatSelected(vec![outside.path().to_owned()]),
            true
        )
        .is_err()
    );
    let queued = request(root, &doc, ProjectAction::Check);
    fs::write(doc.path().unwrap(), "{}").unwrap();
    assert!(
        backend()
            .execute_project(&queued, Arc::new(AtomicBool::new(false)))
            .unwrap_err()
            .contains("changed on disk")
    );
}

#[test]
fn preparation_preserves_configuration_and_missing_plugins_fail_explicitly() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let before = fs::read(doc.path().unwrap()).unwrap();
    let prepared = run(root, &doc, ProjectAction::PreparePlugins);
    assert!(!prepared.summary.is_empty());
    assert_eq!(fs::read(doc.path().unwrap()).unwrap(), before);
    fs::write(
        doc.path().unwrap(),
        json!({"plugins":[root.join("missing.wasm")]}).to_string(),
    )
    .unwrap();
    let broken = ConfigDocument::load(doc.path().unwrap()).unwrap();
    let error = backend()
        .execute_project(
            &request(root, &broken, ProjectAction::PreparePlugins),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
    assert!(error.contains("failed"));
    assert_eq!(
        fs::read_to_string(doc.path().unwrap()).unwrap(),
        broken.saved_text()
    );
}

#[test]
fn protects_local_extends_and_configuration_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut doc = config(root, "lf");
    let shared = root.join("style base.json");
    fs::write(
        &shared,
        "{\"typescript\":{\"quoteStyle\":\"preferSingle\"}}",
    )
    .unwrap();
    doc.set(&["extends"], json!("./style base.json")).unwrap();
    doc.save().unwrap();
    fs::write(root.join("source.ts"), "const a = \"hello\"\n").unwrap();
    let before = fs::read(&shared).unwrap();
    run(root, &doc, ProjectAction::FormatProject);
    assert_eq!(fs::read(&shared).unwrap(), before);
    assert!(
        backend()
            .execute_project(
                &request(root, &doc, ProjectAction::FormatSelected(vec![shared])),
                Arc::new(AtomicBool::new(false))
            )
            .unwrap_err()
            .contains("protected")
    );
}

#[test]
fn remote_plugin_connection_failure_is_reported_without_changing_config() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("dprint.json");
    fs::write(
        &path,
        json!({"plugins":[format!("http://127.0.0.1:{port}/missing.wasm")]}).to_string(),
    )
    .unwrap();
    let doc = ConfigDocument::load(&path).unwrap();
    assert!(
        backend()
            .execute_project(
                &request(temp.path(), &doc, ProjectAction::PreparePlugins),
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
    );
    assert_eq!(fs::read_to_string(path).unwrap(), doc.saved_text());
}

#[test]
fn real_cli_download_is_cancellable_without_changing_config() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("dprint.json");
    fs::write(
        &path,
        json!({"plugins":[format!("http://127.0.0.1:{port}/blocked.wasm")]}).to_string(),
    )
    .unwrap();
    let doc = ConfigDocument::load(&path).unwrap();
    let req = request(temp.path(), &doc, ProjectAction::PreparePlugins);
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = std::thread::spawn(move || backend().execute_project(&req, flag));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let connection = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "dprint never requested plugin"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("{e}"),
        }
    };
    cancelled.store(true, Ordering::Release);
    let stopped = std::time::Instant::now();
    let error = worker.join().unwrap().unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(stopped.elapsed() < Duration::from_secs(5));
    drop(connection);
    assert_eq!(fs::read_to_string(path).unwrap(), doc.saved_text());
}

#[cfg(unix)]
#[test]
fn rejects_paths_that_cli_cannot_represent_and_symlink_escape() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    for name in ["a\nb.ts", "a:b.ts", "a\\b.ts"] {
        let path = root.join(name);
        fs::write(&path, "const a=1").unwrap();
        assert!(
            ProjectRequest::new(
                root.to_owned(),
                &doc,
                ProjectAction::FormatSelected(vec![path]),
                true
            )
            .is_err()
        );
    }
    let outside = tempfile::NamedTempFile::new().unwrap();
    let link = root.join("escape.ts");
    symlink(outside.path(), &link).unwrap();
    assert!(
        ProjectRequest::new(
            root.to_owned(),
            &doc,
            ProjectAction::FormatSelected(vec![link]),
            true
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn timeout_and_inherited_child_pipes_are_bounded() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let executable = root.join("fixture-dprint");
    // A successful main process may leave descendants holding its pipes. The
    // project runner owns a process group and must not block joining readers.
    fs::write(
        &executable,
        "#!/bin/sh\nif [ \"$1\" = '--version' ]; then echo 'dprint 0.63.0'; else sleep 60 & fi\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let started = std::time::Instant::now();
    let result = Backend::new(executable.clone(), vec![])
        .with_timeout(Duration::from_secs(2))
        .execute_project(
            &request(root, &doc, ProjectAction::Check),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(result.checked_files, 0);
    assert!(started.elapsed() < Duration::from_secs(4));
    fs::write(
        &executable,
        "#!/bin/sh\nif [ \"$1\" = '--version' ]; then echo 'dprint 0.63.0'; else sleep 60; fi\n",
    )
    .unwrap();
    let started = std::time::Instant::now();
    let error = Backend::new(executable, vec![])
        .with_timeout(Duration::from_millis(200))
        .execute_project(
            &request(root, &doc, ProjectAction::Check),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
    assert!(error.contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn unsupported_local_extends_forms_fail_closed_before_writing() {
    for reference in [
        "file:///project/base.json",
        "${originConfigDir}/base.json",
        "~/base.json",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut doc = config(root, "lf");
        let base = root.join("base.json");
        fs::write(&base, "{\"lineWidth\":100}").unwrap();
        let source = root.join("source.ts");
        fs::write(&source, "const a={x:1}").unwrap();
        let reference = if reference.starts_with("file:") {
            // file URLs are recognized by dprint itself; they must not silently
            // escape our config protection. Use forward slashes on Windows.
            format!(
                "file://{}{}",
                if cfg!(windows) { "/" } else { "" },
                base.to_string_lossy().replace('\\', "/")
            )
        } else {
            reference.to_owned()
        };
        doc.set(&["extends"], json!(reference)).unwrap();
        doc.save().unwrap();
        let before = fs::read(&base).unwrap();
        let source_before = fs::read(&source).unwrap();
        assert!(
            backend()
                .execute_project(
                    &request(root, &doc, ProjectAction::FormatProject),
                    Arc::new(AtomicBool::new(false))
                )
                .is_err()
        );
        assert_eq!(fs::read(base).unwrap(), before);
        assert_eq!(fs::read(source).unwrap(), source_before);
    }
}

#[cfg(windows)]
#[test]
fn windows_job_reaps_descendants_even_after_cli_parent_exits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let executable = root.join("fixture-dprint.exe");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project_process.rs");
    let built = std::process::Command::new("rustc")
        .arg("--edition=2024")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let started = std::time::Instant::now();
    let result = Backend::new(executable, vec![])
        .with_timeout(Duration::from_secs(2))
        .execute_project(
            &request(root, &doc, ProjectAction::Check),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(result.checked_files, 0);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn configuration_hard_link_aliases_cannot_be_formatted_as_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let alias = root.join("looks-like-source.json");
    fs::hard_link(doc.path().unwrap(), &alias).unwrap();
    let before = fs::read(doc.path().unwrap()).unwrap();
    let report = run(root, &doc, ProjectAction::FormatProject);
    assert_eq!(report.checked_files, 0);
    let error = backend()
        .execute_project(
            &request(root, &doc, ProjectAction::FormatSelected(vec![alias])),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
    assert!(error.contains("protected"));
    assert_eq!(fs::read(doc.path().unwrap()).unwrap(), before);
}

#[test]
fn reviewed_apply_formats_only_selected_and_returns_current_remaining_diffs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let a = root.join("a.ts");
    let b = root.join("b.ts");
    fs::write(&a, "const a={x:1}\n").unwrap();
    fs::write(&b, "const b={x:2}\n").unwrap();
    let review = run(root, &doc, ProjectAction::Check);
    let before = fs::read(&b).unwrap();
    let apply =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![a.clone()], &review, true).unwrap();
    let verified = backend()
        .execute_project(&apply, Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_eq!(verified.checked_files, 2);
    assert_eq!(verified.files.len(), 1);
    assert_eq!(verified.files[0].path, fs::canonicalize(&b).unwrap());
    assert_eq!(fs::read(&b).unwrap(), before);
    assert!(verified.summary.contains("Verified"));
    let finish =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![b], &verified, true).unwrap();
    assert!(
        backend()
            .execute_project(&finish, Arc::new(AtomicBool::new(false)))
            .unwrap()
            .files
            .is_empty()
    );
    assert!(ProjectRequest::from_review(root.to_owned(), &doc, vec![a], &verified, true).is_err());
}

#[test]
fn reviewed_apply_rejects_stale_sources_and_configs_even_after_queueing() {
    for changed in ["source", "nested", "added_nested", "inherited", "root"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut doc = config(root, "lf");
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        let source = nested.join("source.ts");
        fs::write(&source, "const a={x:1}\n").unwrap();
        let child = nested.join("dprint.json");
        if changed == "nested" {
            fs::write(&child, "{\"inherit\":true}").unwrap();
        }
        let base = root.join("base.json");
        fs::write(&base, "{\"lineWidth\":80}").unwrap();
        doc.set(&["extends"], json!("./base.json")).unwrap();
        doc.save().unwrap();
        let review = run(root, &doc, ProjectAction::Check);
        let apply =
            ProjectRequest::from_review(root.to_owned(), &doc, vec![source.clone()], &review, true)
                .unwrap();
        match changed {
            "source" => fs::write(&source, "const edited={x:2}\n").unwrap(),
            "nested" | "added_nested" => {
                fs::write(&child, "{\"inherit\":true,\"lineWidth\":100}").unwrap()
            }
            "inherited" => fs::write(&base, "{\"lineWidth\":100}").unwrap(),
            _ => fs::write(doc.path().unwrap(), "{}").unwrap(),
        }
        let before = fs::read(&source).unwrap();
        let error = backend()
            .execute_project(&apply, Arc::new(AtomicBool::new(false)))
            .unwrap_err();
        assert!(!error.contains("may already"), "{error}");
        assert_eq!(fs::read(&source).unwrap(), before, "{changed}");
    }
}

#[test]
fn reviewed_apply_cancelled_before_write_preserves_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let source = root.join("source.ts");
    fs::write(&source, "const a={x:1}\n").unwrap();
    let review = run(root, &doc, ProjectAction::Check);
    let apply =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![source.clone()], &review, true)
            .unwrap();
    let before = fs::read(&source).unwrap();
    let error = backend()
        .execute_project(&apply, Arc::new(AtomicBool::new(true)))
        .unwrap_err();
    assert!(error.contains("cancelled"));
    assert!(!error.contains("may already"));
    assert_eq!(fs::read(source).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn reviewed_apply_rejects_source_mutation_during_cli_recheck_before_fmt() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let source = root.join("source.ts");
    fs::write(&source, "const a={x:1}\n").unwrap();
    let review = run(root, &doc, ProjectAction::Check);
    let apply =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![source.clone()], &review, true)
            .unwrap();
    let executable = root.join("fixture-dprint");
    fs::write(&executable, r#"#!/bin/sh
case "$1" in
--version) echo 'dprint 0.63.0';;
file-paths) cat >/dev/null; echo './source.ts';;
check) cat >/dev/null; printf 'const changed=2\n' > source.ts; echo '{"file":"source.ts","diff":"changed"}'; exit 20;;
fmt) cat >/dev/null; touch fmt-was-called;;
esac
"#).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let error = Backend::new(executable, vec![])
        .execute_project(&apply, Arc::new(AtomicBool::new(false)))
        .unwrap_err();
    assert!(error.contains("Source changed"), "{error}");
    assert!(!root.join("fmt-was-called").exists());
}

#[test]
fn reviewed_apply_rejects_changed_local_plugin_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut doc = config(root, "lf");
    let plugin = root.join("local-json.wasm");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/plugins/json-0.25.2.wasm"),
        &plugin,
    )
    .unwrap();
    doc.set(
        &["plugins"],
        json!([
            plugin,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/plugins/typescript-0.96.1.wasm")
        ]),
    )
    .unwrap();
    doc.save().unwrap();
    let source = root.join("source.json");
    fs::write(&source, "{\"a\":1}").unwrap();
    let review = run(root, &doc, ProjectAction::Check);
    let apply =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![source.clone()], &review, true)
            .unwrap();
    fs::write(&plugin, "changed plugin").unwrap();
    let before = fs::read(&source).unwrap();
    let error = backend()
        .execute_project(&apply, Arc::new(AtomicBool::new(false)))
        .unwrap_err();
    assert!(!error.contains("may already"));
    assert_eq!(fs::read(source).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn reviewed_apply_rejects_config_or_diff_mutation_during_cli_recheck() {
    use std::os::unix::fs::PermissionsExt;
    for change_config in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let doc = config(root, "lf");
        let source = root.join("source.ts");
        fs::write(&source, "const a={x:1}\n").unwrap();
        let review = run(root, &doc, ProjectAction::Check);
        let apply =
            ProjectRequest::from_review(root.to_owned(), &doc, vec![source.clone()], &review, true)
                .unwrap();
        let executable = root.join("fixture-dprint");
        let mutation = if change_config {
            "printf '{}' > dprint.jsonc;"
        } else {
            ""
        };
        fs::write(
            &executable,
            format!(
                r#"#!/bin/sh
case "$1" in
--version) echo 'dprint 0.63.0';;
file-paths) cat >/dev/null; echo './source.ts';;
check) cat >/dev/null; {mutation} echo '{{"file":"source.ts","diff":"different formatter output"}}'; exit 20;;
fmt) cat >/dev/null; touch fmt-was-called;;
esac
"#
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let before = fs::read(&source).unwrap();
        let error = Backend::new(executable, vec![])
            .execute_project(&apply, Arc::new(AtomicBool::new(false)))
            .unwrap_err();
        assert!(error.contains("changed"), "{error}");
        assert!(!root.join("fmt-was-called").exists());
        assert_eq!(fs::read(&source).unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn cancellation_during_reviewed_format_reports_possible_partial_writes() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let source = root.join("source.ts");
    fs::write(&source, "const a={x:1}\n").unwrap();
    let review = run(root, &doc, ProjectAction::Check);
    let apply =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![source], &review, true).unwrap();
    fs::write(
        root.join("review-output"),
        json!({"file":"source.ts", "diff": review.files[0].diff}).to_string(),
    )
    .unwrap();
    let executable = root.join("fixture-dprint");
    fs::write(
        &executable,
        r#"#!/bin/sh
case "$1" in
--version) echo 'dprint 0.63.0';;
file-paths) cat >/dev/null; echo './source.ts';;
check) cat >/dev/null; cat review-output; exit 20;;
fmt) cat >/dev/null; printf 'const a = { x: 1 };\n' > source.ts; touch fmt-started; sleep 60;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = std::thread::spawn(move || {
        Backend::new(executable, vec![])
            .with_timeout(Duration::from_secs(10))
            .execute_project(&apply, flag)
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while !root.join("fmt-started").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    cancelled.store(true, Ordering::Release);
    let error = worker.join().unwrap().unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(error.contains("not transactional"), "{error}");
    assert!(error.contains("may already"), "{error}");
}

#[cfg(unix)]
#[test]
fn stale_review_is_rejected_before_cli_can_load_changed_dependencies() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    let source = nested.join("source.ts");
    fs::write(&source, "const a={x:1}\n").unwrap();
    let review = run(root, &doc, ProjectAction::Check);
    let apply =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![source], &review, true).unwrap();
    fs::write(
        nested.join("dprint.json"),
        "{\"inherit\":true,\"lineWidth\":100}",
    )
    .unwrap();
    let executable = root.join("fixture-dprint");
    fs::write(&executable, "#!/bin/sh\ntouch cli-was-called\nexit 1\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let error = Backend::new(executable, vec![])
        .execute_project(&apply, Arc::new(AtomicBool::new(false)))
        .unwrap_err();
    assert!(error.contains("changed since review"), "{error}");
    assert!(!root.join("cli-was-called").exists());
}

#[cfg(unix)]
#[test]
fn failed_post_format_verification_returns_error_instead_of_stale_report() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let doc = config(root, "lf");
    let source = root.join("source.ts");
    fs::write(&source, "const a={x:1}\n").unwrap();
    let review = run(root, &doc, ProjectAction::Check);
    let apply =
        ProjectRequest::from_review(root.to_owned(), &doc, vec![source.clone()], &review, true)
            .unwrap();
    fs::write(
        root.join("review-output"),
        json!({"file":"source.ts", "diff": review.files[0].diff}).to_string(),
    )
    .unwrap();
    let executable = root.join("fixture-dprint");
    fs::write(&executable, r#"#!/bin/sh
case "$1" in
--version) echo 'dprint 0.63.0';;
file-paths) cat >/dev/null; echo './source.ts';;
check) cat >/dev/null; if [ -f fmt-finished ]; then echo 'verification failed' >&2; exit 1; fi; cat review-output; exit 20;;
fmt) cat >/dev/null; printf 'const a = { x: 1 };\n' > source.ts; touch fmt-finished;;
esac
"#).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let error = Backend::new(executable, vec![])
        .execute_project(&apply, Arc::new(AtomicBool::new(false)))
        .unwrap_err();
    assert!(
        error.contains("Formatting ran, but the automatic verification failed"),
        "{error}"
    );
    assert!(error.contains("not transactional"), "{error}");
    assert_eq!(fs::read_to_string(source).unwrap(), "const a = { x: 1 };\n");
}
