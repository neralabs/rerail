//! A call's arguments (`arguments`): `Semantic::Arguments`.

use super::{Ecma, Php, Python, Refusal, Refused};
use crate::code_rule_dsl::matcher::{Arguments, Passed};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;

/// An option of `arguments` a family may not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ArgumentFeature {
    /// `/` and `*` in `signature`: Python's positional-only and keyword-only slots.
    SignatureMarkers,
    /// `mode: "syntactic"`: each slot found as Ruff's `find_argument` finds it.
    Syntactic,
    /// `mode: "estree"`: each slot is ESLint's `node.arguments[i]`.
    Estree,
}

/// A family's call arguments.
pub(crate) trait ArgumentRules {
    /// Whether the family reads call arguments: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
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
fn markers(detail: &str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Owned(format!(
        "`/` and `*` mark Python's positional-only and keyword-only slots; {detail}"
    ))))
}

fn refused(message: &'static str) -> Result<(), Refusal> {
    Err(Refusal::Refused(Cow::Borrowed(message)))
}

/// Positional-only and keyword-only parameters; keywords; Ruff's syntactic reading.
impl ArgumentRules for Python {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &["call"];

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers | ArgumentFeature::Syntactic => Ok(()),
            ArgumentFeature::Estree => refused(
                "`estree` reads ESLint's `node.arguments`; use language or syntactic for Python",
            ),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        Some(if arguments.syntactic {
            arguments.syntactic(call)
        } else {
            arguments.python(call)
        })
    }
}

/// Arguments by position only; ESLint's `node.arguments`.
impl ArgumentRules for Ecma {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &["call_expression", "new_expression"];

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers => {
                markers("JavaScript passes arguments by position only")
            }
            ArgumentFeature::Syntactic => refused(
                "`syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords",
            ),
            ArgumentFeature::Estree => Ok(()),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        Some(arguments.typescript(call))
    }
}

/// Named arguments that bind any parameter; Ruff's syntactic reading.
impl ArgumentRules for Php {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &[
        "function_call_expression",
        "member_call_expression",
        "nullsafe_member_call_expression",
        "scoped_call_expression",
        "object_creation_expression",
    ];

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers => {
                markers("PHP has no positional-only or keyword-only parameters")
            }
            ArgumentFeature::Syntactic => Ok(()),
            ArgumentFeature::Estree => refused(
                "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)",
            ),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        if arguments.syntactic {
            arguments.php_syntactic(call)
        } else {
            arguments.php(call)
        }
    }
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
pub(super) mod kit {
    use super::super::syntax::kit::refusal;
    use super::{ArgumentFeature, ArgumentRules};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::language::adapter::Adapter;
    use crate::code_rule_dsl::matcher::{Arguments, Passed, SlotKind};
    use crate::code_rule_dsl::tape::TapeDoc;
    use ast_grep_core::AstGrep;
    use std::collections::BTreeSet;

    /// What a family's `ArgumentRules` answer.
    pub(in super::super) struct Expect {
        pub(in super::super) call_kinds: &'static [&'static str],
        /// The refusals of `/` and `*`, `syntactic` and `estree`; `None` where read.
        pub(in super::super) features: [Option<&'static str>; 3],
        /// A document holding a call, and what the call passes for the slots `a` and `b`, read
        /// as the language binds them: an argument's text, `-` absent, `?` unknown.
        pub(in super::super) passed: (&'static str, [&'static str; 2]),
    }

    pub(in super::super) fn check<A: Adapter>(
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
