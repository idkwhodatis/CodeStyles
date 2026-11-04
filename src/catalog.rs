//! File types supported by the bundled, version-pinned formatter plugins.
pub const EXTENSIONS: &[&str] = &[
    "ts",
    "tsx",
    "js",
    "jsx",
    "json",
    "jsonc",
    "md",
    "markdown",
    "toml",
    "py",
    "pyi",
    "rs",
    "go",
    "dart",
    "vue",
    "java",
    "cs",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "c++",
    "hh",
    "hpp",
    "hxx",
    "h++",
    "sh",
    "bash",
    "lua",
    "luau",
    "dockerfile",
    "caddyfile",
    "C",
    "H",
    "html",
    "htm",
    "css",
    "scss",
    "yaml",
    "yml",
    "sql",
];
pub const INDENT_WIDTHS: &[u8] = &[2, 4];
pub const SEMICOLONS: &[&str] = &["always", "asi"];

pub fn supports(path: &std::path::Path) -> bool {
    // Script/config routing must fail closed on ambiguous non-UTF8 basenames,
    // while unrelated native filenames retain their existing OsString support.
    if crate::script_source::ScriptLanguage::checked_from_path(path).is_err() {
        return false;
    }
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            ["Dockerfile", "Caddyfile"].iter().any(|base| {
                name == *base
                    || name
                        .strip_prefix(base)
                        .is_some_and(|tail| tail.starts_with('.') && tail.len() > 1)
            })
        })
    {
        return true;
    }
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| EXTENSIONS.contains(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comparison::SourceLanguage;
    use std::path::Path;

    #[test]
    fn shell_lua_and_configuration_names_are_conservative() {
        for name in [
            "tool.sh",
            "tool.bash",
            "module.lua",
            "module.luau",
            "Dockerfile",
            "Dockerfile.dev",
            "Dockerfile.lua",
            "build.dockerfile",
            "Caddyfile",
            "Caddyfile.local",
            "Caddyfile.lua",
            "site.caddyfile",
        ] {
            assert!(supports(Path::new(name)), "{name}");
        }
        for name in [
            "dockerfile",
            "caddyfile",
            "Dockerfile.",
            "Caddyfile.",
            "DockerfileBackup",
            "CaddyfileBackup",
            "tool.zsh",
            "tool.fish",
            "tool.SH",
            "build.DOCKERFILE",
        ] {
            assert!(!supports(Path::new(name)), "{name}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn non_utf8_script_basenames_are_not_supported() {
        use std::os::unix::ffi::OsStringExt;
        for ext in ["sh", "bash", "lua", "luau", "dockerfile", "caddyfile"] {
            let mut name = vec![0xff, b'.'];
            name.extend_from_slice(ext.as_bytes());
            assert!(!supports(Path::new(&std::ffi::OsString::from_vec(name))));
        }
    }

    #[test]
    fn every_builtin_language_and_markdown_alias_is_supported() {
        for language in SourceLanguage::ALL {
            assert!(supports(Path::new(&format!(
                "sample.{}",
                language.extension()
            ))));
        }
        for extension in [
            "java", "cs", "c", "h", "cc", "cpp", "cxx", "c++", "hh", "hpp", "hxx", "h++", "C", "H",
        ] {
            let path = format!("source.{extension}");
            assert!(supports(Path::new(&path)));
            assert!(crate::native_format::NativeLanguage::from_path(Path::new(&path)).is_some());
        }
        assert!(supports(Path::new("README.markdown")));
        for extension in ["html", "htm", "css", "scss", "yaml", "yml", "sql"] {
            assert!(supports(Path::new(&format!("sample.{extension}"))));
        }
        assert!(!supports(Path::new("sample.svelte")));
        assert!(!supports(Path::new("sample.less")));
        assert!(!supports(Path::new("Cargo.lock")));
    }
}
