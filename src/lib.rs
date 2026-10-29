//! Migration Guard: static analysis for dangerous SQL migrations.
//!
//! The crate is split into small modules so the CLI, parser integration, rule
//! engine, and output formats can be tested independently.

pub mod analyze;
pub mod cli;
pub mod dialect;
pub mod discover;
pub mod model;
pub mod report;
pub mod rules;
pub mod split;
pub mod style;
