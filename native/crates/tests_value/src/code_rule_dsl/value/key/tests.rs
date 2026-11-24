    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::fixtures::oracle;
    use ast_grep_core::tree_sitter::LanguageExt;
    use serde_json::Value as Json;
    use std::collections::HashMap;

    const ORACLE: &str = oracle!("ecma/value/keys.expected.json");
    const MEMBERS: &str = oracle!("ecma/value/members.expected.json");
    const NAMES: &str = oracle!("ecma/value/names.expected.json");
    const REFERENCES: &str = oracle!("ecma/value/references.expected.json");

    fn parses_javascript(source: &str) -> bool {
        !GrammarDialect::JavaScript
            .ast_grep(source)
            .root()
            .get_doc()
            .tree
            .root_node()
            .has_error()
    }

    /// Every pair in the isSameReference oracle (`fixtures/ecma/value/generate_references.mjs`) is the same
    /// reference here exactly where ESLint's says so, with static computed keys and without.
    #[test]
    fn same_references_are_eslints() {
        let oracle: Json = serde_json::from_str(REFERENCES).expect("oracle JSON");
        let prefix = oracle["prefix"].as_str().expect("prefix");
        let mut compared = 0;
        for case in oracle["cases"].as_array().expect("cases") {
            let (left, right) = (
                case["left"].as_str().expect("left"),
                case["right"].as_str().expect("right"),
            );
            let source = format!("{prefix}{left}, {right}); }} }}\n");
            for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
                if language == GrammarDialect::JavaScript && !parses_javascript(&source) {
                    continue;
                }
                let root = language.ast_grep(&source);
                let arguments = root
                    .root()
                    .dfs()
                    .find(|node| node.kind() == "arguments")
                    .expect("the call's arguments");
                let [a, b] = arguments
                    .named()
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap_or_else(|_| panic!("{language:?} {source:?} has two arguments"));
                for (static_keys, field) in [(true, "same"), (false, "sameWithoutStaticKeys")] {
                    assert_eq!(
                        same_reference(&a, &b, static_keys),
                        Some(case[field].as_bool().expect("bool")),
                        "{language:?} {left} and {right}, static keys {static_keys}"
                    );
                    if language == GrammarDialect::TypeScript {
                        compared += 1;
                    }
                }
            }
        }
        assert!(compared > 100, "{compared} pairs compared");
    }

    #[test]
    fn oversized_bigints_are_unknown_for_reference_comparison() {
        let source =
            "f(340282366920938463463374607431768211456n, 340282366920938463463374607431768211456n)";
        let root = GrammarDialect::TypeScript.ast_grep(source);
        let arguments = root
            .root()
            .dfs()
            .find(|node| node.kind() == "arguments")
            .expect("the call's arguments");
        let [left, right] = arguments
            .named()
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| panic!("two bigint arguments"));
        assert_eq!(same_reference(&left, &right, true), None);
    }

    /// Every identifier and string literal in the names oracle (`fixtures/ecma/value/generate_names.mjs`) has a
    /// node of the same range here whose name is ESTree's.
    #[test]
    fn node_names_are_estrees() {
        let oracle: Json = serde_json::from_str(NAMES).expect("oracle JSON");
        let mut compared = 0;
        for case in oracle["cases"].as_array().expect("cases") {
            let source = case["src"].as_str().expect("src");
            for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
                if language == GrammarDialect::JavaScript && !parses_javascript(source) {
                    continue;
                }
                let root = language.ast_grep(source);
                let mut names = HashMap::new();
                for node in root.root().dfs() {
                    if let Some(name) = node_name(&node) {
                        let range = node.range();
                        names.entry((range.start, range.end)).or_insert(name);
                    }
                }
                for entry in case["names"].as_array().expect("names") {
                    let range = (
                        entry["start"].as_u64().expect("start") as usize,
                        entry["end"].as_u64().expect("end") as usize,
                    );
                    let expected = entry["name"]
                        .as_array()
                        .expect("name")
                        .iter()
                        .map(|unit| unit.as_u64().expect("unit") as u16)
                        .collect::<Vec<_>>();
                    let text = &source[range.0..range.1];
                    assert_eq!(
                        names.get(&range),
                        Some(&expected),
                        "{language:?} {source:?}: {text} ({})",
                        entry["type"]
                    );
                    if language == GrammarDialect::TypeScript {
                        compared += 1;
                    }
                }
            }
        }
        assert!(compared > 40, "{compared} names compared");
    }

    /// Every key typescript-eslint's getStaticMemberAccessValue names in the oracle
    /// (`fixtures/ecma/value/generate_members.mjs`), read with the module's scope: where it has a name Rerail
    /// has the same or none (not known), and where it has none (or a symbol) Rerail has none.
    #[test]
    fn member_access_names_are_typescript_eslints() {
        use super::super::Context;
        use super::super::ecma::decode;
        use crate::code_rule_dsl::scope::ScopeTable;
        let oracle: Json = serde_json::from_str(MEMBERS).expect("oracle JSON");
        let (mut known, mut unknown) = (0, 0);
        for case in oracle["cases"].as_array().expect("cases") {
            let source = case["src"].as_str().expect("src");
            for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
                if language == GrammarDialect::JavaScript && !parses_javascript(source) {
                    continue;
                }
                let root = language.ast_grep(source);
                let table = ScopeTable::build(root.root(), language, usize::MAX, &|| false)
                    .expect("table builds");
                let mut read = |node: &Node<'_, _>| {
                    let mut spend = || true;
                    let mut global = |range: std::ops::Range<usize>, name: &str| {
                        let reference = table.reference(&range)?;
                        Some(table.resolve(reference, name).is_none())
                    };
                    let mut binding = |range: std::ops::Range<usize>, name: &str| {
                        let reference = table.reference(&range)?;
                        let (binding, value) = table.binding(reference, name)?;
                        Some(binding.followed(value?))
                    };
                    let mut context =
                        Context::with_globals(&mut spend, &mut global).following(&mut binding);
                    decode(node, &mut context)
                };
                let mut names = HashMap::new();
                for node in root.root().dfs() {
                    if let Some(key) = key_node(&node) {
                        let range = key.range();
                        names.insert(
                            (range.start, range.end),
                            member_access_name(&node, &mut read),
                        );
                    }
                }
                for key in case["keys"].as_array().expect("keys") {
                    let range = (
                        key["start"].as_u64().expect("start") as usize,
                        key["end"].as_u64().expect("end") as usize,
                    );
                    let Some(actual) = names.get(&range) else {
                        panic!(
                            "{language:?} {source:?}: no owner for {}",
                            &source[range.0..range.1]
                        );
                    };
                    let expected = key["name"].as_array().map(|units| {
                        units
                            .iter()
                            .map(|unit| unit.as_u64().expect("unit") as u16)
                            .collect::<Vec<_>>()
                    });
                    let text = &source[range.0..range.1];
                    match (&expected, actual) {
                        (_, None) => {
                            if language == GrammarDialect::TypeScript {
                                unknown += 1;
                            }
                        }
                        (Some(expected), Some(actual)) => {
                            assert_eq!(actual, expected, "{language:?} {source:?}: {text}");
                            if language == GrammarDialect::TypeScript {
                                known += 1;
                            }
                        }
                        (None, Some(actual)) => panic!(
                            "{language:?} {source:?}: {text} has no name in typescript-eslint, Rerail reads {}",
                            String::from_utf16_lossy(actual)
                        ),
                    }
                }
            }
        }
        assert!(known > 80, "{known} names known, {unknown} not");
    }

    /// Every key ESLint's getStaticPropertyName reads in the oracle (`fixtures/ecma/value/generate_keys.mjs`)
    /// has an owner here with the same key range, and the same name, or none where ESLint has none;
    /// the key node itself (a computed key's expression without its brackets) has the same name.
    #[test]
    fn property_names_are_eslints() {
        let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
        let mut compared = 0;
        for case in oracle["cases"].as_array().expect("cases") {
            let source = case["src"].as_str().expect("src");
            for language in [GrammarDialect::TypeScript, GrammarDialect::JavaScript] {
                if language == GrammarDialect::JavaScript && !parses_javascript(source) {
                    continue;
                }
                let root = language.ast_grep(source);
                let mut names = HashMap::new();
                for node in root.root().dfs() {
                    if let Some(key) = key_node(&node) {
                        let range = key.range();
                        let name = static_property_name(&node);
                        assert_eq!(
                            static_property_name(&key),
                            name,
                            "{language:?} {source:?}: the key {} itself",
                            key.text()
                        );
                        names.insert((range.start, range.end), name);
                    }
                }
                for key in case["keys"].as_array().expect("keys") {
                    let range = (
                        key["start"].as_u64().expect("start") as usize,
                        key["end"].as_u64().expect("end") as usize,
                    );
                    let expected = key["name"].as_array().map(|units| {
                        units
                            .iter()
                            .map(|unit| unit.as_u64().expect("unit") as u16)
                            .collect::<Vec<_>>()
                    });
                    let Some(actual) = names.get(&range) else {
                        panic!(
                            "{language:?} {source:?}: no owner for {} at {range:?}",
                            &source[range.0..range.1]
                        );
                    };
                    assert_eq!(
                        actual,
                        &expected,
                        "{language:?} {source:?}: {} at {range:?}",
                        &source[range.0..range.1]
                    );
                    if language == GrammarDialect::TypeScript {
                        compared += 1;
                    }
                }
            }
        }
        assert!(compared > 60, "{compared} keys compared");
    }

    #[test]
    fn numbers_print_as_javascript_prints_them() {
        for (value, expected) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (0.1 + 0.2, "0.30000000000000004"),
            (100.0, "100"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
            (9007199254740993.0, "9007199254740992"),
            (255.0 / 256.0, "0.99609375"),
        ] {
            assert_eq!(number_to_string(value), expected, "{value:e}");
        }
    }

    #[test]
    fn strings_read_as_javascript_reads_them() {
        let units = |text: &str| text.encode_utf16().collect::<Vec<_>>();
        let number = |text: &str| string_to_number(&units(text));
        assert_eq!(number(""), Some(0.0));
        assert_eq!(number(" \n12 "), Some(12.0));
        assert_eq!(number("-1.5e3"), Some(-1500.0));
        assert_eq!(number(".5"), Some(0.5));
        assert_eq!(number("5."), Some(5.0));
        assert_eq!(number("0x1F"), Some(31.0));
        assert_eq!(number("-Infinity"), Some(f64::NEG_INFINITY));
        for nan in ["1_0", "0x", "-0x1", "abc", "1e", ".", "infinity", "1 2"] {
            assert!(number(nan).is_some_and(f64::is_nan), "{nan}");
        }
        let big = |text: &str| string_to_bigint(&units(text));
        assert_eq!(big(" -12 "), Some(BigInt::from(-12)));
        assert_eq!(big("0x10"), Some(BigInt::from(16)));
        assert_eq!(big("1.5"), None);
        assert_eq!(big("-0x10"), None);
        assert_eq!(big(""), Some(BigInt::zero()));
        assert_eq!(
            big(&"9".repeat(60)),
            BigInt::parse_bytes("9".repeat(60).as_bytes(), 10)
        );
        assert_eq!(to_int32(4294967297.0), 1);
        assert_eq!(to_int32(2147483648.0), -2147483648);
        assert_eq!(to_int32(-1.9), -1);
        assert_eq!(to_uint32(-1.0), 4294967295);
        assert_eq!(to_uint32(f64::NAN), 0);
    }
