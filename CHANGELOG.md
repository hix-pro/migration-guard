# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
