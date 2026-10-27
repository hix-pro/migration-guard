use std::fmt;

use serde::Serialize;

use crate::dialect::DialectKind;

/// How serious a finding is. The ordering is intentional: a `--fail-on`
/// threshold of `risky` includes `destructive` findings as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Risky,
    Destructive,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Risky => "risky",
            Self::Destructive => "destructive",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single rule violation for one statement.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Stable rule identifier, for example `MG007`.
    pub rule: &'static str,
    pub severity: Severity,
    /// File exactly as it was reported by the path scan.
    pub file: String,
    /// 1-based line of the first token of the statement.
    pub line: usize,
    /// Whitespace-normalized statement text, truncated to 200 characters.
    pub statement: String,
    /// Plain-English explanation of the risk.
    pub detail: String,
    /// A concrete way to avoid or reduce the risk.
    pub suggestion: String,
}

/// Analysis results for one input file.
#[derive(Debug, Clone, Serialize)]
pub struct FileReport {
    pub file: String,
    /// Number of top-level statements inspected in this file.
    pub statements: usize,
    pub findings: Vec<Finding>,
}

/// Aggregate counts for a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub files_checked: usize,
    pub statements_analyzed: usize,
    pub findings: usize,
    pub destructive: usize,
    pub risky: usize,
    pub warnings: usize,
}

impl Summary {
    pub fn from_files(files: &[FileReport]) -> Self {
        let mut summary = Self {
            files_checked: files.len(),
            ..Self::default()
        };
        for file in files {
            summary.statements_analyzed += file.statements;
            for finding in &file.findings {
                summary.findings += 1;
                match finding.severity {
                    Severity::Destructive => summary.destructive += 1,
                    Severity::Risky => summary.risky += 1,
                    Severity::Warning => summary.warnings += 1,
                }
            }
        }
        summary
    }

    /// Number of findings at or above a threshold.
    pub const fn at_or_above(&self, threshold: Severity) -> usize {
        match threshold {
            Severity::Destructive => self.destructive,
            Severity::Risky => self.destructive + self.risky,
            Severity::Warning => self.findings,
        }
    }
}

/// The complete machine-readable report. `schema_version` changes only for
/// incompatible JSON changes.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub tool: &'static str,
    pub version: &'static str,
    pub dialect: DialectKind,
    /// `None` means report-only mode (the default).
    pub fail_on: Option<Severity>,
    pub files: Vec<FileReport>,
    pub summary: Summary,
    /// True when at least one finding meets the `fail_on` threshold.
    pub threshold_met: bool,
}

impl Report {
    pub fn new(dialect: DialectKind, files: Vec<FileReport>, fail_on: Option<Severity>) -> Self {
        let summary = Summary::from_files(&files);
        let threshold_met = fail_on.is_some_and(|severity| summary.at_or_above(severity) > 0);
        Self {
            schema_version: 1,
            tool: "migration-guard",
            version: env!("CARGO_PKG_VERSION"),
            dialect,
            fail_on,
            files,
            summary,
            threshold_met,
        }
    }
}
