//! Explicit, bounded interoperability for the supported native option subset.
//! No config discovery, runtime execution, YAML objects/tags/aliases, or disk writes.
use crate::{
    config::{ConfigDocument, ConfigSnapshot},
    native_format::{self, NativeLanguage},
};
use serde_json::{Map, Value};
use std::{collections::HashSet, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeConfigKind {
    CSharpier,
    ClangFormat,
}
impl NativeConfigKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::CSharpier => "CSharpier",
            Self::ClangFormat => "clang-format",
        }
    }
    pub fn filename(self) -> &'static str {
        match self {
            Self::CSharpier => ".csharpierrc.json",
            Self::ClangFormat => ".clang-format",
        }
    }
    fn language(self) -> NativeLanguage {
        match self {
            Self::CSharpier => NativeLanguage::CSharp,
            Self::ClangFormat => NativeLanguage::Cpp,
        }
    }
}

pub fn read_import_config(path: &Path) -> Result<String, String> {
    crate::config_import::read_utf8(path, "native formatter configuration")
}

/// Export the effective supported subset, including explicitly fixed behavior.
/// clang-format JSON is valid YAML; block YAML is easier to inspect in editors.
pub fn export_config(snapshot: &ConfigSnapshot, kind: NativeConfigKind) -> Result<String, String> {
    export_config_inner(snapshot, kind).map_err(bounded_message)
}

fn export_config_inner(
    snapshot: &ConfigSnapshot,
    kind: NativeConfigKind,
) -> Result<String, String> {
    let mut document = ConfigDocument::standalone(&[])?;
    document.replace_draft_text(snapshot.source_text())?;
    ensure_unambiguous(&document, kind)?;
    let settings = native_format::options(snapshot.source_text(), kind.language())?;
    match kind {
        NativeConfigKind::CSharpier => Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?
        )),
        NativeConfigKind::ClangFormat => {
            let mut output =
                String::from("# Supported CodeStyles settings; include order is preserved.\n");
            for (key, value) in settings {
                output.push_str(&format!("{key}: {value}\n"));
            }
            output.push_str("SortIncludes: Never\n");
            Ok(output)
        }
    }
}

/// Merge only present supported keys atomically. Imported comments are not copied;
/// destination JSONC comments, other sections and absent settings are preserved.
pub fn import_config(
    document: &mut ConfigDocument,
    text: &str,
    kind: NativeConfigKind,
) -> Result<usize, String> {
    import_config_inner(document, text, kind).map_err(bounded_message)
}

fn bounded_message(message: String) -> String {
    if message.len() <= 2048 {
        return message;
    }
    let mut end = 2045;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

fn import_config_inner(
    document: &mut ConfigDocument,
    text: &str,
    kind: NativeConfigKind,
) -> Result<usize, String> {
    if text.len() > crate::config_import::LIMIT {
        return Err("Configuration import exceeds the 1 MiB limit.".into());
    }
    let text = text.trim_start_matches('\u{feff}');
    let mut settings = if kind == NativeConfigKind::CSharpier
        || text
            .trim_start_matches([' ', '\t', '\r', '\n'])
            .starts_with('{')
    {
        parse_json(text)?
    } else {
        parse_clang_yaml(text)?
    };
    if kind == NativeConfigKind::ClangFormat
        && let Some(value) = settings.remove("SortIncludes")
        && value != Value::String("Never".into())
        && value != Value::Bool(false)
    {
        return Err("SortIncludes must be Never (or false). CodeStyles preserves include order; no settings were imported.".into());
    }
    if settings.is_empty() {
        return Err("No supported adjustable settings to import.".into());
    }
    for (name, value) in &settings {
        // Avoid reflecting an untrusted 1 MiB property/string into diagnostics.
        if name.len() > 80 || value.as_str().is_some_and(|v| v.len() > 80) {
            return Err(
                "Unsupported or oversized native formatter setting; nothing was imported.".into(),
            );
        }
        native_format::validate_option(kind.language(), name, value)?;
    }
    ensure_unambiguous(document, kind)?;
    let count = settings.len();
    let mut edited = document.clone();
    for (name, value) in settings {
        edited.set(&[kind.language().section(), &name], value)?;
    }
    native_format::options(edited.draft_text(), kind.language())?;
    document.replace_draft_text(edited.draft_text())?;
    Ok(count)
}

fn ensure_unambiguous(document: &ConfigDocument, kind: NativeConfigKind) -> Result<(), String> {
    for name in ["extends", "inherit", "overrides"] {
        document.value(&[name])?;
    }
    let section = kind.language().section();
    if let Some(Value::Object(settings)) = document.value(&[section])? {
        for name in settings.keys() {
            document.value(&[section, name])?;
        }
    }
    Ok(())
}

fn parse_json(text: &str) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| {
        format!(
            "Native config requires strict JSON (line {}, column {}, {:?}).",
            e.line(),
            e.column(),
            e.classify()
        )
    })?;
    let object = value
        .as_object()
        .ok_or("Native configuration must be one JSON object.")?;
    // serde_json alone uses last-key-wins. Inspect the original CST to reject
    // duplicates, including escaped spellings, before importing any values.
    let root = jsonc_parser::cst::CstRootNode::parse(text, &jsonc_parser::ParseOptions::default())
        .map_err(|_| "Cannot inspect native JSON properties.")?;
    let mut seen = HashSet::new();
    for property in root
        .object_value()
        .ok_or("Expected JSON object.")?
        .properties()
    {
        let name = property
            .name()
            .and_then(|n| n.decoded_value().ok())
            .ok_or("Invalid JSON property.")?;
        if !seen.insert(name) {
            return Err("Duplicate native configuration property; nothing was imported.".into());
        }
    }
    Ok(object.clone())
}

/// Deliberately not a general YAML parser. Reject unsupported YAML rather than
/// approximating its semantics (anchors, merges, tags, multiline/nested values).
fn parse_clang_yaml(text: &str) -> Result<Map<String, Value>, String> {
    let mut settings = Map::new();
    let mut started = false;
    let mut ended = false;
    let mut marker = false;
    for (index, line) in text.lines().enumerate() {
        let error = || {
            format!(
                "Unsupported .clang-format syntax at line {}. Import accepts one flat scalar YAML mapping or strict JSON; no aliases, nesting, tags or language overrides.",
                index + 1
            )
        };
        let trimmed = line.trim_matches([' ', '\t']);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "---" && !started && !marker && !ended {
            marker = true;
            continue;
        }
        if trimmed == "..." && started && !ended {
            ended = true;
            continue;
        }
        if ended || line.starts_with([' ', '\t']) {
            return Err(error());
        }
        let (name, raw) = line.split_once(':').ok_or_else(error)?;
        let name = name.trim_end_matches([' ', '\t']);
        if !raw.starts_with([' ', '\t']) {
            return Err(error());
        }
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphabetic()) || name.len() > 80 {
            return Err(error());
        }
        // Values are finite enums, integers or false; none permit # in strings.
        // An inline YAML comment is allowed only after whitespace.
        let raw = raw.trim_matches([' ', '\t']);
        let raw = if let Some(hash) = raw.find('#') {
            if hash == 0 || !matches!(raw.as_bytes()[hash - 1], b' ' | b'\t') {
                return Err(error());
            }
            raw[..hash].trim_end_matches([' ', '\t'])
        } else {
            raw
        };
        if raw.is_empty() || raw.len() > 80 {
            return Err(error());
        }
        let value = if raw.starts_with('"') {
            let value: Value = serde_json::from_str(raw).map_err(|_| error())?;
            if !value.is_string() {
                return Err(error());
            }
            value
        } else if raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2 {
            let value = &raw[1..raw.len() - 1];
            if !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(error());
            }
            Value::String(value.into())
        } else if raw == "false" {
            Value::Bool(false)
        } else if raw.bytes().all(|b| b.is_ascii_digit()) {
            // Avoid YAML octal/version-dependent interpretations.
            if raw.len() > 1 && raw.starts_with('0') {
                return Err(error());
            }
            Value::from(raw.parse::<u64>().map_err(|_| error())?)
        } else if raw.bytes().all(|b| b.is_ascii_alphabetic()) {
            Value::String(raw.into())
        } else {
            return Err(error());
        };
        if settings.insert(name.to_owned(), value).is_some() {
            return Err("Duplicate native configuration property; nothing was imported.".into());
        }
        started = true;
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn document(text: &str) -> ConfigDocument {
        let mut doc = ConfigDocument::standalone(&[]).unwrap();
        doc.replace_draft_text(text).unwrap();
        doc
    }
    #[test]
    fn import_merges_only_explicit_keys_and_keeps_jsonc_and_saved_baseline() {
        for (kind, source, section, key, value) in [
            (
                NativeConfigKind::CSharpier,
                r#"{"printWidth":120}"#,
                "csharpier",
                "printWidth",
                json!(120),
            ),
            (
                NativeConfigKind::ClangFormat,
                "---\n# style\nBasedOnStyle: 'Google' # note\nSortIncludes: Never\n...\n",
                "clangFormat",
                "BasedOnStyle",
                json!("Google"),
            ),
        ] {
            let mut doc = document(
                "{ // keep 中文\n\"other\":{\"value\":42}, \"csharpier\":{\"useTabs\":true}, \"clangFormat\":{\"ColumnLimit\":90}}\n",
            );
            let saved = doc.saved_text().to_owned();
            assert_eq!(import_config(&mut doc, source, kind).unwrap(), 1);
            assert!(doc.draft_text().contains("// keep 中文"));
            assert_eq!(doc.saved_text(), saved);
            assert_eq!(doc.value(&[section, key]).unwrap(), Some(value));
            assert_eq!(doc.value(&["other", "value"]).unwrap(), Some(json!(42)));
            assert_eq!(
                doc.value(&["csharpier", "useTabs"]).unwrap(),
                Some(json!(true))
            );
            assert_eq!(
                doc.value(&["clangFormat", "ColumnLimit"]).unwrap(),
                Some(json!(90))
            );
        }
    }
    #[test]
    fn native_exports_roundtrip_effective_settings_and_preserve_include_order() {
        for kind in [NativeConfigKind::CSharpier, NativeConfigKind::ClangFormat] {
            let doc = document(
                r#"{"csharpier":{"useTabs":true,"indentSize":2,"printWidth":120,"endOfLine":"lf"},"clangFormat":{"BasedOnStyle":"Google","ColumnLimit":0,"UseTab":"Always","BreakBeforeBraces":"Allman","IndentWidth":4}}"#,
            );
            let export = export_config(&doc.snapshot(), kind).unwrap();
            if kind == NativeConfigKind::ClangFormat {
                assert!(export.contains("SortIncludes: Never"));
            }
            let mut imported = document("{}");
            import_config(&mut imported, &export, kind).unwrap();
            assert_eq!(
                native_format::options(doc.draft_text(), kind.language()).unwrap(),
                native_format::options(imported.draft_text(), kind.language()).unwrap()
            );
            assert_eq!(export_config(&imported.snapshot(), kind).unwrap(), export);
        }
    }
    #[test]
    fn unsupported_or_ambiguous_imports_are_atomic_and_diagnostics_bounded() {
        let cases = [
            (NativeConfigKind::CSharpier, "{}"),
            (NativeConfigKind::CSharpier, "[]"),
            (NativeConfigKind::CSharpier, "printWidth: 80"),
            (
                NativeConfigKind::CSharpier,
                r#"{"printWidth":80,"printWidth":90}"#,
            ),
            (
                NativeConfigKind::CSharpier,
                r#"{"printWidth":80,"\u0070rintWidth":90}"#,
            ),
            (
                NativeConfigKind::CSharpier,
                r#"{"printWidth":80,"overrides":[]}"#,
            ),
            (
                NativeConfigKind::CSharpier,
                r#"{"printWidth":80,"indentSize":0}"#,
            ),
            (NativeConfigKind::CSharpier, r#"{"printWidth":80,}"#),
            (
                NativeConfigKind::ClangFormat,
                "BasedOnStyle: LLVM\nSortIncludes: true",
            ),
            (
                NativeConfigKind::ClangFormat,
                "BasedOnStyle: LLVM\nLanguage: Cpp",
            ),
            (
                NativeConfigKind::ClangFormat,
                "BasedOnStyle: InheritParentConfig",
            ),
            (NativeConfigKind::ClangFormat, "IndentWidth: 02"),
            (NativeConfigKind::ClangFormat, "BasedOnStyle:LLVM"),
            (NativeConfigKind::ClangFormat, "IndentWidth:\u{a0}2"),
            (NativeConfigKind::ClangFormat, "IndentWidth: 2\u{a0}"),
            (NativeConfigKind::ClangFormat, "BasedOnStyle\u{a0}: LLVM"),
            (NativeConfigKind::ClangFormat, "\tIndentWidth: 2"),
            (
                NativeConfigKind::ClangFormat,
                "BasedOnStyle: LLVM\n---\nBasedOnStyle: Google",
            ),
            (
                NativeConfigKind::ClangFormat,
                "BasedOnStyle: LLVM\nBasedOnStyle: Google",
            ),
            (NativeConfigKind::ClangFormat, "BasedOnStyle: &base LLVM"),
            (NativeConfigKind::ClangFormat, "BasedOnStyle: !!str LLVM"),
            (NativeConfigKind::ClangFormat, "BasedOnStyle: |\n  LLVM"),
            (
                NativeConfigKind::ClangFormat,
                "BasedOnStyle: LLVM\n  IndentWidth: 2",
            ),
            (
                NativeConfigKind::ClangFormat,
                "BasedOnStyle: LLVM\nIncludeCategories: []",
            ),
            (
                NativeConfigKind::ClangFormat,
                "{\"BasedOnStyle\":\"LLVM\",\"BasedOnStyle\":\"Google\"}",
            ),
        ];
        for (kind, text) in cases {
            let mut doc = document("{ /* retained */ \"other\":true }");
            let before = doc.draft_text().to_owned();
            assert!(import_config(&mut doc, text, kind).is_err(), "{text}");
            assert_eq!(doc.draft_text(), before);
        }
        let mut doc = document("{}");
        for input in [
            format!("{{\"{}\":true}}", "x".repeat(500_000)),
            "{".repeat(100_000),
            format!("{{\"printWidth\":{},}}", " ".repeat(500_000)),
        ] {
            let error = import_config(&mut doc, &input, NativeConfigKind::CSharpier).unwrap_err();
            assert!(error.len() < 2048);
        }
    }
    #[test]
    fn destination_conflicts_and_inheritance_fail_without_partial_merge() {
        for text in [
            r#"{"extends":"outside"}"#,
            r#"{"overrides":[]}"#,
            r#"{"inherit":true}"#,
            r#"{"csharpier":false}"#,
            r#"{"csharpier":{},"csharpier":{}}"#,
            r#"{"csharpier":{"printWidth":80,"printWidth":90}}"#,
            r#"{"csharpier":{"useTabs":true,"useTabs":false}}"#,
            r#"{"inherit":false,"inherit":false}"#,
        ] {
            let mut doc = document(text);
            assert!(
                import_config(
                    &mut doc,
                    r#"{"printWidth":100}"#,
                    NativeConfigKind::CSharpier
                )
                .is_err(),
                "{text}"
            );
            assert_eq!(doc.draft_text(), text);
        }
    }
    #[test]
    fn regular_reader_rejects_nonfiles_large_and_invalid_utf8_without_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("style");
        std::fs::write(&path, "{\"printWidth\":80}").unwrap();
        assert_eq!(read_import_config(&path).unwrap(), "{\"printWidth\":80}");
        assert!(read_import_config(dir.path()).is_err());
        std::fs::write(&path, [255]).unwrap();
        assert!(read_import_config(&path).unwrap_err().contains("UTF-8"));
        std::fs::write(&path, vec![b' '; crate::config_import::LIMIT + 1]).unwrap();
        assert!(read_import_config(&path).unwrap_err().contains("1 MiB"));
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read_import_config(&link).is_err());
        }
    }
}
