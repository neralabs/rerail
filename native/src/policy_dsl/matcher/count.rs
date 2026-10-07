use super::AstMatcher;
#[cfg(test)]
use super::relation::descend_until;
#[cfg(test)]
use super::util::{THREE_VALUED, UNKNOWN, UTIL_EXCEEDED};
#[cfg(test)]
use super::{StopBy, spend_util_step};
#[cfg(test)]
use crate::policy_dsl::budget;
#[cfg(test)]
use ast_grep_core::matcher::Matcher;
#[cfg(test)]
use ast_grep_core::meta_var::MetaVarEnv;
#[cfg(test)]
use ast_grep_core::{Doc, Node};
#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
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
    #[cfg(test)]
    pub(super) fn holds<'tree, D: Doc>(
        &self,
        node: &Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> bool {
        let three = THREE_VALUED.get();
        let saved = three && UNKNOWN.replace(false);
        let mut found = 0usize;
        let mut unknown = 0usize;
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
        let walked = match &self.relation {
            AstMatcher::Inside(rule, stop) => stop.walk(
                || node.parent(),
                || node.ancestors(),
                |n| test(rule, n),
                &open,
                three,
            ),
            AstMatcher::Precedes(rule, stop) => stop.walk(
                || node.next(),
                || node.next_all(),
                |n| test(rule, n),
                &open,
                three,
            ),
            AstMatcher::Follows(rule, stop) => stop.walk(
                || node.prev(),
                || node.prev_all(),
                |n| test(rule, n),
                &open,
                three,
            ),
            AstMatcher::Has(rule, stop) => match stop {
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
            "checks": [{
                "id": "ast-match",
                "forbid": { "ast": { "language": "typescript", "rule": {
                    "kind": "call_expression",
                    "count": { "has": { "pattern": "$ARG", "kind": "identifier", "stopBy": "end" }, "min": 1 },
                } } },
                "finding": { "anchor": "capture.ARG", "code": "AST_MATCH", "message": "x", "severity": "error" },
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
            "finding": finding(),
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
            json!([{ "checkId": "ast-match", "code": "budget-exceeded", "message": UtilLimit::Work.message(), "path": "a.ts" }]),
            "{result}"
        );
        let result = evaluate_file(&rule, "typescript", "a.ts", "{ x; }\n");
        assert_findings(&result, 1);
    }
}
