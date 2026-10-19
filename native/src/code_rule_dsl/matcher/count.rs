use super::AstMatcher;
use super::relation::descend_until;
use super::util::{THREE_VALUED, UNKNOWN, UTIL_EXCEEDED};
use super::{StopBy, spend_util_step};
use crate::code_rule_dsl::budget;
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::cell::Cell;
use std::ops::ControlFlow;

/// `count`: how many nodes along a relation's walk match its rule, bounded by `min`/`max`
/// (a Rerail extension). The walk is the relation's own, except that `has` keeps descending
/// below a match (every matching descendant counts) and stops only where `stopBy` says. The
/// rule reads the caller's bindings; like `not`, nothing it binds is kept. With `as`, the count
/// itself is bound as text (`CountBinding`).
#[derive(Clone)]
pub(crate) struct Count {
    /// `Inside`, `Has`, `Precedes` or `Follows`.
    pub(crate) relation: AstMatcher,
    pub(crate) min: usize,
    pub(crate) max: Option<usize>,
    pub(crate) binding: Option<CountBinding>,
}

/// `as` and `offset`: a count that holds binds `offset` plus the exact count, in decimal, as a
/// transformed variable named `name`, which messages and source-text comparisons read. The walk then runs to
/// its end rather than stopping at `min`, and a count it cannot know exactly is unknown.
#[derive(Clone, Debug)]
pub(crate) struct CountBinding {
    pub(crate) name: String,
    pub(crate) offset: i64,
}

impl CountBinding {
    fn bind<D: Doc>(&self, env: &mut Cow<MetaVarEnv<'_, D>>, found: usize) {
        let value = i128::try_from(found).unwrap_or(i128::MAX) + i128::from(self.offset);
        let bytes = <D::Source as Content>::decode_str(&value.to_string()).into_owned();
        env.to_mut()
            .insert_transformation(&MetaVariable::Multiple, &self.name, bytes);
    }
}

impl Count {
    /// Three-valued: a node whose verdict is unknown may or may not count, and an unknown stop
    /// verdict may hide any number of further nodes. When the bounds could go either way the count
    /// fails with `UNKNOWN` set; so does a bound count (`as`) whose exact value is not known.
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
        // Each tested node spends a step; a spent budget fails the clause for the file.
        let mut test = |rule: &AstMatcher, candidate: Node<'tree, D>| -> ControlFlow<()> {
            if !spend_util_step() || budget::matcher_call_budget_exhausted() {
                return ControlFlow::Break(());
            }
            let mut probe = Cow::Borrowed(env.as_ref());
            if rule.match_node_with_env(candidate, &mut probe).is_some() {
                found += 1;
                // Past `max` the answer is no; at `min` with no `max` it is yes, unless the
                // count is bound, which needs every node counted.
                if self.max.is_some_and(|max| found > max)
                    || (self.max.is_none() && self.binding.is_none() && found >= self.min)
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
        // A bound count breaks off before its end only past `max`; any other break is a spent
        // budget, and a partial count is no value to report.
        if walked.is_break()
            && (UTIL_EXCEEDED.get().is_some()
                || (self.binding.is_some() && self.max.is_none_or(|max| found <= max)))
        {
            return false;
        }
        // The true count lies in `found..=found + unknown`, unbounded above after an unknown stop.
        let high = (!open.get()).then_some(found + unknown);
        let exact = self.binding.is_none() || high == Some(found);
        let holds = exact
            && found >= self.min
            && self
                .max
                .is_none_or(|max| high.is_some_and(|high| high <= max));
        if holds {
            if let Some(binding) = &self.binding {
                binding.bind(env, found);
            }
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
    use crate::code_rule_dsl::CodeRuleDslRuntime;
    use crate::code_rule_dsl::matcher::UtilLimit;
    use crate::code_rule_dsl::test_support::*;
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
        let reason = rejection(CodeRuleDslRuntime::compile(
            document.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(reason.contains("capture.ARG"), "{reason}");
    }

    /// The typescript `ast` query (`rule`, and `utils` or `constraints`) as one clause reporting
    /// `violation`.
    fn bound_clause(mut ast: Value, violation: Value) -> Value {
        ast["language"] = json!("typescript");
        clause_document(
            json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation }),
        )
    }

    fn report(message: &str) -> Value {
        json!({ "anchor": "witness.span", "code": "CYCLOMATIC_COMPLEXITY", "message": message })
    }

    /// The messages of the violations `ast` reports in `source`, each rendering `message`.
    fn messages(ast: Value, message: &str, source: &str) -> Vec<String> {
        let document = bound_clause(ast, report(message));
        let compiled = compile_document_with(document, &capabilities_with(|_| {}))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message").to_owned())
            .collect()
    }

    #[test]
    fn a_bound_count_renders_its_whole_count_plus_offset_in_the_message() {
        let ifs = |n: usize| "  if (a) {}\n".repeat(n);
        let source = format!(
            "function wide(a) {{\n{}}}\nfunction narrow(a) {{\n{}}}\n",
            ifs(9),
            ifs(2)
        );
        let decisions = json!({ "kind": "if_statement", "stopBy": "end" });
        let rule =
            |count: Value| json!({ "rule": { "kind": "function_declaration", "count": count } });
        let message = "Cyclomatic complexity $CCN (limit 6).";
        // Past `min` the walk goes on to the end: nine decisions, plus one.
        assert_eq!(
            messages(
                rule(json!({ "has": decisions, "min": 6, "as": "CCN", "offset": 1 })),
                message,
                &source
            ),
            ["Cyclomatic complexity 10 (limit 6)."]
        );
        // Each function binds its own count; `offset` defaults to 0 and may be negative.
        assert_eq!(
            messages(
                rule(json!({ "has": decisions, "min": 2, "as": "CCN" })),
                message,
                &source
            ),
            [
                "Cyclomatic complexity 9 (limit 6).",
                "Cyclomatic complexity 2 (limit 6)."
            ]
        );
        assert_eq!(
            messages(
                rule(json!({ "has": decisions, "max": 2, "as": "CCN", "offset": -3 })),
                message,
                &source
            ),
            ["Cyclomatic complexity -1 (limit 6)."]
        );
        // A utility shares its caller's environment, the count's value too.
        let utility = json!({
            "rule": { "matches": "complex" },
            "utils": { "complex": { "kind": "function_declaration",
                "count": { "has": decisions, "min": 6, "as": "CCN", "offset": 1 } } },
        });
        assert_eq!(
            messages(utility, message, &source),
            ["Cyclomatic complexity 10 (limit 6)."]
        );
    }

    #[test]
    fn a_bound_count_that_may_miss_a_node_is_unknown_and_reports_nothing() {
        // `if (u)` may or may not be a decision: `f` has two or three, `g` exactly two.
        let source = "function f(a, u) {\n  if (a) {}\n  if (a) {}\n  if (u) {}\n}\n\
                      function g(a) {\n  if (a) {}\n  if (a) {}\n}\n";
        let decisions =
            json!({ "kind": "if_statement", "stopBy": "end", "testUnknown": "^if \\(u\\)" });
        let unbound = json!({ "has": decisions, "min": 2 });
        let bound = json!({ "has": decisions, "min": 2, "as": "CCN" });
        let rule = |rule: Value| json!({ "rule": rule });
        let function = |count: &Value| json!({ "kind": "function_declaration", "count": count });
        // Without `as` the two known decisions reach `min` in both.
        assert_eq!(messages(rule(function(&unbound)), "m", source), ["m", "m"]);
        // With it `f`'s exact count is unknown: no finding, and no finding for its negation either.
        assert_eq!(messages(rule(function(&bound)), "$CCN", source), ["2"]);
        assert_eq!(
            messages(
                rule(json!({ "kind": "function_declaration", "not": { "count": bound } })),
                "m",
                source
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn only_a_bound_count_walks_past_min() {
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxVisitedNodes"] = json!(10_000)
        });
        let source = format!("{}x;{}\n", "{ y; ".repeat(200), "}".repeat(200));
        let run = |count: Value| {
            let ast = json!({ "rule": { "kind": "statement_block", "count": count } });
            let rule = compile_document_with(bound_clause(ast, violation()), &capabilities)
                .unwrap_or_else(|error| panic!("{}", error.reason));
            evaluate_file(&rule, "typescript", "a.ts", &source)
        };
        let identifiers = json!({ "kind": "identifier", "stopBy": "end" });
        // Without `as` each block stops at its first identifier, a few nodes in.
        assert_violations(&run(json!({ "has": identifiers, "min": 1 })), 200);
        // With it every block counts its whole subtree: quadratic in depth, past the budget.
        let result = run(json!({ "has": identifiers, "min": 1, "as": "N" }));
        assert_eq!(
            result["failures"],
            json!([{ "clauseId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Work.message(), "path": "a.ts" }]),
            "{result}"
        );
    }

    #[test]
    fn a_count_value_has_no_source_node() {
        let count =
            json!({ "has": { "kind": "if_statement", "stopBy": "end" }, "min": 1, "as": "CCN" });
        let bound = json!({ "rule": { "kind": "function_declaration", "count": count } });
        let mut constrained = bound.clone();
        constrained["constraints"] = json!({ "CCN": { "regex": "^1$" } });
        for (ast, violation, expected) in [
            (
                bound.clone(),
                json!({ "anchor": "witness.span", "code": "C", "message": "m", "fix": "$CCN" }),
                "/clauses/0/violation/fix: `$CCN` is a count's value, which only a message may interpolate",
            ),
            (
                bound.clone(),
                json!({ "anchor": "capture.CCN", "code": "C", "message": "m" }),
                "/clauses/0/violation/anchor: `capture.CCN` names no capture; the query binds no captures",
            ),
            (
                bound.clone(),
                report("$$$CCN"),
                "/clauses/0/violation/message: `$$$CCN`: a count's value is text; use `$CCN`",
            ),
            (
                json!({ "rule": { "kind": "function_declaration", "not": { "count": count } } }),
                report("$CCN"),
                "/clauses/0/violation/message: `$CCN` names no capture; the query binds no captures; write `\\$CCN` for literal text",
            ),
            (
                json!({ "rule": { "kind": "function_declaration", "count": count,
                    "has": { "kind": "identifier", "pattern": "$CCN" } } }),
                report("$CCN"),
                "/clauses/0/violation/message: `$CCN` names both a capture and a count's value; rename one",
            ),
            (
                constrained,
                report("$CCN"),
                "/clauses/0/forbid/ast/constraints/CCN: metavariable `$CCN` is not defined by the rule, its utils, or its constraints",
            ),
        ] {
            let reason = rejection(compile_document_with(
                bound_clause(ast, violation),
                &capabilities_with(|_| {}),
            ));
            assert_eq!(reason, expected);
        }
    }

    #[test]
    fn a_count_value_compares_as_source_text() {
        let rule = compile_typescript_rule(json!({
            "kind": "function_declaration", "all": [
                { "count": { "has": { "kind": "if_statement", "stopBy": "end" }, "min": 1, "as": "N" } },
                { "has": { "kind": "number", "pattern": "$EXPECTED", "stopBy": "end" } },
                { "compare": { "left": "N", "right": "EXPECTED", "op": "eq", "text": "source" } }
            ]
        })).unwrap();
        assert_violations(
            &evaluate_file(
                &rule,
                "typescript",
                "a.ts",
                "function f() { if (x) return 1; }",
            ),
            1,
        );
        assert_violations(
            &evaluate_file(
                &rule,
                "typescript",
                "a.ts",
                "function f() { if (x) return 1; if (y) return 1; }",
            ),
            0,
        );
    }

    #[test]
    fn a_count_this_dialect_cannot_express_counts_nothing() {
        let rule = compile_typescript_rule(json!({
            "kind": "program",
            "count": { "has": { "kind": "jsx_self_closing_element", "stopBy": "end" }, "max": 0 },
        }))
        .expect("whole in the tsx dialect");
        assert_violations(&evaluate_file(&rule, "typescript", "a.ts", "x;\n"), 1);
        assert_violations(&evaluate_file(&rule, "tsx", "a.tsx", "x;\n"), 1);
        assert_violations(&evaluate_file(&rule, "tsx", "a.tsx", JSX_SOURCE), 1);
        assert_violations(
            &evaluate_file(&rule, "tsx", "a.tsx", "const v = <br />;\n"),
            0,
        );
    }

    #[test]
    fn counted_nodes_spend_the_visited_node_budget() {
        let document = clause_document(json!({
            "id": "ast-match",
            "violation": violation(),
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
        assert_violations(&result, 1);
    }

    /// ESLint no-dupe-keys counts each property's earlier siblings of the same key. The walk reads
    /// only the siblings of that key (`Related::same_key_siblings`), so an object's work grows
    /// with its size, not with its square, as a walk over every earlier sibling's does; both
    /// report the same keys, duplicates of mixed kinds among keys without a static name included.
    #[test]
    fn same_key_sibling_counts_grow_linearly_and_report_as_full_walks() {
        use crate::code_rule_dsl::budget::MatchBudget;
        use crate::code_rule_dsl::matcher::set_same_key_off;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let clause = json!({
            "id": "dupe",
            "forbid": { "ast": no_dupe_keys_ast() },
            "violation": { "anchor": "capture.K", "code": "D", "message": "d" },
        });
        let code_rule = compile_document_with(
            clause_document(clause),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        // Violations, and the budget units their evaluation spent.
        let run = |full: bool, source: &str| {
            set_same_key_off(full);
            let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
            let result = evaluate_file(&code_rule, "typescript", "keys.ts", source);
            let spent = budget.spent();
            drop(budget);
            set_same_key_off(false);
            assert_eq!(result["failures"], json!([]), "{result}");
            let starts = result["violations"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|violation| violation["span"]["startByte"].as_u64().expect("start"))
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
