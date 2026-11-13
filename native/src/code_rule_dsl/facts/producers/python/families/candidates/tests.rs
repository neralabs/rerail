use super::*;

#[test]
fn core_eligibility_equals_reference_on_known_inapplicable_and_broken_expressions() {
    use crate::code_rule_dsl::{
        facts::{core, producers::python::python_facts},
        language::{GrammarDialect, tape_tag},
        lines::{LineTable, Terminators},
        tape::{Grammar as TapeGrammar, TapeDoc, TapeImage},
    };
    use ast_grep_core::{AstGrep, tree_sitter::StrDoc};
    use std::sync::Arc;
    for source in [
        "factory().open()\nhandlers['open']()\nPath('f').open()\n(real.name)()\n",
        "broken = (\n",
        "async def f():\n    (thing.).open()\n",
        "async def f():\n    pkg.ｍｅｔｈｏｄ()\n    ((pkg.name))()\n",
    ] {
        let parsed = StrDoc::new(source, GrammarDialect::Python);
        let grammar = TapeGrammar::new(GrammarDialect::Python).unwrap();
        let image =
            TapeImage::encode(parsed.tree.root_node(), &grammar, source, &|| false).unwrap();
        let root = AstGrep::doc(
            TapeDoc::restore(Arc::new(source.into()), grammar, &image, &|| false).unwrap(),
        );
        let facts = core::from_tape(
            root.root().get_doc(),
            Arc::new(LineTable::new(
                source,
                Terminators::of(GrammarDialect::Python),
            )),
        )
        .unwrap();
        let g = Grammar::new(
            tape_tag(GrammarDialect::Python).unwrap(),
            &GrammarDialect::Python.kinds(),
        );
        // Every expression/node, including malformed ancestors, checks conservative handling.
        for node in root.root().dfs() {
            assert_eq!(
                eligibility(&g, &facts, node.node_id() as u32),
                python_facts::qualification_chain(&node),
                "{source}: {}",
                node.text()
            );
        }
    }
}
