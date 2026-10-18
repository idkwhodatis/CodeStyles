//! Native P1 configuration editing. Formatting and debounce live off the UI thread.
use std::{path::PathBuf, time::Duration};

use codestyles::{
    backend::FormatRequest, config::ConfigDocument, preview::PreviewController, resources,
};
use gpui_kit::{
    component::{
        Disableable, Selectable,
        button::Button,
        input::{InputEvent, Textarea, TextareaState},
    },
    prelude::*,
    *,
};
use serde_json::Value;

// These inputs never get replaced by formatted output: changing a setting always
// reformats the same original source, so comparisons do not accumulate changes.
const SCRIPT: &str = "const preferences={theme:\"dark\",tabs:[\"source\",\"preview\"]};\nfunction greet(name){\nif(name){\nreturn {message:`Hello, ${name}!`,settings:preferences};\n}\nreturn null;\n}\nconsole.log(greet(\"CodeStyles\"));\n";
const TYPESCRIPT: &str = "type User={name:string;active:boolean};\nconst users:User[]=[{name:\"Ada\",active:true},{name:\"Lin\",active:false}];\nfunction activeNames(items:User[]){\nreturn items.filter(user=>user.active).map(user=>{\nreturn user.name;\n});\n}\nconsole.log(activeNames(users));\n";
const JSX: &str = "const items=[\"Source\",\"Preview\"];\nexport function App(){\nreturn <main><h1>CodeStyles</h1><ul>{items.map(item=><li key={item}>{item}</li>)}</ul></main>;\n}\n";
const JSON: &str = "{\"name\":\"CodeStyles\",\"preview\":{\"enabled\":true,\"languages\":[\"TypeScript\",\"JSON\"]}}\n";
const JSONC: &str = "{\n// Formatting preferences\n\"name\":\"CodeStyles\",\"preview\":{\"enabled\":true,\"languages\":[\"TypeScript\",\"JSON\"],},\n}\n";
const SAMPLES: &[(&str, &str, &str)] = &[
    ("TS", "sample.ts", TYPESCRIPT),
    ("JS", "sample.js", SCRIPT),
    ("TSX", "sample.tsx", JSX),
    ("JSX", "sample.jsx", JSX),
    ("JSON", "sample.json", JSON),
    ("JSONC", "sample.jsonc", JSONC),
];

pub struct PreviewWindow {
    controller: Option<PreviewController>,
    document: Option<ConfigDocument>,
    // Only the checksummed bundle's exact plugin references have a known schema.
    known_plugins: Vec<String>,
    trusted: bool,
    document_version: u64,
    dialog_pending: bool,
    disk_warning: Option<String>,
    editor: Option<Entity<TextareaState>>,
    editor_subscription: Option<Subscription>,
    raw_dirty: bool,
    show_config: bool,
    sample: usize,
    revision: u64,
    output: String,
    last_success: Option<(usize, u64)>,
    status: String,
    error: bool,
    pending: bool,
}

impl PreviewWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (controller, document, status, error) = match resources::bundled_backend() {
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
        let known_plugins = document
            .as_ref()
            .and_then(|doc| doc.value(&["plugins"]).ok().flatten())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect();
        let mut view = Self {
            controller,
            document,
            known_plugins,
            trusted: true,
            document_version: 0,
            dialog_pending: false,
            disk_warning: None,
            editor: None,
            editor_subscription: None,
            raw_dirty: false,
            show_config: false,
            sample: 0,
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
                            view.last_success = Some((view.sample, result.revision));
                            view.status = format!(
                                "Formatted {} · draft revision {}",
                                SAMPLES[view.sample].1, result.revision
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
        if !self.dirty() {
            return true;
        }
        if self.dialog_pending {
            return false;
        }
        self.dialog_pending = true;
        let version = self.document_version;
        let confirmation = window.prompt(
            PromptLevel::Warning,
            "Close without saving your configuration changes?",
            Some("Unsaved draft changes and unapplied raw text, including invalid JSONC, will be lost. Cancel to return to your draft."),
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
            self.raw_dirty,
        )
    }

    fn cancel_preview(&mut self) {
        if let Some(controller) = self.controller.as_mut() {
            self.revision = controller.cancel();
        }
        self.pending = false;
    }

    fn request_preview(&mut self, cx: &mut Context<Self>) {
        if !self.trusted || self.raw_dirty {
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
        let (_, path, source) = SAMPLES[self.sample];
        self.revision = controller.submit(FormatRequest {
            source: source.to_owned(),
            source_path: document
                .path()
                .and_then(|path| path.parent())
                .map(|parent| parent.join(path))
                .unwrap_or_else(|| PathBuf::from(path)),
            config: document.snapshot(),
        });
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

    fn setting(&self, path: &[&str]) -> Option<Value> {
        self.document.as_ref()?.value(path).ok().flatten()
    }

    fn known_typescript_schema(&self) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let (Ok(plugins), Ok(extends), Ok(inherit)) = (
            document.value(&["plugins"]),
            document.value(&["extends"]),
            document.value(&["inherit"]),
        ) else {
            return false;
        };
        has_known_typescript_schema(
            plugins,
            extends.is_some() || inherit.is_some_and(|value| value != Value::Bool(false)),
            &self.known_plugins,
        )
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

    fn apply_raw(&mut self, cx: &mut Context<Self>) {
        if !self.raw_dirty {
            return;
        }
        let (Some(editor), Some(document)) = (&self.editor, &mut self.document) else {
            return;
        };
        let text = editor.read(cx).value();
        match document.replace_draft_text(&text) {
            Ok(()) => {
                self.raw_dirty = false;
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
        if self.dialog_pending {
            return;
        }
        self.dialog_pending = true;
        let version = self.document_version;
        if self.dirty() {
            let confirmation = window.prompt(
                PromptLevel::Warning,
                "Open another configuration?",
                Some("Your unsaved changes will be discarded only after another configuration opens successfully."),
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
                                this.document = Some(document);
                                this.disk_warning = None;
                                this.document_version += 1;
                                this.trusted = false;
                                this.output.clear();
                                this.last_success = None;
                                this.show_config = true;
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
            "Trust this configuration and run its plugins?",
            Some("dprint may resolve extends and plugins over the network. Process plugins can execute programs with your account's permissions. Only trust configuration and plugin sources you have reviewed. Trust applies to this open draft; raw edits and Save As require another review."),
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

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_dirty || self.dialog_pending {
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
                self.disk_warning = None;
                self.status = "Configuration saved atomically. Draft and saved file match.".into();
                self.error = false;
            }
            Err(error) => self.fail(format!("Save failed; your draft is kept: {error}")),
        }
        cx.notify();
    }

    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_dirty || self.dialog_pending || self.document.is_none() {
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

// Preview freshness is independent of the last file-operation message. A
// successful Save or Check disk cannot make an earlier failed preview current.
fn preview_is_current(
    result_sample: usize,
    result_revision: u64,
    sample: usize,
    revision: u64,
) -> bool {
    result_sample == sample && result_revision == revision
}

fn has_known_typescript_schema(
    plugins: Option<Value>,
    inherits: bool,
    known_plugins: &[String],
) -> bool {
    if inherits {
        return false;
    }
    let Some(Value::Array(plugins)) = plugins else {
        return false;
    };
    let mut typescript = false;
    for plugin in plugins {
        let Some(plugin) = plugin.as_str() else {
            return false;
        };
        if !known_plugins.iter().any(|known| known == plugin) {
            return false;
        }
        if PathBuf::from(plugin)
            .file_name()
            .and_then(|name| name.to_str())
            == Some("typescript-0.96.1.wasm")
        {
            typescript = true;
        }
    }
    typescript
}

fn code_pane(id: &'static str, title: String, source: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .border_1()
        .border_color(rgb(0x334155))
        .rounded_lg()
        .overflow_hidden()
        .child(div().px_4().py_3().bg(rgb(0x1e293b)).text_sm().child(title))
        .child(
            div()
                .id(id)
                .flex_1()
                .min_h_0()
                .overflow_scroll()
                .p_4()
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
                                .w(px(36.0))
                                .flex_shrink_0()
                                .text_color(rgb(0x64748b))
                                .child((index + 1).to_string()),
                        )
                        .child(line.to_owned())
                })),
        )
}

impl Render for PreviewWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_editor(window, cx);
        let no_document = self.document.is_none();
        let disabled = no_document || self.raw_dirty;
        let json = self.sample >= 4;
        let known_schema = self.known_typescript_schema();
        let indent = self.setting(&["indentWidth"]);
        let semicolons = self.setting(&["typescript", "semiColons"]);
        let path = self
            .document
            .as_ref()
            .and_then(|doc| doc.path())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Untitled · bundled plugins".into());
        let state_label = if self.raw_dirty {
            "Raw edits not applied"
        } else if self.dirty() {
            "Unsaved draft"
        } else if self
            .document
            .as_ref()
            .is_some_and(|doc| doc.path().is_some())
        {
            "Saved · draft matches disk baseline"
        } else {
            "New draft · not saved to disk"
        };
        let preview_title = match self.last_success {
            Some((sample, revision)) => {
                let freshness = if self.raw_dirty || !self.trusted {
                    " · previous result, preview paused"
                } else if self.pending {
                    " · previous result, updating…"
                } else if !preview_is_current(sample, revision, self.sample, self.revision) {
                    " · previous result, not current draft"
                } else {
                    ""
                };
                format!(
                    "Preview · {} · r{}{}",
                    SAMPLES[sample].1, revision, freshness
                )
            }
            None if self.pending => "Preview · waiting for first result…".into(),
            None => "Preview · no successful output yet".into(),
        };
        let caution = if !known_schema {
            "Unknown plugin schema or inherited configuration: semicolon controls are disabled. Raw editing preserves all plugin references."
        } else if json {
            "JSON uses indentation; semicolons do not apply. Unset removes a property and allows inherited/plugin defaults."
        } else {
            "Unset removes the property. Existing plugin versions, comments, and unrelated fields are preserved."
        };
        div().size_full().flex().flex_col().gap_3().p_5().bg(rgb(0x0f172a)).text_color(rgb(0xe2e8f0))
            .child(div().flex().items_center().gap_2().flex_wrap()
                .child(div().text_2xl().mr_4().child("CodeStyles"))
                .child(Button::new("open-config").label("Open config…").disabled(self.dialog_pending)
                    .on_click(cx.listener(|this, _, window, cx| this.open_config(window, cx))))
                .child(Button::new("save-config").label("Save").disabled(disabled || self.dialog_pending)
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))))
                .child(Button::new("save-config-as").label("Save As…").disabled(disabled || self.dialog_pending)
                    .on_click(cx.listener(|this, _, window, cx| this.save_as(window, cx))))
                .child(Button::new("check-disk").label("Check disk").disabled(self.document.as_ref().is_none_or(|doc| doc.path().is_none()))
                    .on_click(cx.listener(|this, _, _, cx| this.check_disk(cx))))
                .child(Button::new("show-config").label(if self.show_config { "Show source" } else { "Edit JSONC" })
                    .selected(self.show_config).disabled(no_document)
                    .on_click(cx.listener(|this, _, _, cx| { this.show_config = !this.show_config; cx.notify(); }))))
            .child(div().text_sm().child(format!("{path} · {state_label}")))
            .child(div().text_xs().text_color(rgb(0x94a3b8))
                .child("Save As creates a new file; relative references stay unchanged and resolve from its new location."))
            .when_some(self.disk_warning.clone(), |panel, warning| panel.child(
                div().text_sm().text_color(rgb(0xfca5a5)).child(warning)))
            .when(!self.trusted && !no_document, |panel| panel.child(
                div().flex().items_center().gap_3().p_3().bg(rgb(0x422006)).rounded_lg()
                    .child(div().flex_1().text_sm().child("Preview paused. This config may resolve network resources or run process plugins with your account's permissions."))
                    .child(Button::new("trust-config").label("Review & trust…").disabled(self.dialog_pending || self.raw_dirty)
                        .on_click(cx.listener(|this, _, window, cx| this.trust_config(window, cx))))))
            .child(div().flex().items_center().gap_2().flex_wrap()
                .child(div().w(px(96.0)).text_sm().child("Sample"))
                .children(SAMPLES.iter().enumerate().map(|(index, sample)| {
                    Button::new(("sample", index)).label(sample.0).selected(self.sample == index)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.sample != index { this.sample = index; this.request_preview(cx); cx.notify(); }
                        }))
                })))
            .child(div().flex().items_center().gap_2().flex_wrap()
                .child(div().w(px(96.0)).text_sm().child("Indent width"))
                .children([2u8, 4].into_iter().map(|width| {
                    Button::new(("indent", width as usize)).label(format!("{width} spaces"))
                        .selected(indent == Some(Value::from(width))).disabled(disabled)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.change_setting(&["indentWidth"], Some(Value::from(width)), window, cx);
                        }))
                }))
                .child(Button::new("unset-indent").label("Unset").selected(indent.is_none()).disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.change_setting(&["indentWidth"], None, window, cx))))
                .child(div().ml_3().text_sm().child("Semicolons"))
                .children(["always", "asi"].into_iter().enumerate().map(|(index, mode)| {
                    Button::new(("semi", index)).label(mode).selected(semicolons.as_ref().and_then(Value::as_str) == Some(mode))
                        .disabled(disabled || json || !known_schema).on_click(cx.listener(move |this, _, window, cx| {
                            this.change_setting(&["typescript", "semiColons"], Some(Value::from(mode)), window, cx);
                        }))
                }))
                .child(Button::new("unset-semi").label("Unset").selected(semicolons.is_none())
                    .disabled(disabled || json || !known_schema)
                    .on_click(cx.listener(|this, _, window, cx| this.change_setting(&["typescript", "semiColons"], None, window, cx)))))
            .child(div().text_xs().text_color(if known_schema { rgb(0x94a3b8) } else { rgb(0xfcd34d) }).child(caution))
            .child(div().flex().gap_4().flex_1().min_h_0()
                .child(if self.show_config {
                    div().flex().flex_col().flex_1().min_w_0().h_full().gap_2()
                        .child(div().flex().items_center().gap_2()
                            .child(div().text_sm().flex_1().child("Configuration draft · JSONC"))
                            .child(Button::new("apply-raw").label("Apply raw draft").disabled(!self.raw_dirty)
                                .on_click(cx.listener(|this, _, _, cx| this.apply_raw(cx)))))
                        .child(Textarea::new(self.editor.as_ref().unwrap()).h_full().font_family("monospace").text_size(px(13.0)).disabled(no_document)
                            .aria_label("Configuration JSONC draft"))
                        .into_any_element()
                } else {
                    code_pane("source-pane", format!("Original · {}", SAMPLES[self.sample].1), SAMPLES[self.sample].2).into_any_element()
                })
                .child(code_pane("preview-pane", preview_title, &self.output)))
            .child(div().max_h(px(70.0)).id("status").overflow_y_scroll().text_sm()
                .text_color(if self.error { rgb(0xfca5a5) } else { rgb(0x94a3b8) }).child(self.status.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        dialog_is_current, has_known_typescript_schema, has_unsaved_changes, preview_is_current,
    };
    use serde_json::json;

    #[test]
    fn semicolons_require_exact_known_plugin_identity_without_inheritance() {
        let known = vec![
            "https://plugins.dprint.dev/typescript-0.96.1.wasm".to_owned(),
            "https://plugins.dprint.dev/json-0.25.2.wasm".to_owned(),
        ];
        let plugins = Some(json!(known));
        assert!(has_known_typescript_schema(plugins.clone(), false, &known));
        assert!(!has_known_typescript_schema(plugins, true, &known));
        for plugins in [
            json!(["/untrusted/typescript-0.96.1.wasm"]),
            json!(["/bundle/typescript-0.97.0.wasm"]),
            json!(["https://example.org/typescript-0.96.1.wasm"]),
            json!([
                "https://plugins.dprint.dev/typescript-0.96.1.wasm",
                "./unknown-plugin.json"
            ]),
            json!(["/bundle/json-0.25.2.wasm"]),
            json!([]),
            json!([null]),
            json!("not an array"),
        ] {
            assert!(!has_known_typescript_schema(Some(plugins), false, &known));
        }
        assert!(!has_known_typescript_schema(None, false, &known));
    }

    #[test]
    fn invalid_or_unapplied_raw_text_requires_discard_confirmation() {
        assert!(!has_unsaved_changes(false, false));
        assert!(has_unsaved_changes(true, false));
        // Parsing failure leaves the document clean but must protect raw text.
        assert!(has_unsaved_changes(false, true));
        assert!(has_unsaved_changes(true, true));
    }

    #[test]
    fn stale_preview_stays_stale_after_successful_file_operations() {
        assert!(preview_is_current(0, 8, 0, 8));
        // A failed newer revision does not replace the last successful result.
        assert!(!preview_is_current(0, 8, 0, 9));
        // File operations may change status/error presentation, never these IDs.
        assert!(!preview_is_current(0, 8, 0, 9));
        assert!(!preview_is_current(0, 8, 1, 8));
        assert!(preview_is_current(1, 10, 1, 10));
    }

    #[test]
    fn late_dialogs_cannot_apply_after_any_draft_revision() {
        let dialog_version = 41;
        assert!(dialog_is_current(dialog_version, 41));
        assert!(!dialog_is_current(dialog_version, 42));
        // Undoing the text is another revision; it does not revive old approval.
        assert!(!dialog_is_current(dialog_version, 43));
    }
}
