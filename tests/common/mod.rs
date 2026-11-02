//! Explicit real-CLI fixtures for parity testing, never production discovery.
#![allow(dead_code)]
use codestyles::{backend::Backend, schema::PinnedPlugin};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn resources() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources")
}

pub fn verified_backend(root: &Path) -> Result<Backend, String> {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("manifest.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if manifest["dprint_version"] != "0.63.0" {
        return Err("Unsupported pinned dprint version".into());
    }
    let executable = format!(
        "bin/{}/dprint{}",
        codestyles::resources::target()?,
        if cfg!(windows) { ".exe" } else { "" }
    );
    let plugins = PinnedPlugin::ALL.map(|p| format!("plugins/{}", p.filename()));
    for path in std::iter::once(executable.as_str()).chain(plugins.iter().map(String::as_str)) {
        let entry = manifest["files"]
            .as_array()
            .ok_or("Missing fixture manifest files")?
            .iter()
            .find(|entry| entry["path"] == path)
            .ok_or_else(|| format!("Missing fixture manifest entry: {path}"))?;
        let bytes = std::fs::read(root.join(path)).map_err(|e| {
            format!("Real parity fixture missing ({path}); explicitly fetch test assets: {e}")
        })?;
        if entry["sha256"] != format!("{:x}", Sha256::digest(&bytes)) {
            return Err(format!("Fixture checksum mismatch: {path}"));
        }
    }
    Ok(Backend::new(
        root.join(executable),
        plugins.map(|path| root.join(path)).into(),
    ))
}

pub fn backend() -> Backend {
    verified_backend(&resources()).expect("verified real dprint parity fixtures")
}
