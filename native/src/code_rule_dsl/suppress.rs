//! `suppress` (a Rerail extension, opt-in per violation): another linter's suppression comments,
//! honoured as that linter honours them. The engine never reads them otherwise. Rerail's own
//! `rerail-ignore` comments (`rerail`) are read for every code rule.

use ast_grep_core::{Doc, Node};
pub(crate) use rerail::RerailIgnores;
// The kinds and what a violation asks of them, below the linters (`kinds`, the lang crate's).
pub(crate) mod kinds;
pub(crate) use kinds::*;

/// The root of the tree a node belongs to.
pub(crate) fn root<'t, D: Doc>(node: &Node<'t, D>) -> Node<'t, D> {
    let mut root = node.clone();
    while let Some(parent) = root.parent() {
        root = parent;
    }
    root
}

/// The end of PHP's comment token for a tree-sitter-php `comment` node: the grammar's `//` or `#`
/// comment that ends in `?` takes the line break after it, which PHP leaves to white space.
fn php_comment_end(source: &str, range: &std::ops::Range<usize>) -> usize {
    let text = &source[range.clone()];
    if text.starts_with("/*") {
        return range.end;
    }
    let text = text
        .strip_suffix('\n')
        .map_or(text, |text| text.strip_suffix('\r').unwrap_or(text));
    range.start + text.len()
}

pub(crate) mod biome;
pub(crate) mod clippy;
pub(crate) mod eslint;
#[cfg(test)]
mod eslint_tests;
pub(crate) mod noqa;
pub(crate) mod nosec;
pub(crate) mod oxlint;
#[cfg(test)]
mod php_corpus_tests;
pub(crate) mod phpcs;
pub(crate) mod phpstan;
pub(crate) mod psalm;
mod rerail;
#[cfg(test)]
mod ruff;
pub(crate) mod sqlfluff;
pub(crate) mod vue;

#[cfg(test)]
mod tests;
