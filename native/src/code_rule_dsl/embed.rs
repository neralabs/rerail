//! Embedded programs: the code a file of one dialect holds in another's grammar. Blade's template
//! compiles to PHP (`compiled-view`); a Vue single-file component holds TypeScript or JavaScript
//! (`script-block`).
//!
//! The registry declares an embedding (`embeds: [{ language, via }]`); `build.rs` generates the
//! `Via` of each supported one and `GrammarDialect::embeds` (the embedded dialects per host dialect).
//! Each `Via`'s embedder is registered for it as the engine loads (`install_embedders`; Vue's is
//! `embed/script_block.rs`, `ScriptBlock`); a `Via` none registered for panics when a file reaches it.
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

pub(crate) mod compiled_view;
mod script_block;
pub(crate) mod segments;
#[cfg(test)]
pub(crate) mod test_script;
#[cfg(test)]
mod tests;

pub(crate) use compiled_view::CompiledView;
pub(crate) use script_block::ScriptBlock;
#[cfg(test)]
pub(crate) use test_script::TestScript;

use crate::code_rule_dsl::language::GrammarDialect;

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
/// The crate that owns an embedding registers its embedder for its `Via` as the engine loads (`install_embedders`).
/// A program it built is scoped as its dialect's family reads any program, unless the embedding gives the
/// program's statements a meaning of its own, which its crate registers too (`scope::install_program_scopes`: a
/// Blade view's markers define the variables its directives do).
pub(crate) trait Embedder: Send + Sync {
    /// The program the host tree rooted at `root`, over `source`, embeds; `None` where it embeds
    /// none.
    fn build(&self, root: TsNode<'_>, source: &str) -> Option<Program>;
}

static EMBEDDERS: std::sync::OnceLock<Vec<(crate::code_rule_dsl::language::Via, &'static dyn Embedder)>> =
    std::sync::OnceLock::new();

/// The engine's embedders, by `Via`; the first call holds.
pub(crate) fn install_embedders(embedders: Vec<(crate::code_rule_dsl::language::Via, &'static dyn Embedder)>) {
    let _ = EMBEDDERS.set(embedders);
}

/// `via`'s embedder. Every `Via` the build has is registered as the engine loads.
pub(crate) fn embedder(via: crate::code_rule_dsl::language::Via) -> &'static dyn Embedder {
    EMBEDDERS
        .get()
        .and_then(|embedders| embedders.iter().find(|(known, _)| *known == via))
        .map(|(_, embedder)| *embedder)
        .unwrap_or_else(|| panic!("no embedder registered for {via:?}"))
}

