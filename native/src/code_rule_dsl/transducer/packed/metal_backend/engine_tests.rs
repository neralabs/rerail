//! Tests of this module that read what the engine registers as it loads (the languages): the engine's facade of
//! the module mounts them.
use super::launch::*;
use crate::code_rule_dsl::transducer::packed::Error;

#[test]
fn production_entry_preserves_the_outer_budget_and_returns_launch_limit() {
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::code_rule_dsl::transducer::packed::{Program, metal_program_backend};
    use ast_grep_core::tree_sitter::StrDoc;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;
    use std::sync::Arc;

    let _evaluation = LaunchScope::enter();
    for _ in 0..KERNEL_LAUNCH_LIMIT {
        assert!(reserve());
    }
    assert!(!reserve());
    let source = "name\n";
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let native = StrDoc::new(source, GrammarDialect::Python);
    let image =
        TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    let doc =
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
    let anchors = vec![vec![(0..doc.transducer_len() as u32).collect()]];
    let program = Program::compile_tree_sitter_query_pattern(
        "(identifier) @name",
        &GrammarDialect::Python.get_ts_language(),
    )
    .unwrap();
    let result = metal_program_backend::execute_packed_with_result_limits(
        &metal_program_backend::compile_interpreter().unwrap(),
        &[program],
        &[doc],
        &anchors,
        None,
    );
    assert!(
        matches!(result, Err(Error::BackendLaunchLimit)),
        "production must preserve the evaluation budget"
    );
    assert_eq!(check_submission(), Err(Error::BackendLaunchLimit));
}
