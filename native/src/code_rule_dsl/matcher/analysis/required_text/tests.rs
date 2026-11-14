    use super::RequiredText;
    use crate::code_rule_dsl::test_support::{compile_typescript_ast, grammar_matchers};
    use serde_json::{Value, json};

    fn file_text(ast: Value) -> Option<RequiredText> {
        let compiled =
            compile_typescript_ast(ast).unwrap_or_else(|error| panic!("{}", error.reason));
        RequiredText::of(&grammar_matchers(&compiled)[0].matcher)
    }

    fn node_text(ast: Value) -> Option<RequiredText> {
        let compiled =
            compile_typescript_ast(ast).unwrap_or_else(|error| panic!("{}", error.reason));
        RequiredText::of_node(&grammar_matchers(&compiled)[0].matcher)
    }

    #[test]
    fn a_file_needs_what_every_match_needs_through_relations_and_utilities() {
        let text = file_text(json!({
            "rule": {
                "kind": "call_expression",
                "inside": { "stopBy": "end", "regex": "^(alpha|beta)Handler$" },
                "has": { "stopBy": "end", "matches": "named" }
            },
            "utils": { "named": { "pattern": "gamma($A)" } }
        }))
        .expect("a requirement");
        assert!(text.holds("gamma(x); betaHandler"));
        assert!(
            !text.holds("gamma(x); deltaHandler"),
            "an alternative must hold"
        );
        assert!(!text.holds("alphaHandler"), "the utility's leaf must hold");
    }

    #[test]
    fn branches_negations_and_values_need_only_what_every_match_must_hold() {
        // A branch without a literal can match anywhere.
        assert_eq!(
            file_text(
                json!({ "rule": { "any": [{ "regex": "alpha" }, { "kind": "identifier" }] } })
            ),
            None
        );
        // A negation holds where its rule's text is missing.
        assert_eq!(
            file_text(json!({ "rule": { "not": { "regex": "alpha" } } })),
            None
        );
        // Two negations match where their rule does.
        let text = file_text(json!({ "rule": { "not": { "not": { "regex": "alpha" } } } }))
            .expect("a requirement");
        assert!(!text.holds("beta"));
        // Every match of a pattern of unbounded text begins with nothing in particular.
        assert_eq!(file_text(json!({ "rule": { "regex": "^.*$" } })), None);
    }

    #[test]
    fn a_node_needs_only_what_it_and_its_descendants_are_tested_for() {
        let text = node_text(json!({
            "rule": {
                "kind": "call_expression",
                "has": { "stopBy": "end", "regex": "^alpha$" },
                "inside": { "stopBy": "end", "regex": "beta" },
                "follows": { "regex": "gamma" }
            }
        }))
        .expect("a requirement");
        assert!(
            text.holds("alpha()"),
            "ancestors and siblings are elsewhere in the file"
        );
        assert!(!text.holds("delta()"));
    }

    #[test]
    fn recursive_utilities_need_what_their_cycle_reaches() {
        let text = file_text(json!({
            "rule": { "matches": "walk" },
            "utils": {
                "walk": { "any": [
                    { "regex": "^alpha$" },
                    { "kind": "parenthesized_expression", "has": { "matches": "walk" } }
                ] }
            }
        }));
        // The recursive branch reads its own reference back as needing nothing.
        assert_eq!(text, None);
    }
