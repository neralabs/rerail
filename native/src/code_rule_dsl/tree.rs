//! Reading a syntax tree's shape: named children, and the anonymous tokens (punctuation,
//! keywords, operators) between them (RER-131).
//!
//! Readers of trees use these instead of filtering `children()` by `is_named()` themselves, or
//! comparing an anonymous child's `kind()`. Stored tapes are named-only and restored whole today,
//! so every function here reads the tree's children; a later format can keep tokens out of the
//! in-memory tree, and then only this module changes: tokens become values recognized from the
//! source, as `tape::tokens` already rebuilds them.
//!
//! - **Named children:** [`Tree::named`] (all), [`Tree::named_code`] (not comments),
//!   [`Tree::first_named`], [`Tree::last_named`], [`Tree::first_code`].
//! - **Tokens:** [`Tree::tokens`], [`Tree::has_token`], [`Tree::has_any_token`],
//!   [`Tree::count_tokens`], [`Tree::first_token`], [`Tree::operator`], [`Tree::operator_token`].
//! - **Tokens beside a node:** [`Tree::token_before`], [`Tree::token_after`],
//!   [`Tree::count_tokens_before`].
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
pub(crate) fn is_punctuation(kind: &str) -> bool {
    matches!(kind, "," | ";" | "(" | ")" | "[" | "]" | "{" | "}" | ":")
}

/// An anonymous token: a keyword, operator or punctuation of its parent node.
#[derive(Clone)]
pub(crate) struct Token<'r, D: Doc> {
    node: Node<'r, D>,
}

impl<'r, D: Doc> Token<'r, D> {
    /// The token as a node: what a capture of it binds.
    pub(crate) fn node(&self) -> &Node<'r, D> {
        &self.node
    }
    /// The token's kind: its text in the grammar (`"+"`, `"async"`, `","`).
    pub(crate) fn kind(&self) -> Cow<'_, str> {
        self.node.kind()
    }
    /// The token's text in the source (`B"`, `FUNCTION`): may differ from its kind in case or
    /// prefix.
    pub(crate) fn text(&self) -> Cow<'r, str> {
        self.node.text()
    }
    /// The token's byte span in the source.
    pub(crate) fn range(&self) -> Range<usize> {
        self.node.range()
    }
    /// Whether the token is punctuation ([`is_punctuation`]).
    pub(crate) fn is_punctuation(&self) -> bool {
        is_punctuation(&self.kind())
    }
}

/// A child in source order: a named node, or one of the parent's tokens.
pub(crate) enum Part<'r, D: Doc> {
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
pub(crate) trait Tree<'r, D: Doc> {
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
    /// How many tokens of kind `kind` come before `child` (one of this node's children): an
    /// argument's index by its commas, say.
    fn count_tokens_before(&self, child: &Node<'r, D>, kind: &str) -> usize {
        let start = child.range().start;
        self.tokens().take_while(|token| token.range().end <= start).filter(|token| token.kind() == kind).count()
    }
    /// Whether the node has a token of kind `kind`.
    fn has_token(&self, kind: &str) -> bool {
        self.tokens().any(|token| token.kind() == kind)
    }
    /// Whether the node has a token of any of `kinds`.
    fn has_any_token(&self, kinds: &[&str]) -> bool {
        self.tokens().any(|token| kinds.contains(&token.kind().as_ref()))
    }
    /// How many of the node's tokens are of kind `kind` (commas, say).
    fn count_tokens(&self, kind: &str) -> usize {
        self.tokens().filter(|token| token.kind() == kind).count()
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
        self.children().map(|node| if node.is_named() { Part::Named(node) } else { Part::Token(Token { node }) })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use std::sync::Arc;

    /// Every query answers the same on tree-sitter's tree and on a restored named-only tape.
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
            assert!(image.primary.is_named_only(), "{lang:?}");
            let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
            let (native, tape) = (AstGrep::doc(native), AstGrep::doc(tape));
            assert_eq!(shape(tape.root()), shape(native.root()), "{lang:?}");
        }
    }

    fn shape<D: Doc>(root: Node<'_, D>) -> Vec<String> {
                root.dfs()
                    .map(|node| {
                        let tokens: Vec<_> = node.tokens().map(|t| format!("{}@{:?}", t.kind(), t.range())).collect();
                        let named: Vec<_> = node.named().map(|n| n.range()).collect();
                        let code: Vec<_> = node.named_code().map(|n| n.range()).collect();
                        let parts: Vec<_> = node.parts().map(|p| format!("{}{}@{:?}", if p.named().is_some() { "n:" } else { "t:" }, p.kind(), p.range())).collect();
                        let commas: Vec<_> = node.named().map(|child| node.count_tokens_before(&child, ",")).collect();
                        format!(
                            "{} {:?} {tokens:?} {named:?} {code:?} {parts:?} {commas:?} {:?} {:?} {:?} {}",
                            node.kind(),
                            node.range(),
                            node.operator(),
                            node.token_before().map(|t| t.kind().into_owned()),
                            node.token_after().map(|t| t.kind().into_owned()),
                            node.count_tokens(","),
                        )
                    })
                    .collect()
    }
}
