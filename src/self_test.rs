//! Headless diagnostics for the actual packaged executable, before GPUI starts.
//! This exercises the production standalone-preview backend, not a substitute CLI.
use crate::{
    backend::{Backend, FormatRequest, cache_directory},
    comparison::{BuiltinSample, SourceLanguage},
    resources,
    schema::PinnedPlugin,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

pub fn requested_report(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<PathBuf>, String> {
    let args: Vec<_> = args.into_iter().collect();
    // Finder and desktop launchers may supply platform-specific arguments.
    // Preserve normal GUI launch behavior unless diagnostics were requested.
    if !args.iter().any(|arg| arg == "--self-test") {
        return Ok(None);
    }
    match args.as_slice() {
        [flag, path] if flag == "--self-test" && !path.is_empty() => Ok(Some(PathBuf::from(path))),
        _ => Err("Usage: codestyles [--self-test <report-path>]".into()),
    }
}

/// Writes a JSON report even when a resource or formatter check fails. A report
/// write error is itself a failure; no GUI or font rendering is attempted here.
pub fn run(report_path: &Path) -> Result<(), String> {
    let started = Instant::now();
    let mut report = json!({
        "format_version": 1,
        "application": "CodeStyles",
        "application_version": env!("CARGO_PKG_VERSION"),
        "status": "failed",
        "mode": "headless_standalone_preview",
        "executable": std::env::current_exe().ok(),
        "target": resources::target().ok(),
        "debug_build": cfg!(debug_assertions),
        "dprint_version": "0.63.0",
        "plugins": PinnedPlugin::ALL.map(|plugin| json!({
            "name": plugin.section(), "version": plugin.version(), "file": plugin.filename()
        })),
        "checks": [],
        "gui": {"status": "not_tested", "reason": "Self-test runs before GPUI initialization; no window, rendering, or keyboard interactions are tested."},
        "fonts": {"status": "not_tested", "source": "system_installed_fonts", "embedded_fonts": false,
            "reason": "GPUI Kit embeds icons, not fonts. Installed font availability and glyph rendering require a native GUI check."},
        "network": {"isolation_enforced_by_self_test": false,
            "scope": "Only standalone configs mapped to verified bundled plugins are exercised; imported remote or process-plugin configs are excluded."}
    });
    let outcome = run_checks(&mut report);
    if let Err(error) = &outcome {
        report["error"] = json!(error);
    } else {
        report["status"] = json!("passed");
    }
    report["elapsed_ms"] = json!(started.elapsed().as_millis());
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::write(report_path, bytes).map_err(|e| {
        format!(
            "Cannot write self-test report {}: {e}",
            report_path.display()
        )
    })?;
    outcome
}

fn run_checks(report: &mut Value) -> Result<(), String> {
    let location = resources::resolve_resources()?;
    report["resources"] = json!({"root": location.root, "discovery": location.discovery});
    let backend = resources::backend_from_root(&location.root)?;
    report["resources"]["checksums_verified"] = json!(true);
    report["dprint_executable"] = json!(backend.executable());
    report["cache_directory"] = json!(cache_directory(std::env::consts::OS, |key| {
        std::env::var_os(key)
    })?);
    let workspace = tempfile::Builder::new()
        .prefix("CodeStyles self-test 样例 ")
        .tempdir()
        .map_err(|e| format!("Cannot create self-test workspace: {e}"))?;
    let mut failures = Vec::new();
    for language in SourceLanguage::ALL
        .into_iter()
        .filter(|language| language.is_bundled())
    {
        let mut check = json!({
            "language": language.label(), "extension": language.extension(),
            "plugin": language.plugin().section(), "status": "failed"
        });
        match check_language(&backend, language, workspace.path()) {
            Ok(evidence) => {
                check["status"] = json!("passed");
                check["evidence"] = evidence;
            }
            Err(error) => {
                failures.push(format!("{}: {error}", language.label()));
                check["error"] = json!(error);
            }
        }
        report["checks"]
            .as_array_mut()
            .expect("checks array")
            .push(check);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn check_language(
    backend: &Backend,
    language: SourceLanguage,
    workspace: &Path,
) -> Result<Value, String> {
    let source = BuiltinSample::Overview.source(language);
    let document = backend.standalone_config()?;
    let request = FormatRequest {
        source: source.into(),
        source_path: workspace.join(format!("示例 with spaces.{}", language.extension())),
        config: document.snapshot(),
    };
    let output = backend.format(&request, Arc::new(AtomicBool::new(false)))?;
    if output.trim().is_empty() || output == source {
        return Err("Bundled formatter did not change the deliberately unformatted sample".into());
    }
    let second = backend.format(
        &FormatRequest {
            source: output.clone(),
            ..request
        },
        Arc::new(AtomicBool::new(false)),
    )?;
    if second != output {
        return Err("Formatting the preview again was not idempotent".into());
    }
    Ok(json!({
        "source_bytes": source.len(), "output_bytes": output.len(),
        "output_sha256": format!("{:x}", Sha256::digest(output.as_bytes())),
        "changed": true, "idempotent": true
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_requires_exact_arguments_and_preserves_unicode_path() {
        assert_eq!(requested_report(Vec::<OsString>::new()).unwrap(), None);
        assert_eq!(requested_report(["-psn_0_12345".into()]).unwrap(), None);
        assert_eq!(requested_report(["--unknown".into()]).unwrap(), None);
        assert_eq!(
            requested_report(["--self-test".into(), "报告 with spaces.json".into()]).unwrap(),
            Some(PathBuf::from("报告 with spaces.json"))
        );
        for args in [
            vec!["--self-test"],
            vec!["--self-test", ""],
            vec!["--self-test", "a", "extra"],
        ] {
            assert!(requested_report(args.into_iter().map(OsString::from)).is_err());
        }
    }

    #[test]
    fn real_bundled_backend_formats_all_ten_self_test_samples() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
        let backend = resources::backend_from_root(&root).unwrap();
        let workspace = tempfile::tempdir().unwrap();
        for language in SourceLanguage::ALL
            .into_iter()
            .filter(|language| language.is_bundled())
        {
            let evidence = check_language(&backend, language, workspace.path())
                .unwrap_or_else(|error| panic!("{}: {error}", language.label()));
            assert_eq!(evidence["changed"], true);
            assert_eq!(evidence["idempotent"], true);
        }
    }

    #[test]
    fn bundled_self_test_samples_cover_all_five_plugins_and_ten_extensions() {
        let extensions: std::collections::HashSet<_> = SourceLanguage::ALL
            .into_iter()
            .filter(|language| language.is_bundled())
            .map(|language| language.extension())
            .collect();
        let plugins: std::collections::HashSet<_> = SourceLanguage::ALL
            .into_iter()
            .filter(|language| language.is_bundled())
            .map(|language| language.plugin())
            .collect();
        assert_eq!(extensions.len(), 10);
        assert_eq!(plugins.len(), 5);
        for language in SourceLanguage::ALL
            .into_iter()
            .filter(|language| language.is_bundled())
        {
            assert!(!BuiltinSample::Overview.source(language).is_empty());
        }
    }
}
