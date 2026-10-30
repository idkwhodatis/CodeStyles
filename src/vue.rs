//! Conservative Vue SFC scope gate for the managed markup + TS + Malva bundle.
//!
//! This is not a Vue compiler or a replacement syntax validator. html5ever's
//! tokenizer identifies comments, quoted attributes and raw-text boundaries;
//! dprint validates the supported languages afterwards. Never scan for tags with
//! regex: JavaScript strings/comments can contain arbitrary tag-looking text.
use html5ever::tokenizer::{
    BufferQueue, EndTag, StartTag, Tag, Token, TokenSink, TokenSinkResult, Tokenizer,
    TokenizerOpts, states::RawKind,
};

const MAX_SOURCE: usize = 32 * 1024 * 1024;

/// Reject blocks whose contents cannot be fully formatted by the fixed Vue pack.
/// No reads, downloads, config changes, external `src` following or execution.
pub fn validate_source(source: &str) -> Result<(), String> {
    if source.len() > MAX_SOURCE {
        return Err("Vue source exceeds the 32 MiB safety limit.".into());
    }
    let mut queue = BufferQueue::default();
    queue.push_back(source.trim_start_matches('\u{feff}').into());
    let mut tokenizer = Tokenizer::new(Scope::default(), TokenizerOpts::default());
    let _ = tokenizer.feed(&mut queue);
    tokenizer.end();
    let scope = tokenizer.sink;
    if let Some(error) = scope.error {
        return Err(error);
    }
    if scope.template_depth != 0 || scope.raw_root.is_some() {
        return Err("Unclosed Vue SFC block; check the template/script/style closing tag.".into());
    }
    if scope.blocks == 0 {
        return Err("Vue formatting requires a template, script or style SFC block.".into());
    }
    Ok(())
}

#[derive(Default)]
struct Scope {
    template_depth: usize,
    raw_root: Option<String>,
    template_seen: bool,
    script_seen: bool,
    setup_seen: bool,
    blocks: usize,
    error: Option<String>,
    duplicate_attr: bool,
}

impl Scope {
    fn fail(&mut self, message: &str, line: u64) {
        if self.error.is_none() {
            self.error = Some(format!(
                "Vue support: {message} (line {line}). Nothing was formatted."
            ));
        }
    }

    fn check_block(&mut self, tag: &Tag, line: u64) {
        let name = tag.name.as_ref();
        let attr = |key: &str| tag.attrs.iter().find(|a| a.name.local.as_ref() == key);
        if [
            "src",
            ":src",
            "v-bind:src",
            ":lang",
            "v-bind:lang",
            ":type",
            "v-bind:type",
        ]
        .iter()
        .any(|key| attr(key).is_some())
        {
            self.fail("external src and dynamic block language/type are unsupported; use inline blocks with an explicit supported language", line);
            return;
        }
        let lang = attr("lang").map(|a| a.value.as_ref());
        let kind = attr("type").map(|a| a.value.as_ref());
        let supported = match name {
            "template" => matches!(lang, None | Some("html")) && kind.is_none(),
            "script" => {
                let script_type = matches!(
                    kind,
                    None | Some(
                        "module"
                            | "application/javascript"
                            | "text/javascript"
                            | "application/ecmascript"
                            | "text/ecmascript"
                            | "application/x-javascript"
                            | "application/x-ecmascript"
                            | "text/x-javascript"
                            | "text/x-ecmascript"
                            | "text/jsx"
                            | "text/babel"
                    )
                );
                let json_type = matches!(
                    kind,
                    Some(
                        "importmap"
                            | "application/json"
                            | "text/json"
                            | "application/ld+json"
                            | "speculationrules"
                    )
                );
                (script_type
                    && matches!(
                        lang,
                        None | Some("js" | "ts" | "jsx" | "tsx" | "mjs" | "mts")
                    ))
                    || (json_type && lang.is_none() && attr("setup").is_none())
            }
            "style" => {
                matches!(lang, None | Some("css" | "scss" | "sass" | "less"))
                    && matches!(kind, None | Some("text/css"))
            }
            _ => false,
        };
        if !supported {
            self.fail("unsupported block lang/type (template: HTML; script: JS/TS or JSON MIME; style: CSS/SCSS/Sass/Less). Pug, Stylus and custom blocks are not supported", line);
        }
    }
}

impl TokenSink for Scope {
    type Handle = ();

    fn process_token(&mut self, token: Token, line: u64) -> TokenSinkResult<()> {
        if self.error.is_some() {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                let duplicate_attr = std::mem::take(&mut self.duplicate_attr);
                if duplicate_attr && matches!(name, "script" | "style") {
                    self.fail(
                        "duplicate attributes make the block language ambiguous",
                        line,
                    );
                }
                if tag.kind == EndTag {
                    if self.raw_root.as_deref() == Some(name) {
                        self.raw_root = None;
                    } else if self.template_depth > 0 {
                        if name == "template" {
                            self.template_depth -= 1;
                        }
                    } else {
                        self.fail("unexpected top-level closing tag", line);
                    }
                    return TokenSinkResult::Continue;
                }
                if tag.kind != StartTag {
                    return TokenSinkResult::Continue;
                }
                if self.template_depth == 0 && self.raw_root.is_none() {
                    if !matches!(name, "template" | "script" | "style") {
                        self.fail("custom top-level blocks are unsupported", line);
                        return TokenSinkResult::Continue;
                    }
                    self.check_block(&tag, line);
                    self.blocks += 1;
                    match name {
                        "template" => {
                            if self.template_seen {
                                self.fail("multiple root template blocks are unsupported", line);
                            }
                            self.template_seen = true;
                        }
                        "script" => {
                            let setup = tag.attrs.iter().any(|a| a.name.local.as_ref() == "setup");
                            let seen = if setup {
                                &mut self.setup_seen
                            } else {
                                &mut self.script_seen
                            };
                            if *seen {
                                self.fail("duplicate script or script setup block", line);
                            } else {
                                *seen = true;
                            }
                        }
                        _ => {}
                    }
                    if !tag.self_closing && name != "template" {
                        self.raw_root = Some(name.to_owned());
                    }
                } else if matches!(name, "script" | "style") {
                    self.check_block(&tag, line);
                }
                if tag.self_closing {
                    return TokenSinkResult::Continue;
                }
                match name {
                    "template" => {
                        self.template_depth += 1;
                        if self.template_depth > 256 {
                            self.fail("template nesting exceeds the safety limit", line);
                        }
                    }
                    "script" => return TokenSinkResult::RawData(RawKind::ScriptData),
                    "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                        return TokenSinkResult::RawData(RawKind::Rawtext);
                    }
                    "textarea" | "title" => return TokenSinkResult::RawData(RawKind::Rcdata),
                    "plaintext" => self.fail("plaintext template content is unsupported", line),
                    _ => {}
                }
            }
            Token::CharacterTokens(text) if self.template_depth == 0 && self.raw_root.is_none() => {
                if !text.chars().all(char::is_whitespace) {
                    self.fail("text outside SFC blocks is unsupported", line);
                }
            }
            Token::ParseError(ref message) if message.as_ref() == "Duplicate attribute" => {
                self.duplicate_attr = true;
                if self.template_depth == 0 && self.raw_root.is_none() {
                    self.fail("duplicate attributes in a top-level block", line);
                }
            }
            Token::ParseError(_) if self.template_depth == 0 && self.raw_root.is_none() => {
                self.fail(
                    "malformed or duplicate attributes in a top-level block",
                    line,
                );
            }
            Token::NullCharacterToken | Token::DoctypeToken(_) => self.fail(
                "NUL and document doctypes are not supported in an SFC",
                line,
            ),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_blocks_and_vue_directives() {
        for style in ["css", "scss", "sass", "less"] {
            let source = format!(
                r#"<template><Input v-model="name"></Input><My.Component :[key]="value" @click.stop="run" #default="slot"><template v-if="ok"><span>{{ value }}</span></template></My.Component></template><script>export default {{}}</script><script setup lang="ts">const s = "<style lang='stylus'>fake</style>"; /* <template lang='pug'> */</script><style lang="{style}" scoped>.x{{color:red}}</style>"#
            );
            validate_source(&source).unwrap();
        }
        validate_source(r#"<template/><script type="application/json">{"x":1}</script><style/>"#)
            .unwrap();
    }

    #[test]
    fn quoted_attributes_comments_and_raw_text_are_not_blocks() {
        validate_source(r#"<!-- <custom/><script src="bad"/> --><template><div title="<script src='bad'>"><textarea><style lang="stylus">text</style></textarea></div></template><script setup>const x = '<i18n>not real</i18n>'; // <style src="bad">
</script><style>/* <template lang="pug"> */ .x{content:"<script>"}</style>"#).unwrap();
    }

    #[test]
    fn unsupported_scope_is_rejected_not_silently_partially_formatted() {
        for source in [
            r#"<template lang="pug">div</template>"#,
            r#"<style lang="stylus">x</style>"#,
            r#"<script src="./outside.ts"/>"#,
            r#"<template src="x"/>"#,
            r#"<style src="x"/>"#,
            r#"<i18n>{}</i18n>"#,
            r#"<script lang="coffee">x</script>"#,
            r#"<script type="unknown">x</script>"#,
            r#"<script :lang="mode"/>"#,
            r#"<template><style lang="stylus">x</style></template>"#,
            r#"<script lang="ts" lang="coffee"/>"#,
            r#"<template><style lang="css" lang="stylus">x</style></template>"#,
            "<template><template></template>",
            "<script>const x=1;",
            "text<template/>",
            "",
            "<template/><template/>",
        ] {
            assert!(validate_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn literal_script_close_uses_html_boundary_even_inside_js_string() {
        assert!(
            validate_source(
                r#"<script>const text = "</script><i18n>real outer block</i18n>";</script>"#
            )
            .is_err()
        );
        validate_source(r#"<script>const text = "<\/script><i18n>inside string</i18n>";</script>"#)
            .unwrap();
    }
}
