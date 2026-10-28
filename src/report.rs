use std::fmt::Write as _;

use anyhow::Result;
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::model::{Report, Severity};
use crate::style::Style;

pub fn render(report: &Report, format: OutputFormat, style: Style) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(report, style)),
        OutputFormat::Json => render_json(report),
        OutputFormat::Markdown => Ok(render_markdown(report)),
        OutputFormat::Sarif => render_sarif(report),
    }
}

#[derive(Debug, Clone, Serialize)]
struct SarifLog {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun>,
}

#[derive(Debug, Clone, Serialize)]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
}

#[derive(Debug, Clone, Serialize)]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDriver {
    name: &'static str,
    information_uri: &'static str,
    version: &'static str,
    rules: Vec<SarifRule>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRule {
    id: &'static str,
    name: &'static str,
    short_description: SarifText,
    help_uri: &'static str,
    default_configuration: SarifLevel,
}

#[derive(Debug, Clone, Serialize)]
struct SarifText {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct SarifLevel {
    level: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult {
    rule_id: &'static str,
    rule_index: usize,
    level: &'static str,
    message: SarifText,
    locations: Vec<SarifLocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation {
    physical_location: SarifPhysicalLocation,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation {
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

#[derive(Debug, Clone, Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRegion {
    start_line: usize,
}

const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const SARIF_HELP_URI: &str = "https://github.com/hix-pro/migration-guard#rules";

const SARIF_RULES: [(&str, &str, &str, &str); 13] = [
    (
        "MG001",
        "DestructiveDdl",
        "DROP or TRUNCATE removes data or schema objects.",
        "error",
    ),
    (
        "MG002",
        "DeleteWithoutWhere",
        "DELETE without WHERE removes every row.",
        "error",
    ),
    (
        "MG003",
        "UpdateWithoutWhere",
        "UPDATE without WHERE rewrites every row.",
        "error",
    ),
    (
        "MG004",
        "ColumnTypeChange",
        "Changing a column type can rewrite the table and break readers.",
        "error",
    ),
    (
        "MG005",
        "AddNotNullWithoutDefault",
        "Adding a NOT NULL column without a default fails existing rows.",
        "error",
    ),
    (
        "MG006",
        "SetNotNull",
        "SET NOT NULL validates every row and takes a lock.",
        "error",
    ),
    (
        "MG007",
        "CreateIndexWithoutConcurrently",
        "CREATE INDEX without CONCURRENTLY blocks writes.",
        "error",
    ),
    (
        "MG008",
        "DropIndexWithoutConcurrently",
        "DROP INDEX without CONCURRENTLY takes a lock.",
        "error",
    ),
    (
        "MG009",
        "ForeignKeyWithoutNotValid",
        "Adding a foreign key without NOT VALID scans the table.",
        "error",
    ),
    (
        "MG010",
        "Rename",
        "Renaming a table or column breaks existing clients.",
        "warning",
    ),
    (
        "MG011",
        "ConcurrentIndexInTransaction",
        "CREATE INDEX CONCURRENTLY cannot run inside a transaction.",
        "warning",
    ),
    (
        "MG012",
        "LockingStatement",
        "VACUUM FULL, CLUSTER, or LOCK takes a heavy lock.",
        "error",
    ),
    (
        "MG013",
        "Unanalyzed",
        "The statement was not recognized, so it was not checked.",
        "warning",
    ),
];

fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Destructive | Severity::Risky => "error",
        Severity::Warning => "warning",
    }
}

fn sarif_rule_index(rule: &str) -> usize {
    SARIF_RULES
        .iter()
        .position(|(id, ..)| *id == rule)
        .unwrap_or(0)
}

fn sarif_uri(path: &str) -> String {
    let path = path.replace('\\', "/");
    path.strip_prefix("./").unwrap_or(&path).to_owned()
}

/// SARIF 2.1.0 output for GitHub code scanning and other SARIF consumers.
fn render_sarif(report: &Report) -> Result<String> {
    let rules = SARIF_RULES
        .iter()
        .map(|(id, name, description, level)| SarifRule {
            id,
            name,
            short_description: SarifText {
                text: (*description).to_owned(),
            },
            help_uri: SARIF_HELP_URI,
            default_configuration: SarifLevel { level },
        })
        .collect();

    let results = report
        .files
        .iter()
        .flat_map(|file| {
            file.findings.iter().map(|finding| SarifResult {
                rule_id: finding.rule,
                rule_index: sarif_rule_index(finding.rule),
                level: sarif_level(finding.severity),
                message: SarifText {
                    text: format!(
                        "{}\n\nStatement: {}\n\nFix: {}",
                        finding.detail, finding.statement, finding.suggestion
                    ),
                },
                locations: vec![SarifLocation {
                    physical_location: SarifPhysicalLocation {
                        artifact_location: SarifArtifactLocation {
                            uri: sarif_uri(&file.file),
                        },
                        region: SarifRegion {
                            start_line: finding.line.max(1),
                        },
                    },
                }],
            })
        })
        .collect();

    let log = SarifLog {
        schema: SARIF_SCHEMA,
        version: "2.1.0",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "Migration Guard",
                    information_uri: "https://github.com/hix-pro/migration-guard",
                    version: report.version,
                    rules,
                },
            },
            results,
        }],
    };

    let mut output = serde_json::to_string_pretty(&log)?;
    output.push('\n');
    Ok(output)
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
    fn sarif_output_has_stable_rules_and_locations() {
        let report = sample_report(Some(Severity::Destructive), vec![finding()]);
        let output = render(&report, OutputFormat::Sarif, Style::disabled()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["version"], "2.1.0");
        assert_eq!(
            value["runs"][0]["tool"]["driver"]["name"],
            "Migration Guard"
        );
        assert_eq!(
            value["runs"][0]["tool"]["driver"]["rules"]
                .as_array()
                .unwrap()
                .len(),
            13
        );

        let result = &value["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "MG001");
        assert_eq!(result["level"], "error");
        assert_eq!(
            result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
            "migration.sql"
        );
        assert_eq!(
            result["locations"][0]["physicalLocation"]["region"]["startLine"],
            1
        );
        assert!(
            result["message"]["text"]
                .as_str()
                .unwrap()
                .contains("Statement: DROP TABLE users;")
        );
    }

    #[test]
    fn sarif_maps_warning_severity_to_warning_level() {
        let warning = Finding {
            rule: "MG010",
            severity: Severity::Warning,
            file: "migration.sql".to_string(),
            line: 4,
            statement: "ALTER TABLE users RENAME TO people;".to_string(),
            detail: "Renaming breaks clients.".to_string(),
            suggestion: "Add a view or keep the old name.".to_string(),
        };
        let report = sample_report(None, vec![warning]);
        let output = render(&report, OutputFormat::Sarif, Style::disabled()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["runs"][0]["results"][0]["level"], "warning");
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
