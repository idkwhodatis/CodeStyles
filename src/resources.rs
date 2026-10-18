//! Explicit, checksummed runtime resources. Never searches PATH for dprint.
use crate::backend::Backend;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn target() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        _ => Err("This architecture is not included in the P0 resource bundle.".into()),
    }
}

pub fn bundled_backend() -> Result<Backend, String> {
    let root = if let Some(root) = std::env::var_os("CODESTYLES_RESOURCES") {
        PathBuf::from(root)
    } else {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let adjacent = exe
            .parent()
            .ok_or("Executable has no parent")?
            .join("resources");
        if adjacent.is_dir() {
            adjacent
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources")
        }
    };
    backend_from_root(&root)
}

pub fn backend_from_root(root: &Path) -> Result<Backend, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot locate resources: {e}"))?;
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("manifest.json"))
            .map_err(|e| format!("Missing resource manifest: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    if manifest["dprint_version"] != "0.63.0" {
        return Err("Unsupported bundled dprint version".into());
    }
    let files = manifest["files"]
        .as_array()
        .ok_or("Manifest is missing files")?;
    let executable = format!(
        "bin/{}/dprint{}",
        target()?,
        if cfg!(windows) { ".exe" } else { "" }
    );
    let plugins = ["plugins/typescript-0.96.1.wasm", "plugins/json-0.25.2.wasm"];
    for name in std::iter::once(executable.as_str()).chain(plugins) {
        let entry = files
            .iter()
            .find(|entry| entry["path"].as_str() == Some(name))
            .ok_or_else(|| format!("Resource absent from manifest: {name}"))?;
        let bytes = std::fs::read(root.join(name))
            .map_err(|e| format!("Cannot read bundled {name}: {e}"))?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if entry["sha256"].as_str() != Some(actual.as_str()) {
            return Err(format!("Bundled resource checksum mismatch: {name}"));
        }
    }
    Ok(Backend::new(
        root.join(executable),
        plugins.iter().map(|p| root.join(p)).collect(),
    ))
}
