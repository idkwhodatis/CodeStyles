//! Deliberately small P0 catalog; no speculative schema-driven form engine.
pub const EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "json", "jsonc"];
pub const INDENT_WIDTHS: &[u8] = &[2, 4];
pub const SEMICOLONS: &[&str] = &["always", "asi"];

pub fn supports(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| EXTENSIONS.contains(&s))
}
