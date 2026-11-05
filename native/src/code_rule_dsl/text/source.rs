//! How a rule spells a text test, and its parse to one syntax tree (`regex_syntax`'s HIR) whatever
//! the spelling. A new spelling is a variant here and its case in `Source::parse`.

use super::Error;
use regex_syntax::hir::Hir;

/// A test's spelling.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code, reason = "the framework's spellings: `Glob` and `OneOf` have no rule front end yet")]
pub(in crate::code_rule_dsl) enum Spelling {
    /// Rust `regex` syntax, as the `regex` crate reads it (W's matcher reads the same), with
    /// backreferences beyond it (`value.regex`, `value::text_regex`).
    Regex(String),
    /// A glob over a name or a string: `*` and `?` match any characters, `/` included.
    Glob(String),
    /// A glob over a path: `*`, `?` and `[…]` stay in one segment, `**` crosses segments
    /// (`logic::strings`'s `@glob`).
    PathGlob(String),
    Prefix(String),
    Suffix(String),
    Contains(String),
    Equal(String),
    /// Any of these strings, exactly.
    OneOf(Vec<String>),
}

/// A test: its spelling, and whether it ignores case.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) struct Source {
    pub(in crate::code_rule_dsl) spelling: Spelling,
    pub(in crate::code_rule_dsl) fold_case: bool,
}

/// A parsed test: a regular language, or a pattern only a backtracking engine runs.
pub(super) enum Parsed {
    Regular(Hir),
    Backtracking(String),
}

impl Source {
    pub(in crate::code_rule_dsl) fn regex(pattern: impl Into<String>) -> Self {
        Self { spelling: Spelling::Regex(pattern.into()), fold_case: false }
    }

    pub(in crate::code_rule_dsl) fn of(spelling: Spelling) -> Self {
        Self { spelling, fold_case: false }
    }

    pub(in crate::code_rule_dsl) fn folded(mut self) -> Self {
        self.fold_case = true;
        self
    }

    pub(super) fn parse(&self) -> Result<Parsed, Error> {
        let literal = |s: &str| regex_syntax::escape(s);
        let regex = match &self.spelling {
            Spelling::Regex(pattern) => {
                return match parse(pattern, self.fold_case) {
                    Ok(hir) => Ok(Parsed::Regular(hir)),
                    Err(error) => match crate::code_rule_dsl::value::text_regex::TextRegex::new(pattern) {
                        Some(crate::code_rule_dsl::value::text_regex::TextRegex::Backreferences(_)) => {
                            Ok(Parsed::Backtracking(if self.fold_case { format!("(?i){pattern}") } else { pattern.clone() }))
                        }
                        _ => Err(error),
                    },
                };
            }
            Spelling::Glob(glob) | Spelling::PathGlob(glob) => {
                let path = matches!(self.spelling, Spelling::PathGlob(_));
                let built = globset::GlobBuilder::new(glob)
                    .literal_separator(path)
                    .case_insensitive(self.fold_case)
                    .build()
                    .map_err(|e| Error(format!("glob {glob:?}: {e}")))?;
                // globset's regex reads bytes (`(?-u)`); the case is folded in it already.
                return parse_bytes(built.regex()).map(Parsed::Regular);
            }
            Spelling::Prefix(s) => format!("^{}", literal(s)),
            Spelling::Suffix(s) => format!("{}$", literal(s)),
            Spelling::Contains(s) => literal(s),
            Spelling::Equal(s) => format!("^{}$", literal(s)),
            Spelling::OneOf(strings) if strings.is_empty() => "[^\\s\\S]".to_owned(),
            Spelling::OneOf(strings) => format!("^(?:{})$", strings.iter().map(|s| literal(s)).collect::<Vec<_>>().join("|")),
        };
        parse(&regex, self.fold_case).map(Parsed::Regular)
    }
}

fn parse(pattern: &str, fold_case: bool) -> Result<Hir, Error> {
    regex_syntax::ParserBuilder::new()
        .case_insensitive(fold_case)
        .build()
        .parse(pattern)
        .map_err(|e| Error(format!("regex {pattern:?}: {e}")))
}

fn parse_bytes(pattern: &str) -> Result<Hir, Error> {
    regex_syntax::ParserBuilder::new()
        .utf8(false)
        .build()
        .parse(pattern)
        .map_err(|e| Error(format!("glob regex {pattern:?}: {e}")))
}
