//! A bounded packed terminal: Smart `$X`, with a nullable or named-leaf input capture.
//! It reads the current TapeDoc. Composite inputs are rejected at preparation, not treated as
//! nonmatches. Returned captures are candidate occurrences, never the input occurrence.

use super::measured;
use crate::code_rule_dsl::sql::tests::differential::tape;
use crate::code_rule_dsl::{language::GrammarDialect, matcher::node_at, tape::TapeDoc};
use ast_grep_core::{
    AstGrep, Pattern,
    matcher::{Matcher, PatternNode},
    meta_var::{MetaVarEnv, MetaVariable},
};
use std::borrow::Cow;

pub(super) struct PackedCapture;
impl PackedCapture {
    fn compile(pattern: &Pattern) -> Result<Self, &'static str> {
        if matches!(pattern.strictness, ast_grep_core::MatchStrictness::Smart)
            && matches!(&pattern.node, PatternNode::MetaVar { meta_var: MetaVariable::Capture(name, true) } if name == "X")
        {
            Ok(Self)
        } else {
            Err("unsupported packed pattern")
        }
    }

    pub(super) fn run(
        &self,
        doc: &TapeDoc,
        candidate: u32,
        bound_text: Option<&str>,
    ) -> Option<u32> {
        let flags = doc.transducer_flags(candidate);
        if flags & 1 == 0 {
            return None;
        }
        if bound_text.is_some_and(|text| text != doc.transducer_text_str(candidate)) {
            return None;
        }
        Some(candidate)
    }
}

pub(super) fn native(
    root: &AstGrep<TapeDoc>,
    pattern: &Pattern,
    candidate: u32,
    bound: Option<u32>,
) -> Option<u32> {
    let mut env = MetaVarEnv::new();
    if let Some(bound) = bound {
        env.insert("X", node_at(root, bound))?;
    }
    let mut env = Cow::Owned(env);
    pattern.match_node_with_env(node_at(root, candidate), &mut env)?;
    Some(env.get_match("X")?.node_id() as u32)
}

#[test]
fn packed_capture_agrees_on_every_node_and_bound_leaf() {
    for (dialect, source) in [
        (
            GrammarDialect::Python,
            "# a\nf(a, a)\nx = 'a'\nif a: f(a)\nf(\n",
        ),
        (
            GrammarDialect::TypeScript,
            "// a\nf(a, a); ({a}); let x = 'a'; f(;\n",
        ),
    ] {
        let doc = tape(dialect, source);
        let root = AstGrep::doc(doc.clone());
        let pattern = Pattern::try_new("$X", dialect).unwrap();
        let program = PackedCapture::compile(&pattern).unwrap();
        let bounds = std::iter::once(None).chain(
            (0..doc.transducer_len() as u32)
                .filter(|&id| node_at(&root, id).is_named_leaf())
                .map(Some),
        );
        for bound in bounds {
            for candidate in 0..doc.transducer_len() as u32 {
                let bound_text = bound.map(|id| doc.transducer_text_str(id));
                assert_eq!(
                    program.run(&doc, candidate, bound_text),
                    native(&root, &pattern, candidate, bound),
                    "{bound:?} -> {candidate}"
                );
            }
        }
        assert!(PackedCapture::compile(&Pattern::try_new("f($X)", dialect).unwrap()).is_err());
    }
}

#[test]
#[ignore = "representation measurement; run with --test-threads=1 --nocapture"]
fn measure_packed_capture_terminal() {
    let doc = tape(GrammarDialect::Python, &"f(a, b, a)\n".repeat(10_000));
    let root = AstGrep::doc(doc.clone());
    let pattern = Pattern::try_new("$X", GrammarDialect::Python).unwrap();
    let program = PackedCapture::compile(&pattern).unwrap();
    let bound = (0..doc.transducer_len() as u32)
        .find(|&id| node_at(&root, id).is_named_leaf() && doc.transducer_text_str(id) == "a")
        .unwrap();
    for bound in [None, Some(bound)] {
        let (oracle, native_ms) = measured(|| {
            (0..doc.transducer_len() as u32)
                .map(|candidate| native(&root, &pattern, candidate, bound))
                .collect::<Vec<_>>()
        });
        let text = bound.map(|id| doc.transducer_text_str(id));
        let (actual, packed_ms) = measured(|| {
            (0..doc.transducer_len() as u32)
                .map(|candidate| program.run(&doc, candidate, text))
                .collect::<Vec<_>>()
        });
        assert_eq!(oracle, actual);
        eprintln!(
            "SPIKE {}",
            serde_json::json!({"name":"packed-single-capture","bound":bound.is_some(),
            "candidates":actual.len(),"matches":actual.iter().flatten().count(),"nativeMs":native_ms,"packedMs":packed_ms,"completeCaptureParity":true})
        );
    }
}
