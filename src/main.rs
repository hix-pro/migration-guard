use std::io::Write;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use clap::error::ErrorKind;

use migration_guard::analyze;
use migration_guard::cli::{CheckArgs, Cli, Command, DiscoverArgs, DiscoverFormat, FailOn};
use migration_guard::discover;
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

    match cli.command {
        Command::Check(args) => run_check(args),
        Command::Discover(args) => run_discover(args),
    }
}

fn run_check(args: CheckArgs) -> Result<u8> {
    let paths = if args.paths.is_empty() {
        let discovery = discover::discover(&args.root)?;
        if discovery.directories.is_empty() {
            bail!(
                "no SQL migration directories found under `{}`; pass a path explicitly or run `migration-guard discover`",
                args.root.display()
            );
        }
        discovery.paths()
    } else {
        args.paths
    };

    let fail_on = args.fail_on.map(FailOn::severity);
    let report = analyze::build_report(args.dialect, &paths, fail_on)?;
    let output = report::render(&report, args.format, Style::auto())?;
    write_stdout(&output)?;

    Ok(if report.threshold_met {
        EXIT_FINDINGS
    } else {
        0
    })
}

fn run_discover(args: DiscoverArgs) -> Result<u8> {
    let discovery = discover::discover(&args.root)?;
    let output = match args.format {
        DiscoverFormat::Text => discover::render_text(&discovery, Style::auto()),
        DiscoverFormat::Json => discover::render_json(&discovery)?,
    };
    write_stdout(&output)?;
    Ok(0)
}

fn write_stdout(output: &str) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(output.as_bytes())
        .context("failed to write report to stdout")?;
    stdout.flush().context("failed to flush stdout")
}
