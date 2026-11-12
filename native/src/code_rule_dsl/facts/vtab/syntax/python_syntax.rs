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

pub use super::core_rr::failure;

#[cfg(test)]
pub mod tests {
    /// Python's verdict is CPython's grammar's: Python 2's `print "x"`, which tree-sitter accepts, is an error; a
    /// `match` statement is not; the line is the first error's.
    #[test]
    #[cfg(feature = "python-syntax-error")]
    fn python_syntax_errors_follow_cpythons_grammar() {
        use super::error as python_error;
        assert_eq!(python_error("x = 1\n"), Ok(None));
        assert_eq!(python_error("match x:\n    case 1:\n        pass\n"), Ok(None));
        assert_eq!(python_error("y = 2\nprint \"x\"\n").map(|e| e.map(|e| e.0)), Ok(Some(2)));
        assert_eq!(python_error("y = 2\r\nz = 3\rdef (:\n").map(|e| e.map(|e| e.0)), Ok(Some(3)), "lines as src_line counts them");
        assert!(python_error("f\"{x:\\x}\"\n").is_err_and(|e| e.starts_with("UnicodeDecodeError: ")), "ast.parse raises");
    }
}
