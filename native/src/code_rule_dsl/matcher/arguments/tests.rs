    use super::lang_python::literal_keywords;
    use super::lang_php::PhpArguments as _;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::matcher::util::{UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
    use ast_grep_core::tree_sitter::LanguageExt;

    /// Every case in the PHP oracle (`fixtures/php/matcher/arguments/generate_php.php`): a slot Rerail
    /// finds passed holds the argument PHP binds to it, a slot Rerail finds absent keeps PHP's
    /// default, and a call PHP refuses passes nothing known. Rerail does not know a slot only where
    /// the case's `unknown` names that slot, with why (a spread whose value it does not know).
    /// `callee: "new class"` passes the arguments to an anonymous class's constructor.
    #[test]
    fn php_calls_bind_as_php_binds_them() {
        use super::{Arguments, Passed, SlotKind};
        use crate::code_rule_dsl::oracle;
        use crate::code_rule_dsl::value::{Context, php};
        use serde_json::Value as Json;

        let oracle: Json = serde_json::from_str(oracle!("php/matcher/arguments/php.expected.json"))
            .expect("oracle JSON");
        assert_eq!(oracle["php"].as_str().map(|v| &v[..3]), Some("8.5"));
        let mut failures = Vec::new();
        for case in oracle["cases"].as_array().expect("cases") {
            let call = case["call"].as_str().expect("call");
            let signature = case["signature"].as_array().expect("signature");
            let slots: Vec<_> = signature
                .iter()
                .map(|name| (name.as_str().expect("slot").to_owned(), SlotKind::Either))
                .collect();
            let arguments = Arguments {
                language: GrammarDialect::Php,
                slots,
                bind: Vec::new(),
                absent: Vec::new(),
                call_kinds: Default::default(),
                syntactic: false,
                estree: false,
            };
            let (source, kind) = match case.get("callee").and_then(Json::as_str) {
                None => (format!("f({call});"), "function_call_expression"),
                Some("new class") => (
                    format!("new class({call}) {{}};"),
                    "object_creation_expression",
                ),
                Some(other) => panic!("{call:?}: callee {other:?}"),
            };
            let root = GrammarDialect::Php.ast_grep(&source);
            let node = root
                .root()
                .dfs()
                .find(|node| node.kind() == kind)
                .expect("a call");
            let passed = arguments.php(&node).expect("a call, not a callable");
            if let Some(error) = case.get("error") {
                if !passed.iter().all(|slot| matches!(slot, Passed::Unknown)) {
                    failures.push(format!(
                        "{call:?}: PHP refuses it ({error}), Rerail binds slots"
                    ));
                }
                continue;
            }
            for (index, (slot, expected)) in passed
                .iter()
                .zip(case["slots"].as_array().expect("slots"))
                .enumerate()
            {
                let (agrees, rerail) = match slot {
                    Passed::Absent => (expected[0].as_str() == Some("absent"), "absent"),
                    Passed::Unknown => {
                        let name = signature[index].as_str().expect("slot");
                        (case["unknown"].get(name).is_some(), "unknown")
                    }
                    Passed::Value(node) => {
                        let mut spend = || true;
                        let mut context = Context::new(&mut spend);
                        let value = php::decode(node, &mut context);
                        (
                            expected[0].as_str() != Some("absent")
                                && php::tests::agrees(&value, expected),
                            "passed",
                        )
                    }
                };
                if !agrees {
                    failures.push(format!(
                        "{call:?}: slot {index}: PHP binds {expected}, Rerail finds it {rerail}"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} disagreements:\n{}",
            failures.len(),
            failures.join("\n")
        );
        // `f(...)` is a first-class callable, not a call.
        let root = GrammarDialect::Php.ast_grep("f(...);");
        let node = root
            .root()
            .dfs()
            .find(|node| node.kind() == "function_call_expression")
            .expect("a call");
        let arguments = Arguments {
            language: GrammarDialect::Php,
            slots: vec![("a".into(), SlotKind::Either)],
            bind: Vec::new(),
            absent: Vec::new(),
            call_kinds: Default::default(),
            syntactic: false,
            estree: false,
        };
        assert!(arguments.php(&node).is_none());
    }

    #[test]
    fn dictionary_splat_key_decoding_uses_the_value_budget() {
        let root = GrammarDialect::Python.ast_grep("f(**{'a': 1})");
        let splat = root
            .root()
            .dfs()
            .find(|node| node.kind() == "dictionary_splat")
            .expect("dictionary splat");
        let saved_steps = UTIL_STEPS_LEFT.replace(0);
        let saved_exceeded = UTIL_EXCEEDED.replace(None);
        assert!(literal_keywords(&splat).is_none());
        assert_eq!(UTIL_EXCEEDED.get(), Some(UtilLimit::Value));
        UTIL_STEPS_LEFT.set(saved_steps);
        UTIL_EXCEEDED.set(saved_exceeded);
    }
