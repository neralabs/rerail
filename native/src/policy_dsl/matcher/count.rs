use super::AstMatcher;
use super::relation::descend_until;
use super::util::{THREE_VALUED, UNKNOWN, UTIL_EXCEEDED};
use super::{StopBy, spend_util_step};
use crate::policy_dsl::budget;
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::cell::Cell;
use std::ops::ControlFlow;

/// `count`: how many nodes along a relation's walk match its rule, bounded by `min`/`max`
/// (a Rerail extension). The walk is the relation's own, except that `has` keeps descending
/// below a match (every matching descendant counts) and stops only where `stopBy` says. The
/// rule reads the caller's bindings; like `not`, nothing it binds is kept.
#[derive(Clone)]
pub(crate) struct Count {
    /// `Inside`, `Has`, `Precedes` or `Follows`.
    pub(crate) relation: AstMatcher,
    pub(crate) min: usize,
    pub(crate) max: Option<usize>,
}

impl Count {
    /// Three-valued: a node whose verdict is unknown may or may not count, and an unknown stop
    /// verdict may hide any number of further nodes. When the bounds could go either way the count
    /// fails with `UNKNOWN` set.
    pub(super) fn holds<'tree, D: Doc>(
        &self,
        node: &Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> bool {
        let three = THREE_VALUED.get();
        let saved = three && UNKNOWN.replace(false);
        let mut found = 0usize;
        let mut unknown = 0usize;
        // To the end, a rule looking for a sibling with the same key as a capture bound outside
        // it is tested at the siblings it may match only (`Related::same_key_siblings`).
        let listed = match &self.relation {
            AstMatcher::Precedes(rule, StopBy::End) => rule.same_key_siblings(node, env, true),
            AstMatcher::Follows(rule, StopBy::End) => rule.same_key_siblings(node, env, false),
            _ => None,
        };
        // Each tested node spends a step; a spent budget fails the check for the file.
        let mut test = |rule: &AstMatcher, candidate: Node<'tree, D>| -> ControlFlow<()> {
            if !spend_util_step() || budget::matcher_call_budget_exhausted() {
                return ControlFlow::Break(());
            }
            let mut probe = Cow::Borrowed(env.as_ref());
            if rule.match_node_with_env(candidate, &mut probe).is_some() {
                found += 1;
                // Past `max` the answer is no; at `min` with no `max` it is yes.
                if self.max.is_some_and(|max| found > max)
                    || (self.max.is_none() && found >= self.min)
                {
                    return ControlFlow::Break(());
                }
            } else if three && UNKNOWN.replace(false) {
                unknown += 1;
            }
            ControlFlow::Continue(())
        };
        let open = Cell::new(false);
        let walked = match (&self.relation, listed) {
            (AstMatcher::Precedes(rule, _) | AstMatcher::Follows(rule, _), Some(listed)) => {
                listed.into_iter().try_for_each(|n| test(rule, n))
            }
            (AstMatcher::Inside(rule, stop), _) => stop.walk(
                || node.parent(),
                || node.ancestors(),
                |n| test(rule, n),
                &open,
                three,
            ),
            (AstMatcher::Precedes(rule, stop), _) => stop.walk(
                || node.next(),
                || node.next_all(),
                |n| test(rule, n),
                &open,
                three,
            ),
            (AstMatcher::Follows(rule, stop), _) => stop.walk(
                || node.prev(),
                || node.prev_all(),
                |n| test(rule, n),
                &open,
                three,
            ),
            (AstMatcher::Has(rule, stop), _) => match stop {
                StopBy::Neighbor => node.children().try_for_each(|n| test(rule, n)),
                StopBy::End => node.dfs().skip(1).try_for_each(|n| test(rule, n)),
                StopBy::Rule(stop) => descend_until(node, stop, |n| test(rule, n), &open, three),
            },
            _ => {
                if three {
                    UNKNOWN.set(saved);
                }
                return false;
            }
        };
        if three {
            UNKNOWN.set(saved);
        }
        if walked.is_break() && UTIL_EXCEEDED.get().is_some() {
            return false;
        }
        // The true count lies in `found..=found + unknown`, unbounded above after an unknown stop.
        let high = (!open.get()).then_some(found + unknown);
        let holds = found >= self.min
            && self
                .max
                .is_none_or(|max| high.is_some_and(|high| high <= max));
        if holds {
            return true;
        }
        let fails =
            high.is_some_and(|high| high < self.min) || self.max.is_some_and(|max| found > max);
        if !fails && three {
            UNKNOWN.set(true);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::PolicyDslRuntime;
    use crate::policy_dsl::matcher::UtilLimit;
    use crate::policy_dsl::test_support::*;
    use serde_json::{Value, json};

    fn count_texts(rule: Value, source: &str) -> Vec<String> {
        let compiled =
            compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
            .into_iter()
            .map(|text| text.lines().next().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn count_bounds_the_nodes_a_relation_reaches() {
        // Cyclomatic complexity: decision points of the function itself, not of nested functions.
        let decisions = json!({ "any": [
            { "kind": "if_statement" },
            { "kind": "ternary_expression" },
            { "kind": "binary_expression", "has": { "regex": "^(&&|\\|\\|)$" } },
        ] });
        let functions =
            json!({ "any": [{ "kind": "function_declaration" }, { "kind": "arrow_function" }] });
        let complex = |min: u64| {
            let mut relation = decisions.clone();
            relation["stopBy"] = functions.clone();
            json!({ "kind": "function_declaration", "count": { "has": relation, "min": min } })
        };
        let source = "function outer(a, b) {\n  if (a && b) { if (a) {} }\n  const f = () => a ? 1 : b || 2;\n}\n\
                      function flat(a) {\n  return a ? 1 : 2;\n}\n";
        // Nested matches count too (`if` inside `if`); the arrow function's two decisions do not.
        assert_eq!(count_texts(complex(3), source), ["function outer(a, b) {"]);
        assert_eq!(count_texts(complex(4), source), Vec::<String>::new());
        assert_eq!(
            count_texts(complex(1), source),
            ["function outer(a, b) {", "function flat(a) {"]
        );
        // `max` alone bounds from above; `max: 0` is `not has`.
        let at_most = |max: u64| json!({ "kind": "function_declaration", "count": { "has": { "kind": "if_statement", "stopBy": "end" }, "max": max } });
        assert_eq!(count_texts(at_most(0), source), ["function flat(a) {"]);
        assert_eq!(
            count_texts(at_most(2), source),
            ["function outer(a, b) {", "function flat(a) {"]
        );
        let exactly_one = json!({
            "kind": "function_declaration",
            "count": { "has": { "kind": "ternary_expression", "stopBy": "end" }, "min": 1, "max": 1 },
        });
        assert_eq!(
            count_texts(exactly_one, source),
            ["function outer(a, b) {", "function flat(a) {"]
        );
        // `neighbor` counts direct children only.
        let two_statements = json!({
            "kind": "statement_block",
            "count": { "has": { "regex": "^(if|const)" }, "min": 2 },
        });
        assert_eq!(count_texts(two_statements, source), ["{"]);
        // Ancestors: an `if` nested in two others.
        let deep = json!({
            "kind": "if_statement",
            "count": { "inside": { "kind": "if_statement", "stopBy": "end" }, "min": 2 },
        });
        let nested = "if (a) { if (b) { if (c) {} } }\n";
        assert_eq!(count_texts(deep, nested), ["if (c) {}"]);
        // Siblings, with a stop rule: at least two statements before this one, walking back to the
        // nearest `reset();`, which is tested like any stop node (so it counts for `e();`).
        let third = json!({
            "pattern": "$F();",
            "count": { "follows": { "kind": "expression_statement", "stopBy": { "pattern": "reset();" } }, "min": 2 },
        });
        let calls = "a();\nb();\nc();\nreset();\nd();\ne();\n";
        assert_eq!(
            count_texts(third, calls),
            ["c(); F=c", "reset(); F=reset", "e(); F=e"]
        );
    }

    #[test]
    fn count_reads_bindings_and_binds_nothing() {
        // The counted rule sees `$F`; a callee named at least twice more inside its own call.
        let source = "f(f, f);\ng(g);\n";
        let rule = json!({
            "pattern": "$F($$$)",
            "count": { "has": { "pattern": "$F", "kind": "identifier", "stopBy": "end" }, "min": 3 },
        });
        assert_eq!(count_texts(rule, source), ["f(f, f) F=f"]);
        let document = json!({
            "format": "rerail.policy/v1",
            "id": "count-captures",
            "revision": 1,
            "mode": "block",
            "clauses": [{
                "id": "ast-match",
                "forbid": { "ast": { "language": "typescript", "rule": {
                    "kind": "call_expression",
                    "count": { "has": { "pattern": "$ARG", "kind": "identifier", "stopBy": "end" }, "min": 1 },
                } } },
                "violation": { "anchor": "capture.ARG", "code": "AST_MATCH", "message": "x" },
            }],
        });
        let reason = rejection(PolicyDslRuntime::compile(
            document.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(reason.contains("capture.ARG"), "{reason}");
    }

    #[test]
    fn a_count_this_dialect_cannot_express_counts_nothing() {
        let rule = compile_typescript_rule(json!({
            "kind": "program",
            "count": { "has": { "kind": "jsx_self_closing_element", "stopBy": "end" }, "max": 0 },
        }))
        .expect("whole in the tsx dialect");
        assert_findings(&evaluate_file(&rule, "typescript", "a.ts", "x;\n"), 1);
        assert_findings(&evaluate_file(&rule, "tsx", "a.tsx", "x;\n"), 1);
        assert_findings(&evaluate_file(&rule, "tsx", "a.tsx", JSX_SOURCE), 1);
        assert_findings(
            &evaluate_file(&rule, "tsx", "a.tsx", "const v = <br />;\n"),
            0,
        );
    }

    #[test]
    fn counted_nodes_spend_the_visited_node_budget() {
        let document = check_document(json!({
            "id": "ast-match",
            "violation": finding(),
            "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "statement_block", "count": { "has": { "kind": "identifier", "stopBy": "end" }, "max": 100_000 } },
            } },
        }));
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxVisitedNodes"] = json!(10_000)
        });
        let rule = compile_document_with(document, &capabilities)
            .unwrap_or_else(|error| panic!("{}", error.reason));
        // Every nested block recounts its whole subtree: quadratic in depth.
        let source = format!("{}x;{}\n", "{ y; ".repeat(200), "}".repeat(200));
        let result = evaluate_file(&rule, "typescript", "a.ts", &source);
        assert_eq!(
            result["failures"],
            json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Work.message(), "path": "a.ts" }]),
            "{result}"
        );
        let result = evaluate_file(&rule, "typescript", "a.ts", "{ x; }\n");
        assert_findings(&result, 1);
    }

    /// ESLint no-dupe-keys counts each property's earlier siblings of the same key. The walk reads
    /// only the siblings of that key (`Related::same_key_siblings`), so an object's work grows
    /// with its size, not with its square, as a walk over every earlier sibling's does; both
    /// report the same keys, duplicates of mixed kinds among keys without a static name included.
    #[test]
    fn same_key_sibling_counts_grow_linearly_and_report_as_full_walks() {
        use crate::policy_dsl::budget::MatchBudget;
        use crate::policy_dsl::matcher::set_same_key_off;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let check = json!({
            "id": "dupe",
            "forbid": { "ast": no_dupe_keys_ast() },
            "violation": { "anchor": "capture.K", "code": "D", "message": "d" },
        });
        let policy = compile_document_with(
            check_document(check),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        // Findings, and the budget units their evaluation spent.
        let run = |full: bool, source: &str| {
            set_same_key_off(full);
            let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
            let result = evaluate_file(&policy, "typescript", "keys.ts", source);
            let spent = budget.spent();
            drop(budget);
            set_same_key_off(false);
            assert_eq!(result["failures"], json!([]), "{result}");
            let starts = result["violations"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
                .collect::<Vec<_>>();
            (starts, spent)
        };
        let mut units = Vec::new();
        for keys in [100, 200, 400, 800] {
            let mut source = "export const o = {\n".to_owned();
            for key in 0..keys {
                source.push_str(&format!("  k{key}: {key},\n"));
            }
            let duplicates =
                "k7: 1, 'k9': 2, [\"k11\"]: 3, 1: 4, [x]: 5, ...y, '1': 6, [x]: 7, 0x1: 8 };\n";
            source.push_str(duplicates);
            let tail = source.len() - duplicates.len();
            let at = |needle: &str| (tail + source[tail..].find(needle).expect(needle)) as u64;
            let reported = vec![at("k7"), at("'k9'"), at("\"k11\""), at("'1'"), at("0x1")];
            let (found, spent) = run(false, &source);
            assert_eq!(found, reported, "{keys} keys");
            let (walked, walk_spent) = run(true, &source);
            assert_eq!(
                walked, reported,
                "{keys} keys, every earlier sibling walked"
            );
            units.push((spent, walk_spent));
        }
        for pair in units.windows(2) {
            let ((before, _), (after, _)) = (pair[0], pair[1]);
            assert!(
                after <= before * 9 / 4,
                "twice the keys spends at most about twice the units: {units:?}"
            );
        }
        let ((_, full_400), (linear_800, full_800)) = (units[2], units[3]);
        assert!(
            full_800 >= full_400 * 3 && full_800 >= linear_800 * 10,
            "the walk over every earlier sibling grows with the square: {units:?}"
        );
    }
}
