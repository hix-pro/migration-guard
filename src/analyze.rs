use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use sqlparser::ast::Statement;
use sqlparser::parser::Parser;

use crate::dialect::DialectKind;
use crate::model::{FileReport, Report, Severity};
use crate::rules::{self, Outcome, RuleContext};
use crate::split::{normalize_statement, split_statements, truncate_statement};

/// Statement snippets are capped at this many characters in output.
pub const STATEMENT_LIMIT: usize = 200;

struct InputFile {
    path: PathBuf,
    display: String,
}

/// Find all `.sql` files under the supplied paths. Directories are scanned
/// recursively; results are sorted and duplicate files are removed.
fn collect_sql_files(paths: &[PathBuf]) -> Result<Vec<InputFile>> {
    let mut files = Vec::new();
    let mut seen_dirs = HashSet::new();
    let mut seen_files = HashSet::new();

    for path in paths {
        let metadata =
            fs::metadata(path).with_context(|| format!("cannot access `{}`", path.display()))?;
        if metadata.is_file() {
            if !has_sql_extension(path) {
                return Err(anyhow!("`{}` is not a .sql file", path.display()));
            }
            add_file(path, &mut files, &mut seen_files)?;
        } else if metadata.is_dir() {
            scan_directory(path, &mut files, &mut seen_dirs, &mut seen_files)?;
        } else {
            return Err(anyhow!(
                "`{}` is neither a regular file nor a directory",
                path.display()
            ));
        }
    }

    files.sort_by(|left, right| left.display.cmp(&right.display));
    Ok(files)
}

fn scan_directory(
    directory: &Path,
    files: &mut Vec<InputFile>,
    seen_dirs: &mut HashSet<PathBuf>,
    seen_files: &mut HashSet<PathBuf>,
) -> Result<()> {
    let canonical = fs::canonicalize(directory)
        .with_context(|| format!("cannot resolve directory `{}`", directory.display()))?;
    if !seen_dirs.insert(canonical) {
        return Ok(());
    }

    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("cannot read directory `{}`", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("cannot read directory `{}`", directory.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let metadata =
            fs::metadata(&path).with_context(|| format!("cannot inspect `{}`", path.display()))?;
        if metadata.is_dir() {
            scan_directory(&path, files, seen_dirs, seen_files)?;
        } else if metadata.is_file() && has_sql_extension(&path) {
            add_file(&path, files, seen_files)?;
        }
    }
    Ok(())
}

fn add_file(
    path: &Path,
    files: &mut Vec<InputFile>,
    seen_files: &mut HashSet<PathBuf>,
) -> Result<()> {
    let canonical =
        fs::canonicalize(path).with_context(|| format!("cannot resolve `{}`", path.display()))?;
    if seen_files.insert(canonical) {
        files.push(InputFile {
            path: path.to_path_buf(),
            display: path.to_string_lossy().into_owned(),
        });
    }
    Ok(())
}

fn has_sql_extension(path: &Path) -> bool {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sql"))
}

/// Analyze one file's contents. The file name is only used for reporting; this
/// function performs no I/O, which keeps the rule tests fast and deterministic.
pub fn analyze_source(
    dialect: DialectKind,
    file: impl Into<String>,
    source: &str,
) -> Result<FileReport> {
    let file = file.into();
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let chunks = split_statements(source, dialect)
        .with_context(|| format!("`{file}`: cannot split SQL statements"))?;
    let parser_dialect = dialect.parser_dialect();
    let mut report = FileReport {
        file: file.clone(),
        statements: 0,
        findings: Vec::new(),
    };
    let mut in_transaction = false;

    for chunk in chunks {
        let normalized = normalize_statement(&chunk.text);
        let statement = truncate_statement(&normalized, STATEMENT_LIMIT);

        match Parser::parse_sql(parser_dialect.as_ref(), &chunk.text) {
            Ok(parsed) => {
                for parsed_statement in &parsed {
                    let context = RuleContext {
                        dialect,
                        in_transaction,
                        file: &file,
                        line: chunk.line,
                        statement: &statement,
                    };
                    report.statements += 1;
                    match rules::analyze_statement(parsed_statement, &context) {
                        Outcome::Analyzed(mut findings) => {
                            report.findings.append(&mut findings);
                        }
                        Outcome::Unrecognized => report.findings.push(rules::unanalyzed(&context)),
                    }
                    update_transaction(parsed_statement, &mut in_transaction);
                }
            }
            Err(parse_error) => {
                let context = RuleContext {
                    dialect,
                    in_transaction,
                    file: &file,
                    line: chunk.line,
                    statement: &statement,
                };
                if let Some(mut findings) = rules::analyze_unsupported(&context, &chunk.text) {
                    report.statements += 1;
                    report.findings.append(&mut findings);
                } else {
                    return Err(anyhow!(
                        "{file}:{}: cannot parse statement: {parse_error}",
                        chunk.line
                    ));
                }
            }
        }
    }

    Ok(report)
}

/// Read and analyze every discovered file, then build the aggregate report.
pub fn build_report(
    dialect: DialectKind,
    paths: &[PathBuf],
    fail_on: Option<Severity>,
) -> Result<Report> {
    let inputs = collect_sql_files(paths)?;
    let mut reports = Vec::with_capacity(inputs.len());
    for input in inputs {
        let source = fs::read_to_string(&input.path)
            .with_context(|| format!("cannot read `{}`", input.display))?;
        reports.push(analyze_source(dialect, input.display, &source)?);
    }
    Ok(Report::new(dialect, reports, fail_on))
}

fn update_transaction(statement: &Statement, in_transaction: &mut bool) {
    match statement {
        Statement::StartTransaction { .. } => *in_transaction = true,
        Statement::Commit { chain, .. } => *in_transaction = *chain,
        Statement::Rollback {
            chain,
            savepoint: None,
            ..
        } => *in_transaction = *chain,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Finding;

    fn analyze(sql: &str) -> FileReport {
        analyze_source(DialectKind::Postgres, "test.sql", sql).unwrap()
    }

    fn analyze_as(dialect: DialectKind, sql: &str) -> FileReport {
        analyze_source(dialect, "test.sql", sql).unwrap()
    }

    fn findings(sql: &str) -> Vec<Finding> {
        analyze(sql).findings
    }

    fn rule_ids(sql: &str) -> Vec<&'static str> {
        findings(sql).iter().map(|finding| finding.rule).collect()
    }

    fn assert_rules(sql: &str, expected: &[&str]) {
        assert_eq!(rule_ids(sql), expected, "unexpected findings for {sql:?}");
    }

    #[test]
    fn drop_table_is_destructive() {
        let findings = findings("DROP TABLE users;");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "MG001");
        assert_eq!(findings[0].severity, Severity::Destructive);
        assert!(findings[0].detail.contains("users"));
        assert_eq!(findings[0].line, 1);
    }

    #[test]
    fn drop_column_and_truncate_are_destructive() {
        assert_rules("ALTER TABLE users DROP COLUMN email;", &["MG001"]);
        assert_rules(
            "ALTER TABLE users DROP COLUMN email, DROP COLUMN phone;",
            &["MG001", "MG001"],
        );
        assert_rules("TRUNCATE audit_log;", &["MG001"]);
        assert_rules("DROP TABLE IF EXISTS a, b;", &["MG001"]);
    }

    #[test]
    fn delete_without_where_is_destructive_but_filtered_delete_is_safe() {
        assert_rules("DELETE FROM sessions;", &["MG002"]);
        assert_rules(
            "DELETE FROM sessions USING users WHERE sessions.user_id = users.id;",
            &[],
        );
        assert_rules("DELETE FROM sessions WHERE expires_at < now();", &[]);
    }

    #[test]
    fn update_without_where_is_destructive_but_filtered_update_is_safe() {
        assert_rules("UPDATE users SET active = false;", &["MG003"]);
        assert_rules("UPDATE users SET active = false WHERE id = 42;", &[]);
    }

    #[test]
    fn column_type_changes_are_risky() {
        assert_rules(
            "ALTER TABLE users ALTER COLUMN age TYPE bigint;",
            &["MG004"],
        );
        assert_rules(
            "ALTER TABLE users ALTER COLUMN age SET DATA TYPE bigint;",
            &["MG004"],
        );
        assert_rules_as(
            DialectKind::Mysql,
            "ALTER TABLE users MODIFY COLUMN age bigint;",
            &["MG004"],
        );
    }

    fn assert_rules_as(dialect: DialectKind, sql: &str, expected: &[&str]) {
        assert_eq!(
            analyze_as(dialect, sql)
                .findings
                .iter()
                .map(|finding| finding.rule)
                .collect::<Vec<_>>(),
            expected,
            "unexpected findings for {sql:?} in {dialect:?}"
        );
    }

    #[test]
    fn adding_not_null_without_default_is_risky() {
        assert_rules(
            "ALTER TABLE users ADD COLUMN last_seen timestamptz NOT NULL;",
            &["MG005"],
        );
        assert_rules(
            "ALTER TABLE users ADD COLUMN last_seen timestamptz NOT NULL DEFAULT now();",
            &[],
        );
        assert_rules("ALTER TABLE users ADD COLUMN last_seen timestamptz;", &[]);
    }

    #[test]
    fn setting_not_null_is_risky() {
        assert_rules(
            "ALTER TABLE users ALTER COLUMN email SET NOT NULL;",
            &["MG006"],
        );
        assert_rules("ALTER TABLE users ALTER COLUMN email DROP NOT NULL;", &[]);
    }

    #[test]
    fn create_index_is_risky_only_on_postgres_and_only_without_concurrently() {
        assert_rules("CREATE INDEX idx_users_email ON users (email);", &["MG007"]);
        assert_rules(
            "CREATE UNIQUE INDEX idx_users_email ON users (email);",
            &["MG007"],
        );
        assert_rules(
            "CREATE INDEX CONCURRENTLY idx_users_email ON users (email);",
            &[],
        );
        for dialect in [
            DialectKind::Mysql,
            DialectKind::Sqlite,
            DialectKind::Generic,
        ] {
            assert_rules_as(
                dialect,
                "CREATE INDEX idx_users_email ON users (email);",
                &[],
            );
        }
    }

    #[test]
    fn drop_index_is_risky_only_on_postgres_and_concurrently_is_accepted() {
        assert_rules("DROP INDEX idx_users_email;", &["MG008"]);
        assert_rules("DROP INDEX CONCURRENTLY IF EXISTS idx_users_email;", &[]);
        assert_rules_as(
            DialectKind::Mysql,
            "DROP INDEX idx_users_email ON users;",
            &[],
        );
        assert_rules_as(DialectKind::Sqlite, "DROP INDEX idx_users_email;", &[]);
    }

    #[test]
    fn foreign_key_without_not_valid_is_risky_on_postgres() {
        assert_rules(
            "ALTER TABLE orders ADD CONSTRAINT fk FOREIGN KEY (user_id) REFERENCES users (id);",
            &["MG009"],
        );
        assert_rules(
            "ALTER TABLE orders ADD CONSTRAINT fk FOREIGN KEY (user_id) REFERENCES users (id) NOT VALID;",
            &[],
        );
        assert_rules_as(
            DialectKind::Mysql,
            "ALTER TABLE orders ADD CONSTRAINT fk FOREIGN KEY (user_id) REFERENCES users (id);",
            &[],
        );
    }

    #[test]
    fn renames_are_warnings() {
        assert_rules("ALTER TABLE users RENAME TO people;", &["MG010"]);
        assert_rules(
            "ALTER TABLE users RENAME COLUMN email TO email_address;",
            &["MG010"],
        );
        assert_rules_as(
            DialectKind::Mysql,
            "RENAME TABLE users TO people;",
            &["MG010"],
        );
        assert!(
            findings("ALTER TABLE users RENAME TO people;")[0]
                .severity
                .eq(&Severity::Warning)
        );
    }

    #[test]
    fn concurrent_index_inside_transaction_is_a_warning() {
        let sql = "BEGIN;\nCREATE INDEX CONCURRENTLY idx ON users (email);\nCOMMIT;";
        let report = analyze(sql);
        assert_eq!(report.statements, 3);
        assert_eq!(
            report
                .findings
                .iter()
                .map(|finding| finding.rule)
                .collect::<Vec<_>>(),
            vec!["MG011"]
        );
        assert_eq!(report.findings[0].severity, Severity::Warning);
        assert!(report.findings[0].detail.contains("transaction"));
    }

    #[test]
    fn transaction_state_tracks_commit_rollback_and_savepoints() {
        let after_commit = analyze(
            "BEGIN;\nCREATE INDEX CONCURRENTLY a ON t (x);\nCOMMIT;\nCREATE INDEX CONCURRENTLY b ON t (y);",
        );
        assert_eq!(after_commit.findings.len(), 1);
        assert_eq!(after_commit.statements, 4);

        let after_rollback = analyze(
            "START TRANSACTION;\nCREATE INDEX CONCURRENTLY a ON t (x);\nROLLBACK;\nCREATE INDEX CONCURRENTLY b ON t (y);",
        );
        assert_eq!(after_rollback.findings.len(), 1);
        assert_eq!(after_rollback.statements, 4);

        let savepoint = analyze(
            "BEGIN;\nCREATE INDEX CONCURRENTLY a ON t (x);\nSAVEPOINT s;\nROLLBACK TO SAVEPOINT s;\nCREATE INDEX CONCURRENTLY b ON t (y);\nCOMMIT;",
        );
        assert_eq!(
            savepoint
                .findings
                .iter()
                .filter(|finding| finding.rule == "MG011")
                .count(),
            2
        );
    }

    #[test]
    fn mysql_does_not_get_postgres_concurrent_transaction_rule() {
        assert_rules_as(
            DialectKind::Mysql,
            "BEGIN;\nCREATE INDEX CONCURRENTLY idx ON users (email);\nCOMMIT;",
            &[],
        );
    }

    #[test]
    fn vacuum_full_cluster_and_lock_are_risky_on_postgres() {
        assert_rules("VACUUM FULL users;", &["MG012"]);
        assert_rules("VACUUM (FULL) users;", &["MG012"]);
        assert_rules("CLUSTER users USING idx_users_email;", &["MG012"]);
        assert_rules("CLUSTER;", &["MG012"]);
        assert_rules("LOCK TABLE users IN ACCESS EXCLUSIVE MODE;", &["MG012"]);
        assert_rules("VACUUM users;", &[]);
    }

    #[test]
    fn locking_rules_do_not_fire_for_other_dialects() {
        assert_rules_as(DialectKind::Mysql, "VACUUM FULL users;", &[]);
        assert_rules_as(DialectKind::Mysql, "LOCK TABLE users;", &[]);
    }

    #[test]
    fn do_blocks_are_unanalyzable_warnings() {
        let sql = "DO $$\nBEGIN\n  DELETE FROM users;\nEND\n$$;";
        let report = analyze(sql);
        assert_eq!(report.statements, 1);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].rule, "MG013");
        assert_eq!(report.findings[0].severity, Severity::Warning);
        assert!(report.findings[0].detail.contains("DO block"));
    }

    #[test]
    fn unknown_statement_types_are_unanalyzable_warnings() {
        assert_rules("CALL cleanup_old_sessions();", &["MG013"]);
    }

    #[test]
    fn safe_statements_produce_no_findings() {
        let sql = "
            CREATE TABLE users (id bigint PRIMARY KEY, email text);
            ALTER TABLE users ADD COLUMN nickname text;
            INSERT INTO users (id, email) VALUES (1, 'a@example.com');
            CREATE VIEW user_emails AS SELECT id, email FROM users;
            CREATE INDEX CONCURRENTLY idx_users_email ON users (email);
            SET statement_timeout = '5s';
            COMMENT ON TABLE users IS 'accounts';
        ";
        let report = analyze(sql);
        assert!(
            report.findings.is_empty(),
            "unexpected findings: {:?}",
            report.findings
        );
        assert_eq!(report.statements, 7);
    }

    #[test]
    fn statement_text_is_truncated_and_whitespace_normalized() {
        let long_name = "x".repeat(260);
        let report = analyze(&format!("DROP\n  TABLE {long_name};"));
        assert_eq!(report.findings.len(), 1);
        let statement = &report.findings[0].statement;
        assert_eq!(statement.chars().count(), STATEMENT_LIMIT);
        assert!(statement.ends_with("..."));
        assert!(!statement.contains('\n'));
    }

    #[test]
    fn line_numbers_point_at_the_statement_not_leading_comments() {
        let report = analyze("-- a comment\n\nDROP TABLE users;\n");
        assert_eq!(report.findings[0].line, 3);
    }

    #[test]
    fn byte_order_mark_is_ignored() {
        let report = analyze("\u{feff}DROP TABLE users;");
        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn parse_errors_and_unterminated_literals_are_errors() {
        let parse_error = analyze_source(
            DialectKind::Postgres,
            "broken.sql",
            "CREATE TABLE broken (id bigint;",
        )
        .unwrap_err();
        assert!(parse_error.to_string().contains("cannot parse statement"));

        let split_error =
            analyze_source(DialectKind::Postgres, "broken.sql", "SELECT 'unterminated;")
                .unwrap_err();
        assert!(split_error.to_string().contains("cannot split"));
    }

    #[test]
    fn multiple_statements_are_counted() {
        let report = analyze("SELECT 1; SELECT 2; SELECT 3;");
        assert_eq!(report.statements, 3);
        assert!(report.findings.is_empty());
    }
}
