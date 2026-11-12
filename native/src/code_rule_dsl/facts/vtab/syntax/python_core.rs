//! Python's reading of its core.rr facts (`core_rr::DialectFacts`): its strings decoded as CPython decodes them,
//! its syntax verdict CPython's grammar's, and its strings' constant parts.

use super::core_rr::DialectFacts;
use super::{Dialect, Projected};
use crate::code_rule_dsl::facts::core::Facts;

pub struct PythonFacts;

impl DialectFacts for PythonFacts {
    fn string_body(&self, prefix: &str, text: &str) -> Option<String> {
        if prefix.contains('b') {
            return None;
        }
        crate::code_rule_dsl::value::python::str_value(text)
    }
    /// Without `python-syntax-error` (the browser build), Python's verdict is tree-sitter's, as for the other
    /// languages: it misses what tree-sitter accepts and CPython does not (Python 2's `print "x"`). `Err`: `ast.parse`
    /// raises no `SyntaxError` but another exception, which fails the batch (`python_syntax::failure`).
    fn verdict(&self, f: &Facts) -> Option<Result<Option<(u32, String)>, String>> {
        #[cfg(feature = "python-syntax-error")]
        return Some(f.python_syntax.get_or_init(|| super::python_syntax::error(&f.source)).clone());
        #[cfg(not(feature = "python-syntax-error"))]
        {
            let _ = f;
            None
        }
    }
    fn str_parts(&self, p: &Projected, dialect: &Dialect, local: u32, words: bool) -> Vec<(String, i64, u8, usize)> {
        super::python_parts::from_tape(p, dialect, local, words)
    }
    fn is_word(&self, c: u32) -> bool {
        crate::code_rule_dsl::value::python::words::is_word(c)
    }
}
