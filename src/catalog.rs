//! File types supported by the bundled, version-pinned formatter plugins.
pub const EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "json", "jsonc", "md", "markdown", "toml",
];
pub const INDENT_WIDTHS: &[u8] = &[2, 4];
pub const SEMICOLONS: &[&str] = &["always", "asi"];

pub fn supports(path: &std::path::Path) -> bool {
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
    fn every_builtin_language_and_markdown_alias_is_supported() {
        for language in SourceLanguage::ALL {
            assert!(supports(Path::new(&format!(
                "sample.{}",
                language.extension()
            ))));
        }
        assert!(supports(Path::new("README.markdown")));
        assert!(!supports(Path::new("sample.yaml")));
        assert!(!supports(Path::new("Cargo.lock")));
    }
}
