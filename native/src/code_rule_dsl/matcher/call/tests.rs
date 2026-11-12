    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn outputs_mapped_to_one_capture_unify_in_any_parameter_order() {
        // `P` and `Q` both return to `X`: `f(g)` binds them to different nodes, so the call fails
        // whichever parameter is declared or mapped first; `h(h)` binds equal ones.
        let source = "f(g);\nh(h);\n";
        for (params, mapping) in [
            (json!(["P", "Q"]), json!({ "P": "$X", "Q": "$X" })),
            (json!(["Q", "P"]), json!({ "Q": "$X", "P": "$X" })),
        ] {
            let rule = compiled(json!({
                "rule": { "kind": "call_expression", "matches": { "pair": mapping } },
                "utils": { "pair": { "params": params, "rule": { "pattern": "$P($Q)" } } },
            }));
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            assert_eq!(matched_texts(&result, source), ["h(h) X=h"], "{params}");
            // Of the equal outputs, `X` is the one first in the source (the callee).
            let capture = &result["violations"][0]["captures"]["X"][0];
            assert_eq!(capture["startByte"], json!(6), "{params}: {result}");
        }
    }
