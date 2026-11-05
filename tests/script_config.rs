//! Explicit real-pack acceptance; never downloads runtimes or executes user files.
use codestyles::{
    backend::{Backend, FormatRequest},
    comparison::SourceLanguage,
    config::ConfigDocument,
    project::{NativeLanguage, ProjectAction, ProjectRequest},
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
fn format(
    backend: &Backend,
    config: &ConfigDocument,
    path: &str,
    source: &str,
) -> Result<String, String> {
    backend.format(
        &FormatRequest {
            source: source.into(),
            source_path: path.into(),
            config: config.snapshot(),
        },
        cancel(),
    )
}
fn install(manager: &ToolchainManager, id: &str, variable: &str) {
    let archive = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} required"));
    drop(
        manager
            .install_verified_archive(id, Path::new(&archive), cancel(), |_| {})
            .unwrap(),
    );
}
fn install_all(manager: &ToolchainManager) {
    for (id, variable) in [
        ("dprint", "CODESTYLES_DPRINT_ARCHIVE"),
        ("shfmt", "CODESTYLES_SHFMT_ARCHIVE"),
        ("stylua", "CODESTYLES_STYLUA_ARCHIVE"),
        ("dockerfile", "CODESTYLES_DOCKERFILE_ARCHIVE"),
        ("caddyfmt", "CODESTYLES_CADDYFMT_ARCHIVE"),
    ] {
        install(manager, id, variable);
    }
}

#[test]
fn rejected_source_and_settings_fail_before_missing_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    let backend = Backend::managed().with_toolchain_manager(manager.clone());
    let mut config = ConfigDocument::standalone(&[]).unwrap();
    for (path, source, expected) in [
        ("unsafe.sh", "#!/bin/zsh\necho hi", "shebang"),
        (
            "Dockerfile",
            "RUN echo \"$(echo \"a  b\")\"",
            "substitutions",
        ),
        ("Dockerfile.lua", "ENV A a  b", "Legacy"),
        ("Dockerfile.rs", "SHELL [\"cmd\"]", "instruction"),
        (
            "Dockerfile",
            "LABEL message a\\\n RUN echo unexpected\n",
            "Legacy",
        ),
        (
            "Dockerfile",
            "ARG x=a\\  \n RUN echo unexpected\n",
            "Continued",
        ),
        ("Dockerfile", "COPY a\\\n/b /dest\n", "Continued"),
        ("Dockerfile", "COPY a\\\t b /dest\n", "backslash-tab"),
        ("Dockerfile", "RUN << EOF\nx\nEOF\n", "heredoc"),
        ("Caddyfile.lua", "respond \"unterminated", "Unterminated"),
    ] {
        let before = config.draft_text().to_owned();
        let error = format(&backend, &config, path, source).unwrap_err();
        assert!(error.contains(expected), "{path}: {error}");
        assert_eq!(config.draft_text(), before);
    }
    config
        .set(&["shfmt", "simplify"], serde_json::json!(true))
        .unwrap();
    assert!(
        format(&backend, &config, "safe.sh", "echo hi")
            .unwrap_err()
            .contains("simplify")
    );
    config.remove(&["shfmt", "simplify"]).unwrap();
    config
        .set(&["stylua", "sortRequires"], serde_json::json!(true))
        .unwrap();
    assert!(
        format(&backend, &config, "safe.lua", "return 1")
            .unwrap_err()
            .contains("sortRequires")
    );
    assert_eq!(manager.status("dprint").unwrap(), ToolchainStatus::Missing);
    assert_eq!(
        manager.status("caddyfmt").unwrap(),
        ToolchainStatus::Missing
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_script_filename_cannot_bypass_source_guards() {
    use std::os::unix::ffi::OsStringExt;
    let config = ConfigDocument::standalone(&[]).unwrap();
    let backend = Backend::managed();
    for name in [b"bad\xff.sh".to_vec(), b"Dockerfile.\xff.rs".to_vec()] {
        let request = FormatRequest {
            source: "ENV A a  b".into(),
            source_path: std::ffi::OsString::from_vec(name).into(),
            config: config.snapshot(),
        };
        assert!(
            backend
                .format(&request, cancel())
                .unwrap_err()
                .contains("UTF-8")
        );
    }
}

#[test]
#[ignore = "requires CODESTYLES_{DPRINT,SHFMT,STYLUA,DOCKERFILE,CADDYFMT}_ARCHIVE with trusted digests"]
fn real_scripts_preview_settings_review_apply_and_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    install_all(&manager);
    let backend = Backend::managed().with_toolchain_manager(manager.clone());
    for (language, scope, name, section) in [
        (
            SourceLanguage::Shell,
            NativeLanguage::Shell,
            "chosen.sh",
            "shfmt",
        ),
        (
            SourceLanguage::Lua,
            NativeLanguage::Lua,
            "chosen.lua",
            "stylua",
        ),
        (
            SourceLanguage::Luau,
            NativeLanguage::Luau,
            "chosen.luau",
            "stylua",
        ),
        (
            SourceLanguage::Dockerfile,
            NativeLanguage::Dockerfile,
            "Dockerfile",
            "dockerfile",
        ),
        (
            SourceLanguage::Caddyfile,
            NativeLanguage::Caddyfile,
            "Caddyfile",
            "caddyfmt",
        ),
    ] {
        let root = temp.path().join(language.extension());
        fs::create_dir(&root).unwrap();
        let config_path = root.join("dprint.jsonc");
        let settings = "{ // preserve comment\n\"metadata\":{\"untouched\":true}}\n";
        fs::write(&config_path, settings).unwrap();
        let mut config = ConfigDocument::load(&config_path).unwrap();
        let source = language.builtin_source();
        let output =
            format(&backend, &config, name, source).unwrap_or_else(|e| panic!("{language:?}: {e}"));
        assert_eq!(format(&backend, &config, name, &output).unwrap(), output);
        if language == SourceLanguage::Dockerfile {
            config
                .set(&[section, "indentStages"], serde_json::json!(true))
                .unwrap();
            config
                .set(&[section, "indentWidth"], serde_json::json!(4))
                .unwrap();
        } else if language != SourceLanguage::Caddyfile {
            config
                .set(&[section, "useTabs"], serde_json::json!(false))
                .unwrap();
            config
                .set(&[section, "indentWidth"], serde_json::json!(4))
                .unwrap();
        }
        let changed = format(&backend, &config, name, source).unwrap();
        if language != SourceLanguage::Caddyfile {
            assert_ne!(changed, output, "{language:?} settings should reach engine");
        }
        assert_eq!(format(&backend, &config, name, &changed).unwrap(), changed);
        assert_eq!(fs::read_to_string(&config_path).unwrap(), settings);
        config.save().unwrap();
        let saved = fs::read(&config_path).unwrap();
        let selected = root.join(name);
        let other = root.join("untouched.txt");
        fs::write(&selected, source).unwrap();
        fs::write(&other, "leave this alone").unwrap();
        let check =
            ProjectRequest::new_native(root.clone(), &config, ProjectAction::Check, true, scope)
                .unwrap();
        let report = backend.execute_project(&check, cancel()).unwrap();
        assert_eq!(report.checked_files, 1);
        assert_eq!(fs::read_to_string(&selected).unwrap(), source);
        if source != changed {
            assert_eq!(report.files.len(), 1);
            let apply = ProjectRequest::from_native_review(
                root.clone(),
                &config,
                vec![selected.clone()],
                &report,
                true,
                scope,
            )
            .unwrap();
            backend.execute_project(&apply, cancel()).unwrap();
            assert_eq!(fs::read_to_string(&selected).unwrap(), changed);
        }
        assert_eq!(fs::read_to_string(&other).unwrap(), "leave this alone");
        assert_eq!(fs::read(&config_path).unwrap(), saved);
    }
    let config = ConfigDocument::standalone(&[]).unwrap();
    // Caddy now explicitly requires the verified dprint engine.
    manager.uninstall("dprint").unwrap();
    assert!(
        format(
            &backend,
            &config,
            "Caddyfile",
            ":8080 {\nrespond \"ok\"\n}\n"
        )
        .unwrap_err()
        .contains("requires the managed dprint")
    );
    assert!(format(&backend, &config, "test.lua", "return 1").is_err());
    install(&manager, "dprint", "CODESTYLES_DPRINT_ARCHIVE");
    let resolved = manager.resolve_cancellable("shfmt", &cancel()).unwrap();
    let plugin = resolved.wasm_plugins[0].clone();
    drop(resolved);
    fs::write(&plugin, b"corrupt").unwrap();
    assert!(format(&backend, &config, "test.sh", "echo hi").is_err());
    install(&manager, "shfmt", "CODESTYLES_SHFMT_ARCHIVE");
    assert!(format(&backend, &config, "test.sh", "echo hi").is_ok());
    for id in ["shfmt", "stylua", "dockerfile", "caddyfmt"] {
        manager.uninstall(id).unwrap();
        assert_eq!(manager.status(id).unwrap(), ToolchainStatus::Missing);
    }
    let request = FormatRequest {
        source: "echo hi".into(),
        source_path: "test.sh".into(),
        config: config.snapshot(),
    };
    assert!(
        backend
            .format(&request, Arc::new(AtomicBool::new(true)))
            .unwrap_err()
            .contains("cancelled")
    );
}

#[test]
#[ignore = "requires all five trusted runtime archives"]
fn real_parser_errors_and_unsafe_project_inputs_preserve_originals() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    install_all(&manager);
    let backend = Backend::managed().with_toolchain_manager(manager);
    for (scope, name, source) in [
        (NativeLanguage::Shell, "bad.sh", "if then\n"),
        (NativeLanguage::Lua, "bad.lua", "local =\n"),
        (NativeLanguage::Luau, "bad.luau", "type =\n"),
        (
            NativeLanguage::Dockerfile,
            "Dockerfile",
            "RUN echo ${x:-a  b}\n",
        ),
        (
            NativeLanguage::Caddyfile,
            "Caddyfile",
            "respond \"unterminated\n",
        ),
    ] {
        let root = temp.path().join(name.replace('.', "_"));
        fs::create_dir(&root).unwrap();
        let config_path = root.join("dprint.jsonc");
        fs::write(&config_path, "{ /* keep */ }\n").unwrap();
        let config = ConfigDocument::load(&config_path).unwrap();
        let file = root.join(name);
        fs::write(&file, source).unwrap();
        assert!(format(&backend, &config, name, source).is_err(), "{name}");
        let request =
            ProjectRequest::new_native(root, &config, ProjectAction::Check, true, scope).unwrap();
        assert!(backend.execute_project(&request, cancel()).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), source);
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "{ /* keep */ }\n"
        );
    }
}

#[test]
#[ignore = "requires all five trusted runtime archives"]
fn real_literals_aliases_and_heredocs_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    install_all(&manager);
    let backend = Backend::managed().with_toolchain_manager(manager);
    let config = ConfigDocument::standalone(&[]).unwrap();
    for source in [
        "FROM alpine\nRUN <<EOF\nprintf 'a  b\\n'\nEOF\n",
        "FROM alpine\nRUN <<'EOF'\nprintf '${x:-a  b}\\n'\nEOF\n",
        "FROM alpine\nCOPY <<EOF /message\na  b  \nEOF\n",
    ] {
        for name in ["Dockerfile", "Dockerfile.rs", "custom.dockerfile"] {
            let output = format(&backend, &config, name, source)
                .unwrap_or_else(|e| panic!("{name} {source:?}: {e}"));
            assert!(output.contains("a  b"));
            assert_eq!(format(&backend, &config, name, &output).unwrap(), output);
        }
    }
    let caddy = ":8080 {\nrespond `a  b`\nimport does-not-exist\n}\n";
    let output = format(&backend, &config, "Caddyfile.lua", caddy).unwrap();
    assert!(output.contains("`a  b`"));
    assert!(output.contains("import does-not-exist"));
    for (name, source, preserved) in [
        (
            "literal.lua",
            "local b=require('b')\nlocal a=require('a')\nreturn [[a  b]]\n",
            "[[a  b]]",
        ),
        (
            "literal.luau",
            "local n:number=1\nreturn `value {n}`\n",
            "`value {n}`",
        ),
    ] {
        let output = format(&backend, &config, name, source).unwrap();
        assert!(output.contains(preserved));
        if name.ends_with(".lua") {
            assert!(
                output.find("require(\"b\")").unwrap() < output.find("require(\"a\")").unwrap()
            );
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires trusted dprint/shfmt archives and /bin/bash for six fixed, side-effect-free fixtures"]
fn real_shell_fixed_fixtures_preserve_observable_behavior() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    fn run(source: &str) -> std::process::Output {
        let mut child = Command::new("/bin/bash")
            .args(["--noprofile", "--norc"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
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
        child.wait_with_output().unwrap()
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    install(&manager, "dprint", "CODESTYLES_DPRINT_ARCHIVE");
    install(&manager, "shfmt", "CODESTYLES_SHFMT_ARCHIVE");
    let backend = Backend::managed().with_toolchain_manager(manager);
    let config = ConfigDocument::standalone(&[]).unwrap();
    for source in [
        "printf '%s\\n' 'a  b'\n",
        "for n in a b;do printf '<%s>' \"$n\";done\n",
        "x=$(printf 'a  b');printf '%s\\n' \"$x\"\n",
        "cat <<'EOF'\na  b $literal\nEOF\n",
        "n=2;printf '%s\\n' \"$((n+3))\"\n",
        "case a in a) printf yes;; *) printf no;;esac\n",
    ] {
        let output = format(&backend, &config, "fixture.bash", source).unwrap();
        let before = run(source);
        let after = run(&output);
        assert_eq!(before.status.code(), after.status.code());
        assert_eq!(before.stdout, after.stdout);
        assert_eq!(before.stderr, after.stderr);
    }
}

#[test]
#[ignore = "requires trusted CODESTYLES_DPRINT_ARCHIVE and CODESTYLES_CADDYFMT_ARCHIVE"]
fn real_caddy_wasm_requires_engine_and_preserves_isolation_and_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    let backend = Backend::managed().with_toolchain_manager(manager.clone());
    let mut config = ConfigDocument::standalone(&[]).unwrap();
    install(&manager, "caddyfmt", "CODESTYLES_CADDYFMT_ARCHIVE");
    assert!(
        format(&backend, &config, "Caddyfile", "localhost {}")
            .unwrap_err()
            .contains("requires the managed dprint")
    );
    install(&manager, "dprint", "CODESTYLES_DPRINT_ARCHIVE");
    config.set(&["useTabs"], serde_json::json!(false)).unwrap();
    config.set(&["indentWidth"], serde_json::json!(8)).unwrap();
    let original = config.draft_text().to_owned();
    let source = "import missing.caddy\nlocalhost {\nrespond \"literal {$SECRET} {host}\"\n}\n";
    let expected = "import missing.caddy\nlocalhost {\n\trespond \"literal {$SECRET} {host}\"\n}\n";
    for path in [
        "Caddyfile",
        "Caddyfile.lua",
        "Caddyfile.dockerfile",
        "site.caddyfile",
    ] {
        assert_eq!(format(&backend, &config, path, source).unwrap(), expected);
    }
    assert_eq!(config.draft_text(), original);
    let runtime = manager.resolve_cancellable("caddyfmt", &cancel()).unwrap();
    assert!(runtime.executable.as_os_str().is_empty());
    let plugin = runtime.wasm_plugins[0].clone();
    drop(runtime);
    fs::write(plugin, b"corrupt").unwrap();
    assert!(format(&backend, &config, "Caddyfile", source).is_err());
    install(&manager, "caddyfmt", "CODESTYLES_CADDYFMT_ARCHIVE");
    assert_eq!(
        format(&backend, &config, "Caddyfile", source).unwrap(),
        expected
    );
    manager.uninstall("dprint").unwrap();
    assert!(
        format(&backend, &config, "Caddyfile", "respond \"unterminated")
            .unwrap_err()
            .contains("Unterminated")
    );
    assert!(
        format(&backend, &config, "Caddyfile", source)
            .unwrap_err()
            .contains("requires the managed dprint")
    );
    manager.uninstall("caddyfmt").unwrap();
    assert!(
        format(&backend, &config, "Caddyfile", source)
            .unwrap_err()
            .contains("Install Caddyfile support")
    );
}
