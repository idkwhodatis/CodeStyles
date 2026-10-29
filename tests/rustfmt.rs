//! Native Rust mode is explicitly separate from dprint's plugin discovery.
use codestyles::{
    backend::{Backend, FormatRequest},
    config::ConfigDocument,
    project::{ProjectAction, ProjectRequest},
    resources, rustfmt,
    toolchains::{ToolchainManager, ToolchainStatus},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

fn cancel(value: bool) -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(value))
}
fn document(root: &Path, value: serde_json::Value) -> ConfigDocument {
    let path = root.join("dprint.jsonc");
    fs::write(&path, value.to_string()).unwrap();
    ConfigDocument::load(&path).unwrap()
}
fn backend() -> Backend {
    resources::backend_from_root(&Path::new(env!("CARGO_MANIFEST_DIR")).join("resources")).unwrap()
}
fn preview(backend: &Backend, document: &ConfigDocument, source: &str) -> Result<String, String> {
    backend.format(
        &FormatRequest {
            source: source.into(),
            source_path: PathBuf::from("preview.rs"),
            config: document.snapshot(),
        },
        cancel(false),
    )
}

#[test]
fn native_rust_config_is_portable_stable_and_not_an_exec_command() {
    let temp = tempfile::tempdir().unwrap();
    let config = document(
        temp.path(),
        json!({"rustfmt":{"edition":"2024","style_edition":"2021","hard_tabs":true,"max_width":100}}),
    );
    let exported = rustfmt::export_config(&config.snapshot()).unwrap();
    assert!(exported.contains("edition = \"2024\""));
    assert!(exported.contains("style_edition = \"2021\""));
    assert!(exported.contains("hard_tabs = true"));
    assert!(!exported.contains("command"));
    for field in [
        "unstable_features",
        "wrap_comments",
        "emit_mode",
        "config_path",
        "command",
    ] {
        let value = json!({"rustfmt":{field:true}});
        assert!(rustfmt::options(&value.to_string()).is_err());
    }
    for raw in [
        r#"{"rustfmt":{"max_width":1.0}}"#,
        r#"{"rustfmt":{"tab_spaces":0}}"#,
        r#"{"rustfmt":{"edition":"2027"}}"#,
    ] {
        assert!(rustfmt::options(raw).is_err());
    }
}

#[test]
fn native_rust_discovery_overrides_fail_closed_before_running_tools() {
    let temp = tempfile::tempdir().unwrap();
    for (field, value) in [
        ("includes", json!(["**/*.rs"])),
        ("excludes", json!(["skip.rs"])),
        ("extends", json!("base.json")),
        ("inherit", json!(true)),
        ("overrides", json!([])),
    ] {
        let config = document(temp.path(), json!({field:value}));
        assert!(
            ProjectRequest::new_rust(
                temp.path().to_path_buf(),
                &config,
                ProjectAction::Check,
                true
            )
            .is_err(),
            "{field}"
        );
    }
    let config = document(temp.path(), json!({"rustfmt":{"edition":"2021"}}));
    assert!(
        !ProjectRequest::new(
            temp.path().to_path_buf(),
            &config,
            ProjectAction::Check,
            true
        )
        .unwrap()
        .is_rust()
    );
    assert!(
        ProjectRequest::new_rust(
            temp.path().to_path_buf(),
            &config,
            ProjectAction::Check,
            true
        )
        .unwrap()
        .is_rust()
    );
}

#[test]
fn native_rust_nested_configs_are_not_silently_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let config = document(root, json!({}));
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/dprint.json"), "{}").unwrap();
    fs::write(root.join("nested/file.rs"), "fn main(){}\n").unwrap();
    let request =
        ProjectRequest::new_rust(root.to_path_buf(), &config, ProjectAction::Check, true).unwrap();
    let error = backend()
        .execute_project(&request, cancel(false))
        .unwrap_err();
    assert!(error.contains("nested"), "{error}");
}

#[test]
fn native_rust_empty_scope_and_cancellation_do_not_install_tools() {
    let temp = tempfile::tempdir().unwrap();
    let config = document(temp.path(), json!({}));
    fs::create_dir(temp.path().join("target")).unwrap();
    fs::write(temp.path().join("target/generated.rs"), "fn main(){}\n").unwrap();
    fs::write(temp.path().join("file.ts"), "let x=1;").unwrap();
    let request = ProjectRequest::new_rust(
        temp.path().to_path_buf(),
        &config,
        ProjectAction::Check,
        true,
    )
    .unwrap();
    assert!(
        backend()
            .execute_project(&request, cancel(true))
            .unwrap_err()
            .contains("cancelled")
    );
    let report = backend().execute_project(&request, cancel(false)).unwrap();
    assert_eq!(report.checked_files, 0);
    assert!(report.summary.contains("Rust"));
}

#[test]
fn native_rust_invalid_ignore_and_oversize_sources_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let config = document(temp.path(), json!({}));
    let request =
        ProjectRequest::new_rust(temp.path().to_owned(), &config, ProjectAction::Check, true)
            .unwrap();
    fs::write(temp.path().join(".gitignore"), "[z-a]\n").unwrap();
    let error = backend()
        .execute_project(&request, cancel(false))
        .unwrap_err();
    assert!(error.contains("ignore"), "{error}");
    fs::remove_file(temp.path().join(".gitignore")).unwrap();
    fs::File::create(temp.path().join("huge.rs"))
        .unwrap()
        .set_len(33 * 1024 * 1024)
        .unwrap();
    let error = backend()
        .execute_project(&request, cancel(false))
        .unwrap_err();
    assert!(error.contains("32 MiB"), "{error}");
}

#[cfg(unix)]
#[test]
fn native_rust_does_not_follow_symlinks_or_format_configuration_hardlinks() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let config = document(temp.path(), json!({}));
    fs::write(outside.path().join("external.rs"), "fn main(){}\n").unwrap();
    symlink(
        outside.path().join("external.rs"),
        temp.path().join("linked.rs"),
    )
    .unwrap();
    fs::hard_link(
        temp.path().join("dprint.jsonc"),
        temp.path().join("alias.rs"),
    )
    .unwrap();
    let request =
        ProjectRequest::new_rust(temp.path().to_owned(), &config, ProjectAction::Check, true)
            .unwrap();
    let report = backend().execute_project(&request, cancel(false)).unwrap();
    assert_eq!(report.checked_files, 0);
    assert_eq!(
        fs::read_to_string(outside.path().join("external.rs")).unwrap(),
        "fn main(){}\n"
    );
    symlink(
        outside.path().join("external.rs"),
        temp.path().join(".gitignore"),
    )
    .unwrap();
    assert!(backend().execute_project(&request, cancel(false)).is_err());
}

/// Requires the exact checked-in candidate digest, not an arbitrary rustfmt/PATH.
/// Run with CODESTYLES_TEST_RUSTFMT_ARCHIVE=/path/to/candidate.tar.gz and --ignored.
#[test]
#[ignore = "requires the pinned native Rust bundle candidate (not published yet)"]
fn real_managed_rust_pack_preview_and_reviewed_project_end_to_end() {
    let archive =
        std::env::var_os("CODESTYLES_TEST_RUSTFMT_ARCHIVE").expect("explicit pinned bundle path");
    let install = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(install.path().join("managed")).unwrap();
    let installed = manager
        .install_verified_archive("rustfmt", Path::new(&archive), cancel(false), |_| {})
        .unwrap();
    let runtime_root = installed.root.clone();
    assert!(installed.executable.starts_with(&runtime_root));
    assert!(runtime_root.starts_with(fs::canonicalize(manager.root()).unwrap()));
    let mut version = std::process::Command::new(&installed.executable);
    version
        .env_clear()
        .envs(installed.environment.iter().cloned());
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            version.env(key, value);
        }
    }
    let version = version
        .current_dir(install.path())
        .arg("--version")
        .output()
        .unwrap();
    assert!(
        version.status.success(),
        "{}",
        String::from_utf8_lossy(&version.stderr)
    );
    let version_text = String::from_utf8(version.stdout).unwrap();
    assert!(version_text.starts_with("rustfmt "), "{version_text}");
    assert!(version_text.contains("stable"), "{version_text}");
    drop(installed); // Shared lease must not block later uninstall.
    assert!(matches!(
        manager.status("rustfmt").unwrap(),
        ToolchainStatus::Installed { .. }
    ));
    // Uninstall must leave siblings outside the owned managed directory alone.
    let sentinel = install.path().join("keep-user-file");
    fs::write(&sentinel, b"untouched").unwrap();
    let backend = backend().with_toolchain_manager(manager.clone());
    let project = tempfile::tempdir().unwrap();
    // macOS /var aliases and Windows verbatim prefixes must match the same
    // canonical identities enforced by ProjectRequest, not lexical temp paths.
    let canonical_root = fs::canonicalize(project.path()).unwrap();
    let root = canonical_root.as_path();
    let mut config = document(
        root,
        json!({"rustfmt":{"edition":"2021","style_edition":"2021"}}),
    );
    let source = "pub async fn  run( ){let items=vec![1,2,3];}\n";
    let original_config = fs::read(config.path().unwrap()).unwrap();
    let a = preview(&backend, &config, source).unwrap();
    assert!(a.contains("    let items"));
    assert_eq!(preview(&backend, &config, &a).unwrap(), a);
    config.set(&["rustfmt", "hard_tabs"], json!(true)).unwrap();
    let b = preview(&backend, &config, source).unwrap();
    assert!(b.contains("\tlet items"));
    config.set(&["rustfmt", "hard_tabs"], json!(false)).unwrap();
    assert_eq!(preview(&backend, &config, source).unwrap(), a);
    config.set(&["rustfmt", "edition"], json!("2015")).unwrap();
    assert!(
        preview(&backend, &config, source)
            .unwrap_err()
            .contains("Rust 2015")
    );
    config.discard_changes();
    assert_eq!(fs::read(config.path().unwrap()).unwrap(), original_config);
    assert!(preview(&backend, &config, "fn main( {\n").is_err());

    fs::write(root.join("main.rs"), "mod child;\nfn main( ){let n=1;}\n").unwrap();
    fs::write(root.join("child.rs"), "pub fn child( ){ }\n").unwrap();
    fs::write(root.join("other.ts"), "const x=1;").unwrap();
    fs::write(root.join("ignored.rs"), "fn ignored( ){ }\n").unwrap();
    fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
    fs::create_dir(root.join("target")).unwrap();
    fs::write(root.join("target/generated.rs"), "invalid rust").unwrap();
    let child_before = fs::read(root.join("child.rs")).unwrap();
    let ignored_before = fs::read(root.join("ignored.rs")).unwrap();
    let check =
        ProjectRequest::new_rust(root.to_path_buf(), &config, ProjectAction::Check, true).unwrap();
    let report = backend.execute_project(&check, cancel(false)).unwrap();
    assert_eq!(report.checked_files, 2);
    assert_eq!(report.files.len(), 2);
    assert_eq!(fs::read(root.join("child.rs")).unwrap(), child_before);
    assert!(
        ProjectRequest::from_review(
            root.to_path_buf(),
            &config,
            vec![root.join("main.rs")],
            &report,
            true
        )
        .is_err()
    );
    let apply = ProjectRequest::from_rust_review(
        root.to_path_buf(),
        &config,
        vec![root.join("main.rs")],
        &report,
        true,
    )
    .unwrap();
    let verified = backend.execute_project(&apply, cancel(false)).unwrap();
    assert_eq!(verified.files.len(), 1);
    assert_eq!(verified.files[0].path, root.join("child.rs"));
    assert_eq!(fs::read(root.join("child.rs")).unwrap(), child_before);
    assert_eq!(fs::read(root.join("ignored.rs")).unwrap(), ignored_before);
    assert_eq!(
        fs::read_to_string(root.join("other.ts")).unwrap(),
        "const x=1;"
    );

    let stale = ProjectRequest::from_rust_review(
        root.to_path_buf(),
        &config,
        vec![root.join("child.rs")],
        &verified,
        true,
    )
    .unwrap();
    fs::write(root.join("new.rs"), "fn newly_added(){}\n").unwrap();
    assert!(
        backend
            .execute_project(&stale, cancel(false))
            .unwrap_err()
            .contains("scope")
    );
    fs::remove_file(root.join("new.rs")).unwrap();
    fs::write(root.join(".gitignore"), "ignored.rs\nchild.rs\n").unwrap();
    assert!(backend.execute_project(&stale, cancel(false)).is_err());
    fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
    fs::write(root.join("child.rs"), "pub fn changed_since_review( ){ }\n").unwrap();
    assert!(
        backend
            .execute_project(&stale, cancel(false))
            .unwrap_err()
            .contains("changed")
    );
    assert!(
        backend
            .execute_project(&stale, cancel(true))
            .unwrap_err()
            .contains("cancelled")
    );
    manager.uninstall("rustfmt").unwrap();
    assert_eq!(manager.status("rustfmt").unwrap(), ToolchainStatus::Missing);
    assert!(
        !runtime_root.exists(),
        "Uninstall must remove the managed runtime"
    );
    assert!(!manager.root().join("rustfmt/active.json").exists());
    let remaining = fs::read_dir(manager.root().join("rustfmt"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(remaining, vec![std::ffi::OsString::from(".lock")]);
    assert_eq!(fs::read(sentinel).unwrap(), b"untouched");
    assert!(
        preview(&backend, &config, source)
            .unwrap_err()
            .contains("Install Rust")
    );
}
