use clap::ValueEnum;
use serde::Serialize;
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};

/// SQL dialect used both for parsing and for dialect-specific rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DialectKind {
    #[default]
    Postgres,
    Mysql,
    Sqlite,
    Generic,
}

impl DialectKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Sqlite => "sqlite",
            Self::Generic => "generic",
        }
    }

    pub fn parser_dialect(self) -> Box<dyn Dialect> {
        match self {
            Self::Postgres => Box::new(PostgreSqlDialect {}),
            Self::Mysql => Box::new(MySqlDialect {}),
            Self::Sqlite => Box::new(SQLiteDialect {}),
            Self::Generic => Box::new(GenericDialect {}),
        }
    }

    /// Rules that depend on PostgreSQL-only syntax and locking behavior are
    /// only enabled for this dialect.
    pub const fn is_postgres(self) -> bool {
        matches!(self, Self::Postgres)
    }

    /// Dollar-quoted strings are PostgreSQL syntax.
    pub const fn supports_dollar_quotes(self) -> bool {
        matches!(self, Self::Postgres | Self::Generic)
    }

    /// MySQL treats backslashes as escapes in string literals by default.
    pub const fn backslash_escapes(self) -> bool {
        matches!(self, Self::Mysql)
    }

    /// Backtick-quoted identifiers are accepted by these dialects.
    pub const fn supports_backticks(self) -> bool {
        matches!(self, Self::Mysql | Self::Sqlite | Self::Generic)
    }
}
