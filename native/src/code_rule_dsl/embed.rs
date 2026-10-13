//! Embedded programs: the code a file of one dialect holds in another's grammar, which its tape
//! carries beside the file's own tree. Blade's template compiles to PHP (`compiled-view`); a Vue
//! single-file component holds TypeScript or JavaScript (`script-block`).
//!
//! The registry declares an embedding (`embeds: [{ language, via }]`); `build.rs` generates the
//! `Via` of each supported one, `GrammarDialect::embeds` (the embedded dialects per host dialect)
//! and `with_embedder!`, which dispatches a `Via` to the type of the same name here. A new `via`
//! is one module and one re-export below (Vue's is `embed/script_block.rs`, `ScriptBlock`); the
//! build fails until it exists.
//!
//! An embedder provides, from a file's tree and source, at most one program per file:
//! - its tree, parsed with the grammar of `Program::dialect`, which must be one of the host
//!   dialect's `GrammarDialect::embeds` for that via (a Vue file's scripts are TypeScript, TSX or
//!   JavaScript, as its `lang` says);
//! - where each of its nodes stands in the host source (`Program::span`), which `SegmentMap`
//!   records as the program is written: copied stretches byte for byte, written ones over the host
//!   bytes they stand for;
//! - which of its nodes are placeholders the program needs and the tape leaves out
//!   (`Program::omitted`).
//!
//! The tape appends the program's root as the host root's last child (`tape::TapeImage::encode`),
//! its kinds and fields numbered after the host grammar's (`language::KindCatalog`), and the
//! family's adapter reads it as a region of that dialect (`adapter::Region`).

mod compiled_view;
mod script_block;
pub(crate) mod segments;

pub(crate) use compiled_view::CompiledView;
pub(crate) use script_block::ScriptBlock;

use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::{Doc, Node};
use std::ops::Range;
use tree_sitter::Node as TsNode;

/// A program an embedder built from a host file.
pub(crate) trait Program {
    /// The dialect its tree is parsed with and read as.
    fn dialect(&self) -> GrammarDialect;
    /// Its tree's root.
    fn root(&self) -> TsNode<'_>;
    /// The host source bytes a node of its tree stands for.
    fn span(&self, node: TsNode<'_>) -> Range<usize>;
    /// Whether a node of its tree is a placeholder the tape leaves out.
    fn omitted(&self, node: TsNode<'_>) -> bool;
}

/// Builds a host file's embedded program: one per registry `via`.
pub(crate) trait Embedder {
    type Program: Program;

    /// The program the host tree rooted at `root`, over `source`, embeds; `None` where it embeds
    /// none.
    fn build(root: TsNode<'_>, source: &str) -> Option<Self::Program>;
}

/// Whether a view wrote a token of `node`, an embedded program's, that the host does not spell
/// (Blade's `@unless(c)` `!(`, a `:` after a directive; the parentheses and `;` Vue's view wraps
/// an interpolation in): the node is then not the code its host bytes are, and what reads its
/// tokens (structure, names, references) does not know it. Every token a view copies, and every
/// one it writes over the same text (Blade's `@isset(` `isset(`), spells its own kind.
pub(crate) fn rewritten<D: Doc>(node: &Node<'_, D>) -> bool {
    node.dfs().any(|token| {
        !token.is_named() && token.is_leaf() && !token.text().eq_ignore_ascii_case(&token.kind())
    })
}
