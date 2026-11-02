//! A deliberately narrow, lexical SQL scope gate, not SQL syntax validation.
//!
//! Only explicit PostgreSQL/MySQL/SQLite dialects are admitted. Transaction
//! BEGIN, procedural keywords used as unquoted names, dollar-quoted strings,
//! executable MySQL comments, and client commands are intentionally excluded.
//! Quoted names and ordinary comments do not trigger the keyword restriction.

pub fn validate_source(source: &str, dialect: &str) -> Result<(), String> {
    if !matches!(dialect, "postgresql" | "mysql" | "sqlite") {
        return Err("SQL scope requires an explicit postgresql, mysql, or sqlite dialect.".into());
    }
    if source.len() > 32 * 1024 * 1024 || source.contains('\0') {
        return Err("SQL source exceeds the safety limit or contains NUL.".into());
    }
    // Match the standalone source gate, including deliberate false positives.
    if ["{{", "{%", "{#", "<%", "<?"]
        .iter()
        .any(|marker| source.contains(marker))
    {
        return Err("SQL templates are outside the initial supported scope.".into());
    }
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut line_start = true;
    let mut cases = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            if matches!(c, b'\n' | b'\r') {
                line_start = true;
            }
            i += 1;
            continue;
        }
        if c == b'\\' || (line_start && c == b'.') {
            return Err("SQL client commands are outside the initial supported scope.".into());
        }
        // PostgreSQL accepts -- without following whitespace; MySQL does not.
        let dash_comment = bytes.get(i..i + 2) == Some(b"--")
            && (dialect != "mysql" || bytes.get(i + 2).is_none_or(|c| c.is_ascii_whitespace()));
        if dash_comment || (dialect == "mysql" && c == b'#') {
            i += if dash_comment { 2 } else { 1 };
            while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                i += 1;
            }
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"/*") {
            if bytes.get(i + 2) == Some(&b'!')
                || bytes
                    .get(i..i + 4)
                    .is_some_and(|s| s.eq_ignore_ascii_case(b"/*M!"))
            {
                return Err(
                    "Executable SQL comments are outside the initial supported scope.".into(),
                );
            }
            i += 2;
            let mut depth = 1usize;
            while i < bytes.len() && depth > 0 {
                if dialect == "postgresql" && bytes.get(i..i + 2) == Some(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes.get(i..i + 2) == Some(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    if matches!(bytes[i], b'\n' | b'\r') {
                        line_start = true;
                    }
                    i += 1;
                }
            }
            if depth != 0 {
                return Err("Unclosed SQL block comment is outside the supported scope.".into());
            }
            continue;
        }
        if c == b'$' {
            return Err(
                "SQL dollar quoting/parameters are outside the initial supported scope.".into(),
            );
        }
        if matches!(c, b'\'' | b'"' | b'`' | b'[') {
            if (c == b'`' && dialect == "postgresql") || (c == b'[' && dialect != "sqlite") {
                return Err("Quoted SQL syntax does not match the selected dialect.".into());
            }
            let closing = if c == b'[' { b']' } else { c };
            let pg_escape = dialect == "postgresql"
                && c == b'\''
                && i > 0
                && matches!(bytes[i - 1], b'e' | b'E')
                && (i < 2 || !is_word(bytes[i - 2]));
            let backslash = pg_escape || (dialect == "mysql" && matches!(c, b'\'' | b'"'));
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if backslash && bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == closing {
                    if c != b'[' && bytes.get(i + 1) == Some(&closing) {
                        i += 2;
                    } else {
                        i += 1;
                        closed = true;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if !closed {
                return Err("Unclosed SQL quoted text is outside the supported scope.".into());
            }
            line_start = false;
            continue;
        }
        if is_word(c) {
            let start = i;
            while i < bytes.len() && is_word(bytes[i]) {
                i += 1;
            }
            let word = &source[start..i];
            if [
                "DO",
                "DELIMITER",
                "DECLARE",
                "BEGIN",
                "PROCEDURE",
                "FUNCTION",
                "TRIGGER",
                "EVENT",
                "LOOP",
                "WHILE",
                "REPEAT",
                "CALL",
                "EXEC",
                "EXECUTE",
                "GO",
                "SOURCE",
            ]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
            {
                return Err(format!(
                    "SQL keyword {word} is outside the initial non-procedural scope (including transaction BEGIN and unquoted names)."
                ));
            }
            if word.eq_ignore_ascii_case("CASE") {
                cases += 1;
            } else if word.eq_ignore_ascii_case("END") {
                if cases == 0 {
                    return Err("SQL END outside a CASE expression is unsupported.".into());
                }
                cases -= 1;
            }
        } else {
            if c == b';' {
                cases = 0;
            }
            // Advance whole UTF-8 characters; never slice inside a codepoint.
            i += source[i..].chars().next().expect("in bounds").len_utf8();
        }
        line_start = false;
    }
    Ok(())
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::validate_source;

    #[test]
    fn normal_queries_and_case_are_allowed() {
        for dialect in ["postgresql", "mysql", "sqlite"] {
            assert!(
                validate_source(
                    "SELECT CASE WHEN x = 1 THEN 'BEGIN' ELSE 'END' END FROM t;",
                    dialect
                )
                .is_ok()
            );
            assert!(
                validate_source(
                    "SELECT 'FUNCTION', \"TRIGGER\" /* BEGIN */; -- DO\n",
                    dialect
                )
                .is_ok()
            );
        }
        assert!(validate_source("SELECT '中文', café FROM t;", "postgresql").is_ok());
    }

    #[test]
    fn procedural_and_client_scope_is_rejected() {
        for source in [
            "DO $$ BEGIN END $$;",
            "CREATE OR REPLACE FUNCTION f()",
            "CREATE DEFINER = 'u' PROCEDURE p()",
            "CREATE TRIGGER t",
            "BEGIN; SELECT 1; COMMIT;",
            "DECLARE x int;",
            "DELIMITER //",
            "SELECT 1;\n\\i file.sql",
            "  .read file.sql",
            "END;",
            "/*! CREATE PROCEDURE p() */",
            "/*M! BEGIN */",
        ] {
            assert!(validate_source(source, "mysql").is_err(), "{source}");
        }
        assert!(validate_source("SELECT $1", "postgresql").is_err());
    }

    #[test]
    fn quotes_and_comments_follow_the_dialect() {
        assert!(validate_source("SELECT E'escaped\\' BEGIN';", "postgresql").is_ok());
        assert!(validate_source("SELECT 'escaped\\' BEGIN';", "mysql").is_ok());
        assert!(validate_source("SELECT 'escaped\\' BEGIN';", "postgresql").is_err());
        assert!(validate_source("SELECT `FUNCTION`; # BEGIN\n", "mysql").is_ok());
        assert!(validate_source("SELECT [FUNCTION], 'it''s BEGIN';", "sqlite").is_ok());
        assert!(validate_source("/* outer /* nested */ BEGIN */ SELECT 1", "postgresql").is_ok());
        assert!(validate_source("/* outer /* nested */ BEGIN */ SELECT 1", "mysql").is_err());
        assert!(validate_source("SELECT 1 --x BEGIN", "mysql").is_err());
        assert!(validate_source("SELECT 1 --x BEGIN", "postgresql").is_ok());
    }

    #[test]
    fn fail_closed_without_claiming_syntax_validation() {
        for source in [
            "SELECT 'oops",
            "/* comment",
            "SELECT {{ref('x')}}",
            "SELECT $$x$$",
            "SELECT '\0'",
        ] {
            assert!(validate_source(source, "postgresql").is_err());
        }
        assert!(validate_source("SELECT FROM WHERE;", "postgresql").is_ok());
        assert!(validate_source("SELECT 1", "generic").is_err());
    }
}
