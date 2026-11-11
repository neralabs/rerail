//! A call's arguments (`arguments`): `Semantic::Arguments`.

use super::{Refusal, Refused};
use crate::code_rule_dsl::matcher::{Arguments, Passed};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
// cratify: `lang_python` is rerail-lang-python's (it mounts it)
// cratify: `lang_ecma` is rerail-lang-ecma's (it mounts it)
// cratify: `lang_php` is rerail-lang-php's (it mounts it)

/// An option of `arguments` a family may not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArgumentFeature {
    /// `/` and `*` in `signature`: Python's positional-only and keyword-only slots.
    SignatureMarkers,
    /// `mode: "syntactic"`: each slot found as Ruff's `find_argument` finds it.
    Syntactic,
    /// `mode: "estree"`: each slot is ESLint's `node.arguments[i]`.
    Estree,
}

/// A family's call arguments.
pub trait ArgumentRules {
    /// Whether the family reads call arguments: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    const SUPPORTED: bool;
    /// The kinds of the family's calls.
    const CALL_KINDS: &'static [&'static str];
    /// Whether the family reads `feature`, or why not.
    fn feature(feature: ArgumentFeature) -> Result<(), Refusal>;
    /// What `call`, of one of `CALL_KINDS`, passes for each of `arguments`' slots, by slot index;
    /// `None` where it is no call.
    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>>;
}

/// The refusal of `/` and `*` by a family whose parameters have no such kinds (`detail`).
pub fn markers(detail: &str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Owned(format!(
        "`/` and `*` mark Python's positional-only and keyword-only slots; {detail}"
    ))))
}

pub fn refused(message: &'static str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Borrowed(message)))
}

/// No calls: the capability's refusal (`unsupported_semantic`) precedes any feature's.
impl ArgumentRules for Refused {
    const SUPPORTED: bool = false;
    const CALL_KINDS: &'static [&'static str] = &[];

    fn feature(_: ArgumentFeature) -> Result<(), Refusal> {
        Ok(())
    }

    fn passed<'r, D: Doc>(_: &Arguments, _: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        None
    }
}

/// The kit's check of a family's `ArgumentRules`.
#[cfg(test)]
pub mod kit {
    use super::super::syntax::kit::refusal;
    use super::{ArgumentFeature, ArgumentRules};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::language::adapter::Adapter;
    use crate::code_rule_dsl::matcher::{Arguments, Passed, SlotKind};
    use crate::code_rule_dsl::tape::TapeDoc;
    use ast_grep_core::AstGrep;
    use std::collections::BTreeSet;

    /// What a family's `ArgumentRules` answer.
    pub struct Expect {
        pub call_kinds: &'static [&'static str],
        /// The refusals of `/` and `*`, `syntactic` and `estree`; `None` where read.
        pub features: [Option<&'static str>; 3],
        /// A document holding a call, and what the call passes for the slots `a` and `b`, read
        /// as the language binds them: an argument's text, `-` absent, `?` unknown.
        pub passed: (&'static str, [&'static str; 2]),
    }

    pub fn check<A: Adapter>(
        expect: &Expect,
        dialect: GrammarDialect,
        tape: impl Fn(&str) -> AstGrep<TapeDoc>,
    ) {
        assert_eq!(
            <A::Arguments as ArgumentRules>::CALL_KINDS,
            expect.call_kinds
        );
        let features = [
            ArgumentFeature::SignatureMarkers,
            ArgumentFeature::Syntactic,
            ArgumentFeature::Estree,
        ]
        .map(|feature| refusal(<A::Arguments as ArgumentRules>::feature(feature)));
        assert_eq!(features, expect.features.map(|m| m.map(str::to_owned)));
        let (source, slots) = expect.passed;
        let doc = tape(source);
        let call = doc
            .root()
            .dfs()
            .find(|node| expect.call_kinds.contains(&&*node.kind()))
            .expect("a call");
        let arguments = Arguments {
            language: dialect,
            slots: vec![
                ("a".to_owned(), SlotKind::Either),
                ("b".to_owned(), SlotKind::Either),
            ],
            bind: Vec::new(),
            absent: Vec::new(),
            call_kinds: BTreeSet::new(),
            syntactic: false,
            estree: false,
        };
        let passed = <A::Arguments as ArgumentRules>::passed(&arguments, &call)
            .expect("a call")
            .iter()
            .map(|passed| match passed {
                Passed::Absent => "-".to_owned(),
                Passed::Unknown => "?".to_owned(),
                Passed::Value(node) => node.text().into_owned(),
            })
            .collect::<Vec<_>>();
        assert_eq!(passed, slots, "{source:?}");
    }
}
