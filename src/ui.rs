//! A deliberately small native UI. Formatting and debounce live off the UI thread.
use std::{path::PathBuf, time::Duration};

use codestyles::{backend::FormatRequest, preview::PreviewController, resources};
use gpui_kit::{
    component::{Disableable, Selectable, button::Button},
    prelude::*,
    *,
};

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
    sample: usize,
    indent_width: u8,
    semi_colons: &'static str,
    revision: u64,
    output: String,
    last_success: Option<(usize, u64)>,
    status: String,
    error: bool,
    pending: bool,
}

impl PreviewWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (controller, status, error) = match resources::bundled_backend() {
            Ok(backend) => (Some(PreviewController::new(backend)), "Ready".into(), false),
            Err(error) => (None, format!("Backend unavailable: {error}"), true),
        };
        let mut view = Self {
            controller,
            sample: 0,
            indent_width: 2,
            semi_colons: "always",
            revision: 0,
            output: String::new(),
            last_success: None,
            status,
            error,
            pending: false,
        };
        view.request_preview(cx);
        // Keep only a weak view reference across the timer. Closing the window
        // ends this task rather than retaining the formatter worker indefinitely.
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
                    if result.revision != view.revision {
                        return;
                    }
                    view.pending = false;
                    match result.result {
                        Ok(output) => {
                            view.output = output;
                            view.last_success = Some((view.sample, result.revision));
                            view.status = format!(
                                "Formatted {} · revision {}",
                                SAMPLES[view.sample].1, result.revision
                            );
                            view.error = false;
                        }
                        Err(error) => {
                            view.status = format!("Formatting failed: {error}");
                            view.error = true;
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        view
    }

    fn request_preview(&mut self, cx: &mut Context<Self>) {
        let Some(controller) = self.controller.as_mut() else {
            return;
        };
        let (_, path, source) = SAMPLES[self.sample];
        self.revision = controller.submit(FormatRequest {
            source: source.to_owned(),
            source_path: PathBuf::from(path),
            config_path: None,
            indent_width: self.indent_width,
            semi_colons: self.semi_colons.to_owned(),
        });
        self.pending = true;
        self.error = false;
        self.status = "Updating preview…".into();
        cx.notify();
    }
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.controller.is_none();
        let json = self.sample >= 4;
        let preview_title = match self.last_success {
            Some((sample, revision)) => {
                let freshness = if self.pending {
                    " · previous result, updating…"
                } else if self.error {
                    " · previous result, update failed"
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
        div().size_full().flex().flex_col().gap_4().p_6()
            .bg(rgb(0x0f172a)).text_color(rgb(0xe2e8f0))
            .child(div().text_2xl().child("CodeStyles"))
            .child(div().text_sm().text_color(rgb(0x94a3b8))
                .child("Native formatting preview · dprint · local files stay local"))
            .child(div().flex().items_center().gap_2()
                .child(div().w(px(96.0)).text_sm().child("Sample"))
                .children(SAMPLES.iter().enumerate().map(|(index, sample)| {
                    Button::new(("sample", index)).label(sample.0).selected(self.sample == index)
                        .disabled(disabled).on_click(cx.listener(move |this, _, _, cx| {
                            if this.sample != index { this.sample = index; this.request_preview(cx); }
                        }))
                })))
            .child(div().flex().items_center().gap_2()
                .child(div().w(px(96.0)).text_sm().child("Indent width"))
                .children([2u8, 4].into_iter().map(|width| {
                    Button::new(("indent", width as usize)).label(format!("{width} spaces"))
                        .selected(self.indent_width == width).disabled(disabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.indent_width != width { this.indent_width = width; this.request_preview(cx); }
                        }))
                }))
                .child(div().ml_4().text_sm().child("Semicolons"))
                .children(["always", "asi"].into_iter().enumerate().map(|(index, mode)| {
                    Button::new(("semi", index)).label(mode).selected(self.semi_colons == mode)
                        .disabled(disabled || json).on_click(cx.listener(move |this, _, _, cx| {
                            if this.semi_colons != mode { this.semi_colons = mode; this.request_preview(cx); }
                        }))
                })))
            .child(div().text_sm().text_color(rgb(0x94a3b8))
                .child(if json { "JSON uses indentation; semicolons do not apply." } else {
                    "Change a setting to reformat the unchanged original sample. ASI uses automatic semicolon insertion."
                }))
            .child(div().flex().gap_4().flex_1().min_h_0()
                .child(code_pane("source-pane", format!("Original · {}", SAMPLES[self.sample].1), SAMPLES[self.sample].2))
                .child(code_pane("preview-pane", preview_title, &self.output)))
            .child(div().max_h(px(100.0)).id("status").overflow_y_scroll().text_sm()
                .text_color(if self.error { rgb(0xfca5a5) } else { rgb(0x94a3b8) })
                .child(self.status.clone()))
    }
}
