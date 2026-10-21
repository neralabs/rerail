//! `script-block`: a Vue single-file component's scripts and template expressions, read as the
//! one TypeScript, TSX or JavaScript program its `<script>` blocks' `lang` names (`vue::view`).

use super::{Embedder, Program};
use crate::code_rule_dsl::vue::view::View;
use tree_sitter::Node as TsNode;

/// The embedder of `Via::ScriptBlock`.
pub(crate) struct ScriptBlock;

impl Embedder for ScriptBlock {
    fn build(root: TsNode<'_>, source: &str) -> Option<Program> {
        View::build(root, source).map(View::into_program)
    }
}
