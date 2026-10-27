use anyhow::{Result, bail};

use crate::dialect::DialectKind;

/// A top-level SQL statement found in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementChunk {
    /// Statement text with leading comments removed. The trailing semicolon is
    /// kept when it was present in the source.
    pub text: String,
    /// 1-based line of the first non-comment token.
    pub line: usize,
}

/// Split SQL into top-level statements without being fooled by semicolons in
/// strings, quoted identifiers, comments, or PostgreSQL dollar-quoted bodies.
pub fn split_statements(source: &str, dialect: DialectKind) -> Result<Vec<StatementChunk>> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b'-' if i + 1 < len && bytes[i + 1] == b'-' => {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                i = skip_block_comment(source, i)?;
            }
            b'\'' => {
                i = skip_single_quoted(source, i, dialect)?;
            }
            b'"' => {
                i = skip_double_quoted(source, i, dialect)?;
            }
            b'`' if dialect.supports_backticks() => {
                i = skip_backtick_quoted(source, i)?;
            }
            b'$' if dialect.supports_dollar_quotes() => {
                if let Some(tag) = dollar_quote_tag(source, i) {
                    let body_start = i + tag.len();
                    if let Some(relative) = source[body_start..].find(tag) {
                        i = body_start + relative + tag.len();
                    } else {
                        bail!(
                            "unterminated dollar-quoted string starting on line {}",
                            line_at(source, i)
                        );
                    }
                } else {
                    i += 1;
                }
            }
            b'#' if dialect == DialectKind::Mysql => {
                i += 1;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b';' => {
                push_chunk(source, start, i + 1, &mut chunks, dialect);
                start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }

    push_chunk(source, start, len, &mut chunks, dialect);
    Ok(chunks)
}

fn push_chunk(
    source: &str,
    start: usize,
    end: usize,
    chunks: &mut Vec<StatementChunk>,
    dialect: DialectKind,
) {
    let raw = &source[start..end];
    let offset = leading_trivia_len(raw, dialect);
    if offset >= raw.len() {
        return;
    }
    let text = raw[offset..].trim();
    if text.is_empty() || text.chars().all(|c| c == ';' || c.is_whitespace()) {
        return;
    }
    chunks.push(StatementChunk {
        text: text.to_string(),
        line: line_at(source, start + offset),
    });
}

fn line_at(source: &str, offset: usize) -> usize {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn skip_block_comment(source: &str, start: usize) -> Result<usize> {
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut i = start + 2;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Ok(i);
            }
        } else {
            i += 1;
        }
    }
    bail!(
        "unterminated block comment starting on line {}",
        line_at(source, start)
    )
}

fn skip_single_quoted(source: &str, start: usize, dialect: DialectKind) -> Result<usize> {
    let bytes = source.as_bytes();
    let escape_backslash = dialect.backslash_escapes() || is_escape_string_prefix(source, start);
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && escape_backslash {
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == b'\'' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                i += 2;
            } else {
                return Ok(i + 1);
            }
        } else {
            i += 1;
        }
    }
    bail!(
        "unterminated string literal starting on line {}",
        line_at(source, start)
    )
}

fn skip_double_quoted(source: &str, start: usize, dialect: DialectKind) -> Result<usize> {
    let bytes = source.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' && dialect.backslash_escapes() {
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == b'"' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                i += 2;
            } else {
                return Ok(i + 1);
            }
        } else {
            i += 1;
        }
    }
    bail!(
        "unterminated quoted identifier starting on line {}",
        line_at(source, start)
    )
}

fn skip_backtick_quoted(source: &str, start: usize) -> Result<usize> {
    let bytes = source.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'`' {
                i += 2;
            } else {
                return Ok(i + 1);
            }
        } else {
            i += 1;
        }
    }
    bail!(
        "unterminated backtick-quoted identifier starting on line {}",
        line_at(source, start)
    )
}

/// Return the complete dollar-quote tag (for example `$$` or `$body$`) that
/// starts at `start`, if the input has one.
fn dollar_quote_tag(source: &str, start: usize) -> Option<&str> {
    let bytes = source.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'$' {
        Some(&source[start..=i])
    } else {
        None
    }
}

/// PostgreSQL's `E'...'` strings allow backslash escapes; ordinary strings do
/// not. This keeps a trailing backslash in `'C:\'` from swallowing the quote.
fn is_escape_string_prefix(source: &str, quote: usize) -> bool {
    let bytes = source.as_bytes();
    if quote == 0 || !matches!(bytes[quote - 1], b'e' | b'E') {
        return false;
    }
    quote == 1 || !bytes[quote - 2].is_ascii_alphanumeric() && bytes[quote - 2] != b'_'
}

fn leading_trivia_len(raw: &str, dialect: DialectKind) -> usize {
    let bytes = raw.as_bytes();
    let mut i = 0;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if dialect == DialectKind::Mysql && i < bytes.len() && bytes[i] == b'#' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        break;
    }
    i
}

/// Collapse all whitespace to single spaces for display and JSON output.
pub fn normalize_statement(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate on a character boundary and mark the result with `...`.
pub fn truncate_statement(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let mut truncated: String = text.chars().take(max_chars - 3).collect();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_statements_and_tracks_lines() {
        let chunks = split_statements("SELECT 1;\n\nSELECT 2;\n", DialectKind::Postgres).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "SELECT 1;");
        assert_eq!(chunks[0].line, 1);
        assert_eq!(chunks[1].text, "SELECT 2;");
        assert_eq!(chunks[1].line, 3);
    }

    #[test]
    fn ignores_semicolons_in_strings_comments_and_dollar_quotes() {
        let sql = r#"
            INSERT INTO t VALUES ('a;b');
            -- a comment; with a semicolon
            SELECT 1;
            DO $$ BEGIN PERFORM 1; END $$;
            /* block; /* nested; */ still block; */
            SELECT 2;
        "#;
        let chunks = split_statements(sql, DialectKind::Postgres).unwrap();
        assert_eq!(chunks.len(), 4);
        assert!(chunks[2].text.starts_with("DO $$"));
        assert!(chunks[3].text.starts_with("SELECT 2"));
    }

    #[test]
    fn supports_tagged_dollar_quotes() {
        let sql = "CREATE FUNCTION f() RETURNS void AS $body$ BEGIN PERFORM 1; END; $body$ LANGUAGE plpgsql; SELECT 1;";
        let chunks = split_statements(sql, DialectKind::Postgres).unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].text.contains("$body$"));
    }

    #[test]
    fn postgres_escape_strings_are_handled() {
        let sql = r"SELECT E'it\'s; fine'; SELECT 2;";
        let chunks = split_statements(sql, DialectKind::Postgres).unwrap();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn standard_postgres_string_can_end_with_backslash() {
        let sql = r"SELECT 'C:\'; SELECT 2;";
        let chunks = split_statements(sql, DialectKind::Postgres).unwrap();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn mysql_backslash_escapes_are_handled() {
        let sql = r"SELECT 'it\'s; fine'; SELECT 2;";
        let chunks = split_statements(sql, DialectKind::Mysql).unwrap();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn mysql_hash_comments_are_ignored() {
        let sql = "# a comment; with a semicolon\nSELECT 1;\n# another; comment\nSELECT 2;";
        let chunks = split_statements(sql, DialectKind::Mysql).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "SELECT 1;");
        assert_eq!(chunks[0].line, 2);
    }

    #[test]
    fn mysql_backtick_identifiers_can_contain_semicolons() {
        let sql = "SELECT `odd;name` FROM t; SELECT 2;";
        let chunks = split_statements(sql, DialectKind::Mysql).unwrap();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn leading_comments_are_not_statement_text_but_line_is_correct() {
        let sql = "-- header\n/* block */\n\nDROP TABLE users;\n";
        let chunks = split_statements(sql, DialectKind::Postgres).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "DROP TABLE users;");
        assert_eq!(chunks[0].line, 4);
    }

    #[test]
    fn empty_and_comment_only_inputs_have_no_statements() {
        assert!(
            split_statements("  \n\t", DialectKind::Postgres)
                .unwrap()
                .is_empty()
        );
        assert!(
            split_statements("-- hello\n/* world */\n", DialectKind::Postgres)
                .unwrap()
                .is_empty()
        );
        assert!(
            split_statements(";;;", DialectKind::Postgres)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unterminated_literals_are_errors() {
        assert!(split_statements("SELECT 'oops;", DialectKind::Postgres).is_err());
        assert!(split_statements("SELECT $$oops;", DialectKind::Postgres).is_err());
        assert!(split_statements("SELECT 1; /* oops", DialectKind::Postgres).is_err());
        assert!(split_statements("SELECT \"oops;", DialectKind::Postgres).is_err());
    }

    #[test]
    fn normalization_and_truncation_are_multibyte_safe() {
        assert_eq!(
            normalize_statement("  SELECT\n\t1   "),
            "SELECT 1".to_string()
        );
        let text = "é".repeat(10);
        let truncated = truncate_statement(&text, 6);
        assert_eq!(truncated.chars().count(), 6);
        assert!(truncated.ends_with("..."));
        assert_eq!(truncate_statement("short", 10), "short");
    }
}
