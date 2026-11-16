//! Fail-closed source guards for optional script/config formatting. These are
//! deliberately narrower than the languages; they never execute user source.
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptLanguage {
    Shell,
    Lua,
    Luau,
    Dockerfile,
    Caddyfile,
}

impl ScriptLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        let named = |base: &str| {
            name == base
                || name
                    .strip_prefix(base)
                    .is_some_and(|tail| tail.starts_with('.') && tail.len() > 1)
        };
        // Special names win over a misleading final extension (Dockerfile.lua).
        if named("Dockerfile") {
            return Some(Self::Dockerfile);
        }
        if named("Caddyfile") {
            return Some(Self::Caddyfile);
        }
        if name
            .strip_suffix(".dockerfile")
            .is_some_and(|head| !head.is_empty())
        {
            return Some(Self::Dockerfile);
        }
        if name
            .strip_suffix(".caddyfile")
            .is_some_and(|head| !head.is_empty())
        {
            return Some(Self::Caddyfile);
        }
        match path.extension()?.to_str()? {
            "sh" | "bash" => Some(Self::Shell),
            "lua" => Some(Self::Lua),
            "luau" => Some(Self::Luau),
            _ => None,
        }
    }

    pub fn checked_from_path(path: &Path) -> Result<Option<Self>, String> {
        if path.file_name().is_some_and(|name| {
            name.to_str().is_none()
                && Self::from_path(Path::new(name.to_string_lossy().as_ref())).is_some()
        }) {
            return Err("Formatting requires a UTF-8 filename so language safety checks cannot be bypassed.".into());
        }
        Ok(Self::from_path(path))
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Shell => "sh",
            Self::Lua => "lua",
            Self::Luau => "luau",
            Self::Dockerfile => "dockerfile",
            Self::Caddyfile => "caddyfile",
        }
    }

    pub fn pack_id(self) -> &'static str {
        match self {
            Self::Shell => "shfmt",
            Self::Lua | Self::Luau => "stylua",
            Self::Dockerfile => "dockerfile",
            Self::Caddyfile => "caddyfmt",
        }
    }
}

pub fn validate_source(
    language: ScriptLanguage,
    source: &str,
    settings: &serde_json::Value,
) -> Result<(), String> {
    if source.contains('\0') {
        return Err("Source contains a NUL byte; refusing to format it.".into());
    }
    if source.len() > 32 * 1024 * 1024 {
        return Err("Source exceeds the 32 MiB limit.".into());
    }
    match language {
        ScriptLanguage::Shell => validate_shell(source, settings),
        ScriptLanguage::Lua | ScriptLanguage::Luau => Ok(()),
        ScriptLanguage::Dockerfile => docker_literals(source).map(|_| ()),
        // caddy fmt is a whitespace formatter, not a full Caddy parser. Never
        // call adapt/validate or resolve imports to pretend otherwise.
        ScriptLanguage::Caddyfile => caddy_tokens(source).map(|_| ()),
    }
}

fn validate_shell(source: &str, settings: &serde_json::Value) -> Result<(), String> {
    let first = source.lines().next().unwrap_or("").trim_end_matches('\r');
    if let Some(shebang) = first.strip_prefix("#!") {
        let parts: Vec<_> = shebang.split_whitespace().collect();
        let shell = match parts.as_slice() {
            ["/bin/sh" | "/usr/bin/sh" | "/bin/dash" | "/usr/bin/dash"] => "posix",
            ["/bin/bash" | "/usr/bin/bash"] => "bash",
            ["/usr/bin/env", "sh" | "dash"] => "posix",
            ["/usr/bin/env", "bash"] => "bash",
            _ => return Err("Unsupported shell shebang. Only explicit sh, dash or bash (or /usr/bin/env with one shell name and no options) is supported.".into()),
        };
        let configured = settings["shfmt"]["dialect"].as_str().unwrap_or("auto");
        if configured != "auto" && configured != shell {
            return Err("Shell dialect conflicts with the source shebang. Select a matching explicit dialect or auto.".into());
        }
    }
    Ok(())
}

fn unsupported_template(source: &str) -> bool {
    ["{{", "}}", "{%", "%}", "<%", "%>"]
        .iter()
        .any(|marker| source.contains(marker))
}

/// Retains complete quoted values and heredocs. Upstream is not a shell parser:
/// expansions/legacy ENV/custom SHELL have observed whitespace-changing cases.
fn docker_literals(source: &str) -> Result<Vec<String>, String> {
    if unsupported_template(source) {
        return Err("Dockerfile templates are unsupported; refusing to format.".into());
    }
    let lines: Vec<_> = source.split_inclusive('\n').collect();
    let mut protected = Vec::new();
    let mut index = 0;
    let mut continuation = false;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            let directive = trimmed
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_lowercase();
            if directive.starts_with("#escape=") && directive != "#escape=\\" {
                return Err("Dockerfile custom escape directives are unsupported.".into());
            }
            if directive.starts_with("#syntax=") {
                return Err("Dockerfile frontend syntax directives are unsupported by this pinned formatter scope.".into());
            }
            continue;
        }
        if line.contains('`') || line.contains('^') || line.contains("$(") || line.contains("${") {
            return Err("Dockerfile shell substitutions, backtick/caret escapes and parameter expansions are unsupported because the formatter can change literal content.".into());
        }
        if line.contains("\\\t") {
            return Err("Dockerfile backslash-tab literals are unsupported because the formatter can replace the tab.".into());
        }
        let (instruction, arguments) = trimmed
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed, ""));
        let instruction = instruction.to_ascii_uppercase();
        if !continuation {
            if !matches!(
                instruction.as_str(),
                "FROM"
                    | "RUN"
                    | "CMD"
                    | "LABEL"
                    | "MAINTAINER"
                    | "EXPOSE"
                    | "ENV"
                    | "ADD"
                    | "COPY"
                    | "ENTRYPOINT"
                    | "VOLUME"
                    | "USER"
                    | "WORKDIR"
                    | "ARG"
                    | "STOPSIGNAL"
                    | "HEALTHCHECK"
            ) {
                return Err(format!(
                    "Unsupported Dockerfile instruction {instruction}. Custom SHELL, ONBUILD and unknown instructions are intentionally excluded."
                ));
            }
            if arguments.trim().is_empty() {
                return Err(format!("Dockerfile {instruction} requires arguments."));
            }
            if matches!(instruction.as_str(), "ARG" | "COPY" | "ADD")
                && line.trim_end().ends_with('\\')
            {
                return Err(format!(
                    "Continued Dockerfile {instruction} instructions are unsupported because the formatter can change token boundaries."
                ));
            }
            if matches!(instruction.as_str(), "ENV" | "LABEL")
                && !arguments
                    .split_whitespace()
                    .next()
                    .is_some_and(|word| word.contains('='))
            {
                return Err("Legacy Dockerfile ENV/LABEL key value syntax can change literal content. Use explicit key=value syntax before formatting.".into());
            }
            protected.push(format!("instruction:{instruction}"));
        }
        protected.extend(quoted_literals(line)?);
        if let Some((_, tail)) = line.split_once("<<") {
            if continuation
                || !matches!(instruction.as_str(), "RUN" | "COPY")
                || tail.contains("<<")
                || line.matches("<<").count() != 1
            {
                return Err("Only one adjacent RUN/COPY heredoc with an ASCII identifier delimiter is supported.".into());
            }
            if tail.starts_with('-') || line.trim_end().ends_with('\\') {
                return Err(
                    "Dockerfile tab-stripping or continued heredoc headers are unsupported.".into(),
                );
            }
            let quote = tail.chars().next().filter(|c| matches!(c, '\'' | '"'));
            let (delimiter, remainder) = if let Some(quote) = quote {
                let rest = &tail[quote.len_utf8()..];
                let Some(end) = rest.find(quote) else {
                    return Err("Unterminated Dockerfile heredoc delimiter.".into());
                };
                (&rest[..end], &rest[end + quote.len_utf8()..])
            } else {
                let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
                (&tail[..end], &tail[end..])
            };
            if delimiter.is_empty()
                || !delimiter.bytes().enumerate().all(|(i, c)| {
                    c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit())
                })
                || (!remainder.trim().is_empty() && instruction != "COPY")
            {
                return Err("Unsupported Dockerfile heredoc delimiter or command suffix; use an adjacent ASCII identifier.".into());
            }
            let mut body = String::new();
            let mut closed = false;
            while index < lines.len() {
                let next = lines[index];
                index += 1;
                body.push_str(next);
                let candidate = next.trim_end_matches(['\r', '\n']);
                if candidate == delimiter {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err("Unterminated Dockerfile heredoc; refusing to format.".into());
            }
            protected.push(format!("heredoc:{quote:?}:{delimiter}:{body}"));
        }
        continuation = line.trim_end().ends_with('\\');
    }
    if continuation {
        return Err("Dockerfile ends with an incomplete line continuation.".into());
    }
    Ok(protected)
}

fn quoted_literals(source: &str) -> Result<Vec<String>, String> {
    let mut literals = Vec::new();
    let mut start = None;
    let mut escaped = false;
    for (index, ch) in source.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some((from, quote)) = start {
            if ch == quote {
                literals.push(source[from..index + ch.len_utf8()].to_owned());
                start = None;
            }
        } else if ch == '\'' || ch == '"' {
            start = Some((index, ch));
        }
    }
    if start.is_some() {
        return Err("Multiline or unterminated Dockerfile quoted values are unsupported.".into());
    }
    Ok(literals)
}

/// Caddy's formatter must preserve lexical tokens. Whitespace outside quoted
/// tokens may change; comments and quoted/backtick values must not.
fn caddy_tokens(source: &str) -> Result<Vec<String>, String> {
    if unsupported_template(source) {
        return Err("Caddyfile templates are unsupported; refusing to format.".into());
    }
    let mut tokens = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        let mut end = start + ch.len_utf8();
        if ch == '"' || ch == '`' {
            let mut escaped = false;
            let mut closed = false;
            for (index, value) in chars.by_ref() {
                end = index + value.len_utf8();
                if escaped {
                    escaped = false;
                    continue;
                }
                if value == '\\' && ch == '"' {
                    escaped = true;
                    continue;
                }
                if value == ch {
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err("Unterminated Caddyfile quoted value; refusing to format.".into());
            }
        } else if ch == '#' {
            while let Some(&(index, value)) = chars.peek() {
                if value == '\n' || value == '\r' {
                    break;
                }
                end = index + value.len_utf8();
                chars.next();
            }
        } else {
            while let Some(&(index, value)) = chars.peek() {
                if value.is_whitespace() {
                    break;
                }
                end = index + value.len_utf8();
                chars.next();
            }
        }
        tokens.push(source[start..end].to_owned());
    }
    Ok(tokens)
}

pub fn validate_output(language: ScriptLanguage, source: &str, output: &str) -> Result<(), String> {
    if !source.trim().is_empty() && output.trim().is_empty() {
        return Err(
            "Formatter returned empty output for nonempty source; original content preserved."
                .into(),
        );
    }
    let same = match language {
        ScriptLanguage::Dockerfile => docker_literals(source)? == docker_literals(output)?,
        ScriptLanguage::Caddyfile => caddy_tokens(source)? == caddy_tokens(output)?,
        _ => true,
    };
    if !same {
        return Err(
            "Formatter changed protected literal content; original content preserved.".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn exact_names_and_extension_priority() {
        for path in [
            "Dockerfile",
            "Dockerfile.dev",
            "Dockerfile.lua",
            "sample.dockerfile",
        ] {
            assert_eq!(
                ScriptLanguage::from_path(Path::new(path)),
                Some(ScriptLanguage::Dockerfile)
            );
        }
        for path in [
            "Caddyfile",
            "Caddyfile.rs",
            "Caddyfile.dockerfile",
            "site.caddyfile",
        ] {
            assert_eq!(
                ScriptLanguage::from_path(Path::new(path)),
                Some(ScriptLanguage::Caddyfile)
            );
        }
        assert_eq!(
            ScriptLanguage::from_path(Path::new(".hidden.sh")),
            Some(ScriptLanguage::Shell)
        );
        for path in [
            ".bashrc",
            "tool",
            "tool.zsh",
            "tool.fish",
            "Dockerfile.",
            ".dockerfile",
            "Caddyfile.",
            "sample.SH",
        ] {
            assert!(
                ScriptLanguage::from_path(Path::new(path)).is_none(),
                "{path}"
            );
        }
    }
    #[test]
    fn unsafe_shell_dialects_and_shebang_options_reject() {
        for source in [
            "#!/bin/zsh\necho hi",
            "#!/usr/bin/env -S bash -e\necho hi",
            "#!/bin/bash -e\necho hi",
        ] {
            assert!(validate_source(ScriptLanguage::Shell, source, &json!({})).is_err());
        }
        assert!(
            validate_source(
                ScriptLanguage::Shell,
                "#!/bin/bash\necho hi",
                &json!({"shfmt":{"dialect":"posix"}})
            )
            .is_err()
        );
        assert!(validate_source(ScriptLanguage::Shell, "#!/bin/sh\necho hi", &json!({})).is_ok());
    }
    #[test]
    fn docker_known_literal_changing_inputs_fail_before_engine() {
        for directive in ["#escape=`", "# escape = `", "# escape=^"] {
            assert!(docker_literals(&format!("{directive}\nFROM alpine\n")).is_err());
        }
        assert!(docker_literals("LABEL message a\\\n  RUN echo b\n").is_err());
        for source in [
            "ARG x=a\\\n RUN echo unexpected\n",
            "ARG x=a\\  \n RUN echo unexpected\n",
            "COPY a\\\n/b /dest\n",
            "COPY a\\  \n/b /dest\n",
            "ADD a\\\n/b /dest\n",
            "COPY a\\\t b /dest\n",
            "# syntax=docker/dockerfile:1\nFROM alpine\n",
        ] {
            assert!(docker_literals(source).is_err(), "{source:?}");
        }
        assert!(
            validate_output(
                ScriptLanguage::Dockerfile,
                "RUN echo a \\\n RUN echo b\n",
                "RUN echo a\nRUN echo b\n"
            )
            .is_err()
        );
        for source in [
            "RUN echo \"$(echo \"a  b\")\"\n",
            "RUN echo ${x:-a  b}\n",
            "ENV GREETING a  b\n",
            "ONBUILD ENV GREETING a  b\n",
            "SHELL [\"powershell\"]\n",
            "RUN <<123\nhello\n123\n",
            "RUN <<EOF-X\nx\nEOF-X\n",
            "RUN << EOF\nx\nEOF\n",
            "RUN <<EOF\nx\n",
            "FROM {{image}}\n",
            "RUN echo `date`\n",
            "RUN echo hi \\\n",
        ] {
            assert!(docker_literals(source).is_err(), "{source:?}");
        }
    }
    #[test]
    fn docker_preserves_heredoc_quotes_and_body() {
        assert!(docker_literals("RUN <<-EOF\n\techo hi\nEOF\n").is_err());
        assert!(docker_literals("COPY <<EOF /path \\\nbody\nEOF\n").is_err());
        for source in [
            "FROM alpine\nRUN <<EOF\necho  hi\nEOF\n",
            "FROM alpine\nRUN <<'EOF'\necho ${x:-a  b}\nEOF\n",
            "FROM alpine\nCOPY <<EOF /a\n  a  b  \nEOF\n",
        ] {
            assert!(docker_literals(source).is_ok(), "{source:?}");
            assert!(validate_output(ScriptLanguage::Dockerfile, source, source).is_ok());
            assert!(
                validate_output(
                    ScriptLanguage::Dockerfile,
                    source,
                    &source.replace("  ", " ")
                )
                .is_err()
            );
        }
    }
    #[test]
    fn caddy_lexical_preservation_without_loading_imports() {
        let source = "localhost {\nrespond \"a  b\"\nimport missing.caddyfile\n}\n";
        let output = "localhost {\n\trespond \"a  b\"\n\timport missing.caddyfile\n}\n";
        assert!(validate_output(ScriptLanguage::Caddyfile, source, output).is_ok());
        assert!(
            validate_output(
                ScriptLanguage::Caddyfile,
                source,
                &output.replace("a  b", "a b")
            )
            .is_err()
        );
        assert!(validate_source(ScriptLanguage::Caddyfile, "respond \"oops", &json!({})).is_err());
    }
}
