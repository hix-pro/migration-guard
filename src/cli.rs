use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::dialect::DialectKind;
use crate::model::Severity;

#[derive(Debug, Parser)]
#[command(
    name = "migration-guard",
    version,
    about = "Catch dangerous SQL migrations before they lock tables or delete data",
    long_about = "Migration Guard statically analyzes .sql migration files for destructive statements \
                  and operations that can lock tables in production. It never connects to a database.",
    after_help = "Examples:\n  migration-guard check\n  migration-guard check migrations --fail-on destructive\n  migration-guard discover\n  migration-guard discover --format json\n\nExit codes:\n  0  no findings at or above --fail-on (or report-only mode)\n  1  I/O, parse, or usage error\n  2  findings at or above --fail-on"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check SQL files or directories for risky migrations
    Check(CheckArgs),
    /// Discover SQL migration directories used by common frameworks
    Discover(DiscoverArgs),
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// A .sql file or a directory scanned recursively for .sql files.
    /// Omit paths to auto-discover known migration directories.
    #[arg(value_name = "PATH", num_args = 0..)]
    pub paths: Vec<PathBuf>,

    /// Project root used for framework discovery when no paths are given
    #[arg(
        long,
        value_name = "DIR",
        default_value = ".",
        conflicts_with = "paths"
    )]
    pub root: PathBuf,

    /// SQL dialect used to parse statements
    #[arg(long, value_enum, default_value_t = DialectKind::Postgres)]
    pub dialect: DialectKind,

    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// Exit 2 when any finding is at or above this severity (default: report only)
    #[arg(long, value_enum)]
    pub fail_on: Option<FailOn>,
}

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Project root to search for migration directories
    #[arg(value_name = "PATH", default_value = ".")]
    pub root: PathBuf,

    /// Output format
    #[arg(long, value_enum, default_value_t = DiscoverFormat::Text)]
    pub format: DiscoverFormat,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum DiscoverFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Markdown,
    Sarif,
}

/// CLI spelling of severity thresholds. The declarations are ordered from most
/// severe to least severe so `--fail-on` help reads naturally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailOn {
    Destructive,
    Risky,
    Warning,
}

impl FailOn {
    pub const fn severity(self) -> Severity {
        match self {
            Self::Destructive => Severity::Destructive,
            Self::Risky => Severity::Risky,
            Self::Warning => Severity::Warning,
        }
    }
}
