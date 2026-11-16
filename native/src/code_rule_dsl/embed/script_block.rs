//! `script-block`: a Vue single-file component's scripts and template expressions, read as the
//! one TypeScript, TSX or JavaScript program its `<script>` blocks' `lang` names (`vue::view`).

use super::{Embedder, Program};
use super::host::HostNode;
use crate::code_rule_dsl::vue::view::View;

/// The embedder of `Via::ScriptBlock`.
pub struct ScriptBlock;

impl Embedder for ScriptBlock {
    fn build(&self, root: HostNode<'_>, source: &str, budget: &dyn Fn() -> bool) -> Option<Program> {
        View::build(root, source, budget).map(View::into_program)
    }
}
