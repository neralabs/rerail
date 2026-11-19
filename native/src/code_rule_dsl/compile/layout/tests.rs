    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    #[test]
    fn malformed_layouts_are_rejected() {
        let reject = |layout: Value| {
            rejection(compile_typescript_rule(
                json!({ "kind": "identifier", "layout": layout }),
            ))
        };
        let at = "/clauses/0/forbid/ast/rule/layout";
        for (layout, expected) in [
            (
                json!({}),
                format!("{at} must hold span, start, before or after"),
            ),
            (
                json!({ "lines": { "eq": 0 } }),
                format!("unknown field `lines` at {at}"),
            ),
            (json!({ "span": 1 }), format!("{at}/span must be an object")),
            (
                json!({ "span": {} }),
                format!("{at}/span must set eq, min or max"),
            ),
            (
                json!({ "span": { "eq": -1 } }),
                format!("{at}/span/eq must be a non-negative integer"),
            ),
            (
                json!({ "span": { "eq": 1, "max": 2 } }),
                format!("{at}/span: eq cannot be combined with min or max"),
            ),
            (
                json!({ "start": { "min": 3, "max": 2 } }),
                format!("{at}/start: min must not exceed max"),
            ),
            (
                json!({ "before": { "breaks": { "eq": 0 } } }),
                format!("{at}/before/to must be \"token\", \"sibling\" or \"capture.<NAME>\""),
            ),
            (
                json!({ "before": { "to": "parent", "breaks": { "eq": 0 } } }),
                format!("{at}/before/to must be \"token\", \"sibling\" or \"capture.<NAME>\""),
            ),
            (
                json!({ "before": { "to": "capture.$A", "breaks": { "eq": 0 } } }),
                format!("{at}/before/to: `$A` is not a capture name (A-Z, 0-9, _, without `$`)"),
            ),
            (
                json!({ "after": { "to": "token" } }),
                format!("{at}/after must count breaks, blank or both"),
            ),
            (
                json!({ "after": { "to": "token", "comments": 1, "breaks": { "eq": 0 } } }),
                format!("{at}/after/comments must be a boolean"),
            ),
            (
                json!({ "after": { "to": "capture.A", "edge": true, "breaks": { "eq": 0 } } }),
                format!(
                    "{at}/after/edge: edge chooses the neighbouring token or sibling; a capture names its node"
                ),
            ),
            (
                json!({ "after": { "to": "token", "lines": 1, "breaks": { "eq": 0 } } }),
                format!("unknown field `lines` at {at}/after"),
            ),
        ] {
            assert_eq!(reject(layout.clone()), expected, "{layout}");
        }
    }

    #[test]
    fn a_gap_runs_to_a_single_capture_bound_where_it_runs() {
        let reject = |ast: Value| rejection(compile_typescript_ast(ast));
        let gap = |name: &str| json!({ "to": format!("capture.{name}"), "breaks": { "eq": 0 } });
        assert_eq!(
            reject(
                json!({ "rule": { "pattern": "f($A)", "not": { "pattern": "g($B)" }, "layout": { "before": gap("B") } } })
            ),
            "/clauses/0/forbid/ast/rule/layout/before/to: `$B` is never bound where this layout runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        assert_eq!(
            reject(json!({ "rule": { "pattern": "f($$$A)", "layout": { "after": gap("A") } } })),
            "/clauses/0/forbid/ast/rule/layout/after/to: `$$$A` captures a node list; a gap runs to a single node"
        );
        assert_eq!(
            reject(
                json!({ "rule": { "pattern": "f($A)", "layout": { "after": gap("T") } },
                "transform": { "T": { "substring": { "source": "$A", "startChar": 1 } } } })
            ),
            "/clauses/0/forbid/ast/rule/layout/after/to: `$T` is a transform variable, text; to reads a node"
        );
        compiled(
            json!({ "rule": { "kind": "identifier", "follows": { "pattern": "$P" }, "layout": { "before": gap("P") } } }),
        );
        // A parsed tree has no lines of its own.
        let python = clause_document(
            json!({ "id": "c", "violation": violation(), "forbid": { "ast": { "language": "python",
            "rule": { "kind": "string", "parse": { "grammar": "percentFormat", "has": { "kind": "placeholder", "layout": { "span": { "eq": 0 } } } } } } } }),
        );
        assert_eq!(
            rejection(compile_document_with(
                python,
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities")
            )),
            "/clauses/0/forbid/ast/rule/parse/has/layout: `layout` is not supported inside parse (percentFormat trees)"
        );
    }
