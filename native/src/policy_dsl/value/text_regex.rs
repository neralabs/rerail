//! `value.regex`: Rust regex syntax over a decoded string, and backreferences beyond it. A pattern
//! the `regex` crate refuses only for its backreferences (`(.).*\1`, Ruff's B005 asks whether a
//! character repeats) runs on fancy-regex, a backtracking engine, with a backtrack limit and a cap
//! on the text it reads; past either the verdict is unknown.

use crate::policy_dsl::value::MAX_STRING_BYTES;
use regex::Regex;
use regex_syntax::ast::ErrorKind;

/// Backtracking steps one match may take.
pub(crate) const BACKTRACK_LIMIT: usize = 1_000_000;

#[derive(Clone, Debug)]
pub(crate) enum TextRegex {
    Plain(Regex),
    /// A pattern with backreferences.
    Backreferences(fancy_regex::Regex),
}

impl TextRegex {
    /// Compiles a pattern: Rust syntax, or Rust syntax with backreferences. `None` for anything
    /// else (look-around, atomic groups, conditionals, a malformed pattern).
    pub(crate) fn new(pattern: &str) -> Option<Self> {
        if let Ok(regex) = Regex::new(pattern) {
            return Some(Self::Plain(regex));
        }
        // Only a pattern whose one fault is a backreference goes on.
        let error = regex_syntax::ast::parse::Parser::new()
            .parse(pattern)
            .err()?;
        if *error.kind() != ErrorKind::UnsupportedBackreference {
            return None;
        }
        let tree = fancy_regex::Expr::parse_tree(pattern).ok()?;
        if !backreferences_only(&tree.expr) || !has_backreference(&tree.expr) {
            return None;
        }
        fancy_regex::RegexBuilder::new(pattern)
            .backtrack_limit(BACKTRACK_LIMIT)
            .build()
            .ok()
            .map(Self::Backreferences)
    }

    /// Whether the pattern matches `text`: `None` when a backreference pattern ran out of its
    /// backtrack limit or the text is past 64 KiB.
    pub(crate) fn matches(&self, text: &str) -> Option<bool> {
        match self {
            Self::Plain(regex) => Some(regex.is_match(text)),
            Self::Backreferences(regex) => {
                if text.len() > MAX_STRING_BYTES {
                    return None;
                }
                regex.is_match(text).ok()
            }
        }
    }
}

/// Whether the parsed pattern holds a backreference.
fn has_backreference(expr: &fancy_regex::Expr) -> bool {
    use fancy_regex::Expr;
    match expr {
        Expr::Backref(_) => true,
        Expr::Concat(items) | Expr::Alt(items) => items.iter().any(has_backreference),
        Expr::Group(inner) => has_backreference(inner),
        Expr::Repeat { child, .. } => has_backreference(child),
        _ => false,
    }
}

/// Whether the parsed pattern uses no fancy-regex construct but backreferences.
fn backreferences_only(expr: &fancy_regex::Expr) -> bool {
    use fancy_regex::Expr;
    match expr {
        Expr::Empty
        | Expr::Any { .. }
        | Expr::Assertion(_)
        | Expr::Literal { .. }
        | Expr::Delegate { .. }
        | Expr::Backref(_) => true,
        Expr::Concat(items) | Expr::Alt(items) => items.iter().all(backreferences_only),
        Expr::Group(inner) => backreferences_only(inner),
        Expr::Repeat { child, .. } => backreferences_only(child),
        Expr::LookAround(..)
        | Expr::AtomicGroup(_)
        | Expr::KeepOut
        | Expr::ContinueFromPreviousMatchEnd
        | Expr::BackrefExistsCondition(_)
        | Expr::Conditional { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backreferences_run_on_fancy_regex_and_nothing_else_does() {
        let repeat = TextRegex::new("(?s)(.).*\\1").expect("backreference");
        assert!(matches!(repeat, TextRegex::Backreferences(_)));
        assert_eq!(repeat.matches("test"), Some(true));
        assert_eq!(repeat.matches("abc"), Some(false));
        assert_eq!(repeat.matches("ああ"), Some(true));
        assert_eq!(repeat.matches("a\na"), Some(true));
        assert!(matches!(TextRegex::new("^a+$"), Some(TextRegex::Plain(_))));
        for refused in [
            "(?=a)",
            "(a)(?=b)\\1",
            "(?>a)\\1",
            "(",
            "\\1(",
            "(a)(?(1)b)\\1",
        ] {
            assert!(TextRegex::new(refused).is_none(), "{refused}");
        }
        // Past the cap, or the backtrack limit, the verdict is unknown.
        let long = "ab".repeat(MAX_STRING_BYTES);
        assert_eq!(repeat.matches(&long), None);
        let catastrophic = TextRegex::new("^(a+)+\\1b$").expect("backreference");
        assert_eq!(catastrophic.matches(&"a".repeat(64)), None);
    }
}
