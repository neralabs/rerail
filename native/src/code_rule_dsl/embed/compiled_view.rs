//! `compiled-view`: Blade's template, compiled to the PHP program its view runs (`blade::view`).

use super::{Embedder, Program};
use crate::code_rule_dsl::{blade::view::View, language::GrammarDialect};
use std::ops::Range;
use tree_sitter::Node as TsNode;

/// The embedder of `Via::CompiledView`.
pub(crate) struct CompiledView;

impl Embedder for CompiledView {
    type Program = View;

    fn build(root: TsNode<'_>, source: &str) -> Option<Self::Program> {
        View::build(root, source)
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
