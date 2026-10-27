use std::env;
use std::io::IsTerminal;

use crate::model::Severity;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[1;31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[1;32m";

/// ANSI styling that is enabled only for interactive terminals and disabled by
/// the `NO_COLOR` convention (<https://no-color.org/>).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    enabled: bool,
}

impl Style {
    pub fn auto() -> Self {
        let enabled = std::io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none();
        Self { enabled }
    }

    pub const fn disabled() -> Self {
        Self { enabled: false }
    }

    #[cfg(test)]
    const fn forced() -> Self {
        Self { enabled: true }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    pub fn bold(self, text: &str) -> String {
        self.paint(BOLD, text)
    }

    pub fn dim(self, text: &str) -> String {
        self.paint(DIM, text)
    }

    pub fn pass(self, text: &str) -> String {
        self.paint(GREEN, text)
    }

    pub fn fail(self, text: &str) -> String {
        self.paint(RED, text)
    }

    pub fn severity(self, severity: Severity, text: &str) -> String {
        let code = match severity {
            Severity::Destructive => RED,
            Severity::Risky => YELLOW,
            Severity::Warning => CYAN,
        };
        self.paint(code, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_style_returns_plain_text() {
        let style = Style::disabled();
        assert_eq!(style.bold("hello"), "hello");
        assert_eq!(style.dim("hello"), "hello");
        assert_eq!(style.fail("FAIL"), "FAIL");
        assert_eq!(
            style.severity(Severity::Destructive, "destructive"),
            "destructive"
        );
    }

    #[test]
    fn forced_style_wraps_text() {
        let style = Style::forced();
        assert!(style.bold("hello").contains("\x1b[1m"));
        assert!(
            style
                .severity(Severity::Risky, "risky")
                .contains("\x1b[33m")
        );
        assert!(style.pass("PASS").contains("\x1b[1;32m"));
    }
}
