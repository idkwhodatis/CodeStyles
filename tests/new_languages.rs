//! Explicit local acceptance: only archives matching the embedded catalog work.
//! Each ignored test installs one language pack; no executable or URL override.
use codestyles::{
    backend::{Backend, FormatRequest},
    config::ConfigDocument,
    project::{NativeLanguage, ProjectAction, ProjectRequest},
    resources,
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

fn acceptance(
    env: &str,
    id: &str,
    language: NativeLanguage,
    extension: &str,
    source: &str,
    invalid: Option<&str>,
) {
    let archive = std::env::var_os(env).expect("explicit pinned archive path required");
    let dir = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(dir.path().join("managed")).unwrap();
    let installed = manager
        .install_verified_archive(id, Path::new(&archive), cancel(), |_| {})
        .unwrap();
    let runtime_root = installed.root.clone();
    drop(installed);
    let backend: Backend =
        resources::backend_from_root(&Path::new(env!("CARGO_MANIFEST_DIR")).join("resources"))
            .unwrap()
            .with_toolchain_manager(manager.clone());
    let project = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(project.path()).unwrap();
    fs::write(root.join("dprint.jsonc"), "{}\n").unwrap();
    let mut config = ConfigDocument::load(&root.join("dprint.jsonc")).unwrap();
    // Exercise explicit native import/export through the actual managed engine,
    // not merely parser round-trips. Only this test's private config is saved.
    let interoperability = match language {
        NativeLanguage::CSharp => Some((
            codestyles::native_config::NativeConfigKind::CSharpier,
            r#"{"printWidth":80,"indentSize":2,"useTabs":true,"endOfLine":"lf"}"#,
        )),
        NativeLanguage::C | NativeLanguage::Cpp => Some((
            codestyles::native_config::NativeConfigKind::ClangFormat,
            "BasedOnStyle: Google\nIndentWidth: 4\nColumnLimit: 100\nBreakBeforeBraces: Allman\nSortIncludes: Never\n",
        )),
        _ => None,
    };
    if let Some((kind, input)) = interoperability {
        codestyles::native_config::import_config(&mut config, input, kind).unwrap();
        let exported = codestyles::native_config::export_config(&config.snapshot(), kind).unwrap();
        let mut reimported = ConfigDocument::standalone(&[]).unwrap();
        codestyles::native_config::import_config(&mut reimported, &exported, kind).unwrap();
        assert_eq!(
            codestyles::native_config::export_config(&reimported.snapshot(), kind).unwrap(),
            exported
        );
        config.save().unwrap();
    }
    let original_config = fs::read(config.path().unwrap()).unwrap();
    let format = |text: &str| {
        backend.format(
            &FormatRequest {
                source: text.into(),
                // The UI's selected C++ preview uses a .cpp stdin hint even
                // when project files use shared .h names. Direct .h requests
                // intentionally default to C and are not equivalent here.
                source_path: root.join(format!(
                    "preview.{}",
                    if language == NativeLanguage::Cpp && extension == "h" {
                        "cpp"
                    } else {
                        extension
                    }
                )),
                config: config.snapshot(),
            },
            cancel(),
        )
    };
    let formatted = format(source).unwrap();
    assert_ne!(formatted, source);
    assert_eq!(format(&formatted).unwrap(), formatted);
    if let Some(invalid) = invalid {
        assert!(format(invalid).is_err());
    }
    assert_eq!(fs::read(config.path().unwrap()).unwrap(), original_config);

    let selected = root.join(format!("selected.{extension}"));
    let remaining = root.join(format!("remaining.{extension}"));
    fs::write(&selected, source).unwrap();
    fs::write(&remaining, source).unwrap();
    let other_extension = if extension == "go" { "dart" } else { "go" };
    let other = root.join(format!("other.{other_extension}"));
    fs::write(&other, "not valid source; must remain untouched").unwrap();
    let request =
        ProjectRequest::new_native(root.clone(), &config, ProjectAction::Check, true, language)
            .unwrap();
    let report = backend.execute_project(&request, cancel()).unwrap();
    assert_eq!(report.checked_files, 2);
    assert_eq!(report.files.len(), 2);
    assert_eq!(fs::read_to_string(&selected).unwrap(), source);
    assert_eq!(fs::read_to_string(&remaining).unwrap(), source);
    let apply = ProjectRequest::from_native_review(
        root.clone(),
        &config,
        vec![selected.clone()],
        &report,
        true,
        language,
    )
    .unwrap();
    let verified = backend.execute_project(&apply, cancel()).unwrap();
    assert_eq!(verified.checked_files, 2);
    assert_eq!(verified.files.len(), 1);
    assert_eq!(verified.files[0].path, remaining);
    assert_eq!(fs::read_to_string(&selected).unwrap(), formatted);
    assert_eq!(fs::read_to_string(&remaining).unwrap(), source);
    assert_eq!(
        fs::read_to_string(other).unwrap(),
        "not valid source; must remain untouched"
    );
    assert_eq!(fs::read(config.path().unwrap()).unwrap(), original_config);
    manager.uninstall(id).unwrap();
    assert_eq!(manager.status(id).unwrap(), ToolchainStatus::Missing);
    assert!(!runtime_root.exists());
    assert!(format(source).is_err());
}

#[test]
#[ignore = "requires CODESTYLES_TEST_GO_PACK matching the embedded digest"]
fn real_go_pack_preview_review_write_and_uninstall() {
    acceptance(
        "CODESTYLES_TEST_GO_PACK",
        "gofmt",
        NativeLanguage::Go,
        "go",
        "package main\nfunc main( ){println(1)}\n",
        Some("package main\nfunc main( {\n"),
    );
}

#[test]
#[ignore = "requires CODESTYLES_TEST_DART_PACK matching the embedded digest"]
fn real_dart_pack_preview_review_write_and_uninstall() {
    acceptance(
        "CODESTYLES_TEST_DART_PACK",
        "dartfmt",
        NativeLanguage::Dart,
        "dart",
        "void main( ){print('hello');}\n",
        Some("void main( {\n"),
    );
}

#[test]
#[ignore = "requires CODESTYLES_TEST_VUE_PACK matching the embedded digest"]
fn real_vue_pack_preview_review_write_and_uninstall() {
    acceptance(
        "CODESTYLES_TEST_VUE_PACK",
        "vue",
        NativeLanguage::Vue,
        "vue",
        "<template><div>{{ message }}</div></template>\n<script setup>const message='hello'</script>\n<style>.card{color:red}</style>\n",
        Some("<template><div>unclosed"),
    );
}

#[test]
#[ignore = "requires CODESTYLES_TEST_JAVA_PACK matching a reviewed embedded candidate digest"]
fn real_java_pack_preview_review_write_and_uninstall() {
    acceptance(
        "CODESTYLES_TEST_JAVA_PACK",
        "google-java-format",
        NativeLanguage::Java,
        "java",
        "class Example{void run(){System.out.println(1);}}\n",
        Some("class Broken { void method( {"),
    );
}

#[test]
#[ignore = "requires CODESTYLES_TEST_CSHARP_PACK matching a reviewed embedded candidate digest"]
fn real_csharp_pack_preview_review_write_and_uninstall() {
    acceptance(
        "CODESTYLES_TEST_CSHARP_PACK",
        "csharpier",
        NativeLanguage::CSharp,
        "cs",
        "class Example{void Run(){System.Console.WriteLine(1);}}\n",
        Some("class Broken { void Method( {"),
    );
}

#[test]
#[ignore = "requires CODESTYLES_TEST_CLANG_PACK matching a reviewed embedded candidate digest"]
fn real_c_pack_preview_review_write_and_uninstall() {
    acceptance(
        "CODESTYLES_TEST_CLANG_PACK",
        "clang-format",
        NativeLanguage::C,
        "c",
        "int main(void){int value=1;return value;}\n",
        // Verified incomplete-format rejection, not general compiler validation.
        Some("int main( {"),
    );
}

#[test]
#[ignore = "requires CODESTYLES_TEST_CLANG_PACK matching a reviewed embedded candidate digest"]
fn real_cpp_pack_preview_review_write_and_uninstall() {
    acceptance(
        "CODESTYLES_TEST_CLANG_PACK",
        "clang-format",
        NativeLanguage::Cpp,
        "cpp",
        "class Example{public:int value(){return 1;}};\n",
        // Verified incomplete-format rejection, not general compiler validation.
        Some("int main( {"),
    );
}

#[test]
#[ignore = "requires CODESTYLES_TEST_CLANG_PACK matching a reviewed embedded candidate digest"]
fn real_clang_pack_c_and_cpp_preview_review_write_and_uninstall() {
    real_c_pack_preview_review_write_and_uninstall();
    real_cpp_pack_preview_review_write_and_uninstall();
    // C++ project mode includes the common shared .h suffix without changing
    // the real source/diff/write identity to its synthetic stdin hint.
    acceptance(
        "CODESTYLES_TEST_CLANG_PACK",
        "clang-format",
        NativeLanguage::Cpp,
        "h",
        "class Example{public:int value(){return 1;}};\n",
        Some("int main( {"),
    );
}
