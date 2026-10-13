//! `compiled-view`: Blade's template, compiled to the PHP program its view runs (`blade::view`).
//! Blade's host tree is Forte's (`blade::forte`), not tree-sitter's, so its view is built where a
//! template's tape is (`tape::TapeImage::encode_blade`, `View::build`), with the template's syntax
//! errors; a tree-sitter tree never holds a compiled view.

use super::{Embedder, Program};
use crate::code_rule_dsl::{blade::view::View, language::GrammarDialect};
use std::ops::Range;
use tree_sitter::Node as TsNode;

/// The embedder of `Via::CompiledView`.
pub(crate) struct CompiledView;

impl Embedder for CompiledView {
    type Program = View;

    /// No tree-sitter host compiles to a view: Blade's is built from its Forte tree.
    fn build(_: TsNode<'_>, _: &str) -> Option<Self::Program> {
        None
    }
}

impl Program for View {
    fn dialect(&self) -> GrammarDialect {
        GrammarDialect::Php
    }

    fn root(&self) -> TsNode<'_> {
        self.root()
    }

    fn span(&self, node: TsNode<'_>) -> Range<usize> {
        self.span(node)
    }

    fn omitted(&self, node: TsNode<'_>) -> bool {
        self.omitted(node)
    }
}
