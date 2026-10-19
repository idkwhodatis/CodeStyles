//! Explicit, checksummed runtime resources. Never searches PATH for dprint.
use crate::{backend::Backend, schema::PinnedPlugin};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn target() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        _ => Err("This architecture is not included in the resource bundle.".into()),
    }
}

/// The resource root and how it was found, used by package diagnostics.
#[derive(Clone, Debug)]
pub struct ResourceLocation {
    pub root: PathBuf,
    pub discovery: &'static str,
}

pub fn bundled_backend() -> Result<Backend, String> {
    backend_from_root(&resolve_resources()?.root)
}

pub fn resolve_resources() -> Result<ResourceLocation, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    // A release executable must never silently use the build machine's checkout.
    #[cfg(debug_assertions)]
    let development = Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources"));
    #[cfg(not(debug_assertions))]
    let development: Option<PathBuf> = None;
    locate_resources(
        &executable,
        std::env::var_os("CODESTYLES_RESOURCES").map(PathBuf::from),
        development,
    )
}

fn locate_resources(
    executable: &Path,
    explicit: Option<PathBuf>,
    development: Option<PathBuf>,
) -> Result<ResourceLocation, String> {
    let parent = executable.parent().ok_or("Executable has no parent")?;
    let (root, discovery) = if let Some(root) = explicit {
        (root, "environment_override")
    } else {
        let adjacent = parent.join("resources");
        let app_resources = parent
            .parent()
            .filter(|contents| {
                parent.file_name() == Some(std::ffi::OsStr::new("MacOS"))
                    && contents.file_name() == Some(std::ffi::OsStr::new("Contents"))
            })
            .map(|contents| contents.join("Resources/resources"));
        if let Some(root) = app_resources.filter(|path| path.is_dir()) {
            (root, "macos_app_bundle")
        } else if adjacent.is_dir() {
            (adjacent, "beside_executable")
        } else if let Some(root) = development {
            (root, "development_checkout")
        } else {
            return Err(format!(
                "Bundled resources are missing beside {}. Reinstall the complete CodeStyles package, or explicitly set CODESTYLES_RESOURCES.",
                executable.display()
            ));
        }
    };
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cannot locate resources {}: {e}", root.display()))?;
    if !root.is_dir() {
        return Err(format!(
            "Resource root is not a directory: {}",
            root.display()
        ));
    }
    Ok(ResourceLocation { root, discovery })
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
    let plugins = PinnedPlugin::ALL.map(|plugin| format!("plugins/{}", plugin.filename()));
    for name in std::iter::once(executable.as_str()).chain(plugins.iter().map(String::as_str)) {
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

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn portable_bundle_uses_executable_directory_not_current_directory() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("CodeStyles 安装 with spaces");
        std::fs::create_dir_all(parent.join("resources")).unwrap();
        let found = locate_resources(&parent.join("codestyles"), None, None).unwrap();
        assert_eq!(found.root, parent.join("resources").canonicalize().unwrap());
        assert_eq!(found.discovery, "beside_executable");
    }

    #[test]
    fn macos_standard_app_resources_are_discovered() {
        let temp = tempfile::tempdir().unwrap();
        let contents = temp.path().join("CodeStyles.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Resources/resources")).unwrap();
        let found = locate_resources(&contents.join("MacOS/codestyles"), None, None).unwrap();
        assert_eq!(found.discovery, "macos_app_bundle");
        assert_eq!(
            found.root,
            contents.join("Resources/resources").canonicalize().unwrap()
        );
    }

    #[test]
    fn release_without_resources_fails_instead_of_using_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("codestyles");
        assert!(
            locate_resources(&executable, None, None)
                .unwrap_err()
                .contains("Reinstall")
        );
        let found = locate_resources(&executable, None, Some(temp.path().into())).unwrap();
        assert_eq!(found.discovery, "development_checkout");
    }

    #[test]
    fn explicit_override_never_silently_falls_back() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("codestyles");
        let missing = temp.path().join("missing");
        assert!(locate_resources(&executable, Some(missing), Some(temp.path().into())).is_err());
        let found = locate_resources(&executable, Some(temp.path().into()), None).unwrap();
        assert_eq!(found.discovery, "environment_override");
    }
}
