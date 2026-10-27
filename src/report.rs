use std::fmt::Write as _;

use anyhow::Result;

use crate::cli::OutputFormat;
use crate::model::Report;
use crate::style::Style;

pub fn render(report: &Report, format: OutputFormat, style: Style) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(report, style)),
        OutputFormat::Json => render_json(report),
        OutputFormat::Markdown => Ok(render_markdown(report)),
    }
}

fn render_json(report: &Report) -> Result<String> {
    let mut output = serde_json::to_string_pretty(report)?;
    output.push('\n');
    Ok(output)
}

fn render_text(report: &Report, style: Style) -> String {
    let mut output = String::new();
    let file_count = report.summary.files_checked;
    let header = format!(
        "{}  ·  {}  ·  {} {}",
        style.bold(&format!("migration-guard {}", report.version)),
        report.dialect.as_str(),
        file_count,
        plural(file_count, "file", "files")
    );
    writeln!(output, "{header}").unwrap();
    writeln!(output).unwrap();

    if report.files.is_empty() {
        writeln!(output, "No .sql files found.").unwrap();
        writeln!(output).unwrap();
    }

    for file in &report.files {
        let statement_count = file.statements;
        writeln!(
            output,
            "{}  {}",
            style.bold(&file.file),
            style.dim(&format!(
                "({statement_count} {})",
                plural(statement_count, "statement", "statements")
            ))
        )
        .unwrap();

        if file.findings.is_empty() {
            writeln!(output, "  {}", style.dim("no findings")).unwrap();
            writeln!(output).unwrap();
            continue;
        }

        for finding in &file.findings {
            let padded = format!("{:<11}", finding.severity.as_str());
            writeln!(
                output,
                "  {}  {}  line {}",
                style.severity(finding.severity, &padded),
                style.bold(finding.rule),
                finding.line
            )
            .unwrap();
            writeln!(output, "      {}", style.dim(&finding.statement)).unwrap();
            writeln!(output, "      {} {}", style.bold("Why:"), finding.detail).unwrap();
            writeln!(
                output,
                "      {} {}",
                style.bold("Fix:"),
                finding.suggestion
            )
            .unwrap();
            writeln!(output).unwrap();
        }
    }

    writeln!(output, "{}", style.bold(&summary_text(report))).unwrap();
    let (result, passed) = result_text(report);
    let result = if passed {
        style.pass(&result)
    } else {
        style.fail(&result)
    };
    writeln!(output, "{result}").unwrap();
    output
}

fn render_markdown(report: &Report) -> String {
    let mut output = String::new();
    writeln!(output, "# Migration Guard report").unwrap();
    writeln!(output).unwrap();
    writeln!(output, "- **Tool:** migration-guard {}", report.version).unwrap();
    writeln!(output, "- **Dialect:** {}", report.dialect.as_str()).unwrap();
    writeln!(
        output,
        "- **Fail on:** {}",
        report.fail_on.map_or_else(
            || "report only".to_string(),
            |severity| severity.to_string()
        )
    )
    .unwrap();
    writeln!(output, "- **Files:** {}", report.summary.files_checked).unwrap();
    writeln!(
        output,
        "- **Statements:** {}",
        report.summary.statements_analyzed
    )
    .unwrap();
    writeln!(output).unwrap();

    if report.files.is_empty() {
        writeln!(output, "_No .sql files found._").unwrap();
        writeln!(output).unwrap();
    }

    for file in &report.files {
        writeln!(output, "## `{}`", file.file).unwrap();
        writeln!(output).unwrap();
        if file.findings.is_empty() {
            writeln!(output, "_No findings._").unwrap();
            writeln!(output).unwrap();
            continue;
        }

        for finding in &file.findings {
            writeln!(
                output,
                "### {} · {} · line {}",
                finding.severity, finding.rule, finding.line
            )
            .unwrap();
            writeln!(output).unwrap();
            writeln!(output, "{}", code_block(&finding.statement)).unwrap();
            writeln!(output).unwrap();
            writeln!(output, "**Why:** {}", finding.detail).unwrap();
            writeln!(output).unwrap();
            writeln!(output, "**Suggestion:** {}", finding.suggestion).unwrap();
            writeln!(output).unwrap();
        }
    }

    writeln!(output, "## Summary").unwrap();
    writeln!(output).unwrap();
    writeln!(output, "| Metric | Count |").unwrap();
    writeln!(output, "| --- | ---: |").unwrap();
    writeln!(
        output,
        "| Files checked | {} |",
        report.summary.files_checked
    )
    .unwrap();
    writeln!(
        output,
        "| Statements | {} |",
        report.summary.statements_analyzed
    )
    .unwrap();
    writeln!(output, "| Findings | {} |", report.summary.findings).unwrap();
    writeln!(output, "| Destructive | {} |", report.summary.destructive).unwrap();
    writeln!(output, "| Risky | {} |", report.summary.risky).unwrap();
    writeln!(output, "| Warnings | {} |", report.summary.warnings).unwrap();
    writeln!(output).unwrap();
    let (result, _) = result_text(report);
    let result = result.strip_prefix("Result: ").unwrap_or(&result);
    writeln!(output, "**Result:** {result}").unwrap();
    output
}

fn summary_text(report: &Report) -> String {
    let summary = report.summary;
    format!(
        "Summary: {} {}, {} {}, {} {} ({} destructive, {} risky, {} {})",
        summary.files_checked,
        plural(summary.files_checked, "file", "files"),
        summary.statements_analyzed,
        plural(summary.statements_analyzed, "statement", "statements"),
        summary.findings,
        plural(summary.findings, "finding", "findings"),
        summary.destructive,
        summary.risky,
        summary.warnings,
        plural(summary.warnings, "warning", "warnings")
    )
}

/// Returns the final result line and whether the run should be considered a
/// pass by the user-facing threshold.
fn result_text(report: &Report) -> (String, bool) {
    match report.fail_on {
        None if report.summary.findings == 0 => ("Result: PASS — no findings.".to_string(), true),
        None => (
            format!(
                "Result: PASS — {} {} reported (report-only; no failure threshold).",
                report.summary.findings,
                plural(report.summary.findings, "finding", "findings")
            ),
            true,
        ),
        Some(threshold) => {
            let count = report.summary.at_or_above(threshold);
            if report.threshold_met {
                (
                    format!(
                        "Result: FAIL — {count} {} at or above {threshold}.",
                        plural(count, "finding", "findings")
                    ),
                    false,
                )
            } else {
                (
                    format!("Result: PASS — no findings at or above {threshold}."),
                    true,
                )
            }
        }
    }
}

fn code_block(statement: &str) -> String {
    let mut longest_run = 0;
    let mut current_run = 0;
    for character in statement.chars() {
        if character == '`' {
            current_run += 1;
            longest_run = longest_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    let fence = "`".repeat((longest_run + 1).max(3));
    format!("{fence}sql\n{statement}\n{fence}")
}

fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileReport, Finding, Severity};

    fn sample_report(fail_on: Option<Severity>, findings: Vec<Finding>) -> Report {
        Report::new(
            crate::dialect::DialectKind::Postgres,
            vec![FileReport {
                file: "migration.sql".to_string(),
                statements: 2,
                findings,
            }],
            fail_on,
        )
    }

    fn finding() -> Finding {
        Finding {
            rule: "MG001",
            severity: Severity::Destructive,
            file: "migration.sql".to_string(),
            line: 1,
            statement: "DROP TABLE users;".to_string(),
            detail: "This deletes data.".to_string(),
            suggestion: "Back it up.".to_string(),
        }
    }

    #[test]
    fn text_output_contains_summary_and_result() {
        let report = sample_report(None, vec![finding()]);
        let output = render(&report, OutputFormat::Text, Style::disabled()).unwrap();
        assert!(output.contains("migration.sql"));
        assert!(output.contains("MG001"));
        assert!(output.contains("Summary: 1 file, 2 statements, 1 finding"));
        assert!(output.contains("Result: PASS"));
    }

    #[test]
    fn json_output_is_pretty_and_documented_shape() {
        let report = sample_report(Some(Severity::Destructive), vec![finding()]);
        let output = render(&report, OutputFormat::Json, Style::disabled()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["tool"], "migration-guard");
        assert_eq!(value["fail_on"], "destructive");
        assert_eq!(value["threshold_met"], true);
        assert_eq!(value["files"][0]["findings"][0]["rule"], "MG001");
    }

    #[test]
    fn markdown_uses_a_safe_code_fence() {
        let report = sample_report(None, vec![finding()]);
        let output = render(&report, OutputFormat::Markdown, Style::disabled()).unwrap();
        assert!(output.starts_with("# Migration Guard report"));
        assert!(output.contains("```sql\nDROP TABLE users;\n```"));
        assert!(output.contains("**Result:**"));
    }
}
