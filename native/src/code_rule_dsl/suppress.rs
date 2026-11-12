//! `suppress` (a Rerail extension, opt-in per violation): another linter's suppression comments,
//! honoured as that linter honours them. The engine never reads them otherwise. Rerail's own
//! `rerail-ignore` comments (`rerail`) are read for every code rule.

use ast_grep_core::{Doc, Node};
pub use rerail::RerailIgnores;
// The kinds and what a violation asks of them, below the linters (`kinds`, the lang crate's).
pub use rerail_lang::code_rule_dsl::suppress::kinds;
pub use kinds::*;

/// The root of the tree a node belongs to.
pub fn root<'t, D: Doc>(node: &Node<'t, D>) -> Node<'t, D> {
    let mut root = node.clone();
    while let Some(parent) = root.parent() {
        root = parent;
    }
    root
}

/// The end of PHP's comment token for a tree-sitter-php `comment` node: the grammar's `//` or `#`
/// comment that ends in `?` takes the line break after it, which PHP leaves to white space.
pub fn php_comment_end(source: &str, range: &std::ops::Range<usize>) -> usize {
    let text = &source[range.clone()];
    if text.starts_with("/*") {
        return range.end;
    }
    let text = text
        .strip_suffix('\n')
        .map_or(text, |text| text.strip_suffix('\r').unwrap_or(text));
    range.start + text.len()
}

// cratify: `biome` is rerail-lang-ecma's (it mounts it)
// cratify: `clippy` is rerail-lang-rust's (it mounts it)
// cratify: `eslint` is rerail-lang-ecma's (it mounts it)
// cratify: `eslint_tests` is rerail-engine's (it mounts it)
// cratify: `noqa` is rerail-lang-python's (it mounts it)
// cratify: `nosec` is rerail-lang-python's (it mounts it)
// cratify: `oxlint` is rerail-lang-ecma's (it mounts it)
// cratify: `php_corpus_tests` is rerail-lang-php's (it mounts it)
// cratify: `phpcs` is rerail-lang-php's (it mounts it)
// cratify: `phpstan` is rerail-lang-php's (it mounts it)
// cratify: `psalm` is rerail-lang-php's (it mounts it)
pub mod rerail;
// cratify: `ruff` is rerail-engine's (it mounts it)
// cratify: `sqlfluff` is rerail-lang-sql's (it mounts it)
// cratify: `vue` is rerail-lang-ecma's (it mounts it)

// cratify: `tests` is rerail-engine's (it mounts it)
