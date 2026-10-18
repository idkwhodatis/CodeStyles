//! Saved-config project operations using the pinned dprint CLI's native discovery.
//!
//! Check/preparation never call `fmt`. Writes require a separate explicit action.
//! No temporary config, plugin override, or config-discovery override is used here.
//! CLI contract verified against dprint 0.63.0 help and tagged source:
//! `arg_parser.rs`, `commands/{formatting,config}.rs`, `utils/glob/glob_utils.rs`.
use crate::{
    backend::{Backend, cache_directory},
    config::{ConfigDocument, ConfigSnapshot},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const PINNED_CLI_VERSION: &str = "dprint 0.63.0";
const CONFIG_NAMES: [&str; 4] = [
    "dprint.json",
    "dprint.jsonc",
    ".dprint.json",
    ".dprint.jsonc",
];
const OUTPUT_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectAction {
    Check,
    PreparePlugins,
    FormatSelected(Vec<PathBuf>),
    FormatProject,
}
impl ProjectAction {
    pub fn writes_sources(&self) -> bool {
        matches!(self, Self::FormatSelected(_) | Self::FormatProject)
    }
}

/// An immutable, explicitly authorized request. Private fields prevent callers
/// bypassing dirty/trust/disk-baseline checks by constructing it directly.
#[derive(Clone, Debug)]
pub struct ProjectRequest {
    root: PathBuf,
    config: ConfigSnapshot,
    action: ProjectAction,
}
impl ProjectRequest {
    /// `trusted` means the user approved this run's saved config, inherited and
    /// nested configs, their plugins (including process plugins), and downloads.
    /// The UI must additionally reject invalid/pending raw-editor changes.
    pub fn new(
        root: PathBuf,
        document: &ConfigDocument,
        action: ProjectAction,
        trusted: bool,
    ) -> Result<Self, String> {
        if !trusted {
            return Err("Trust the project's saved configuration and plugins before running a project action.".into());
        }
        if document.is_dirty() {
            return Err(
                "Save or discard configuration changes before running a project action.".into(),
            );
        }
        let path = document
            .path()
            .ok_or("Save the configuration before running a project action.")?;
        let root = fs::canonicalize(&root)
            .map_err(|e| format!("Cannot open project {}: {e}", root.display()))?;
        if !root.is_dir() || path.parent() != Some(root.as_path()) {
            return Err(
                "The project root must be the saved configuration's parent directory.".into(),
            );
        }
        if root.to_str().is_none_or(|s| s.contains(['\n', '\r'])) {
            return Err("Project paths must be valid UTF-8 without newline characters.".into());
        }
        let request = Self {
            root,
            config: document.saved_snapshot(),
            action,
        };
        request.verify_saved_config()?;
        if let ProjectAction::FormatSelected(paths) = &request.action {
            if paths.is_empty() {
                return Err("Select at least one source file to format.".into());
            }
            for path in paths {
                request.safe_file(path)?;
            }
        }
        Ok(request)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn action(&self) -> &ProjectAction {
        &self.action
    }
    pub fn config_path(&self) -> &Path {
        self.config.original_path().expect("saved request")
    }

    fn verify_saved_config(&self) -> Result<(), String> {
        let bytes = fs::read(self.config_path())
            .map_err(|e| format!("Cannot read saved configuration: {e}"))?;
        if bytes != self.config.source_text().as_bytes() {
            return Err("The saved configuration changed on disk. Reload it and review it before running again.".into());
        }
        if fs::canonicalize(self.config_path()).ok().as_deref() != Some(self.config_path()) {
            return Err(
                "The saved configuration's path changed. Reload it before running again.".into(),
            );
        }
        Ok(())
    }

    fn safe_file(&self, path: &Path) -> Result<PathBuf, String> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            self.root.join(path)
        };
        let canonical = fs::canonicalize(&path)
            .map_err(|e| format!("Cannot resolve source {}: {e}", path.display()))?;
        if !canonical.starts_with(&self.root) || !canonical.is_file() {
            return Err(format!(
                "Source must be a regular file inside the project: {}",
                path.display()
            ));
        }
        literal_pattern(&self.root, &canonical)?;
        Ok(canonical)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFile {
    pub path: PathBuf,
    /// The exact unified diff supplied by `dprint check --json`.
    pub diff: String,
}
#[derive(Clone, Debug)]
pub struct ProjectReport {
    pub action: ProjectAction,
    pub cli_version: String,
    /// Check: differing files. Format: diffs captured immediately before writing.
    pub files: Vec<ProjectFile>,
    pub diagnostics: String,
    pub summary: String,
    pub checked_files: usize,
    pub protected_configs: usize,
}

impl Backend {
    pub fn execute_project(
        &self,
        request: &ProjectRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ProjectReport, String> {
        let result = self.execute_project_inner(request, &cancelled);
        result.map_err(|error| {
            if request.action.writes_sources() {
                format!("{error}\nFormatting is not transactional; files may already have been changed. Run Check project to inspect the current state.")
            } else { error }
        })
    }

    fn execute_project_inner(
        &self,
        request: &ProjectRequest,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<ProjectReport, String> {
        request.verify_saved_config()?;
        let version = self.project_process(request, &["--version"], None, cancelled)?;
        ensure_success(&version, "Version check")?;
        let version =
            String::from_utf8(version.stdout).map_err(|_| "dprint version is not UTF-8.")?;
        if version.trim() != PINNED_CLI_VERSION {
            return Err(format!(
                "Unsupported CLI version {:?}; expected {PINNED_CLI_VERSION}.",
                version.trim()
            ));
        }
        let mut diagnostics = String::new();
        let mut report = ProjectReport {
            action: request.action.clone(),
            cli_version: version.trim().into(),
            files: Vec::new(),
            diagnostics: String::new(),
            summary: String::new(),
            checked_files: 0,
            protected_configs: 0,
        };
        if request.action == ProjectAction::PreparePlugins {
            // No positional plugins: unlike `install URL`, this never edits the
            // configuration. --recursive prepares the ordinarily discovered scopes.
            let prepared =
                self.project_process(request, &["install", "--recursive"], None, cancelled)?;
            ensure_success(&prepared, "Plugin preparation")?;
            append_diagnostics(&mut diagnostics, &prepared.stderr);
            let status =
                String::from_utf8(prepared.stdout).map_err(|_| "Plugin status is not UTF-8.")?;
            request.verify_saved_config()?;
            report.summary = if status.trim().is_empty() {
                "Plugin preparation completed.".into()
            } else {
                status.trim().into()
            };
            report.diagnostics = diagnostics;
            return Ok(report);
        }

        // Native enumeration retains includes/excludes/.gitignore and nested
        // discovery. Then bound the write/check scope to regular in-root files.
        let enumerated = self.project_process(request, &["file-paths"], None, cancelled)?;
        ensure_success(&enumerated, "Project file discovery")?;
        append_diagnostics(&mut diagnostics, &enumerated.stderr);
        let all_files = parse_file_paths(request, &enumerated.stdout)?;
        let configs = protected_configs(request, &all_files)?;
        // Canonical paths resolve symlinks, not hard links. Compare file identity
        // too so a source-looking hard link cannot rewrite a protected config.
        let handles = configs
            .keys()
            .map(same_file::Handle::from_path)
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(|error| format!("Cannot identify protected configuration files: {error}"))?;
        let mut protected: BTreeSet<PathBuf> = configs.keys().cloned().collect();
        for file in &all_files {
            let handle = same_file::Handle::from_path(file)
                .map_err(|error| format!("Cannot identify source {}: {error}", file.display()))?;
            if handles.contains(&handle) {
                protected.insert(file.clone());
            }
        }
        report.protected_configs = protected.len();
        let mut targets: BTreeSet<PathBuf> = all_files.difference(&protected).cloned().collect();
        if let ProjectAction::FormatSelected(selected) = &request.action {
            let mut exact = BTreeSet::new();
            for path in selected {
                let path = request.safe_file(path)?;
                if protected.contains(&path) {
                    return Err(format!(
                        "Configuration files are protected from project formatting: {}",
                        path.display()
                    ));
                }
                if !targets.contains(&path) {
                    return Err(format!(
                        "The saved project configuration does not include {}",
                        path.display()
                    ));
                }
                exact.insert(path);
            }
            targets = exact;
        }
        report.checked_files = targets.len();
        if targets.is_empty() {
            report.summary = "No included source files. Configuration files are protected.".into();
            report.diagnostics = diagnostics;
            return Ok(report);
        }
        let input = targets
            .iter()
            .map(|path| literal_pattern(&request.root, path))
            .collect::<Result<Vec<_>, _>>()?
            .join("\n")
            + "\n";
        // CLI patterns may prioritize an existing literal name over a pattern.
        // Verify our escaped inputs resolve to exactly the requested files before
        // any write; this rejects ambiguous escaped-name collisions as well.
        let resolved = self.project_process(
            request,
            &["file-paths", "--stdin-files"],
            Some(input.clone()),
            cancelled,
        )?;
        ensure_success(&resolved, "Selected-file validation")?;
        append_diagnostics(&mut diagnostics, &resolved.stderr);
        if parse_file_paths(request, &resolved.stdout)? != targets {
            return Err("Cannot represent these file names unambiguously in dprint's CLI. No formatting was started.".into());
        }
        verify_configs(&configs)?;
        let checked = self.project_process(
            request,
            &["check", "--json", "--incremental=false", "--stdin-files"],
            Some(input.clone()),
            cancelled,
        )?;
        if !matches!(checked.status.code(), Some(0 | 20)) {
            ensure_success(&checked, "Project check")?;
        }
        append_diagnostics(&mut diagnostics, &checked.stderr);
        report.files = parse_check_json(request, &checked.stdout, &targets)?;
        if (checked.status.code() == Some(20)) == report.files.is_empty() {
            return Err("dprint returned an inconsistent check result (exit status and JSON differences disagree).".into());
        }
        verify_configs(&configs)?;
        if request.action.writes_sources() {
            request.verify_saved_config()?;
            // Recheck file identity/containment immediately before starting writes.
            for path in &targets {
                if request.safe_file(path)? != *path {
                    return Err("A source path changed during project validation. Check again before formatting.".into());
                }
            }
            let formatted = self.project_process(
                request,
                &["fmt", "--incremental=false", "--stdin-files"],
                Some(input),
                cancelled,
            )?;
            ensure_success(&formatted, "Project formatting")?;
            append_diagnostics(&mut diagnostics, &formatted.stderr);
            append_diagnostics(&mut diagnostics, &formatted.stdout);
            verify_configs(&configs)?;
            report.summary = format!(
                "Formatting completed for {} selected source files; {} had differences. Configuration and local plugin files were protected. Run Check project to refresh.",
                targets.len(),
                report.files.len()
            );
        } else {
            report.summary = if report.files.is_empty() {
                format!(
                    "All {} included source files are formatted. Configuration and local plugin files were protected.",
                    targets.len()
                )
            } else {
                format!(
                    "{} of {} included source files need formatting. No source files were changed.",
                    report.files.len(),
                    targets.len()
                )
            };
        }
        request.verify_saved_config()?;
        report.diagnostics = diagnostics;
        Ok(report)
    }

    fn project_process(
        &self,
        request: &ProjectRequest,
        args: &[&str],
        input: Option<String>,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<ProcessOutput, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("Project operation cancelled.".into());
        }
        request.verify_saved_config()?;
        let cache = cache_directory(std::env::consts::OS, |key| std::env::var_os(key))?;
        fs::create_dir_all(&cache)
            .map_err(|e| format!("Cannot create dprint cache {}: {e}", cache.display()))?;
        let mut command = Command::new(self.executable());
        command
            .current_dir(&request.root)
            .env("DPRINT_CACHE_DIR", cache)
            .env("NO_COLOR", "1")
            .env_remove("FORCE_COLOR")
            // A caller's preview environment must not disable native project discovery.
            .env_remove("DPRINT_CONFIG_DISCOVERY")
            .args(args);
        if args != ["--version"] {
            let mut config_arg = OsString::from("--config=");
            config_arg.push(request.config_path());
            command.arg(config_arg);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
        }
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if cancelled.load(Ordering::Acquire) {
            return Err("Project operation cancelled.".into());
        }
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Cannot start bundled dprint {}: {e}",
                self.executable().display()
            )
        })?;
        #[cfg(windows)]
        let process_tree = WindowsProcessTree::attach_and_resume(&mut child, cancelled)?;
        let writer = input.map(|input| {
            let mut stdin = child.stdin.take().expect("piped stdin");
            thread::spawn(move || stdin.write_all(input.as_bytes()))
        });
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let reader = thread::spawn(move || read_bounded(stdout));
        let errors = thread::spawn(move || read_bounded(stderr));
        let started = Instant::now();
        let status = loop {
            if cancelled.load(Ordering::Acquire) {
                break Err("Project operation cancelled.".to_owned());
            }
            if started.elapsed() >= self.timeout() {
                break Err(format!(
                    "Project operation timed out after {} seconds.",
                    self.timeout().as_secs_f64()
                ));
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(e) => break Err(format!("Cannot wait for dprint: {e}")),
            }
        };
        // Stop plugin descendants before joining pipes, including a plugin that
        // kept inherited stdout open after dprint itself exited. Reap dprint too.
        #[cfg(windows)]
        process_tree.terminate();
        terminate_process_tree(&mut child);
        let _ = child.wait();
        let written = writer.map(|writer| writer.join());
        let output = reader.join().map_err(|_| "dprint output reader failed.")?;
        let errors = errors
            .join()
            .map_err(|_| "dprint diagnostic reader failed.")?;
        let status = status?;
        let stdout = output?;
        let stderr = errors?;
        if status.success()
            && let Some(written) = written
        {
            written
                .map_err(|_| "dprint input writer failed.")?
                .map_err(|e| format!("Cannot send source file list: {e}"))?;
        }
        if cancelled.load(Ordering::Acquire) {
            return Err("Project operation cancelled.".into());
        }
        Ok(ProcessOutput {
            status,
            stdout,
            stderr,
        })
    }
}

struct ProcessOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn ensure_success(output: &ProcessOutput, step: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "{step} failed ({}). {}{}",
        output.status,
        diagnostics.trim(),
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!("\n{}", stdout.trim())
        }
    ))
}
fn append_diagnostics(target: &mut String, bytes: &[u8]) {
    let message = String::from_utf8_lossy(bytes);
    if !message.trim().is_empty() {
        if !target.is_empty() {
            target.push('\n');
        }
        target.push_str(message.trim());
    }
}
fn read_bounded(mut stream: impl Read) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut buffer = [0; 8192];
    let mut overflow = false;
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|e| format!("Cannot read dprint output: {e}"))?;
        if count == 0 {
            break;
        }
        let available = OUTPUT_LIMIT.saturating_sub(result.len());
        result.extend_from_slice(&buffer[..count.min(available)]);
        overflow |= count > available;
    }
    if overflow {
        Err("dprint output exceeded the 32 MiB safety limit. Use a smaller project scope.".into())
    } else {
        Ok(result)
    }
}
// The CLI starts suspended, joins an owned Job Object, then resumes. Descendants
// remain members after the CLI exits, so they cannot keep our pipes open forever.
#[cfg(windows)]
struct WindowsProcessTree(*mut std::ffi::c_void);
#[cfg(windows)]
mod windows_process_api {
    use std::ffi::c_void;
    #[repr(C)]
    pub struct ThreadEntry {
        pub size: u32,
        pub usage: u32,
        pub id: u32,
        pub owner: u32,
        pub base_priority: i32,
        pub delta_priority: i32,
        pub flags: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        pub fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        pub fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
        pub fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> *mut c_void;
        pub fn Thread32First(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        pub fn Thread32Next(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        pub fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> *mut c_void;
        pub fn ResumeThread(thread: *mut c_void) -> u32;
    }
}
#[cfg(windows)]
impl WindowsProcessTree {
    fn attach_and_resume(child: &mut Child, cancelled: &AtomicBool) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_process_api::*;
        // SAFETY: null name/attributes request an unnamed job; the Child owns a
        // valid live process handle and was created suspended immediately above.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            let error = std::io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Cannot create cancellable project job: {error}"));
        }
        let tree = Self(job);
        if unsafe { AssignProcessToJobObject(job, child.as_raw_handle()) } == 0 {
            let error = std::io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Cannot contain project process in a cancellable job: {error}"
            ));
        }
        if cancelled.load(Ordering::Acquire) {
            tree.terminate();
            let _ = child.wait();
            return Err("Project operation cancelled.".into());
        }
        // The suspended process has not run its primary thread. Enumerating its
        // owner PID lets us obtain that thread's handle without undocumented APIs.
        let result = unsafe {
            let snapshot = CreateToolhelp32Snapshot(4, 0); // TH32CS_SNAPTHREAD
            if snapshot == (-1_isize) as *mut std::ffi::c_void {
                Err(std::io::Error::last_os_error())
            } else {
                let mut entry = ThreadEntry {
                    size: std::mem::size_of::<ThreadEntry>() as u32,
                    usage: 0,
                    id: 0,
                    owner: 0,
                    base_priority: 0,
                    delta_priority: 0,
                    flags: 0,
                };
                let mut found = Err(std::io::Error::other(
                    "Suspended project thread was not found.",
                ));
                let mut valid = Thread32First(snapshot, &mut entry) != 0;
                while valid {
                    if entry.owner == child.id() {
                        let thread = OpenThread(2, 0, entry.id); // THREAD_SUSPEND_RESUME
                        if thread.is_null() {
                            found = Err(std::io::Error::last_os_error());
                        } else {
                            found = if ResumeThread(thread) == u32::MAX {
                                Err(std::io::Error::last_os_error())
                            } else {
                                Ok(())
                            };
                            CloseHandle(thread);
                        }
                        break;
                    }
                    entry.size = std::mem::size_of::<ThreadEntry>() as u32;
                    valid = Thread32Next(snapshot, &mut entry) != 0;
                }
                CloseHandle(snapshot);
                found
            }
        };
        if let Err(error) = result {
            tree.terminate();
            let _ = child.wait();
            return Err(format!("Cannot resume contained project process: {error}"));
        }
        Ok(tree)
    }
    fn terminate(&self) {
        // SAFETY: this guard owns the live job handle until Drop.
        unsafe {
            windows_process_api::TerminateJobObject(self.0, 1);
        }
    }
}
#[cfg(windows)]
impl Drop for WindowsProcessTree {
    fn drop(&mut self) {
        self.terminate();
        // SAFETY: close our uniquely owned job handle exactly once.
        unsafe {
            windows_process_api::CloseHandle(self.0);
        }
    }
}

fn terminate_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // SAFETY: each child gets its own process group via process_group(0).
        // A negative PID addresses that group only; SIGKILL is defined as 9 on
        // the supported Linux/macOS platforms. No pointers cross this boundary.
        unsafe {
            kill(-(child.id() as i32), 9);
        }
    }
    let _ = child.kill();
}

/// Exact character-class escaping from dprint 0.63.0's
/// `escape_glob_text_for_cli`. Backslash escaping is NOT valid in CLI patterns.
fn literal_pattern(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Source path is outside the project.")?;
    let text = relative
        .to_str()
        .ok_or("Source paths must be valid UTF-8.")?;
    #[cfg(windows)]
    let text = text.replace('\\', "/");
    // Colons can become fmt line ranges; newlines split stdin records, and
    // dprint treats every backslash as a separator, including on Unix.
    if text.is_empty() || text.contains(['\n', '\r', '\\', ':']) {
        return Err(format!(
            "Cannot safely pass this source path to dprint (newline, backslash, or colon): {}",
            path.display()
        ));
    }
    let mut pattern = String::from("./");
    for c in text.chars() {
        if matches!(c, '[' | ']' | '{' | '}' | '*' | '?') {
            pattern.push('[');
            pattern.push(c);
            pattern.push(']');
        } else {
            pattern.push(c);
        }
    }
    Ok(pattern)
}
fn parse_file_paths(request: &ProjectRequest, output: &[u8]) -> Result<BTreeSet<PathBuf>, String> {
    let text = std::str::from_utf8(output).map_err(|_| "dprint file list is not valid UTF-8.")?;
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| request.safe_file(Path::new(line)))
        .collect()
}
fn parse_check_json(
    request: &ProjectRequest,
    output: &[u8],
    targets: &BTreeSet<PathBuf>,
) -> Result<Vec<ProjectFile>, String> {
    let text =
        std::str::from_utf8(output).map_err(|_| "dprint check output is not valid UTF-8.")?;
    let mut files = BTreeMap::new();
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
    {
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| format!("Invalid dprint check JSON on line {}: {e}", index + 1))?;
        let path = value
            .get("file")
            .and_then(|v| v.as_str())
            .ok_or("dprint check JSON is missing a string 'file'.")?;
        let diff = value
            .get("diff")
            .and_then(|v| v.as_str())
            .ok_or("dprint check JSON is missing a string 'diff'.")?;
        let path = request.safe_file(Path::new(path))?;
        if !targets.contains(&path) {
            return Err("dprint returned a difference outside the approved source scope.".into());
        }
        if files.insert(path, diff.to_owned()).is_some() {
            return Err("dprint returned a duplicate source difference.".into());
        }
    }
    Ok(files
        .into_iter()
        .map(|(path, diff)| ProjectFile { path, diff })
        .collect())
}

fn protected_configs(
    request: &ProjectRequest,
    files: &BTreeSet<PathBuf>,
) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let mut configs = BTreeMap::new();
    protect_config(
        request.config_path(),
        &mut configs,
        request.action.writes_sources(),
    )?;
    let mut directories = BTreeSet::new();
    for path in files {
        for parent in path
            .ancestors()
            .skip(1)
            .take_while(|p| p.starts_with(&request.root))
        {
            directories.insert(parent.to_owned());
        }
    }
    for directory in directories {
        for name in CONFIG_NAMES {
            let config = directory.join(name);
            if config.is_file() {
                protect_config(&config, &mut configs, request.action.writes_sources())?;
            }
        }
    }
    Ok(configs)
}
fn protect_config(
    path: &Path,
    configs: &mut BTreeMap<PathBuf, Vec<u8>>,
    writes: bool,
) -> Result<(), String> {
    let path = fs::canonicalize(path).map_err(|e| {
        format!(
            "Cannot resolve project configuration {}: {e}",
            path.display()
        )
    })?;
    if configs.contains_key(&path) {
        return Ok(());
    }
    if configs.len() >= 4096 {
        return Err("Project configuration dependency limit exceeded.".into());
    }
    let bytes = fs::read(&path)
        .map_err(|e| format!("Cannot read project configuration {}: {e}", path.display()))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "Project configuration is not UTF-8.")?;
    let value = jsonc_parser::parse_to_serde_value(text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| {
            format!(
                "Cannot inspect project configuration {}: {e}",
                path.display()
            )
        })?
        .ok_or("Project configuration is empty.")?;
    let extends: Vec<String> = match value.get("extends") {
        Some(serde_json::Value::String(path)) => vec![path.clone()],
        Some(serde_json::Value::Array(paths)) => paths
            .iter()
            .filter_map(|p| p.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    let plugins: Vec<String> = value
        .get("plugins")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    configs.insert(path.clone(), bytes);
    let directory = path.parent().expect("file has parent");
    for reference in extends {
        // Remote config sources are never formatting targets. Preserve all
        // locally resolvable extends, including arbitrary-named JSON/JSONC.
        if reference.starts_with("file:")
            || reference.starts_with("~/")
            || reference.starts_with("~\\")
        {
            if !writes {
                continue;
            }
            return Err(format!(
                "Cannot safely protect local inherited configuration reference {reference:?}. Use an ordinary relative or absolute filesystem path before project operations."
            ));
        }
        if reference.contains("://") || reference.starts_with("npm:") {
            if writes {
                return Err("Project formatting cannot safely protect local configurations inherited through remote or package references. Use Check project, or a configuration with only ordinary local extends paths.".into());
            }
            continue;
        }
        let reference = reference.replace("${configDir}", &directory.to_string_lossy());
        if reference.contains("${") {
            if !writes {
                continue;
            }
            return Err(format!(
                "Cannot safely resolve inherited configuration placeholder in {reference:?}. Use an ordinary filesystem path before project operations."
            ));
        }
        let resolved = local_dependency_path(directory, &reference);
        if resolved.is_file() {
            protect_config(&resolved, configs, writes)?;
        }
    }
    for reference in plugins {
        let reference = reference
            .rsplit_once('@')
            .filter(|(_, checksum)| {
                checksum.len() == 64 && checksum.chars().all(|c| c.is_ascii_hexdigit())
            })
            .map(|(path, _)| path)
            .unwrap_or(&reference);
        if reference.starts_with("file:")
            || reference.starts_with("~/")
            || reference.starts_with("~\\")
        {
            if writes {
                return Err(format!(
                    "Cannot safely protect local plugin reference {reference:?}. Use an ordinary relative or absolute filesystem path before formatting."
                ));
            }
            continue;
        }
        if reference.contains("://") || reference.starts_with("npm:") {
            continue;
        }
        let reference = reference.replace("${configDir}", &directory.to_string_lossy());
        if reference.contains("${") {
            if writes {
                return Err(format!(
                    "Cannot safely resolve local plugin placeholder in {reference:?}. Use an ordinary filesystem path before formatting."
                ));
            }
            continue;
        }
        let file = local_dependency_path(directory, &reference);
        if file.is_file() {
            let file = fs::canonicalize(file)
                .map_err(|error| format!("Cannot protect local plugin: {error}"))?;
            let bytes = fs::read(&file)
                .map_err(|error| format!("Cannot read local plugin {}: {error}", file.display()))?;
            configs.entry(file).or_insert(bytes);
        }
    }
    Ok(())
}
fn local_dependency_path(directory: &Path, reference: &str) -> PathBuf {
    // Windows canonical paths use the verbatim \\?\ prefix. Unlike ordinary
    // Win32 paths, those do not normalize forward slashes. Config references
    // commonly use '/', including after a ${configDir} substitution.
    #[cfg(windows)]
    let reference = reference.replace('/', "\\");
    directory.join(reference)
}

fn verify_configs(configs: &BTreeMap<PathBuf, Vec<u8>>) -> Result<(), String> {
    for (path, expected) in configs {
        if fs::read(path).ok().as_ref() != Some(expected)
            || fs::canonicalize(path).ok().as_ref() != Some(path)
        {
            return Err(format!(
                "Project configuration changed during the operation: {}. Reload and review before running again.",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_dependency_graphs_are_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("dprint.json");
        for reference in [
            "file:///some/config.json",
            "~/base.json",
            "${originConfigDir}/base.json",
            "https://example.invalid/base.json",
            "npm:config",
        ] {
            fs::write(
                &config,
                serde_json::json!({"extends":reference}).to_string(),
            )
            .unwrap();
            let error = protect_config(&config, &mut BTreeMap::new(), true).unwrap_err();
            assert!(error.contains("safely"), "{error}");
            protect_config(&config, &mut BTreeMap::new(), false).unwrap();
        }
    }

    #[test]
    fn local_plugin_descriptors_are_protected_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let descriptor = dir.path().join("plugin.json");
        fs::write(&descriptor, "{\"schemaVersion\":2}").unwrap();
        let config = dir.path().join("dprint.json");
        fs::write(
            &config,
            serde_json::json!({"plugins":["${configDir}/plugin.json"]}).to_string(),
        )
        .unwrap();
        let mut protected = BTreeMap::new();
        protect_config(&config, &mut protected, true).unwrap();
        assert_eq!(
            protected[&fs::canonicalize(&descriptor).unwrap()],
            fs::read(&descriptor).unwrap()
        );
        fs::write(descriptor, "changed").unwrap();
        assert!(verify_configs(&protected).is_err());
    }
}
