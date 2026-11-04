//! Optional-runtime startup and explicit trusted-pack lifecycle acceptance.
use codestyles::{
    backend::{Backend, FormatRequest},
    comparison::{BuiltinSample, SourceLanguage},
    config::ConfigDocument,
    project::{ProjectAction, ProjectRequest},
    schema::{PinnedPlugin, SchemaCatalog},
    toolchains::{ToolchainManager, ToolchainStatus},
};
use std::{
    fs,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn missing_dprint_keeps_settings_usable_and_actions_fail_explicitly() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    let backend = Backend::managed().with_toolchain_manager(manager.clone());
    assert!(backend.executable().as_os_str().is_empty());
    let mut config = backend.standalone_config().unwrap();
    let catalog = SchemaCatalog::bundled().unwrap();
    assert!(!catalog.fields(PinnedPlugin::TypeScript).is_empty());
    let field = catalog
        .schema(PinnedPlugin::Json)
        .field("indentWidth")
        .unwrap();
    field
        .set(&mut config, PinnedPlugin::Json, serde_json::json!(4))
        .unwrap();
    let source = "{\"n\":1}";
    let result = backend.format(
        &FormatRequest {
            source: source.into(),
            source_path: "preview.json".into(),
            config: config.snapshot(),
        },
        cancel(),
    );
    assert!(result.unwrap_err().contains("Install or repair dprint"));
    assert_eq!(manager.status("dprint").unwrap(), ToolchainStatus::Missing);
    assert!(
        manager.download_available("dprint").is_err(),
        "unpublished candidates must not become downloadable"
    );
    // Saving portable settings has no installation or formatting prerequisite.
    assert!(config.draft_text().contains("https://plugins.dprint.dev/"));
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("dprint.jsonc"), config.draft_text()).unwrap();
    fs::write(root.join("source.json"), source).unwrap();
    let config = ConfigDocument::load(&root.join("dprint.jsonc")).unwrap();
    let request = ProjectRequest::new(root.clone(), &config, ProjectAction::Check, true).unwrap();
    assert!(
        backend
            .execute_project(&request, cancel())
            .unwrap_err()
            .contains("Install or repair dprint")
    );
    assert_eq!(
        fs::read_to_string(root.join("source.json")).unwrap(),
        source
    );
}

#[test]
#[ignore = "requires CODESTYLES_DPRINT_ARCHIVE matching the embedded candidate digest"]
fn real_managed_dprint_install_preview_project_corrupt_reinstall_uninstall() {
    let archive =
        std::env::var_os("CODESTYLES_DPRINT_ARCHIVE").expect("explicit trusted archive required");
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    let install = || {
        manager
            .install_verified_archive("dprint", Path::new(&archive), cancel(), |_| {})
            .unwrap()
    };
    let installed = install();
    let runtime_root = installed.root.clone();
    let plugins = installed.wasm_plugins.clone();
    let executable = installed.executable.clone();
    drop(installed);
    let backend = Backend::managed().with_toolchain_manager(manager.clone());
    let config = backend.standalone_config().unwrap();
    for language in SourceLanguage::ALL
        .into_iter()
        .filter(|language| language.is_bundled())
    {
        let source = BuiltinSample::Overview.source(language);
        let request = FormatRequest {
            source: source.into(),
            source_path: format!("sample.{}", language.extension()).into(),
            config: config.snapshot(),
        };
        let output = backend.format(&request, cancel()).unwrap();
        assert_ne!(output, source, "{}", language.label());
        assert_eq!(
            backend
                .format(
                    &FormatRequest {
                        source: output.clone(),
                        ..request
                    },
                    cancel()
                )
                .unwrap(),
            output
        );
    }
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("dprint.jsonc"),
        serde_json::json!({"plugins":plugins,"includes":["**/*.json"]}).to_string(),
    )
    .unwrap();
    let project_config = ConfigDocument::load(&root.join("dprint.jsonc")).unwrap();
    let source = root.join("source.json");
    fs::write(&source, "{\"n\":1}").unwrap();
    let request =
        ProjectRequest::new(root.clone(), &project_config, ProjectAction::Check, true).unwrap();
    let report = backend.execute_project(&request, cancel()).unwrap();
    assert_eq!(report.files.len(), 1);
    let apply =
        ProjectRequest::from_review(root, &project_config, vec![source.clone()], &report, true)
            .unwrap();
    assert!(
        backend
            .execute_project(&apply, cancel())
            .unwrap()
            .files
            .is_empty()
    );
    assert_ne!(fs::read_to_string(&source).unwrap(), "{\"n\":1}");
    fs::write(&executable, "corrupt").unwrap();
    assert!(matches!(
        manager.status("dprint").unwrap(),
        ToolchainStatus::Corrupt { .. }
    ));
    let request = FormatRequest {
        source: "{\"n\":1}".into(),
        source_path: "preview.json".into(),
        config: config.snapshot(),
    };
    assert!(
        backend
            .format(&request, cancel())
            .unwrap_err()
            .contains("Install or repair")
    );
    drop(install());
    assert!(backend.format(&request, cancel()).is_ok());
    manager.uninstall("dprint").unwrap();
    assert_eq!(manager.status("dprint").unwrap(), ToolchainStatus::Missing);
    assert!(!runtime_root.exists());
    assert!(
        backend
            .format(&request, cancel())
            .unwrap_err()
            .contains("Install or repair")
    );
    assert!(backend.standalone_config().is_ok());
}

#[test]
fn production_headless_startup_without_binary_or_wasm() {
    const CHILD_REPORT: &str = "CODESTYLES_TEST_OFFLINE_STARTUP_REPORT";
    if let Some(report) = std::env::var_os(CHILD_REPORT) {
        codestyles::self_test::run(Path::new(&report)).unwrap();
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let resources = temp.path().join("offline resources");
    fs::create_dir(&resources).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(source.join("manifest.json")).unwrap()).unwrap();
    let mut assets = vec!["manifest.json".to_owned()];
    assets.extend(
        manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| {
                let path = item["path"].as_str()?;
                (path.starts_with("schemas/") || path.starts_with("licenses/"))
                    .then(|| path.to_owned())
            }),
    );
    for asset in assets {
        let destination = resources.join(&asset);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(source.join(asset), destination).unwrap();
    }
    let managed = temp.path().join("empty managed tools");
    let report_path = temp.path().join("startup-report.json");
    // All environment changes are private to a child process: test workers can
    // concurrently run real formatter checks without observing these overrides.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "production_headless_startup_without_binary_or_wasm",
            "--nocapture",
        ])
        .env(CHILD_REPORT, &report_path)
        .env("CODESTYLES_RESOURCES", &resources)
        .env("CODESTYLES_TOOLCHAINS_DIR", &managed)
        .env("DPRINT_CACHE_DIR", temp.path().join("empty cache"))
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["status"], "passed");
    assert_eq!(report["mode"], "headless_optional_runtime");
    assert_eq!(report["runtime"]["status"], "not_installed");
    assert_eq!(report["offline_configuration"]["status"], "passed");
    assert_eq!(report["resources"]["discovery"], "environment_override");
    assert_eq!(report["resources"]["checksums_verified"], true);
    assert!(report["dprint_executable"].is_null());
    let checks = report["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 10);
    for check in checks {
        assert_eq!(check["status"], "skipped");
        assert!(check["reason"].as_str().unwrap().contains("Install dprint"));
    }
    assert!(!resources.join("bin").exists());
    assert!(!resources.join("plugins").exists());
    assert!(
        !managed.join("dprint").exists(),
        "startup must not silently install the runtime"
    );
}
