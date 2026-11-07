//! `compiled-view`: Blade's template, compiled to the PHP program its view runs (`blade::view`).
//! Blade's host tree is Forte's (`blade::forte`), not tree-sitter's, so its view is built where a
//! template's tape is (`tape::TapeImage::encode_blade`, `View::build`), with the template's syntax
//! errors as Blade reads them; a tree-sitter tree never holds a compiled view.

use super::{Embedder, Program};
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::scope::{BuildError, Builder, Reading};
use ast_grep_core::{Doc, Node};
use tree_sitter::Node as TsNode;

/// The embedder of `Via::CompiledView`.
pub(crate) struct CompiledView;

impl Embedder for CompiledView {
    /// No tree-sitter host compiles to a view: Blade's is built from its Forte tree.
    fn build(_: TsNode<'_>, _: &str) -> Option<Program> {
        None
    }

    /// The view's PHP, whose markers stand for the variables Blade's directives define and whose
    /// top level is the scope of the view's data (`scope::php::visit_blade_program`).
    fn scope<D: Doc>(
        _: Family,
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        _: Reading,
    ) -> Result<(), BuildError> {
        crate::code_rule_dsl::scope::php::visit_blade_program(builder, root)
    }
}
