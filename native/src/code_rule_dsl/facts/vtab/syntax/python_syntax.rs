//! Native grammar validation for Python AST consumers. Raw source remains available on a rejected parse.
//! The tape owns the demanded, cached verdict; no Python interpreter is linked or invoked.
//!
//! The verdict is `ast.parse`'s as CPython 3.13 gives it (`cpython::syntax`, checked against CPython by
//! `cpython::parity`): its `SyntaxError`'s `lineno` and `msg`, the line as `src_line` counts it. Where `ast.parse`
//! raises another exception (a format spec's escape it cannot decode: `UnicodeDecodeError`), W's worker, which catches
//! only `SyntaxError`, fails its whole batch: so does a batch that reads CPython's facts of that document
//! (`failure`).

pub(super) fn error(source: &str) -> Result<Option<(u32, String)>, String> {
    crate::code_rule_dsl::cpython::syntax::syntax_error(source)
        .map(|e| e.map(|e| (e.line as u32, e.msg)))
        .map_err(|r| format!("{}: {}", r.class, r.message))
}

/// The error a batch reading CPython's facts of document `path` fails with, where `ast.parse` raises no `SyntaxError`.
pub(super) fn failure(path: &str, raised: &str) -> crate::content_store::StoreError {
    crate::content_store::StoreError::Invalid(format!("{path}: CPython's ast.parse raises {raised}"))
}
