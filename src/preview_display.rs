//! Display-only highlighted preview: no editable input, focus or text cursor.
use codestyles::comparison::SourceLanguage;
use gpui_kit::{
    component::{ActiveTheme, Rope, highlighter::SyntaxHighlighter},
    prelude::*,
    *,
};

#[derive(Default)]
pub struct PreviewDisplay {
    language: Option<SourceLanguage>,
    text: SharedString,
    highlighter: Option<SyntaxHighlighter>,
}

impl PreviewDisplay {
    fn update(&mut self, output: &str, language: SourceLanguage) {
        let changed_language = self.language != Some(language);
        if changed_language || self.highlighter.is_none() {
            self.highlighter = Some(SyntaxHighlighter::new(highlight_language(language)));
            self.language = Some(language);
        }
        if changed_language || self.text.as_ref() != output {
            self.text = output.to_owned().into();
            // Built-in examples are small. Complete parsing avoids painting stale
            // ranges after a change, and reuse avoids reparsing on hover/scroll.
            self.highlighter
                .as_mut()
                .unwrap()
                .update(None, &Rope::from_str(output), None);
        }
    }

    pub fn render(&mut self, output: &str, language: SourceLanguage, cx: &App) -> impl IntoElement {
        self.update(output, language);
        // Resolve colors each frame so switching light/dark never keeps old colors.
        let highlights = self
            .highlighter
            .as_ref()
            .unwrap()
            .styles(&(0..self.text.len()), cx.theme().highlight_theme.as_ref());
        div()
            .id("formatted-example-display")
            .aria_label("Formatted built-in language example, read only")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .overflow_scroll()
            .cursor_default()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .bg(cx.theme().background)
            .font_family("monospace")
            .text_size(px(13.0))
            .line_height(px(22.0))
            .whitespace_nowrap()
            // Keep a non-shrinking, intrinsic-width content box. A direct text
            // child gets the viewport width even when its glyphs paint beyond
            // it, so the scroll container cannot discover horizontal overflow.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .child(StyledText::new(self.text.clone()).with_highlights(highlights)),
            )
    }
}

fn highlight_language(language: SourceLanguage) -> &'static str {
    match language {
        SourceLanguage::TypeScript => "typescript",
        SourceLanguage::JavaScript | SourceLanguage::Jsx => "javascript",
        SourceLanguage::Tsx => "tsx",
        SourceLanguage::Json | SourceLanguage::Jsonc => "json",
        SourceLanguage::Markdown => "markdown",
        SourceLanguage::Toml => "toml",
        SourceLanguage::Rust => "rust",
        SourceLanguage::Go => "go",
        SourceLanguage::Dart => "text",
        SourceLanguage::Vue => "html",
        SourceLanguage::Python | SourceLanguage::PythonStub => "python",
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewDisplay;
    use codestyles::comparison::SourceLanguage;
    use gpui_kit::component::highlighter::HighlightTheme;

    #[test]
    fn every_language_has_safe_highlights_in_both_themes() {
        let text = "// café 😀\nconst 名 = \"hello\";\n# heading\n[section]\nkey = true\n";
        let mut display = PreviewDisplay::default();
        for language in SourceLanguage::ALL {
            display.update(language.builtin_source(), language);
            for theme in [
                HighlightTheme::default_light(),
                HighlightTheme::default_dark(),
            ] {
                let spans = display
                    .highlighter
                    .as_ref()
                    .unwrap()
                    .styles(&(0..language.builtin_source().len()), theme.as_ref());
                if !matches!(
                    language,
                    SourceLanguage::Go | SourceLanguage::Dart | SourceLanguage::Vue
                ) {
                    assert!(
                        spans.iter().any(|(_, style)| style.color.is_some()),
                        "{language:?} must have syntax colors"
                    );
                }
            }
            display.update(text, language);
            assert_eq!(display.text.as_ref(), text);
            for theme in [
                HighlightTheme::default_light(),
                HighlightTheme::default_dark(),
            ] {
                let spans = display
                    .highlighter
                    .as_ref()
                    .unwrap()
                    .styles(&(0..text.len()), theme.as_ref());
                for (range, _) in spans {
                    assert!(range.start <= range.end && range.end <= text.len());
                    assert!(text.is_char_boundary(range.start));
                    assert!(text.is_char_boundary(range.end));
                }
            }
        }
        display.update("", SourceLanguage::TypeScript);
        assert!(display.text.is_empty());
    }
}
