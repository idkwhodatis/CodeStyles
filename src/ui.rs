//! Native P2 schema-driven style editing. Formatting and debounce live off the UI thread.
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    time::Duration,
};

use crate::preview_display::PreviewDisplay;
use codestyles::{
    backend::FormatRequest,
    comparison::{DiffKind, SourceLanguage, SourceState, line_diff},
    config::ConfigDocument,
    integration,
    preview::PreviewController,
    project::{NativeLanguage, ProjectAction, ProjectReport, ProjectRequest},
    project_worker::ProjectController,
    resources,
    schema::{
        FieldKind, PinnedPlugin, SchemaCatalog, SchemaContext, SchemaField, setting_categories,
    },
    toolchain_worker::{PackAction, PackSnapshot, ToolchainController},
    toolchains::{PackFootprint, ToolchainStatus},
};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Selectable, Sizable, Theme, ThemeMode, WindowExt,
        button::Button,
        checkbox::Checkbox,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        select::{Select, SelectEvent, SelectState},
        switch::Switch,
        tab::{Tab, TabBar},
        tooltip::Tooltip,
    },
    prelude::*,
    *,
};
use serde_json::Value;

pub struct PreviewWindow {
    controller: Option<PreviewController>,
    project_controller: Option<ProjectController>,
    executable: Option<PathBuf>,
    toolchain_controller: ToolchainController,
    pack_snapshot: Option<PackSnapshot>,
    pack_status: String,
    pack_error: bool,
    page: Page,
    project_report: Option<ProjectReport>,
    project_selection: ProjectSelection,
    project_status: String,
    project_error: bool,
    document: Option<ConfigDocument>,
    // Only the checksummed bundle's exact plugin references have a known schema.
    schema: Result<SchemaCatalog, String>,
    source: SourceState,
    preview_display: PreviewDisplay,
    inline_selects: HashMap<String, Entity<SelectState<Vec<String>>>>,
    inline_editors: HashMap<String, Entity<InputState>>,
    search_editor: Option<Entity<InputState>>,
    field_editor: Option<Entity<InputState>>,
    field_draft: FieldDraft,
    subscriptions: Vec<Subscription>,
    selected_field: Option<String>,
    group: String,
    vue_settings: VueSettingsGroup,
    search: String,
    diff_mode: bool,
    baseline: Option<String>,
    baseline_error: Option<String>,
    trusted: bool,
    document_version: u64,
    dialog_pending: bool,
    rustfmt_import: ImportOperation,
    disk_warning: Option<String>,
    editor: Option<Entity<TextareaState>>,
    editor_subscription: Option<Subscription>,
    raw_dirty: bool,
    show_config: bool,
    show_saved: bool,

    revision: u64,
    output: String,
    last_success: Option<u64>,
    status: String,
    error: bool,
    pending: bool,
}

impl PreviewWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let backend = resources::bundled_backend();
        let executable = backend
            .as_ref()
            .ok()
            .map(|backend| backend.executable().to_path_buf());
        let project_controller = backend
            .as_ref()
            .ok()
            .map(|backend| ProjectController::new(backend.clone()));
        let (controller, document, status, error) = match backend {
            Ok(backend) => match backend.standalone_config() {
                Ok(document) => (
                    Some(PreviewController::new(backend)),
                    Some(document),
                    "Ready".into(),
                    false,
                ),
                Err(error) => (None, None, error, true),
            },
            Err(error) => (None, None, format!("Backend unavailable: {error}"), true),
        };
        let mut view = Self {
            controller,
            project_controller,
            executable,
            toolchain_controller: ToolchainController::new(),
            pack_snapshot: None,
            pack_status: "Checking installed language packs…".into(),
            pack_error: false,
            page: Page::Styles,
            project_report: None,
            project_selection: ProjectSelection::default(),
            project_status:
                "Choose a saved project configuration to check or format its directory.".into(),
            project_error: false,
            document,
            schema: SchemaCatalog::bundled(),
            source: SourceState::default(),
            preview_display: PreviewDisplay::default(),
            inline_selects: HashMap::new(),
            inline_editors: HashMap::new(),
            search_editor: None,
            field_editor: None,
            field_draft: FieldDraft::default(),
            subscriptions: Vec::new(),
            selected_field: None,
            group: String::new(),
            vue_settings: VueSettingsGroup::Template,
            search: String::new(),
            diff_mode: false,
            baseline: None,
            baseline_error: None,
            trusted: true,
            document_version: 0,
            dialog_pending: false,
            rustfmt_import: ImportOperation::default(),
            disk_warning: None,
            editor: None,
            editor_subscription: None,
            raw_dirty: false,
            show_config: false,
            show_saved: false,
            revision: 0,
            output: String::new(),
            last_success: None,
            status,
            error,
            pending: false,
        };
        let _ = view.toolchain_controller.submit(PackAction::Refresh);
        view.request_preview(cx);
        // The timer owns only a weak view reference, so closing the window also
        // releases the controller and its cancellable formatter worker.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let Some(view) = this.upgrade() else { break };
                view.update(cx, |view, cx| {
                    if let Some(progress) = view.toolchain_controller.progress() {
                        view.pack_status = match progress.total_bytes {
                            Some(total) => format!("{} · {:.1} / {:.1} MB", progress.phase, progress.downloaded_bytes as f64 / 1_000_000.0, total as f64 / 1_000_000.0),
                            None => format!("{} · {:.1} MB", progress.phase, progress.downloaded_bytes as f64 / 1_000_000.0),
                        };
                        cx.notify();
                    }
                    if let Some(completed) = view.toolchain_controller.poll() {
                        if completed.pack_id != view.toolchain_controller.pack_id() { return; }
                        let changed = !matches!(completed.action, PackAction::Refresh);
                        view.pack_error = completed.result.is_err();
                        view.pack_status = match completed.result {
                            Ok(()) => match completed.action {
                                PackAction::Refresh => "Language-pack status refreshed.".into(),
                                PackAction::Uninstall => "Selected language pack uninstalled. Bundled languages are unchanged.".into(),
                                _ => "Verified language pack installed.".into(),
                            },
                            Err(error) => error,
                        };
                        match completed.snapshot {
                            Ok(snapshot) => view.pack_snapshot = Some(snapshot),
                            Err(error) => { view.pack_snapshot = None; view.pack_error = true; view.pack_status.push_str(&format!("\nStatus unavailable: {error}")); }
                        }
                        if changed && pack_matches_preview(view.source.language(), completed.pack_id) {
                            view.project_report = None;
                            view.project_selection.clear();
                        }
                        if pack_matches_preview(view.source.language(), completed.pack_id) {
                            view.cancel_preview();
                            view.last_success = None;
                            view.output.clear();
                            view.request_preview(cx);
                        }
                        cx.notify();
                    }
                    if let Some(result) = view.project_controller.as_mut().and_then(|c| c.poll()) {
                        match result.result {
                            Ok(report) => {
                                view.project_status = report.summary.clone();
                                view.project_error = false;
                                view.project_selection.clear();
                                view.project_selection.focused =
                                    report.files.first().map(|file| file.path.clone());
                                view.project_report = Some(report);
                            }
                            Err(error) => {
                                view.project_status = error;
                                view.project_error = true;
                                view.project_selection.clear();
                                view.project_report = None;
                            }
                        }
                        cx.notify();
                    }
                    let Some(result) = view.controller.as_mut().and_then(|c| c.poll()) else {
                        return;
                    };
                    if result.revision != view.revision || !view.pending {
                        return;
                    }
                    view.pending = false;
                    match result.result {
                        Ok(output) => {
                            view.output = output;
                            view.last_success = Some(result.revision);
                            view.baseline = None;
                            view.baseline_error = None;
                            match result.baseline_result {
                                Some(Ok(text)) => view.baseline = Some(text),
                                Some(Err(error)) => view.baseline_error = Some(error),
                                None => {}
                            }
                            view.status = format!(
                                "{} · revision {}",
                                view.source.language().display_name(),
                                result.revision
                            );
                            view.error = false;
                        }
                        Err(error) => view.fail(format!("Formatting failed: {error}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        view
    }

    pub fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.rustfmt_import.cancel_for_close() {
            ImportClose::Picker => {
                self.status = "Import cancelled. Dismiss the native file picker, then close again. No imported settings will be applied.".into();
                cx.notify();
                return false;
            }
            ImportClose::Reading => {
                // Invalidate only this import, then use the ordinary unsaved
                // confirmation below. The OS read may still be pending.
                self.dialog_pending = false;
                self.status =
                    "Import cancelled. Any late file-read result will be discarded.".into();
            }
            ImportClose::None => {}
        }
        if self.toolchain_controller.is_running() {
            self.toolchain_controller.cancel();
            self.page = Page::Toolchains;
            self.pack_status = "Cancellation requested. Wait for the language-pack worker to stop, then close again. The previous installed version is retained if activation has not completed.".into();
            cx.notify();
            return false;
        }
        if self.project_busy() {
            self.project_status = "Cancel the running project operation and wait for it to stop before closing. Files already formatted are not rolled back.".into();
            cx.notify();
            return false;
        }
        // A pending file picker/confirmation must settle before closing; its
        // late answer must never begin a pack mutation after a close decision.
        if self.dialog_pending {
            return false;
        }
        if !self.dirty() && self.source.custom_source().is_none() {
            return true;
        }
        self.dialog_pending = true;
        let version = self.document_version;
        let confirmation = window.prompt(
            PromptLevel::Warning,
            "Close and discard unsaved work?",
            Some("Unsaved configuration changes, unapplied setting values, raw text, and custom source text will be lost. Cancel to return to your draft."),
            &["Cancel", "Discard changes and close"], cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let accepted = confirmation.await == Ok(1);
            let _ = this.update_in(cx, |this, window, cx| {
                this.dialog_pending = false;
                if accepted && dialog_is_current(version, this.document_version) {
                    this.cancel_preview();
                    window.remove_window();
                } else if accepted {
                    this.fail("The draft changed while the close dialog was open. Your changes are kept; close again when ready.".into());
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
        false
    }

    fn fail(&mut self, message: String) {
        self.status = message;
        self.error = true;
    }

    fn dirty(&self) -> bool {
        has_unsaved_changes(
            self.document.as_ref().is_some_and(ConfigDocument::is_dirty),
            self.raw_dirty || self.field_draft.is_dirty(),
        )
    }

    fn cancel_preview(&mut self) {
        if let Some(controller) = self.controller.as_mut() {
            self.revision = controller.cancel();
        }
        self.pending = false;
    }

    fn request_preview(&mut self, cx: &mut Context<Self>) {
        if self.project_busy()
            || (pack_matches_preview(self.source.language(), self.toolchain_controller.pack_id())
                && self.toolchain_controller.is_running())
            || !self.trusted
            || self.raw_dirty
        {
            self.cancel_preview();
            cx.notify();
            return;
        }
        if self.document.is_none() {
            return;
        }
        if self.controller.is_none() {
            self.fail("Preview unavailable: the bundled formatter could not be loaded. Configuration editing and saving remain available.".into());
            cx.notify();
            return;
        }
        let (Some(controller), Some(document)) = (self.controller.as_mut(), self.document.as_ref())
        else {
            return;
        };
        let path = self.source.source_path();
        let source = self.source.source();
        self.revision = controller.submit_comparison(
            FormatRequest {
                source: source.to_owned(),
                source_path: document
                    .path()
                    .and_then(|path| path.parent())
                    .map(|parent| parent.join(path))
                    .unwrap_or_else(|| PathBuf::from(path)),
                config: document.snapshot(),
            },
            document.saved_snapshot(),
        );
        self.pending = true;
        self.error = false;
        self.status = "Updating preview from the current draft…".into();
        cx.notify();
    }

    fn ensure_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        let text = self
            .document
            .as_ref()
            .map(|doc| doc.draft_text())
            .unwrap_or("")
            .to_owned();
        let editor = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx);
            state.set_value(text, window, cx);
            state
        });
        self.editor_subscription = Some(cx.subscribe(&editor, |this, editor, event, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let text = editor.read(cx).value();
            this.raw_dirty = this
                .document
                .as_ref()
                .is_some_and(|doc| doc.draft_text() != text.as_ref());
            this.project_selection.clear();
            this.project_report = None;
            this.document_version += 1;
            this.cancel_preview();
            this.status = if this.raw_dirty {
                "Raw edits are not applied. Apply valid JSONC before previewing or saving.".into()
            } else {
                "Raw text matches the current draft.".into()
            };
            this.error = false;
            if !this.raw_dirty {
                this.request_preview(cx);
            }
            cx.notify();
        }));
        self.editor = Some(editor);
    }

    fn sync_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(editor), Some(document)) = (&self.editor, &self.document) {
            let text = document.draft_text().to_owned();
            editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
        }
        self.raw_dirty = false;
    }

    fn change_setting(
        &mut self,
        field: &SchemaField,
        value: Option<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.raw_dirty {
            return false;
        }
        let plugin = self.plugin();
        let context = self.schema_context();
        let Some(document) = self.document.as_mut() else {
            return false;
        };
        let result = match value {
            Some(value) => field.set_in_context(document, plugin, value, context),
            None => field.unset_in_context(document, plugin, context),
        };
        let applied = result.is_ok();
        match result {
            Ok(()) => {
                self.project_selection.clear();
                self.project_report = None;
                self.document_version += 1;
                self.sync_editor(window, cx);
                if self.trusted {
                    self.request_preview(cx);
                } else {
                    self.status =
                        "Draft updated. Trust this configuration to enable preview.".into();
                    self.error = false;
                }
            }
            Err(error) => self.fail(error),
        }
        cx.notify();
        applied
    }

    fn apply_raw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.raw_dirty || !self.allow_field_transition(cx) {
            return;
        }
        let (Some(editor), Some(document)) = (&self.editor, &mut self.document) else {
            return;
        };
        let text = editor.read(cx).value();
        match document.replace_draft_text(&text) {
            Ok(()) => {
                self.raw_dirty = false;
                self.sync_field(window, cx);
                self.project_selection.clear();
                self.project_report = None;
                self.document_version += 1;
                // Raw edits can introduce or replace process plugins and extends.
                self.trusted = false;
                self.cancel_preview();
                self.status =
                    "Raw draft applied. Review and trust it before running a preview.".into();
                self.error = false;
            }
            Err(error) => self.fail(format!("Raw draft is invalid; your text is kept: {error}")),
        }
        cx.notify();
    }

    fn open_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_pending || self.project_busy() {
            return;
        }
        self.dialog_pending = true;
        let version = self.document_version;
        if self.dirty() {
            let confirmation = window.prompt(
                PromptLevel::Warning,
                "Open another configuration?",
                Some("Your unsaved configuration and pending setting/raw input will be discarded only after another configuration opens successfully. Custom source is kept."),
                &["Cancel", "Choose file and discard changes"], cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                let accepted = confirmation.await == Ok(1);
                let _ = this.update_in(cx, |this, window, cx| {
                    if !accepted {
                        this.dialog_pending = false;
                    } else if !dialog_is_current(version, this.document_version) {
                        this.dialog_pending = false;
                        this.fail("The draft changed while the dialog was open. Open the file again when ready.".into());
                    } else {
                        this.choose_config(window, cx);
                    }
                    cx.notify();
                });
            }).detach();
        } else {
            self.choose_config(window, cx);
        }
        cx.notify();
    }

    fn choose_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let version = self.document_version;
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open dprint JSON or JSONC configuration".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = match selection.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next().map(Ok),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(format!("Cannot open file picker: {error}"))),
                Err(error) => Some(Err(format!("File picker closed: {error}"))),
            };
            // Reading is allowed before trust; no formatter or plugin is started.
            let loaded = match selected {
                Some(Ok(path)) => Some(cx.background_executor().spawn(async move {
                    ConfigDocument::load(&path)
                }).await),
                Some(Err(error)) => Some(Err(error)),
                None => None,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.dialog_pending = false;
                if let Some(loaded) = loaded {
                    if !dialog_is_current(version, this.document_version) {
                        this.fail("The draft changed while the file picker was open. Your current draft was kept.".into());
                    } else {
                        match loaded {
                            Ok(document) => {
                                this.cancel_preview();
                                this.project_selection.clear();
                                this.project_report = None;
                                this.project_status = "Project changed. Check again to refresh results.".into();
                                this.document = Some(document);
                                this.disk_warning = None;
                                this.document_version += 1;
                                this.trusted = false;
                                this.output.clear();
                                this.last_success = None;
                                this.show_config = true;
                                this.selected_field = None;
                                this.sync_field(window, cx);
                                this.baseline = None;
                                this.baseline_error = None;
                                this.sync_editor(window, cx);
                                this.status = "Configuration imported for review. Preview is paused until you explicitly trust it.".into();
                                this.error = false;
                            }
                            Err(error) => this.fail(error),
                        }
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn trust_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_pending || self.raw_dirty || self.document.is_none() {
            return;
        }
        self.dialog_pending = true;
        let version = self.document_version;
        let confirmation = window.prompt(
            PromptLevel::Warning,
            "Trust the draft and saved baseline configurations?",
            Some("dprint may resolve extends and plugins over the network. Process plugins can execute programs with your account's permissions. Preview and Diff run both the current draft and the original saved baseline. Only trust both configurations and plugin sources you have reviewed. Trust applies to this pair; raw edits and Save As require another review."),
            &["Keep preview paused", "Trust and preview"], cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let accepted = confirmation.await == Ok(1);
            let _ = this.update(cx, |this, cx| {
                this.dialog_pending = false;
                if accepted && dialog_is_current(version, this.document_version) {
                    this.trusted = true;
                    this.request_preview(cx);
                } else if accepted {
                    this.fail("The draft changed while trust was requested. Review the latest draft again.".into());
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn check_disk(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        match document.has_external_changes() {
            Ok(true) => {
                let warning = "The saved file changed on disk. Your draft is kept. Save will refuse to overwrite it; reopen to review the disk version or Save As to a new file.".to_owned();
                self.disk_warning = Some(warning.clone());
                self.fail(warning);
            }
            Ok(false) => {
                self.disk_warning = None;
                self.status =
                    "Disk check: the saved file still matches the loaded baseline.".into();
                self.error = false;
            }
            Err(error) => {
                let warning = format!("Cannot verify the saved file: {error}");
                self.disk_warning = Some(warning.clone());
                self.fail(warning);
            }
        }
        cx.notify();
    }

    fn validate_for_save(&self) -> Result<(), String> {
        if let (Ok(schema), Some(document)) = (&self.schema, &self.document) {
            schema.validate_document(document)?;
        }
        Ok(())
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_dirty
            || self.field_draft.is_dirty()
            || self.dialog_pending
            || self.project_busy()
        {
            return;
        }
        if let Err(error) = self.validate_for_save() {
            self.fail(format!("Save validation failed: {error}"));
            cx.notify();
            return;
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        if document.path().is_none() {
            self.save_as(window, cx);
            return;
        }
        match document.save() {
            Ok(()) => {
                self.project_selection.clear();
                self.project_report = None;
                self.document_version += 1;
                self.project_status =
                    "Configuration saved. Check again to refresh project results.".into();
                self.disk_warning = None;
                self.status = "Configuration saved atomically. Draft and saved file match.".into();
                self.error = false;
                self.request_preview(cx);
            }
            Err(error) => self.fail(format!("Save failed; your draft is kept: {error}")),
        }
        cx.notify();
    }

    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_dirty
            || self.field_draft.is_dirty()
            || self.dialog_pending
            || self.project_busy()
            || self.document.is_none()
        {
            return;
        }
        if let Err(error) = self.validate_for_save() {
            self.fail(format!("Save validation failed: {error}"));
            cx.notify();
            return;
        }
        let document = self.document.as_ref().unwrap();
        let directory = document
            .path()
            .and_then(|path| path.parent())
            .map(|path| path.to_path_buf())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested = document
            .path()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("dprint.jsonc");
        let selection = cx.prompt_for_new_path(&directory, Some(suggested));
        let version = self.document_version;
        self.dialog_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            let selected = selection.await;
            let _ = this.update(cx, |this, cx| {
                this.dialog_pending = false;
                match selected {
                    Ok(Ok(None)) => {},
                    Ok(Ok(Some(path))) => {
                        if !dialog_is_current(version, this.document_version) {
                            this.fail("The draft changed while Save As was open. Nothing was saved; choose a destination again.".into());
                        } else if let Some(document) = this.document.as_mut() {
                            let previous_path = document.path().map(|path| path.to_path_buf());
                            match document.save_as(&path) {
                                Ok(()) => {
                                    let moved = previous_path.as_deref() != document.path();
                                    this.disk_warning = None;
                                    this.project_selection.clear();
                                    this.project_report = None;
                                    this.project_status = "Configuration saved to a new location. Check again to refresh project results.".into();
                                    this.document_version += 1;
                                    if moved {
                                        this.trusted = false;
                                        this.cancel_preview();
                                    }
                                    this.status = if moved {
                                        "Saved. Relative references are unchanged and now resolve from the new location. Review and trust before previewing.".into()
                                    } else {
                                        "Configuration saved. Draft and saved file match.".into()
                                    };
                                    this.error = false;
                                    if !moved { this.request_preview(cx); }
                                }
                                Err(error) => this.fail(format!("Save As failed; your draft is kept: {error}")),
                            }
                        }
                    }
                    Ok(Err(error)) => this.fail(format!("Cannot open Save As picker: {error}")),
                    Err(error) => this.fail(format!("Save As picker closed: {error}")),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VueSettingsGroup {
    Template,
    Script,
    Styles,
}
impl VueSettingsGroup {
    const ALL: [Self; 3] = [Self::Template, Self::Script, Self::Styles];
    fn plugin(self) -> PinnedPlugin {
        match self {
            Self::Template => PinnedPlugin::Markup,
            Self::Script => PinnedPlugin::TypeScript,
            Self::Styles => PinnedPlugin::Malva,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Template => "Template",
            Self::Script => "Script (JS / TS)",
            Self::Styles => "Styles",
        }
    }
}
fn vue_group_can_change(
    raw_dirty: bool,
    field_dirty: bool,
    project_busy: bool,
    dialog_pending: bool,
) -> bool {
    !raw_dirty && !field_dirty && !project_busy && !dialog_pending
}
fn inline_control_key(context: SchemaContext, plugin: PinnedPlugin, name: &str) -> String {
    format!("{:?}.{}.{}", context, plugin.section(), name)
}

fn setting_choice_label(value: &Value, typed: bool) -> String {
    if !typed {
        return value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
    }
    match value {
        Value::Null => "null (explicit)".into(),
        Value::Bool(value) => format!("{value} (boolean)"),
        Value::String(value) => format!(
            "{} (text)",
            serde_json::to_string(value).expect("string serialization")
        ),
        value => value.to_string(),
    }
}
fn remote_pack_availability(available: bool) -> &'static str {
    if available {
        "GitHub download available for this platform (verified pinned package)."
    } else {
        "GitHub download unavailable for this platform. An unpublished or unsupported package cannot be installed remotely; see the catalog reason above."
    }
}

fn field_annotation(field: &SchemaField, explicit: Option<&Value>) -> String {
    match explicit {
        Some(value) => format!("Explicit: {value}"),
        None => format!(
            "Inherited / default (not written) · Schema default: {}",
            field
                .default
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_else(|| "not declared".into())
        ),
    }
}

fn format_package_bytes(bytes: Option<u64>) -> String {
    match bytes {
        None => "Unknown (not recorded in the trusted catalog)".into(),
        Some(bytes) if bytes < 1024 => format!("{bytes} bytes"),
        Some(bytes) if bytes < 1024 * 1024 => {
            format!("{:.1} KiB ({bytes} bytes)", bytes as f64 / 1024.0)
        }
        Some(bytes) if bytes < 1024 * 1024 * 1024 => format!(
            "{:.1} MiB ({bytes} bytes)",
            bytes as f64 / (1024.0 * 1024.0)
        ),
        Some(bytes) => format!(
            "{:.1} GiB ({bytes} bytes)",
            bytes as f64 / (1024.0 * 1024.0 * 1024.0)
        ),
    }
}

fn package_footprint_text(metadata: Option<&Result<Option<PackFootprint>, String>>) -> String {
    match metadata {
        Some(Ok(Some(size))) => {
            let mut lines = vec![
                format!(
                    "Compressed download: {}",
                    format_package_bytes(size.download_bytes)
                ),
                format!(
                    "Installed file payload: {}",
                    format_package_bytes(size.installed_payload_bytes)
                ),
            ];
            if size.installed_payload_bytes != Some(size.installed_payload_limit_bytes) {
                lines.push(format!(
                    "Installed payload limit: {}",
                    format_package_bytes(Some(size.installed_payload_limit_bytes))
                ));
            }
            lines.push(format!(
                "New staging payload limit: {}",
                format_package_bytes(Some(size.staging_payload_limit_bytes))
            ));
            lines.join("\n")
        }
        Some(Ok(None)) => {
            "Package sizes unknown: no archive metadata is recorded for this target.".into()
        }
        Some(Err(error)) => {
            format!("Package size metadata error: {error}. No size estimate is shown.")
        }
        None => "Package size metadata is not yet available. Refresh / verify to check.".into(),
    }
}

fn pack_matches_preview(language: SourceLanguage, pack_id: &str) -> bool {
    language.pack_id() == Some(pack_id)
}

fn pack_mutation_affects_preview(
    language: SourceLanguage,
    pack_id: &str,
    action: &PackAction,
) -> bool {
    !matches!(action, PackAction::Refresh) && pack_matches_preview(language, pack_id)
}

fn native_language(language: SourceLanguage) -> Option<NativeLanguage> {
    match language {
        SourceLanguage::Rust => Some(NativeLanguage::Rust),
        SourceLanguage::Go => Some(NativeLanguage::Go),
        SourceLanguage::Dart => Some(NativeLanguage::Dart),
        SourceLanguage::Vue => Some(NativeLanguage::Vue),
        _ => None,
    }
}

fn has_unsaved_changes(document_dirty: bool, raw_dirty: bool) -> bool {
    document_dirty || raw_dirty
}

// A native dialog is asynchronous. The entire draft, including unapplied raw
// text and successful saves, must still be the version the dialog reviewed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportClose {
    None,
    Picker,
    Reading,
}

#[derive(Default)]
struct ImportOperation {
    generation: u64,
    active: Option<(u64, ImportClose, bool)>,
}
impl ImportOperation {
    fn begin(&mut self) -> u64 {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("import operation IDs exhausted");
        self.active = Some((self.generation, ImportClose::Picker, false));
        self.generation
    }
    fn begin_read(&mut self, id: u64) -> bool {
        match self.active.as_mut() {
            Some((current, phase, false)) if *current == id => {
                *phase = ImportClose::Reading;
                true
            }
            _ => false,
        }
    }
    fn finish(&mut self, id: u64) -> Option<bool> {
        if self.active.is_some_and(|(current, _, _)| current == id) {
            self.active.take().map(|(_, _, cancelled)| cancelled)
        } else {
            None
        }
    }
    fn cancel_for_close(&mut self) -> ImportClose {
        match self.active.as_mut() {
            Some((_, ImportClose::Picker, cancelled)) => {
                *cancelled = true;
                ImportClose::Picker
            }
            Some(_) => {
                self.active = None;
                ImportClose::Reading
            }
            None => ImportClose::None,
        }
    }
}

fn rustfmt_import_allowed(
    has_document: bool,
    pending_input: bool,
    busy: bool,
    dialog_pending: bool,
) -> bool {
    has_document && !pending_input && !busy && !dialog_pending
}

fn rustfmt_import_is_current(
    opened: u64,
    current: u64,
    pending_input: bool,
    busy: bool,
    rust_selected: bool,
    cancelled: bool,
) -> bool {
    dialog_is_current(opened, current) && !pending_input && !busy && rust_selected && !cancelled
}

fn dialog_is_current(opened_version: u64, current_version: u64) -> bool {
    opened_version == current_version
}

impl PreviewWindow {
    fn schema_context(&self) -> SchemaContext {
        if self.source.language() == SourceLanguage::Vue {
            SchemaContext::ManagedVue
        } else {
            SchemaContext::Standalone
        }
    }

    fn plugin(&self) -> PinnedPlugin {
        if self.source.language() == SourceLanguage::Vue {
            self.vue_settings.plugin()
        } else {
            self.source.language().plugin()
        }
    }

    fn selected_schema_field(&self) -> Option<SchemaField> {
        self.schema
            .as_ref()
            .ok()?
            .fields(self.plugin())
            .iter()
            .find(|field| Some(&field.name) == self.selected_field.as_ref())
            .cloned()
    }

    fn ensure_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_editor.is_some() {
            return;
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search this tab…"));
        self.subscriptions
            .push(cx.subscribe(&search, |this, editor, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search = editor.read(cx).value().to_string();
                    cx.notify();
                }
            }));
        self.search_editor = Some(search);
    }

    fn sync_source(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.request_preview(cx);
    }

    fn inline_editor(
        &mut self,
        field: &SchemaField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let context = self.schema_context();
        let key = inline_control_key(context, self.plugin(), &field.name);
        let baseline = self
            .document
            .as_ref()
            .map(|doc| {
                field_input_text_in_context(field, doc, self.plugin(), self.schema_context())
            })
            .unwrap_or_default();
        if let Some(editor) = self.inline_editors.get(&key) {
            if !(self.selected_field.as_ref() == Some(&field.name) && self.field_draft.is_dirty())
                && editor.read(cx).value().as_ref() != baseline
            {
                editor.update(cx, |editor, cx| editor.set_value(baseline, window, cx));
            }
            return editor.clone();
        }
        let editor = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Value");
            state.set_value(baseline, window, cx);
            state
        });
        let field = field.clone();
        let plugin = self.plugin();
        self.subscriptions.push(cx.subscribe_in(
            &editor,
            window,
            move |this, editor, event, window, cx| {
                if this.plugin() != plugin || this.schema_context() != context {
                    return;
                }
                if matches!(event, InputEvent::Change) {
                    let text = editor.read(cx).value().to_string();
                    let baseline = this
                        .document
                        .as_ref()
                        .map(|doc| field_input_text_in_context(&field, doc, plugin, context))
                        .unwrap_or_default();
                    // Programmatic synchronization is not a pending user edit.
                    if text == baseline && this.selected_field.as_ref() != Some(&field.name) {
                        return;
                    }
                    if this.selected_field.as_ref() != Some(&field.name) {
                        if !this.allow_field_transition(cx) {
                            editor.update(cx, |editor, cx| editor.set_value(baseline, window, cx));
                            return;
                        }
                        this.selected_field = Some(field.name.clone());
                        this.field_draft.reset(baseline);
                    }
                    this.field_editor = Some(editor.clone());
                    if this.field_draft.text != text {
                        this.field_draft.text = text;
                        this.project_selection.clear();
                        this.project_report = None;
                        this.document_version += 1;
                        cx.notify();
                    }
                } else if matches!(event, InputEvent::PressEnter { .. }) {
                    let baseline = this
                        .document
                        .as_ref()
                        .map(|doc| field_input_text_in_context(&field, doc, plugin, context))
                        .unwrap_or_default();
                    let text = editor.read(cx).value().to_string();
                    if !prepare_inline_apply(
                        &mut this.selected_field,
                        &mut this.field_draft,
                        &field.name,
                        baseline,
                        text,
                    ) {
                        this.allow_field_transition(cx);
                        return;
                    }
                    this.field_editor = Some(editor.clone());
                    this.apply_field(window, cx);
                }
            },
        ));
        self.inline_editors.insert(key, editor.clone());
        editor
    }

    fn inline_select(
        &mut self,
        field: &SchemaField,
        values: Vec<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Vec<String>>> {
        let context = self.schema_context();
        let key = inline_control_key(context, self.plugin(), &field.name);
        let explicit = self
            .document
            .as_ref()
            .and_then(|doc| {
                field
                    .state_in_context(doc, self.plugin(), self.schema_context())
                    .ok()
            })
            .and_then(|state| state.explicit);
        let typed_labels = values.iter().any(Value::is_null)
            || (values.iter().any(Value::is_string)
                && values.iter().any(|value| !value.is_string()));
        let label = |value: &Value| setting_choice_label(value, typed_labels);
        let selected = explicit.as_ref().map(label);
        if let Some(select) = self.inline_selects.get(&key) {
            if select.read(cx).selected_value() != selected.as_ref() {
                select.update(cx, |select, cx| {
                    if let Some(value) = &selected {
                        select.set_selected_value(value, window, cx);
                    } else {
                        select.set_selected_index(None, window, cx);
                    }
                });
            }
            return select.clone();
        }
        let labels: Vec<String> = values.iter().map(label).collect();
        let select = cx.new(|cx| {
            let mut state = SelectState::new(labels.clone(), None, window, cx);
            if let Some(value) = &selected {
                state.set_selected_value(value, window, cx);
            }
            state
        });
        let name = field.name.clone();
        let plugin = self.plugin();
        self.subscriptions.push(cx.subscribe_in(
            &select,
            window,
            move |this, _, event, window, cx| {
                if this.plugin() != plugin || this.schema_context() != context {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                if let Some(index) = value
                    .as_ref()
                    .and_then(|value| labels.iter().position(|label| label == value))
                {
                    this.set_inline_field(&name, Some(values[index].clone()), window, cx);
                }
            },
        ));
        self.inline_selects.insert(key, select.clone());
        select
    }

    fn set_inline_field(
        &mut self,
        name: &str,
        value: Option<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.allow_field_transition(cx) {
            return;
        }
        self.selected_field = Some(name.to_owned());
        self.field_editor = self
            .inline_editors
            .get(&inline_control_key(
                self.schema_context(),
                self.plugin(),
                name,
            ))
            .cloned();
        self.set_field(value, window, cx);
    }

    fn sync_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_field.is_none() {
            self.field_draft.reset(String::new());
            self.field_editor = None;
            return;
        }
        let text = self
            .selected_schema_field()
            .and_then(|field| {
                self.document.as_ref().map(|doc| {
                    field_input_text_in_context(&field, doc, self.plugin(), self.schema_context())
                })
            })
            .unwrap_or_default();
        self.field_draft.reset(text.clone());
        if let Some(editor) = &self.field_editor {
            editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
        }
    }

    fn allow_field_transition(&mut self, cx: &mut Context<Self>) -> bool {
        if self.field_draft.is_dirty() {
            self.fail("Apply or discard the pending value before changing settings, category, language or applying raw JSONC.".into());
            cx.notify();
            false
        } else {
            true
        }
    }

    fn set_field(&mut self, value: Option<Value>, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_dirty {
            self.fail("Apply or discard raw changes first. The pending value is kept.".into());
            cx.notify();
            return;
        }
        let Some(field) = self.selected_schema_field() else {
            return;
        };
        let plugin = self.plugin();
        let Some(document) = &self.document else {
            return;
        };
        let Some(catalog) = self.schema.as_ref().ok() else {
            return;
        };
        if let Err(error) = catalog.available_in_context(document, plugin, self.schema_context()) {
            self.fail(error);
            cx.notify();
            return;
        }
        if let Some(value) = &value
            && let Err(error) = field.validate(value)
        {
            self.fail(error);
            cx.notify();
            return;
        }
        if self.change_setting(&field, value, window, cx) {
            self.sync_field(window, cx);
        }
    }

    fn apply_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.selected_schema_field() else {
            return;
        };
        let Some(editor) = &self.field_editor else {
            return;
        };
        match field.parse_input(editor.read(cx).value().as_ref()) {
            Ok(value) => self.set_field(Some(value), window, cx),
            Err(error) => {
                self.fail(error);
                cx.notify();
            }
        }
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_inputs(window, cx);
        if let Some(editor) = &self.search_editor {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
    }

    fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let plugin = self.plugin();
        let availability = self
            .schema
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|schema| {
                self.document
                    .as_ref()
                    .ok_or_else(|| "No configuration loaded".into())
                    .and_then(|doc| schema.available_in_context(doc, plugin, self.schema_context()))
            });
        let disabled = self.raw_dirty || availability.is_err() || self.project_busy();
        let fields = self
            .schema
            .as_ref()
            .map(|schema| schema.fields(plugin).to_vec())
            .unwrap_or_default();
        let tabs: Vec<String> = setting_categories(&fields)
            .into_iter()
            .map(str::to_owned)
            .collect();
        if !tabs.contains(&self.group) {
            self.group = tabs.first().cloned().unwrap_or_default();
        }
        let mut visible: Vec<_> = fields
            .into_iter()
            .filter(|field| field.category() == self.group && field.matches(&self.search))
            .collect();
        visible.sort_by_key(|field| (field.subgroup_label(), field.name.clone()));
        let mut rows = Vec::new();
        let mut previous_section = None;
        for (index, field) in visible.iter().enumerate() {
            let section = field.subgroup_label();
            if previous_section.as_ref() != Some(&section) {
                if !section.is_empty() {
                    rows.push(
                        div()
                            .pt_3()
                            .pb_1()
                            .mt_2()
                            .when(previous_section.is_some(), |row| {
                                row.border_t_1().border_color(cx.theme().border)
                            })
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(section.clone())
                            .into_any_element(),
                    );
                }
                previous_section = Some(section);
            }
            let state = self
                .document
                .as_ref()
                .ok_or_else(|| "No document".to_owned())
                .and_then(|doc| field.state_in_context(doc, plugin, self.schema_context()));
            let explicit = state.as_ref().ok().and_then(|state| state.explicit.clone());
            let pending =
                self.field_draft.is_dirty() && self.selected_field.as_ref() == Some(&field.name);
            let blocked = disabled || state.is_err() || (self.field_draft.is_dirty() && !pending);
            let mut controls = div().flex().items_center().gap_1().flex_wrap();
            match &field.kind {
                FieldKind::Boolean => {
                    if let Some(checked) = explicit.as_ref().and_then(Value::as_bool) {
                        let name = field.name.clone();
                        controls = controls.child(
                            Switch::new(("boolean", index))
                                .small()
                                .checked(checked)
                                .label(if checked { "true" } else { "false" })
                                .accessibility_label(field.name.clone())
                                .disabled(blocked)
                                .on_change(cx.listener(move |this, value: &bool, window, cx| {
                                    this.set_inline_field(
                                        &name,
                                        Some(Value::Bool(*value)),
                                        window,
                                        cx,
                                    )
                                })),
                        );
                    } else {
                        // Unset is a third state, not false. Offer either explicit value directly.
                        let select = self.inline_select(
                            field,
                            vec![Value::Bool(false), Value::Bool(true)],
                            window,
                            cx,
                        );
                        controls = controls.child(
                            Select::new(&select)
                                .w(px(170.0))
                                .small()
                                .placeholder("Inherit / default")
                                .accessibility_label(field.name.clone())
                                .disabled(blocked),
                        );
                    }
                }
                FieldKind::Enum(choices) => {
                    let select = self.inline_select(
                        field,
                        choices.iter().map(|choice| choice.value.clone()).collect(),
                        window,
                        cx,
                    );
                    controls = controls.child(
                        Select::new(&select)
                            .w(px(170.0))
                            .small()
                            .placeholder("Inherit / default")
                            .accessibility_label(field.name.clone())
                            .disabled(blocked),
                    );
                }
                FieldKind::NullableNumber(_) => {
                    let editor = self.inline_editor(field, window, cx);
                    let name = field.name.clone();
                    controls = controls
                        .child(
                            div().w(px(130.0)).child(
                                Input::new(&editor)
                                    .disabled(blocked)
                                    .aria_label(format!("Number or null for {}", field.name)),
                            ),
                        )
                        .child(
                            Button::new(("explicit-null", index))
                                .small()
                                .label("null")
                                .tooltip("Write explicit null, not inherit/default")
                                .disabled(blocked || pending)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_inline_field(&name, Some(Value::Null), window, cx)
                                })),
                        )
                        .child(div().text_xs().child("number / null"));
                }
                FieldKind::Const(value) => {
                    let select = self.inline_select(field, vec![value.clone()], window, cx);
                    controls = controls.child(
                        Select::new(&select)
                            .w(px(170.0))
                            .small()
                            .placeholder("Inherit / default")
                            .accessibility_label(field.name.clone())
                            .disabled(blocked),
                    );
                }
                FieldKind::Raw { reason } => {
                    controls =
                        controls.child(div().text_xs().child(format!("Edit in JSONC: {reason}")));
                }
                _ => {
                    let editor = self.inline_editor(field, window, cx);
                    controls = controls.child(
                        div().w(px(160.0)).child(
                            Input::new(&editor)
                                .disabled(blocked)
                                .aria_label(format!("Value for {}", field.name)),
                        ),
                    )
                }
            }
            let name = field.name.clone();
            let annotation = field_annotation(field, explicit.as_ref());
            let help = field_help(field, explicit.as_ref());
            let mut row = div().id(("setting-row", index)).aria_label(field.name.clone()).aria_description(help.clone()).flex().flex_col().gap_1().py_2()
                .child(div().flex().items_center().gap_3()
                    .child(div().id(("setting-label", index)).flex_1().min_w_0().text_sm().child(field.short_label()).tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx)))
                    .child(controls)
                    .child(Button::new(("reset", index)).small().label("Inherit").tooltip("Remove explicit value; inherit configuration or plugin default")
                        .disabled(blocked || pending || explicit.is_none()).on_click(cx.listener(move |this, _, window, cx| this.set_inline_field(&name, None, window, cx)))))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(annotation));
            if pending {
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().text_xs().child("Pending input"))
                        .child(
                            Button::new(("apply", index))
                                .small()
                                .label("Apply")
                                .disabled(blocked)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.apply_field(window, cx)),
                                ),
                        )
                        .child(
                            Button::new(("discard", index))
                                .small()
                                .label("Discard")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.sync_field(window, cx);
                                    this.project_selection.clear();
                                    this.project_report = None;
                                    this.document_version += 1;
                                    cx.notify();
                                })),
                        ),
                );
            }
            if let Some(error) = state
                .as_ref()
                .err()
                .cloned()
                .or_else(|| state.ok().and_then(|state| state.validation_error))
            {
                row = row.child(div().text_xs().text_color(cx.theme().danger).child(error));
            }
            rows.push(row.into_any_element());
        }
        let selected_tab = tabs
            .iter()
            .position(|group| group == &self.group)
            .unwrap_or(0);
        div()
            .id("settings-pane")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(360.0))
            .h_full()
            .gap_2()
            .child(div().text_lg().child("Settings"))
            .when(self.source.language() == SourceLanguage::Go, |panel| panel.child(div().text_sm().child("Go uses gofmt's canonical style. There are no style controls; CodeStyles does not expose gofmt rewrite or simplify flags.")))
            .when(self.source.language() == SourceLanguage::Dart, |panel| panel.child(div().text_sm().child("Dart / Flutter source formatting: languageVersion, pageWidth, trailingCommas. No Flutter SDK, analyzer, pub tooling or widget generation. Dart preview uses plain text when no highlighting grammar is available.")))
            .when(self.source.language() == SourceLanguage::Vue, |panel| panel.child(div().text_xs().child("Template → markup (markup_fmt) · Script → shared typescript settings (also affect JS/TS previews) · Styles → malva. JSONC and unknown options are preserved. Settings can be edited before installing support; download availability follows the language-pack catalog.")))
            .when(self.source.language() == SourceLanguage::Vue, |panel| panel.child(div().flex().gap_2().children(VueSettingsGroup::ALL.into_iter().enumerate().map(|(index, group)| Button::new(("vue-setting-group", index)).label(group.label()).selected(self.vue_settings == group).disabled(!vue_group_can_change(self.raw_dirty, self.field_draft.is_dirty(), self.project_busy(), self.dialog_pending)).on_click(cx.listener(move |this, _, window, cx| {
                if vue_group_can_change(this.raw_dirty, this.field_draft.is_dirty(), this.project_busy(), this.dialog_pending) && this.allow_field_transition(cx) { this.vue_settings = group; this.selected_field = None; this.group.clear(); this.sync_field(window, cx); cx.notify(); }
            }))))))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} settings", visible.len())),
            )
            .child(
                Input::new(self.search_editor.as_ref().unwrap())
                    .disabled(self.field_draft.is_dirty())
                    .aria_label(format!(
                        "Search setting names and descriptions in {}",
                        self.group
                    )),
            )
            .child(
                div()
                    .id("category-scroll")
                    .overflow_x_scroll()
                    .flex_shrink_0()
                    .child(
                        TabBar::new("category-tabs")
                            .underline()
                            .menu(true)
                            .selected_index(selected_tab)
                            .children(tabs.iter().map(|group| Tab::new().label(group.clone())))
                            .on_click(cx.listener(move |this, index: &usize, _, cx| {
                                if this.allow_field_transition(cx) {
                                    this.group = tabs[*index].clone();
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .when_some(availability.err(), |panel, error| {
                panel.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(format!("{error} Use Edit JSONC.")),
                )
            })
            .child(
                div()
                    .id("settings-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(rows.is_empty(), |list| {
                        list.child(div().py_3().text_sm().child(if self.source.language() == SourceLanguage::Go { "gofmt has no configurable style settings." } else { "No matches in this tab" }))
                    })
                    .children(rows),
            )
            .into_any_element()
    }

    fn open_language_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.toolchain_controller.is_running() || self.project_busy() || self.dialog_pending {
            return;
        }
        if !self.allow_field_transition(cx) {
            return;
        }
        let view = cx.entity().downgrade();
        let selected = self.source.language();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Choose language")
                .w(px(520.0))
                .overlay_closable(true)
                .child(
                    div().flex().flex_col().gap_4().children(
                        ["Code", "Data", "Documentation"]
                            .into_iter()
                            .map(|category| {
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(div().text_sm().child(category))
                                    .child(
                                        div().flex().flex_wrap().gap_2().children(
                                            SourceLanguage::ALL
                                                .into_iter()
                                                .filter(|language| language.category() == category)
                                                .enumerate()
                                                .map(|(index, language)| {
                                                    let view = view.clone();
                                                    Button::new((category, index))
                                                        .label(language.display_name())
                                                        .selected(selected == language)
                                                        .on_click(move |_, window, cx| {
                                                            let _ = view.update(cx, |this, cx| {
                                                                if this.toolchain_controller.is_running() || this.project_busy() || this.dialog_pending || !this.allow_field_transition(cx)
                                                                {
                                                                    return;
                                                                }
                                                                this.switch_pack(language.pack_id().unwrap_or("rustfmt"), cx);
                                                                this.vue_settings = VueSettingsGroup::Template;
                                                                this.source
                                                                    .select_language(language);
                                                                this.project_report = None;
                                                                this.project_selection.clear();
                                                                this.project_status = "Language scope changed. Check the project again before formatting.".into();
                                                                this.selected_field = None;
                                                                this.field_editor = None;
                                                                this.group.clear();
                                                                this.sync_source(window, cx);
                                                                cx.notify();
                                                                window.close_dialog(cx);
                                                            });
                                                        })
                                                }),
                                        ),
                                    )
                            }),
                    ),
                )
        });
    }
}

fn project_can_start(dirty: bool, dialog: bool, running: bool, saved: bool, backend: bool) -> bool {
    !dirty && !dialog && !running && saved && backend
}

/// Paths, rather than row indexes, keep selection and focused review independent.
#[derive(Default)]
struct ProjectSelection {
    selected: BTreeSet<PathBuf>,
    focused: Option<PathBuf>,
}

impl ProjectSelection {
    fn clear(&mut self) {
        self.selected.clear();
        self.focused = None;
    }

    fn set(&mut self, path: PathBuf, checked: bool, available: &[PathBuf]) {
        if !available.contains(&path) {
            return;
        }
        if checked {
            self.selected.insert(path);
        } else {
            self.selected.remove(&path);
        }
    }

    fn paths(&self, available: &[PathBuf]) -> Vec<PathBuf> {
        available
            .iter()
            .filter(|path| self.selected.contains(*path))
            .cloned()
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Styles,
    Project,
    Integration,
    Toolchains,
}

impl PreviewWindow {
    fn project_busy(&self) -> bool {
        self.project_controller
            .as_ref()
            .is_some_and(ProjectController::is_running)
    }

    fn project_ready(&self) -> bool {
        !self.toolchain_controller.is_running()
            && project_can_start(
                self.dirty(),
                self.dialog_pending,
                self.project_busy(),
                self.document
                    .as_ref()
                    .is_some_and(|doc| doc.path().is_some()),
                self.project_controller.is_some(),
            )
    }

    fn project_action(
        &mut self,
        action: ProjectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_ready() {
            return;
        }
        let review = if let ProjectAction::FormatSelected(paths) = &action {
            let Some(report) = self.project_report.as_ref() else {
                return;
            };
            if paths.is_empty()
                || paths
                    .iter()
                    .any(|path| !report.files.iter().any(|file| &file.path == path))
            {
                return;
            }
            Some(report.clone())
        } else if matches!(action, ProjectAction::FormatProject) {
            // The UI only applies paths from an explicitly reviewed report.
            return;
        } else {
            None
        };
        let document = self.document.as_ref().unwrap();
        let root = document.path().unwrap().parent().unwrap().to_path_buf();
        let version = self.document_version;
        let native = native_language(self.source.language());
        let native_project = native.is_some();
        let writes = matches!(
            action,
            ProjectAction::FormatProject | ProjectAction::FormatSelected(_)
        );
        let scope = match &action {
            ProjectAction::FormatSelected(paths) => format!(
                "{} selected files:\n{}",
                paths.len(),
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            _ => format!("Project directory: {}", root.display()),
        };
        let engine_scope = if native_project {
            "Only the selected language is included, using saved CodeStyles settings and its verified managed pack. Local ignore rules apply; symlinks and generated/build directories are skipped. Unsupported discovery options and nested configs are rejected. This is not mixed-language formatting; Vue does not read external src or arbitrary custom block languages."
        } else {
            "Uses dprint's normal nested configuration discovery. Review this project's nested configs, extends and plugin sources: they may download resources and execute process plugins with your account's permissions."
        };
        let explanation = format!(
            "{scope}\n\n{engine_scope} {}",
            if writes {
                "This operation writes source files. Review or back up your work first. Cancellation stops remaining work; files already written are not rolled back."
            } else if native_project {
                "This operation checks only the selected language without writing it. It uses manifest-verified formatters, not executable commands from the project."
            } else {
                "This operation does not intentionally format source files. Plugin preparation may populate dprint's cache. Trust is not a sandbox."
            }
        );
        self.dialog_pending = true;
        let prompt = window.prompt(
            PromptLevel::Warning,
            if writes {
                "Trust project and format files?"
            } else if native_project {
                "Check selected-language-only project scope?"
            } else {
                "Trust project configuration and plugins?"
            },
            Some(&explanation),
            &[
                "Cancel",
                if writes {
                    "Trust and format"
                } else {
                    "Trust and continue"
                },
            ],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let accepted = prompt.await == Ok(1);
            let _ = this.update(cx, |this, cx| {
                this.dialog_pending = false;
                if !accepted { cx.notify(); return; }
                if !dialog_is_current(version, this.document_version) || !this.project_ready() || native != native_language(this.source.language()) {
                    this.project_status = "Configuration changed during confirmation. Review it again before running.".into();
                    this.project_error = true;
                    cx.notify(); return;
                }
                let document = this.document.as_ref().unwrap();
                let request = match (action, review) {
                    (ProjectAction::FormatSelected(paths), Some(report)) if native_project =>
                        ProjectRequest::from_native_review(root, document, paths, &report, true, native.unwrap()),
                    (ProjectAction::FormatSelected(paths), Some(report)) =>
                        ProjectRequest::from_review(root, document, paths, &report, true),
                    (action, None) if native_project => ProjectRequest::new_native(root, document, action, true, native.unwrap()),
                    (action, None) => ProjectRequest::new(root, document, action, true),
                    _ => Err("Check the project again before formatting.".into()),
                };
                match request.and_then(|request| this.project_controller.as_mut().unwrap().submit(request)) {
                    Ok(_) => {
                        this.cancel_preview();
                        this.page = Page::Project;
                        this.project_selection.clear();
                        this.project_report = None;
                        this.project_error = false;
                        this.project_status = if native_project { "Running selected-language-only project operation with the verified pack…" } else { "Running bundled dprint with the saved project configuration…" }.into();
                    }
                    Err(error) => { this.project_status = error; this.project_error = true; }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn project_paths(&self) -> Vec<PathBuf> {
        self.project_report
            .as_ref()
            .map(|report| report.files.iter().map(|file| file.path.clone()).collect())
            .unwrap_or_default()
    }

    fn render_project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = !self.project_ready();
        let root = self
            .document
            .as_ref()
            .and_then(|doc| doc.path())
            .and_then(|path| path.parent());
        let root_label = root
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Save or open a configuration first.".into());
        let paths = self.project_paths();
        let selected = self.project_selection.paths(&paths).len();
        let count = self
            .project_report
            .as_ref()
            .map(|report| {
                format!(
                    "{} checked · {} changed · {} selected",
                    report.checked_files,
                    report.files.len(),
                    selected
                )
            })
            .unwrap_or_else(|| "Check the project to review formatting changes.".into());
        let focused = self.project_report.as_ref().and_then(|report| {
            report
                .files
                .iter()
                .find(|file| Some(&file.path) == self.project_selection.focused.as_ref())
        });
        let diff_title = focused
            .map(|file| format!("{} · unified diff", file.path.display()))
            .unwrap_or_else(|| "File review".into());
        let diff = focused.map(|file| file.diff.as_str()).unwrap_or_else(|| {
            if self.project_report.as_ref().is_some_and(|report| {
                report.files.is_empty() && !matches!(report.action, ProjectAction::PreparePlugins)
            }) {
                "No remaining formatting differences."
            } else {
                "Choose a changed file to inspect its diff, then select files to format."
            }
        });
        let rows: Vec<_> = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let label = root
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(path)
                    .display()
                    .to_string();
                let select_path = path.clone();
                let focus_path = path.clone();
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_1()
                    .child(
                        Checkbox::new(("project-selected", index))
                            .accessibility_label(format!("Select {label} for formatting"))
                            .checked(self.project_selection.selected.contains(path))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, checked, _, cx| {
                                if this.project_ready() {
                                    this.project_selection.set(
                                        select_path.clone(),
                                        *checked,
                                        &this.project_paths(),
                                    );
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new(("project-file", index))
                            .label(label)
                            .small()
                            .selected(self.project_selection.focused.as_ref() == Some(path))
                            .disabled(self.project_busy() || self.dialog_pending)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.project_busy()
                                    && !this.dialog_pending
                                    && this.project_paths().contains(&focus_path)
                                {
                                    this.project_selection.focused = Some(focus_path.clone());
                                    cx.notify();
                                }
                            })),
                    )
            })
            .collect();
        div().flex().flex_col().flex_1().min_h_0().gap_3()
            .child(div().text_sm().child(format!("Project root · {root_label}")))
            .when(native_language(self.source.language()).is_some(), |panel| panel.child(div().text_sm().child(format!("{}-only scope · local ignore files · no symlinks or unsupported nested configuration. Not mixed-language discovery. Vue external src/custom block languages are rejected.", self.source.language().display_name()))))
            .when(self.dirty(), |panel| panel.child(div().text_color(cx.theme().warning).child("Save or discard pending configuration edits before checking or formatting.")))
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("project-check").label("Check project…").disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.project_action(ProjectAction::Check, window, cx))))
                .child(Button::new("project-prepare").label("Prepare plugins…").disabled(disabled || native_language(self.source.language()).is_some()).on_click(cx.listener(|this, _, window, cx| this.project_action(ProjectAction::PreparePlugins, window, cx))))
                .child(Button::new("project-format-selected").label(format!("Format selected ({selected})…")).disabled(disabled || selected == 0).on_click(cx.listener(|this, _, window, cx| {
                    let paths = this.project_selection.paths(&this.project_paths());
                    this.project_action(ProjectAction::FormatSelected(paths), window, cx);
                })))
                .child(Button::new("project-cancel").label("Cancel").disabled(!self.project_busy()).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(controller) = &mut this.project_controller { controller.cancel(); }
                    this.project_status = "Cancellation requested. Waiting for the formatter to stop; files already written are not rolled back.".into(); cx.notify();
                }))))
            .child(div().text_sm().text_color(if self.project_error { cx.theme().danger } else { cx.theme().muted_foreground }).child(self.project_status.clone()))
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().text_sm().child(count))
                .child(Button::new("project-select-all").label("Select all").small().disabled(disabled || paths.is_empty()).on_click(cx.listener(|this, _, _, cx| {
                    if this.project_ready() { this.project_selection.selected = this.project_paths().into_iter().collect(); cx.notify(); }
                })))
                .child(Button::new("project-select-none").label("Select none").small().disabled(disabled || selected == 0).on_click(cx.listener(|this, _, _, cx| {
                    if this.project_ready() { this.project_selection.selected.clear(); cx.notify(); }
                }))))
            .child(div().flex().flex_1().min_h_0().gap_3()
                .child(div().id("project-file-list").w(px(280.0)).flex_shrink_0().overflow_scroll().border_1().border_color(cx.theme().border).rounded_lg().children(rows))
                .child(code_pane("project-file-diff", diff_title, diff, cx)))
            .when_some(self.project_report.as_ref().filter(|report| !report.diagnostics.trim().is_empty()).map(|report| report.diagnostics.clone()), |panel, diagnostics| panel.child(div().id("project-diagnostics").max_h(px(70.0)).overflow_y_scroll().text_sm().child(diagnostics)))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Saved config · point-in-time check · selected files are checked again before writing and the project is verified afterward."))
            .into_any_element()
    }

    fn render_integration(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.source.language() == SourceLanguage::Rust {
            return self.render_rust_integration(cx);
        }
        if !self.source.language().is_bundled() {
            return div().flex().flex_col().gap_3()
                .child(div().text_lg().child(format!("{} editor & CI", self.source.language().display_name())))
                .child(div().text_sm().child("This optional language uses a verified managed pack. Installing it does not change editor settings or add project plugin URLs. Configure external editor/CI tools explicitly; the bundled offline dprint config alone does not provide this language."))
                .child(div().text_sm().child(match self.source.language() {
                    SourceLanguage::Go => "Go uses canonical gofmt with no style settings. Configure your normal Go editor tooling separately.",
                    SourceLanguage::Dart => "Dart / Flutter source uses the pinned Dart formatter. This does not install Flutter or run pub/analyzer commands. The three supported options are languageVersion, pageWidth, and trailingCommas.",
                    _ => "Vue support is Vue-only SFC formatting with pinned markup_fmt / Malva and bundled script formatters. External src and unsupported custom block languages fail closed. It is not a Vue build system or arbitrary embedded-language runner.",
                })).into_any_element();
        }
        let guide = self
            .executable
            .as_ref()
            .ok_or_else(|| {
                "Bundled executable is unavailable. Verify the resource bundle first.".to_owned()
            })
            .and_then(|path| integration::guidance(path));
        let shell = integration::CommandShell::native();
        let project = self
            .executable
            .as_ref()
            .ok_or_else(|| "Bundled executable is unavailable.".to_owned())
            .and_then(|path| {
                integration::project_guidance(
                    path,
                    self.document.as_ref().and_then(|doc| doc.path()),
                    self.dirty(),
                    shell,
                )
            });
        let project_status = project
            .as_ref()
            .map(|guide| guide.status.clone())
            .unwrap_or_else(Clone::clone);
        let check_command = project
            .as_ref()
            .ok()
            .and_then(|guide| guide.check_command.clone());
        let format_command = project
            .as_ref()
            .ok()
            .and_then(|guide| guide.format_command.clone());
        let settings = guide.as_ref().ok().map(|guide| guide.settings_json.clone());
        let (text, path) = match guide {
            Ok(guide) => (
                format!(
                    "{}\n\nUser settings JSON:\n{}\n\n{}",
                    guide.editor_steps, guide.settings_json, guide.ci_instructions
                ),
                Some(guide.executable_path),
            ),
            Err(error) => (error, None),
        };
        div().flex().flex_col().flex_1().min_h_0().gap_3()
            .child(div().text_lg().child("Editor & CI"))
            .child(div().text_sm().child(project_status))
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("copy-project-check").label("Copy check command").disabled(check_command.is_none() || self.dialog_pending || self.project_busy()).on_click(cx.listener(move |this, _, _, cx| {
                    if !this.dirty() && let Some(command) = &check_command {
                        cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                        this.status = format!("Saved-project check command copied for {}.", shell.label()); cx.notify();
                    }
                })))
                .child(Button::new("copy-project-format").label("Copy format command").disabled(format_command.is_none() || self.dialog_pending || self.project_busy()).on_click(cx.listener(move |this, _, _, cx| {
                    if !this.dirty() && let Some(command) = &format_command {
                        cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                        this.status = format!("Format command copied for {}. Running it writes project source files.", shell.label()); cx.notify();
                    }
                })))
                .child(Button::new("copy-ci-step").label("Copy CI step").on_click(cx.listener(|this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(integration::CI_STEP.into()));
                    this.status = "CI step copied. Use after checkout with a committed, discoverable dprint configuration.".into(); cx.notify();
                }))))
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("copy-dprint-path").label("Copy dprint path").disabled(path.is_none()).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(path) = &path {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                        this.status = "Bundled dprint path copied. Paste it into VS Code user settings.".into(); cx.notify();
                    }
                })))
                .child(Button::new("copy-editor-settings").label("Copy settings").disabled(settings.is_none()).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(settings) = &settings { cx.write_to_clipboard(ClipboardItem::new_string(settings.clone())); this.status = "Settings JSON copied. Merge it into VS Code user settings, preserving existing entries.".into(); cx.notify(); }
                })))
                .child(Button::new("open-dprint-extension").label("VS Code extension").on_click(|_, _, cx| cx.open_url(integration::EXTENSION_URL))))
            .child(code_pane("integration-guide", "Editor & CI setup · no settings changed automatically".into(), &text, cx))
            .into_any_element()
    }
}

impl PreviewWindow {
    fn switch_pack(&mut self, id: &str, cx: &mut Context<Self>) {
        if id == self.toolchain_controller.pack_id()
            || self.toolchain_controller.is_running()
            || self.project_busy()
            || self.dialog_pending
        {
            return;
        }
        match ToolchainController::for_pack(id) {
            Ok(controller) => {
                self.toolchain_controller = controller;
                self.pack_snapshot = None;
                self.pack_error = false;
                self.pack_action(PackAction::Refresh, cx);
            }
            Err(error) => {
                self.pack_status = error;
                self.pack_error = true;
                cx.notify();
            }
        }
    }

    fn pack_action(&mut self, action: PackAction, cx: &mut Context<Self>) {
        if self.toolchain_controller.is_running() || self.project_busy() || self.dialog_pending {
            return;
        }
        if pack_mutation_affects_preview(
            self.source.language(),
            self.toolchain_controller.pack_id(),
            &action,
        ) {
            self.cancel_preview();
            self.project_report = None;
            self.project_selection.clear();
        }
        self.pack_status = match &action {
            PackAction::Refresh => "Verifying installed language-pack status…",
            PackAction::Install => "Starting the pinned GitHub download…",
            PackAction::InstallLocal(_) => {
                "Verifying the selected archive against the built-in package digest…"
            }
            PackAction::Uninstall => "Removing only the selected managed language pack…",
        }
        .into();
        self.pack_error = false;
        if let Err(error) = self.toolchain_controller.submit(action) {
            self.pack_status = error;
            self.pack_error = true;
        }
        cx.notify();
    }

    fn choose_local_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.toolchain_controller.is_running() || self.project_busy() || self.dialog_pending {
            return;
        }
        let pack_id = self.toolchain_controller.pack_id();
        self.dialog_pending = true;
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                "Install selected language pack matching the built-in verified digest".into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = match selection.await {
                Ok(Ok(Some(paths))) => Ok(paths.into_iter().next()),
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(format!("Cannot select language pack: {error}")),
                Err(error) => Err(format!("File picker closed: {error}")),
            };
            let _ = this.update(cx, |this, cx| {
                if this.toolchain_controller.pack_id() != pack_id {
                    return;
                }
                this.dialog_pending = false;
                match selected {
                    Ok(Some(path)) => this.pack_action(PackAction::InstallLocal(path), cx),
                    Ok(None) => {
                        this.pack_status =
                            "Selection cancelled. Installed language packs are unchanged.".into();
                        this.pack_error = false;
                    }
                    Err(error) => {
                        this.pack_status = error;
                        this.pack_error = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn confirm_uninstall(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.toolchain_controller.is_running() || self.project_busy() || self.dialog_pending {
            return;
        }
        let pack_id = self.toolchain_controller.pack_id();
        self.dialog_pending = true;
        let location = self
            .pack_snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.display().to_string())
            .unwrap_or_else(|| "the dedicated managed directory".into());
        let explanation = format!(
            "Remove only the managed {} pack under {location}? Other language packs, bundled languages, project source files, and saved configuration are not removed. Formatting this language will require reinstalling its verified pack.",
            self.toolchain_controller.pack_id()
        );
        let prompt = window.prompt(
            PromptLevel::Warning,
            "Uninstall selected language support?",
            Some(&explanation),
            &["Cancel", "Uninstall selected pack"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let accepted = prompt.await == Ok(1);
            let _ = this.update(cx, |this, cx| {
                if this.toolchain_controller.pack_id() != pack_id {
                    return;
                }
                this.dialog_pending = false;
                if accepted {
                    this.pack_action(PackAction::Uninstall, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_toolchains(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.toolchain_controller.is_running();
        let blocked = busy || self.project_busy() || self.dialog_pending;
        let footprint = package_footprint_text(
            self.pack_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.footprint),
        );
        let footprint_error = self
            .pack_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.footprint.is_err());
        let (state, root, publication, remote, local, installed) = match &self.pack_snapshot {
            Some(snapshot) => (
                match &snapshot.status {
                    ToolchainStatus::Missing => "Not installed".into(),
                    ToolchainStatus::Installed { version, root } => {
                        format!("Installed · {version}\nRuntime: {}", root.display())
                    }
                    ToolchainStatus::Corrupt { reason } => format!("Needs repair · {reason}"),
                },
                snapshot.root.display().to_string(),
                snapshot.publication_note.clone(),
                snapshot.available,
                snapshot.local_available,
                !matches!(snapshot.status, ToolchainStatus::Missing),
            ),
            None => (
                "Status not yet available".into(),
                "Resolved on the worker when status is checked.".into(),
                "Remote availability is not yet verified.".into(),
                false,
                false,
                false,
            ),
        };
        div().id("language-support-content").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().gap_3()
            .child(div().text_lg().child("Language support"))
            .child(div().flex().flex_wrap().gap_2().children([("rustfmt", "Rust"), ("gofmt", "Go"), ("dartfmt", "Dart / Flutter"), ("vue", "Vue")].into_iter().enumerate().map(|(index, (id, label))| Button::new(("pack-choice", index)).label(label).selected(self.toolchain_controller.pack_id() == id).disabled(blocked).on_click(cx.listener(move |this, _, _, cx| this.switch_pack(id, cx))))))
            .child(div().p_3().border_1().border_color(cx.theme().border).rounded_lg().flex().flex_col().gap_2()
                .child(div().child(if self.controller.is_some() { "Built in · resources verified" } else { "Built in · resource verification unavailable" }))
                .child(div().text_sm().child("TypeScript / JavaScript / JSX / TSX · JSON / JSONC · Markdown · TOML · Python / Python Stub"))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Existing pinned dprint and five WASM plugins remain bundled. This page does not remove or upgrade existing project plugin references.")))
            .child(div().p_3().border_1().border_color(cx.theme().border).rounded_lg().flex().flex_col().gap_2()
                .child(div().child(format!("{} · verified optional language pack", self.toolchain_controller.pack_id())))
                .child(div().text_sm().child(state))
                .child(div().text_sm().child(format!("Managed directory: {root}")))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(publication))
                .child(div().text_sm().text_color(if remote { cx.theme().muted_foreground } else { cx.theme().warning }).child(remote_pack_availability(remote)))
                .child(div().text_sm().text_color(if footprint_error { cx.theme().warning } else { cx.theme().muted_foreground }).child(footprint))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Catalog payload sizes, not measured disk usage. New staging limit = compressed archive limits + installed payload limit; excludes receipts, filesystem overhead and existing/retained runtimes. Existing versions need additional space. Free disk space is not checked. Size metadata does not mean an unpublished download is available."))
                .child(div().flex().flex_wrap().gap_2()
                    .child(Button::new("pack-refresh").label("Refresh / verify").disabled(blocked).on_click(cx.listener(|this, _, _, cx| this.pack_action(PackAction::Refresh, cx))))
                    .child(Button::new("pack-install").label(if installed { "Repair from GitHub" } else { "Install from GitHub" }).disabled(blocked || !remote).on_click(cx.listener(|this, _, _, cx| this.pack_action(PackAction::Install, cx))))
                    .child(Button::new("pack-local").label("Install verified local pack…").disabled(blocked || !local).on_click(cx.listener(|this, _, window, cx| this.choose_local_pack(window, cx))))
                    .child(Button::new("pack-remove").label("Uninstall…").disabled(blocked || !installed).on_click(cx.listener(|this, _, window, cx| this.confirm_uninstall(window, cx))))
                    .child(Button::new("pack-cancel").label("Cancel").disabled(!busy).on_click(cx.listener(|this, _, _, cx| {
                        this.toolchain_controller.cancel();
                        this.pack_status = "Cancellation requested; waiting for cleanup. A previously activated version is retained unless uninstall already completed.".into(); cx.notify();
                    }))))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("No preinstalled language toolchain required. Only the built-in manifest's exact package digest is accepted, including for local archives. Unpublished releases cannot be downloaded; local installation does not imply a release was published.")))
            .child(div().id("pack-status").overflow_y_scroll().max_h(px(140.0)).text_sm().text_color(if self.pack_error { cx.theme().danger } else { cx.theme().muted_foreground }).child(self.pack_status.clone()))
            .child(div().text_sm().child("Install location: a dedicated directory in the application's installation directory (outside a macOS .app) by default; if it is not writable, the OS user-data directory is used. Archive/translocated launches cannot always be detected. Launch with CODESTYLES_TRANSIENT_LAUNCH=1 to explicitly use user data, or CODESTYLES_TOOLCHAINS_DIR to select a dedicated managed root."))
            .child(div().text_sm().child("Download and verification run in the background. Keep the application open until completion or cancellation; failed installation does not replace the previous active version."))
            .into_any_element()
    }

    fn import_rustfmt_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.raw_dirty || self.field_draft.is_dirty();
        let busy = self.project_busy() || self.toolchain_controller.is_running();
        if !rustfmt_import_allowed(self.document.is_some(), pending, busy, self.dialog_pending) {
            self.fail("Apply or discard pending editor input and wait for active operations before importing rustfmt settings.".into());
            cx.notify();
            return;
        }
        if self.source.language() != SourceLanguage::Rust {
            return;
        }
        let Some(mut document) = self.document.clone() else {
            return;
        };
        let version = self.document_version;
        self.dialog_pending = true;
        let operation = self.rustfmt_import.begin();
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import rustfmt.toml or .rustfmt.toml into the unsaved draft".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = match selection.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next().map(Ok),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(format!("Cannot select rustfmt configuration: {error}"))),
                Err(error) => Some(Err(format!("File picker closed: {error}"))),
            };
            // Cancel while the native picker was open must not start I/O.
            // A detached old operation must never clear a newer dialog.
            let may_read = this.update_in(cx, |this, _, cx| {
                if this.rustfmt_import.begin_read(operation) { return true; }
                if this.rustfmt_import.finish(operation).is_some() {
                    this.dialog_pending = false;
                    this.status = "Import cancelled. Configuration unchanged.".into();
                    this.error = false;
                    cx.notify();
                }
                false
            }).unwrap_or(false);
            if !may_read { return; }
            let imported = match selected {
                Some(Ok(path)) => Some(cx.background_executor().spawn(async move {
                    let text = codestyles::rustfmt::read_import_config(&path)?;
                    let count = codestyles::rustfmt::import_config(&mut document, &text)?;
                    Ok::<_, String>((document, count))
                }).await),
                Some(Err(error)) => Some(Err(error)),
                None => None,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(cancelled) = this.rustfmt_import.finish(operation) else { return };
                this.dialog_pending = false;
                if !rustfmt_import_is_current(version, this.document_version,
                    this.raw_dirty || this.field_draft.is_dirty(),
                    this.project_busy() || this.toolchain_controller.is_running(),
                    this.source.language() == SourceLanguage::Rust, cancelled) {
                    this.fail("Import cancelled or not applied: the draft, pending input, or active view changed. Your current configuration was kept.".into());
                } else {
                    match imported {
                        Some(Ok((document, count))) => {
                            this.cancel_preview();
                            this.document = Some(document);
                            this.document_version += 1;
                            this.project_report = None;
                            this.project_selection.clear();
                            this.project_status = "Rust settings imported. Check again to refresh project results.".into();
                            this.last_success = None;
                            this.output.clear();
                            this.sync_editor(window, cx);
                            this.sync_field(window, cx);
                            this.request_preview(cx);
                            this.status = format!("Imported {count} rustfmt setting(s) into the draft. Neither the selected TOML file nor your configuration was saved.{}", if this.trusted { "" } else { " Review and trust the configuration to enable preview." });
                            this.error = false;
                        }
                        Some(Err(error)) => this.fail(format!("Rustfmt import rejected; current configuration kept: {error}")),
                        None => { this.status = "Import cancelled. Configuration unchanged.".into(); this.error = false; }
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn render_rust_integration(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let config = self
            .document
            .as_ref()
            .ok_or_else(|| "Open a configuration first.".to_owned())
            .and_then(|doc| codestyles::rustfmt::export_config(&doc.snapshot()));
        let copy = config.as_ref().ok().cloned();
        let text =
            config.unwrap_or_else(|error| format!("Cannot export rustfmt configuration: {error}"));
        let pending = self.raw_dirty || self.field_draft.is_dirty();
        let import_disabled = !rustfmt_import_allowed(
            self.document.is_some(),
            pending,
            self.project_busy() || self.toolchain_controller.is_running(),
            self.dialog_pending,
        );
        div().flex().flex_col().flex_1().min_h_0().gap_3()
            .child(div().text_lg().child("Rust editor & CI"))
            .child(div().text_sm().child("Rust formatting is native to CodeStyles, not a dprint WASM plugin. The dprint CLI and dprint VS Code settings do not use the rustfmt section in this JSONC file."))
            .child(div().text_sm().child("Export this configuration to rustfmt.toml and configure rust-analyzer / your Rust toolchain separately. CodeStyles does not read Cargo.toml, rustfmt.toml, or .rustfmt.toml automatically. Edition and style edition are explicit; cargo fmt may select a broader module/workspace scope than CodeStyles."))
            .child(div().text_sm().child("Explicitly import rustfmt.toml / .rustfmt.toml to merge supported stable settings into this draft. Unknown options reject the whole import. Existing comments and unrelated settings are kept; no files are saved automatically."))
            .child(Button::new("import-rustfmt-config").label("Import rustfmt.toml…").disabled(import_disabled).on_click(cx.listener(|this, _, window, cx| this.import_rustfmt_config(window, cx))))
            .child(Button::new("copy-rustfmt-config").label("Copy rustfmt.toml").disabled(copy.is_none() || pending).on_click(cx.listener(move |this, _, _, cx| {
                if !this.raw_dirty && !this.field_draft.is_dirty() && let Some(config) = &copy {
                    cx.write_to_clipboard(ClipboardItem::new_string(config.clone())); this.status = "rustfmt.toml copied from the current applied draft. No project file or editor setting was changed.".into(); cx.notify();
                }
            })))
            .when(pending, |panel| panel.child(div().text_color(cx.theme().warning).child("Apply or discard pending editor input before importing or exporting.")))
            .child(code_pane("rustfmt-export", "Current applied draft · rustfmt.toml".into(), &text, cx))
            .into_any_element()
    }
}

#[derive(Default)]
struct FieldDraft {
    baseline: String,
    text: String,
}
impl FieldDraft {
    fn reset(&mut self, text: String) {
        self.baseline = text.clone();
        self.text = text;
    }
    fn is_dirty(&self) -> bool {
        self.text != self.baseline
    }
}

/// Enter explicitly applies even an unchanged displayed default. Never steal a
/// pending draft from another row (including queued keyboard events).
fn prepare_inline_apply(
    selected: &mut Option<String>,
    draft: &mut FieldDraft,
    name: &str,
    baseline: String,
    text: String,
) -> bool {
    if selected.as_deref() != Some(name) {
        if draft.is_dirty() {
            return false;
        }
        *selected = Some(name.to_owned());
        draft.reset(baseline);
    }
    draft.text = text;
    true
}

fn visible_diff(baseline: &str, draft: &str) -> String {
    line_diff(baseline, draft)
        .iter()
        .map(|line| {
            let sign = match line.kind {
                DiffKind::Unchanged => " ",
                DiffKind::Removed => "−",
                DiffKind::Added => "+",
            };
            let ending = if line.kind == DiffKind::Unchanged {
                ""
            } else if line.text.ends_with("\r\n") {
                " ⟦CRLF⟧"
            } else if line.text.ends_with('\n') {
                " ⟦LF⟧"
            } else {
                " ⟦no newline⟧"
            };
            format!(
                "{sign} {}{ending}",
                line.text.trim_end_matches(['\r', '\n'])
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn field_help(field: &SchemaField, explicit: Option<&Value>) -> String {
    format!(
        "{}\n{}\nExplicit value: {}\nSchema default: {}. Unset values may inherit a different value.",
        field.name,
        field.description,
        explicit
            .map(Value::to_string)
            .unwrap_or_else(|| "unset".into()),
        field
            .default
            .as_ref()
            .map(Value::to_string)
            .unwrap_or_else(|| "not declared".into())
    )
}

#[cfg(test)]
fn field_input_text(
    field: &SchemaField,
    document: &ConfigDocument,
    plugin: PinnedPlugin,
) -> String {
    field_input_text_in_context(field, document, plugin, SchemaContext::Standalone)
}

fn field_input_text_in_context(
    field: &SchemaField,
    document: &ConfigDocument,
    plugin: PinnedPlugin,
    context: SchemaContext,
) -> String {
    field
        .state_in_context(document, plugin, context)
        .ok()
        .and_then(|state| state.explicit.or(state.schema_default))
        .map(|value| match value {
            Value::String(text) => text,
            value => value.to_string(),
        })
        .unwrap_or_default()
}

fn comparison_is_current(
    last_success: Option<u64>,
    revision: u64,
    pending: bool,
    trusted: bool,
    raw_dirty: bool,
) -> bool {
    last_success == Some(revision) && !pending && trusted && !raw_dirty
}

#[cfg(test)]
fn highlight_language(language: SourceLanguage) -> &'static str {
    match language {
        SourceLanguage::TypeScript => "typescript",
        SourceLanguage::JavaScript | SourceLanguage::Jsx => "javascript",
        SourceLanguage::Tsx => "tsx",
        SourceLanguage::Json | SourceLanguage::Jsonc => "json",
        SourceLanguage::Markdown => "markdown",
        SourceLanguage::Toml => "toml",
        SourceLanguage::Python | SourceLanguage::PythonStub => "python",
        SourceLanguage::Rust => "rust",
        SourceLanguage::Go => "go",
        SourceLanguage::Dart => "text",
        SourceLanguage::Vue => "html",
    }
}

fn code_pane(id: &'static str, title: String, source: &str, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .border_1()
        .border_color(cx.theme().border)
        .rounded_lg()
        .overflow_hidden()
        .child(
            div()
                .px_3()
                .py_2()
                .bg(cx.theme().muted)
                .text_sm()
                .child(title),
        )
        .child(
            div()
                .id(id)
                .flex_1()
                .min_h_0()
                .overflow_scroll()
                .p_3()
                .font_family("monospace")
                .text_size(px(13.0))
                .line_height(px(22.0))
                .children(source.lines().enumerate().map(|(index, line)| {
                    div()
                        .flex()
                        .whitespace_nowrap()
                        .min_h(px(22.0))
                        .child(
                            div()
                                .w(px(40.0))
                                .flex_shrink_0()
                                .text_color(cx.theme().muted_foreground)
                                .child((index + 1).to_string()),
                        )
                        .child(line.to_owned())
                })),
        )
}

impl Render for PreviewWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_editor(window, cx);
        self.ensure_inputs(window, cx);
        let no_document = self.document.is_none();
        let disabled =
            no_document || self.raw_dirty || self.field_draft.is_dirty() || self.project_busy();
        let path = self
            .document
            .as_ref()
            .and_then(|doc| doc.path())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Untitled · bundled plugins".into());
        let state_label = if self.field_draft.is_dirty() {
            "Unapplied value"
        } else if self.raw_dirty {
            "Unapplied JSONC"
        } else if self.dirty() {
            "Unsaved"
        } else {
            "Saved"
        };
        let freshness = if self.raw_dirty || !self.trusted {
            " · paused; previous result"
        } else if self.pending {
            " · updating; previous result"
        } else if self.last_success != Some(self.revision) {
            " · stale result"
        } else {
            ""
        };
        let preview_title = format!("Preview{freshness}");
        let language = self.source.language();
        let settings = self.render_settings(window, cx);
        let source = if self.show_saved {
            code_pane(
                "saved-config-pane",
                "Saved config · read only".into(),
                self.document
                    .as_ref()
                    .map(ConfigDocument::saved_text)
                    .unwrap_or(""),
                cx,
            )
            .into_any_element()
        } else if self.show_config {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().text_sm().flex_1().child("JSONC draft"))
                        .child(
                            Button::new("apply-raw")
                                .label("Apply")
                                .disabled(!self.raw_dirty || self.field_draft.is_dirty())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.apply_raw(window, cx);
                                })),
                        ),
                )
                .child(
                    Textarea::new(self.editor.as_ref().unwrap())
                        .h_full()
                        .font_family("monospace")
                        .text_size(px(13.0))
                        .disabled(no_document)
                        .aria_label("Configuration JSONC draft"),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let result = if self.diff_mode {
            let text = if !comparison_is_current(
                self.last_success,
                self.revision,
                self.pending,
                self.trusted,
                self.raw_dirty,
            ) {
                "Comparison unavailable until both saved and draft previews succeed for the current source.".into()
            } else if let Some(error) = &self.baseline_error {
                format!("Saved baseline formatting failed: {error}")
            } else if let Some(baseline) = &self.baseline {
                if baseline == &self.output {
                    "No formatting differences between saved configuration and draft.".into()
                } else {
                    visible_diff(baseline, &self.output)
                }
            } else {
                "No saved baseline result yet.".into()
            };
            code_pane(
                "diff-pane",
                "Diff · − saved config / + draft · same source".into(),
                &text,
                cx,
            )
            .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(preview_title),
                )
                .child(self.preview_display.render(&self.output, language, cx))
                .into_any_element()
        };
        let body = match self.page {
            Page::Styles => div()
                .flex()
                .gap_3()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .p_2()
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded_lg()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_sm()
                                        .child(self.source.language().display_name()),
                                )
                                .child(
                                    Button::new("copy-preview")
                                        .label("Copy")
                                        .tooltip("Copy formatted preview")
                                        .disabled(
                                            self.output.is_empty()
                                                || self.show_config
                                                || self.show_saved
                                                || self.diff_mode,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                this.output.clone(),
                                            ));
                                            if !this.error {
                                                this.status = "Preview copied".into();
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("change-language")
                                        .label("Change")
                                        .disabled(self.field_draft.is_dirty())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_language_picker(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("diff")
                                        .label(if self.diff_mode {
                                            "Preview"
                                        } else {
                                            "Compare saved"
                                        })
                                        .selected(self.diff_mode)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.diff_mode = !this.diff_mode;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(if self.show_config || self.show_saved {
                            source
                        } else {
                            result
                        }),
                )
                .child(settings)
                .into_any_element(),
            Page::Project => self.render_project(cx),
            Page::Integration => self.render_integration(cx),
            Page::Toolchains => self.render_toolchains(cx),
        };
        div().relative().size_full().flex().flex_col().gap_2().p_4().bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().flex().items_center().gap_2().flex_wrap()
                .child(div().text_2xl().mr_3().child("CodeStyles"))
                .child(Button::new("open-config").label("Open…").disabled(self.dialog_pending || self.project_busy()).on_click(cx.listener(|this, _, window, cx| this.open_config(window, cx))))
                .child(Button::new("save-config").label("Save").disabled(disabled || self.dialog_pending).on_click(cx.listener(|this, _, window, cx| this.save(window, cx))))
                .child(Button::new("save-config-as").label("Save As…").disabled(disabled || self.dialog_pending).on_click(cx.listener(|this, _, window, cx| this.save_as(window, cx))))
                .child(Button::new("check-disk").label("Check disk").disabled(self.document.as_ref().is_none_or(|doc| doc.path().is_none())).on_click(cx.listener(|this, _, _, cx| this.check_disk(cx))))
                .child(Button::new("show-config").label(if self.show_config { "Preview" } else { "Edit JSONC" }).selected(self.show_config).disabled(no_document)
                    .on_click(cx.listener(|this, _, _, cx| { this.show_config = !this.show_config; this.show_saved = false; cx.notify(); })))
                .child(Button::new("saved-baseline").label(if self.show_saved { "Preview" } else { "Saved config" }).selected(self.show_saved).disabled(no_document).on_click(cx.listener(|this, _, _, cx| { this.show_saved = !this.show_saved; cx.notify(); })))
                .child(Button::new("theme").label(if cx.theme().is_dark() { "Light theme" } else { "Dark theme" }).on_click(cx.listener(|_, _, window, cx| {
                    let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark }; Theme::change(mode, Some(window), cx); cx.notify();
                }))))
            .child(div().text_sm().child(format!("{path} · {state_label}")))
            .when_some(self.disk_warning.clone(), |panel, warning| panel.child(div().text_sm().text_color(cx.theme().danger).child(warning)))
            .when(!self.trusted && !no_document, |panel| panel.child(div().flex().items_center().gap_3().p_2().border_1().border_color(cx.theme().warning).rounded_lg()
                .child(div().flex_1().text_sm().child("Preview paused. Review this config before allowing network resources or process plugins."))
                .child(Button::new("trust-config").label("Review & trust…").disabled(self.dialog_pending || self.project_busy() || self.raw_dirty).on_click(cx.listener(|this, _, window, cx| this.trust_config(window, cx))))))
            .child(div().flex().gap_2()
                .children([(Page::Styles, "Styles"), (Page::Project, "Project"), (Page::Integration, "Editor & CI"), (Page::Toolchains, "Languages")].into_iter().enumerate().map(|(index, (page, label))| Button::new(("page", index)).label(label).selected(self.page == page).disabled(self.project_busy() || self.dialog_pending).on_click(cx.listener(move |this, _, _, cx| { this.page = page; if page == Page::Styles { this.request_preview(cx); } cx.notify(); })))))
            .child(body)
            .child(div().max_h(px(55.0)).id("status").overflow_y_scroll().text_sm().text_color(if self.error { cx.theme().danger } else { cx.theme().muted_foreground }).child(self.status.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FieldDraft, ImportClose, ImportOperation, comparison_is_current, dialog_is_current,
        field_input_text, format_package_bytes, has_unsaved_changes, package_footprint_text,
        project_can_start, rustfmt_import_allowed, rustfmt_import_is_current, visible_diff,
    };
    use codestyles::{
        config::ConfigDocument,
        schema::{PinnedPlugin, SchemaCatalog},
        toolchains::PackFootprint,
    };

    #[test]
    fn inline_control_cache_uses_identical_keys_for_creation_and_update_lookup() {
        use codestyles::schema::SchemaContext;
        let mut controls = std::collections::HashMap::new();
        let plugin = PinnedPlugin::TypeScript;
        let vue = super::inline_control_key(SchemaContext::ManagedVue, plugin, "lineWidth");
        controls.insert(vue.clone(), "Vue editor");
        assert_eq!(
            controls.get(&super::inline_control_key(
                SchemaContext::ManagedVue,
                plugin,
                "lineWidth"
            )),
            Some(&"Vue editor")
        );
        let standalone = super::inline_control_key(SchemaContext::Standalone, plugin, "lineWidth");
        assert_ne!(vue, standalone);
        assert!(!controls.contains_key(&standalone));
        controls.insert(standalone.clone(), "Standalone editor");
        assert_eq!(controls.get(&standalone), Some(&"Standalone editor"));
        assert_eq!(controls.get(&vue), Some(&"Vue editor"));
        assert_ne!(
            vue,
            super::inline_control_key(SchemaContext::ManagedVue, PinnedPlugin::Markup, "lineWidth")
        );
        assert_ne!(
            vue,
            super::inline_control_key(SchemaContext::ManagedVue, plugin, "indentWidth")
        );
    }

    #[test]
    fn dart_has_exactly_three_actual_source_formatter_controls() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let names: std::collections::BTreeSet<_> = catalog
            .fields(PinnedPlugin::Dartfmt)
            .iter()
            .map(|field| field.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["languageVersion", "pageWidth", "trailingCommas"]
                .into_iter()
                .collect()
        );
        assert!(catalog.fields(PinnedPlugin::Gofmt).is_empty());
    }

    #[test]
    fn vue_groups_map_to_existing_sections_and_block_pending_transitions() {
        assert_eq!(
            super::VueSettingsGroup::ALL.map(|group| group.plugin().section()),
            ["markup", "typescript", "malva"]
        );
        assert!(super::vue_group_can_change(false, false, false, false));
        for (raw, field, busy, dialog) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            assert!(!super::vue_group_can_change(raw, field, busy, dialog));
        }
    }

    #[test]
    fn nullable_choices_preserve_json_types_and_distinguish_inheritance() {
        use serde_json::{Value, json};
        assert_eq!(
            super::setting_choice_label(&Value::Null, true),
            "null (explicit)"
        );
        assert_ne!(
            super::setting_choice_label(&json!(false), true),
            super::setting_choice_label(&json!("false"), true)
        );
        assert_ne!(
            super::setting_choice_label(&Value::Null, true),
            super::setting_choice_label(&json!("null"), true)
        );
        let catalog = SchemaCatalog::bundled().unwrap();
        let field = catalog
            .schema(PinnedPlugin::TypeScript)
            .field("lineWidth")
            .unwrap();
        assert!(super::field_annotation(field, None).contains("Inherited / default (not written)"));
        assert_eq!(
            super::field_annotation(field, Some(&Value::Null)),
            "Explicit: null"
        );
        assert_eq!(
            super::field_annotation(field, Some(&json!(80))),
            "Explicit: 80"
        );
    }

    #[test]
    fn publication_state_is_not_inferred_from_editable_settings() {
        assert!(super::remote_pack_availability(false).contains("cannot be installed remotely"));
        assert!(super::remote_pack_availability(true).contains("available for this platform"));
    }

    #[test]
    fn unrelated_pack_mutations_never_cancel_or_block_the_current_preview() {
        use codestyles::{comparison::SourceLanguage, toolchain_worker::PackAction};
        for action in [
            PackAction::Install,
            PackAction::Uninstall,
            PackAction::InstallLocal("candidate.tar.gz".into()),
        ] {
            assert!(!super::pack_mutation_affects_preview(
                SourceLanguage::Vue,
                "gofmt",
                &action
            ));
            assert!(!super::pack_mutation_affects_preview(
                SourceLanguage::TypeScript,
                "gofmt",
                &action
            ));
            assert!(super::pack_mutation_affects_preview(
                SourceLanguage::Go,
                "gofmt",
                &action
            ));
        }
        assert!(!super::pack_mutation_affects_preview(
            SourceLanguage::Go,
            "gofmt",
            &PackAction::Refresh
        ));
        assert!(!super::pack_matches_preview(SourceLanguage::Vue, "gofmt"));
        assert!(!super::pack_matches_preview(
            SourceLanguage::TypeScript,
            "gofmt"
        ));
        assert!(super::pack_matches_preview(SourceLanguage::Go, "gofmt"));
    }

    #[test]
    fn optional_languages_bind_to_distinct_pack_and_project_scopes() {
        use codestyles::comparison::SourceLanguage;
        use codestyles::project::NativeLanguage;
        for (language, id, native) in [
            (SourceLanguage::Rust, "rustfmt", NativeLanguage::Rust),
            (SourceLanguage::Go, "gofmt", NativeLanguage::Go),
            (SourceLanguage::Dart, "dartfmt", NativeLanguage::Dart),
            (SourceLanguage::Vue, "vue", NativeLanguage::Vue),
        ] {
            assert_eq!(language.pack_id(), Some(id));
            assert_eq!(super::native_language(language), Some(native));
            assert!(!language.is_bundled());
        }
        assert_eq!(super::native_language(SourceLanguage::TypeScript), None);
        assert_eq!(SourceLanguage::TypeScript.pack_id(), None);
    }

    #[test]
    fn reading_close_detaches_import_and_old_result_cannot_take_new_dialog() {
        let mut state = ImportOperation::default();
        let old = state.begin();
        assert!(state.begin_read(old));
        // Reading does not block the caller's normal dirty/close prompt.
        assert_eq!(state.cancel_for_close(), ImportClose::Reading);
        assert_eq!(state.finish(old), None);
        assert_eq!(state.cancel_for_close(), ImportClose::None);
        let new = state.begin();
        assert_ne!(old, new);
        assert!(!state.begin_read(old));
        assert_eq!(state.finish(old), None);
        assert!(state.begin_read(new));
        assert_eq!(state.finish(new), Some(false));
    }

    #[test]
    fn picker_close_requires_dismissal_and_never_starts_a_cancelled_read() {
        let mut state = ImportOperation::default();
        let id = state.begin();
        assert_eq!(state.cancel_for_close(), ImportClose::Picker);
        assert_eq!(state.cancel_for_close(), ImportClose::Picker);
        assert!(!state.begin_read(id));
        assert_eq!(state.finish(id), Some(true));
        assert_eq!(state.cancel_for_close(), ImportClose::None);
    }

    #[test]
    fn rustfmt_import_blocks_pending_input_repeated_dialogs_and_busy_operations() {
        assert!(rustfmt_import_allowed(true, false, false, false));
        assert!(!rustfmt_import_allowed(false, false, false, false));
        assert!(!rustfmt_import_allowed(true, true, false, false));
        assert!(!rustfmt_import_allowed(true, false, true, false));
        assert!(!rustfmt_import_allowed(true, false, false, true));
        // A saved or already-dirty applied draft is allowed; only unapplied
        // input blocks merging, and the import never performs a save.
    }

    #[test]
    fn rustfmt_import_late_result_cannot_replace_new_input_or_another_scope() {
        assert!(rustfmt_import_is_current(3, 3, false, false, true, false));
        assert!(!rustfmt_import_is_current(3, 3, false, false, true, true));
        assert!(!rustfmt_import_is_current(3, 4, false, false, true, false));
        assert!(!rustfmt_import_is_current(3, 3, true, false, true, false));
        assert!(!rustfmt_import_is_current(3, 3, false, true, true, false));
        assert!(!rustfmt_import_is_current(3, 3, false, false, false, false));
    }

    #[test]
    fn package_size_units_preserve_exact_bytes_and_never_turn_unknown_into_zero() {
        assert_eq!(format_package_bytes(Some(0)), "0 bytes");
        assert_eq!(format_package_bytes(Some(1023)), "1023 bytes");
        assert_eq!(format_package_bytes(Some(1024)), "1.0 KiB (1024 bytes)");
        assert_eq!(
            format_package_bytes(Some(1_048_576)),
            "1.0 MiB (1048576 bytes)"
        );
        assert_eq!(
            format_package_bytes(Some(1_073_741_824)),
            "1.0 GiB (1073741824 bytes)"
        );
        assert!(format_package_bytes(Some(u64::MAX)).contains(&u64::MAX.to_string()));
        assert!(format_package_bytes(None).starts_with("Unknown"));
        assert!(!format_package_bytes(None).contains("0 bytes"));
    }

    #[test]
    fn package_footprint_keeps_unknown_and_error_distinct_from_available_sizes() {
        assert!(package_footprint_text(None).contains("not yet available"));
        assert!(package_footprint_text(Some(&Ok(None))).contains("no archive metadata"));
        let invalid = Err("invalid size bounds".into());
        let error = package_footprint_text(Some(&invalid));
        assert!(error.contains("invalid size bounds"));
        assert!(error.contains("No size estimate"));
        assert!(!error.contains("0 bytes"));
        // A pinned unpublished candidate has the same size metadata as a
        // published bundle; publication gating remains independent.
        let metadata = Ok(Some(PackFootprint {
            download_bytes: None,
            installed_payload_bytes: Some(2048),
            installed_payload_limit_bytes: 4096,
            staging_payload_limit_bytes: 8192,
        }));
        let text = package_footprint_text(Some(&metadata));
        assert!(text.contains("Compressed download: Unknown"));
        assert!(text.contains("Installed file payload: 2.0 KiB (2048 bytes)"));
        assert!(text.contains("Installed payload limit: 4.0 KiB (4096 bytes)"));
        assert!(text.contains("New staging payload limit: 8.0 KiB (8192 bytes)"));
        assert!(!text.contains("available"));
        let equal = Ok(Some(PackFootprint {
            download_bytes: Some(1024),
            installed_payload_bytes: Some(4096),
            installed_payload_limit_bytes: 4096,
            staging_payload_limit_bytes: 8192,
        }));
        let text = package_footprint_text(Some(&equal));
        assert_eq!(text.lines().count(), 3);
        assert!(!text.contains("Installed payload limit"));
        let unknown = Ok(Some(PackFootprint {
            download_bytes: None,
            installed_payload_bytes: None,
            installed_payload_limit_bytes: 4096,
            staging_payload_limit_bytes: 8192,
        }));
        let text = package_footprint_text(Some(&unknown));
        assert!(text.contains("Installed file payload: Unknown"));
        assert!(text.contains("Installed payload limit: 4.0 KiB (4096 bytes)"));
    }

    #[test]
    fn raw_setting_help_preserves_explicit_value_and_default_distinction() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let field = catalog
            .schema(PinnedPlugin::Markdown)
            .field("tags")
            .unwrap();
        assert!(!field.kind.is_supported());
        let value = serde_json::json!(["custom", "café"]);
        let help = super::field_help(field, Some(&value));
        assert!(help.contains("Explicit value: [\"custom\",\"café\"]"));
        assert!(help.contains(&field.name));
        assert!(help.contains(&field.description));
        assert!(help.contains("Schema default:"));
        assert!(super::field_help(field, None).contains("Explicit value: unset"));
    }

    #[test]
    fn enter_selects_unchanged_default_and_preserves_other_pending_input() {
        let mut selected = None;
        let mut draft = FieldDraft::default();
        assert!(super::prepare_inline_apply(
            &mut selected,
            &mut draft,
            "lineWidth",
            "120".into(),
            "120".into()
        ));
        assert_eq!(selected.as_deref(), Some("lineWidth"));
        assert_eq!(draft.text, "120");
        assert!(!draft.is_dirty());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dprint.jsonc");
        let plugin = PinnedPlugin::TypeScript;
        std::fs::write(
            &path,
            format!(r#"{{"plugins":["{}"],"typescript":{{}}}}"#, plugin.url()),
        )
        .unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        let catalog = SchemaCatalog::bundled().unwrap();
        let field = catalog.schema(plugin).field("lineWidth").unwrap();
        field
            .set(&mut doc, plugin, field.parse_input(&draft.text).unwrap())
            .unwrap();
        assert_eq!(
            field.state(&doc, plugin).unwrap().explicit,
            Some(serde_json::json!(120))
        );
        draft.text = "invalid".into();
        assert!(!super::prepare_inline_apply(
            &mut selected,
            &mut draft,
            "indentWidth",
            "2".into(),
            "2".into()
        ));
        assert_eq!(selected.as_deref(), Some("lineWidth"));
        assert_eq!(draft.text, "invalid");
        assert!(draft.is_dirty());
        assert!(super::prepare_inline_apply(
            &mut selected,
            &mut draft,
            "lineWidth",
            "120".into(),
            "130".into()
        ));
        assert_eq!(draft.text, "130");
    }

    #[test]
    fn installed_preview_grammars_have_real_highlighting_and_optional_languages_allow_plain_text() {
        use codestyles::comparison::SourceLanguage;
        use gpui_kit::component::{
            highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter},
            input::Rope,
        };
        for language in SourceLanguage::ALL {
            let grammar = super::highlight_language(language);
            if matches!(
                language,
                SourceLanguage::Go | SourceLanguage::Dart | SourceLanguage::Vue
            ) && !LanguageRegistry::singleton()
                .language(grammar)
                .is_some_and(|config| config.has_grammar())
            {
                continue; // Formatter support does not imply an installed display grammar.
            }
            assert!(
                LanguageRegistry::singleton()
                    .language(grammar)
                    .is_some_and(|config| config.has_grammar()),
                "missing grammar: {grammar}"
            );
            let text = language.builtin_source();
            let mut highlighter = SyntaxHighlighter::new(grammar);
            assert!(highlighter.update(None, &Rope::from_str(text), None));
            assert!(highlighter.tree().is_some(), "no parse tree: {grammar}");
            for theme in [
                HighlightTheme::default_dark(),
                HighlightTheme::default_light(),
            ] {
                let spans = highlighter.styles(&(0..text.len()), &*theme);
                assert!(spans.len() > 2, "missing token spans: {grammar}");
                assert!(
                    spans.iter().any(|(_, style)| style.color.is_some()),
                    "no token color: {grammar}"
                );
                assert!(spans.iter().all(|(range, _)| range.start <= range.end
                    && range.end <= text.len()
                    && text.is_char_boundary(range.start)
                    && text.is_char_boundary(range.end)));
            }
        }
    }

    #[test]
    fn jsonc_comments_are_colored_as_comments() {
        use gpui_kit::component::{
            highlighter::{HighlightTheme, SyntaxHighlighter},
            input::Rope,
        };
        let text = "// comment\n{\"enabled\": true}\n";
        let mut highlighter = SyntaxHighlighter::new(super::highlight_language(
            codestyles::comparison::SourceLanguage::Jsonc,
        ));
        highlighter.update(None, &Rope::from_str(text), None);
        let theme = HighlightTheme::default_dark();
        let comment_color = theme.style("comment").unwrap().color;
        assert!(
            highlighter
                .styles(&(0..text.len()), &*theme)
                .iter()
                .any(|(range, style)| range.start == 0
                    && range.end >= 10
                    && style.color == comment_color)
        );
    }

    #[test]
    fn project_selection_is_explicit_and_independent_of_focused_diff() {
        use std::path::PathBuf;
        let paths = vec![PathBuf::from("a.ts"), PathBuf::from("b.ts")];
        let mut selection = super::ProjectSelection::default();
        assert!(selection.paths(&paths).is_empty());
        selection.focused = Some(paths[1].clone());
        assert!(selection.paths(&paths).is_empty());
        selection.set(paths[0].clone(), true, &paths);
        selection.set(paths[0].clone(), true, &paths);
        assert_eq!(selection.paths(&paths), vec![paths[0].clone()]);
        assert_eq!(selection.focused, Some(paths[1].clone()));
        selection.set(paths[0].clone(), false, &paths);
        assert!(selection.paths(&paths).is_empty());
    }

    #[test]
    fn project_selection_rejects_unknown_paths_and_clears_for_replacement() {
        use std::path::PathBuf;
        let paths = vec![PathBuf::from("a.ts"), PathBuf::from("b.ts")];
        let mut selection = super::ProjectSelection::default();
        selection.set(PathBuf::from("unreviewed.ts"), true, &paths);
        assert!(selection.selected.is_empty());
        selection.selected = paths.iter().cloned().collect();
        selection.focused = Some(paths[0].clone());
        assert_eq!(selection.paths(&paths[1..]), vec![paths[1].clone()]);
        selection.clear();
        assert!(selection.paths(&paths).is_empty());
        assert!(selection.focused.is_none());
    }

    #[test]
    fn project_actions_require_saved_clean_idle_state() {
        assert!(project_can_start(false, false, false, true, true));
        assert!(!project_can_start(true, false, false, true, true));
        assert!(!project_can_start(false, true, false, true, true));
        assert!(!project_can_start(false, false, true, true, true));
        assert!(!project_can_start(false, false, false, false, true));
        assert!(!project_can_start(false, false, false, true, false));
    }

    #[test]
    fn pending_values_survive_blocked_actions_until_explicit_apply_or_discard() {
        let mut field = FieldDraft::default();
        field.reset("120".into());
        field.text = "100".into();
        // Save and destructive selection/category/language/raw transitions share this guard;
        // close/open route the same dirty state through their confirmation.
        for _action in [
            "save",
            "save-as",
            "close",
            "open",
            "category",
            "selection",
            "language",
            "raw-apply",
        ] {
            assert!(field.is_dirty());
            assert!(has_unsaved_changes(false, field.is_dirty()));
            assert_eq!(field.text, "100");
        }
        field.reset("100".into());
        assert!(!field.is_dirty()); // successful Apply
        field.text = "invalid".into();
        assert!(field.is_dirty());
        field.reset("100".into());
        assert!(!field.is_dirty()); // explicit Discard
    }
    #[test]
    fn diff_exposes_line_ending_and_final_newline_changes() {
        let diff = visible_diff("a\n", "a\r\n");
        assert!(diff.contains("− a ⟦LF⟧"));
        assert!(diff.contains("+ a ⟦CRLF⟧"));
        let diff = visible_diff("a\n", "a");
        assert!(diff.contains("− a ⟦LF⟧"));
        assert!(diff.contains("+ a ⟦no newline⟧"));
    }
    #[test]
    fn field_input_tracks_applied_raw_values_and_never_masks_invalid_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dprint.jsonc");
        let plugin = PinnedPlugin::TypeScript;
        let source = |settings: &str| {
            format!(
                r#"{{"plugins":["{}"],"typescript":{settings}}}"#,
                plugin.url()
            )
        };
        std::fs::write(&path, source("{}")).unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        let catalog = SchemaCatalog::bundled().unwrap();
        let field = catalog.schema(plugin).field("lineWidth").unwrap();
        assert_eq!(field_input_text(field, &doc, plugin), "120");
        doc.replace_draft_text(&source(r#"{"lineWidth":80}"#))
            .unwrap();
        assert_eq!(field_input_text(field, &doc, plugin), "80");
        doc.replace_draft_text(&source(r#"{"lineWidth":80,"lineWidth":90}"#))
            .unwrap();
        assert_eq!(field_input_text(field, &doc, plugin), "");
        doc.replace_draft_text(&source("false")).unwrap();
        assert_eq!(field_input_text(field, &doc, plugin), "");
    }
    #[test]
    fn raw_edits_and_draft_changes_need_discard_confirmation() {
        assert!(!has_unsaved_changes(false, false));
        for (draft, raw) in [(true, false), (false, true), (true, true)] {
            assert!(has_unsaved_changes(draft, raw));
        }
    }
    #[test]
    fn comparisons_require_current_success_and_active_trust() {
        assert!(comparison_is_current(Some(4), 4, false, true, false));
        assert!(!comparison_is_current(Some(4), 5, false, true, false));
        assert!(!comparison_is_current(Some(4), 4, true, true, false));
        assert!(!comparison_is_current(Some(4), 4, false, false, false));
        assert!(!comparison_is_current(Some(4), 4, false, true, true));
        assert!(!comparison_is_current(None, 4, false, true, false));
    }
    #[test]
    fn late_dialogs_do_not_apply_after_any_revision() {
        assert!(dialog_is_current(41, 41));
        assert!(!dialog_is_current(41, 42));
        assert!(!dialog_is_current(41, 43));
    }
}
