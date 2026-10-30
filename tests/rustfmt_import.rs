use codestyles::{config::ConfigDocument, rustfmt};
use serde_json::json;
use std::fs;

fn document(text: &str) -> (tempfile::TempDir, ConfigDocument) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dprint.jsonc");
    fs::write(&path, text).unwrap();
    let document = ConfigDocument::load(&path).unwrap();
    (directory, document)
}

#[test]
fn import_merges_only_explicit_keys_preserving_jsonc_and_saved_baseline() {
    let original = "{\n // keep root\n \"futurePlugin\": {\"unknown\": 42},\n \"rustfmt\": {\n // width note\n \"max_width\": 80,\n // untouched note\n \"hard_tabs\": true\n },\n \"typescript\": {\"quoteStyle\": \"alwaysSingle\"}\n}\n";
    let (_directory, mut config) = document(original);
    assert_eq!(
        rustfmt::import_config(
            &mut config,
            "# imported comment\nmax_width = 100\nedition = '2024'\n"
        )
        .unwrap(),
        2
    );
    assert_eq!(
        config.value(&["rustfmt", "max_width"]).unwrap(),
        Some(json!(100))
    );
    assert_eq!(
        config.value(&["rustfmt", "hard_tabs"]).unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        config.value(&["rustfmt", "edition"]).unwrap(),
        Some(json!("2024"))
    );
    assert_eq!(config.value(&["rustfmt", "style_edition"]).unwrap(), None);
    for text in [
        "// keep root",
        "// width note",
        "// untouched note",
        "\"futurePlugin\": {\"unknown\": 42}",
        "\"typescript\": {\"quoteStyle\": \"alwaysSingle\"}",
    ] {
        assert!(config.draft_text().contains(text), "{text}");
    }
    assert!(!config.draft_text().contains("imported comment"));
    assert_eq!(config.saved_text(), original);
    assert_eq!(
        fs::read_to_string(config.path().unwrap()).unwrap(),
        original
    );
    assert!(config.is_dirty());
    let draft = config.draft_text().to_owned();
    rustfmt::import_config(&mut config, "max_width = 100\nedition = '2024'").unwrap();
    assert_eq!(config.draft_text(), draft);
}

#[test]
fn export_import_roundtrip_covers_all_supported_options() {
    let (_directory, mut config) = document("{}");
    let input = r#"
edition = "2024"
style_edition = "2021"
max_width = 100
tab_spaces = 2
hard_tabs = true
newline_style = "Unix"
use_small_heuristics = "Max"
reorder_imports = false
reorder_modules = true
use_field_init_shorthand = true
use_try_shorthand = false
match_block_trailing_comma = true
"#;
    assert_eq!(
        rustfmt::import_config(&mut config, input).unwrap(),
        rustfmt::OPTIONS.len()
    );
    let exported = rustfmt::export_config(&config.snapshot()).unwrap();
    let (_other, mut copy) = document("{}");
    rustfmt::import_config(&mut copy, &exported).unwrap();
    assert_eq!(
        config.value(&["rustfmt"]).unwrap(),
        copy.value(&["rustfmt"]).unwrap()
    );
}

#[test]
fn invalid_or_unsupported_import_is_atomic_even_with_valid_keys() {
    let (_directory, mut config) = document("{ // keep\n \"rustfmt\": {\"hard_tabs\": true}}\n");
    config.set(&["rustfmt", "tab_spaces"], json!(2)).unwrap();
    let before = config.draft_text().to_owned();
    for input in [
        "",
        "# comments only\n",
        "max_width =",
        "max_width = 80\nmax_width = 90",
        "max_width = 80\nwrap_comments = true",
        "max_width = 80\ncommand = 'evil'",
        "[rustfmt]\nmax_width = 80",
        "max_width = 80.0",
        "max_width = -1",
        "tab_spaces = 17",
        "hard_tabs = 'true'",
        "edition = 2024",
        "style_edition = '2027'",
        "newline_style = ['Unix']",
        "max_width = 1979-05-27",
        "max_width.value = 80",
    ] {
        assert!(
            rustfmt::import_config(&mut config, input).is_err(),
            "{input}"
        );
        assert_eq!(config.draft_text(), before, "{input}");
    }
    assert!(rustfmt::import_config(&mut config, &" ".repeat(1024 * 1024 + 1)).is_err());
    assert_eq!(config.draft_text(), before);
}

#[test]
fn destination_conflicts_fail_without_partial_cst_changes() {
    for original in [
        r#"{"rustfmt":false}"#,
        r#"{"rustfmt":{},"rustfmt":{}}"#,
        r#"{"rustfmt":{"max_width":80,"max_width":90}}"#,
        r#"{"extends":"remote","rustfmt":{}}"#,
        r#"{"rustfmt":{"unknown":true}}"#,
    ] {
        let (_directory, mut config) = document(original);
        assert!(
            rustfmt::import_config(&mut config, "hard_tabs = true\nmax_width = 100").is_err(),
            "{original}"
        );
        assert_eq!(config.draft_text(), original);
        assert!(!config.is_dirty());
    }
}

#[test]
fn import_file_read_is_regular_bounded_and_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustfmt.toml");
    fs::write(&path, "max_width = 100\n").unwrap();
    assert_eq!(
        rustfmt::read_import_config(&path).unwrap(),
        "max_width = 100\n"
    );
    fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(
        rustfmt::read_import_config(&path)
            .unwrap_err()
            .contains("UTF-8")
    );
    fs::File::create(&path)
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    assert!(
        rustfmt::read_import_config(&path)
            .unwrap_err()
            .contains("1 MiB")
    );
    assert!(rustfmt::read_import_config(dir.path()).is_err());
    assert!(rustfmt::read_import_config(&dir.path().join("missing")).is_err());
}

#[cfg(unix)]
#[test]
fn import_file_read_rejects_symlink_and_fifo_without_blocking() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustfmt.toml");
    fs::write(&path, "max_width = 100\n").unwrap();
    let link = dir.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(rustfmt::read_import_config(&link).is_err());
    let fifo = dir.path().join("pipe");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: name is a valid NUL-terminated path in this test-owned directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(rustfmt::read_import_config(&fifo).is_err());
}

#[test]
fn identical_import_into_clean_document_is_byte_identical_and_not_dirty() {
    let original =
        "{ // preserve whitespace\n  \"rustfmt\": {\"max_width\": 100, \"edition\": \"2021\"}\n}\n";
    let (_directory, mut config) = document(original);
    assert!(!config.is_dirty());
    assert_eq!(
        rustfmt::import_config(&mut config, "edition = '2021'\nmax_width = 100\n").unwrap(),
        2
    );
    assert_eq!(config.draft_text(), original);
    assert_eq!(config.saved_text(), original);
    assert!(!config.is_dirty());
}

#[test]
fn import_diagnostics_are_bounded_utf8_and_never_dump_a_large_source_line() {
    let (_directory, mut config) = document("{ // original\n \"rustfmt\": {\"max_width\": 80}}");
    let before = config.draft_text().to_owned();
    let malformed = format!("max_width = {}!", " ".repeat(900_000));
    let error = rustfmt::import_config(&mut config, &malformed).unwrap_err();
    assert!(error.contains("line 1, column"), "{error}");
    assert!(!error.contains(&" ".repeat(100)), "source line leaked");
    assert!(error.len() <= 2048);
    for input in [
        format!("\"{}\" = true", "界🙂".repeat(100_000)),
        format!("newline_style = \"{}\"", "界🙂".repeat(100_000)),
        format!("edition = \"{}\"", "x".repeat(900_000)),
        format!("\"{}\" = ", "界🙂".repeat(100_000)),
    ] {
        let error = rustfmt::import_config(&mut config, &input).unwrap_err();
        assert!(error.len() <= 2048, "diagnostic length {}", error.len());
        assert!(std::str::from_utf8(error.as_bytes()).is_ok());
        assert!(error.contains('…') || error.contains("Invalid rustfmt.toml"));
        assert_eq!(config.draft_text(), before);
        assert!(!config.is_dirty());
    }
    assert_eq!(config.draft_text(), before);
}
