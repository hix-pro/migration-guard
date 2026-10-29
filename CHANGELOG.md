# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-10-29

### Added
- `migration-guard discover` lists known SQL migration directories for Flyway, Liquibase, Prisma, Drizzle, Supabase, and generic `migrations` layouts, with optional JSON output.
- `migration-guard check` auto-discovers those directories when no paths are supplied, so it can run from a repository root without arguments. `--root` selects a different discovery root. If nothing is found, check exits `1` with an actionable error instead of a clean empty report.
- `--format sarif` emits a SARIF 2.1.0 report for GitHub code scanning. Rule IDs match the text and JSON reports, and results point at the SQL file and statement line.
- The composite Action writes and uploads SARIF by default (`sarif: true`), with a `category` input for separating runs. The caller needs `security-events: write`; the upload never changes the check's exit code.

### Changed
- `check` no longer requires a positional path. Explicit paths behave as before.
- The composite Action's `paths` input now defaults to empty, which enables discovery. Set `paths: .` to scan every `.sql` file or pass specific paths to preserve the previous behavior.

## [0.3.1] - 2026-10-28

### Fixed
- Valid PostgreSQL identifiers that contain `$` (for example `SELECT foo$bar$baz;`) no longer trigger false "unterminated dollar-quoted string" errors, and dollar-quote tags may now contain Unicode letters (for example `DO $é$ … $é$;`).
- `NO_COLOR` now follows the no-color.org convention: only a non-empty value disables colored output, so an empty `NO_COLOR` keeps color on interactive terminals.

## [0.3.0] - 2026-10-27

### Added
- Prebuilt release binaries: version tags now trigger builds for Linux x86_64 (gnu), macOS (Apple Silicon), and Windows x86_64, attaching `migration-guard-{target}.tar.gz` tarballs with SHA-256 checksums to the release.
- The composite Action prefers the prebuilt binary matching its version tag and falls back to building from source when it is not available.

## [0.2.0] - 2026-10-27

### Added
- Composite GitHub Action (`action.yml`) so workflows can run Migration Guard without installing Rust themselves; the repository dogfoods it by re-checking its safe fixtures.
- Initial public release.
