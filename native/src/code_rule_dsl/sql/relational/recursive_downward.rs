//! Syntactic proof for capture-free downward utility wrappers.
//! Unlike a functional chain, a neighbor child search can invoke several failed
//! siblings before succeeding. Its execution contract must preserve that order.
use super::{ir::Set, lower::Lowerer, recursive_chain::pure};
use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};

#[derive(Clone, Debug)]
pub(super) struct Downward {
    pub base: Set,
    /// Evaluated on the current wrapper, before entering its child search.
    pub guard: Set,
    pub child_kinds: Option<Vec<u16>>,
}

/// Unsupported syntax leaves the supplied lowerer untouched. The self call is
/// identified by its utility table and index, never its spelling or rule ID.
pub(super) fn try_lower(util: &UtilRef, lower: &mut Lowerer) -> Result<Option<Downward>, String> {
    let identity = (util.table.as_ptr() as usize, util.index);
    util.with_target(|body| {
        let AstMatcher::Any(items) = body else {
            return Ok(None);
        };
        if items.len() != 2 || pure(&items[0]).is_err() {
            return Ok(None);
        }
        let (guards, continuation) = match &items[1] {
            AstMatcher::All(terms) if !terms.is_empty() => {
                (&terms[..terms.len() - 1], terms.last().unwrap())
            }
            continuation => (&[][..], continuation),
        };
        let AstMatcher::Has(related, StopBy::Neighbor) = continuation else {
            return Ok(None);
        };
        let AstMatcher::Util(child) = related.rule() else {
            return Ok(None);
        };
        if (child.table.as_ptr() as usize, child.index) != identity
            || guards.iter().any(|guard| pure(guard).is_err())
        {
            return Ok(None);
        }
        // Every proof above precedes registration of terminals or relations.
        Ok(Some(Downward {
            child_kinds: None,
            base: lower.set(&items[0])?,
            guard: Set::Intersect(
                guards
                    .iter()
                    .map(|guard| lower.set(guard))
                    .collect::<Result<_, _>>()?,
            ),
        }))
    })
    .unwrap_or(Ok(None))
}

impl Downward {
    /// Necessary kinds of the current wrapper. None has no finite kind proof;
    /// callers requiring an indexed global domain must refuse that case.
    pub fn guard_kinds(&self) -> Option<Vec<u16>> {
        use std::collections::BTreeSet;
        fn necessary(set: &Set) -> Option<BTreeSet<u16>> {
            match set {
                Set::Kinds(kinds) => Some(kinds.iter().copied().collect()),
                Set::Empty => Some(BTreeSet::new()),
                Set::Intersect(items) => items
                    .iter()
                    .filter_map(necessary)
                    .reduce(|left, right| left.intersection(&right).copied().collect()),
                Set::Union(items) => {
                    let mut kinds = BTreeSet::new();
                    for item in items {
                        kinds.extend(necessary(item)?);
                    }
                    Some(kinds)
                }
                _ => None,
            }
        }
        necessary(&self.guard).map(|kinds| kinds.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{sql::relational::tests::policy, test_support::grammar_matchers};
    use serde_json::{Value, json};

    fn lower_body(body: Value) -> (Option<Downward>, Lowerer) {
        let p = policy(
            json!({"matches":"u"}),
            json!({"u":body,"other":{"kind":"none"}}),
        );
        let grammars = grammar_matchers(&p);
        let grammar = &grammars[0];
        let AstMatcher::Util(util) = &grammar.matcher else {
            panic!("utility root required")
        };
        let mut lower = Lowerer::new(grammar.language);
        let result = try_lower(util, &mut lower).unwrap();
        (result, lower)
    }

    #[test]
    fn recognizes_python_wrapper_and_pure_base_composition() {
        for base in [
            json!({"kind":"none"}),
            json!({"any":[{"pattern":"None"},{"kind":"identifier","regex":"^x$","not":{"inside":{"stopBy":"neighbor","kind":"call"}}}]}),
        ] {
            let (downward, _) = lower_body(json!({"any":[base,{
                "kind":"parenthesized_expression",
                "has":{"stopBy":"neighbor","matches":"u"}
            }]}));
            assert!(!downward.unwrap().guard_kinds().unwrap().is_empty());
        }
        let (downward, _) = lower_body(json!({"any":[{"kind":"none"},
            {"has":{"stopBy":"neighbor","matches":"u"}}
        ]}));
        assert_eq!(downward.unwrap().guard_kinds(), None);
    }

    #[test]
    fn refuses_effects_wrong_identity_stop_and_order_without_mutation() {
        for body in [
            json!({"any":[{"pattern":"$X"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]}),
            json!({"any":[{"resolves":{"global":["None"]}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]}),
            json!({"any":[{"matches":"other"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]}),
            json!({"any":[{"regex":"seed"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"other"}}]}),
            json!({"any":[{"regex":"seed"},{"kind":"parenthesized_expression","has":{"stopBy":"end","matches":"u"}}]}),
            json!({"any":[{"regex":"seed"},{"all":[{"has":{"stopBy":"neighbor","matches":"u"}},{"kind":"parenthesized_expression"}]}]}),
            json!({"any":[{"regex":"seed"},{"all":[{"pattern":"$X"},{"has":{"stopBy":"neighbor","matches":"u"}}]}]}),
            json!({"any":[{"regex":"seed"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","kind":"identifier","matches":"u"}}]}),
            json!({"any":[{"kind":"none"},{"kind":"true"},{"has":{"stopBy":"neighbor","matches":"u"}}]}),
        ] {
            let (downward, lower) = lower_body(body);
            assert!(downward.is_none());
            assert!(lower.program.terminals.is_empty());
            assert!(lower.program.relations.is_empty());
        }
    }

    #[test]
    fn finite_kind_proof_requires_every_union_arm() {
        let kinds = |guard| {
            Downward {
                base: Set::Empty,
                child_kinds: None,
                guard,
            }
            .guard_kinds()
        };
        assert_eq!(kinds(Set::Intersect(vec![])), None);
        assert_eq!(
            kinds(Set::Intersect(vec![
                Set::Kinds(vec![3, 1, 3]),
                Set::Regex(0)
            ])),
            Some(vec![1, 3])
        );
        assert_eq!(
            kinds(Set::Union(vec![Set::Kinds(vec![1]), Set::Regex(0)])),
            None
        );
        assert_eq!(
            kinds(Set::Intersect(vec![
                Set::Kinds(vec![1]),
                Set::Kinds(vec![3])
            ])),
            Some(vec![])
        );
    }
}
