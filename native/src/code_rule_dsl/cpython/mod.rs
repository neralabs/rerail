//! CPython's own facts about Python source, computed natively: no Python process runs. Ruff's parser
//! (`ruff_python_parser` 0.0.10 from crates.io, Ruff's 0.16.4) reads the text as CPython's grammar does, and these modules give
//! what CPython's `ast` module would answer, for CPython 3.13 (`VERSION`), the version W's `pythonAst` worker required
//! and nera-data-layer's `.python-version` names:
//!
//! - `syntax`: `ast.parse`'s verdict, the line and message of its `SyntaxError` (core.rr's `syntax_error(d, l, msg)`).
//! - `literal`: `json.dumps(ast.literal_eval(node), sort_keys=True, separators=(",", ":"))` of each expression, W's
//!   pythonAst `literal_json` (core.rr's `literal_json(n, v)`).
//! - `value`: Python values as CPython builds and prints them (a str with lone surrogates, an int of any size, the
//!   reprs, `str.isprintable`).
//! - `ast`: the file's `ast` tree, CPython's classes, fields and spans, built from Ruff's.
//! - `unparse`: `ast.unparse` of an expression (core.rr's `@unparse(n)`); `dump`: `ast.dump(node,
//!   include_attributes=False)` (core.rr's `@ast_dump(n)`) and W's hashes of it, `dump_sha256`, `body_sha256` and
//!   `signature_sha256`; `printed`: those of each located node of a file, and the tree-sitter kinds W's worker read
//!   as each class.
//!
//! Real CPython runs only as the test oracle of `parity` (ignored tests, on the boxes).

pub(crate) mod ast;
pub(crate) mod dump;
pub(crate) mod literal;
pub(crate) mod printed;
pub(crate) mod syntax;
pub(crate) mod unparse;
pub(crate) mod value;

#[cfg(test)]
mod parity;
#[cfg(test)]
mod tests;

/// The CPython these facts are CPython's: its grammar decides the verdict (3.14's `except A, B:` and t-strings are
/// syntax errors here, as in 3.13).
pub(crate) const VERSION: ruff_python_ast::PythonVersion = ruff_python_ast::PythonVersion::PY313;
