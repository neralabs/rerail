use crate::policy_dsl::json::invalid;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::language::{self, PolicyLanguage};
use crate::policy_dsl::matcher::AstMatcher;
use crate::policy_dsl::model::Capabilities;
use napi::Result;
use serde_json::Value;

/// Grammar dialects a rule language spans, keyed by the file language label that selects each one.
type GrammarDialects = Vec<(&'static str, GrammarDialect)>;

pub(super) fn language_capability(
    name: &str,
    capabilities: &Capabilities,
) -> Result<(GrammarDialects, String)> {
    let languages = capabilities
        .raw
        .get("ast")
        .and_then(|value| value.get("languages"))
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("missing ast language capabilities"))?;
    let grammar = languages
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing {name} language capability")))?
        .to_owned();
    let policy_language = PolicyLanguage::parse(name)
        .ok_or_else(|| invalid(format!("missing {name} language capability")))?;
    let dialects = policy_language
        .dialects()
        .iter()
        .map(|&dialect| {
            (
                language::label(dialect).expect("policy dialect has a file label"),
                dialect,
            )
        })
        .collect();
    Ok((dialects, grammar))
}

pub(super) fn grammar_gap(gaps: &mut Vec<String>, reason: String) -> AstMatcher {
    gaps.push(reason);
    AstMatcher::Never
}

#[cfg(test)]
mod tests {
    use crate::policy_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn jsx_rules_match_tsx_and_stay_inert_for_plain_ecmascript() {
        for rule in [
            json!({ "kind": "jsx_opening_element" }),
            json!({ "pattern": "<button $$$A>$$$B</button>" }),
        ] {
            let compiled = compile_typescript_rule(rule.clone())
                .unwrap_or_else(|error| panic!("{rule} compiles: {}", error.reason));
            assert_findings(&evaluate_file(&compiled, "tsx", "view.tsx", JSX_SOURCE), 1);
            assert_findings(
                &evaluate_file(
                    &compiled,
                    "typescript",
                    "plain.ts",
                    "export const n: number = 1;\n",
                ),
                0,
            );
            assert_findings(
                &evaluate_file(&compiled, "javascript", "plain.js", "const n = 1;\n"),
                0,
            );
        }
    }

    #[test]
    fn rules_every_dialect_rejects_name_each_grammar() {
        let error = compile_typescript_rule(json!({ "kind": "not_a_real_kind" }))
            .err()
            .expect("unknown kind is rejected");
        assert!(
            error.reason.contains("/clauses/0/forbid/ast/rule/kind"),
            "{}",
            error.reason
        );
        assert!(error.reason.contains("not_a_real_kind"), "{}", error.reason);
        assert!(
            error
                .reason
                .ends_with("(typescript, tsx, javascript grammars)"),
            "{}",
            error.reason
        );

        let error = compile_typescript_rule(
            json!({ "kind": "jsx_opening_element", "has": { "kind": "type_assertion" } }),
        )
        .err()
        .expect("a rule mixing JSX and angle-bracket assertions is rejected");
        assert_eq!(
            error.reason,
            "/clauses/0/forbid/ast/rule/kind: Kind `jsx_opening_element` is invalid. (typescript grammar); \
             /clauses/0/forbid/ast/rule/has/kind: Kind `type_assertion` is invalid. (tsx grammar); \
             /clauses/0/forbid/ast/rule/has/kind: Kind `type_assertion` is invalid. (javascript grammar)"
        );
    }

    #[test]
    fn typescript_rules_match_every_ecmascript_dialect_where_expressible() {
        for rule in [
            json!({ "kind": "return_statement" }),
            json!({ "pattern": "const $A: any = $B" }),
        ] {
            let compiled = compile_typescript_rule(rule.clone())
                .unwrap_or_else(|error| panic!("{rule} compiles: {}", error.reason));
            let source = "export function f(x: unknown) {\n  const y: any = x;\n  return y;\n}\n";
            assert_findings(
                &evaluate_file(&compiled, "typescript", "plain.ts", source),
                1,
            );
            assert_findings(&evaluate_file(&compiled, "tsx", "view.tsx", source), 1);
        }
        let compiled = compile_typescript_rule(json!({ "kind": "return_statement" }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_findings(
            &evaluate_file(
                &compiled,
                "javascript",
                "plain.js",
                "export function f(x) { return x; }\n",
            ),
            1,
        );
    }

    #[test]
    fn php_rules_compile_and_match_php_only_sources() {
        let document = check_document(json!({
            "id": "php-match",
            "forbid": {
                "ast": {
                    "language": "php",
                    "rule": { "pattern": "echo $VALUE;" }
                }
            },
            "violation": finding()
        }));
        let capabilities = serde_json::from_str(CAPABILITIES).expect("capabilities");
        let compiled = compile_document_with(document, &capabilities)
            .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_findings(
            &evaluate_file(&compiled, "php", "sample.php", "<?php\necho $value;\n"),
            1,
        );
    }

    #[test]
    fn a_sub_rule_one_dialect_cannot_express_never_matches_there() {
        let plain = "export function f(x: number) {\n  return g(x);\n}\n";
        // Under `not`: a JSX construct cannot occur in a .ts file, so the negation holds.
        let compiled = compile_typescript_rule(json!({
            "kind": "call_expression",
            "not": { "inside": { "kind": "jsx_expression", "stopBy": "end" } },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_findings(
            &evaluate_file(&compiled, "typescript", "plain.ts", plain),
            1,
        );
        let jsx = "export const V = () => <div>{g(1)}</div>;\nh(2);\n";
        assert_findings(&evaluate_file(&compiled, "tsx", "view.tsx", jsx), 1);

        // In one `any` branch: the other branch still works for .ts files.
        let compiled = compile_typescript_rule(json!({
            "any": [{ "kind": "jsx_attribute" }, { "pattern": "g($A)" }],
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_findings(
            &evaluate_file(&compiled, "typescript", "plain.ts", plain),
            1,
        );
        assert_findings(
            &evaluate_file(
                &compiled,
                "tsx",
                "view.tsx",
                "export const V = () => <b id=\"x\">{g(1)}</b>;\n",
            ),
            2,
        );

        // Positive position: inert in the plain dialect, live in tsx.
        let compiled = compile_typescript_rule(json!({
            "kind": "call_expression",
            "inside": { "kind": "jsx_expression", "stopBy": "end" },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_findings(
            &evaluate_file(&compiled, "typescript", "plain.ts", plain),
            0,
        );
        assert_findings(&evaluate_file(&compiled, "tsx", "view.tsx", jsx), 1);

        // A rule no dialect can express in full is still an error naming each dialect.
        let reason = rejection(compile_typescript_rule(json!({
            "any": [{ "kind": "jsx_attribute" }, { "kind": "type_assertion" }],
        })));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/any/0/kind: Kind `jsx_attribute` is invalid. (typescript grammar); \
             /clauses/0/forbid/ast/rule/any/1/kind: Kind `type_assertion` is invalid. (tsx grammar); \
             /clauses/0/forbid/ast/rule/any/1/kind: Kind `type_assertion` is invalid. (javascript grammar)"
        );
    }
}
