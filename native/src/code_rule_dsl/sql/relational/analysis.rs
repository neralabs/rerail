//! Admission proofs for unordered set evaluation. More expressive constructs fail closed.
use crate::code_rule_dsl::matcher::{CaptureAnalysis, GrammarMatcher};

pub(super) fn admit(grammar: &GrammarMatcher) -> Result<(), String> {
    if grammar.three_valued {
        return Err(
            "relational compiler: unknown outcomes need an explicit outcome relation".into(),
        );
    }
    if grammar.transform.is_some() {
        return Err("relational compiler: transformed captures are not admitted".into());
    }
    let mut captures = CaptureAnalysis::default();
    if !captures.mentions(&grammar.matcher).is_empty()
        || !captures.counts(&grammar.matcher).is_empty()
    {
        return Err(
            "relational compiler: capture-bearing rules need ordered witness relations".into(),
        );
    }
    // Lowering restricts leaves to kinds, regexes, and capture-free patterns, with explicit walks.
    // That whitelist excludes semantic effects, query context, and fallible leaves.
    Ok(())
}
