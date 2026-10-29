use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_migration-guard"))
        .current_dir(manifest_dir())
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .expect("failed to run migration-guard")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_exit(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "unexpected exit code\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

#[test]
fn safe_file_passes_with_no_findings() {
    let output = run(&["check", "tests/fixtures/safe.sql"]);
    assert_exit(&output, 0);
    let text = stdout(&output);
    assert!(text.contains("no findings"));
    assert!(text.contains("Summary: 1 file, 4 statements, 0 findings"));
    assert!(text.contains("Result: PASS — no findings."));
    assert!(
        !text.contains("\x1b["),
        "piped output must not contain ANSI"
    );
}

#[test]
fn report_only_is_the_default_even_for_destructive_findings() {
    let output = run(&["check", "tests/fixtures/destructive.sql"]);
    assert_exit(&output, 0);
    let text = stdout(&output);
    assert!(text.contains("MG001"));
    assert!(text.contains("MG002"));
    assert!(text.contains("MG003"));
    assert!(text.contains(
        "Summary: 1 file, 5 statements, 5 findings (5 destructive, 0 risky, 0 warnings)"
    ));
    assert!(text.contains("Result: PASS — 5 findings reported (report-only"));
}

#[test]
fn fail_on_destructive_exits_two_for_destructive_findings() {
    let output = run(&[
        "check",
        "tests/fixtures/destructive.sql",
        "--fail-on",
        "destructive",
    ]);
    assert_exit(&output, 2);
    let text = stdout(&output);
    assert!(text.contains("Result: FAIL — 5 findings at or above destructive."));
}

#[test]
fn destructive_findings_meet_any_lower_threshold() {
    for threshold in ["risky", "warning"] {
        let output = run(&[
            "check",
            "tests/fixtures/destructive.sql",
            "--fail-on",
            threshold,
        ]);
        assert_exit(&output, 2);
    }
}

#[test]
fn risky_findings_do_not_trip_a_destructive_threshold() {
    let output = run(&[
        "check",
        "tests/fixtures/risky.sql",
        "--fail-on",
        "destructive",
    ]);
    assert_exit(&output, 0);
    let text = stdout(&output);
    assert!(text.contains("Result: PASS — no findings at or above destructive."));
}

#[test]
fn risky_and_warning_thresholds_fail_on_risky_fixture() {
    for threshold in ["risky", "warning"] {
        let output = run(&["check", "tests/fixtures/risky.sql", "--fail-on", threshold]);
        assert_exit(&output, 2);
    }
}

#[test]
fn warning_findings_only_trip_a_warning_threshold() {
    let default = run(&["check", "tests/fixtures/unanalyzable.sql"]);
    assert_exit(&default, 0);

    let risky = run(&[
        "check",
        "tests/fixtures/unanalyzable.sql",
        "--fail-on",
        "risky",
    ]);
    assert_exit(&risky, 0);
    assert!(stdout(&risky).contains("no findings at or above risky"));

    let warning = run(&[
        "check",
        "tests/fixtures/unanalyzable.sql",
        "--fail-on",
        "warning",
    ]);
    assert_exit(&warning, 2);
    assert!(stdout(&warning).contains("Result: FAIL — 2 findings at or above warning."));
}

#[test]
fn json_output_has_the_documented_shape() {
    let output = run(&[
        "check",
        "tests/fixtures/tree",
        "--format",
        "json",
        "--fail-on",
        "destructive",
    ]);
    assert_exit(&output, 2);
    let value: Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["tool"], "migration-guard");
    assert_eq!(value["dialect"], "postgres");
    assert_eq!(value["fail_on"], "destructive");
    assert_eq!(value["threshold_met"], true);

    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0]["file"], "tests/fixtures/tree/a_safe.sql");
    assert_eq!(files[1]["file"], "tests/fixtures/tree/b_destructive.sql");
    assert_eq!(files[2]["file"], "tests/fixtures/tree/nested/c_risky.sql");
    assert_eq!(files[0]["findings"].as_array().unwrap().len(), 0);

    let summary = &value["summary"];
    assert_eq!(summary["files_checked"], 3);
    assert_eq!(summary["statements_analyzed"], 3);
    assert_eq!(summary["findings"], 2);
    assert_eq!(summary["destructive"], 1);
    assert_eq!(summary["risky"], 1);
    assert_eq!(summary["warnings"], 0);

    let finding = &files[1]["findings"][0];
    for key in [
        "rule",
        "severity",
        "file",
        "line",
        "statement",
        "detail",
        "suggestion",
    ] {
        assert!(finding.get(key).is_some(), "missing JSON key {key}");
    }
    assert_eq!(finding["rule"], "MG001");
    assert_eq!(finding["severity"], "destructive");
    assert!(finding["statement"].as_str().unwrap().chars().count() <= 200);
    assert!(!stdout(&output).contains("\x1b["));
}

#[test]
fn json_report_only_uses_null_fail_on() {
    let output = run(&["check", "tests/fixtures/risky.sql", "--format", "json"]);
    assert_exit(&output, 0);
    let value: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(value["fail_on"].is_null());
    assert_eq!(value["threshold_met"], false);
    assert_eq!(value["summary"]["findings"], 5);
    assert_eq!(value["summary"]["warnings"], 1);
}

#[test]
fn sarif_output_is_valid_and_preserves_the_exit_code() {
    let output = run(&[
        "check",
        "tests/fixtures/risky.sql",
        "--format",
        "sarif",
        "--fail-on",
        "risky",
    ]);
    assert_exit(&output, 2);

    let value: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["version"], "2.1.0");
    assert_eq!(
        value["runs"][0]["tool"]["driver"]["name"],
        "Migration Guard"
    );
    let results = value["runs"][0]["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["ruleId"], "MG005");
    assert_eq!(
        results[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "tests/fixtures/risky.sql"
    );
}

#[test]
fn markdown_output_is_grouped_and_has_a_result_line() {
    let output = run(&[
        "check",
        "tests/fixtures/risky.sql",
        "--format",
        "markdown",
        "--fail-on",
        "risky",
    ]);
    assert_exit(&output, 2);
    let text = stdout(&output);
    assert!(text.starts_with("# Migration Guard report"));
    assert!(text.contains("## `tests/fixtures/risky.sql`"));
    assert!(text.contains("### risky · MG005 · line 1"));
    assert!(text.contains("**Suggestion:**"));
    assert!(text.contains("**Result:** FAIL — 4 findings at or above risky."));
}

#[test]
fn multiple_statements_and_transaction_state_are_tracked() {
    let output = run(&["check", "tests/fixtures/transaction.sql"]);
    assert_exit(&output, 0);
    let text = stdout(&output);
    assert!(text.contains("(5 statements)"));
    assert_eq!(text.matches("MG011").count(), 1);
    assert!(!text.contains("MG008"));
    assert!(
        text.contains(
            "Summary: 1 file, 5 statements, 1 finding (0 destructive, 0 risky, 1 warning)"
        )
    );
}

#[test]
fn mysql_dialect_does_not_claim_postgres_only_index_and_fk_rules() {
    let output = run(&[
        "check",
        "tests/fixtures/dialect_mysql.sql",
        "--dialect",
        "mysql",
        "--fail-on",
        "risky",
    ]);
    assert_exit(&output, 2);
    let text = stdout(&output);
    assert!(!text.contains("MG007"));
    assert!(!text.contains("MG008"));
    assert!(!text.contains("MG009"));
    assert!(text.contains("MG005"));
    assert!(text.contains("MG010"));
    assert!(
        text.contains(
            "Summary: 1 file, 5 statements, 2 findings (0 destructive, 1 risky, 1 warning)"
        )
    );
}

#[test]
fn parse_errors_exit_one_and_explain_the_file() {
    let output = run(&["check", "tests/fixtures/parse_error.sql"]);
    assert_exit(&output, 1);
    assert!(stdout(&output).is_empty());
    let error = stderr(&output);
    assert!(error.contains("migration-guard: error:"));
    assert!(error.contains("tests/fixtures/parse_error.sql:1"));
    assert!(error.contains("cannot parse statement"));
}

#[test]
fn missing_paths_exit_one() {
    let output = run(&["check", "tests/fixtures/does-not-exist.sql"]);
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("cannot access"));
}

#[test]
fn explicit_non_sql_files_are_rejected() {
    let output = run(&["check", "tests/fixtures/empty/README.txt"]);
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("is not a .sql file"));
}

#[test]
fn invalid_usage_exits_one_not_two() {
    let output = run(&["check", "tests/fixtures/safe.sql", "--dialect", "oracle"]);
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("invalid value"));
}

#[test]
fn directory_without_sql_files_is_a_clean_empty_report() {
    let output = run(&["check", "tests/fixtures/empty"]);
    assert_exit(&output, 0);
    let text = stdout(&output);
    assert!(text.contains("No .sql files found."));
    assert!(text.contains("Summary: 0 files, 0 statements, 0 findings"));
    assert!(text.contains("Result: PASS — no findings."));
}

#[test]
fn help_and_version_exit_zero() {
    let help = run(&["--help"]);
    assert_exit(&help, 0);
    assert!(stdout(&help).contains("Usage:"));

    let check_help = run(&["check", "--help"]);
    assert_exit(&check_help, 0);
    assert!(stdout(&check_help).contains("PATH"));

    let version = run(&["--version"]);
    assert_exit(&version, 0);
    assert!(stdout(&version).contains(&format!("migration-guard {}", env!("CARGO_PKG_VERSION"))));
}

struct TempTree(PathBuf);

impl TempTree {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("migration-guard-cli-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn root_str(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn discover_lists_frameworks_in_text_and_json() {
    let tree = TempTree::new();
    tree.write("prisma/migrations/20240101_init/migration.sql", "SELECT 1;");
    tree.write("db/migration/V1__init.sql", "SELECT 1;");
    tree.write("node_modules/pkg/migrations/ignored.sql", "SELECT 1;");

    let text = run(&["discover", tree.root_str()]);
    assert_exit(&text, 0);
    let output = stdout(&text);
    assert!(output.contains("discovery"));
    assert!(output.contains("Prisma"));
    assert!(output.contains("prisma/migrations"));
    assert!(output.contains("Flyway"));
    assert!(output.contains("db/migration"));
    assert!(!output.contains("node_modules"));
    assert!(!output.contains("ignored.sql"));

    let json = run(&["discover", tree.root_str(), "--format", "json"]);
    assert_exit(&json, 0);
    let value: Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["tool"], "migration-guard");
    let directories = value["directories"].as_array().unwrap();
    assert_eq!(directories.len(), 2);
    assert_eq!(directories[0]["path"], "db/migration");
    assert_eq!(directories[0]["framework"], "flyway");
    assert_eq!(directories[0]["sql_files"], 1);
    assert_eq!(directories[1]["path"], "prisma/migrations");
    assert_eq!(directories[1]["framework"], "prisma");
    assert_eq!(directories[1]["sql_files"], 1);
}

#[test]
fn check_without_paths_discovers_migrations_and_preserves_the_exit_code() {
    let tree = TempTree::new();
    tree.write(
        "prisma/migrations/20240101_init/migration.sql",
        "DROP TABLE sessions;",
    );
    tree.write(
        "db/migration/V1__init.sql",
        "CREATE TABLE users (id bigint);",
    );
    tree.write(
        "node_modules/pkg/migrations/ignored.sql",
        "DROP TABLE ignored;",
    );

    let output = run(&[
        "check",
        "--root",
        tree.root_str(),
        "--fail-on",
        "destructive",
    ]);
    assert_exit(&output, 2);
    let text = stdout(&output);
    assert!(text.contains("2 files"));
    assert!(text.contains("prisma/migrations/20240101_init/migration.sql"));
    assert!(text.contains("db/migration/V1__init.sql"));
    assert!(!text.contains("node_modules"));
    assert!(text.contains("MG001"));
}

#[test]
fn check_without_paths_fails_when_nothing_is_discovered() {
    let tree = TempTree::new();
    tree.write("docs/readme.md", "no migrations here");

    let output = run(&["check", "--root", tree.root_str()]);
    assert_exit(&output, 1);
    assert!(stdout(&output).is_empty());
    let error = stderr(&output);
    assert!(error.contains("no SQL migration directories found"));
    assert!(error.contains("migration-guard discover"));
}

#[test]
fn discover_with_no_directories_is_a_clean_result() {
    let tree = TempTree::new();
    tree.write("docs/readme.md", "no migrations here");

    let output = run(&["discover", tree.root_str()]);
    assert_exit(&output, 0);
    assert!(stdout(&output).contains("No SQL migration directories found."));
}

#[test]
fn discovery_root_conflicts_with_explicit_paths() {
    let output = run(&["check", "tests/fixtures/safe.sql", "--root", "."]);
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("cannot be used with"));
}
