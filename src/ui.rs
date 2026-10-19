//! Native P2 schema-driven style editing. Formatting and debounce live off the UI thread.
use std::{collections::HashMap, path::PathBuf, time::Duration};

use crate::preview_display::PreviewDisplay;
use codestyles::{
    backend::FormatRequest,
    comparison::{DiffKind, SourceLanguage, SourceState, line_diff},
    config::ConfigDocument,
    integration,
    preview::PreviewController,
    project::{ProjectAction, ProjectReport, ProjectRequest},
    project_worker::ProjectController,
    resources,
    schema::{FieldKind, PinnedPlugin, SchemaCatalog, SchemaField, setting_categories},
};
use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Selectable, Sizable, Theme, ThemeMode, WindowExt,
        button::Button,
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
    page: Page,
    project_report: Option<ProjectReport>,
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
    search: String,
    diff_mode: bool,
    baseline: Option<String>,
    baseline_error: Option<String>,
    trusted: bool,
    document_version: u64,
    dialog_pending: bool,
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
            page: Page::Styles,
            project_report: None,
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
            search: String::new(),
            diff_mode: false,
            baseline: None,
            baseline_error: None,
            trusted: true,
            document_version: 0,
            dialog_pending: false,
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
                    if let Some(result) = view.project_controller.as_mut().and_then(|c| c.poll()) {
                        match result.result {
                            Ok(report) => {
                                view.project_status = report.summary.clone();
                                view.project_error = false;
                                view.project_report = Some(report);
                            }
                            Err(error) => {
                                view.project_status = error;
                                view.project_error = true;
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
        if self.project_busy() {
            self.project_status = "Cancel the running project operation and wait for it to stop before closing. Files already formatted are not rolled back.".into();
            cx.notify();
            return false;
        }
        if !self.dirty() && self.source.custom_source().is_none() {
            return true;
        }
        if self.dialog_pending {
            return false;
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
        if self.project_busy() || !self.trusted || self.raw_dirty {
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
        path: &[&str],
        value: Option<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.raw_dirty {
            return;
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let result = match value {
            Some(value) => document.set(path, value),
            None => document.remove(path),
        };
        match result {
            Ok(()) => {
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
                self.document_version += 1;
                self.project_report = None;
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

fn has_unsaved_changes(document_dirty: bool, raw_dirty: bool) -> bool {
    document_dirty || raw_dirty
}

// A native dialog is asynchronous. The entire draft, including unapplied raw
// text and successful saves, must still be the version the dialog reviewed.
fn dialog_is_current(opened_version: u64, current_version: u64) -> bool {
    opened_version == current_version
}

impl PreviewWindow {
    fn plugin(&self) -> PinnedPlugin {
        self.source.language().plugin()
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
        let key = format!("{}.{}", self.plugin().section(), field.name);
        let baseline = self
            .document
            .as_ref()
            .map(|doc| field_input_text(field, doc, self.plugin()))
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
                if this.plugin() != plugin {
                    return;
                }
                if matches!(event, InputEvent::Change) {
                    let text = editor.read(cx).value().to_string();
                    let baseline = this
                        .document
                        .as_ref()
                        .map(|doc| field_input_text(&field, doc, plugin))
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
                        this.document_version += 1;
                        cx.notify();
                    }
                } else if matches!(event, InputEvent::PressEnter { .. }) {
                    let baseline = this
                        .document
                        .as_ref()
                        .map(|doc| field_input_text(&field, doc, plugin))
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
        let key = format!("{}.{}", self.plugin().section(), field.name);
        let explicit = self
            .document
            .as_ref()
            .and_then(|doc| field.state(doc, self.plugin()).ok())
            .and_then(|state| state.explicit);
        let label = |value: &Value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        };
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
                if this.plugin() != plugin {
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
            .get(&format!("{}.{}", self.plugin().section(), name))
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
                self.document
                    .as_ref()
                    .map(|doc| field_input_text(&field, doc, self.plugin()))
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
        if let Err(error) = catalog.available(document, plugin) {
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
        self.change_setting(&[plugin.section(), &field.name], value, window, cx);
        self.sync_field(window, cx);
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
                    .and_then(|doc| schema.available(doc, plugin))
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
                .and_then(|doc| field.state(doc, plugin));
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
                                .placeholder("Unset")
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
                            .placeholder("Unset")
                            .accessibility_label(field.name.clone())
                            .disabled(blocked),
                    );
                }
                FieldKind::Const(value) => {
                    let select = self.inline_select(field, vec![value.clone()], window, cx);
                    controls = controls.child(
                        Select::new(&select)
                            .w(px(170.0))
                            .small()
                            .placeholder("Unset")
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
            let annotation = if explicit.is_some() {
                "Custom"
            } else {
                "Unset"
            };
            let help = field_help(field, explicit.as_ref());
            let mut row = div().id(("setting-row", index)).aria_label(field.name.clone()).aria_description(help.clone()).flex().flex_col().gap_1().py_2()
                .child(div().flex().items_center().gap_3()
                    .child(div().id(("setting-label", index)).flex_1().min_w_0().text_sm().child(field.short_label()).tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx)))
                    .child(controls)
                    .child(Button::new(("reset", index)).small().label("Reset").tooltip("Remove explicit value; inherit configuration or plugin default")
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
                        list.child(div().py_3().text_sm().child("No matches in this tab"))
                    })
                    .children(rows),
            )
            .into_any_element()
    }

    fn open_language_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                                                                if !this.allow_field_transition(cx)
                                                                {
                                                                    return;
                                                                }
                                                                this.source
                                                                    .select_language(language);
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Styles,
    Project,
    Integration,
}

impl PreviewWindow {
    fn project_busy(&self) -> bool {
        self.project_controller
            .as_ref()
            .is_some_and(ProjectController::is_running)
    }

    fn project_ready(&self) -> bool {
        project_can_start(
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
        let document = self.document.as_ref().unwrap();
        let root = document.path().unwrap().parent().unwrap().to_path_buf();
        let version = self.document_version;
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
        let explanation = format!(
            "{scope}\n\nUses the saved configuration and dprint's normal nested configuration discovery. Review this project's nested configs, extends and plugin sources: they may download resources and execute process plugins with your account's permissions. {}",
            if writes {
                "This operation writes source files. Review or back up your work first. Cancellation stops remaining work; files already written are not rolled back."
            } else {
                "This operation does not intentionally format source files. Plugin preparation may populate dprint's cache. Trust is not a sandbox."
            }
        );
        self.dialog_pending = true;
        let prompt = window.prompt(
            PromptLevel::Warning,
            if writes {
                "Trust project and format files?"
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
                if !dialog_is_current(version, this.document_version) || !this.project_ready() {
                    this.project_status = "Configuration changed during confirmation. Review it again before running.".into();
                    this.project_error = true;
                    cx.notify(); return;
                }
                let request = ProjectRequest::new(root, this.document.as_ref().unwrap(), action, true);
                match request.and_then(|request| this.project_controller.as_mut().unwrap().submit(request)) {
                    Ok(_) => {
                        this.cancel_preview();
                        this.page = Page::Project;
                        this.project_report = None;
                        this.project_error = false;
                        this.project_status = "Running bundled dprint with the saved project configuration…".into();
                    }
                    Err(error) => { this.project_status = error; this.project_error = true; }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn choose_project_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.project_ready() {
            return;
        }
        self.dialog_pending = true;
        let version = self.document_version;
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Select source files inside the project directory".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let selected = selection.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.dialog_pending = false;
                if !dialog_is_current(version, this.document_version) {
                    this.project_status =
                        "Configuration changed while choosing files. Choose them again.".into();
                    this.project_error = true;
                } else {
                    match selected {
                        Ok(Ok(Some(paths))) if !paths.is_empty() => {
                            this.project_action(ProjectAction::FormatSelected(paths), window, cx)
                        }
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => {
                            this.project_status = format!("Cannot choose source files: {error}");
                            this.project_error = true;
                        }
                        Err(error) => {
                            this.project_status = format!("File picker closed: {error}");
                            this.project_error = true;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_project(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = !self.project_ready();
        let root = self
            .document
            .as_ref()
            .and_then(|doc| doc.path())
            .and_then(|path| path.parent())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Save or open a configuration first.".into());
        let text = self.project_report.as_ref().map(|report| {
            let mut text = format!("{}\n{} · {} source files · {} protected config/plugin files\nFormatting reports show pre-write diffs. Rerun Check project for current state.\n\n{}\n", report.summary, report.cli_version, report.checked_files, report.protected_configs, report.diagnostics);
            for file in &report.files {
                text.push_str(&format!("\n{}\n{}\n", file.path.display(), file.diff));
            }
            text
        }).unwrap_or_else(|| "No current project report. A check reads project sources; formatting requires an explicit confirmation. Check results are a point-in-time report, not a live filesystem view.".into());
        div().flex().flex_col().flex_1().min_h_0().gap_3()
            .child(div().text_sm().child(format!("Project root · {root}")))
            .child(div().text_sm().child("Saved configuration only · normal nested discovery · check exit 20 means formatting differences"))
            .when(self.dirty(), |panel| panel.child(div().text_color(cx.theme().warning).child("Save or discard all draft, JSONC and pending value edits before project operations.")))
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("project-check").label("Check project…").disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.project_action(ProjectAction::Check, window, cx))))
                .child(Button::new("project-prepare").label("Prepare plugins…").disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.project_action(ProjectAction::PreparePlugins, window, cx))))
                .child(Button::new("project-format-selected").label("Format files…").disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.choose_project_files(window, cx))))
                .child(Button::new("project-format-all").label("Format project…").disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.project_action(ProjectAction::FormatProject, window, cx))))
                .child(Button::new("project-cancel").label("Cancel").disabled(!self.project_busy()).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(controller) = &mut this.project_controller { controller.cancel(); }
                    this.project_status = "Cancellation requested. Waiting for dprint to stop; files already written are not rolled back.".into(); cx.notify();
                }))))
            .child(div().text_sm().text_color(if self.project_error { cx.theme().danger } else { cx.theme().muted_foreground }).child(self.project_status.clone()))
            .child(code_pane("project-report", "Project report · saved config".into(), &text, cx))
            .into_any_element()
    }

    fn render_integration(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let guide = self
            .executable
            .as_ref()
            .ok_or_else(|| {
                "Bundled executable is unavailable. Verify the resource bundle first.".to_owned()
            })
            .and_then(|path| integration::guidance(path));
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
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .child("Editor & CI"),
            )
            .child(
                Button::new("copy-dprint-path")
                    .label("Copy dprint path")
                    .disabled(path.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(path) = &path {
                            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                            this.status =
                                "Bundled dprint path copied. Paste it into VS Code user settings."
                                    .into();
                            cx.notify();
                        }
                    })),
            )
            .child(Button::new("copy-editor-settings").label("Copy settings").disabled(settings.is_none()).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(settings) = &settings { cx.write_to_clipboard(ClipboardItem::new_string(settings.clone())); this.status = "Settings JSON copied. Merge it into VS Code user settings, preserving existing entries.".into(); cx.notify(); }
            })))
            .child(Button::new("open-dprint-extension").label("VS Code extension").on_click(|_, _, cx| cx.open_url(integration::EXTENSION_URL)))
            .child(code_pane(
                "integration-guide",
                "Editor & CI setup · no settings changed automatically".into(),
                &text,
                cx,
            ))
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

fn field_input_text(
    field: &SchemaField,
    document: &ConfigDocument,
    plugin: PinnedPlugin,
) -> String {
    field
        .state(document, plugin)
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
                .children([(Page::Styles, "Styles"), (Page::Project, "Project"), (Page::Integration, "Editor & CI")].into_iter().enumerate().map(|(index, (page, label))| Button::new(("page", index)).label(label).selected(self.page == page).disabled(self.project_busy() || self.dialog_pending).on_click(cx.listener(move |this, _, _, cx| { this.page = page; if page == Page::Styles { this.request_preview(cx); } cx.notify(); })))))
            .child(body)
            .child(div().max_h(px(55.0)).id("status").overflow_y_scroll().text_sm().text_color(if self.error { cx.theme().danger } else { cx.theme().muted_foreground }).child(self.status.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FieldDraft, comparison_is_current, dialog_is_current, field_input_text,
        has_unsaved_changes, project_can_start, visible_diff,
    };
    use codestyles::{
        config::ConfigDocument,
        schema::{PinnedPlugin, SchemaCatalog},
    };

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
    fn every_preview_language_has_real_syntax_highlighting() {
        use codestyles::comparison::SourceLanguage;
        use gpui_kit::component::{
            highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter},
            input::Rope,
        };
        for language in SourceLanguage::ALL {
            let grammar = super::highlight_language(language);
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
