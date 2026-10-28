# Migration Guard

**Catch dangerous SQL migrations before they lock tables or delete data.**

[![CI](https://github.com/hix-pro/migration-guard/actions/workflows/ci.yml/badge.svg)](https://github.com/hix-pro/migration-guard/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/hix-pro/migration-guard)](https://github.com/hix-pro/migration-guard/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Migration Guard is a local-first Rust CLI that statically analyzes `.sql` migration files. It parses each statement, applies a small set of deterministic rules, and reports destructive commands, blocking DDL, and breaking renames. It never connects to a database and makes no network requests.

This is an early, focused tool. It is meant to add a guardrail to review and CI, not to replace staging tests, backups, or a rollback plan.

## Terminal example

```console
$ migration-guard check examples/risky.sql --fail-on risky
migration-guard 0.3.1  ·  postgres  ·  1 file

examples/risky.sql  (8 statements)
  risky        MG005  line 2
      ALTER TABLE accounts ADD COLUMN last_seen_at timestamptz NOT NULL;
      Why: Adding NOT NULL column `last_seen_at` to `accounts` without a DEFAULT can fail on existing rows or force a table rewrite.
      Fix: Add the column as nullable, backfill it, then set NOT NULL. Alternatively provide a DEFAULT if one is appropriate.

  risky        MG004  line 4
      ALTER TABLE accounts ALTER COLUMN email TYPE citext;
      Why: Changing `accounts.email` to `citext` can rewrite the table and block reads and writes while it runs.
      Fix: Create a new column with the target type, backfill in batches, then swap columns under a short lock, or use a maintenance window.

  risky        MG007  line 6
      CREATE INDEX idx_accounts_email ON accounts (email);
      Why: Building an index on `accounts` without CONCURRENTLY blocks writes for the duration of the build.
      Fix: Use CREATE INDEX CONCURRENTLY outside a transaction. If the build fails, drop the invalid index before retrying.

  risky        MG009  line 8
      ALTER TABLE orders ADD CONSTRAINT orders_account_fk FOREIGN KEY (account_id) REFERENCES accounts (id);
      Why: Adding a foreign key from `orders` to `accounts` validates every existing row while holding locks.
      Fix: Add the constraint with NOT VALID, then run ALTER TABLE ... VALIDATE CONSTRAINT as a separate step.

  warning      MG010  line 12
      ALTER TABLE accounts RENAME COLUMN email TO email_address;
      Why: Renaming `accounts.email` to `email_address` breaks queries, views, and clients that still use the old name.
      Fix: Use expand/contract: add the new name, update readers and writers, then remove the old name in a later release.

  warning      MG011  line 15
      CREATE INDEX CONCURRENTLY idx_accounts_display_name ON accounts (display_name);
      Why: PostgreSQL cannot run CREATE INDEX CONCURRENTLY inside a transaction block; this statement will fail at execution time.
      Fix: Move the concurrent index build outside BEGIN/COMMIT and make it its own migration step.

Summary: 1 file, 8 statements, 6 findings (0 destructive, 4 risky, 2 warnings)
Result: FAIL — 4 findings at or above risky.
$ echo $?
2
```

The `examples/` directory also contains a migration with no findings and a fully destructive migration.

## Install

Build from source (Rust 1.85 or newer, 2024 edition):

```sh
git clone https://github.com/hix-pro/migration-guard.git
cd migration-guard
cargo install --path . --locked
```

Or grab a prebuilt binary — every version ships tarballs for Linux x86_64 (gnu), macOS (Apple Silicon), and Windows x86_64, each with a SHA-256 checksum:

```sh
curl -LO https://github.com/hix-pro/migration-guard/releases/latest/download/migration-guard-x86_64-unknown-linux-gnu.tar.gz
tar xzf migration-guard-x86_64-unknown-linux-gnu.tar.gz
install migration-guard-x86_64-unknown-linux-gnu/migration-guard /usr/local/bin/
```

Or build a release binary in place:

```sh
cargo build --release --locked
./target/release/migration-guard --version
```

The repository commits `Cargo.lock`, so `--locked` builds are reproducible.

## Usage

```text
migration-guard check <PATH...> [OPTIONS]
```

Each `PATH` is a `.sql` file or a directory. Directories are scanned recursively for `.sql` files (case-insensitive extension). Results are sorted by path and duplicate files are removed.

| Option | Values | Default | Description |
| --- | --- | --- | --- |
| `--dialect` | `postgres`, `mysql`, `sqlite`, `generic` | `postgres` | Parser dialect and rule set |
| `--format` | `text`, `json`, `markdown` | `text` | Report format |
| `--fail-on` | `destructive`, `risky`, `warning` | unset | Exit `2` when a finding is at or above this severity |
| `-h`, `--help` | | | Show help |
| `-V`, `--version` | | | Show the version |

Without `--fail-on`, the tool is report-only: findings are printed, but the exit code is `0`. A threshold is inclusive, so `--fail-on risky` fails on both `risky` and `destructive` findings.

### Examples

```sh
# Report everything, but do not fail the command
migration-guard check migrations

# Fail CI on data loss, allow risky findings for now
migration-guard check migrations --fail-on destructive

# Parse MySQL migrations and only apply dialect-appropriate rules
migration-guard check migrations --dialect mysql --fail-on risky

# Machine-readable output
migration-guard check migrations --format json --fail-on risky > migration-report.json

# A report for a pull request comment
migration-guard check migrations --format markdown > migration-report.md
```

## Exit codes

| Code | Meaning |
| ---: | --- |
| `0` | No findings at or above `--fail-on`; also the default report-only mode and empty directories |
| `1` | I/O error, unreadable or invalid UTF-8 file, SQL parse error, or command-line usage error |
| `2` | At least one finding at or above `--fail-on` |

A parse error stops the run and is reported on stderr. Constructs that are recognized but cannot be analyzed, such as `DO` blocks, produce a `warning` finding instead. See [Limitations](#limitations).

## Rules

Findings include the rule ID, severity, file, line, normalized statement text, an explanation, and a suggested fix.

| ID | Severity | Dialect | Catches |
| --- | --- | --- | --- |
| `MG001` | destructive | all | `DROP TABLE`, `ALTER TABLE ... DROP COLUMN`, `TRUNCATE` |
| `MG002` | destructive | all | `DELETE` without a `WHERE` clause |
| `MG003` | destructive | all | `UPDATE` without a `WHERE` clause |
| `MG004` | risky | all | `ALTER COLUMN ... TYPE`; MySQL `MODIFY COLUMN` / `CHANGE COLUMN` type changes |
| `MG005` | risky | all | `ADD COLUMN ... NOT NULL` without a `DEFAULT` |
| `MG006` | risky | all | `ALTER COLUMN ... SET NOT NULL` |
| `MG007` | risky | PostgreSQL | `CREATE [UNIQUE] INDEX` without `CONCURRENTLY` |
| `MG008` | risky | PostgreSQL | `DROP INDEX` without `CONCURRENTLY` |
| `MG009` | risky | PostgreSQL | `ADD FOREIGN KEY` without `NOT VALID` |
| `MG010` | warning | all | `RENAME TABLE` / `RENAME COLUMN` (MySQL `RENAME TABLE` included) |
| `MG011` | warning | PostgreSQL | `CREATE INDEX CONCURRENTLY` inside an explicit transaction |
| `MG012` | risky | PostgreSQL | `VACUUM FULL`, `CLUSTER`, explicit `LOCK TABLE` |
| `MG013` | warning | all | `DO` blocks and statement types outside the known rule set |

### Dialect-aware behavior

- The PostgreSQL-only rules (`MG007`–`MG009`, `MG011`, `MG012`) are not applied to MySQL, SQLite, or generic SQL. In particular, index statements on MySQL and SQLite do not produce PostgreSQL `CONCURRENTLY` advice.
- General rules such as destructive DDL, unfiltered `UPDATE`/`DELETE`, type changes, and renames apply to every dialect.
- `--dialect generic` uses a permissive parser but does not assume PostgreSQL locking behavior.
- `DROP INDEX CONCURRENTLY` is accepted on PostgreSQL even though it is not represented in `sqlparser`'s AST. It is treated as the non-blocking form.
- `CLUSTER`, parenthesized `VACUUM (FULL)`, and `DO` are detected before the parser fallback so they can still produce a useful result.

### What is considered safe

Statements that are known not to match the rules above are silently accepted. This includes `CREATE TABLE`, nullable `ADD COLUMN`, `INSERT`, `CREATE VIEW`, `SELECT`, `SET`, `COMMENT`, `GRANT`, transaction control, and index creation in dialects where the PostgreSQL rules do not apply. A statement type outside the known-safe and known-risky sets produces `MG013` rather than being ignored.

## Output formats

### Text

Text output is grouped by file. On an interactive terminal, severities are colored; `NO_COLOR=1` (or any non-empty `NO_COLOR`) disables ANSI color, and piped output is always plain. The final two lines are the `Summary:` counts followed by a `Result: PASS` or `Result: FAIL` line.

### JSON

JSON output is a single document with `schema_version: 1`. It is stable within schema version 1; incompatible changes will increment the version.

```json
{
  "schema_version": 1,
  "tool": "migration-guard",
  "version": "0.3.1",
  "dialect": "postgres",
  "fail_on": "risky",
  "files": [
    {
      "file": "migrations/0002_accounts.sql",
      "statements": 1,
      "findings": [
        {
          "rule": "MG007",
          "severity": "risky",
          "file": "migrations/0002_accounts.sql",
          "line": 3,
          "statement": "CREATE INDEX idx_accounts_email ON accounts (email);",
          "detail": "Building an index on `accounts` without CONCURRENTLY blocks writes for the duration of the build.",
          "suggestion": "Use CREATE INDEX CONCURRENTLY outside a transaction. If the build fails, drop the invalid index before retrying."
        }
      ]
    }
  ],
  "summary": {
    "files_checked": 1,
    "statements_analyzed": 1,
    "findings": 1,
    "destructive": 0,
    "risky": 1,
    "warnings": 0
  },
  "threshold_met": true
}
```

Field notes:

- `fail_on` is `null` in report-only mode.
- `threshold_met` says whether the selected threshold was reached; use it instead of reimplementing severity ordering.
- `files` includes files with no findings and their statement counts.
- `statement` is whitespace-normalized and truncated to at most 200 characters, with `...` when truncated. Do not execute it as SQL.
- `severity` is one of `warning`, `risky`, or `destructive`. Severity order is `warning < risky < destructive`.
- `summary.statements_analyzed` counts every top-level statement inspected, including statements that produced `MG013`.

### Markdown

Markdown output has a fixed structure: a metadata list, one section per file, a heading for each finding with the statement in a fenced SQL block, and a final Summary table followed by a bold `**Result:**` line. It is intended for pull-request comments and reports. Use JSON for automation.

## GitHub Action

The repository ships a composite action that installs Migration Guard and runs it against your workspace — no Rust setup in your workflow:

```yaml
- uses: hix-pro/migration-guard@v0.3.1
  with:
    paths: migrations
    fail-on: destructive
```

Inputs: `paths` (space-separated files or directories, default `.`), `dialect` (`postgres` or `mysql`, default `postgres`), `fail-on` (`destructive`, `risky`, `warning`, or empty for report-only), and `args` for extra CLI flags such as `--format markdown`. The threshold is inclusive, so `risky` also fails on `destructive`. The first run compiles the tool from source, which takes a minute or two; pin a tag so upgrades are explicit. The repository's own pipeline re-checks its safe fixtures through the action in a Dogfood workflow.

## CI

The tool does not require a database, so it can run in any job that has the binary:

```yaml
name: migration-guard
on:
  pull_request:
    paths:
      - "migrations/**"

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo install --path . --locked
      - run: migration-guard check migrations --fail-on destructive
```

Start with `destructive` to block data loss immediately, then tighten to `risky` or `warning` as existing migrations are cleaned up. The repository's own CI workflow runs formatting, Clippy, all tests, and a release build.

## How it works

1. Paths are expanded into a sorted, deduplicated list of `.sql` files.
2. A small scanner splits the file into top-level statements. It understands single-quoted strings, quoted identifiers, backticks in MySQL/SQLite, `--`, `#` (MySQL), and nested `/* */` comments, and PostgreSQL dollar-quoted bodies, so semicolons inside them do not create false statement boundaries.
3. Each statement is parsed with `sqlparser` using the selected dialect.
4. The AST is checked against the rules, while an explicit transaction flag is updated by `BEGIN`/`START TRANSACTION`, `COMMIT`, `ROLLBACK`, and savepoints. That flag powers `MG011`.
5. A few PostgreSQL constructs that `sqlparser` 0.63 cannot represent are classified from tokens: `DO` blocks (opaque warning), `CLUSTER`, parenthesized `VACUUM (FULL)`, and `DROP INDEX CONCURRENTLY` (safe form).
6. Findings are aggregated, counted, and rendered. The process exits with `2` only when the threshold is met.

## Limitations

- **Static analysis only.** Migration Guard never connects to a database, so it cannot know table size, row count, server version, or current workload. Statements about lock duration are workload- and version-dependent; the rules flag patterns, not measured impact.
- **No schema introspection.** `UPDATE ... WHERE true` and `DELETE ... WHERE 1=1` contain a `WHERE` clause and are treated as filtered. The tool cannot verify that a predicate is selective or correct.
- **Not a proof of safety.** A migration with no findings can still be slow, incorrect, or dangerous for reasons outside these rules. Review migrations and test against realistic data.
- **Transaction scope is per file.** Only explicit `BEGIN`/`START TRANSACTION` and `COMMIT`/`ROLLBACK` in the same file are tracked. A migration runner that wraps files in a transaction can still turn `CREATE INDEX CONCURRENTLY` into a runtime error; conversely, an explicit transaction that spans files is not detected.
- **Opaque SQL is warned about, not inspected.** `DO` blocks and statement types outside the rule set produce `MG013`. The SQL inside `DO` is not analyzed.
- **Custom SQL splitting is limited.** MySQL `DELIMITER` directives and unusual procedural bodies are not supported. If a statement cannot be parsed, the run exits `1` with the file and statement line. Fixes that require sharing transaction state across files or executing SQL are out of scope.
- **Dialect parsing is approximate.** `sqlparser` accepts some syntax across dialects. Dialect-specific rules are gated, but the parser is not a database server.
- **Only `.sql` input.** Files must be valid UTF-8. Non-`.sql` explicit paths are rejected; non-`.sql` files inside scanned directories are ignored.
- **Rule set, not a security scanner.** It does not analyze permissions, injection, or application code.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
cargo run -- check examples --fail-on risky
```

Unit tests cover every rule, statement splitting, transaction transitions, and report rendering. Integration tests in `tests/cli.rs` exercise exit codes, recursive discovery, JSON shape, Markdown output, parse failures, and dialect behavior using fixtures in `tests/fixtures/`.

## Related tools

| Tool | What it does |
| --- | --- |
| [EnvAudit](https://github.com/hix-pro/env-audit) | find missing, undocumented, and unused environment variables |
| [API Watch](https://github.com/hix-pro/api-watch) | catch breaking changes in your OpenAPI and JSON contracts |
| [RepoRace](https://github.com/hix-pro/reporace) | see which GitHub repositories are accelerating |
| [TypeShift](https://github.com/hix-pro/typeshift) | decide the semver bump from a TypeScript declaration diff |
| [ActionLens](https://github.com/hix-pro/action-lens) | see what a GitHub Actions workflow really does, before it hits main |
| [ComposeMap](https://github.com/hix-pro/compose-map) | understand your Docker Compose stack and its production risks at a glance |
| [OpenCode Telegram](https://github.com/hix-pro/opencode-telegram) | control a local OpenCode server from Telegram |
---

_Part of the **[hix tools](https://github.com/hix-pro#readme)** suite — local-first, no backend, CI-ready._

## License

MIT. See [LICENSE](LICENSE).
