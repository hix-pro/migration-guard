use sqlparser::ast::{
    AlterColumnOperation, AlterTable, AlterTableOperation, ColumnOption, CreateIndex, Delete,
    ObjectType, RenameTableNameKind, Statement, TableConstraint, Update, VacuumStatement,
};
use sqlparser::keywords::Keyword;
use sqlparser::tokenizer::{Token, Tokenizer};

use crate::dialect::DialectKind;
use crate::model::{Finding, Severity};

/// Stable rule identifiers and their default severities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    DestructiveDdl,
    DeleteWithoutWhere,
    UpdateWithoutWhere,
    ColumnTypeChange,
    AddNotNullWithoutDefault,
    SetNotNull,
    CreateIndexWithoutConcurrently,
    DropIndexWithoutConcurrently,
    ForeignKeyWithoutNotValid,
    Rename,
    ConcurrentIndexInTransaction,
    LockingStatement,
    Unanalyzed,
}

impl Rule {
    pub const fn id(self) -> &'static str {
        match self {
            Self::DestructiveDdl => "MG001",
            Self::DeleteWithoutWhere => "MG002",
            Self::UpdateWithoutWhere => "MG003",
            Self::ColumnTypeChange => "MG004",
            Self::AddNotNullWithoutDefault => "MG005",
            Self::SetNotNull => "MG006",
            Self::CreateIndexWithoutConcurrently => "MG007",
            Self::DropIndexWithoutConcurrently => "MG008",
            Self::ForeignKeyWithoutNotValid => "MG009",
            Self::Rename => "MG010",
            Self::ConcurrentIndexInTransaction => "MG011",
            Self::LockingStatement => "MG012",
            Self::Unanalyzed => "MG013",
        }
    }

    pub const fn severity(self) -> Severity {
        match self {
            Self::DestructiveDdl | Self::DeleteWithoutWhere | Self::UpdateWithoutWhere => {
                Severity::Destructive
            }
            Self::Rename | Self::ConcurrentIndexInTransaction | Self::Unanalyzed => {
                Severity::Warning
            }
            Self::ColumnTypeChange
            | Self::AddNotNullWithoutDefault
            | Self::SetNotNull
            | Self::CreateIndexWithoutConcurrently
            | Self::DropIndexWithoutConcurrently
            | Self::ForeignKeyWithoutNotValid
            | Self::LockingStatement => Severity::Risky,
        }
    }
}

/// Everything a rule needs to build a finding.
#[derive(Debug, Clone, Copy)]
pub struct RuleContext<'a> {
    pub dialect: DialectKind,
    pub in_transaction: bool,
    pub file: &'a str,
    pub line: usize,
    pub statement: &'a str,
}

impl RuleContext<'_> {
    fn finding(
        &self,
        rule: Rule,
        detail: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Finding {
        Finding {
            rule: rule.id(),
            severity: rule.severity(),
            file: self.file.to_string(),
            line: self.line,
            statement: self.statement.to_string(),
            detail: detail.into(),
            suggestion: suggestion.into(),
        }
    }
}

/// The result of checking one parsed statement.
pub enum Outcome {
    /// The statement is understood. Findings may be empty.
    Analyzed(Vec<Finding>),
    /// The statement is not covered by the rule set.
    Unrecognized,
}

/// Check one parsed statement. `Outcome::Unrecognized` lets the caller emit a
/// single MG013 warning instead of silently ignoring unknown syntax.
pub fn analyze_statement(statement: &Statement, ctx: &RuleContext<'_>) -> Outcome {
    match statement {
        Statement::Drop {
            object_type, names, ..
        } => analyze_drop(*object_type, names, ctx),
        Statement::Truncate(truncate) => {
            let tables = truncate
                .table_names
                .iter()
                .map(|target| target.name.to_string())
                .collect::<Vec<_>>();
            let targets = quote_list(&tables);
            let lock_note = if ctx.dialect.is_postgres() {
                " and takes an ACCESS EXCLUSIVE lock on PostgreSQL"
            } else {
                ""
            };
            Outcome::Analyzed(vec![ctx.finding(
                Rule::DestructiveDdl,
                format!("TRUNCATE removes every row from {targets}{lock_note}."),
                "Use a filtered DELETE for partial cleanup, or archive the rows and run TRUNCATE in a maintenance window.",
            )])
        }
        Statement::Delete(delete) => analyze_delete(delete, ctx),
        Statement::Update(update) => analyze_update(update, ctx),
        Statement::CreateIndex(index) => analyze_create_index(index, ctx),
        Statement::AlterTable(alter) => Outcome::Analyzed(analyze_alter_table(alter, ctx)),
        Statement::RenameTable(renames) => {
            let mut findings = Vec::new();
            for rename in renames {
                findings.push(ctx.finding(
                    Rule::Rename,
                    format!(
                        "Renaming table `{}` to `{}` immediately breaks clients that still use the old name.",
                        rename.old_name, rename.new_name
                    ),
                    "Use expand/contract: create the new name, update readers and writers, then remove the old name in a later release.",
                ));
            }
            Outcome::Analyzed(findings)
        }
        Statement::Vacuum(vacuum) => analyze_vacuum(vacuum, ctx),
        Statement::Lock(lock) => {
            if !ctx.dialect.is_postgres() {
                return Outcome::Analyzed(Vec::new());
            }
            let tables = lock
                .tables
                .iter()
                .map(|target| target.name.to_string())
                .collect::<Vec<_>>();
            let targets = quote_list(&tables);
            Outcome::Analyzed(vec![ctx.finding(
                Rule::LockingStatement,
                format!(
                    "This explicit LOCK TABLE holds a lock on {targets} until the end of the transaction, blocking conflicting traffic."
                ),
                "Avoid explicit table locks in migrations. If one is unavoidable, keep the transaction short and set a lock timeout.",
            )])
        }
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. }
        | Statement::Set(_)
        | Statement::Use(_)
        | Statement::Comment { .. }
        | Statement::Grant(_)
        | Statement::Revoke(_)
        | Statement::Analyze(_)
        | Statement::Explain { .. }
        | Statement::Query(_)
        | Statement::Insert(_)
        | Statement::CreateTable(_)
        | Statement::CreateView(_)
        | Statement::CreateSchema { .. }
        | Statement::CreateSequence { .. }
        | Statement::CreateType { .. }
        | Statement::CreateExtension(_)
        | Statement::CreateFunction(_)
        | Statement::CreateTrigger(_)
        | Statement::CreatePolicy(_)
        | Statement::CreateRole(_)
        | Statement::CreateDomain(_)
        | Statement::CreateCollation(_)
        | Statement::CreateVirtualTable { .. } => Outcome::Analyzed(Vec::new()),
        _ => Outcome::Unrecognized,
    }
}

fn analyze_drop(
    object_type: ObjectType,
    names: &[sqlparser::ast::ObjectName],
    ctx: &RuleContext<'_>,
) -> Outcome {
    match object_type {
        ObjectType::Table => {
            let label = if names.len() == 1 { "table" } else { "tables" };
            let targets = quote_list(&names.iter().map(ToString::to_string).collect::<Vec<_>>());
            Outcome::Analyzed(vec![ctx.finding(
                Rule::DestructiveDdl,
                format!(
                    "Dropping {label} {targets} permanently deletes its data and can break dependent objects."
                ),
                "Back up the table, remove application references first, and rename it for a rollback window before dropping it.",
            )])
        }
        ObjectType::Index => {
            if !ctx.dialect.is_postgres() {
                return Outcome::Analyzed(Vec::new());
            }
            let targets = quote_list(&names.iter().map(ToString::to_string).collect::<Vec<_>>());
            Outcome::Analyzed(vec![ctx.finding(
                Rule::DropIndexWithoutConcurrently,
                format!(
                    "Dropping index {targets} without CONCURRENTLY takes an ACCESS EXCLUSIVE lock on PostgreSQL."
                ),
                "Use DROP INDEX CONCURRENTLY outside a transaction, and be prepared for an invalid index if the drop is interrupted.",
            )])
        }
        ObjectType::View | ObjectType::MaterializedView => Outcome::Analyzed(Vec::new()),
        _ => Outcome::Unrecognized,
    }
}

fn analyze_delete(delete: &Delete, ctx: &RuleContext<'_>) -> Outcome {
    if delete.selection.is_some() {
        return Outcome::Analyzed(Vec::new());
    }
    Outcome::Analyzed(vec![ctx.finding(
        Rule::DeleteWithoutWhere,
        "This DELETE has no WHERE clause, so it removes every row from the target table.",
        "Add a WHERE clause, verify the predicate with a SELECT first, and take a backup before applying it.",
    )])
}

fn analyze_update(update: &Update, ctx: &RuleContext<'_>) -> Outcome {
    if update.selection.is_some() {
        return Outcome::Analyzed(Vec::new());
    }
    Outcome::Analyzed(vec![ctx.finding(
        Rule::UpdateWithoutWhere,
        "This UPDATE has no WHERE clause, so it rewrites every row in the target table.",
        "Add a WHERE clause, verify the rows with a SELECT first, and run the update in small batches if the table is large.",
    )])
}

fn analyze_create_index(index: &CreateIndex, ctx: &RuleContext<'_>) -> Outcome {
    if !ctx.dialect.is_postgres() {
        return Outcome::Analyzed(Vec::new());
    }
    if index.concurrently {
        if ctx.in_transaction {
            return Outcome::Analyzed(vec![ctx.finding(
                Rule::ConcurrentIndexInTransaction,
                "PostgreSQL cannot run CREATE INDEX CONCURRENTLY inside a transaction block; this statement will fail at execution time.",
                "Move the concurrent index build outside BEGIN/COMMIT and make it its own migration step.",
            )]);
        }
        return Outcome::Analyzed(Vec::new());
    }
    Outcome::Analyzed(vec![ctx.finding(
        Rule::CreateIndexWithoutConcurrently,
        format!(
            "Building an index on {} without CONCURRENTLY blocks writes for the duration of the build.",
            quote_ident(&index.table_name.to_string())
        ),
        "Use CREATE INDEX CONCURRENTLY outside a transaction. If the build fails, drop the invalid index before retrying.",
    )])
}

fn analyze_alter_table(alter: &AlterTable, ctx: &RuleContext<'_>) -> Vec<Finding> {
    let table = alter.name.to_string();
    let mut findings = Vec::new();

    for operation in &alter.operations {
        match operation {
            AlterTableOperation::DropColumn { column_names, .. } => {
                let label = if column_names.len() == 1 {
                    "column"
                } else {
                    "columns"
                };
                let columns = quote_list(
                    &column_names
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                );
                findings.push(ctx.finding(
                    Rule::DestructiveDdl,
                    format!(
                        "Dropping {label} {columns} from `{table}` permanently removes its data and can break queries that still reference it."
                    ),
                    "Deploy code that stops using the column first. Keep a backup and drop the column in a later migration.",
                ));
            }
            AlterTableOperation::AddColumn { column_def, .. } => {
                let has_not_null = column_def
                    .options
                    .iter()
                    .any(|option| matches!(option.option, ColumnOption::NotNull));
                let has_default = column_def
                    .options
                    .iter()
                    .any(|option| matches!(option.option, ColumnOption::Default(_)));
                if has_not_null && !has_default {
                    findings.push(ctx.finding(
                        Rule::AddNotNullWithoutDefault,
                        format!(
                            "Adding NOT NULL column `{}` to `{table}` without a DEFAULT can fail on existing rows or force a table rewrite.",
                            column_def.name
                        ),
                        "Add the column as nullable, backfill it, then set NOT NULL. Alternatively provide a DEFAULT if one is appropriate.",
                    ));
                }
            }
            AlterTableOperation::AlterColumn { column_name, op } => match op {
                AlterColumnOperation::SetNotNull => {
                    findings.push(ctx.finding(
                        Rule::SetNotNull,
                        format!(
                            "SET NOT NULL on `{table}.{column_name}` scans the table while holding a lock that can block writes."
                        ),
                        "On PostgreSQL, validate a NOT VALID CHECK constraint first, then set NOT NULL. Elsewhere, schedule the scan during low traffic.",
                    ));
                }
                AlterColumnOperation::SetDataType { data_type, .. } => {
                    findings.push(ctx.finding(
                        Rule::ColumnTypeChange,
                        format!(
                            "Changing `{table}.{column_name}` to `{data_type}` can rewrite the table and block reads and writes while it runs."
                        ),
                        "Create a new column with the target type, backfill in batches, then swap columns under a short lock, or use a maintenance window.",
                    ));
                }
                _ => {}
            },
            AlterTableOperation::ModifyColumn {
                col_name,
                data_type,
                ..
            } => {
                findings.push(ctx.finding(
                    Rule::ColumnTypeChange,
                    format!(
                        "Changing `{table}.{col_name}` to `{data_type}` can rebuild the table and block reads and writes while it runs."
                    ),
                    "Use an online schema-change tool or a shadow table, then swap under a short lock.",
                ));
            }
            AlterTableOperation::ChangeColumn {
                new_name,
                data_type,
                ..
            } => {
                findings.push(ctx.finding(
                    Rule::ColumnTypeChange,
                    format!(
                        "Changing `{table}.{new_name}` to `{data_type}` can rebuild the table and block reads and writes while it runs."
                    ),
                    "Use an online schema-change tool or a shadow table, then swap under a short lock.",
                ));
                findings.push(ctx.finding(
                    Rule::Rename,
                    format!(
                        "Renaming a column to `{new_name}` in `{table}` breaks clients that still use the old name."
                    ),
                    "Use expand/contract: add the new name, update readers and writers, then remove the old name in a later release.",
                ));
            }
            AlterTableOperation::RenameColumn {
                old_column_name,
                new_column_name,
            } => {
                findings.push(ctx.finding(
                    Rule::Rename,
                    format!(
                        "Renaming `{table}.{old_column_name}` to `{new_column_name}` breaks queries, views, and clients that still use the old name."
                    ),
                    "Use expand/contract: add the new name, update readers and writers, then remove the old name in a later release.",
                ));
            }
            AlterTableOperation::RenameTable { table_name } => {
                let new_name = match table_name {
                    RenameTableNameKind::To(name) | RenameTableNameKind::As(name) => name,
                };
                findings.push(ctx.finding(
                    Rule::Rename,
                    format!(
                        "Renaming table `{table}` to `{new_name}` immediately breaks clients that still use the old name."
                    ),
                    "Use expand/contract: add the new name, update readers and writers, then remove the old name in a later release.",
                ));
            }
            AlterTableOperation::AddConstraint {
                constraint: TableConstraint::ForeignKey(foreign_key),
                not_valid: false,
            } if ctx.dialect.is_postgres() => {
                findings.push(ctx.finding(
                    Rule::ForeignKeyWithoutNotValid,
                    format!(
                        "Adding a foreign key from `{table}` to `{}` validates every existing row while holding locks.",
                        foreign_key.foreign_table
                    ),
                    "Add the constraint with NOT VALID, then run ALTER TABLE ... VALIDATE CONSTRAINT as a separate step.",
                ));
            }
            _ => {}
        }
    }

    findings
}

fn analyze_vacuum(vacuum: &VacuumStatement, ctx: &RuleContext<'_>) -> Outcome {
    if !vacuum.full || !ctx.dialect.is_postgres() {
        return Outcome::Analyzed(Vec::new());
    }
    let table = vacuum
        .table_name
        .as_ref()
        .map_or_else(|| "the database".to_string(), ToString::to_string);
    Outcome::Analyzed(vec![ctx.finding(
        Rule::LockingStatement,
        format!(
            "VACUUM FULL rewrites {table} while holding an ACCESS EXCLUSIVE lock, blocking reads and writes."
        ),
        "Use regular VACUUM, pg_repack, or a maintenance window. Lock duration depends on table size and workload.",
    )])
}

/// Handle constructs that `sqlparser` cannot parse but that are common in
/// PostgreSQL migrations. Returning `None` leaves the original parse error in
/// place.
pub fn analyze_unsupported(ctx: &RuleContext<'_>, raw: &str) -> Option<Vec<Finding>> {
    let dialect = ctx.dialect.parser_dialect();
    let tokens = Tokenizer::new(dialect.as_ref(), raw).tokenize().ok()?;
    let words: Vec<&sqlparser::tokenizer::Word> = tokens
        .iter()
        .filter_map(|token| match token {
            Token::Word(word) => Some(word),
            _ => None,
        })
        .collect();
    let first = words.first()?;

    if first.keyword == Keyword::DO {
        return Some(vec![ctx.finding(
            Rule::Unanalyzed,
            "SQL inside a DO block cannot be inspected statically; it may contain destructive or blocking statements.",
            "Review the block manually, or move its SQL into regular migration statements that can be checked.",
        )]);
    }

    if !ctx.dialect.is_postgres() {
        return None;
    }

    match first.keyword {
        Keyword::CLUSTER => Some(vec![ctx.finding(
            Rule::LockingStatement,
            "CLUSTER rewrites table data while holding an ACCESS EXCLUSIVE lock, blocking reads and writes.",
            "Use pg_repack where possible, or schedule CLUSTER during a maintenance window.",
        )]),
        Keyword::VACUUM if words.iter().any(|word| word.keyword == Keyword::FULL) => {
            Some(vec![ctx.finding(
                Rule::LockingStatement,
                "VACUUM FULL rewrites the table while holding an ACCESS EXCLUSIVE lock, blocking reads and writes.",
                "Use regular VACUUM, pg_repack, or a maintenance window. Lock duration depends on table size and workload.",
            )])
        }
        Keyword::DROP
            if words.get(1).is_some_and(|word| word.keyword == Keyword::INDEX)
                && words
                    .get(2)
                    .is_some_and(|word| word.keyword == Keyword::CONCURRENTLY)
                && words.iter().skip(3).any(|word| {
                    !matches!(
                        word.keyword,
                        Keyword::IF | Keyword::EXISTS | Keyword::CASCADE | Keyword::RESTRICT
                    )
                }) =>
        {
            // DROP INDEX CONCURRENTLY is not in sqlparser 0.63's AST. It does
            // not take the blocking lock that the normal form does.
            Some(Vec::new())
        }
        _ => None,
    }
}

/// Build the MG013 warning for a parsed statement outside the rule set.
pub fn unanalyzed(ctx: &RuleContext<'_>) -> Finding {
    ctx.finding(
        Rule::Unanalyzed,
        "This statement type is outside migration-guard's rule set, so its side effects were not checked.",
        "Review the statement manually, or split it into supported DDL and DML so future migrations can be checked.",
    )
}

fn quote_ident(name: &str) -> String {
    format!("`{name}`")
}

fn quote_list(names: &[String]) -> String {
    if names.is_empty() {
        return "the target object".to_string();
    }
    names
        .iter()
        .map(|name| quote_ident(name))
        .collect::<Vec<_>>()
        .join(", ")
}
