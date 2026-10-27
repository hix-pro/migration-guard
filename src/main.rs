use std::io::Write;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use clap::error::ErrorKind;

use migration_guard::analyze;
use migration_guard::cli::{Cli, Command, FailOn};
use migration_guard::report;
use migration_guard::style::Style;

const EXIT_FINDINGS: u8 = 2;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("migration-guard: error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<u8> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit = match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 1,
            };
            error
                .print()
                .context("failed to print command-line help or error")?;
            return Ok(exit);
        }
    };

    let Command::Check(args) = cli.command;
    let fail_on = args.fail_on.map(FailOn::severity);
    let report = analyze::build_report(args.dialect, &args.paths, fail_on)?;
    let output = report::render(&report, args.format, Style::auto())?;

    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(output.as_bytes())
        .context("failed to write report to stdout")?;
    stdout.flush().context("failed to flush stdout")?;

    Ok(if report.threshold_met {
        EXIT_FINDINGS
    } else {
        0
    })
}
