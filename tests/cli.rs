use std::path::PathBuf;
use std::process::{Command, Output};

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
