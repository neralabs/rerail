//! Reading a syntax tree's shape: named children, and the anonymous tokens (punctuation,
//! keywords, operators) between them (RER-131).
//!
//! Readers of trees use these instead of filtering `children()` by `is_named()` themselves, or
//! comparing an anonymous child's `kind()`. Every function here reads the tree's children. A tape
//! holds the named nodes and the tokens the grammar does not imply (`tape::tokens`), and the token
//! queries read those alone: an implied token is in no tape, and nothing here makes one up. A
//! reader that needs one asks the kind, the named children or the fields (`tape::tokens`'s classes
//! say which tokens a kind may leave out).
//!
//! - **Named children:** [`Tree::named`] (all), [`Tree::named_code`] (not comments),
//!   [`Tree::first_named`], [`Tree::last_named`], [`Tree::first_code`].
//! - **Tokens:** [`Tree::tokens`], [`Tree::has_token`], [`Tree::has_any_token`],
//!   [`Tree::first_token`], [`Tree::operator`], [`Tree::operator_token`].
//! - **Tokens beside a node:** [`Tree::token_before`], [`Tree::token_after`].
//! - **Layout:** [`Tree::parts`], named nodes and tokens in source order ([`Part`]).
//!
//! A [`Token`] is a value: its kind, text and span. Code that edits a token edits its span.

use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::ops::Range;

/// A comment, by kind: `comment`, and a grammar's own (`line_comment`, `html_comment`, ...).
pub(crate) fn is_comment(kind: &str) -> bool {
    kind.ends_with("comment")
}

/// Punctuation the tree's shape already records: separators and brackets.
pub fn is_punctuation(kind: &str) -> bool {
    matches!(kind, "," | ";" | "(" | ")" | "[" | "]" | "{" | "}" | ":")
}

/// An anonymous token the tree stores: a keyword, operator or punctuation of its parent node.
#[derive(Clone)]
pub struct Token<'r, D: Doc> {
    node: Node<'r, D>,
}

impl<'r, D: Doc> Token<'r, D> {
    /// The token's kind: its text in the grammar (`"+"`, `"async"`, `","`).
    pub fn kind(&self) -> Cow<'_, str> {
        self.node.kind()
    }
    /// The token's text in the source (`B"`, `FUNCTION`): may differ from its kind in case or
    /// prefix.
    pub fn text(&self) -> Cow<'r, str> {
        self.node.text()
    }
    /// The token's byte span in the source.
    pub fn range(&self) -> Range<usize> {
        self.node.range()
    }
}

/// A child in source order: a named node, or one of the parent's tokens.
pub enum Part<'r, D: Doc> {
    Named(Node<'r, D>),
    Token(Token<'r, D>),
}

impl<'r, D: Doc> Part<'r, D> {
    /// The part's kind.
    pub(crate) fn kind(&self) -> Cow<'_, str> {
        match self {
            Self::Named(node) => node.kind(),
            Self::Token(token) => token.kind(),
        }
    }
    /// The part's byte span.
    pub(crate) fn range(&self) -> Range<usize> {
        match self {
            Self::Named(node) => node.range(),
            Self::Token(token) => token.range(),
        }
    }
    /// The named node, if the part is one.
    pub(crate) fn named(&self) -> Option<&Node<'r, D>> {
        match self {
            Self::Named(node) => Some(node),
            Self::Token(_) => None,
        }
    }
    /// The token, if the part is one.
    pub(crate) fn token(&self) -> Option<&Token<'r, D>> {
        match self {
            Self::Named(_) => None,
            Self::Token(token) => Some(token),
        }
    }
}

/// Shape queries on a tree node; see the module documentation.
pub trait Tree<'r, D: Doc> {
    /// The named children, comments included.
    fn named(&self) -> impl Iterator<Item = Node<'r, D>> + '_;
    /// The named children that are not comments: what a reader of code walks.
    fn named_code(&self) -> impl Iterator<Item = Node<'r, D>> + '_;
    /// The first named child, a comment included.
    fn first_named(&self) -> Option<Node<'r, D>> {
        self.named().next()
    }
    /// The last named child, a comment included.
    fn last_named(&self) -> Option<Node<'r, D>> {
        self.named().last()
    }
    /// The first named child that is not a comment.
    fn first_code(&self) -> Option<Node<'r, D>> {
        self.named_code().next()
    }
    /// The node's own tokens, in order.
    fn tokens(&self) -> impl Iterator<Item = Token<'r, D>> + '_;
    /// The children in source order, named nodes and tokens interleaved: for a reader of a node's
    /// layout (a string's delimiters and pieces, say).
    fn parts(&self) -> impl Iterator<Item = Part<'r, D>> + '_;
    /// Whether the node has a token of kind `kind`.
    fn has_token(&self, kind: &str) -> bool {
        self.tokens().any(|token| token.kind() == kind)
    }
    /// Whether the node has a token of any of `kinds`.
    fn has_any_token(&self, kinds: &[&str]) -> bool {
        self.tokens().any(|token| kinds.contains(&token.kind().as_ref()))
    }
    /// The node's first token.
    fn first_token(&self) -> Option<Token<'r, D>> {
        self.tokens().next()
    }
    /// The kind of the token in the node's `operator` field (`"+"`, `"not in"`).
    fn operator(&self) -> Option<String> {
        self.operator_token().map(|token| token.kind().into_owned())
    }
    /// The token in the node's `operator` field.
    fn operator_token(&self) -> Option<Token<'r, D>>;
    /// The token just before this node among its parent's children, if that child is a token.
    fn token_before(&self) -> Option<Token<'r, D>>;
    /// The token just after this node among its parent's children, if that child is a token.
    fn token_after(&self) -> Option<Token<'r, D>>;
}

impl<'r, D: Doc> Tree<'r, D> for Node<'r, D> {
    fn named(&self) -> impl Iterator<Item = Node<'r, D>> + '_ {
        self.children().filter(|child| child.is_named())
    }
    fn named_code(&self) -> impl Iterator<Item = Node<'r, D>> + '_ {
        self.children().filter(|child| child.is_named() && !is_comment(&child.kind()))
    }
    fn tokens(&self) -> impl Iterator<Item = Token<'r, D>> + '_ {
        self.children().filter(|child| !child.is_named()).map(|node| Token { node })
    }
    fn parts(&self) -> impl Iterator<Item = Part<'r, D>> + '_ {
        self.children().map(part)
    }
    fn operator_token(&self) -> Option<Token<'r, D>> {
        self.field("operator").map(|node| Token { node })
    }
    fn token_before(&self) -> Option<Token<'r, D>> {
        self.prev().filter(|node| !node.is_named()).map(|node| Token { node })
    }
    fn token_after(&self) -> Option<Token<'r, D>> {
        self.next().filter(|node| !node.is_named()).map(|node| Token { node })
    }
}

/// A stored child as a part.
fn part<'r, D: Doc>(child: Node<'r, D>) -> Part<'r, D> {
    if child.is_named() { Part::Named(child) } else { Part::Token(Token { node: child }) }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use std::sync::Arc;

    /// The named queries answer as on tree-sitter's tree, and the token queries read what the
    /// tape stores: each node's parts are tree-sitter's less the tokens the writer leaves out, with
    /// tree-sitter's spans, and no part is made up.
    #[test]
    fn shape_queries_agree_on_native_trees_and_restored_tapes() {
        for (lang, source) in [
            (GrammarDialect::Python, "async def f(a, b=1):  # c\n    return a if a is not b else [x for x in b]\n"),
            (GrammarDialect::TypeScript, "const f = async (a, b?) => { let x = [1,,2]; return a?.b ?? 0; };\n"),
            (GrammarDialect::Rust, "fn f(x: &[u8]) -> Option<u8> { if let Some(y) = x.first() { return Some(*y + 1); } None }\n"),
            (GrammarDialect::Php, "<?php\nFUNCTION f($a, $b = [1, 2,]) { return $a ?? $b; }\n"),
        ] {
            let native = StrDoc::new(source, lang);
            let grammar = Grammar::new(lang).unwrap();
            let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
            let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
            let stored: Vec<Vec<(String, std::ops::Range<usize>, bool)>> = {
                let mut out = Vec::new();
                let mut stack = vec![native.tree.root_node()];
                while let Some(node) = stack.pop() {
                    let children = crate::code_rule_dsl::tape::stored_children_for_test(lang, node);
                    if node.is_named() {
                        out.push(children.iter().map(|(child, _)| (child.kind().to_owned(), child.byte_range(), !child.is_named())).collect());
                    }
                    stack.extend(children.into_iter().rev().map(|(child, _)| child));
                }
                out
            };
            let (native, tape) = (AstGrep::doc(native), AstGrep::doc(tape));
            assert_eq!(named(tape.root()), named(native.root()), "{lang:?}");
            let tape_parts: Vec<_> = parts(tape.root()).into_iter().map(|(_, parts)| parts).collect();
            assert_eq!(tape_parts, stored, "{lang:?}");
            let all: usize = parts(native.root()).iter().flat_map(|(_, parts)| parts).filter(|part| part.2).count();
            let kept: usize = tape_parts.iter().flatten().filter(|part| part.2).count();
            assert!(kept < all, "{lang:?}: the tape leaves tokens out, and no query makes them up");
        }
    }

    /// Each named node's parts: kind, span, and whether a token.
    fn parts<D: Doc>(root: Node<'_, D>) -> Vec<(String, Vec<(String, std::ops::Range<usize>, bool)>)> {
        root.dfs()
            .filter(|node| node.is_named())
            .map(|node| (format!("{} {:?}", node.kind(), node.range()), node.parts().map(|part| (part.kind().into_owned(), part.range(), part.token().is_some())).collect()))
            .collect()
    }

    /// Each named node's named children, as each query reads them.
    fn named<D: Doc>(root: Node<'_, D>) -> Vec<String> {
        root.dfs()
            .filter(|node| node.is_named())
            .map(|node| {
                let named: Vec<_> = node.named().map(|n| n.range()).collect();
                let code: Vec<_> = node.named_code().map(|n| n.range()).collect();
                let first = node.first_named().map(|n| n.range());
                let last = node.last_named().map(|n| n.range());
                format!("{} {:?} {named:?} {code:?} {first:?} {last:?}", node.kind(), node.range())
            })
            .collect()
    }
}
