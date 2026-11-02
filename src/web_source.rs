//! Conservative scope checks for standalone managed Web/config formats.
//!
//! This is not a syntax validator. The pinned formatter remains responsible for
//! parsing supported input. These checks only reject unsupported template/block
//! composition before invoking it; they never read external sources. Template
//! markers are rejected even in comments and quoted strings (a deliberate false
//! positive), rather than interpreting a template language or guessing intent.
use html5ever::tokenizer::{
    BufferQueue, EndTag, StartTag, Tag, Token, TokenSink, TokenSinkResult, Tokenizer,
    TokenizerOpts, states::RawKind,
};

pub fn validate_source(extension: &str, source: &str) -> Result<(), String> {
    if source.len() > 32 * 1024 * 1024 {
        return Err("Managed source exceeds the 32 MiB safety limit.".into());
    }
    if source.contains('\0') {
        return Err("NUL characters are unsupported in managed source.".into());
    }
    if ["{{", "{%", "{#", "<%", "<?"]
        .iter()
        .any(|marker| source.contains(marker))
    {
        return Err("Template markers are unsupported in standalone managed formatting, including in comments or strings. Render or remove templates first; no source was formatted.".into());
    }
    if matches!(extension, "yaml" | "yml")
        && source
            .as_bytes()
            .windows(2)
            .enumerate()
            .any(|(index, pair)| {
                matches!(pair[0], b'|' | b'>')
                    && (pair[1] == b'+'
                        || ((b'1'..=b'9').contains(&pair[1])
                            && source.as_bytes().get(index + 2) == Some(&b'+')))
            })
    {
        return Err("YAML keep-chomping block scalars (|+ or >+, including indentation indicators) are unsupported: the pinned formatter can remove semantically significant trailing newlines. This conservative check also rejects matching literal strings/comments.".into());
    }
    match extension {
        "html" | "htm" => validate_html(source),
        "css" | "scss" | "yaml" | "yml" | "sql" => Ok(()),
        _ => Err("Unsupported standalone managed source extension.".into()),
    }
}

fn validate_html(source: &str) -> Result<(), String> {
    let mut input = BufferQueue::default();
    input.push_back(source.trim_start_matches('\u{feff}').into());
    let mut tokenizer = Tokenizer::new(HtmlScope::default(), TokenizerOpts::default());
    let _ = tokenizer.feed(&mut input);
    tokenizer.end();
    let scope = tokenizer.sink;
    if let Some(error) = scope.error {
        return Err(error);
    }
    if scope.raw.is_some() {
        return Err(
            "Unclosed HTML raw-text block; close the script/style/text element before formatting."
                .into(),
        );
    }
    Ok(())
}

#[derive(Default)]
struct HtmlScope {
    raw: Option<String>,
    error: Option<String>,
}
impl HtmlScope {
    fn fail(&mut self, message: &str, line: u64) {
        if self.error.is_none() {
            self.error = Some(format!(
                "HTML support: {message} (line {line}). Nothing was formatted."
            ));
        }
    }
    fn check_block(&mut self, tag: &Tag, line: u64) {
        let attr = |key: &str| {
            tag.attrs
                .iter()
                .find(|attribute| attribute.name.local.as_ref() == key)
        };
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
            self.fail(
                "external src and dynamic block language/type are unsupported; use inline blocks",
                line,
            );
            return;
        }
        if attr("language").is_some() {
            self.fail(
                "legacy language attributes are unsupported; use an explicit supported lang/type",
                line,
            );
            return;
        }
        if tag.self_closing {
            self.fail("script/style self-closing syntax is ambiguous in HTML; use an explicit closing tag", line);
            return;
        }
        let lang = attr("lang").map(|attribute| attribute.value.as_ref());
        let kind = attr("type").map(|attribute| attribute.value.as_ref());
        let supported = match tag.name.as_ref() {
            "script" => {
                let javascript = matches!(
                    kind,
                    None | Some(
                        "module"
                            | "application/javascript"
                            | "text/javascript"
                            | "application/ecmascript"
                            | "text/ecmascript"
                    )
                );
                let json = matches!(
                    kind,
                    Some(
                        "importmap"
                            | "application/json"
                            | "text/json"
                            | "application/ld+json"
                            | "speculationrules"
                    )
                );
                (javascript && matches!(lang, None | Some("js" | "ts"))) || (json && lang.is_none())
            }
            "style" => {
                matches!(lang, None | Some("css" | "scss"))
                    && matches!(kind, None | Some("text/css"))
            }
            _ => false,
        };
        if !supported {
            self.fail(
                "unsupported inline block lang/type (script: JS/TS or JSON MIME; style: CSS/SCSS)",
                line,
            );
        }
    }
}
impl TokenSink for HtmlScope {
    type Handle = ();
    fn process_token(&mut self, token: Token, line: u64) -> TokenSinkResult<()> {
        if self.error.is_some() {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if tag.kind == EndTag {
                    if self.raw.as_deref() == Some(name) {
                        self.raw = None;
                    }
                    return TokenSinkResult::Continue;
                }
                if tag.kind != StartTag {
                    return TokenSinkResult::Continue;
                }
                if matches!(name, "script" | "style") {
                    self.check_block(&tag, line);
                }
                if name == "template"
                    && tag.attrs.iter().any(|attribute| {
                        matches!(
                            attribute.name.local.as_ref(),
                            "lang" | "src" | ":lang" | ":src"
                        )
                    })
                {
                    self.fail(
                        "HTML template elements cannot select another language or external source",
                        line,
                    );
                }
                // The tokenizer needs the same raw-text boundaries as an HTML
                // parser; apparent tags inside JS/CSS strings are not elements.
                let raw = match name {
                    "script" => Some(RawKind::ScriptData),
                    "style" | "xmp" | "iframe" | "noembed" | "noframes" => Some(RawKind::Rawtext),
                    "textarea" | "title" => Some(RawKind::Rcdata),
                    "plaintext" => {
                        self.fail("plaintext elements cannot be safely composed", line);
                        None
                    }
                    _ => None,
                };
                if let Some(kind) = raw {
                    self.raw = Some(name.to_owned());
                    return TokenSinkResult::RawData(kind);
                }
            }
            Token::ParseError(message) if message.as_ref() == "Duplicate attribute" => {
                self.fail("duplicate attributes are ambiguous", line);
            }
            Token::NullCharacterToken => self.fail("NUL characters are unsupported", line),
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_document_and_verified_inline_languages_are_allowed() {
        for source in [
            "<!DOCTYPE html><html><head><title>Example</title></head><body><p>Hello &amp; welcome</p></body></html>",
            r#"<script lang="ts">const x:number=1;</script><style lang="scss">$color:red;.x{color:$color}</style>"#,
            r#"<script type="application/ld+json">{"name":"test"}</script><script type="module">export const x=1;</script>"#,
            "<template><div>Static content</div></template>",
            "",
        ] {
            validate_source("html", source).unwrap();
            validate_source("htm", source).unwrap();
        }
    }

    #[test]
    fn tag_looking_text_inside_attributes_comments_and_raw_blocks_is_not_composition() {
        validate_source("html", r#"<!-- <script src="bad"> --><div title="<style lang='less'>">ok</div><textarea><script src="bad">text</script></textarea><script>const text='<style lang="less">not a tag</style>'; /* <script src="bad"> */</script><style>.x{content:'<script src="bad">';}</style>"#).unwrap();
    }

    #[test]
    fn html_unsupported_and_ambiguous_blocks_fail_closed() {
        for source in [
            r#"<script src="outside.js"></script>"#,
            r#"<style src="outside.css"></style>"#,
            r#"<script lang="coffee">x</script>"#,
            r#"<script language="vbscript">x</script>"#,
            r#"<script type="text/babel">x</script>"#,
            r#"<script type="application/json" lang="ts">{}</script>"#,
            r#"<style lang="less">x</style>"#,
            r#"<style lang="sass">x</style>"#,
            r#"<script lang="js" lang="ts">x</script>"#,
            r#"<div id="a" id="b">x</div>"#,
            r#"<script :type="mode">x</script>"#,
            r#"<template lang="pug">x</template>"#,
            "<script/>",
            "<style/>",
            "<script>const x=1;",
            "<style>.x{}",
            "<plaintext>text",
        ] {
            assert!(validate_source("html", source).is_err(), "{source}");
        }
    }

    #[test]
    fn yaml_keep_chomping_is_rejected_before_unsafe_upstream_formatting() {
        for indicator in ["|+", ">+", "|2+", ">2+", "|+2", ">+2", "|9+", ">+9"] {
            for extension in ["yaml", "yml"] {
                let source = format!("text: {indicator}\n  line\n\n");
                assert!(
                    validate_source(extension, &source)
                        .unwrap_err()
                        .contains("keep-chomping")
                );
            }
        }
        for indicator in ["|", ">", "|-", ">-", "|2-", ">-2"] {
            validate_source("yaml", &format!("text: {indicator}\n  line\n\n")).unwrap();
        }
        assert!(validate_source("yaml", "text: '|+ literal'").is_err());
        validate_source("css", ".x { content: '|+'; }").unwrap();
    }

    #[test]
    fn css_scss_yaml_and_sql_have_no_template_interpretation() {
        for (extension, source) in [
            ("css", ".x{color:red;--x:1;}"),
            ("scss", "$color:red;.x{color:$color;&:hover{opacity:.8}}"),
            (
                "yaml",
                "# keep\nbase: &base\n  value: 1\ncopy: *base\ntext: |\n  hello\n  world\n",
            ),
            ("yml", "items: [true, null, '001']\n"),
            ("sql", "SELECT a FROM items WHERE id = 1;"),
        ] {
            validate_source(extension, source).unwrap();
            for marker in [
                "{{ value }}",
                "{% if value %}",
                "{# comment #}",
                "<%= value %>",
                "<?php echo 1; ?>",
                "\0",
            ] {
                assert!(validate_source(extension, &format!("{source}\n{marker}")).is_err());
            }
        }
        assert!(validate_source("yaml", "value: '{{ literal }}'").is_err());
        assert!(validate_source("unknown", "").is_err());
    }
}
