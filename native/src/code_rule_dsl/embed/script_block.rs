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

    /// The view is built from what the component's recovered tree holds, as vue-eslint-parser
    /// recovers from a template's syntax errors and ESLint reads the scripts: a template's error
    /// leaves a clause over a clean program complete.
    const APART: bool = true;

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
