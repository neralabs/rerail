//! Select semantic representations before physical planning or SQL generation.
//! Classification is not admission: the selected compiler still validates its complete contract.
use crate::code_rule_dsl::matcher::{AstMatcher, GrammarMatcher, StopBy};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Representation {
    TotalRelations,
    CapturedRelations,
    OrderedOutcomes,
}

#[derive(Default)]
struct Effects {
    captures: bool,
    outcomes: bool,
    utilities: HashSet<(usize, usize)>,
}

impl Effects {
    fn visit(&mut self, rule: &AstMatcher) -> Result<(), String> {
        match rule {
            AstMatcher::Pattern(pattern) => {
                self.captures |= !pattern.defined_vars().is_empty();
            }
            AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {}
            AstMatcher::All(items) | AstMatcher::Any(items) => {
                for item in items {
                    self.visit(item)?;
                }
            }
            // A capture inside negation can read an outer binding, even when it exports none.
            AstMatcher::Not(rule) => self.visit(rule)?,
            AstMatcher::Inside(rule, stop)
            | AstMatcher::Has(rule, stop)
            | AstMatcher::Follows(rule, stop)
            | AstMatcher::Precedes(rule, stop) => {
                self.visit(rule)?;
                // Stop bindings have their own environment, but still require capture semantics.
                if let StopBy::Rule(stop) = stop {
                    self.visit(stop)?;
                }
            }
            AstMatcher::Util(util) => {
                let key = (util.table.as_ptr() as usize, util.index);
                if self.utilities.insert(key) {
                    util.with_target(|rule| self.visit(rule))
                        .ok_or("relational classification: utility definition is unavailable")??;
                }
            }
            AstMatcher::Semantic(_)
            | AstMatcher::Count(_)
            | AstMatcher::Field(_)
            | AstMatcher::NthChild(_)
            | AstMatcher::Constrained(_)
            | AstMatcher::Compare(_)
            | AstMatcher::Call(_) => self.outcomes = true,
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => self.outcomes = true,
        }
        Ok(())
    }
}

pub(super) fn classify(grammar: &GrammarMatcher) -> Result<Representation, String> {
    if grammar.three_valued || grammar.transform.is_some() {
        return Ok(Representation::OrderedOutcomes);
    }
    let mut effects = Effects::default();
    effects.visit(&grammar.matcher)?;
    Ok(if effects.outcomes {
        Representation::OrderedOutcomes
    } else if effects.captures {
        Representation::CapturedRelations
    } else {
        Representation::TotalRelations
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime,
        sql::relational::tests::policy as compile_policy,
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use serde_json::{Value, json};

    fn selected(rule: Value, utils: Value) -> Representation {
        let policy = compile_policy(rule, utils);
        classify(&grammar_matchers(&policy)[0]).unwrap()
    }

    #[test]
    fn classification_retains_structural_sets_and_safe_recursive_utilities() {
        let utils = json!({
            "a":{"any":[{"kind":"identifier"},{"inside":{"matches":"b"}}]},
            "b":{"inside":{"matches":"a"}}
        });
        assert_eq!(
            selected(
                json!({"all":[{"matches":"a"},{"not":{"matches":"b"}}]}),
                utils,
            ),
            Representation::TotalRelations
        );
        for rule in [
            json!({"pattern":"foo.bar"}),
            json!({"not":{"follows":{"stopBy":"end","regex":"^x$"}}}),
            json!({"inside":{"stopBy":{"kind":"class_definition"},"kind":"function_definition"}}),
        ] {
            assert_eq!(selected(rule, json!({})), Representation::TotalRelations);
        }
    }

    #[test]
    fn classification_finds_captures_under_negation_stops_and_utilities() {
        for rule in [
            json!({"pattern":"$X"}),
            json!({"not":{"pattern":"foo($X)"}}),
            json!({"pattern":"foo($$$ARGS)"}),
            json!({"inside":{"kind":"call","stopBy":{"pattern":"$STOP"}}}),
        ] {
            assert_eq!(selected(rule, json!({})), Representation::CapturedRelations);
        }
        let utils = json!({
            "a":{"any":[{"inside":{"matches":"b"}},{"not":{"pattern":"$X"}}]},
            "b":{"inside":{"matches":"a"}}
        });
        assert_eq!(
            selected(json!({"matches":"b"}), utils),
            Representation::CapturedRelations
        );
    }

    #[test]
    fn classification_prioritizes_outcome_effects_over_capture_presence() {
        for rule in [
            json!({"has":{"field":"function","kind":"identifier"}}),
            json!({"nthChild":1}),
            json!({"count":{"has":{"kind":"identifier"},"min":1}}),
            json!({"kind":"identifier","value":{"constant":true}}),
            json!({"pattern":"f($A, $B)","compare":{"left":"A","op":"eq","right":"B","text":"node"}}),
        ] {
            assert_eq!(selected(rule, json!({})), Representation::OrderedOutcomes);
        }
        let utils = json!({
            "shared":{"nthChild":1},
            "outer":{"all":[{"pattern":"$X"},{"matches":"shared"}]}
        });
        assert_eq!(
            selected(json!({"matches":"outer"}), utils),
            Representation::OrderedOutcomes
        );
        assert_eq!(
            selected(
                json!({"matches":{"identity":{"P":"$X"}}}),
                json!({"identity":{"params":["P"],"rule":{"pattern":"$P"}}}),
            ),
            Representation::OrderedOutcomes
        );
    }

    #[test]
    fn classification_checks_constraint_wrappers_and_truth_mode() {
        let policy = CodeRuleDslRuntime::compile(
            json!({"format":"rerail.policy/v1","id":"classifier","revision":1,
                "mode":"steer","scope":{"include":["**/*"],"exclude":[]},
                "clauses":[{"id":"c","forbid":{"ast":{"language":"python",
                    "rule":{"pattern":"$X"},"constraints":{"X":{"kind":"identifier"}}}},
                    "violation":{"code":"c","message":"match","anchor":"witness.span"}}]})
            .to_string(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap();
        assert_eq!(
            classify(&grammar_matchers(&policy)[0]).unwrap(),
            Representation::OrderedOutcomes
        );
        let policy = compile_policy(json!({"kind":"identifier"}), json!({}));
        let mut grammar = grammar_matchers(&policy)[0].clone();
        grammar.three_valued = true;
        assert_eq!(classify(&grammar).unwrap(), Representation::OrderedOutcomes);
    }

    #[test]
    fn classification_rejects_expired_utility_definitions() {
        let owner = compile_policy(json!({"matches":"a"}), json!({"a":{"kind":"identifier"}}));
        let matcher = grammar_matchers(&owner)[0].matcher.clone();
        drop(owner);
        let policy = compile_policy(json!({"kind":"identifier"}), json!({}));
        let mut grammar = grammar_matchers(&policy)[0].clone();
        grammar.matcher = matcher;
        assert!(classify(&grammar).unwrap_err().contains("unavailable"));
    }
}
