//! Native grammar validation for Python AST consumers. Raw source remains available on a rejected parse.
//! The tape owns the demanded, cached verdict; no Python interpreter is linked or invoked.
//!
//! The verdict is `ast.parse`'s as CPython 3.13 gives it (`cpython::syntax`, checked against CPython by
//! `cpython::parity`): its `SyntaxError`'s `lineno` and `msg`, the line as `src_line` counts it.

pub(super) fn error(source: &str) -> Option<(u32, String)> {
    crate::code_rule_dsl::cpython::syntax::syntax_error(source).map(|e| (e.line as u32, e.msg))
}
