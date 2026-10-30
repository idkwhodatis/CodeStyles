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
use sha2::{Digest, Sha256};
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

/// Closed set of managed formatters with single-language project scopes.
/// This is not an arbitrary executable or mixed-language project mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeLanguage {
    Rust,
    Go,
    Dart,
    Vue,
}
impl NativeLanguage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Go => "Go",
            Self::Dart => "Dart",
            Self::Vue => "Vue",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::Go => "go",
            Self::Dart => "dart",
            Self::Vue => "vue",
        }
    }
    fn formatter(self) -> &'static str {
        match self {
            Self::Rust => "rustfmt",
            Self::Go => "gofmt",
            Self::Dart => "dart format",
            Self::Vue => "dprint Vue WASM",
        }
    }
    fn excluded_directory(self, name: &str) -> bool {
        name == ".git"
            || match self {
                Self::Rust => name == "target",
                Self::Go => name == "vendor",
                Self::Dart => matches!(name, "build" | ".dart_tool"),
                Self::Vue => matches!(name, "node_modules" | "dist" | ".nuxt"),
            }
    }
    fn metadata_names(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &[
                ".gitignore",
                ".ignore",
                "Cargo.toml",
                "Cargo.lock",
                "rustfmt.toml",
                ".rustfmt.toml",
                "rust-toolchain.toml",
                "rust-toolchain",
            ],
            Self::Go => &[
                ".gitignore",
                ".ignore",
                "go.mod",
                "go.sum",
                "go.work",
                "go.work.sum",
            ],
            Self::Vue => &[
                ".gitignore",
                ".ignore",
                "package.json",
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
            ],
            Self::Dart => &[
                ".gitignore",
                ".ignore",
                "pubspec.yaml",
                "pubspec.lock",
                "analysis_options.yaml",
                ".dart_tool/package_config.json",
            ],
        }
    }
}

/// An immutable, explicitly authorized request. Private fields prevent callers
/// bypassing dirty/trust/disk-baseline checks by constructing it directly.
#[derive(Clone, Debug)]
pub struct ProjectRequest {
    root: PathBuf,
    config: ConfigSnapshot,
    action: ProjectAction,
    review: Option<Arc<ReviewSnapshot>>,
    native_language: Option<NativeLanguage>,
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
            review: None,
            native_language: None,
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
    /// Rust-only project scope. This never implies stock dprint can format Rust.
    pub fn new_rust(
        root: PathBuf,
        document: &ConfigDocument,
        action: ProjectAction,
        trusted: bool,
    ) -> Result<Self, String> {
        Self::new_native(root, document, action, trusted, NativeLanguage::Rust)
    }

    /// Explicit single-language scope using only a trusted managed formatter.
    /// Native project actions never prepare plugins or install tools implicitly.
    pub fn new_native(
        root: PathBuf,
        document: &ConfigDocument,
        action: ProjectAction,
        trusted: bool,
        language: NativeLanguage,
    ) -> Result<Self, String> {
        let mut request = Self::new(root, document, action, trusted)?;
        request.native_language = Some(language);
        validate_native_project_config(request.config.source_text(), language)?;
        Ok(request)
    }

    pub fn from_native_review(
        root: PathBuf,
        document: &ConfigDocument,
        selected_paths: Vec<PathBuf>,
        report: &ProjectReport,
        trusted: bool,
        language: NativeLanguage,
    ) -> Result<Self, String> {
        Self::new_native(
            root,
            document,
            ProjectAction::FormatSelected(selected_paths),
            trusted,
            language,
        )?
        .attach_review(report)
    }

    pub fn native_language(&self) -> Option<NativeLanguage> {
        self.native_language
    }

    pub fn from_rust_review(
        root: PathBuf,
        document: &ConfigDocument,
        selected_paths: Vec<PathBuf>,
        report: &ProjectReport,
        trusted: bool,
    ) -> Result<Self, String> {
        Self::new_rust(
            root,
            document,
            ProjectAction::FormatSelected(selected_paths),
            trusted,
        )?
        .attach_review(report)
    }

    pub fn is_rust(&self) -> bool {
        self.native_language == Some(NativeLanguage::Rust)
    }

    /// Format only differing files selected from a successful displayed check.
    /// The worker revalidates sources, dependencies and diffs before formatting.
    /// This is not an atomic patch application: trusted plugins are rerun, so
    /// avoid concurrent edits and rely on the automatic post-format check.
    pub fn from_review(
        root: PathBuf,
        document: &ConfigDocument,
        selected_paths: Vec<PathBuf>,
        report: &ProjectReport,
        trusted: bool,
    ) -> Result<Self, String> {
        Self::new(
            root,
            document,
            ProjectAction::FormatSelected(selected_paths),
            trusted,
        )?
        .attach_review(report)
    }

    fn attach_review(mut self, report: &ProjectReport) -> Result<Self, String> {
        let request = &mut self;
        let review = report
            .review
            .as_ref()
            .ok_or("Run Check project before applying reviewed changes.")?;
        if review.root != request.root
            || review.config_path != request.config_path()
            || review.native_language != request.native_language
        {
            return Err(
                "The review belongs to a different project configuration. Check again.".into(),
            );
        }
        if let ProjectAction::FormatSelected(paths) = &request.action {
            for path in paths {
                if !review.sources.contains_key(&request.safe_file(path)?) {
                    return Err("Only files with differences in the displayed review may be applied. Check again.".into());
                }
            }
        }
        request.review = Some(review.clone());
        Ok(self)
    }

    fn verify_review(&self, discovered: &BTreeSet<PathBuf>) -> Result<(), String> {
        let Some(review) = &self.review else {
            return Ok(());
        };
        if protected_configs(self, discovered)? != review.configs {
            return Err("Project configuration or local plugins changed since review. Run Check project again before applying.".into());
        }
        verify_configs(&review.configs)?;
        if let ProjectAction::FormatSelected(paths) = &self.action {
            for path in paths {
                let path = self.safe_file(path)?;
                let expected = review
                    .sources
                    .get(&path)
                    .ok_or("A selected source path changed since review. Check again.")?;
                verify_source(&path, expected)?;
            }
        }
        Ok(())
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
    /// Current differing files, including the full-project check after formatting.
    pub files: Vec<ProjectFile>,
    pub diagnostics: String,
    pub summary: String,
    pub checked_files: usize,
    pub protected_configs: usize,
    review: Option<Arc<ReviewSnapshot>>,
}

#[derive(Clone, Debug)]
struct ReviewSnapshot {
    root: PathBuf,
    config_path: PathBuf,
    discovered: BTreeSet<PathBuf>,
    configs: BTreeMap<PathBuf, Vec<u8>>,
    sources: BTreeMap<PathBuf, SourceFingerprint>,
    diffs: BTreeMap<PathBuf, String>,
    native_language: Option<NativeLanguage>,
}

impl Backend {
    pub fn execute_project(
        &self,
        request: &ProjectRequest,
        cancelled: Arc<AtomicBool>,
    ) -> Result<ProjectReport, String> {
        let mut writes_started = false;
        let result = if request.native_language.is_some() {
            self.execute_native_project(request, &cancelled, &mut writes_started)
        } else {
            self.execute_project_inner(request, &cancelled, &mut writes_started)
        };
        result.map_err(|error| {
            if writes_started {
                format!("{error}\nFormatting is not transactional; files may already have been changed. Run Check project to inspect the current state.")
            } else { error }
        })
    }

    fn execute_project_inner(
        &self,
        request: &ProjectRequest,
        cancelled: &Arc<AtomicBool>,
        writes_started: &mut bool,
    ) -> Result<ProjectReport, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("Project operation cancelled.".into());
        }
        request.verify_saved_config()?;
        if let Some(review) = &request.review {
            // Reject stale trusted resources before any CLI operation can load
            // or execute configuration plugins. Fresh discovery is checked again.
            request.verify_review(&review.discovered)?;
        }
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
            review: None,
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
        request.verify_review(&all_files)?;
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
        let sources = targets
            .iter()
            .map(|path| source_fingerprint(path).map(|fingerprint| (path.clone(), fingerprint)))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
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
        for (path, bytes) in &sources {
            verify_source(path, bytes)?;
        }
        request.verify_review(&all_files)?;
        if let Some(review) = &request.review {
            let current: BTreeMap<_, _> = report
                .files
                .iter()
                .map(|file| (file.path.clone(), file.diff.clone()))
                .collect();
            let expected: BTreeMap<_, _> = targets
                .iter()
                .map(|path| (path.clone(), review.diffs[path].clone()))
                .collect();
            if current != expected {
                return Err("Formatting differences changed since review. Run Check project again before applying.".into());
            }
        }
        if protected_configs(request, &all_files)? != configs {
            return Err(
                "Project configuration dependencies changed during check. Check again.".into(),
            );
        }
        if request.action.writes_sources() {
            request.verify_saved_config()?;
            // Recheck file identity/containment immediately before starting writes.
            for path in &targets {
                if request.safe_file(path)? != *path {
                    return Err("A source path changed during project validation. Check again before formatting.".into());
                }
            }
            // Last content/config recheck catches changes during the CLI check.
            // CLI writes are not atomic with these reads; concurrent editors must
            // still be avoided while applying, and failures may be partial.
            request.verify_review(&all_files)?;
            let current_protected = configs
                .keys()
                .map(same_file::Handle::from_path)
                .collect::<Result<std::collections::HashSet<_>, _>>()
                .map_err(|e| format!("Cannot revalidate protected files: {e}"))?;
            for path in &targets {
                let identity = same_file::Handle::from_path(path)
                    .map_err(|e| format!("Cannot revalidate source {}: {e}", path.display()))?;
                if current_protected.contains(&identity) {
                    return Err("A source became an alias of a protected configuration or plugin. Check again before formatting.".into());
                }
            }
            if cancelled.load(Ordering::Acquire) {
                return Err("Project operation cancelled.".into());
            }
            *writes_started = true;
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
            let verification_request = ProjectRequest {
                root: request.root.clone(),
                config: request.config.clone(),
                action: ProjectAction::Check,
                review: None,
                native_language: None,
            };
            let mut verification = self
                .execute_project_inner(&verification_request, cancelled, writes_started)
                .map_err(|error| {
                    format!("Formatting ran, but the automatic verification failed: {error}")
                })?;
            verification.action = request.action.clone();
            verification.summary = format!(
                "Formatted {} selected source files. Verified all {} included source files; {} still need formatting. Configuration and local plugin files were protected.",
                targets.len(),
                verification.checked_files,
                verification.files.len()
            );
            append_diagnostics(&mut diagnostics, verification.diagnostics.as_bytes());
            verification.diagnostics = diagnostics;
            return Ok(verification);
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
        report.review = Some(Arc::new(ReviewSnapshot {
            root: request.root.clone(),
            config_path: request.config_path().to_owned(),
            discovered: all_files,
            configs,
            native_language: None,
            diffs: report
                .files
                .iter()
                .map(|file| (file.path.clone(), file.diff.clone()))
                .collect(),
            sources: report
                .files
                .iter()
                .map(|file| (file.path.clone(), sources[&file.path]))
                .collect(),
        }));
        report.diagnostics = diagnostics;
        Ok(report)
    }

    fn execute_native_project(
        &self,
        request: &ProjectRequest,
        cancelled: &Arc<AtomicBool>,
        writes_started: &mut bool,
    ) -> Result<ProjectReport, String> {
        let language = request
            .native_language
            .ok_or("Native project mode is required.")?;
        let label = language.label();
        request.verify_saved_config()?;
        validate_native_project_config(request.config.source_text(), language)?;
        if cancelled.load(Ordering::Acquire) {
            return Err("Native project operation cancelled.".into());
        }
        if request.action == ProjectAction::PreparePlugins {
            return Err("Use Language support to install this language. Native project checks never download or install tools implicitly.".into());
        }
        let (all_files, configs) = native_scope(request, cancelled)?;
        let protected = configs
            .keys()
            .map(same_file::Handle::from_path)
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(|e| format!("Cannot identify protected native configuration files: {e}"))?;
        let mut candidates = BTreeSet::new();
        for path in &all_files {
            let identity = same_file::Handle::from_path(path).map_err(|e| e.to_string())?;
            if !protected.contains(&identity) {
                candidates.insert(path.clone());
            }
        }
        if let Some(review) = &request.review {
            if review.native_language != Some(language)
                || review.discovered != all_files
                || review.configs != configs
            {
                return Err("Native project scope or configuration changed since review. Check again before applying.".into());
            }
            verify_configs(&review.configs)?;
        }
        let targets = match &request.action {
            ProjectAction::FormatSelected(selected) => selected.iter().map(|path| {
                let path = request.safe_file(path)?;
                if !candidates.contains(&path) { return Err("Selected native file is excluded, protected, or outside the reviewed native scope.".into()); }
                Ok(path)
            }).collect::<Result<BTreeSet<_>, String>>()?,
            _ => candidates.clone(),
        };
        let mut outputs = BTreeMap::new();
        let mut sources = BTreeMap::new();
        let mut differences = Vec::new();
        for path in &targets {
            if cancelled.load(Ordering::Acquire) {
                return Err("Native project operation cancelled.".into());
            }
            let fingerprint = native_source(path, cancelled)?.1;
            if fingerprint.bytes > OUTPUT_LIMIT as u64 {
                return Err("Native source exceeds the 32 MiB safety limit.".into());
            }
            if let Some(review) = &request.review {
                let expected = review
                    .sources
                    .get(path)
                    .ok_or("Native source is not in the reviewed changes.")?;
                verify_native_source(path, expected, cancelled)?;
            }
            let source = String::from_utf8(native_source(path, cancelled)?.0)
                .map_err(|_| "Native source is not valid UTF-8.")?;
            verify_native_source(path, &fingerprint, cancelled)?;
            let output = self.format(
                &crate::backend::FormatRequest {
                    source: source.clone(),
                    source_path: path.clone(),
                    config: request.config.clone(),
                },
                cancelled.clone(),
            )?;
            verify_native_source(path, &fingerprint, cancelled)?;
            if output != source {
                differences.push(ProjectFile {
                    path: path.clone(),
                    diff: native_diff(path, &source, &output, language),
                });
                outputs.insert(path.clone(), output);
            }
            sources.insert(path.clone(), fingerprint);
        }
        verify_configs(&configs)?;
        if native_scope(request, cancelled)? != (all_files.clone(), configs.clone()) {
            return Err(
                "Native project scope or ignore rules changed during check. Check again.".into(),
            );
        }
        if let Some(review) = &request.review {
            let actual: BTreeMap<_, _> = differences
                .iter()
                .map(|file| (file.path.clone(), file.diff.clone()))
                .collect();
            let expected: BTreeMap<_, _> = targets
                .iter()
                .map(|path| (path.clone(), review.diffs[path].clone()))
                .collect();
            if actual != expected {
                return Err(
                    "Native formatting differences changed since review. Check again.".into(),
                );
            }
        }
        if request.action.writes_sources() {
            request.verify_saved_config()?;
            for path in &targets {
                verify_native_source(path, &sources[path], cancelled)?;
            }
            for (path, output) in outputs {
                if cancelled.load(Ordering::Acquire) {
                    return Err("Native project operation cancelled.".into());
                }
                verify_configs(&configs)?;
                verify_native_source(&path, &sources[&path], cancelled)?;
                if request.safe_file(&path)? != path {
                    return Err("Native source path changed before write.".into());
                }
                replace_native_source(
                    &path,
                    &output,
                    &sources[&path],
                    &configs,
                    cancelled,
                    writes_started,
                )?;
            }
            let verification = ProjectRequest {
                root: request.root.clone(),
                config: request.config.clone(),
                action: ProjectAction::Check,
                review: None,
                native_language: Some(language),
            };
            let mut report = self
                .execute_native_project(&verification, cancelled, writes_started)
                .map_err(|e| {
                    format!("Native formatting ran but automatic verification failed: {e}")
                })?;
            report.action = request.action.clone();
            report.summary = format!(
                "Formatted {} selected {label} files; verified {} {label} files, {} still differ. Other languages were not changed.",
                targets.len(),
                report.checked_files,
                report.files.len()
            );
            return Ok(report);
        }
        request.verify_saved_config()?;
        let review = Arc::new(ReviewSnapshot {
            root: request.root.clone(),
            config_path: request.config_path().to_path_buf(),
            discovered: all_files,
            configs: configs.clone(),
            native_language: Some(language),
            diffs: differences
                .iter()
                .map(|file| (file.path.clone(), file.diff.clone()))
                .collect(),
            sources: differences
                .iter()
                .map(|file| (file.path.clone(), sources[&file.path]))
                .collect(),
        });
        Ok(ProjectReport {
            action: request.action.clone(), cli_version: if language == NativeLanguage::Rust { "Managed stable rustfmt".into() } else { format!("Managed {}", language.formatter()) },
            checked_files: candidates.len(), protected_configs: configs.len(),
            summary: format!("{} of {} {label} files need formatting. {label}-only scope; no source files changed.", differences.len(), candidates.len()),
            files: differences,
            diagnostics: match language {
                NativeLanguage::Rust => "Rust-only project mode respects local .gitignore/.ignore, skips target/.git and symlinks. It uses the explicit rustfmt section, not Cargo.toml or rustfmt.toml discovery. Other languages use the dprint project mode.",
                NativeLanguage::Go => "Go-only project mode respects local .gitignore/.ignore and skips vendor/.git and symlinks. Fixed gofmt behavior; no module discovery, dependency downloads or configurable style rules.",
                NativeLanguage::Vue => "Vue-only project mode uses verified markup/TypeScript/JSON/style WASM with explicit settings. It excludes node_modules/dist/.nuxt; external src and unsupported SFC blocks are rejected, not silently skipped.",
                NativeLanguage::Dart => "Dart-only project mode respects local .gitignore/.ignore and skips build/.dart_tool/.git and symlinks. Explicit snapshot formatting settings only; protected project metadata is not automatically inherited. Flutter support means Dart source only.",
            }.into(),
            review: Some(review),
        })
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
pub(crate) struct WindowsProcessTree(*mut std::ffi::c_void);
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
    pub(crate) fn attach_and_resume(
        child: &mut Child,
        cancelled: &AtomicBool,
    ) -> Result<Self, String> {
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
    pub(crate) fn terminate(&self) {
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

fn validate_native_project_config(text: &str, language: NativeLanguage) -> Result<(), String> {
    if language == NativeLanguage::Rust {
        crate::rustfmt::options(text)?;
    } else if language == NativeLanguage::Go {
        crate::native_format::options(text, crate::native_format::NativeLanguage::Go)?;
    } else if language == NativeLanguage::Dart {
        crate::native_format::options(text, crate::native_format::NativeLanguage::Dart)?;
    } else if language == NativeLanguage::Vue {
        crate::config::validate_vue_config(text)?;
    }
    let value = jsonc_parser::parse_to_serde_value(text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())?
        .ok_or("Empty project configuration.")?;
    if !value.is_object() {
        return Err("Native project configuration must be a JSONC object.".into());
    }
    for key in [
        "includes",
        "excludes",
        "incremental",
        "extends",
        "overrides",
    ] {
        if value.get(key).is_some() {
            return Err(format!(
                "{}-only project mode does not interpret dprint {key}. Use a separate explicit configuration without dprint discovery overrides; local .gitignore/.ignore defines the scope.",
                language.label()
            ));
        }
    }
    if value
        .get("inherit")
        .is_some_and(|v| v != &serde_json::Value::Bool(false))
    {
        return Err("Native project mode does not resolve inherited configuration.".into());
    }
    Ok(())
}

type NativeScope = (BTreeSet<PathBuf>, BTreeMap<PathBuf, Vec<u8>>);

/// Exact local single-language scope and all files which influence that scope. Avoid
/// parent/global ignore rules so the review is independent of ambient user state.
fn native_scope(request: &ProjectRequest, cancelled: &AtomicBool) -> Result<NativeScope, String> {
    let language = request
        .native_language
        .ok_or("Native project mode is required.")?;
    let mut files = BTreeSet::new();
    let mut configs = BTreeMap::new();
    configs.insert(
        request.config_path().to_path_buf(),
        fs::read(request.config_path()).map_err(|e| e.to_string())?,
    );
    let walker = ignore::WalkBuilder::new(&request.root)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(false)
        .max_depth(Some(128))
        .filter_entry(move |entry| {
            entry.depth() == 0
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| language.excluded_directory(name))
        })
        .build();
    let mut count = 0;
    for entry in walker {
        if cancelled.load(Ordering::Acquire) {
            return Err("Native project operation cancelled.".into());
        }
        let entry = entry.map_err(|e| format!("Cannot discover Native files: {e}"))?;
        if let Some(error) = entry.error() {
            return Err(format!("Cannot interpret Native ignore rules: {error}"));
        }
        count += 1;
        if count > 100_000 {
            return Err(
                "Native project discovery exceeded the 100,000-entry limit; choose a smaller root."
                    .into(),
            );
        }
        let path = entry.path();
        let kind = entry.file_type();
        if kind.is_some_and(|kind| kind.is_symlink()) {
            continue;
        }
        if kind.is_some_and(|kind| kind.is_dir()) {
            if entry.depth() == 128 {
                return Err("Native project discovery exceeded the directory-depth limit; choose a smaller root.".into());
            }
            for name in CONFIG_NAMES {
                let config = path.join(name);
                if config.is_file()
                    && fs::canonicalize(&config).ok().as_deref() != Some(request.config_path())
                {
                    return Err(format!(
                        "Native-only project mode does not resolve nested or competing dprint configuration {}. Choose that directory/configuration as a separate project.",
                        config.display()
                    ));
                }
            }
            for name in language.metadata_names() {
                let config = path.join(name);
                // A skipped .dart_tool directory must not hide an external
                // package config reached through a symlink/reparse point.
                if name.contains('/') {
                    let parent = config.parent().expect("nested metadata has a parent");
                    match fs::symlink_metadata(parent) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => return Err(e.to_string()),
                        Ok(metadata) => {
                            if !metadata.is_dir()
                                || metadata.file_type().is_symlink()
                                || fs::canonicalize(parent).ok().as_deref() != Some(parent)
                            {
                                return Err("Native project metadata directory must be a real in-root directory.".into());
                            }
                        }
                    }
                }
                match fs::symlink_metadata(&config) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(format!("Cannot inspect Native scope metadata: {e}")),
                    Ok(metadata) => {
                        if !metadata.is_file() || metadata.file_type().is_symlink() {
                            return Err(format!(
                                "Native project configuration must be a regular non-symlink file: {}",
                                config.display()
                            ));
                        }
                        if metadata.len() > 4 * 1024 * 1024 {
                            return Err(
                                "Native project metadata exceeds the 4 MiB safety limit.".into()
                            );
                        }
                        configs.insert(
                            fs::canonicalize(&config).map_err(|e| e.to_string())?,
                            fs::read(&config).map_err(|e| e.to_string())?,
                        );
                        if configs.len() > 4096 {
                            return Err("Native project configuration limit exceeded.".into());
                        }
                    }
                }
            }
        } else if kind.is_some_and(|kind| kind.is_file())
            && path
                .extension()
                .is_some_and(|extension| extension == language.extension())
        {
            files.insert(request.safe_file(path)?);
        }
    }
    Ok((files, configs))
}

/// Stage beside the destination and atomically replace it, never unlinking the
/// original first. All identity and staging handles are closed before commit:
/// Windows replacement must not depend on open-handle sharing semantics.
fn replace_native_source(
    path: &Path,
    output: &str,
    expected: &SourceFingerprint,
    configs: &BTreeMap<PathBuf, Vec<u8>>,
    cancelled: &AtomicBool,
    writes_started: &mut bool,
) -> Result<(), String> {
    {
        let identity = same_file::Handle::from_path(path).map_err(|e| e.to_string())?;
        let protected = configs
            .keys()
            .map(same_file::Handle::from_path)
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(|e| e.to_string())?;
        if protected.contains(&identity) {
            return Err("Native source aliases a protected configuration.".into());
        }
    }
    let permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    if permissions.readonly() {
        return Err("Cannot replace read-only Native source.".into());
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Native source has no parent.")?)
            .map_err(|e| format!("Cannot stage Native formatting: {e}"))?;
    temporary
        .write_all(output.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary
        .as_file()
        .set_permissions(permissions)
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    // into_temp_path closes the writable handle but retains RAII cleanup if any
    // final guard or the atomic rename fails. No delete-before-rename fallback.
    let temporary = temporary.into_temp_path();
    verify_native_source(path, expected, cancelled)?;
    verify_configs(configs)?;
    if fs::metadata(path)
        .map_err(|e| e.to_string())?
        .permissions()
        .readonly()
    {
        return Err("Cannot replace read-only Native source.".into());
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("Native project operation cancelled.".into());
    }
    temporary
        .persist(path)
        .map_err(|e| format!("Cannot replace Native source: {}", e.error))?;
    *writes_started = true;
    Ok(())
}

fn native_diff(path: &Path, source: &str, formatted: &str, language: NativeLanguage) -> String {
    use crate::comparison::{DiffKind, line_diff};
    let mut output = format!(
        "--- {}\n+++ {} ({})\n",
        path.display(),
        path.display(),
        language.formatter()
    );
    for line in line_diff(source, formatted) {
        output.push(match line.kind {
            DiffKind::Unchanged => ' ',
            DiffKind::Removed => '-',
            DiffKind::Added => '+',
        });
        output.push_str(&line.text);
        if !line.text.ends_with('\n') {
            output.push_str("\n\\ No newline at end of file\n");
        }
    }
    output
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceFingerprint {
    bytes: u64,
    sha256: [u8; 32],
}
fn native_source(
    path: &Path,
    cancelled: &AtomicBool,
) -> Result<(Vec<u8>, SourceFingerprint), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > OUTPUT_LIMIT as u64
    {
        return Err(
            "Native source must be a regular non-symlink file within the 32 MiB safety limit."
                .into(),
        );
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Native project operation cancelled.".into());
        }
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if data.len() + count > OUTPUT_LIMIT {
            return Err("Native source exceeds the 32 MiB safety limit.".into());
        }
        data.extend_from_slice(&buffer[..count]);
    }
    let fingerprint = SourceFingerprint {
        bytes: data.len() as u64,
        sha256: Sha256::digest(&data).into(),
    };
    Ok((data, fingerprint))
}
fn verify_native_source(
    path: &Path,
    expected: &SourceFingerprint,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    if native_source(path, cancelled)?.1 != *expected
        || fs::canonicalize(path).ok().as_deref() != Some(path)
    {
        return Err(format!(
            "Source changed since review or during check: {}. Run Check project again before applying.",
            path.display()
        ));
    }
    Ok(())
}
fn source_fingerprint(path: &Path) -> Result<SourceFingerprint, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Cannot read source {}: {e}", path.display()))?;
    let mut digest = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("Cannot read source {}: {e}", path.display()))?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        digest.update(&buffer[..count]);
    }
    Ok(SourceFingerprint {
        bytes,
        sha256: digest.finalize().into(),
    })
}
fn verify_source(path: &Path, expected: &SourceFingerprint) -> Result<(), String> {
    if source_fingerprint(path).ok().as_ref() != Some(expected)
        || fs::canonicalize(path).ok().as_deref() != Some(path)
    {
        return Err(format!(
            "Source changed since review or during check: {}. Run Check project again before applying.",
            path.display()
        ));
    }
    Ok(())
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

    fn native_fixture(
        language: NativeLanguage,
    ) -> (tempfile::TempDir, ConfigDocument, ProjectRequest) {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("dprint.jsonc"), "{}").unwrap();
        let document = ConfigDocument::load(&root.join("dprint.jsonc")).unwrap();
        let request =
            ProjectRequest::new_native(root, &document, ProjectAction::Check, true, language)
                .unwrap();
        (dir, document, request)
    }

    #[test]
    fn native_scopes_are_single_language_and_protect_metadata_without_inheriting_it() {
        for language in [
            NativeLanguage::Rust,
            NativeLanguage::Go,
            NativeLanguage::Dart,
        ] {
            let (_dir, _document, request) = native_fixture(language);
            let root = request.root();
            for extension in ["rs", "go", "dart", "vue"] {
                fs::write(root.join(format!("source.{extension}")), "source").unwrap();
            }
            let skipped = match language {
                NativeLanguage::Rust => "target",
                NativeLanguage::Go => "vendor",
                NativeLanguage::Dart => "build",
                NativeLanguage::Vue => "node_modules",
            };
            fs::create_dir(root.join(skipped)).unwrap();
            fs::write(
                root.join(skipped)
                    .join(format!("skip.{}", language.extension())),
                "skip",
            )
            .unwrap();
            for name in language.metadata_names() {
                let path = root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "").unwrap();
            }
            let cancel = AtomicBool::new(false);
            let (files, configs) = native_scope(&request, &cancel).unwrap();
            assert_eq!(
                files,
                BTreeSet::from([root.join(format!("source.{}", language.extension()))])
            );
            for name in language.metadata_names() {
                assert!(configs.contains_key(&root.join(name)));
            }
            let metadata = root.join(language.metadata_names()[2]);
            fs::write(&metadata, "changed").unwrap();
            assert!(verify_configs(&configs).is_err());
            assert_ne!(native_scope(&request, &cancel).unwrap().1, configs);
            fs::remove_file(metadata).unwrap();
            assert_ne!(native_scope(&request, &cancel).unwrap().1, configs);
            assert!(native_scope(&request, &AtomicBool::new(true)).is_err());
        }
    }

    #[test]
    fn native_review_cannot_cross_language_or_dprint_mode() {
        let (_dir, document, request) = native_fixture(NativeLanguage::Go);
        let path = request.root().join("source.go");
        fs::write(&path, "package main").unwrap();
        let fingerprint = native_source(&path, &AtomicBool::new(false)).unwrap().1;
        let report = ProjectReport {
            action: ProjectAction::Check,
            cli_version: String::new(),
            files: Vec::new(),
            diagnostics: String::new(),
            summary: String::new(),
            checked_files: 1,
            protected_configs: 1,
            review: Some(Arc::new(ReviewSnapshot {
                root: request.root.clone(),
                config_path: request.config_path().into(),
                discovered: BTreeSet::from([path.clone()]),
                configs: BTreeMap::new(),
                sources: BTreeMap::from([(path.clone(), fingerprint)]),
                diffs: BTreeMap::new(),
                native_language: Some(NativeLanguage::Go),
            })),
        };
        assert!(
            ProjectRequest::from_native_review(
                request.root.clone(),
                &document,
                vec![path.clone()],
                &report,
                true,
                NativeLanguage::Go
            )
            .is_ok()
        );
        for other in [NativeLanguage::Rust, NativeLanguage::Dart] {
            assert!(
                ProjectRequest::from_native_review(
                    request.root.clone(),
                    &document,
                    vec![path.clone()],
                    &report,
                    true,
                    other
                )
                .is_err()
            );
        }
        assert!(
            ProjectRequest::from_review(request.root.clone(), &document, vec![path], &report, true)
                .is_err()
        );
        let rust =
            ProjectRequest::new_rust(request.root.clone(), &document, ProjectAction::Check, true)
                .unwrap();
        assert!(rust.is_rust());
        assert_eq!(rust.native_language(), Some(NativeLanguage::Rust));
    }

    #[test]
    fn native_modes_reject_implicit_discovery_and_nested_configs() {
        for language in [
            NativeLanguage::Rust,
            NativeLanguage::Go,
            NativeLanguage::Dart,
        ] {
            for config in [
                r#"{"extends":"base.json"}"#,
                r#"{"includes":["**/*"]}"#,
                r#"{"inherit":true}"#,
            ] {
                assert!(validate_native_project_config(config, language).is_err());
            }
            let (_dir, _document, request) = native_fixture(language);
            fs::create_dir(request.root.join("nested")).unwrap();
            fs::write(request.root.join("nested/dprint.json"), "{}").unwrap();
            assert!(native_scope(&request, &AtomicBool::new(false)).is_err());
        }
    }

    #[test]
    fn native_source_atomic_replace_existing_preserves_permissions_and_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(dir.path()).unwrap().join("source.rs");
        fs::write(&path, "fn main( ){}\n").unwrap();
        let sibling = path.with_file_name("sibling.rs");
        fs::write(&sibling, "untouched").unwrap();
        let permissions = fs::metadata(&path).unwrap().permissions();
        let cancelled = AtomicBool::new(false);
        let expected = native_source(&path, &cancelled).unwrap().1;
        let mut written = false;
        replace_native_source(
            &path,
            "fn main() {}\n",
            &expected,
            &BTreeMap::new(),
            &cancelled,
            &mut written,
        )
        .unwrap();
        assert!(written);
        assert_eq!(fs::read_to_string(&path).unwrap(), "fn main() {}\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions(), permissions);
        assert_eq!(fs::read_to_string(sibling).unwrap(), "untouched");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn native_source_atomic_replace_readonly_and_stale_fail_without_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(dir.path()).unwrap().join("source.rs");
        fs::write(&path, "original").unwrap();
        let cancelled = AtomicBool::new(false);
        let expected = native_source(&path, &cancelled).unwrap().1;
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly = original_permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let mut written = false;
        let error = replace_native_source(
            &path,
            "changed",
            &expected,
            &BTreeMap::new(),
            &cancelled,
            &mut written,
        )
        .unwrap_err();
        assert!(error.contains("read-only"), "{error}");
        assert!(!written);
        assert_eq!(fs::read(&path).unwrap(), b"original");
        fs::set_permissions(&path, original_permissions).unwrap();
        fs::write(&path, "external edit").unwrap();
        assert!(
            replace_native_source(
                &path,
                "changed",
                &expected,
                &BTreeMap::new(),
                &cancelled,
                &mut written
            )
            .is_err()
        );
        assert!(!written);
        assert_eq!(fs::read(&path).unwrap(), b"external edit");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

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

#[cfg(test)]
mod vue_config_contract_tests {
    use super::*;
    #[test]
    fn empty_vue_scope_still_rejects_invalid_formatter_settings() {
        for text in [
            r#"{"markup":false}"#,
            r#"{"markup":{"scriptFormatter":"prettier"}}"#,
            r#"{"malva":[]}"#,
        ] {
            assert!(validate_native_project_config(text, NativeLanguage::Vue).is_err());
        }
        assert!(validate_native_project_config("{}", NativeLanguage::Vue).is_ok());
    }
}
