//! Reading a syntax tree's shape: named children, and the anonymous tokens (punctuation,
//! keywords, operators) between them (RER-131).
//!
//! Readers of trees use these instead of filtering `children()` by `is_named()` themselves, or
//! comparing an anonymous child's `kind()`. Every function here reads the tree's children. A tape
//! holds the named nodes and the tokens the grammar does not imply (`tape::tokens`). An implied
//! token is in no tape: the token queries answer it from the parent's gaps
//! ([`TokenGrammar::gaps`](crate::code_rule_dsl::tape::tokens::TokenGrammar::gaps)), by kind,
//! from the stored children and the grammar alone (t31). It is no node, and its span is exact
//! only at the parent's edges (a node starts at its first token and ends at its last); between
//! two stored children it is the gap's span.
//!
//! - **Named children:** [`Tree::named`] (all), [`Tree::named_code`] (not comments),
//!   [`Tree::first_named`], [`Tree::last_named`], [`Tree::first_code`].
//! - **Tokens:** [`Tree::tokens`], [`Tree::has_token`], [`Tree::has_any_token`],
//!   [`Tree::first_token`], [`Tree::operator`], [`Tree::operator_token`].
//! - **Tokens beside a node:** [`Tree::token_before`], [`Tree::token_after`].
//! - **Layout:** [`Tree::parts`], named nodes and tokens in source order ([`Part`]).
//!
//! A [`Token`] is a value: its kind, text and span. Code that edits a token edits its span.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::gap::stored_gaps;
use crate::code_rule_dsl::tape::tokens::TokenGrammar;
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
    held: Held<'r, D>,
}

#[derive(Clone)]
enum Held<'r, D: Doc> {
    /// A token the tree holds.
    Stored(Node<'r, D>),
    /// A token the grammar implies in a gap: its kind, field and span (see the module).
    Implied { kind: &'static str, field: &'static str, range: Range<usize> },
}

impl<'r, D: Doc> Token<'r, D> {
    /// The token's kind: its text in the grammar (`"+"`, `"async"`, `","`).
    pub(crate) fn kind(&self) -> Cow<'_, str> {
        match &self.held {
            Held::Stored(node) => node.kind(),
            Held::Implied { kind, .. } => Cow::Borrowed(kind),
        }
    }
    /// The token's text in the source (`B"`, `FUNCTION`): may differ from its kind in case or
    /// prefix. An implied token's is its kind.
    pub(crate) fn text(&self) -> Cow<'r, str> {
        match &self.held {
            Held::Stored(node) => node.text(),
            Held::Implied { kind, .. } => Cow::Borrowed(kind),
        }
    }
    /// The token's byte span in the source; an implied token's between two stored children is
    /// the gap's.
    pub(crate) fn range(&self) -> Range<usize> {
        match &self.held {
            Held::Stored(node) => node.range(),
            Held::Implied { range, .. } => range.clone(),
        }
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
    /// The node a part is, if the tree holds it: a named node or a stored token (not an implied
    /// one).
    pub(crate) fn node(&self) -> Option<Node<'r, D>> {
        match self {
            Self::Named(node) => Some(node.clone()),
            Self::Token(Token { held: Held::Stored(node) }) => Some(node.clone()),
            Self::Token(_) => None,
        }
    }
    /// The token, if the part is one.
    fn into_token(self) -> Option<Token<'r, D>> {
        match self {
            Self::Named(_) => None,
            Self::Token(token) => Some(token),
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
        parts(self).into_iter().filter_map(|part| match part {
            Part::Token(token) => Some(token),
            Part::Named(_) => None,
        })
    }
    fn parts(&self) -> impl Iterator<Item = Part<'r, D>> + '_ {
        parts(self).into_iter()
    }
    fn operator_token(&self) -> Option<Token<'r, D>> {
        match self.field("operator") {
            Some(node) => Some(Token { held: Held::Stored(node) }),
            None => self.tokens().find(|token| matches!(token.held, Held::Implied { field: "operator", .. })),
        }
    }
    fn token_before(&self) -> Option<Token<'r, D>> {
        let parts = parts(&self.parent()?);
        let at = parts.iter().position(|part| part.named().is_some_and(|node| node.node_id() == self.node_id()))?;
        parts.into_iter().nth(at.checked_sub(1)?).and_then(|part| part.into_token())
    }
    fn token_after(&self) -> Option<Token<'r, D>> {
        let parts = parts(&self.parent()?);
        let at = parts.iter().position(|part| part.named().is_some_and(|node| node.node_id() == self.node_id()))?;
        parts.into_iter().nth(at + 1).and_then(|part| part.into_token())
    }
}

/// A tape's token grammar, where `node` is a tape's (or a tree-sitter tree's, whose gaps hold
/// their tokens): its dialect's automata.
fn token_grammar<D: Doc>(node: &Node<'_, D>) -> Option<std::sync::Arc<TokenGrammar>> {
    let lang: &dyn std::any::Any = node.lang();
    TokenGrammar::of_dialect(*lang.downcast_ref::<GrammarDialect>()?)
}

/// `node`'s children in source order, with the tokens its gaps imply between them.
fn parts<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Part<'r, D>> {
    let stored = |child: Node<'r, D>| if child.is_named() { Part::Named(child) } else { Part::Token(Token { held: Held::Stored(child) }) };
    let gapped = token_grammar(node).and_then(|grammar| {
        let (children, gaps) = stored_gaps(&grammar, node)?;
        let range = node.range();
        let mut parts = Vec::with_capacity(children.len() * 2);
        // The end of the last stored child that is not an extra: where a gap's span starts.
        let (mut low, mut first) = (range.start, true);
        let word = |parts: &mut Vec<Part<'r, D>>, word: &[(&str, &str)], low: usize, high: usize, edge_start: bool, edge_end: bool| {
            for (at, (kind, field)) in word.iter().enumerate() {
                let (kind, field) = (grammar.intern(kind), grammar.intern(field));
                let range = if edge_start && at == 0 {
                    low..(low + kind.len()).min(high)
                } else if edge_end && at + 1 == word.len() {
                    high.saturating_sub(kind.len()).max(low)..high
                } else {
                    low..high
                };
                parts.push(Part::Token(Token { held: Held::Implied { kind, field, range } }));
            }
        };
        // Extras (comments) between two stored children: the gap's word is put before them, as
        // code is usually laid out (`):  # c`); its span is the whole gap either way.
        let mut extras = Vec::new();
        for (child, before) in children.into_iter().zip(&gaps.before) {
            if grammar.is_extra(&child.kind()) {
                extras.push(child);
                first = false;
                continue;
            }
            word(&mut parts, before, low, child.range().start, first, false);
            first = false;
            parts.extend(extras.drain(..).map(stored));
            low = child.range().end;
            parts.push(stored(child));
        }
        word(&mut parts, &gaps.end, low, range.end, first && extras.is_empty(), true);
        parts.extend(extras.drain(..).map(stored));
        Some(parts)
    });
    gapped.unwrap_or_else(|| node.children().map(stored).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use std::sync::Arc;

    /// The named queries answer as on tree-sitter's tree, and so do the token queries: a tape's
    /// parts are tree-sitter's by kind, the implied tokens answered from the gaps, each token's
    /// span holding tree-sitter's, and exact at the parent's edges (t31).
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
            let (native, tape) = (AstGrep::doc(native), AstGrep::doc(tape));
            assert_eq!(named(tape.root()), named(native.root()), "{lang:?}");
            let (gapped, all) = (parts(tape.root()), parts(native.root()));
            assert_eq!(gapped.len(), all.len(), "{lang:?}");
            for ((parent, tape_parts), (_, native_parts)) in gapped.iter().zip(&all) {
                // Extras aside: a gap's word is put before the comments in it (see `parts`).
                let code = |parts: &[(String, std::ops::Range<usize>, bool)]| parts.iter().filter(|part| !crate::code_rule_dsl::tree::is_comment(&part.0)).cloned().collect::<Vec<_>>();
                let (tape_parts, native_parts) = (&code(tape_parts), &code(native_parts));
                let kinds = |parts: &[(String, std::ops::Range<usize>, bool)]| parts.iter().map(|part| part.0.clone()).collect::<Vec<_>>();
                assert_eq!(kinds(tape_parts), kinds(native_parts), "{lang:?} {parent}");
                for (tape_part, native_part) in tape_parts.iter().zip(native_parts) {
                    let holds = tape_part.1.start <= native_part.1.start && native_part.1.end <= tape_part.1.end;
                    assert!(holds, "{lang:?} {parent}: {tape_part:?} {native_part:?}");
                }
                let (first, last) = (tape_parts.first(), tape_parts.last());
                for (at, part) in [(0, first), (tape_parts.len().saturating_sub(1), last)] {
                    if let Some(part) = part.filter(|part| part.2) {
                        assert_eq!(part.1, native_parts[at].1, "{lang:?} {parent}: an edge token's span");
                    }
                }
            }
            let stored = tape.root().dfs().filter(|node| !node.is_named()).count();
            let answered = gapped.iter().flat_map(|(_, parts)| parts).filter(|part| part.2).count();
            assert!(stored < answered, "{lang:?}: the tape leaves tokens out, and the gaps answer them");
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
