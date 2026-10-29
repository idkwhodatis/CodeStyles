//! Opt-in, pinned toolchain packs. No PATH discovery, install scripts or caller URLs.
//!
//! `install` is blocking and belongs on a worker thread. Downloads are cancellable;
//! activation is a final atomic pointer replacement, leaving the old runtime intact
//! on failure. Unix managed directories are owner-writable only; Windows uses
//! inherited directory ACLs (no stronger ACL guarantee is claimed). This is not a sandbox against another
//! process running as the same account concurrently replacing filesystem objects.
use fs2::FileExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

const MARKER: &str = ".codestyles-managed-tools";
const MARKER_CONTENT: &[u8] = b"CodeStyles toolchains v1\n";
const RECEIPT: &str = ".receipt.json";
const MAX_METADATA: u64 = 1024 * 1024;
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

// fs2 uses EWOULDBLOCK on Unix, but ERROR_LOCK_VIOLATION on Windows,
// which Rust does not classify as ErrorKind::WouldBlock. Match only the
// library's native contention code; unrelated I/O errors must still fail.
fn lock_is_contended(error: &io::Error) -> bool {
    error.raw_os_error().is_some()
        && error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolchainStatus {
    Missing,
    Installed { version: String, root: PathBuf },
    Corrupt { reason: String },
}

#[derive(Clone, Debug)]
pub struct InstallProgress {
    pub phase: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ResolvedToolchain {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub version: String,
    pub environment: Vec<(OsString, OsString)>,
    // Retains a shared pack lock until the formatter process is finished.
    _lease: Option<Arc<File>>,
}

type RuntimeStamps = BTreeMap<PathBuf, FileStamp>;
type VerificationCache = BTreeMap<String, (PathBuf, RuntimeStamps)>;

#[derive(Clone, Debug)]
pub struct ToolchainManager {
    root: PathBuf,
    target: String,
    catalog: Value,
    verified: Arc<Mutex<VerificationCache>>,
}

impl ToolchainManager {
    /// Prefer a dedicated directory beside the executable; an unsuccessful write
    /// probe falls back to OS user data. `CODESTYLES_TRANSIENT_LAUNCH=1` explicitly
    /// requests user data for archive/translocated launches. We do not guess ZIPs.
    /// `CODESTYLES_TOOLCHAINS_DIR` selects an explicit dedicated managed root.
    pub fn discover() -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(err)?;
        let root = choose_root(
            &executable,
            std::env::var_os("CODESTYLES_TOOLCHAINS_DIR"),
            std::env::var_os("CODESTYLES_TRANSIENT_LAUNCH").as_deref() == Some(OsStr::new("1")),
            user_data_root(std::env::consts::OS, |key| std::env::var_os(key)),
        )?;
        Self::at_managed_root(root)
    }

    /// Explicit location, not an arbitrary manifest or executable. Existing
    /// nonempty/unmarked directories are rejected, never adopted or deleted.
    pub fn at_managed_root(root: PathBuf) -> Result<Self, String> {
        initialize_root(&root)?;
        let catalog: Value = serde_json::from_str(include_str!("../resources/toolchains.json"))
            .map_err(|e| format!("Invalid built-in toolchain manifest: {e}"))?;
        if catalog["format_version"] != 1 {
            return Err("Unsupported built-in toolchain manifest.".into());
        }
        Ok(Self {
            root: root.canonicalize().map_err(err)?,
            target: crate::resources::target()?.into(),
            catalog,
            verified: Default::default(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn status(&self, id: &str) -> Result<ToolchainStatus, String> {
        self.status_cancellable(id, &AtomicBool::new(false))
    }

    pub fn status_cancellable(
        &self,
        id: &str,
        cancel: &AtomicBool,
    ) -> Result<ToolchainStatus, String> {
        check_cancel(cancel)?;
        self.spec(id)?;
        self.check_root()?;
        let pack = self.root.join(id);
        if !path_present(&pack)? {
            return Ok(ToolchainStatus::Missing);
        }
        let _lock = self.lock(id, cancel, false)?;
        if !path_present(&pack.join("active.json"))? {
            return Ok(ToolchainStatus::Missing);
        }
        let resolved = self.resolve_locked_cancellable(id, cancel);
        check_cancel(cancel)?;
        Ok(match resolved {
            Ok(runtime) => ToolchainStatus::Installed {
                version: runtime.version,
                root: runtime.root,
            },
            Err(reason) => ToolchainStatus::Corrupt { reason },
        })
    }

    pub fn resolve(&self, id: &str) -> Result<ResolvedToolchain, String> {
        self.resolve_cancellable(id, &AtomicBool::new(false))
    }

    pub fn resolve_cancellable(
        &self,
        id: &str,
        cancel: &AtomicBool,
    ) -> Result<ResolvedToolchain, String> {
        self.spec(id)?;
        let lock = self.lock(id, cancel, false)?;
        let mut runtime = self.resolve_locked_cancellable(id, cancel)?;
        runtime._lease = Some(Arc::new(lock));
        Ok(runtime)
    }

    pub fn install(
        &self,
        id: &str,
        cancelled: Arc<AtomicBool>,
        progress: impl Fn(InstallProgress),
    ) -> Result<ResolvedToolchain, String> {
        self.download_available(id)?;
        self.install_with(
            id,
            &cancelled,
            &progress,
            |archive, output, cancel, progress| download(archive, output, cancel, progress),
        )
    }

    pub fn download_available(&self, id: &str) -> Result<(), String> {
        let (_, spec) = self.spec(id)?;
        if spec["available"] != true {
            return Err(spec["reason"]
                .as_str()
                .unwrap_or("This toolchain bundle has not been published to GitHub.")
                .into());
        }
        for archive in array(spec, "archives")? {
            trusted_download_url(string(archive, "url")?)?;
        }
        if array(spec, "archives")?.is_empty() {
            return Err("No published toolchain bundle is configured.".into());
        }
        Ok(())
    }

    /// Local acceptance testing/import of the exact built-in candidate bundle.
    /// This never accepts a caller manifest, digest, URL or executable path.
    pub fn install_verified_archive(
        &self,
        id: &str,
        archive: &Path,
        cancelled: Arc<AtomicBool>,
        progress: impl Fn(InstallProgress),
    ) -> Result<ResolvedToolchain, String> {
        let (_, spec) = self.spec(id)?;
        if array(spec, "archives")?.len() != 1 {
            return Err("No single pinned local bundle is available for this platform.".into());
        }
        regular_file(archive)?;
        self.install_with(
            id,
            &cancelled,
            &progress,
            |spec, output, cancel, progress| {
                let maximum = number(spec, "max_download_bytes")?;
                let mut source = File::open(archive).map_err(err)?;
                let mut target = new_file(output)?;
                let mut buffer = [0u8; 65536];
                let mut bytes = 0u64;
                loop {
                    check_cancel(cancel)?;
                    let count = source.read(&mut buffer).map_err(err)?;
                    if count == 0 {
                        break;
                    }
                    bytes += count as u64;
                    if bytes > maximum {
                        return Err("Local toolchain bundle exceeds compressed size limit.".into());
                    }
                    target.write_all(&buffer[..count]).map_err(err)?;
                    progress(InstallProgress {
                        phase: "copying",
                        downloaded_bytes: bytes,
                        total_bytes: spec["bytes"].as_u64(),
                    });
                }
                target.sync_all().map_err(err)
            },
        )
    }

    fn install_with(
        &self,
        id: &str,
        cancel: &AtomicBool,
        progress: &dyn Fn(InstallProgress),
        fetch: impl Fn(&Value, &Path, &AtomicBool, &dyn Fn(InstallProgress)) -> Result<(), String>,
    ) -> Result<ResolvedToolchain, String> {
        let (version, spec) = self.spec(id)?;
        let lock = self.lock(id, cancel, true)?;
        check_cancel(cancel)?;
        let pack = self.root.join(id);
        let stage = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(&pack)
            .map_err(err)?;
        let runtime = stage.path().join("runtime");
        create_directory(&runtime)?;
        let archives = array(spec, "archives")?;
        if archives.is_empty() {
            return Err("No pinned toolchain bundle exists for this platform.".into());
        }
        let mut expected = BTreeMap::new();
        let mut installed = 0u64;
        let limit = number(spec, "max_installed_bytes")?;
        for (index, archive) in archives.iter().enumerate() {
            check_cancel(cancel)?;
            let path = stage.path().join(format!("archive-{index}.tar.xz"));
            progress(InstallProgress {
                phase: "downloading",
                downloaded_bytes: 0,
                total_bytes: archive["bytes"].as_u64(),
            });
            fetch(archive, &path, cancel, progress)?;
            verify_archive(&path, archive, cancel)?;
            progress(InstallProgress {
                phase: "extracting",
                downloaded_bytes: 0,
                total_bytes: None,
            });
            extract_archive(
                &path,
                archive,
                &runtime,
                cancel,
                &mut expected,
                &mut installed,
                limit,
            )?;
        }
        let receipt = json!({"format_version":1,"id":id,"version":version,"target":self.target,"files":expected});
        write_new(
            &runtime.join(RECEIPT),
            &serde_json::to_vec(&receipt).map_err(err)?,
        )?;
        validate_runtime(&runtime, spec, &receipt, cancel)?;
        check_cancel(cancel)?;
        // Each activation uses a fresh immutable directory; never overwrite a
        // version that an existing process may still be using (notably Windows).
        let suffix = stage
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(".staging-", "");
        let name = format!("runtime-{suffix}");
        let destination = pack.join(&name);
        if path_present(&destination)? {
            return Err("Runtime destination already exists.".into());
        }
        fs::rename(&runtime, &destination).map_err(err)?;
        let active =
            json!({"format_version":1,"directory":name,"version":version,"target":self.target});
        if let Err(error) =
            check_cancel(cancel).and_then(|_| atomic_json(&pack.join("active.json"), &active))
        {
            // This is our newly created, never-active directory only.
            let _ = remove_managed_runtime(&destination, spec);
            return Err(error);
        }
        // The full runtime was already hashed under this exclusive lock before
        // its rename. Seed this manager's metadata cache after activation instead
        // of hashing large compiler libraries a second time on the success path.
        if let Ok(stamps) = runtime_stamps(&destination, &pack.join("active.json"))
            && let Ok(mut cache) = self.verified.lock()
        {
            cache.insert(id.into(), (destination.clone(), stamps));
        }
        progress(InstallProgress {
            phase: "installed",
            downloaded_bytes: 0,
            total_bytes: None,
        });
        drop(lock);
        // Activation has committed. A cancellation arriving after this point
        // must not turn a successful install into a misleading cancelled result.
        self.resolve(id)
    }

    /// Removes only recognized CodeStyles runtime directories in this pack.
    /// Foreign files or symlinks fail closed rather than extending deletion scope.
    pub fn uninstall(&self, id: &str) -> Result<(), String> {
        self.uninstall_cancellable(id, &AtomicBool::new(false))
    }

    /// Cancellation is honored through validation and waiting for active users.
    /// Once the pointer is unpublished, deletion finishes without interruption.
    pub fn uninstall_cancellable(&self, id: &str, cancel: &AtomicBool) -> Result<(), String> {
        let (_, spec) = self.spec(id)?;
        let _lock = self.lock(id, cancel, true)?;
        let pack = self.root.join(id);
        let mut runtimes = Vec::new();
        for entry in fs::read_dir(&pack).map_err(err)? {
            check_cancel(cancel)?;
            let entry = entry.map_err(err)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".lock" || name == "active.json" {
                regular_file(&entry.path())?;
                continue;
            }
            if !name.starts_with("runtime-") {
                return Err(format!("Unrecognized managed pack entry: {name}"));
            }
            validate_removal(&entry.path(), spec)?;
            runtimes.push(entry.path());
        }
        // Unpublish before removing bytes. A failed deletion leaves a safely
        // inactive runtime that can be retried, never an active half-install.
        let pointer = pack.join("active.json");
        check_cancel(cancel)?;
        if path_present(&pointer)? {
            fs::remove_file(pointer).map_err(err)?;
        }
        for root in runtimes {
            remove_managed_runtime(&root, spec)?;
        }
        Ok(())
    }

    fn spec(&self, id: &str) -> Result<(&str, &Value), String> {
        safe_component(id)?;
        let tool = self.catalog["toolchains"]
            .get(id)
            .ok_or("Unknown toolchain pack.")?;
        let version = string(tool, "version")?;
        let spec = tool["targets"]
            .get(&self.target)
            .ok_or("This toolchain is unavailable for this platform.")?;
        Ok((version, spec))
    }

    fn check_root(&self) -> Result<(), String> {
        real_directory(&self.root)?;
        if read_small(&self.root.join(MARKER))? != MARKER_CONTENT {
            return Err("Toolchain directory is not owned by CodeStyles.".into());
        }
        Ok(())
    }

    fn lock(&self, id: &str, cancel: &AtomicBool, exclusive: bool) -> Result<File, String> {
        self.check_root()?;
        self.spec(id)?;
        let pack = self.root.join(id);
        create_directory(&pack)?;
        let path = pack.join(".lock");
        if path_present(&path)? {
            regular_file(&path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(err)?;
        regular_file(&path)?;
        let started = Instant::now();
        loop {
            check_cancel(cancel)?;
            match if exclusive {
                FileExt::try_lock_exclusive(&file)
            } else {
                FileExt::try_lock_shared(&file)
            } {
                Ok(()) => return Ok(file),
                Err(error) if lock_is_contended(&error) => {
                    if started.elapsed() >= LOCK_TIMEOUT {
                        return Err("Another toolchain operation is still running.".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(err(error)),
            }
        }
    }

    fn resolve_locked_cancellable(
        &self,
        id: &str,
        cancel: &AtomicBool,
    ) -> Result<ResolvedToolchain, String> {
        check_cancel(cancel)?;
        self.check_root()?;
        let (version, spec) = self.spec(id)?;
        let pack = self.root.join(id);
        real_directory(&pack)?;
        let active: Value =
            serde_json::from_slice(&read_small(&pack.join("active.json"))?).map_err(err)?;
        let name = string(&active, "directory")?;
        safe_component(name)?;
        if !name.starts_with("runtime-")
            || active["format_version"] != 1
            || active["version"] != version
            || active["target"] != self.target
        {
            return Err("Active toolchain pointer does not match the trusted release.".into());
        }
        let root = pack.join(name);
        real_directory(&root)?;
        let receipt: Value =
            serde_json::from_slice(&read_small(&root.join(RECEIPT))?).map_err(err)?;
        if receipt["format_version"] != 1
            || receipt["id"] != id
            || receipt["version"] != version
            || receipt["target"] != self.target
        {
            return Err("Toolchain receipt does not match the trusted release.".into());
        }
        validate_removal(&root, spec)?;
        let stamps = runtime_stamps(&root, &pack.join("active.json"))?;
        let cached = self
            .verified
            .lock()
            .map_err(|_| "Toolchain validation cache failed.")?
            .get(id)
            .is_some_and(|(cached_root, cached_stamps)| {
                cached_root == &root && cached_stamps == &stamps
            });
        if !cached {
            validate_runtime(&root, spec, &receipt, cancel)?;
            if runtime_stamps(&root, &pack.join("active.json"))? != stamps {
                return Err("Toolchain runtime changed during verification.".into());
            }
            self.verified
                .lock()
                .map_err(|_| "Toolchain validation cache failed.")?
                .insert(id.into(), (root.clone(), stamps));
        }
        let executable = checked_join(&root, string(spec, "executable")?)?;
        let mut environment = Vec::new();
        if let Some(values) = spec["environment"].as_object() {
            for (key, value) in values {
                if !["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"].contains(&key.as_str()) {
                    return Err("Unsupported toolchain environment key.".into());
                }
                let path = checked_join(
                    &root,
                    value.as_str().ok_or("Invalid runtime environment path.")?,
                )?;
                real_directory(&path)?;
                environment.push((key.into(), path.into_os_string()));
            }
        }
        Ok(ResolvedToolchain {
            executable,
            root,
            version: version.into(),
            environment,
            _lease: None,
        })
    }
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("Missing manifest string: {key}"))
}
fn number(value: &Value, key: &str) -> Result<u64, String> {
    value[key]
        .as_u64()
        .ok_or_else(|| format!("Missing manifest limit: {key}"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("Missing manifest list: {key}"))
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Toolchain operation cancelled.".into())
    } else {
        Ok(())
    }
}

fn safe_component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 255
        || value == "."
        || value == ".."
        || value.ends_with(['.', ' '])
        || value
            .chars()
            .any(|c| c.is_control() || ['/', '\\', ':'].contains(&c))
    {
        return Err(format!("Unsafe toolchain path component: {value:?}"));
    }
    let stem = value.split('.').next().unwrap().to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("Reserved Windows path component.".into());
    }
    Ok(())
}
fn normalized_path(value: &str) -> Result<String, String> {
    if value.len() > 4096 {
        return Err("Toolchain path exceeds length limit.".into());
    }
    let value = value.strip_suffix('/').unwrap_or(value);
    for part in value.split('/') {
        safe_component(part)?;
    }
    Ok(value.into())
}
fn checked_join(root: &Path, path: &str) -> Result<PathBuf, String> {
    Ok(root.join(normalized_path(path)?))
}
fn path_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(err(error)),
    }
}
fn real_directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(err)?;
    if !meta.is_dir() || is_link(&meta) {
        return Err(format!("Not a real managed directory: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o022 != 0 {
            return Err("Managed directory is writable by other accounts.".into());
        }
    }
    Ok(())
}
fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn regular_file(path: &Path) -> Result<fs::Metadata, String> {
    let meta = fs::symlink_metadata(path).map_err(err)?;
    if !meta.is_file() || is_link(&meta) {
        return Err(format!("Not a regular managed file: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err("Managed file has hard links.".into());
        }
    }
    Ok(meta)
}
fn create_directory(path: &Path) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(err(e)),
    }
    real_directory(path)
}
fn read_small(path: &Path) -> Result<Vec<u8>, String> {
    if regular_file(path)?.len() > MAX_METADATA {
        return Err("Toolchain metadata exceeds size limit.".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(err)?
        .take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err("Toolchain metadata exceeds size limit.".into());
    }
    Ok(bytes)
}
fn new_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(err)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = new_file(path)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_all().map_err(err)
}
fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    atomic_json_with_directory_sync(path, value, |parent| {
        #[cfg(unix)]
        {
            File::open(parent).and_then(|file| file.sync_all())
        }
        #[cfg(not(unix))]
        {
            let _ = parent;
            Ok(())
        }
    })
}
fn atomic_json_with_directory_sync(
    path: &Path,
    value: &Value,
    sync_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), String> {
    if path_present(path)? {
        regular_file(path)?;
    }
    let parent = path.parent().ok_or("Missing pointer directory.")?;
    real_directory(parent)?;
    let mut file = NamedTempFile::new_in(parent).map_err(err)?;
    serde_json::to_writer(file.as_file_mut(), value).map_err(err)?;
    file.as_file().sync_all().map_err(err)?;
    file.persist(path).map_err(err)?;
    // Pointer replacement is the commit point. Directory fsync improves crash
    // durability but failure here cannot be reported as failed activation: the
    // new runtime is already active and must never be removed as rollback.
    let _ = sync_directory(parent);
    Ok(())
}

fn initialize_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("Toolchain root must be absolute.".into());
    }
    // Parents may be ordinary OS-selected paths. Never adopt a nonempty target.
    fs::create_dir_all(root.parent().ok_or("Toolchain root has no parent.")?).map_err(err)?;
    create_directory(root)?;
    let marker = root.join(MARKER);
    if path_present(&marker)? {
        if read_small(&marker)? != MARKER_CONTENT {
            return Err("Invalid CodeStyles toolchain marker.".into());
        }
    } else {
        if fs::read_dir(root).map_err(err)?.next().is_some() {
            return Err("Refusing to adopt an unmarked nonempty toolchain directory.".into());
        }
        write_new(&marker, MARKER_CONTENT)?;
    }
    let probe = NamedTempFile::new_in(root).map_err(err)?;
    probe.as_file().sync_all().map_err(err)?;
    Ok(())
}
fn choose_root(
    executable: &Path,
    explicit: Option<OsString>,
    transient: bool,
    fallback: Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    choose_root_with_temp(
        executable,
        explicit,
        transient,
        fallback,
        &std::env::temp_dir(),
    )
}
fn choose_root_with_temp(
    executable: &Path,
    explicit: Option<OsString>,
    transient: bool,
    fallback: Result<PathBuf, String>,
    temporary: &Path,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        let path = PathBuf::from(path);
        initialize_root(&path)?;
        return Ok(path);
    }
    if !transient && !temporary_launch(executable, temporary) {
        let path = installation_directory(executable)?.join("CodeStyles-tools");
        if initialize_root(&path).is_ok() {
            return Ok(path);
        }
    }
    let fallback = fallback?;
    initialize_root(&fallback)?;
    Ok(fallback)
}

fn installation_directory(executable: &Path) -> Result<&Path, String> {
    let parent = executable
        .parent()
        .ok_or("Executable has no parent directory.")?;
    // Preserve signed macOS bundles: put the managed sibling beside Foo.app,
    // never inside Foo.app/Contents/MacOS.
    if parent.file_name() == Some(OsStr::new("MacOS"))
        && let Some(contents) = parent.parent()
        && contents.file_name() == Some(OsStr::new("Contents"))
        && let Some(app) = contents.parent()
        && app.extension() == Some(OsStr::new("app"))
    {
        return app
            .parent()
            .ok_or("Application bundle has no parent directory.".into());
    }
    Ok(parent)
}
fn temporary_launch(executable: &Path, temporary: &Path) -> bool {
    executable.starts_with(temporary)
        || executable
            .components()
            .any(|part| part.as_os_str() == OsStr::new("AppTranslocation"))
}
fn user_data_root(os: &str, env: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    let base = match os {
        "windows" => env("LOCALAPPDATA").map(PathBuf::from),
        "macos" => env("HOME").map(|home| PathBuf::from(home).join("Library/Application Support")),
        _ => env("XDG_DATA_HOME")
            .filter(|p| Path::new(p).is_absolute())
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".local/share"))),
    }
    .ok_or("No OS user-data location is available for toolchains.")?;
    if !base.is_absolute() {
        return Err("OS user-data directory must be absolute.".into());
    }
    Ok(base.join("CodeStyles/toolchains"))
}

fn trusted_download_url(url: &str) -> Result<reqwest::Url, String> {
    let parsed = reqwest::Url::parse(url).map_err(err)?;
    let parts: Vec<_> = parsed.path().split('/').collect();
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parts.len() != 7
        || parts[3] != "releases"
        || parts[4] != "download"
        || parts.iter().skip(1).any(|part| part.is_empty())
    {
        return Err("Toolchain downloads require a pinned GitHub release asset URL.".into());
    }
    Ok(parsed)
}

async fn cancellable<F: std::future::Future>(
    future: F,
    cancel: &AtomicBool,
) -> Result<F::Output, String> {
    tokio::pin!(future);
    loop {
        check_cancel(cancel)?;
        tokio::select! {
            value = &mut future => return Ok(value),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
        }
    }
}

fn download(
    archive: &Value,
    output: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(InstallProgress),
) -> Result<(), String> {
    let url = trusted_download_url(string(archive, "url")?)?;
    let maximum = number(archive, "max_download_bytes")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(err)?;
    let result = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .timeout(DOWNLOAD_TIMEOUT)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("Too many toolchain redirects");
                }
                let url = attempt.url();
                if url.scheme() == "https"
                    && [
                        "github.com",
                        "release-assets.githubusercontent.com",
                        "objects.githubusercontent.com",
                    ]
                    .contains(&url.host_str().unwrap_or(""))
                {
                    attempt.follow()
                } else {
                    attempt.error("Toolchain redirect left GitHub asset hosts")
                }
            }))
            .build()
            .map_err(err)?;
        let mut response = cancellable(client.get(url).send(), cancel)
            .await?
            .map_err(err)?
            .error_for_status()
            .map_err(err)?;
        let total = response.content_length();
        if total.is_some_and(|bytes| bytes > maximum) {
            return Err("Toolchain download exceeds compressed size limit.".into());
        }
        let mut file = new_file(output)?;
        let mut bytes = 0u64;
        while let Some(chunk) = cancellable(response.chunk(), cancel).await?.map_err(err)? {
            bytes = bytes
                .checked_add(chunk.len() as u64)
                .ok_or("Download size overflow.")?;
            if bytes > maximum {
                return Err("Toolchain download exceeds compressed size limit.".into());
            }
            file.write_all(&chunk).map_err(err)?;
            progress(InstallProgress {
                phase: "downloading",
                downloaded_bytes: bytes,
                total_bytes: total,
            });
        }
        check_cancel(cancel)?;
        file.sync_all().map_err(err)
    });
    // Cancelled DNS resolution may still have an OS blocking resolver task;
    // never make closing the worker wait indefinitely for that task to finish.
    runtime.shutdown_timeout(Duration::from_millis(200));
    result
}

fn hash_file(path: &Path, maximum: u64, cancel: &AtomicBool) -> Result<(String, u64), String> {
    if regular_file(path)?.len() > maximum {
        return Err("Managed file exceeds expected size.".into());
    }
    let mut file = File::open(path).map_err(err)?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        check_cancel(cancel)?;
        let count = file.read(&mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > maximum {
            return Err("Managed file exceeds expected size.".into());
        }
        hash.update(&buffer[..count]);
    }
    Ok((format!("{:x}", hash.finalize()), bytes))
}
fn verify_archive(path: &Path, spec: &Value, cancel: &AtomicBool) -> Result<(), String> {
    let (hash, bytes) = hash_file(path, number(spec, "max_download_bytes")?, cancel)?;
    if hash != string(spec, "sha256")?
        || spec["bytes"]
            .as_u64()
            .is_some_and(|expected| expected != bytes)
    {
        return Err("Toolchain archive checksum or length mismatch.".into());
    }
    Ok(())
}
struct LimitedReader<'a, R> {
    inner: R,
    remaining: u64,
    cancel: &'a AtomicBool,
    started: Instant,
}
impl<R: Read> Read for LimitedReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(io::Error::other("Toolchain operation cancelled."));
        }
        if self.started.elapsed() > DOWNLOAD_TIMEOUT {
            return Err(io::Error::other("Toolchain extraction timed out."));
        }
        let count = self.inner.read(buffer)?;
        self.remaining = self
            .remaining
            .checked_sub(count as u64)
            .ok_or_else(|| io::Error::other("Toolchain expanded size limit exceeded."))?;
        Ok(count)
    }
}
fn expected_members(spec: &Value) -> Result<BTreeMap<String, Value>, String> {
    let mut files = BTreeMap::new();
    let mut folded = BTreeSet::new();
    for archive in array(spec, "archives")? {
        for member in array(archive, "members")? {
            let path = normalized_path(string(member, "path")?)?;
            if path == RECEIPT
                || !folded.insert(path.to_lowercase())
                || files.insert(path, member.clone()).is_some()
            {
                return Err("Duplicate or reserved runtime member in trusted manifest.".into());
            }
        }
    }
    if files.is_empty() {
        return Err("Toolchain has no pinned runtime files.".into());
    }
    Ok(files)
}
fn make_parents(root: &Path, relative: &str) -> Result<(), String> {
    let mut parent = root.to_owned();
    let parts: Vec<_> = relative.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        parent.push(part);
        create_directory(&parent)?;
    }
    Ok(())
}
fn extract_archive(
    archive: &Path,
    spec: &Value,
    root: &Path,
    cancel: &AtomicBool,
    installed: &mut BTreeMap<String, Value>,
    installed_bytes: &mut u64,
    maximum: u64,
) -> Result<(), String> {
    if string(spec, "format")? != "tar.gz" {
        return Err("Only pinned tar.gz toolchain packs are supported.".into());
    }
    let decoder = flate2::read::GzDecoder::new(File::open(archive).map_err(err)?);
    let limited = LimitedReader {
        inner: decoder,
        remaining: number(spec, "max_expanded_bytes")?,
        cancel,
        started: Instant::now(),
    };
    let mut tar = tar::Archive::new(limited);
    let mut allowed = BTreeMap::new();
    for member in array(spec, "members")? {
        let archive_path = normalized_path(string(member, "archive_path")?)?;
        if allowed.insert(archive_path, member).is_some() {
            return Err("Duplicate archive member in manifest.".into());
        }
    }
    let mut seen = BTreeSet::new();
    let mut found = BTreeSet::new();
    for entry in tar.entries().map_err(err)? {
        check_cancel(cancel)?;
        let mut entry = entry.map_err(err)?;
        let raw = entry.path_bytes();
        let path = normalized_path(std::str::from_utf8(&raw).map_err(err)?)?;
        if seen.len() >= 100_000 {
            return Err("Toolchain archive has too many entries.".into());
        }
        if !seen.insert(path.to_lowercase()) {
            return Err("Duplicate archive path.".into());
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err("Toolchain archives may not contain links or special files.".into());
        }
        if kind.is_dir() {
            if !allowed
                .keys()
                .any(|member| member.starts_with(&format!("{path}/")))
            {
                return Err("Toolchain archive contains an unrecognized directory.".into());
            }
            continue;
        }
        let member = allowed
            .get(&path)
            .ok_or("Toolchain archive contains an unrecognized file.")?;
        let relative = normalized_path(string(member, "path")?)?;
        if installed
            .keys()
            .any(|key| key.eq_ignore_ascii_case(&relative))
        {
            return Err("Duplicate installed runtime path.".into());
        }
        let size = entry.size();
        *installed_bytes = installed_bytes
            .checked_add(size)
            .ok_or("Installed size overflow.")?;
        if *installed_bytes > maximum || member["bytes"].as_u64().is_some_and(|bytes| bytes != size)
        {
            return Err("Toolchain member exceeds expected installed size.".into());
        }
        make_parents(root, &relative)?;
        let destination = checked_join(root, &relative)?;
        let mut output = new_file(&destination)?;
        let mut digest = Sha256::new();
        let mut written = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            check_cancel(cancel)?;
            let count = entry.read(&mut buffer).map_err(err)?;
            if count == 0 {
                break;
            }
            written += count as u64;
            if written > size {
                return Err("Archive member length mismatch.".into());
            }
            output.write_all(&buffer[..count]).map_err(err)?;
            digest.update(&buffer[..count]);
        }
        let hash = format!("{:x}", digest.finalize());
        if written != size
            || member["sha256"]
                .as_str()
                .is_some_and(|expected| expected != hash)
        {
            return Err("Toolchain member checksum mismatch.".into());
        }
        output.sync_all().map_err(err)?;
        let executable = member["executable"].as_bool().unwrap_or(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &destination,
                fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
            )
            .map_err(err)?;
        }
        installed.insert(
            relative,
            json!({"sha256":hash,"bytes":size,"executable":executable}),
        );
        found.insert(path);
    }
    // Also bound decompressed padding/trailing data, not just selected members.
    io::copy(&mut tar.into_inner(), &mut io::sink()).map_err(err)?;
    if found.len() != allowed.len() {
        return Err("Toolchain archive is missing a required runtime member.".into());
    }
    Ok(())
}

fn walk_files(root: &Path, path: &Path, files: &mut BTreeSet<String>) -> Result<(), String> {
    real_directory(path)?;
    for entry in fs::read_dir(path).map_err(err)? {
        let entry = entry.map_err(err)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(err)?;
        if is_link(&metadata) {
            return Err("Links are not allowed in managed runtimes.".into());
        }
        if metadata.is_dir() {
            walk_files(root, &entry.path(), files)?;
        } else {
            regular_file(&entry.path())?;
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(err)?
                .to_string_lossy()
                .replace('\\', "/");
            normalized_path(&relative)?;
            if !files.insert(relative) {
                return Err("Duplicate runtime file.".into());
            }
        }
    }
    Ok(())
}
fn validate_removal(root: &Path, spec: &Value) -> Result<(), String> {
    let mut files = BTreeSet::new();
    walk_files(root, root, &mut files)?;
    let mut expected: BTreeSet<_> = expected_members(spec)?.into_keys().collect();
    expected.insert(RECEIPT.into());
    if files != expected {
        return Err(
            "Managed runtime contains unexpected or missing files; refusing recursive removal."
                .into(),
        );
    }
    let receipt: Value = serde_json::from_slice(&read_small(&root.join(RECEIPT))?).map_err(err)?;
    if receipt["format_version"] != 1 {
        return Err("Runtime has no valid ownership receipt.".into());
    }
    Ok(())
}
fn validate_runtime(
    root: &Path,
    spec: &Value,
    receipt: &Value,
    cancel: &AtomicBool,
) -> Result<(), String> {
    validate_removal(root, spec)?;
    let expected = expected_members(spec)?;
    let records = receipt["files"]
        .as_object()
        .ok_or("Runtime receipt has no file hashes.")?;
    if records.len() != expected.len() {
        return Err("Runtime receipt file count mismatch.".into());
    }
    for (path, member) in expected {
        let record = records.get(&path).ok_or("Runtime receipt is incomplete.")?;
        let target = checked_join(root, &path)?;
        let (hash, bytes) = hash_file(&target, number(record, "bytes")?, cancel)?;
        if record["sha256"] != hash
            || record["bytes"] != bytes
            || member["sha256"].as_str().is_some_and(|value| value != hash)
            || member["bytes"].as_u64().is_some_and(|value| value != bytes)
            || record["executable"] != member["executable"]
        {
            return Err(format!("Runtime integrity check failed: {path}"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if member["executable"] == true
                && regular_file(&target)?.permissions().mode() & 0o111 == 0
            {
                return Err("Runtime executable permission is missing.".into());
            }
        }
    }
    let executable = normalized_path(string(spec, "executable")?)?;
    if !records
        .get(&executable)
        .is_some_and(|record| record["executable"] == true)
    {
        return Err("Runtime executable is not allowlisted.".into());
    }
    Ok(())
}
fn remove_managed_runtime(root: &Path, spec: &Value) -> Result<(), String> {
    validate_removal(root, spec)?;
    // Scope has been checked against the built-in member list, and no links are
    // allowed. Rust's remove_dir_all does not follow symbolic links.
    fs::remove_dir_all(root).map_err(err)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    bytes: u64,
    modified: Option<std::time::SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    unix: (u64, u64, i64, i64, u32),
}
fn runtime_stamps(root: &Path, active: &Path) -> Result<BTreeMap<PathBuf, FileStamp>, String> {
    let mut files = BTreeSet::new();
    walk_files(root, root, &mut files)?;
    let paths = files
        .into_iter()
        .map(|path| root.join(path))
        .chain(std::iter::once(active.to_owned()));
    paths
        .map(|path| {
            let meta = regular_file(&path)?;
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            let stamp = FileStamp {
                bytes: meta.len(),
                modified: meta.modified().ok(),
                readonly: meta.permissions().readonly(),
                #[cfg(unix)]
                unix: (
                    meta.dev(),
                    meta.ino(),
                    meta.ctime(),
                    meta.ctime_nsec(),
                    meta.mode(),
                ),
            };
            Ok((path, stamp))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn archive(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, kind, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::new(*kind));
            // Deliberately raw names allow negative fixtures that the builder's
            // safe path convenience API correctly refuses to construct.
            header.as_mut_bytes()[..100].fill(0);
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            if *kind == b'1' || *kind == b'2' {
                header.set_link_name("outside").unwrap();
            }
            header.set_cksum();
            tar.append(&header, *bytes).unwrap();
        }
        let raw = tar.into_inner().unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&raw).unwrap();
        gzip.finish().unwrap()
    }
    fn fixture(bytes: &[u8]) -> (tempfile::TempDir, ToolchainManager) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("managed");
        initialize_root(&root).unwrap();
        let member = b"verified formatter\n";
        let catalog = json!({"format_version":1,"toolchains":{"rustfmt":{"version":"test-1", "targets":{"test":{
            "available":false,"reason":"Unpublished test bundle", "executable":"bin/rustfmt", "environment":{}, "max_installed_bytes":1024,
            "archives":[{"format":"tar.gz","sha256":format!("{:x}",Sha256::digest(bytes)),"bytes":bytes.len(),"max_download_bytes":4096,"max_expanded_bytes":65536,
                "members":[{"archive_path":"bin/rustfmt","path":"bin/rustfmt","sha256":format!("{:x}",Sha256::digest(member)),"bytes":member.len(),"executable":true}]}]
        }}}}});
        (
            dir,
            ToolchainManager {
                root,
                target: "test".into(),
                catalog,
                verified: Default::default(),
            },
        )
    }
    fn install(manager: &ToolchainManager, bytes: &[u8]) -> Result<ResolvedToolchain, String> {
        manager.install_with(
            "rustfmt",
            &AtomicBool::new(false),
            &|_| {},
            |_, path, _, _| write_new(path, bytes),
        )
    }
    fn good_archive() -> Vec<u8> {
        archive(&[("bin/rustfmt", b'0', b"verified formatter\n")])
    }

    #[test]
    fn local_install_resolve_cache_and_uninstall_are_owned_and_verified() {
        let bytes = good_archive();
        let (_dir, manager) = fixture(&bytes);
        assert_eq!(manager.status("rustfmt").unwrap(), ToolchainStatus::Missing);
        let installed = install(&manager, &bytes).unwrap();
        let path = installed.executable.clone();
        assert_eq!(fs::read(&path).unwrap(), b"verified formatter\n");
        drop(installed);
        assert!(matches!(
            manager.status("rustfmt").unwrap(),
            ToolchainStatus::Installed { .. }
        ));
        drop(manager.resolve("rustfmt").unwrap());
        fs::write(&path, b"untrusted formatter").unwrap();
        assert!(matches!(
            manager.status("rustfmt").unwrap(),
            ToolchainStatus::Corrupt { .. }
        ));
        assert!(manager.resolve("rustfmt").is_err());
        manager.uninstall("rustfmt").unwrap();
        assert_eq!(manager.status("rustfmt").unwrap(), ToolchainStatus::Missing);
        assert!(manager.root.join(MARKER).exists());
    }

    #[test]
    fn checksum_failure_cancellation_and_bad_unpack_preserve_active() {
        let bytes = good_archive();
        let (_dir, mut manager) = fixture(&bytes);
        let original = install(&manager, &bytes).unwrap().root;
        let pointer = fs::read(manager.root.join("rustfmt/active.json")).unwrap();
        assert!(
            install(&manager, b"corrupt download")
                .unwrap_err()
                .contains("checksum")
        );
        let cancel = AtomicBool::new(false);
        let result = manager.install_with("rustfmt", &cancel, &|_| {}, |_, path, _, _| {
            write_new(path, &bytes)?;
            cancel.store(true, Ordering::Release);
            Ok(())
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert_eq!(
            fs::read(manager.root.join("rustfmt/active.json")).unwrap(),
            pointer
        );
        assert!(original.exists());
        let evil = archive(&[("../escape", b'0', b"oops")]);
        manager.catalog["toolchains"]["rustfmt"]["targets"]["test"]["archives"][0]["sha256"] =
            json!(format!("{:x}", Sha256::digest(&evil)));
        manager.catalog["toolchains"]["rustfmt"]["targets"]["test"]["archives"][0]["bytes"] =
            json!(evil.len());
        assert!(install(&manager, &evil).is_err());
        assert_eq!(
            fs::read(manager.root.join("rustfmt/active.json")).unwrap(),
            pointer
        );
        assert!(!manager.root.join("escape").exists());
        assert!(
            !fs::read_dir(manager.root.join("rustfmt"))
                .unwrap()
                .any(|e| e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".staging-"))
        );
    }

    #[test]
    fn unsafe_archive_paths_links_devices_and_duplicates_fail_closed() {
        for entries in [
            vec![("../escape", b'0', b"bad".as_slice())],
            vec![("/absolute", b'0', b"bad".as_slice())],
            vec![("C:/escape", b'0', b"bad".as_slice())],
            vec![("a\\escape", b'0', b"bad".as_slice())],
            vec![("bin/link", b'1', b"".as_slice())],
            vec![("bin/link", b'2', b"".as_slice())],
            vec![("bin/device", b'3', b"".as_slice())],
            vec![("bin/pipe", b'6', b"".as_slice())],
            vec![
                ("bin/rustfmt", b'0', b"verified formatter\n".as_slice()),
                ("bin/rustfmt", b'0', b"verified formatter\n".as_slice()),
            ],
            vec![
                ("BIN/rustfmt", b'0', b"verified formatter\n".as_slice()),
                ("bin/rustfmt", b'0', b"verified formatter\n".as_slice()),
            ],
        ] {
            let bytes = archive(&entries);
            let (_dir, manager) = fixture(&bytes);
            assert!(install(&manager, &bytes).is_err(), "{entries:?}");
            assert_eq!(manager.status("rustfmt").unwrap(), ToolchainStatus::Missing);
        }
    }

    #[test]
    fn compressed_expanded_and_installed_limits_and_missing_members_are_enforced() {
        let bytes = good_archive();
        for kind in ["compressed", "expanded", "installed", "missing", "digest"] {
            let (_dir, mut manager) = fixture(&bytes);
            let spec = &mut manager.catalog["toolchains"]["rustfmt"]["targets"]["test"];
            match kind {
                "compressed" => spec["archives"][0]["max_download_bytes"] = json!(1),
                "expanded" => spec["archives"][0]["max_expanded_bytes"] = json!(1),
                "installed" => spec["max_installed_bytes"] = json!(1),
                "missing" => spec["archives"][0]["members"][0]["archive_path"] = json!("missing"),
                _ => spec["archives"][0]["members"][0]["sha256"] = json!("0".repeat(64)),
            }
            assert!(install(&manager, &bytes).is_err(), "{kind}");
            assert_eq!(manager.status("rustfmt").unwrap(), ToolchainStatus::Missing);
        }
        let payload = vec![0; 100_000];
        let bomb = archive(&[("bin/rustfmt", b'0', &payload)]);
        let (_dir, mut manager) = fixture(&bomb);
        let spec = &mut manager.catalog["toolchains"]["rustfmt"]["targets"]["test"];
        spec["max_installed_bytes"] = json!(200_000);
        spec["archives"][0]["members"][0]["bytes"] = json!(payload.len());
        spec["archives"][0]["members"][0]["sha256"] =
            json!(format!("{:x}", Sha256::digest(&payload)));
        assert!(install(&manager, &bomb).unwrap_err().contains("expanded"));
    }

    #[test]
    fn untrusted_locations_ids_receipts_and_additional_files_are_never_deleted() {
        let bytes = good_archive();
        let (dir, manager) = fixture(&bytes);
        let foreign = dir.path().join("foreign");
        fs::create_dir(&foreign).unwrap();
        fs::write(foreign.join("important"), "keep").unwrap();
        assert!(initialize_root(&foreign).is_err());
        for id in ["..", "../foreign", "rustfmt/../../foreign", "unknown"] {
            assert!(manager.uninstall(id).is_err());
        }
        let runtime = install(&manager, &bytes).unwrap().root;
        fs::write(runtime.join("foreign"), "keep").unwrap();
        assert!(manager.uninstall("rustfmt").is_err());
        assert_eq!(fs::read_to_string(runtime.join("foreign")).unwrap(), "keep");
        assert_eq!(
            fs::read_to_string(foreign.join("important")).unwrap(),
            "keep"
        );
        assert!(manager.resolve("rustfmt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hardlink_roots_pointers_and_runtime_files_are_rejected() {
        use std::os::unix::fs::symlink;
        let bytes = good_archive();
        let (dir, manager) = fixture(&bytes);
        let link = dir.path().join("link");
        symlink(&manager.root, &link).unwrap();
        assert!(initialize_root(&link).is_err());
        let runtime = install(&manager, &bytes).unwrap();
        let executable = runtime.executable.clone();
        drop(runtime);
        let outside = dir.path().join("outside");
        fs::write(&outside, "keep").unwrap();
        fs::remove_file(&executable).unwrap();
        symlink(&outside, &executable).unwrap();
        assert!(manager.resolve("rustfmt").is_err());
        assert!(manager.uninstall("rustfmt").is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
        fs::remove_file(&executable).unwrap();
        fs::hard_link(&outside, &executable).unwrap();
        assert!(manager.resolve("rustfmt").is_err());
        assert!(manager.uninstall("rustfmt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn managed_directories_are_private_and_existing_shared_write_roots_are_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let bytes = good_archive();
        let (_dir, manager) = fixture(&bytes);
        assert_eq!(
            fs::metadata(&manager.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        drop(install(&manager, &bytes).unwrap());
        assert_eq!(
            fs::metadata(manager.root.join("rustfmt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::set_permissions(&manager.root, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(
            initialize_root(&manager.root)
                .unwrap_err()
                .contains("other accounts")
        );
        assert!(manager.resolve("rustfmt").is_err());
        fs::set_permissions(&manager.root, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_lock_symlink_cannot_create_a_file_outside_managed_root() {
        use std::os::unix::fs::symlink;
        let bytes = good_archive();
        let (dir, manager) = fixture(&bytes);
        let pack = manager.root.join("rustfmt");
        create_directory(&pack).unwrap();
        let outside = dir.path().join("must-not-create");
        symlink(&outside, pack.join(".lock")).unwrap();
        assert!(install(&manager, &bytes).is_err());
        assert!(!outside.exists());
    }

    #[test]
    fn published_url_gate_and_local_digest_gate_reject_untrusted_inputs() {
        let bytes = good_archive();
        let (dir, manager) = fixture(&bytes);
        assert!(
            manager
                .download_available("rustfmt")
                .unwrap_err()
                .contains("Unpublished")
        );
        assert!(
            manager
                .install("rustfmt", Arc::new(AtomicBool::new(false)), |_| {})
                .is_err()
        );
        let source = dir.path().join("pack.tar.gz");
        fs::write(&source, &bytes).unwrap();
        drop(
            manager
                .install_verified_archive(
                    "rustfmt",
                    &source,
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .unwrap(),
        );
        fs::write(&source, "not the pinned archive").unwrap();
        assert!(
            manager
                .install_verified_archive(
                    "rustfmt",
                    &source,
                    Arc::new(AtomicBool::new(false)),
                    |_| {}
                )
                .is_err()
        );
        for url in [
            "http://github.com/a/b/releases/download/v/p.tar.gz",
            "https://evil.example/a/b/releases/download/v/p.tar.gz",
            "https://github.com/a/b/blob/v/p.tar.gz",
            "https://github.com/a/b/releases/download/v/p.tar.gz?other=1",
            "file:///tmp/a",
        ] {
            assert!(trusted_download_url(url).is_err());
        }
        assert!(
            trusted_download_url("https://github.com/a/b/releases/download/v/p.tar.gz").is_ok()
        );
    }

    #[test]
    fn root_selection_handles_explicit_fallback_transient_and_signed_macos_bundles() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("explicit");
        assert_eq!(
            choose_root(
                Path::new("/nonexistent/app"),
                Some(explicit.clone().into_os_string()),
                false,
                Err("no home".into())
            )
            .unwrap(),
            explicit
        );
        let fallback = dir.path().join("fallback");
        assert_eq!(
            choose_root(
                Path::new("/nonexistent/app"),
                None,
                true,
                Ok(fallback.clone())
            )
            .unwrap(),
            fallback
        );
        assert_eq!(
            installation_directory(Path::new(
                "/Applications/CodeStyles.app/Contents/MacOS/codestyles"
            ))
            .unwrap(),
            Path::new("/Applications")
        );
        assert!(temporary_launch(
            Path::new("/tmp/unpacked/codestyles"),
            Path::new("/tmp")
        ));
        assert!(!temporary_launch(
            Path::new("/tmp-other/codestyles"),
            Path::new("/tmp")
        ));
        assert!(temporary_launch(
            Path::new("/private/var/folders/x/AppTranslocation/uuid/App.app/Contents/MacOS/app"),
            Path::new("/tmp")
        ));
        // Use a native absolute path even when testing Linux's environment
        // precedence on a Windows runner ("/home/user" is drive-relative there).
        let home = dir.path().join("user-home");
        assert!(home.is_absolute());
        assert_eq!(
            user_data_root("linux", |key| match key {
                "XDG_DATA_HOME" => Some("relative".into()),
                "HOME" => Some(home.clone().into_os_string()),
                _ => None,
            })
            .unwrap(),
            home.join(".local/share/CodeStyles/toolchains")
        );
    }

    #[test]
    fn portable_root_probe_prefers_sibling_and_falls_back_without_touching_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("Example.app/Contents/MacOS/codestyles");
        let fake_temp = dir.path().join("different-temp");
        let fallback = dir.path().join("user-data");
        let chosen =
            choose_root_with_temp(&executable, None, false, Ok(fallback.clone()), &fake_temp)
                .unwrap();
        assert_eq!(chosen, dir.path().join("CodeStyles-tools"));
        assert!(!dir.path().join("Example.app").exists());
        // A foreign/unwritable sibling is not adopted or recursively altered.
        fs::remove_file(chosen.join(MARKER)).unwrap();
        fs::write(chosen.join("foreign"), "keep").unwrap();
        let chosen_again =
            choose_root_with_temp(&executable, None, false, Ok(fallback.clone()), &fake_temp)
                .unwrap();
        assert_eq!(chosen_again, fallback);
        assert_eq!(fs::read_to_string(chosen.join("foreign")).unwrap(), "keep");
    }

    #[test]
    fn lock_contention_matches_native_code_not_error_kind() {
        let native = fs2::lock_contended_error();
        assert!(lock_is_contended(&native));
        assert!(!lock_is_contended(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!lock_is_contended(&io::Error::other(
            "unrelated lock failure"
        )));
        assert!(!lock_is_contended(&io::Error::from_raw_os_error(0)));
        #[cfg(windows)]
        assert_eq!(native.raw_os_error(), Some(33)); // ERROR_LOCK_VIOLATION
    }

    #[test]
    fn lock_child_probe() {
        let Some(path) = std::env::var_os("CODESTYLES_TEST_LOCK_PATH") else {
            return;
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let result = FileExt::try_lock_exclusive(&file);
        if std::env::var_os("CODESTYLES_TEST_LOCK_AVAILABLE").is_some() {
            assert!(result.is_ok());
        } else {
            assert!(lock_is_contended(&result.unwrap_err()));
        }
    }

    #[test]
    fn runtime_lease_is_a_real_cross_process_lock() {
        let bytes = good_archive();
        let (_dir, manager) = fixture(&bytes);
        let lease = install(&manager, &bytes).unwrap();
        let run = |available: bool| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "toolchains::tests::lock_child_probe"])
                .env(
                    "CODESTYLES_TEST_LOCK_PATH",
                    manager.root.join("rustfmt/.lock"),
                )
                .env_remove("CODESTYLES_TEST_LOCK_AVAILABLE");
            if available {
                command.env("CODESTYLES_TEST_LOCK_AVAILABLE", "1");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        run(false);
        drop(lease);
        run(true);
    }

    #[test]
    fn shared_runtime_lease_blocks_mutation_and_waiting_lock_can_cancel() {
        let bytes = good_archive();
        let (_dir, manager) = fixture(&bytes);
        let runtime = install(&manager, &bytes).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let other = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            other.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(
            manager
                .lock("rustfmt", &cancel, true)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(runtime);
        thread.join().unwrap();
        assert!(
            manager
                .lock("rustfmt", &AtomicBool::new(false), true)
                .is_ok()
        );
    }

    #[test]
    fn resolve_waiting_for_install_lock_and_digest_verification_are_cancellable() {
        let bytes = good_archive();
        let (dir, manager) = fixture(&bytes);
        drop(install(&manager, &bytes).unwrap());
        let exclusive = manager
            .lock("rustfmt", &AtomicBool::new(false), true)
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let other = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            other.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(
            manager
                .resolve_cancellable("rustfmt", &cancel)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(exclusive);
        thread.join().unwrap();
        let file = dir.path().join("digest");
        fs::write(&file, b"hash me").unwrap();
        assert!(
            hash_file(&file, 10, &cancel)
                .unwrap_err()
                .contains("cancelled")
        );
    }

    #[test]
    fn post_commit_directory_sync_failure_never_reports_uncommitted_activation() {
        let dir = tempfile::tempdir().unwrap();
        let pointer = dir.path().join("active.json");
        fs::write(&pointer, b"old pointer").unwrap();
        atomic_json_with_directory_sync(&pointer, &json!({"directory":"runtime-new"}), |_| {
            Err(io::Error::other("injected sync error"))
        })
        .unwrap();
        let active: Value = serde_json::from_slice(&fs::read(pointer).unwrap()).unwrap();
        assert_eq!(active["directory"], "runtime-new");
    }

    #[test]
    fn cancelled_uninstall_never_unpublishes_a_runtime() {
        let bytes = good_archive();
        let (_dir, manager) = fixture(&bytes);
        let lease = install(&manager, &bytes).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let other = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            other.store(true, Ordering::Release);
        });
        assert!(
            manager
                .uninstall_cancellable("rustfmt", &cancel)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(lease.executable.exists());
        assert!(manager.root.join("rustfmt/active.json").exists());
        thread.join().unwrap();
    }

    #[test]
    fn cancellation_interrupts_pending_async_download_work() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let other = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            other.store(true, Ordering::Release);
        });
        let started = Instant::now();
        let result = runtime.block_on(cancellable(std::future::pending::<()>(), &cancel));
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(1));
        thread.join().unwrap();
    }

    #[test]
    fn expanded_reader_bounds_trailing_bytes_and_observes_cancel() {
        let cancel = AtomicBool::new(false);
        let mut reader = LimitedReader {
            inner: Cursor::new(vec![0; 100]),
            remaining: 10,
            cancel: &cancel,
            started: Instant::now(),
        };
        assert!(io::copy(&mut reader, &mut io::sink()).is_err());
        cancel.store(true, Ordering::Release);
        let mut reader = LimitedReader {
            inner: Cursor::new(vec![0; 1]),
            remaining: 10,
            cancel: &cancel,
            started: Instant::now(),
        };
        assert!(reader.read(&mut [0]).is_err());
    }
}
