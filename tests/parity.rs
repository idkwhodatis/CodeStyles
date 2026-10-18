use codestyles::backend::{Backend, FormatRequest};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool},
};

#[cfg(unix)]
use std::{
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

fn assets() -> (PathBuf, Vec<PathBuf>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let target = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else {
        "x86_64-pc-windows-msvc"
    };
    let executable = root.join("bin").join(target).join(if cfg!(windows) {
        "dprint.exe"
    } else {
        "dprint"
    });
    assert!(
        executable.exists(),
        "Missing bundled CLI: {}",
        executable.display()
    );
    (
        executable,
        vec![
            root.join("plugins/typescript-0.96.1.wasm"),
            root.join("plugins/json-0.25.2.wasm"),
        ],
    )
}
fn cli(exe: &Path, config: &Path, path: &Path, input: &str) -> String {
    let mut child = Command::new(exe)
        .arg("fmt")
        .arg("--config")
        .arg(config)
        .arg("--stdin")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
fn request(
    source: &str,
    path: PathBuf,
    config: Option<PathBuf>,
    indent: u8,
    semi: &str,
) -> FormatRequest {
    FormatRequest {
        source: source.into(),
        source_path: path,
        config_path: config,
        indent_width: indent,
        semi_colons: semi.into(),
    }
}

#[test]
fn all_six_languages_match_saved_configuration_byte_for_byte() {
    let (exe, plugins) = assets();
    let backend = Backend::new(exe.clone(), plugins.clone());
    let dir = tempfile::tempdir().unwrap();
    for (source, extension) in [
        (
            "function hello(){console.log('hello');return {a:1,b:[2,3]};}",
            "ts",
        ),
        (
            "function hello(){console.log('hello');return {a:1,b:[2,3]};}",
            "js",
        ),
        (
            "const Hello=()=>{return <div className='hello'><span>World</span></div>}",
            "tsx",
        ),
        (
            "const Hello=()=>{return <div className='hello'><span>World</span></div>}",
            "jsx",
        ),
        ("{\"a\":{\"b\":[1,2,3]}}", "json"),
        ("{ /* comment */ \"a\":{\"b\":[1,2,3],},}", "jsonc"),
    ] {
        for indent in [2, 4] {
            for semi in ["always", "asi"] {
                let saved = dir.path().join("saved-config.json");
                fs::write(&saved, serde_json::to_string(&json!({"plugins":plugins,"indentWidth":indent,"typescript":{"semiColons":semi}})).unwrap()).unwrap();
                let path = dir.path().join(format!("example.{extension}"));
                let output = backend
                    .format(
                        &request(source, path.clone(), None, indent, semi),
                        Arc::new(AtomicBool::new(false)),
                    )
                    .unwrap();
                assert_eq!(output, cli(&exe, &saved, &path, source));
            }
        }
    }
}

#[test]
fn relative_extends_config_dir_and_overrides_match_saved_project_config() {
    let (exe, plugins) = assets();
    let backend = Backend::new(exe.clone(), plugins.clone());
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("configs")).unwrap();
    fs::copy(&plugins[0], dir.path().join("typescript.wasm")).unwrap();
    fs::write(
        dir.path().join("configs/base.json"),
        r#"{"typescript":{"quoteStyle":"preferSingle","lineWidth":60}}"#,
    )
    .unwrap();
    let original = dir.path().join("dprint.jsonc");
    let text = r#"{ // do not mutate me
      "extends": "./configs/base.json",
      "plugins": ["${configDir}/typescript.wasm"],
      "typescript": {"overrides":[{"files":"**/special.ts","indentWidth":8}]},
    }"#;
    fs::write(&original, text).unwrap();
    let baseline = dir.path().join("saved-config.json");
    fs::write(&baseline, serde_json::to_string(&json!({"extends":"./configs/base.json","plugins":["${configDir}/typescript.wasm"],"indentWidth":4,"typescript":{"semiColons":"asi","overrides":[{"files":"**/special.ts","indentWidth":8}]}})).unwrap()).unwrap();
    for name in ["normal.ts", "special.ts"] {
        let path = dir.path().join(name);
        let source = "function hello(){console.log(\"hello\");return 1;}";
        let output = backend
            .format(
                &request(source, path.clone(), Some(original.clone()), 4, "asi"),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        assert_eq!(output, cli(&exe, &baseline, &path, source));
        assert!(output.contains("'hello'"), "extends must take effect");
        if name == "special.ts" {
            assert!(
                output.contains("\n        console"),
                "override must take effect"
            );
        }
    }
    assert_eq!(fs::read_to_string(&original).unwrap(), text);
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".codestyles-draft-")
    }));
}

#[test]
fn invalid_jsonc_is_reported_without_modifying_original() {
    let (exe, plugins) = assets();
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("dprint.jsonc");
    fs::write(&config, "{ broken").unwrap();
    let result = Backend::new(exe, plugins).format(
        &request("let a=1", "a.ts".into(), Some(config.clone()), 2, "always"),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(result.unwrap_err().contains("Invalid JSONC"));
    assert_eq!(fs::read_to_string(config).unwrap(), "{ broken");
}

#[cfg(unix)]
fn slow_backend(dir: &Path) -> Backend {
    use std::os::unix::fs::PermissionsExt;
    let executable = dir.join("fake-dprint");
    fs::write(&executable,"#!/usr/bin/env python3\nimport os,time,sys\nopen(os.path.join(os.path.dirname(__file__),'pid'),'w').write(str(os.getpid()))\nsys.stderr.write('x'*200000)\nsys.stderr.flush()\ntime.sleep(60)\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    Backend::new(executable, vec![])
}

#[test]
#[cfg(unix)]
fn cancellation_kills_reaps_and_cleans_draft_with_blocked_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let backend = slow_backend(dir.path());
    let config = dir.path().join("dprint.json");
    fs::write(&config, "{}").unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let request = request(
        &"x".repeat(2_000_000),
        "input.ts".into(),
        Some(config),
        2,
        "always",
    );
    let worker = thread::spawn(move || backend.format(&request, worker_cancelled));
    let start = Instant::now();
    while fs::read_to_string(dir.path().join("pid")).map_or(true, |pid| pid.trim().is_empty()) {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    cancelled.store(true, Ordering::Release);
    assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(start.elapsed() < Duration::from_secs(5));
    #[cfg(target_os = "linux")]
    let pid = fs::read_to_string(dir.path().join("pid")).unwrap();
    #[cfg(target_os = "linux")]
    assert!(
        !Path::new("/proc").join(pid.trim()).exists(),
        "child must be reaped"
    );
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".codestyles-draft-")
    }));
}

#[test]
#[cfg(unix)]
fn timeout_terminates_child() {
    let dir = tempfile::tempdir().unwrap();
    let backend = slow_backend(dir.path()).with_timeout(Duration::from_millis(100));
    let start = Instant::now();
    let result = backend.format(
        &request("x", "input.ts".into(), None, 2, "always"),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(result.unwrap_err().contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn changing_settings_never_mutates_source_buffer_or_file() {
    let (exe, plugins) = assets();
    let backend = Backend::new(exe, plugins);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("original.ts");
    let source = "function hello(){console.log('hello');return 1;}";
    fs::write(&path, source).unwrap();
    let mut input = request(source, path.clone(), None, 2, "always");
    let first = backend
        .format(&input, Arc::new(AtomicBool::new(false)))
        .unwrap();
    input.indent_width = 4;
    input.semi_colons = "asi".into();
    let second = backend
        .format(&input, Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_ne!(first, second, "controls must change output");
    assert_eq!(input.source, source, "immutable source buffer");
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        source,
        "immutable source file"
    );
}

#[test]
fn changed_bundled_executable_is_rejected_before_execution() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let (exe, _) = assets();
    let resource_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let relative = exe.strip_prefix(&resource_root).unwrap();
    fs::create_dir_all(dir.path().join(relative).parent().unwrap()).unwrap();
    fs::write(dir.path().join(relative), b"tampered").unwrap();
    let manifest = json!({"dprint_version":"0.63.0", "files":[{"path":relative.to_string_lossy().replace('\\', "/"),"sha256":format!("{:x}", Sha256::digest(b"expected"))}]});
    fs::write(
        dir.path().join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let result = codestyles::resources::backend_from_root(dir.path());
    assert!(result.unwrap_err().contains("checksum mismatch"));
}

#[test]
fn unpinned_cli_version_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("manifest.json"),
        r#"{"dprint_version":"0.99.0","files":[]}"#,
    )
    .unwrap();
    assert!(
        codestyles::resources::backend_from_root(dir.path())
            .unwrap_err()
            .contains("Unsupported bundled dprint version")
    );
}

#[test]
fn inherit_true_uses_ancestor_config_exactly_like_saved_cli_config() {
    let (exe, plugins) = assets();
    let dir = tempfile::tempdir().unwrap();
    let child_dir = dir.path().join("child");
    fs::create_dir(&child_dir).unwrap();
    fs::write(
        dir.path().join("dprint.json"),
        serde_json::to_vec(
            &json!({"plugins":plugins,"typescript":{"quoteStyle":"preferSingle","lineWidth":70}}),
        )
        .unwrap(),
    )
    .unwrap();
    let original = child_dir.join("dprint.jsonc");
    let text = "{ // inherit parent settings and plugins\n\"inherit\":true,\"typescript\":{\"bracePosition\":\"nextLine\"}}";
    fs::write(&original, text).unwrap();
    let saved = child_dir.join("saved.json");
    fs::write(&saved, serde_json::to_vec(&json!({"inherit":true,"indentWidth":4,"typescript":{"bracePosition":"nextLine","semiColons":"asi"}})).unwrap()).unwrap();
    let path = child_dir.join("example.ts");
    let source = "function hello(){console.log(\"inherited\");return 1;}";
    let output = Backend::new(exe.clone(), plugins)
        .format(
            &request(source, path.clone(), Some(original.clone()), 4, "asi"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(output, cli(&exe, &saved, &path, source));
    assert!(
        output.contains("'inherited'"),
        "ancestor quote style and plugin must be inherited"
    );
    assert_eq!(fs::read_to_string(original).unwrap(), text);
}

#[test]
#[cfg(unix)]
fn child_receives_literal_argv_stdin_and_dedicated_cache_environment() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("inspect-child");
    fs::write(&executable,"#!/usr/bin/env python3\nimport os,sys,json\nprint(json.dumps({'argv':sys.argv[1:],'cache':os.environ.get('DPRINT_CACHE_DIR'),'stdin':sys.stdin.read()}))\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let path = PathBuf::from("literal file;$(do-not-run).ts");
    let source = "const text = 'literal $input';";
    let result = Backend::new(executable, vec![])
        .format(
            &request(source, path.clone(), None, 2, "always"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["argv"][0], "fmt");
    assert_eq!(result["argv"][1], "--config-discovery=false");
    assert_eq!(result["argv"][2], "--config");
    assert_eq!(
        result["argv"][4],
        format!("--stdin={}", path.to_string_lossy())
    );
    assert_eq!(result["argv"].as_array().unwrap().len(), 5);
    assert_eq!(result["stdin"], source);
    let cache = PathBuf::from(result["cache"].as_str().unwrap());
    assert!(cache.is_dir());
    if let Some(explicit) = std::env::var_os("DPRINT_CACHE_DIR") {
        assert_eq!(cache, PathBuf::from(explicit));
    } else {
        assert!(cache.ends_with("CodeStyles/dprint"));
    }
}

#[test]
fn verified_resource_bundle_formats_through_real_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let backend = codestyles::resources::backend_from_root(&root).unwrap();
    let output = backend
        .format(
            &request(
                "function hello(){return 1}",
                "bundle-smoke.ts".into(),
                None,
                4,
                "always",
            ),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    assert_eq!(output, "function hello() {\n    return 1;\n}\n");
}

#[test]
fn relative_source_paths_starting_with_dash_are_literal() {
    let (exe, plugins) = assets();
    let backend = Backend::new(exe, plugins);
    for name in ["-sample.ts", "--help.ts", "--version.ts"] {
        let output = backend
            .format(
                &request("const x=1", name.into(), None, 2, "always"),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        assert_eq!(output, "const x = 1;\n", "source path: {name}");
    }
}
