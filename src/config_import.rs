//! Shared bounded reader for explicitly selected native formatter configuration.
use std::{fs, io::Read, path::Path};
pub(crate) const LIMIT: usize = 1024 * 1024;

pub(crate) fn read_utf8(path: &Path, label: &str) -> Result<String, String> {
    fn regular(metadata: &fs::Metadata) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return false;
            }
        }
        metadata.is_file() && !metadata.file_type().is_symlink()
    }
    let before = fs::symlink_metadata(path).map_err(|e| format!("Cannot inspect {label}: {e}"))?;
    if !regular(&before) {
        return Err("Configuration import requires a regular non-symlink file.".into());
    }
    if before.len() > LIMIT as u64 {
        return Err("Configuration import exceeds the 1 MiB limit.".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|e| format!("Cannot open {label} safely: {e}"))?;
    if !regular(
        &file
            .metadata()
            .map_err(|e| format!("Cannot inspect opened {label}: {e}"))?,
    ) {
        return Err("Configuration import requires a regular non-symlink file.".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read {label}: {e}"))?;
    if bytes.len() > LIMIT {
        return Err("Configuration import exceeds the 1 MiB limit.".into());
    }
    String::from_utf8(bytes).map_err(|_| format!("{label} is not valid UTF-8."))
}
