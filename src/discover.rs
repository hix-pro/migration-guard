use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::analyze::has_sql_extension;
use crate::style::Style;

/// Maximum directory depth below the discovery root. Deep monorepos often put
/// a Flyway directory under `apps/<name>/src/main/resources/db/migration`.
const MAX_DEPTH: usize = 10;

/// Directories that never contain hand-written migrations worth scanning and
/// can be very large. Matching is case-insensitive.
const SKIP_DIRS: &[&str] = &[
    ".cache",
    ".git",
    ".gradle",
    ".hg",
    ".idea",
    ".mypy_cache",
    ".next",
    ".nuxt",
    ".pytest_cache",
    ".svn",
    ".turbo",
    ".venv",
    ".vscode",
    "__pycache__",
    "bin",
    "build",
    "coverage",
    "deriveddata",
    "dist",
    "env",
    "logs",
    "node_modules",
    "obj",
    "out",
    "pods",
    "target",
    "temp",
    "tmp",
    "vendor",
    "venv",
];

/// Best-effort framework label for a discovered directory. The label is
/// informational; the SQL rules are the same for every framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Framework {
    Flyway,
    Liquibase,
    Prisma,
    Drizzle,
    Supabase,
    Generic,
}

impl Framework {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Flyway => "Flyway",
            Self::Liquibase => "Liquibase",
            Self::Prisma => "Prisma",
            Self::Drizzle => "Drizzle",
            Self::Supabase => "Supabase",
            Self::Generic => "Generic SQL",
        }
    }
}

/// One directory that discovery considers a migration source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationDir {
    /// Directory relative to the discovery root, using `/` separators.
    pub path: String,
    pub framework: Framework,
    /// Number of `.sql` files found recursively below `path`.
    pub sql_files: usize,
}

/// Result of scanning a project root for known migration directories.
#[derive(Debug, Clone, Serialize)]
pub struct Discovery {
    pub schema_version: u32,
    pub tool: &'static str,
    pub version: &'static str,
    /// Root exactly as supplied on the command line.
    pub root: PathBuf,
    pub directories: Vec<MigrationDir>,
}

impl Discovery {
    /// Filesystem paths to pass to the analyzer. When the root is `.`, the
    /// paths stay relative and do not gain a `./` prefix.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.directories
            .iter()
            .map(|directory| {
                if self.root == Path::new(".") {
                    PathBuf::from(&directory.path)
                } else {
                    self.root.join(&directory.path)
                }
            })
            .collect()
    }

    pub fn sql_files(&self) -> usize {
        self.directories
            .iter()
            .map(|directory| directory.sql_files)
            .sum()
    }
}

/// Find known migration directories below `root`.
///
/// Discovery is deliberately conservative: it only reports a directory when
/// it matches a known framework layout *and* contains at least one `.sql`
/// file. Dependency, build, cache, and VCS directories are skipped, as are
/// symlinks, so the walk stays inside the project.
pub fn discover(root: &Path) -> Result<Discovery> {
    let metadata = fs::metadata(root)
        .with_context(|| format!("cannot access discovery root `{}`", root.display()))?;
    if !metadata.is_dir() {
        bail!("discovery root is not a directory: `{}`", root.display());
    }

    let mut directories = Vec::new();
    walk(root, root, 0, &mut directories)?;
    directories.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(Discovery {
        schema_version: 1,
        tool: "migration-guard",
        version: env!("CARGO_PKG_VERSION"),
        root: root.to_path_buf(),
        directories,
    })
}

fn walk(
    root: &Path,
    current: &Path,
    depth: usize,
    directories: &mut Vec<MigrationDir>,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }

    if depth > 0 {
        let relative = current.strip_prefix(root).unwrap_or(current);
        if let Some(framework) = classify(relative) {
            let sql_files = count_sql_files(current)?;
            if sql_files > 0 {
                directories.push(MigrationDir {
                    path: display_relative(relative),
                    framework,
                    sql_files,
                });
            }
            // A matched directory is scanned recursively by the analyzer, so
            // there is no reason to look for nested migration directories.
            return Ok(());
        }
    }

    let mut entries = read_dir(current)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_type = entry
            .file_type()
            .with_context(|| format!("cannot inspect `{}`", entry.path().display()))?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        if should_skip(&entry.file_name().to_string_lossy()) {
            continue;
        }
        walk(root, &entry.path(), depth + 1, directories)?;
    }
    Ok(())
}

fn count_sql_files(directory: &Path) -> Result<usize> {
    let mut count = 0;
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        let mut entries = read_dir(&current)?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let file_type = entry
                .file_type()
                .with_context(|| format!("cannot inspect `{}`", entry.path().display()))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if !should_skip(&entry.file_name().to_string_lossy()) {
                    pending.push(entry.path());
                }
            } else if file_type.is_file() && has_sql_extension(&entry.path()) {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn read_dir(directory: &Path) -> Result<Vec<fs::DirEntry>> {
    fs::read_dir(directory)
        .with_context(|| format!("cannot read directory `{}`", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("cannot read directory `{}`", directory.display()))
}

fn should_skip(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SKIP_DIRS.iter().any(|skip| *skip == name)
}

fn display_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Classify a relative directory path using the conventions of common SQL
/// migration tools. Matching is case-insensitive.
fn classify(relative: &Path) -> Option<Framework> {
    let parts: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let last = parts.last()?.as_str();

    if ends_with(&parts, &["prisma", "migrations"]) {
        Some(Framework::Prisma)
    } else if ends_with(&parts, &["supabase", "migrations"]) {
        Some(Framework::Supabase)
    } else if ends_with(&parts, &["db", "migration"]) {
        Some(Framework::Flyway)
    } else if ends_with(&parts, &["db", "changelog"]) || ends_with(&parts, &["db", "changelogs"]) {
        Some(Framework::Liquibase)
    } else if last == "drizzle" || ends_with(&parts, &["drizzle", "migrations"]) {
        Some(Framework::Drizzle)
    } else if matches!(last, "migrations" | "migration" | "migrate") {
        Some(Framework::Generic)
    } else {
        None
    }
}

fn ends_with(parts: &[String], suffix: &[&str]) -> bool {
    parts.len() >= suffix.len()
        && parts[parts.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(part, expected)| part.as_str() == *expected)
}

pub fn render_json(discovery: &Discovery) -> Result<String> {
    let mut output = serde_json::to_string_pretty(discovery)?;
    output.push('\n');
    Ok(output)
}

pub fn render_text(discovery: &Discovery, style: Style) -> String {
    let count = discovery.directories.len();
    let files = discovery.sql_files();
    let mut output = format!(
        "{}  ·  discovery  ·  {} {}, {} {}\n",
        style.bold(&format!("migration-guard {}", discovery.version)),
        count,
        plural(count, "directory", "directories"),
        files,
        plural(files, "SQL file", "SQL files")
    );
    output.push_str(&format!("root: {}\n\n", discovery.root.display()));

    if discovery.directories.is_empty() {
        output.push_str("No SQL migration directories found.\n\n");
        output.push_str(
            "Looked for Prisma, Drizzle, Flyway, Liquibase, Supabase, and generic migrations directories.\n",
        );
        return output;
    }

    for directory in &discovery.directories {
        output.push_str(&format!(
            "  {:<11} {}  {}\n",
            directory.framework.label(),
            style.bold(&directory.path),
            style.dim(&format!(
                "({} {})",
                directory.sql_files,
                plural(directory.sql_files, "file", "files")
            ))
        ));
    }
    output.push('\n');
    output.push_str(&format!(
        "Run {} to analyze these directories.\n",
        style.bold("migration-guard check")
    ));
    output
}

fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct TempTree(PathBuf);

    impl TempTree {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "migration-guard-discover-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn paths(discovery: &Discovery) -> Vec<&str> {
        discovery
            .directories
            .iter()
            .map(|directory| directory.path.as_str())
            .collect()
    }

    #[test]
    fn finds_and_classifies_common_framework_directories() {
        let tree = TempTree::new();
        tree.write("prisma/migrations/20240101_init/migration.sql", "SELECT 1;");
        tree.write("supabase/migrations/20240101_init.sql", "SELECT 1;");
        tree.write(
            "services/api/src/main/resources/db/migration/V1__init.sql",
            "SELECT 1;",
        );
        tree.write("db/changelog/changes/001.sql", "SELECT 1;");
        tree.write("drizzle/0000_init.sql", "SELECT 1;");
        tree.write("migrations/001.sql", "SELECT 1;");
        tree.write("packages/db/migrations/002.sql", "SELECT 1;");
        tree.write("docs/migrations/readme.md", "not sql");
        tree.write("notes/readme.md", "not sql");

        let discovery = discover(&tree.0).unwrap();
        assert_eq!(
            paths(&discovery),
            vec![
                "db/changelog",
                "drizzle",
                "migrations",
                "packages/db/migrations",
                "prisma/migrations",
                "services/api/src/main/resources/db/migration",
                "supabase/migrations",
            ]
        );

        let frameworks: Vec<Framework> = discovery
            .directories
            .iter()
            .map(|directory| directory.framework)
            .collect();
        assert_eq!(
            frameworks,
            vec![
                Framework::Liquibase,
                Framework::Drizzle,
                Framework::Generic,
                Framework::Generic,
                Framework::Prisma,
                Framework::Flyway,
                Framework::Supabase,
            ]
        );
        assert!(discovery.directories.iter().all(|dir| dir.sql_files == 1));
    }

    #[test]
    fn skips_dependency_build_and_cache_directories() {
        let tree = TempTree::new();
        tree.write("node_modules/pkg/migrations/001.sql", "SELECT 1;");
        tree.write("target/debug/migrations/001.sql", "SELECT 1;");
        tree.write("vendor/lib/migrations/001.sql", "SELECT 1;");
        tree.write("dist/migrations/001.sql", "SELECT 1;");
        tree.write(".cache/migrations/001.sql", "SELECT 1;");
        tree.write("app/migrations/001.sql", "SELECT 1;");

        let discovery = discover(&tree.0).unwrap();
        assert_eq!(paths(&discovery), vec!["app/migrations"]);
    }

    #[test]
    fn requires_at_least_one_sql_file() {
        let tree = TempTree::new();
        tree.write("migrations/README.md", "documentation only");
        tree.write(
            "prisma/migrations/20240101_init/migration.txt",
            "not a SQL file",
        );
        tree.write("db/migrations/20240101_init/up.sql", "SELECT 1;");

        let discovery = discover(&tree.0).unwrap();
        assert_eq!(paths(&discovery), vec!["db/migrations"]);
        assert_eq!(discovery.sql_files(), 1);
    }

    #[test]
    fn counts_nested_sql_without_duplicating_the_directory() {
        let tree = TempTree::new();
        tree.write("migrations/001.sql", "SELECT 1;");
        tree.write("migrations/archive/002.sql", "SELECT 1;");
        tree.write("migrations/archive/003.sql", "SELECT 1;");

        let discovery = discover(&tree.0).unwrap();
        assert_eq!(paths(&discovery), vec!["migrations"]);
        assert_eq!(discovery.directories[0].sql_files, 3);
    }

    #[test]
    fn paths_are_relative_to_the_root() {
        let tree = TempTree::new();
        tree.write("prisma/migrations/20240101_init/migration.sql", "SELECT 1;");

        let discovery = discover(&tree.0).unwrap();
        assert_eq!(
            discovery.paths(),
            vec![tree.0.join("prisma").join("migrations")]
        );
        assert_eq!(discovery.directories[0].path, "prisma/migrations");
    }

    #[test]
    fn empty_root_is_a_clean_empty_result() {
        let tree = TempTree::new();
        let discovery = discover(&tree.0).unwrap();
        assert!(discovery.directories.is_empty());
        assert_eq!(discovery.sql_files(), 0);
    }

    #[test]
    fn rejects_a_missing_or_non_directory_root() {
        let tree = TempTree::new();
        let missing = discover(&tree.0.join("missing")).unwrap_err();
        assert!(format!("{missing:#}").contains("cannot access discovery root"));

        tree.write("file.sql", "SELECT 1;");
        let file = discover(&tree.0.join("file.sql")).unwrap_err();
        assert!(format!("{file:#}").contains("is not a directory"));
    }

    #[test]
    fn json_output_has_stable_fields() {
        let tree = TempTree::new();
        tree.write("prisma/migrations/20240101_init/migration.sql", "SELECT 1;");
        let discovery = discover(&tree.0).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&render_json(&discovery).unwrap()).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["tool"], "migration-guard");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["directories"][0]["path"], "prisma/migrations");
        assert_eq!(value["directories"][0]["framework"], "prisma");
        assert_eq!(value["directories"][0]["sql_files"], 1);
    }

    #[test]
    fn text_output_lists_frameworks_and_paths() {
        let tree = TempTree::new();
        tree.write("db/migration/V1__init.sql", "SELECT 1;");
        let discovery = discover(&tree.0).unwrap();
        let text = render_text(&discovery, Style::disabled());

        assert!(text.contains("migration-guard"));
        assert!(text.contains("discovery"));
        assert!(text.contains("Flyway"));
        assert!(text.contains("db/migration"));
        assert!(text.contains("migration-guard check"));
        assert!(!text.contains("\x1b["));
    }
}
