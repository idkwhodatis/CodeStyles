//! Explicit real-WASM acceptance. All archives must match the embedded catalog;
//! this test never fetches tools, discovers PATH executables, or connects to a DB.
use codestyles::{
    backend::{Backend, FormatRequest},
    comparison::SourceLanguage,
    config::ConfigDocument,
    project::{NativeLanguage, ProjectAction, ProjectRequest},
    toolchains::{ToolchainManager, ToolchainStatus},
};
use serde_json::json;
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
    document: &ConfigDocument,
    extension: &str,
    source: &str,
) -> Result<String, String> {
    backend.format(
        &FormatRequest {
            source: source.into(),
            source_path: format!("preview.{extension}").into(),
            config: document.snapshot(),
        },
        cancel(),
    )
}

#[test]
#[ignore = "requires CODESTYLES_{DPRINT,VUE,YAML,SQL}_ARCHIVE with embedded trusted digests"]
fn real_web_config_sql_preview_review_apply_and_offline_lifecycle() {
    // One shared private install set keeps disk use bounded. Runtime leases are
    // dropped before uninstall; project source directories are independently scoped.
    let temp = tempfile::tempdir().unwrap();
    let manager = ToolchainManager::at_managed_root(temp.path().join("managed")).unwrap();
    for (id, variable) in [
        ("dprint", "CODESTYLES_DPRINT_ARCHIVE"),
        ("vue", "CODESTYLES_VUE_ARCHIVE"),
        ("yaml", "CODESTYLES_YAML_ARCHIVE"),
        ("sql", "CODESTYLES_SQL_ARCHIVE"),
    ] {
        let archive = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} required"));
        drop(
            manager
                .install_verified_archive(id, Path::new(&archive), cancel(), |_| {})
                .unwrap(),
        );
    }
    let backend = Backend::managed().with_toolchain_manager(manager.clone());
    let settings = "{ // keep this comment\n\"sql\":{\"dialect\":\"postgresql\"},\"customMetadata\":{\"keep\":true}}\n";
    for (language, scope, section) in [
        (SourceLanguage::Html, NativeLanguage::Html, "markup"),
        (SourceLanguage::Css, NativeLanguage::Css, "malva"),
        (SourceLanguage::Scss, NativeLanguage::Scss, "malva"),
        (SourceLanguage::Yaml, NativeLanguage::Yaml, "yaml"),
        (SourceLanguage::Sql, NativeLanguage::Sql, "sql"),
    ] {
        let root = temp.path().join(language.extension());
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let config_path = root.join("dprint.jsonc");
        fs::write(&config_path, settings).unwrap();
        let mut config = ConfigDocument::load(&config_path).unwrap();
        let source = language.builtin_source();
        let output = format(&backend, &config, language.extension(), source)
            .unwrap_or_else(|error| panic!("{language:?}: {error}"));
        assert_eq!(
            format(&backend, &config, language.extension(), &output).unwrap(),
            output,
            "{language:?} idempotence"
        );
        assert_eq!(fs::read_to_string(&config_path).unwrap(), settings);
        // Explicit style changes must affect actual formatter output, not only UI state.
        if language == SourceLanguage::Sql {
            config.set(&[section, "casing"], json!("upper")).unwrap();
        } else {
            config.set(&[section, "indentWidth"], json!(4)).unwrap();
        }
        let changed = format(&backend, &config, language.extension(), source).unwrap();
        assert_ne!(
            changed, output,
            "{language:?} changed settings must reach its engine"
        );
        assert_eq!(
            format(&backend, &config, language.extension(), &changed).unwrap(),
            changed
        );
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            settings,
            "preview cannot save settings"
        );
        config.save().unwrap();
        let saved_settings = fs::read(&config_path).unwrap();
        assert!(String::from_utf8_lossy(&saved_settings).contains("keep this comment"));
        assert!(String::from_utf8_lossy(&saved_settings).contains("customMetadata"));

        if language == SourceLanguage::Yaml {
            // Token preservation is not claimed as a full semantic parser oracle.
            for token in [
                "&defaults",
                "*defaults",
                "Keep this line.",
                "And this indentation.",
                "A folded message",
                "remains a message.",
                "# Reuse defaults",
            ] {
                assert!(changed.contains(token), "YAML lost {token}");
            }
        }
        if language == SourceLanguage::Html {
            assert!(changed.contains("<script>"));
            assert!(changed.contains("<style>"));
            assert!(changed.contains("Hello <em>world</em>!"));
            assert!(
                changed.contains("const items ="),
                "embedded script must be formatted"
            );
            assert!(
                changed.contains("color: #"),
                "embedded CSS must be formatted"
            );
        }
        let selected = root.join(format!("selected.{}", language.extension()));
        let remaining = root.join(format!("remaining.{}", language.extension()));
        fs::write(&selected, source).unwrap();
        fs::write(&remaining, source).unwrap();
        let untouched = "not this selected language; preserve byte for byte\n";
        let other = root.join(if language == SourceLanguage::Css {
            "other.scss"
        } else {
            "other.css"
        });
        fs::write(&other, untouched).unwrap();
        let check =
            ProjectRequest::new_native(root.clone(), &config, ProjectAction::Check, true, scope)
                .unwrap();
        let report = backend.execute_project(&check, cancel()).unwrap();
        assert_eq!(
            report.checked_files, 2,
            "{language:?} selected-language scope"
        );
        assert_eq!(report.files.len(), 2);
        assert_eq!(fs::read_to_string(&selected).unwrap(), source);
        assert_eq!(fs::read_to_string(&remaining).unwrap(), source);
        let apply = ProjectRequest::from_native_review(
            root.clone(),
            &config,
            vec![selected.clone()],
            &report,
            true,
            scope,
        )
        .unwrap();
        let reviewed = backend.execute_project(&apply, cancel()).unwrap();
        assert_eq!(reviewed.files.len(), 1);
        assert_eq!(fs::read_to_string(&selected).unwrap(), changed);
        assert_eq!(fs::read_to_string(&remaining).unwrap(), source);
        // Apply every remaining reviewed difference, rather than bypassing review.
        let paths = reviewed
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        let all = ProjectRequest::from_native_review(root, &config, paths, &reviewed, true, scope)
            .unwrap();
        assert!(
            backend
                .execute_project(&all, cancel())
                .unwrap()
                .files
                .is_empty()
        );
        assert_eq!(fs::read_to_string(&remaining).unwrap(), changed);
        assert_eq!(fs::read_to_string(&other).unwrap(), untouched);
        assert_eq!(fs::read(&config_path).unwrap(), saved_settings);
    }

    let mut config = ConfigDocument::standalone(&[]).unwrap();
    config
        .set(&["sql", "dialect"], json!("postgresql"))
        .unwrap();
    let boundary_config_before = config.draft_text().to_owned();
    // Adapter boundary failures are intentional, not general compiler validation.
    for (extension, input) in [
        ("html", "<div>{{ template }}</div>"),
        ("yaml", "value: {{ template }}\n"),
        ("yaml", "value: |+\n  keep trailing lines\n\n"),
        ("yaml", "value: >+\n  keep trailing lines\n\n"),
        ("sql", "DO $$ BEGIN END $$;"),
        ("sql", "SELECT $$quoted text$$;"),
        (
            "sql",
            "CREATE FUNCTION f() RETURNS integer AS 'SELECT 1' LANGUAGE SQL;",
        ),
        ("sql", "BEGIN; SELECT 1; COMMIT;"),
        ("sql", "select * from {{ ref('items') }};"),
        ("html", "<div><span>unclosed"),
        ("css", ".card { color: red;"),
        ("yaml", "items: [one, two\n"),
    ] {
        assert!(
            format(&backend, &config, extension, input).is_err(),
            "must reject {extension}: {input}"
        );
        assert_eq!(config.draft_text(), boundary_config_before);
    }
    // A rejected source also leaves on-disk project/config bytes untouched.
    for (extension, scope, source) in [
        (
            "yaml",
            NativeLanguage::Yaml,
            "value: |+\n  keep trailing lines\n\n",
        ),
        ("sql", NativeLanguage::Sql, "DO $$ BEGIN END $$;"),
    ] {
        let root = temp.path().join(format!("rejected-{extension}"));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let config_path = root.join("dprint.jsonc");
        fs::write(&config_path, boundary_config_before.as_bytes()).unwrap();
        let source_path = root.join(format!("rejected.{extension}"));
        fs::write(&source_path, source).unwrap();
        let saved = ConfigDocument::load(&config_path).unwrap();
        let check =
            ProjectRequest::new_native(root, &saved, ProjectAction::Check, true, scope).unwrap();
        assert!(backend.execute_project(&check, cancel()).is_err());
        assert_eq!(fs::read_to_string(source_path).unwrap(), source);
        assert_eq!(
            fs::read_to_string(config_path).unwrap(),
            boundary_config_before
        );
    }
    // HTML composition covers the advertised TS/SCSS/JSON chain, not only CSS/JS.
    let composed = r#"<script lang="ts">const count:number=1;const value={count};</script><style lang="scss">$color:#aabbcc;.card{color:$color;&:hover{opacity:.8}}</style><script type="application/json">{"x":[1,2]}</script>"#;
    let composed_output = format(&backend, &config, "html", composed).unwrap();
    assert!(composed_output.contains("count: number = 1"));
    assert!(composed_output.contains("$color: #aabbcc"));
    assert!(composed_output.contains("opacity: 0.8") || composed_output.contains("opacity: .8"));
    assert_eq!(
        format(&backend, &config, "html", &composed_output).unwrap(),
        composed_output
    );
    // Aliases use the same formatter chain and virtual-path semantics.
    for (canonical, alias, source) in [
        ("html", "htm", SourceLanguage::Html.builtin_source()),
        ("yaml", "yml", SourceLanguage::Yaml.builtin_source()),
    ] {
        assert_eq!(
            format(&backend, &config, canonical, source).unwrap(),
            format(&backend, &config, alias, source).unwrap()
        );
    }
    // All accepted explicit dialects exercise dialect-specific quoting/comments;
    // these are formatter fixtures, not database execution or syntax certification.
    for (dialect, source, preserved) in [
        (
            "postgresql",
            "select \"BEGIN\", E'escaped\\\\text' from items where id=1;",
            "\"BEGIN\"",
        ),
        (
            "mysql",
            "select `FUNCTION`, 'text' from items where id=1; # keep comment\n",
            "# keep comment",
        ),
        (
            "sqlite",
            "select [FUNCTION], 'it''s BEGIN' from items where id=1;",
            "'it''s BEGIN'",
        ),
    ] {
        config.set(&["sql", "dialect"], json!(dialect)).unwrap();
        let output = format(&backend, &config, "sql", source)
            .unwrap_or_else(|error| panic!("{dialect}: {error}"));
        assert!(output.contains(preserved), "{dialect}: {output}");
        assert_eq!(
            format(&backend, &config, "sql", &output).unwrap(),
            output,
            "{dialect} idempotence"
        );
    }
    for unsupported in ["generic", "sqlserver", "oracle", "bigquery", "snowflake"] {
        config.set(&["sql", "dialect"], json!(unsupported)).unwrap();
        assert!(format(&backend, &config, "sql", "select 1;").is_err());
    }
    config
        .set(&["sql", "dialect"], json!("postgresql"))
        .unwrap();
    let yaml_scalars = "bool_string: \"true\"\nnull_string: \"null\"\nnumber_string: \"42\"\nleading_zero: \"001\"\nactual_bool: true\nactual_null: null\nactual_number: 42\n";
    for quote in ["preferDouble", "preferSingle", "forceDouble", "forceSingle"] {
        config.set(&["yaml", "quotes"], json!(quote)).unwrap();
        let output = format(&backend, &config, "yaml", yaml_scalars).unwrap();
        for (key, scalar) in [
            ("bool_string", "true"),
            ("null_string", "null"),
            ("number_string", "42"),
            ("leading_zero", "001"),
        ] {
            assert!(
                output.contains(&format!("{key}: \"{scalar}\""))
                    || output.contains(&format!("{key}: '{scalar}'")),
                "{quote} unquoted scalar: {output}"
            );
        }
        for line in [
            "actual_bool: true",
            "actual_null: null",
            "actual_number: 42",
        ] {
            assert!(output.contains(line), "{quote}: {output}");
        }
        assert_eq!(format(&backend, &config, "yaml", &output).unwrap(), output);
    }
    // Significant trailing spaces in block scalars must survive our safe
    // absent-option default and explicit false; unsafe upstream true is rejected.
    let block_spaces = "literal: |-\n  first  \n  second \nfolded: >-\n  first  \n  second \n";
    let absent_before = config.draft_text().to_owned();
    let safe_absent = format(&backend, &config, "yaml", block_spaces).unwrap();
    assert_eq!(config.draft_text(), absent_before);
    for line in ["  first  \n", "  second \n"] {
        assert!(
            safe_absent.contains(line),
            "significant block spaces lost: {safe_absent:?}"
        );
    }
    config
        .set(&["yaml", "trimTrailingWhitespaces"], json!(false))
        .unwrap();
    let explicit_before = config.draft_text().to_owned();
    assert_eq!(
        format(&backend, &config, "yaml", block_spaces).unwrap(),
        safe_absent
    );
    assert_eq!(config.draft_text(), explicit_before);
    config
        .set(&["yaml", "trimTrailingWhitespaces"], json!(true))
        .unwrap();
    let unsafe_before = config.draft_text().to_owned();
    assert!(format(&backend, &config, "yaml", block_spaces).is_err());
    assert_eq!(config.draft_text(), unsafe_before);
    config
        .set(&["yaml", "trimTrailingWhitespaces"], json!(false))
        .unwrap();
    // A warm verification cache must not mask a replaced WASM member. Repair is
    // explicitly from the original exact archive, not a silent download.
    for (id, extension, variable, source) in [
        (
            "yaml",
            "yaml",
            "CODESTYLES_YAML_ARCHIVE",
            "items: [one,two]\n",
        ),
        (
            "sql",
            "sql",
            "CODESTYLES_SQL_ARCHIVE",
            "select name from items where id=1;",
        ),
    ] {
        let before = format(&backend, &config, extension, source).unwrap();
        let resolved = manager.resolve(id).unwrap();
        let member = resolved.wasm_plugins[0].clone();
        drop(resolved);
        let mut bytes = fs::read(&member).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        fs::write(&member, bytes).unwrap();
        assert!(manager.resolve(id).is_err());
        assert!(matches!(
            manager.status(id).unwrap(),
            ToolchainStatus::Corrupt { .. }
        ));
        assert!(format(&backend, &config, extension, source).is_err());
        let archive = std::env::var_os(variable).unwrap();
        drop(
            manager
                .install_verified_archive(id, Path::new(&archive), cancel(), |_| {})
                .unwrap(),
        );
        assert_eq!(
            format(&backend, &config, extension, source).unwrap(),
            before
        );
        let bad_archive = temp.path().join(format!("invalid-{id}.tar.gz"));
        fs::write(&bad_archive, b"not the pinned archive").unwrap();
        assert!(
            manager
                .install_verified_archive(id, &bad_archive, cancel(), |_| {})
                .is_err()
        );
        assert_eq!(
            format(&backend, &config, extension, source).unwrap(),
            before
        );
        let cancelled = Arc::new(AtomicBool::new(true));
        assert!(
            manager
                .install_verified_archive(id, Path::new(&archive), cancelled, |_| {})
                .is_err()
        );
        assert_eq!(
            format(&backend, &config, extension, source).unwrap(),
            before
        );
    }
    let no_dialect = ConfigDocument::standalone(&[]).unwrap();
    assert!(format(&backend, &no_dialect, "sql", "select 1;").is_err());
    // SQL formatting is not SQL validation: do not assert arbitrary invalid
    // statements fail when the pinned formatter only guarantees token handling.
    for id in ["sql", "yaml", "vue", "dprint"] {
        manager.uninstall(id).unwrap();
        assert_eq!(manager.status(id).unwrap(), ToolchainStatus::Missing);
    }
    for language in [
        SourceLanguage::Html,
        SourceLanguage::Css,
        SourceLanguage::Scss,
        SourceLanguage::Yaml,
        SourceLanguage::Sql,
    ] {
        assert!(
            format(
                &backend,
                &config,
                language.extension(),
                language.builtin_source()
            )
            .is_err()
        );
    }
}
