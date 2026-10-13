//! `script-block`: a Vue single-file component's scripts and template expressions, read as the
//! one TypeScript, TSX or JavaScript program its `<script>` blocks' `lang` names (`vue::view`).

use super::{Embedder, Program};
use crate::code_rule_dsl::{language::GrammarDialect, vue::view::View};
use std::ops::Range;
use tree_sitter::Node as TsNode;

/// The embedder of `Via::ScriptBlock`.
pub(crate) struct ScriptBlock;

impl Embedder for ScriptBlock {
    type Program = View;

    fn build(root: TsNode<'_>, source: &str) -> Option<Self::Program> {
        View::build(root, source)
    }
}

impl Program for View {
    fn dialect(&self) -> GrammarDialect {
        self.dialect()
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
