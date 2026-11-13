    use super::{candidates_tested, may_bind, reset_candidates_tested};
    use crate::code_rule_dsl::CodeRuleDslRuntime;
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn stop_rule_texts(rule: Value, source: &str) -> Vec<String> {
        let compiled =
            compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
    }

    #[test]
    fn stop_rules_end_the_walk_after_the_first_node_they_match() {
        // `inside`: the stop node itself is still tested (ast-grep-config `inclusive_until`).
        let source = "class A { m() { f(); } }\nclass B { m() { return () => { g(); }; } }\n";
        let inside = |target: &str| {
            json!({
                "pattern": "$F()",
                "inside": { "kind": target, "stopBy": { "kind": "method_definition" } },
            })
        };
        assert_eq!(
            stop_rule_texts(inside("class_body"), source),
            Vec::<String>::new()
        );
        assert_eq!(
            stop_rule_texts(inside("method_definition"), source),
            ["f() F=f", "g() F=g"]
        );
        assert_eq!(
            stop_rule_texts(inside("arrow_function"), source),
            ["g() F=g"]
        );
        // `has`: a descendant the stop rule matches is tested but not entered.
        let source =
            "function f() { a(); function g() { b(); } }\nfunction h() { const k = () => c(); }\n";
        let has = |target: Value| {
            json!({
                "kind": "function_declaration",
                "has": { "all": [target], "stopBy": { "any": [
                    { "kind": "function_declaration" },
                    { "kind": "arrow_function" },
                ] } },
            })
        };
        assert_eq!(
            stop_rule_texts(has(json!({ "pattern": "b()" })), source),
            ["function g() { b(); }"]
        );
        assert_eq!(
            stop_rule_texts(has(json!({ "pattern": "c()" })), source),
            Vec::<String>::new()
        );
        assert_eq!(
            stop_rule_texts(has(json!({ "kind": "arrow_function" })), source),
            ["function h() { const k = () => c(); }"]
        );
        // `precedes` / `follows`: the stop sibling is tested, the siblings past it are not.
        let source = "a();\nbreak_here;\nb();\n";
        let sibling = |relation: &str, target: &str| {
            json!({
                "kind": "expression_statement",
                "pattern": target,
                relation: { "pattern": "$X();", "stopBy": { "pattern": "break_here;" } },
            })
        };
        assert_eq!(
            stop_rule_texts(sibling("precedes", "a();"), source),
            Vec::<String>::new()
        );
        assert_eq!(
            stop_rule_texts(sibling("follows", "b();"), source),
            Vec::<String>::new()
        );
        let reach_stop = json!({
            "pattern": "a();",
            "precedes": { "pattern": "break_here;", "stopBy": { "pattern": "break_here;" } },
        });
        assert_eq!(stop_rule_texts(reach_stop, source), ["a();"]);
    }

    #[test]
    fn stop_rules_neither_read_nor_bind_the_callers_metavariables() {
        // `$F` is bound to `f` when the stop rule runs; upstream matches the stop rule in a fresh
        // environment, so `$F()` stops at the first call of any name.
        let source = "f(g(h(f())));\n";
        let rule = json!({
            "pattern": "$F($$$)",
            "has": { "pattern": "$F()", "stopBy": { "pattern": "$F($$$)" } },
        });
        assert_eq!(stop_rule_texts(rule, source), Vec::<String>::new());
        // Names a stop rule would bind are not captures: a message or constraint using one is an error.
        let document = json!({
            "format": "rerail.policy/v1",
            "id": "stop-captures",
            "revision": 1,
            "mode": "block",
            "clauses": [{
                "id": "ast-match",
                "forbid": { "ast": { "language": "typescript", "rule": {
                    "pattern": "$F()",
                    "inside": { "kind": "function_declaration", "stopBy": { "pattern": "function $NAME() {}" } },
                } } },
                "violation": { "anchor": "witness.span", "code": "AST_MATCH", "message": "in $NAME" },
            }],
        });
        let reason = rejection(CodeRuleDslRuntime::compile(
            document.to_string(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        ));
        assert!(reason.contains("$NAME"), "{reason}");
        let reason = rejection(compile_typescript_ast(json!({
            "rule": { "pattern": "$F()", "inside": { "kind": "program", "stopBy": { "pattern": "$G($$$)" } } },
            "constraints": { "G": { "kind": "identifier" } },
        })));
        assert!(reason.contains("`$G` is not defined"), "{reason}");
    }

    #[test]
    fn stop_rules_compile_per_dialect_and_may_use_utils() {
        // Under the plain dialect the JSX stop rule never matches, so the walk runs to the end.
        let rule = json!({
            "kind": "identifier",
            "regex": "^go$",
            "inside": { "kind": "arrow_function", "stopBy": { "kind": "jsx_expression" } },
        });
        let jsx = compile_typescript_rule(rule).expect("whole in the tsx dialect");
        let source = "export const V = () => <b onClick={go} />;\n";
        assert_violations(&evaluate_file(&jsx, "tsx", "a.tsx", source), 0);
        assert_violations(
            &evaluate_file(&jsx, "typescript", "a.ts", "const V = () => go;\n"),
            1,
        );
        let rule = compiled(json!({
            "rule": { "pattern": "$F()", "inside": { "kind": "class_body", "stopBy": { "matches": "function" } } },
            "utils": { "function": { "any": [{ "kind": "function_declaration" }, { "kind": "method_definition" }] } },
        }));
        let source = "class A { x = f(); m() { g(); } }\n";
        let result = evaluate_file(&rule, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["f() F=f"], "{result}");
    }

    #[test]
    fn inside_with_a_field_requires_the_path_to_enter_through_it() {
        // ast-grep-config 0.45.3 `Inside`: each ancestor tested must hold the previous node of the
        // walk in the field; an ancestor reached through another child is skipped, not stopped at.
        let source = "const a = b;\nif (a && x) { x; }\n";
        assert_eq!(
            stop_rule_texts(
                json!({ "kind": "identifier", "inside": { "kind": "variable_declarator", "field": "name" } }),
                source,
            ),
            ["a"]
        );
        let condition = json!({
            "kind": "identifier",
            "regex": "^x$",
            "inside": { "kind": "if_statement", "field": "condition", "stopBy": "end" },
        });
        assert_eq!(stop_rule_texts(condition, source), ["x"]);
        // With a stop rule the walk ends at the first stop node, entered through the field or not.
        let stopped = json!({
            "kind": "identifier",
            "regex": "^x$",
            "inside": { "kind": "if_statement", "field": "condition", "stopBy": { "kind": "binary_expression" } },
        });
        assert_eq!(stop_rule_texts(stopped, source), Vec::<String>::new());
        let reached = json!({
            "kind": "identifier",
            "regex": "^x$",
            "inside": { "kind": "if_statement", "field": "condition", "stopBy": { "kind": "if_statement" } },
        });
        assert_eq!(stop_rule_texts(reached, source), ["x"]);
    }

    #[test]
    fn has_with_a_field_starts_at_the_field_child() {
        let source = "function f() {}\nfunction g() { a(); }\nfunction h() { { b(); } }\n";
        let empty = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "regex": "^\\{\\s*\\}$" },
        });
        assert_eq!(stop_rule_texts(empty, source), ["function f() {}"]);
        // `end` walks the field child and its descendants.
        let end = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "stopBy": "end", "kind": "call_expression" },
        });
        assert_eq!(
            stop_rule_texts(end, source),
            ["function g() { a(); }", "function h() { { b(); } }"]
        );
        // The field child itself is a candidate under `end`, as upstream's `dfs` includes it.
        let own = json!({
            "kind": "call_expression",
            "has": { "field": "function", "stopBy": "end", "kind": "identifier" },
        });
        assert_eq!(stop_rule_texts(own, "a(b);\n"), ["a(b)"]);
        // With a stop rule upstream tests the field child, then only its children: one level.
        let quirk = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "stopBy": { "kind": "return_statement" }, "kind": "expression_statement" },
        });
        assert_eq!(stop_rule_texts(quirk, source), ["function g() { a(); }"]);
        // A field child the stop rule matches is tested but not entered.
        let stop_at_body = json!({
            "kind": "function_declaration",
            "has": { "field": "body", "stopBy": { "kind": "statement_block" }, "kind": "expression_statement" },
        });
        assert_eq!(stop_rule_texts(stop_at_body, source), Vec::<String>::new());
        // A node without the field has nothing to test.
        let missing = json!({ "kind": "arrow_function", "has": { "field": "parameters", "kind": "formal_parameters" } });
        assert_eq!(
            stop_rule_texts(missing, "const k = x => x;\nconst l = (y) => y;\n"),
            ["(y) => y"]
        );
    }

    #[test]
    fn fields_are_checked_per_grammar_and_refused_elsewhere() {
        for (rule, expected) in [
            (
                json!({ "kind": "identifier", "precedes": { "kind": "identifier", "field": "name" } }),
                "/clauses/0/forbid/ast/rule/precedes/field: field is only supported in has/inside",
            ),
            (
                json!({ "kind": "identifier", "follows": { "kind": "identifier", "field": "name" } }),
                "/clauses/0/forbid/ast/rule/follows/field: field is only supported in has/inside",
            ),
            (
                json!({ "kind": "identifier", "count": { "has": { "kind": "identifier", "field": "name" }, "min": 1 } }),
                "/clauses/0/forbid/ast/rule/count/has/field: field is not supported in count",
            ),
            (
                json!({ "kind": "identifier", "inside": { "kind": "identifier", "field": "" } }),
                "/clauses/0/forbid/ast/rule/inside/field must be a non-empty string",
            ),
            (
                json!({ "kind": "identifier", "inside": { "kind": "identifier", "field": 3 } }),
                "/clauses/0/forbid/ast/rule/inside/field must be a non-empty string",
            ),
            (
                json!({ "kind": "identifier", "field": "name" }),
                "unknown field `field` at /clauses/0/forbid/ast/rule",
            ),
            (
                json!({ "kind": "identifier", "inside": { "kind": "call_expression", "field": "nme" } }),
                "/clauses/0/forbid/ast/rule/inside/field: `nme` is not a field of this grammar (typescript, tsx, javascript grammars)",
            ),
        ] {
            assert_eq!(
                rejection(compile_typescript_rule(rule.clone())),
                expected,
                "{rule}"
            );
        }
        // Python's fields are Python's: a `yield` in a default is not in the function's body.
        let python = |field: &str| {
            compile_document_with(
                clause_document(json!({
                    "id": "c",
                    "forbid": { "ast": { "language": "python", "rule": {
                        "kind": "yield",
                        "not": { "inside": { "kind": "function_definition", "field": field, "stopBy": "end" } },
                    } } },
                    "violation": violation(),
                })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        let compiled = python("body").unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "def f(x=(yield)):\n    yield 1\nyield 2\n";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        assert_eq!(matched_texts(&result, source), ["yield", "yield 2"]);
        assert!(rejection(python("declarator")).contains("`declarator` is not a field"));
    }
    #[test]
    fn strict_relation_search_recognizes_argument_bindings() {
        let compiled = compile_typescript_rule(json!({
            "kind": "program",
            "has": {
                "stopBy": "end",
                "kind": "call_expression",
                "arguments": { "signature": ["value"], "bind": { "ARG": "value" } }
            }
        }))
        .expect("rule compiles");
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        assert!(may_bind(matcher));
    }

    #[test]
    fn sibling_relations_to_the_end_test_each_sibling_once() {
        const STATEMENTS: usize = 400;
        let statement = |index: usize| {
            if index == STATEMENTS / 2 {
                "b;".to_owned()
            } else {
                format!("a{index};")
            }
        };
        let source = (0..STATEMENTS)
            .map(|index| statement(index) + "\n")
            .collect::<String>();
        for (relation, expected) in [
            (
                "follows",
                (STATEMENTS / 2 + 1..STATEMENTS).collect::<Vec<_>>(),
            ),
            ("precedes", (0..STATEMENTS / 2).collect()),
        ] {
            let rule = json!({ "kind": "expression_statement",
                               relation: { "stopBy": "end", "regex": "^b;$" } });
            reset_candidates_tested();
            assert_eq!(
                stop_rule_texts(rule, &source),
                expected.into_iter().map(statement).collect::<Vec<_>>(),
                "{relation}"
            );
            // Every statement walking every sibling before it tests 80,000 of them.
            assert!(
                candidates_tested() <= 2 * STATEMENTS,
                "{relation}: {} siblings tested",
                candidates_tested()
            );
        }
    }

    #[test]
    fn a_walk_over_passed_over_candidates_notices_the_deadline() {
        use crate::code_rule_dsl::budget::{self, MatchBudget};
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_core::tree_sitter::LanguageExt;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let compiled = compile_typescript_rule(json!({ "kind": "expression_statement",
            "precedes": { "stopBy": "end", "kind": "class_declaration" } }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let grammar = &grammar_matchers(&compiled)[0];
        let source = "a;\n".repeat(5_000);
        let root = GrammarDialect::TypeScript.ast_grep(&source);
        let first = root.root().child(0).expect("a statement");
        let deadline = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        // This poll spares the next calls theirs.
        assert!(!budget::matcher_call_budget_exhausted());
        budget::pass_deadline_unpolled();
        let mut work = 1_000_000;
        assert!(
            grammar
                .match_root(first, &mut work)
                .expect("fits")
                .is_none()
        );
        assert!(
            deadline.observed(),
            "the walk ran past the deadline unnoticed"
        );
    }

    #[test]
    fn has_to_the_end_finds_a_needle_in_nested_candidates() {
        // Nested calls' texts overlap; each is tested where its own text holds `aa`.
        let source = "p(q(aa), r(a, a), s(t(xaaay)), aa(u));\n";
        let rule = json!({ "kind": "call_expression",
            "has": { "stopBy": "end", "kind": "call_expression", "regex": "aa" } });
        assert_eq!(
            stop_rule_texts(rule, source),
            ["p(q(aa), r(a, a), s(t(xaaay)), aa(u))", "s(t(xaaay))"]
        );
    }

    /// A TypeScript clause of `rule`, evaluated on `source` with node-local relations' answers
    /// remembered for the file or, when `off`, walked again each time (`set_answers_off`): the
    /// result, and the budget units the evaluation spent.
    fn values_clause(rule: &Value, source: &str, off: bool) -> (Value, u64) {
        use crate::code_rule_dsl::budget::MatchBudget;
        use crate::code_rule_dsl::matcher::set_answers_off;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let compiled = compile_document_with(
            clause_document(json!({
                "id": "values",
                "forbid": { "ast": { "language": "typescript", "rule": rule } },
                "violation": violation(),
            })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        set_answers_off(off);
        let budget = MatchBudget::work(&Arc::new(AtomicBool::new(false)), u64::MAX);
        let result = evaluate_file(&compiled, "typescript", "values.ts", source);
        let spent = budget.spent();
        drop(budget);
        set_answers_off(false);
        assert_eq!(result["failures"], json!([]), "{rule}: {result}");
        (result, spent)
    }

    /// `split-literal-separator`'s `has` to the end over a `string` whose value is `','`
    /// remembers each node's answer for the file: nested calls cost work linear in their depth,
    /// where a walk over every call's subtree costs its square. Both report every call that holds
    /// the separator.
    #[test]
    fn value_relations_grow_linearly_with_call_depth() {
        let rule = json!({
            "kind": "call_expression",
            "has": { "stopBy": "end", "kind": "string", "value": { "eq": "," } },
        });
        let found = |result: &Value| result["violations"].as_array().map(Vec::len);
        let mut units = Vec::new();
        for depth in [100, 200, 400] {
            let nested = |bottom: &str| {
                format!(
                    "{}g({bottom}){}",
                    "f('.', ".repeat(depth),
                    ")".repeat(depth)
                )
            };
            let source = format!(
                "export const a = {};\nexport const b = {};\n",
                nested("','"),
                nested("'.'")
            );
            let (remembered, spent) = values_clause(&rule, &source, false);
            assert_eq!(found(&remembered), Some(depth + 1), "depth {depth}");
            let (walked, walk_spent) = values_clause(&rule, &source, true);
            assert_eq!(walked, remembered, "depth {depth}, every subtree walked");
            units.push((spent, walk_spent));
        }
        for pair in units.windows(2) {
            let ((before, _), (after, _)) = (pair[0], pair[1]);
            assert!(
                after <= before * 9 / 4,
                "twice the depth spends at most about twice the units: {units:?}"
            );
        }
        let ((_, walked_200), (remembered_400, walked_400)) = (units[1], units[2]);
        assert!(
            walked_400 >= walked_200 * 3 && walked_400 >= remembered_400 * 4,
            "the walk over every call's subtree grows with the square: {units:?}"
        );
    }

    /// Remembered answers of `value` tests along every relation to the end give the violations
    /// walking again gives, under `not`, `any` and `all`, where values are known, unknown (a
    /// parameter, a call) and followed through bindings.
    #[test]
    fn remembered_value_relations_match_walks() {
        let source = "\
const a = 1, b = 'x', c = ',';
let u = g();
f(a, ',');
f(b, h(c, u));
s.split(',');
s.split(c);
q(u, [1, 2, u ? 3 : 4]);
if (x) { k(0); k(u + 3); }
function w(p: string) { return p.split(u, 2) + 1; }
const z = cond ? f(1, 'a') : f(u, 0);
m(m(m(',', p), u), 'b');
n(u + 1, n(1 + 1, [n(2 + 2)]), n(u * 2));
r([1, u, 2, 3], [4, 5, u, 6]);
";
        let rules = [
            json!({ "kind": "call_expression", "has": { "stopBy": "end", "kind": "string", "value": { "eq": "," } } }),
            json!({ "kind": "call_expression", "has": { "stopBy": "end", "value": { "eq": ",", "follow": true } } }),
            json!({ "kind": "call_expression", "not": { "has": { "stopBy": "end", "value": { "eq": 1, "follow": true } } } }),
            json!({ "kind": "call_expression", "not": { "has": { "stopBy": "end", "kind": "binary_expression", "value": { "eq": 5 } } } }),
            json!({ "kind": "identifier", "inside": { "stopBy": "end", "kind": "call_expression",
                    "has": { "kind": "arguments", "has": { "value": { "truthy": true } } } } }),
            json!({ "kind": "identifier", "not": { "inside": { "kind": "arguments", "has": { "value": { "constant": true, "follow": true } } } } }),
            json!({ "kind": "number", "precedes": { "stopBy": "end", "value": { "type": "string", "follow": true } } }),
            json!({ "kind": "identifier", "not": { "follows": { "stopBy": "end", "not": { "value": { "constant": true, "follow": true } } } } }),
            json!({ "kind": "number", "follows": { "value": { "gt": 0 } } }),
            json!({ "kind": "number", "not": { "precedes": { "stopBy": "end", "value": { "eq": 9 } } } }),
            json!({ "kind": "number", "not": { "follows": { "stopBy": "end", "value": { "eq": 9 } } } }),
            json!({ "kind": "number", "not": { "inside": { "stopBy": "end", "kind": "call_expression", "has": { "kind": "arguments", "has": { "value": { "eq": 9 } } } } } }),
            json!({ "kind": "expression_statement", "any": [
                { "has": { "kind": "call_expression", "has": { "kind": "arguments", "has": { "value": { "eq": 0 } } } } },
                { "has": { "stopBy": "end", "kind": "identifier", "value": { "gt": 2, "follow": true } } },
            ] }),
            json!({ "kind": "call_expression", "all": [
                { "has": { "stopBy": "end", "kind": "string" } },
                { "not": { "has": { "stopBy": "end", "kind": "identifier", "value": { "type": "string", "follow": true } } } },
            ] }),
        ];
        let mut reported = 0;
        for rule in &rules {
            let found = |off: bool| matched_texts(&values_clause(rule, source, off).0, source);
            let (remembered, walked) = (found(false), found(true));
            assert_eq!(remembered, walked, "{rule}");
            reported += remembered.len();
        }
        assert!(reported > 20, "the rules report findings: {reported}");
    }
