//! Embedded programs: the code a file of one dialect holds in another's grammar. Blade's template
//! compiles to PHP (`compiled-view`); a Vue single-file component holds TypeScript or JavaScript
//! (`script-block`).
//!
//! The registry declares an embedding (`embeds: [{ language, via }]`); `build.rs` generates the
//! `Via` of each supported one, `GrammarDialect::embeds` (the embedded dialects per host dialect)
//! and `with_embedder!`, which dispatches a `Via` to the type of the same name here. A new `via`
//! is one module and one re-export below (Vue's is `embed/script_block.rs`, `ScriptBlock`); the
//! build fails until it exists.
//!
//! A file that embeds code is several documents: the **host**, the file's own tree, and one
//! **guest** per `via` its dialect embeds that the file holds a program for. Every document holds
//! one grammar and passes the same tape checks. An embedder builds, from a file's tree and source,
//! at most one program, which is
//! - its text, written as the host's language reads the file (a Blade view's PHP, a Vue
//!   component's scripts and template expressions), and its tree, parsed from that text with the
//!   grammar of `Program::dialect`, one of the dialects the host embeds through that `via` (a Vue
//!   file's scripts are TypeScript, TSX or JavaScript, as its `lang` says). Its nodes span the
//!   text, as any tree's do;
//! - where its text stands in the host source (`SegmentMap`), recorded as the program is written:
//!   copied stretches byte for byte, written ones over the host bytes they stand for. A guest node
//!   reports, suppresses and is compared with the host through the host bytes it stands for: the
//!   guest is an `origin::Origin`, and `origin::InFile` places its nodes in the file;
//! - the host bytes it cannot be read through (`Program::host_errors`): a Blade view reads the
//!   template's bytes, so the template's syntax errors are its; a Vue program is built from what
//!   the component's recovered tree holds, so the component's errors are not.
//!
//! Each guest is a document derived from the host's (`tape::DerivedImage`): its text, segments,
//! host errors and a tape of its own, which the store keeps as a row of its own beside the host's
//! (`file_documents`). A rule reads a guest through `ast.embedded` (the clause runs on it) or the
//! `embedded` relation (from a host node); a guest node reads the host through `host`.

mod compiled_view;
mod script_block;
pub mod segments;
#[cfg(test)]
pub(crate) mod test_script;
#[cfg(test)]
mod tests;

pub(crate) use compiled_view::CompiledView;
pub(crate) use script_block::ScriptBlock;
#[cfg(test)]
pub(crate) use test_script::TestScript;

use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::scope::{BuildError, Builder, Reading};
use ast_grep_core::{Doc, Node};
use segments::SegmentMap;
use std::ops::Range;
use tree_sitter::Node as TsNode;

/// A program an embedder built from a host file.
pub(crate) struct Program {
    /// The dialect its tree is parsed with and read as.
    pub(crate) dialect: GrammarDialect,
    /// Its tree, which spans `text`.
    pub(crate) tree: tree_sitter::Tree,
    pub(crate) text: String,
    /// Where its text stands in the host source (indexed: `SegmentMap::index`), and its
    /// placeholders the tape leaves out.
    pub(crate) segments: SegmentMap,
    /// The host bytes it cannot be read through, sorted: the host's syntax errors it reads.
    pub(crate) host_errors: Vec<Range<usize>>,
}

impl Program {
    pub(crate) fn root(&self) -> TsNode<'_> {
        self.tree.root_node()
    }
}

/// Builds a host file's embedded program: one per registry `via`.
pub(crate) trait Embedder {
    /// The program the host tree rooted at `root`, over `source`, embeds; `None` where it embeds
    /// none.
    fn build(root: TsNode<'_>, source: &str) -> Option<Program>;

    /// Builds the scope of a program this embedder built, rooted at `root`, whose dialect is of
    /// `family`: as that family reads any program, unless the embedding gives the program's
    /// statements a meaning of its own (a Blade view's markers define the variables its
    /// directives do).
    fn scope<D: Doc>(
        family: Family,
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        reading: Reading,
    ) -> Result<(), BuildError> {
        with_adapter!(family, A => <<A as Adapter>::Scope as ScopeRules>::visit(builder, root, reading))
    }
}

/// Whether an embedder wrote a token of `node`, a guest document's, that the host does not spell
/// (Blade's `@unless(c)` `!(`, a `:` after a directive; the parentheses and `;` Vue's view wraps
/// an interpolation in): the node is then not the code its host bytes are, and what reads its
/// tokens (structure, names, references) does not know it. A token copied, or written over the
/// same text (Blade's `@isset(` `isset(`), spells itself. A host document's nodes spell
/// themselves.
pub(crate) fn rewritten<D: Doc>(node: &Node<'_, D>) -> bool {
    let Some(doc) = crate::code_rule_dsl::tape::tape_doc(node).filter(|doc| doc.is_guest()) else {
        return false;
    };
    let file = crate::code_rule_dsl::origin::InFile::tape(doc);
    node.dfs()
        .any(|token| !token.is_named() && token.is_leaf() && !file.spells(&token))
}
