//! Lossless JSONC drafts, immutable preview inputs, and explicit atomic saves.
//!
//! Editing and previewing never write the original file. Trust and schema-based
//! validation belong to the caller; loading an unfamiliar plugin configuration
//! must not silently substitute bundled plugins or invent defaults.
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstNode, CstObject, CstObjectProp, CstRootNode},
};
use serde_json::Value;
use std::{
    fs::{self, File, Metadata},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tempfile::{NamedTempFile, TempDir};

/// Exported references are portable. Only these exact URLs may be mapped to
/// verified bundle paths for offline preview, never another version or host.
const PINNED_PLUGINS: [(&str, &str); 2] = [
    (
        "typescript-0.96.1.wasm",
        "https://plugins.dprint.dev/typescript-0.96.1.wasm",
    ),
    (
        "json-0.25.2.wasm",
        "https://plugins.dprint.dev/json-0.25.2.wasm",
    ),
];

/// Owned input captured at submission time, independent of subsequent edits.
/// The original path supplies relative-config semantics, never fresh contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigSnapshot {
    source_text: String,
    original_path: Option<PathBuf>,
}

impl ConfigSnapshot {
    pub fn source_text(&self) -> &str {
        &self.source_text
    }
    pub fn original_path(&self) -> Option<&Path> {
        self.original_path.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl FileStamp {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        }
    }
}

/// A saved baseline and a separately editable draft. A fresh standalone document
/// has no destination until Save As succeeds.
#[derive(Clone, Debug)]
pub struct ConfigDocument {
    path: Option<PathBuf>,
    saved_text: String,
    draft_text: String,
    saved_stamp: Option<FileStamp>,
}

impl ConfigDocument {
    pub fn load(path: &Path) -> Result<Self, String> {
        let path = fs::canonicalize(path)
            .map_err(|e| format!("Cannot open configuration {}: {e}", path.display()))?;
        let (text, stamp) = read_stable_file(&path)?;
        parse_object(&text)?;
        Ok(Self {
            path: Some(path),
            saved_text: text.clone(),
            draft_text: text,
            saved_stamp: Some(stamp),
        })
    }

    /// Standalone documents use portable URLs for the supplied pinned bundle.
    /// Imported documents retain every plugin and unknown property verbatim.
    pub fn standalone(plugins: &[PathBuf]) -> Result<Self, String> {
        let plugins = plugins
            .iter()
            .map(|path| {
                fs::canonicalize(path)
                    .map_err(|e| format!("Cannot locate bundled plugin {}: {e}", path.display()))?;
                let url = PINNED_PLUGINS
                    .iter()
                    .find(|(name, _)| path.file_name() == Some(std::ffi::OsStr::new(name)))
                    .map(|(_, url)| *url)
                    .ok_or_else(|| {
                        format!(
                            "No portable pinned reference for bundled plugin {}",
                            path.display()
                        )
                    })?;
                Ok::<Value, String>(Value::String(url.into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({"plugins": plugins}))
                .map_err(|e| format!("Cannot create standalone configuration: {e}"))?
        );
        Ok(Self {
            path: None,
            saved_text: text.clone(),
            draft_text: text,
            saved_stamp: None,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn saved_text(&self) -> &str {
        &self.saved_text
    }
    pub fn draft_text(&self) -> &str {
        &self.draft_text
    }
    pub fn is_dirty(&self) -> bool {
        self.draft_text != self.saved_text
    }

    pub fn snapshot(&self) -> ConfigSnapshot {
        ConfigSnapshot {
            source_text: self.draft_text.clone(),
            original_path: self.path.clone(),
        }
    }

    /// Capture the last loaded/saved configuration for an A/B preview. This is
    /// deliberately the in-memory baseline, not another read of the disk file.
    pub fn saved_snapshot(&self) -> ConfigSnapshot {
        ConfigSnapshot {
            source_text: self.saved_text.clone(),
            original_path: self.path.clone(),
        }
    }

    /// Reject invalid text without destroying the last valid draft. An editor
    /// may retain its own invalid input while showing this validation error.
    pub fn replace_draft_text(&mut self, text: &str) -> Result<(), String> {
        parse_object(text)?;
        self.draft_text = text.to_owned();
        Ok(())
    }

    pub fn discard_changes(&mut self) {
        self.draft_text.clone_from(&self.saved_text);
    }

    /// Missing means inherited/unset, not an implicit default or JSON null.
    pub fn value(&self, path: &[&str]) -> Result<Option<Value>, String> {
        validate_path(path)?;
        let root = parse_object(&self.draft_text)?;
        let Some(object) = parent_object(&root, path, false)? else {
            return Ok(None);
        };
        Ok(unique_property(&object, path[path.len() - 1])?.and_then(|prop| prop.to_serde_value()))
    }

    /// Change only the requested CST value, adding absent parent objects. A
    /// malformed/non-object parent or duplicate path component is an error.
    /// Mutations are performed on a fresh CST and committed only on success.
    pub fn set(&mut self, path: &[&str], value: Value) -> Result<(), String> {
        validate_path(path)?;
        let root = parse_object(&self.draft_text)?;
        let object = parent_object(&root, path, true)?.expect("set creates missing objects");
        let key = path[path.len() - 1];
        match unique_property(&object, key)? {
            Some(prop) => {
                if prop.to_serde_value().as_ref() == Some(&value) {
                    return Ok(());
                }
                prop.set_value(cst_input(value));
            }
            None => {
                object.append(key, cst_input(value));
            }
        }
        self.replace_draft_text(&root.to_string())
    }

    /// Remove the key entirely so inherited/default behavior can apply. Empty
    /// parent objects stay in place, along with their comments and unknown keys.
    pub fn remove(&mut self, path: &[&str]) -> Result<(), String> {
        validate_path(path)?;
        let root = parse_object(&self.draft_text)?;
        let Some(object) = parent_object(&root, path, false)? else {
            return Ok(());
        };
        let Some(prop) = unique_property(&object, path[path.len() - 1])? else {
            return Ok(());
        };
        // CstObjectProp::remove also discards adjacent comments. Use CST node
        // boundaries to remove only this property and its separator instead.
        let separator = prop
            .trailing_comma()
            .map(CstNode::from)
            .or_else(|| prop.previous_siblings().find(|node| node.is_comma()));
        let mut ranges = vec![node_range(&prop.into())];
        if let Some(separator) = separator {
            ranges.push(node_range(&separator));
        }
        ranges.sort_unstable_by_key(|range| std::cmp::Reverse(range.start));
        let mut edited = self.draft_text.clone();
        for range in ranges {
            edited.replace_range(range, "");
        }
        self.replace_draft_text(&edited)
    }

    /// Includes deletion, replacement, and content changes since load/save.
    /// This is optimistic detection, not a cross-process transactional lock.
    pub fn has_external_changes(&self) -> Result<bool, String> {
        let Some(path) = self.path() else {
            return Ok(false);
        };
        match fs::canonicalize(path) {
            Ok(current) if current != path => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => {
                return Err(format!(
                    "Cannot inspect configuration {}: {error}",
                    path.display()
                ));
            }
            _ => {}
        }
        let (text, stamp) = read_stable_file(path)?;
        Ok(text != self.saved_text || self.saved_stamp.as_ref() != Some(&stamp))
    }

    /// Flush a same-directory temporary file, recheck the saved baseline, then
    /// atomically replace the original. A writer racing after the final check
    /// cannot be excluded without a cooperative filesystem locking protocol.
    pub fn save(&mut self) -> Result<(), String> {
        self.save_with_before_commit(|| {})
    }

    fn save_with_before_commit(&mut self, before_commit: impl FnOnce()) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or("Choose a destination with Save As first.")?;
        self.validate_for_save()?;
        self.ensure_unchanged()?;
        if !self.is_dirty() {
            return Ok(());
        }
        let mut file = save_tempfile(&path, &self.draft_text)?;
        let permissions = fs::metadata(&path)
            .map_err(|e| format!("Cannot inspect original configuration: {e}"))?
            .permissions();
        file.as_file_mut()
            .set_permissions(permissions)
            .map_err(|e| format!("Cannot preserve configuration permissions: {e}"))?;
        file.as_file()
            .sync_all()
            .map_err(|e| format!("Cannot sync configuration: {e}"))?;
        before_commit();
        self.ensure_unchanged()?;
        let persisted = file
            .persist(&path)
            .map_err(|e| format!("Cannot save configuration {}: {}", path.display(), e.error))?;
        self.finish_save(path, &persisted)
    }

    /// Export without overwriting another file. Relative references are kept
    /// verbatim and resolve relative to the new destination after this succeeds.
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        let name = path
            .file_name()
            .ok_or("Destination must include a file name.")?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = fs::canonicalize(parent)
            .map_err(|e| format!("Cannot open destination directory: {e}"))?;
        let normalized_path = parent.join(name);
        // Even a deleted original must take the conflict-checked save path.
        if self.path.as_ref() == Some(&normalized_path) {
            return self.save();
        }
        if let Ok(existing) = fs::canonicalize(path) {
            if self.path.as_ref() == Some(&existing) {
                return self.save();
            }
            return Err(format!(
                "Destination already exists: {}. Choose a new file.",
                path.display()
            ));
        }
        // Also reject dangling symlinks, which canonicalize cannot resolve.
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(format!(
                    "Destination already exists: {}. Choose a new file.",
                    path.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Cannot inspect destination {}: {error}",
                    path.display()
                ));
            }
        }
        self.validate_for_save()?;
        let path = normalized_path;
        let file = save_tempfile(&path, &self.draft_text)?;
        let persisted = file.persist_noclobber(&path).map_err(|e| {
            format!(
                "Cannot create configuration {} without overwriting: {}",
                path.display(),
                e.error
            )
        })?;
        self.finish_save(path, &persisted)
    }

    fn ensure_unchanged(&self) -> Result<(), String> {
        if self.has_external_changes()? {
            Err("Configuration changed on disk. Reload it or use Save As; your draft has been kept.".into())
        } else {
            Ok(())
        }
    }

    /// Check only known core semantics and an unambiguous pinned plugin schema.
    /// Raw apply deliberately remains syntax-only so unsupported/custom schemas
    /// and formatter diagnostics can be inspected without discarding draft text.
    fn validate_for_save(&self) -> Result<(), String> {
        let root = parse_object(&self.draft_text)?;
        let object = root.object_value().expect("validated root object");
        if let Some(indent) = unique_property(&object, "indentWidth")? {
            // Both bundled schemas declare a number; the pinned CLI also
            // accepts strings parsed as u8. Preserve that runtime compatibility
            // (including 0 and 255), rather than the UI's narrower choices.
            let valid = indent.value().is_some_and(|node| {
                if let Some(number) = node.as_number_lit() {
                    // Use the spelling: dprint accepts -0, but rejects 0.0/0e0.
                    return number
                        .to_string()
                        .parse::<i64>()
                        .ok()
                        .is_some_and(|value| u8::try_from(value).is_ok());
                }
                node.as_string_lit()
                    .and_then(|string| string.decoded_value().ok())
                    .is_some_and(|value| value.parse::<u8>().is_ok())
            });
            if !valid {
                return Err("Cannot save: indentWidth must be an integer from 0 to 255. Your draft has been kept.".into());
            }
        }
        if !has_pinned_typescript_schema(&object) {
            return Ok(());
        }
        if let Some(typescript) = unique_property(&object, "typescript")? {
            let typescript = typescript.object_value()
                .ok_or("Cannot save: typescript must be an object for the pinned TypeScript plugin. Your draft has been kept.")?;
            if let Some(semicolons) = unique_property(&typescript, "semiColons")? {
                let valid = semicolons.to_serde_value().and_then(|value| {
                    value
                        .as_str()
                        .map(|value| matches!(value, "always" | "prefer" | "asi"))
                }) == Some(true);
                if !valid {
                    return Err("Cannot save: typescript.semiColons must be always, prefer, or asi for TypeScript 0.96.1. Your draft has been kept.".into());
                }
            }
        }
        Ok(())
    }

    fn finish_save(&mut self, path: PathBuf, file: &File) -> Result<(), String> {
        let stamp = FileStamp::from_metadata(&file.metadata().map_err(|e| {
            format!("Configuration was saved, but its metadata cannot be read: {e}")
        })?);
        self.path = Some(path);
        self.saved_text.clone_from(&self.draft_text);
        self.saved_stamp = Some(stamp);
        Ok(())
    }
}

fn jsonc_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}

fn parse_object(text: &str) -> Result<CstRootNode, String> {
    let root = CstRootNode::parse(text, &jsonc_options())
        .map_err(|e| format!("Invalid JSONC configuration: {e}"))?;
    if root.object_value().is_none() {
        return Err("Configuration must contain a JSON object.".into());
    }
    Ok(root)
}

fn validate_path(path: &[&str]) -> Result<(), String> {
    if path.is_empty() || path.iter().any(|part| part.is_empty()) {
        Err("Configuration property path must not be empty.".into())
    } else {
        Ok(())
    }
}

fn unique_property(object: &CstObject, key: &str) -> Result<Option<CstObjectProp>, String> {
    let mut matching = object.properties().into_iter().filter(|prop| {
        prop.name()
            .and_then(|name| name.decoded_value().ok())
            .as_deref()
            == Some(key)
    });
    let first = matching.next();
    if matching.next().is_some() {
        return Err(format!(
            "Duplicate configuration property {key:?}; resolve it in the JSONC editor first."
        ));
    }
    Ok(first)
}

fn has_pinned_typescript_schema(object: &CstObject) -> bool {
    let (Ok(plugins), Ok(extends), Ok(inherit)) = (
        unique_property(object, "plugins"),
        unique_property(object, "extends"),
        unique_property(object, "inherit"),
    ) else {
        return false;
    };
    if extends.is_some()
        || inherit.is_some_and(|prop| prop.to_serde_value() != Some(Value::Bool(false)))
    {
        return false;
    }
    let Some(plugins) = plugins.and_then(|prop| prop.array_value()) else {
        return false;
    };
    let mut typescript = false;
    for plugin in plugins.elements() {
        let Some(Value::String(plugin)) = plugin.to_serde_value() else {
            return false;
        };
        if !PINNED_PLUGINS.iter().any(|(_, url)| *url == plugin) {
            return false;
        }
        typescript |= plugin == PINNED_PLUGINS[0].1;
    }
    typescript
}

fn parent_object(
    root: &CstRootNode,
    path: &[&str],
    create: bool,
) -> Result<Option<CstObject>, String> {
    let mut object = root.object_value().expect("validated root object");
    for (index, key) in path[..path.len() - 1].iter().enumerate() {
        object = match unique_property(&object, key)? {
            Some(prop) => prop.object_value().ok_or_else(|| {
                format!(
                    "The {} configuration must be an object.",
                    path[..=index].join(".")
                )
            })?,
            None if create => object
                .object_value_or_create(key)
                .expect("missing object can be created"),
            None => return Ok(None),
        };
    }
    Ok(Some(object))
}

fn cst_input(value: Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(value) => CstInputValue::Bool(value),
        Value::Number(value) => CstInputValue::Number(value.to_string()),
        Value::String(value) => CstInputValue::String(value),
        Value::Array(values) => CstInputValue::Array(values.into_iter().map(cst_input).collect()),
        Value::Object(values) => CstInputValue::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, cst_input(value)))
                .collect(),
        ),
    }
}

fn node_range(node: &CstNode) -> std::ops::Range<usize> {
    let start = node
        .parent()
        .map(|parent| {
            let preceding = parent
                .children()
                .iter()
                .take(node.child_index())
                .map(|node| node.to_string().len())
                .sum::<usize>();
            node_range(&parent.into()).start + preceding
        })
        .unwrap_or(0);
    start..start + node.to_string().len()
}

fn read_stable_file(path: &Path) -> Result<(String, FileStamp), String> {
    let mut file = File::open(path)
        .map_err(|e| format!("Cannot read configuration {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Cannot inspect configuration: {e}"))?;
    if !metadata.is_file() {
        return Err("Configuration must be a regular file.".into());
    }
    let before = FileStamp::from_metadata(&metadata);
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("Cannot read configuration: {e}"))?;
    let after = FileStamp::from_metadata(
        &file
            .metadata()
            .map_err(|e| format!("Cannot inspect configuration: {e}"))?,
    );
    if before != after {
        return Err("Configuration changed while it was being read. Try loading it again.".into());
    }
    Ok((text, after))
}

fn save_tempfile(path: &Path, text: &str) -> Result<NamedTempFile, String> {
    let parent = path
        .parent()
        .ok_or("Configuration has no parent directory.")?;
    let mut file = tempfile::Builder::new()
        .prefix(".codestyles-save-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|e| format!("Cannot create save file beside configuration: {e}"))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("Cannot write configuration: {e}"))?;
    file.flush()
        .map_err(|e| format!("Cannot flush configuration: {e}"))?;
    file.as_file()
        .sync_all()
        .map_err(|e| format!("Cannot sync configuration: {e}"))?;
    Ok(file)
}

/// Keeps a non-discoverable preview file alive until all child pipe readers exit.
pub struct DraftConfig {
    file: NamedTempFile,
    _directory: Option<TempDir>,
}

impl DraftConfig {
    /// P0 compatibility: explicit requested settings become CST overrides.
    pub fn new(
        original: Option<&Path>,
        plugins: &[PathBuf],
        indent_width: u8,
        semi_colons: &str,
    ) -> Result<Self, String> {
        if !(1..=16).contains(&indent_width) {
            return Err("Indent width must be between 1 and 16.".into());
        }
        if !["always", "prefer", "asi"].contains(&semi_colons) {
            return Err("Semicolons must be always, prefer, or asi.".into());
        }
        let mut document = match original {
            Some(path) => ConfigDocument::load(path)?,
            None => ConfigDocument::standalone(plugins)?,
        };
        document.set(&["indentWidth"], Value::from(indent_width))?;
        document.set(
            &["typescript", "semiColons"],
            Value::String(semi_colons.into()),
        )?;
        Self::for_preview(&document.snapshot(), plugins)
    }

    /// Resolve only our exact pinned portable URLs to caller-verified bundled
    /// WASM files in this ephemeral copy. Other hosts, versions, local paths,
    /// plugins, comments and exported/original document text remain unchanged.
    pub fn for_preview(snapshot: &ConfigSnapshot, plugins: &[PathBuf]) -> Result<Self, String> {
        let root = parse_object(snapshot.source_text())?;
        let object = root.object_value().expect("validated root object");
        let Some(array) = unique_property(&object, "plugins")?.and_then(|prop| prop.array_value())
        else {
            return Self::from_snapshot(snapshot);
        };
        for element in array.elements() {
            let Some(literal) = element.as_string_lit() else {
                continue;
            };
            let reference = literal
                .decoded_value()
                .map_err(|e| format!("Invalid plugin reference: {e:?}"))?;
            let Some((name, _)) = PINNED_PLUGINS.iter().find(|(_, url)| *url == reference) else {
                continue;
            };
            let bundled = plugins.iter().find(|path| path.file_name() == Some(std::ffi::OsStr::new(name)))
                .ok_or_else(|| format!("The verified bundle is missing {name}; offline preview cannot resolve {reference}."))?;
            let bundled = fs::canonicalize(bundled)
                .map_err(|e| format!("Cannot locate bundled plugin {}: {e}", bundled.display()))?;
            let raw = serde_json::to_string(&bundled.to_string_lossy())
                .map_err(|e| format!("Cannot encode bundled plugin path: {e}"))?;
            literal.set_raw_value(raw);
        }
        Self::from_snapshot(&ConfigSnapshot {
            source_text: root.to_string(),
            original_path: snapshot.original_path.clone(),
        })
    }

    pub fn from_snapshot(snapshot: &ConfigSnapshot) -> Result<Self, String> {
        parse_object(snapshot.source_text())?;
        let directory = if snapshot.original_path().is_none() {
            Some(
                tempfile::tempdir()
                    .map_err(|e| format!("Cannot create configuration directory: {e}"))?,
            )
        } else {
            None
        };
        let parent = match snapshot.original_path() {
            Some(path) => path
                .parent()
                .ok_or("Configuration has no parent directory.")?,
            None => directory.as_ref().expect("standalone directory").path(),
        };
        // Same directory delegates relative extends, plugins, configDir and
        // overrides to dprint. Never reread or rewrite the original here.
        let mut file = tempfile::Builder::new()
            .prefix(".codestyles-draft-")
            .suffix(".jsonc")
            .tempfile_in(parent)
            .map_err(|e| {
                format!(
                    "Cannot create draft beside configuration (directory must be writable): {e}"
                )
            })?;
        file.write_all(snapshot.source_text().as_bytes())
            .map_err(|e| format!("Cannot write draft configuration: {e}"))?;
        file.flush()
            .map_err(|e| format!("Cannot flush draft configuration: {e}"))?;
        Ok(Self {
            file,
            _directory: directory,
        })
    }

    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document(text: &str) -> ConfigDocument {
        let mut doc = ConfigDocument::standalone(&[]).unwrap();
        doc.replace_draft_text(text).unwrap();
        doc.saved_text = text.into();
        doc
    }

    fn parse(text: &str) -> Value {
        jsonc_parser::parse_to_serde_value(text, &jsonc_options())
            .unwrap()
            .unwrap()
    }

    #[test]
    fn set_preserves_comments_unknown_nodes_and_saved_baseline() {
        let original = "// header\r\n{\r\n  // indent note\r\n  \"indentWidth\" : 2,\r\n  \"unknown\": { \"odd spacing\" : [1,  2,], /* untouched */ },\r\n  \"typescript\": { /* keep */ \"quoteStyle\" : \"preferSingle\", \"semiColons\" : \"always\", },\r\n  \"extends\": \"./base.json\",\r\n  \"plugins\": [\"./custom.wasm\"],\r\n  \"overrides\": [{\"includes\":\"${configDir}/special/**\",\"indentWidth\":8}],\r\n}\r\n";
        let mut doc = document(original);
        doc.set(&["indentWidth"], json!(4)).unwrap();
        doc.set(&["typescript", "semiColons"], json!("asi"))
            .unwrap();
        let expected = original
            .replace("\"indentWidth\" : 2", "\"indentWidth\" : 4")
            .replace("\"semiColons\" : \"always\"", "\"semiColons\" : \"asi\"");
        assert_eq!(doc.draft_text(), expected);
        assert_eq!(doc.saved_text(), original);
        assert!(doc.is_dirty());
        doc.discard_changes();
        assert_eq!(doc.draft_text(), original);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn unset_is_deletion_not_a_default_or_null_and_keeps_comments() {
        let mut doc = document(
            "{\n  // setting\n  \"indentWidth\": 4, // keep this note\n  \"typescript\": { /* inside */ \"semiColons\": \"asi\" },\n  \"unknown\": 7\n}\n",
        );
        doc.remove(&["indentWidth"]).unwrap();
        doc.remove(&["typescript", "semiColons"]).unwrap();
        assert_eq!(doc.value(&["indentWidth"]).unwrap(), None);
        assert_eq!(doc.value(&["typescript", "semiColons"]).unwrap(), None);
        assert_eq!(parse(doc.draft_text())["typescript"], json!({}));
        for comment in ["// setting", "// keep this note", "/* inside */"] {
            assert!(doc.draft_text().contains(comment));
        }
        assert!(doc.draft_text().contains("\"unknown\": 7"));
        let text = doc.draft_text().to_owned();
        doc.remove(&["absent", "child"]).unwrap();
        assert_eq!(doc.draft_text(), text);
    }

    #[test]
    fn deletion_handles_first_middle_last_and_trailing_commas() {
        for input in [r#"{"a":1,"b":2,"c":3}"#, r#"{"a":1,"b":2,"c":3,}"#] {
            for key in ["a", "b", "c"] {
                let mut doc = document(input);
                doc.remove(&[key]).unwrap();
                assert_eq!(parse(doc.draft_text()).as_object().unwrap().len(), 2);
                assert_eq!(doc.value(&[key]).unwrap(), None);
            }
        }
        let mut doc = document("{/* before */\"a\":1 /*after*/, /* final */}");
        doc.remove(&["a"]).unwrap();
        assert_eq!(parse(doc.draft_text()), json!({}));
        assert!(doc.draft_text().contains("/*after*/"));
    }

    #[test]
    fn failed_edits_keep_draft_and_duplicate_paths_are_rejected() {
        for input in [
            r#"{"typescript": false}"#,
            r#"{"typescript":{"semiColons":"asi","semiColons":"always"}}"#,
            r#"{"typescript":{},"typescript":{}}"#,
        ] {
            let mut doc = document(input);
            assert!(
                doc.set(&["typescript", "semiColons"], json!("prefer"))
                    .is_err()
            );
            assert_eq!(doc.draft_text(), input);
            assert!(doc.remove(&["typescript", "semiColons"]).is_err());
            assert_eq!(doc.draft_text(), input);
        }
        let mut doc = document("{\"untouched\": 1}");
        for invalid in ["", "[]", "{", "{\"a\":1 \"b\":2}", "{'a':1}"] {
            assert!(doc.replace_draft_text(invalid).is_err());
            assert_eq!(doc.draft_text(), "{\"untouched\": 1}");
        }
        assert!(doc.set(&[], json!(1)).is_err());
        assert!(doc.remove(&[""]).is_err());
        assert!(!doc.is_dirty());
    }

    #[test]
    fn unchanged_value_does_not_reformat_or_mark_dirty() {
        let original =
            "{\n \"indentWidth\" : 2, /* leave */\n \"typescript\": {\"semiColons\": \"asi\"}\n}";
        let mut doc = document(original);
        doc.set(&["indentWidth"], json!(2)).unwrap();
        doc.set(&["typescript", "semiColons"], json!("asi"))
            .unwrap();
        assert_eq!(doc.draft_text(), original);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn snapshot_is_immutable_preserves_plugins_and_temporary_file_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("dprint.jsonc");
        let text = "{ // comment\n\"plugins\":[\"./custom.wasm\"],\"extends\":\"./base.json\",\"indentWidth\":2}";
        fs::write(&original, text).unwrap();
        let mut doc = ConfigDocument::load(&original).unwrap();
        let snapshot = doc.snapshot();
        doc.set(&["indentWidth"], json!(4)).unwrap();
        fs::write(&original, "{}").unwrap();
        let path;
        {
            let draft = DraftConfig::from_snapshot(&snapshot).unwrap();
            path = draft.path().to_owned();
            assert_eq!(path.parent(), original.canonicalize().unwrap().parent());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
            assert!(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".codestyles-draft-")
            );
        }
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&original).unwrap(), "{}");
    }

    #[test]
    fn p0_constructor_preserves_comments_and_relative_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("dprint.jsonc");
        let text = "{ // keep my comments\n\"extends\": \"./base.json\", \"typescript\": {\"quoteStyle\":\"preferSingle\"}, \"overrides\": [{\"includes\":\"${configDir}/special/**\",\"indentWidth\":8}], }";
        fs::write(&original, text).unwrap();
        let draft = DraftConfig::new(Some(&original), &[], 4, "asi").unwrap();
        let rendered = fs::read_to_string(draft.path()).unwrap();
        let value = parse(&rendered);
        assert!(rendered.contains("// keep my comments"));
        assert_eq!(value["extends"], "./base.json");
        assert_eq!(value["typescript"]["quoteStyle"], "preferSingle");
        assert_eq!(value["typescript"]["semiColons"], "asi");
        assert_eq!(value["overrides"][0]["includes"], "${configDir}/special/**");
        assert_eq!(fs::read_to_string(&original).unwrap(), text);
    }

    #[test]
    fn explicit_save_updates_baseline_and_preserves_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("dprint.jsonc");
        fs::write(&original, "{ // keep\n\"indentWidth\":2}\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&original, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let mut doc = ConfigDocument::load(&original).unwrap();
        doc.set(&["indentWidth"], json!(4)).unwrap();
        let expected = doc.draft_text().to_owned();
        doc.save().unwrap();
        assert_eq!(fs::read_to_string(&original).unwrap(), expected);
        assert_eq!(doc.saved_text(), expected);
        assert!(!doc.is_dirty());
        assert!(!doc.has_external_changes().unwrap());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&original).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
    }

    #[test]
    fn external_changes_and_deletion_refuse_save_and_keep_draft() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("dprint.jsonc");
        fs::write(&original, "{\"indentWidth\":2}").unwrap();
        let mut doc = ConfigDocument::load(&original).unwrap();
        doc.set(&["indentWidth"], json!(4)).unwrap();
        let draft = doc.draft_text().to_owned();
        fs::write(&original, "{\"indentWidth\":8}").unwrap();
        assert!(doc.has_external_changes().unwrap());
        assert!(doc.save().unwrap_err().contains("changed on disk"));
        assert_eq!(
            fs::read_to_string(&original).unwrap(),
            "{\"indentWidth\":8}"
        );
        assert_eq!(doc.draft_text(), draft);
        assert!(doc.is_dirty());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        fs::remove_file(&original).unwrap();
        assert!(doc.has_external_changes().unwrap());
        assert!(doc.save().is_err());
        assert!(!original.exists());
    }

    #[test]
    fn save_as_creates_new_refuses_existing_and_keeps_failed_draft() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.jsonc");
        fs::write(&existing, "important").unwrap();
        let mut doc = ConfigDocument::standalone(&[]).unwrap();
        doc.set(&["lineWidth"], json!(100)).unwrap();
        let text = doc.draft_text().to_owned();
        assert!(doc.save().is_err());
        assert!(doc.save_as(&existing).is_err());
        assert!(
            doc.save_as(&dir.path().join("missing/config.jsonc"))
                .is_err()
        );
        assert_eq!(doc.draft_text(), text);
        assert!(doc.is_dirty());
        assert_eq!(fs::read_to_string(&existing).unwrap(), "important");
        assert!(doc.path().is_none());
        let new_path = dir.path().join("new.jsonc");
        doc.save_as(&new_path).unwrap();
        assert_eq!(doc.path(), Some(new_path.canonicalize().unwrap().as_path()));
        assert_eq!(fs::read_to_string(&new_path).unwrap(), text);
        assert!(!doc.is_dirty());
        doc.set(&["lineWidth"], json!(110)).unwrap();
        doc.save_as(&new_path).unwrap();
        assert_eq!(
            parse(&fs::read_to_string(&new_path).unwrap())["lineWidth"],
            110
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_paths_and_replaced_symlinks_are_safe() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original.jsonc");
        let link = dir.path().join("link.jsonc");
        let other = dir.path().join("other.jsonc");
        fs::write(&original, "{}").unwrap();
        fs::write(&other, "{}").unwrap();
        symlink(&original, &link).unwrap();
        let mut doc = ConfigDocument::load(&link).unwrap();
        assert_eq!(doc.path(), Some(original.canonicalize().unwrap().as_path()));
        doc.set(&["lineWidth"], json!(80)).unwrap();
        fs::remove_file(&original).unwrap();
        symlink(&other, &original).unwrap();
        assert!(doc.has_external_changes().unwrap());
        assert!(doc.save().is_err());
        assert_eq!(fs::read_to_string(&other).unwrap(), "{}");
        let dangling = dir.path().join("dangling.jsonc");
        symlink(dir.path().join("does-not-exist"), &dangling).unwrap();
        assert!(doc.save_as(&dangling).is_err());
    }

    #[test]
    fn inserting_missing_setting_preserves_unknown_nodes_and_unicode_deletion_is_lossless() {
        let original = "{\n  // 保持 комментарий 🎨\n  \"未知\": { \"字\" : [1,  2,], /* 不変 */ },\n  \"日本語\": { /* 中 */ \"引数\": \"😀\", \"残す\": \"é\" },\n  \"plugins\": [\"./私の.wasm\"]\n}\n";
        let mut doc = document(original);
        doc.remove(&["日本語", "引数"]).unwrap();
        assert_eq!(doc.draft_text(), original.replace("\"引数\": \"😀\",", ""));
        doc.set(&["typescript", "semiColons"], json!("asi"))
            .unwrap();
        assert!(
            doc.draft_text()
                .contains("\"未知\": { \"字\" : [1,  2,], /* 不変 */ }")
        );
        assert!(doc.draft_text().contains("// 保持 комментарий 🎨"));
        assert_eq!(doc.value(&["日本語", "残す"]).unwrap(), Some(json!("é")));
        assert_eq!(
            doc.value(&["typescript", "semiColons"]).unwrap(),
            Some(json!("asi"))
        );
    }

    #[test]
    fn final_conflict_check_cleans_up_written_temporary_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dprint.jsonc");
        fs::write(&path, "{\"indentWidth\":2}").unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        doc.set(&["indentWidth"], json!(4)).unwrap();
        let expected = doc.draft_text().to_owned();
        let result = doc.save_with_before_commit(|| {
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
            fs::write(&path, "{\"indentWidth\":8}").unwrap();
        });
        assert!(result.unwrap_err().contains("changed on disk"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"indentWidth\":8}");
        assert_eq!(doc.draft_text(), expected);
        assert!(doc.is_dirty());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        fs::remove_file(&path).unwrap();
        assert!(doc.save_as(&path).unwrap_err().contains("changed on disk"));
        assert!(!path.exists());
    }

    fn fake_bundle(dir: &Path) -> Vec<PathBuf> {
        PINNED_PLUGINS
            .iter()
            .map(|(name, _)| {
                let path = dir.join(name);
                fs::write(&path, "test fixture, not executable").unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn standalone_export_is_portable_but_preview_uses_bundle_and_keeps_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = fake_bundle(dir.path());
        let mut doc = ConfigDocument::standalone(&plugins).unwrap();
        let original = doc.draft_text().to_owned();
        for (_, url) in PINNED_PLUGINS {
            assert!(original.contains(url));
        }
        assert!(!original.contains(dir.path().to_str().unwrap()));
        let saved = dir.path().join("export.jsonc");
        doc.save_as(&saved).unwrap();
        assert_eq!(fs::read_to_string(&saved).unwrap(), original);
        let reopened = ConfigDocument::load(&saved).unwrap();
        let snapshot = reopened.snapshot();
        let preview = DraftConfig::for_preview(&snapshot, &plugins).unwrap();
        let preview_value = parse(&fs::read_to_string(preview.path()).unwrap());
        assert_eq!(
            preview.path().parent(),
            saved.canonicalize().unwrap().parent()
        );
        for (index, path) in plugins.iter().enumerate() {
            assert_eq!(
                preview_value["plugins"][index],
                path.canonicalize().unwrap().to_string_lossy().as_ref()
            );
        }
        assert_eq!(snapshot.source_text(), original);
        assert_eq!(doc.draft_text(), original);
        assert_eq!(fs::read_to_string(&saved).unwrap(), original);
        let exact = DraftConfig::from_snapshot(&snapshot).unwrap();
        assert_eq!(fs::read_to_string(exact.path()).unwrap(), original);
        assert!(DraftConfig::for_preview(&snapshot, &[]).is_err());
    }

    #[test]
    fn preview_substitution_is_exact_and_keeps_unknown_plugins_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = fake_bundle(dir.path());
        let text = r#"{
  "plugins": [
    // local offline mapping
    "https://plugins.dprint.dev/typescript-0.96.1.wasm", /* retain this */
    "https://other.example/typescript-0.96.1.wasm",
    "https://plugins.dprint.dev/typescript-0.97.0.wasm",
    "https://plugins.dprint.dev/json-0.25.2.wasm#different-checksum",
    "./json-0.25.2.wasm",
    "./unfamiliar.wasm"
  ],
  "unknown": {"custom": true}
}"#;
        let doc = document(text);
        let draft = DraftConfig::for_preview(&doc.snapshot(), &plugins).unwrap();
        let rendered = fs::read_to_string(draft.path()).unwrap();
        let encoded =
            serde_json::to_string(&plugins[0].canonicalize().unwrap().to_string_lossy()).unwrap();
        assert_eq!(
            rendered,
            text.replace(
                "\"https://plugins.dprint.dev/typescript-0.96.1.wasm\"",
                &encoded
            )
        );
        assert_eq!(doc.draft_text(), text);
        let unsupported = dir.path().join("typescript-99.wasm");
        fs::write(&unsupported, "fixture").unwrap();
        assert!(ConfigDocument::standalone(&[unsupported]).is_err());
    }

    #[test]
    fn save_validation_matches_pinned_core_indent_runtime_and_retains_invalid_draft() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.jsonc");
        let baseline = "{\"indentWidth\":2}";
        fs::write(&path, baseline).unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        for invalid in [
            json!(-1),
            json!(256),
            json!(2.0),
            json!(2.5),
            json!(true),
            Value::Null,
            json!("oops"),
            json!(" 4"),
            json!("4 "),
            json!("-0"),
            json!("2.0"),
            json!("256"),
            json!([]),
            json!({}),
        ] {
            let text = serde_json::to_string(&json!({"indentWidth":invalid})).unwrap();
            doc.replace_draft_text(&text).unwrap();
            assert!(doc.save().unwrap_err().contains("indentWidth"), "{invalid}");
            assert!(
                doc.save_as(&dir.path().join("new.jsonc"))
                    .unwrap_err()
                    .contains("indentWidth")
            );
            assert_eq!(doc.draft_text(), text);
            assert_eq!(doc.saved_text(), baseline);
            assert_eq!(fs::read_to_string(&path).unwrap(), baseline);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
        for valid in [
            json!(0),
            json!(17),
            json!(255),
            json!("4"),
            json!("+4"),
            json!("04"),
            json!("255"),
        ] {
            doc.set(&["indentWidth"], valid).unwrap();
            doc.save().unwrap();
            assert!(!doc.is_dirty());
            assert_eq!(fs::read_to_string(&path).unwrap(), doc.draft_text());
        }
        doc.replace_draft_text("{\"indentWidth\":-0}").unwrap();
        doc.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"indentWidth\":-0}");
        for literal in ["0.0", "0e0", "1e1"] {
            doc.replace_draft_text(&format!("{{\"indentWidth\":{literal}}}"))
                .unwrap();
            assert!(doc.save().unwrap_err().contains("indentWidth"));
        }
        fs::write(&path, "{\"indentWidth\":false}").unwrap();
        let mut clean_invalid = ConfigDocument::load(&path).unwrap();
        assert!(!clean_invalid.is_dirty());
        assert!(clean_invalid.save().unwrap_err().contains("indentWidth"));
    }

    #[test]
    fn known_typescript_validation_matches_schema_without_rejecting_unknown_fields() {
        let schema: Value =
            serde_json::from_str(include_str!("../resources/schemas/typescript-0.96.1.json"))
                .unwrap();
        assert_eq!(schema["properties"]["indentWidth"]["type"], "number");
        let enum_values = schema["definitions"]["semiColons"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["const"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(enum_values, ["always", "prefer", "asi"]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.jsonc");
        let baseline = serde_json::to_string(&json!({"plugins":[PINNED_PLUGINS[0].1]})).unwrap();
        fs::write(&path, &baseline).unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        for invalid in [
            json!("never"),
            json!(1),
            json!(false),
            Value::Null,
            json!([]),
        ] {
            doc.set(&["typescript", "semiColons"], invalid).unwrap();
            let draft = doc.draft_text().to_owned();
            assert!(doc.save().unwrap_err().contains("semiColons"));
            assert!(
                doc.save_as(&dir.path().join("new.jsonc"))
                    .unwrap_err()
                    .contains("semiColons")
            );
            assert_eq!(doc.saved_text(), baseline);
            assert_eq!(doc.draft_text(), draft);
            assert_eq!(fs::read_to_string(&path).unwrap(), baseline);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
        for valid in enum_values {
            doc.set(&["typescript", "semiColons"], json!(valid))
                .unwrap();
            doc.set(
                &["typescript", "futureUnknownField"],
                json!({"custom":true}),
            )
            .unwrap();
            doc.save().unwrap();
        }
        doc.set(&["typescript"], json!(false)).unwrap();
        assert!(
            doc.save()
                .unwrap_err()
                .contains("typescript must be an object")
        );
    }

    #[test]
    fn semantic_save_does_not_guess_unknown_or_inherited_plugin_schemas() {
        let dir = tempfile::tempdir().unwrap();
        let pinned = PINNED_PLUGINS[0].1;
        let cases = [
            json!({"plugins":["https://plugins.dprint.dev/typescript-0.97.0.wasm"],"typescript":{"semiColons":"custom"}}),
            json!({"plugins":["https://untrusted.example/typescript-0.96.1.wasm"],"typescript":{"semiColons":42}}),
            json!({"plugins":["./typescript-0.96.1.wasm"],"typescript":false}),
            json!({"plugins":[pinned,"./custom.wasm"],"typescript":{"semiColons":"custom"}}),
            json!({"plugins":[pinned],"extends":"./base.json","typescript":{"semiColons":"custom"}}),
            json!({"plugins":[pinned],"inherit":true,"typescript":{"semiColons":"custom"}}),
            json!({"typescript":{"semiColons":"custom"}}),
        ];
        for (index, value) in cases.iter().enumerate() {
            let text = serde_json::to_string(value).unwrap();
            let mut doc = document(&text);
            let path = dir.path().join(format!("unknown-{index}.jsonc"));
            doc.save_as(&path).unwrap();
            assert_eq!(fs::read_to_string(path).unwrap(), text);
        }
        let mut known_no_inherit = document(
            &serde_json::to_string(
                &json!({"plugins":[pinned],"inherit":false,"typescript":{"semiColons":"custom"}}),
            )
            .unwrap(),
        );
        assert!(
            known_no_inherit
                .save_as(&dir.path().join("bad.jsonc"))
                .unwrap_err()
                .contains("semiColons")
        );
    }
}
