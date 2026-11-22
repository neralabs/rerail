//! Tests of this module that read what the engine registers as it loads (the languages): the engine's facade of
//! the module mounts them.
use super::launch::*;
use crate::code_rule_dsl::transducer::packed::Error;

#[test]
fn production_entry_preserves_the_outer_budget_and_fails_past_it() {
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use crate::code_rule_dsl::transducer::packed::{Program, metal_program_backend};
    use ast_grep_core::tree_sitter::StrDoc;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;
    use std::sync::Arc;

    let _evaluation = LaunchScope::enter();
    // The outer evaluation's whole budget taken: the production entry's first launch is past it.
    for _ in 0..KERNEL_LAUNCH_LIMIT {
        attempt(1, "the outer evaluation's", [1; 3], [1; 3], false, Vec::new());
    }
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
    let interpreter = metal_program_backend::compile_interpreter().unwrap();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        metal_program_backend::execute_packed_with_result_limits(&interpreter, &[program], &[doc], &anchors, None)
    }));
    // Past the budget is the hard failure (decision 0601), not an error the caller falls back on.
    let payload = failed.err().expect("production must preserve the evaluation budget");
    assert!(payload.downcast::<rerail_accel::exec::OverBudget>().is_ok(), "the launch budget's failure");
}
