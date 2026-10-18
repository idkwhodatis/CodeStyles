//! Temporary draft configuration. The original file is never opened for writing.
use serde_json::{Map, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::{NamedTempFile, TempDir};

/// Keeps the draft alive until the child process and all pipe readers have exited.
pub struct DraftConfig {
    file: NamedTempFile,
    _directory: Option<TempDir>,
}

impl DraftConfig {
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
        let (mut root, parent, directory) = if let Some(path) = original {
            let path = fs::canonicalize(path)
                .map_err(|e| format!("Cannot open configuration {}: {e}", path.display()))?;
            let input =
                fs::read_to_string(&path).map_err(|e| format!("Cannot read configuration: {e}"))?;
            let parsed = jsonc_parser::parse_to_serde_value(&input, &Default::default())
                .map_err(|e| format!("Invalid JSONC configuration: {e}"))?;
            let value = parsed.unwrap_or(Value::Null);
            let parent = path
                .parent()
                .ok_or("Configuration has no parent directory")?
                .to_path_buf();
            (value, parent, None)
        } else {
            let dir = tempfile::tempdir()
                .map_err(|e| format!("Cannot create configuration directory: {e}"))?;
            let parent = dir.path().to_path_buf();
            let plugin_values: Result<Vec<Value>, String> = plugins
                .iter()
                .map(|path| {
                    fs::canonicalize(path)
                        .map(|path| Value::String(path.to_string_lossy().into_owned()))
                        .map_err(|e| {
                            format!("Cannot locate bundled plugin {}: {e}", path.display())
                        })
                })
                .collect();
            let mut root = Map::new();
            root.insert("plugins".into(), Value::Array(plugin_values?));
            (Value::Object(root), parent, Some(dir))
        };
        let object = root
            .as_object_mut()
            .ok_or("Configuration must contain a JSON object.")?;
        object.insert("indentWidth".into(), Value::from(indent_width));
        let typescript = object
            .entry("typescript")
            .or_insert_with(|| Value::Object(Map::new()));
        let typescript = typescript
            .as_object_mut()
            .ok_or("The typescript configuration must be an object.")?;
        typescript.insert("semiColons".into(), Value::String(semi_colons.into()));
        // Never use a discoverable dprint.json name. Keeping this beside the original
        // delegates relative extends, plugin paths, configDir, and overrides to dprint.
        let mut file = tempfile::Builder::new()
            .prefix(".codestyles-draft-")
            .suffix(".json")
            .tempfile_in(&parent)
            .map_err(|e| {
                format!(
                    "Cannot create draft beside configuration (directory must be writable): {e}"
                )
            })?;
        serde_json::to_writer_pretty(file.as_file_mut(), &root)
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
    #[test]
    fn keeps_original_and_relative_semantics_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("dprint.jsonc");
        let contents = "{ // keep my comments\n\"extends\": \"./base.json\", \"typescript\": {\"quoteStyle\":\"preferSingle\"}, \"overrides\": [{\"includes\":\"${configDir}/special/**\",\"indentWidth\":8}], }";
        fs::write(&original, contents).unwrap();
        let path;
        {
            let draft = DraftConfig::new(Some(&original), &[], 4, "asi").unwrap();
            path = draft.path().to_path_buf();
            assert_eq!(path.parent(), original.canonicalize().unwrap().parent());
            let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(v["extends"], "./base.json");
            assert_eq!(v["typescript"]["quoteStyle"], "preferSingle");
            assert_eq!(v["typescript"]["semiColons"], "asi");
            assert_eq!(v["overrides"][0]["includes"], "${configDir}/special/**");
            assert_eq!(fs::read_to_string(&original).unwrap(), contents);
        }
        assert!(!path.exists());
    }
}
