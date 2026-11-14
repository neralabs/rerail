    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    fn document(language: &str, suppress: Value) -> Value {
        let mut violation = violation();
        violation["suppress"] = suppress;
        let kind = if language == "php" {
            "variable_name"
        } else {
            "identifier"
        };
        clause_document(json!({ "id": "c", "violation": violation,
            "forbid": { "ast": { "language": language, "rule": { "kind": kind } } } }))
    }

    fn compile(document: Value) -> napi::Result<crate::code_rule_dsl::CompiledCodeRuleDsl> {
        compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    /// The registered kinds, in order, each named as the registry names it: its key (the generated
    /// `Family::honours` matches on it).
    #[test]
    fn every_kind_is_one_row_named_as_the_registry_names_it() {
        use super::Suppression;
        let keys = Suppression::all()
            .iter()
            .map(|kind| kind.key())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "eslint", "noqa", "nosec", "oxlint", "biome", "phpstan", "psalm", "phpcs",
                "clippy", "sqlfluff"
            ]
        );
        for kind in Suppression::all() {
            let key = kind.key();
            assert_eq!(format!("{kind:?}"), key);
            assert!(kind.detail().starts_with(&format!("{key}: ")), "{key}");
        }
    }

    #[test]
    fn suppress_is_checked_and_opt_in_per_linter() {
        let at = "/clauses/0/violation/suppress";
        for (language, suppress, expected) in [
            (
                "typescript",
                json!({}),
                format!(
                    "{at} must hold eslint, noqa, nosec, oxlint, biome, phpstan, psalm, phpcs, clippy, or sqlfluff"
                ),
            ),
            (
                "typescript",
                json!({ "jshint": {} }),
                format!("unknown field `jshint` at {at}"),
            ),
            (
                "typescript",
                json!({ "eslint": {} }),
                format!("{at}/eslint/rules must be a list of 1 to 256 rule names"),
            ),
            (
                "typescript",
                json!({ "eslint": { "rules": [] } }),
                format!("{at}/eslint/rules must be a list of 1 to 256 rule names"),
            ),
            (
                "typescript",
                json!({ "eslint": { "rules": ["a, b"] } }),
                format!(
                    "{at}/eslint/rules/0 must be a rule name (non-empty, without commas or surrounding white space)"
                ),
            ),
            (
                "typescript",
                json!({ "eslint": { "rules": ["strict"], "all": true } }),
                format!("unknown field `all` at {at}/eslint"),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["f401"] } }),
                format!(
                    "{at}/noqa/codes/0 must be a rule code (uppercase letters, then digits: F401)"
                ),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["F"] } }),
                format!(
                    "{at}/noqa/codes/0 must be a rule code (uppercase letters, then digits: F401)"
                ),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["F401"], "parent": "import" } }),
                format!("{at}/noqa/parent must be \"statement\""),
            ),
            (
                "python",
                json!({ "nosec": { "codes": ["b101"] } }),
                format!("{at}/nosec/codes/0 must be a Bandit test id (B, then digits: B101)"),
            ),
            (
                "python",
                json!({ "nosec": { "codes": ["B101"], "names": ["assert_used"] } }),
                format!("unknown field `names` at {at}/nosec"),
            ),
            (
                "typescript",
                json!({ "nosec": { "codes": ["B101"] } }),
                format!("{at}/nosec: nosec comments are Bandit's; a typescript query has none"),
            ),
            (
                "php",
                json!({ "nosec": { "codes": ["B101"] } }),
                format!("{at}/nosec: nosec comments are Bandit's; a php query has none"),
            ),
            (
                "typescript",
                json!({ "oxlint": { "rules": [] } }),
                format!("{at}/oxlint/rules must be a list of 1 to 256 oxlint rule names"),
            ),
            (
                "typescript",
                json!({ "oxlint": { "rules": ["react-hooks/rules-of-hooks"] } }),
                format!(
                    "{at}/oxlint/rules/0 must be an oxlint rule name (non-empty, without a plugin prefix, commas or white space: rules-of-hooks)"
                ),
            ),
            (
                "typescript",
                json!({ "oxlint": { "rules": ["no console"] } }),
                format!(
                    "{at}/oxlint/rules/0 must be an oxlint rule name (non-empty, without a plugin prefix, commas or white space: rules-of-hooks)"
                ),
            ),
            (
                "typescript",
                json!({ "oxlint": { "rules": ["no-console"], "plugins": ["eslint"] } }),
                format!("unknown field `plugins` at {at}/oxlint"),
            ),
            (
                "typescript",
                json!({ "biome": { "rules": [] } }),
                format!("{at}/biome/rules must be a list of 1 to 256 Biome lint rule names"),
            ),
            (
                "typescript",
                json!({ "biome": { "rules": ["correctness/useHookAtTopLevel"] } }),
                format!(
                    "{at}/biome/rules/0 must be a lint rule of Biome 2.5.15's JavaScript analyzer (lint/<group>/<rule>: lint/correctness/useHookAtTopLevel)"
                ),
            ),
            (
                "typescript",
                json!({ "biome": { "rules": ["lint/correctness"] } }),
                format!(
                    "{at}/biome/rules/0 must be a lint rule of Biome 2.5.15's JavaScript analyzer (lint/<group>/<rule>: lint/correctness/useHookAtTopLevel)"
                ),
            ),
            (
                "typescript",
                json!({ "biome": { "rules": ["lint/a11y/useGenericFontNames"] } }),
                format!(
                    "{at}/biome/rules/0 must be a lint rule of Biome 2.5.15's JavaScript analyzer (lint/<group>/<rule>: lint/correctness/useHookAtTopLevel)"
                ),
            ),
            (
                "typescript",
                json!({ "biome": { "rules": ["lint/correctness/useHookAtTopLevel"], "instance": "x" } }),
                format!("unknown field `instance` at {at}/biome"),
            ),
            (
                "python",
                json!({ "biome": { "rules": ["lint/suspicious/noConsole"] } }),
                format!("{at}/biome: biome-ignore comments are Biome's; a python query has none"),
            ),
            (
                "php",
                json!({ "biome": { "rules": ["lint/suspicious/noConsole"] } }),
                format!("{at}/biome: biome-ignore comments are Biome's; a php query has none"),
            ),
            (
                "python",
                json!({ "oxlint": { "rules": ["no-console"] } }),
                format!(
                    "{at}/oxlint: oxlint-disable comments are Oxlint's; a python query has none"
                ),
            ),
            (
                "php",
                json!({ "oxlint": { "rules": ["no-console"] } }),
                format!("{at}/oxlint: oxlint-disable comments are Oxlint's; a php query has none"),
            ),
            (
                "python",
                json!({ "eslint": { "rules": ["strict"] } }),
                format!(
                    "{at}/eslint: eslint-disable comments are ESLint's; a python query has none"
                ),
            ),
            (
                "typescript",
                json!({ "noqa": { "codes": ["F401"] } }),
                format!("{at}/noqa: noqa comments are Ruff's; a typescript query has none"),
            ),
            (
                "php",
                json!({ "phpstan": { "identifiers": ["echo."] } }),
                format!(
                    "{at}/phpstan/identifiers/0 must be a PHPStan identifier (letters or digits, with internal dots)"
                ),
            ),
            (
                "php",
                json!({ "eslint": { "rules": ["strict"] } }),
                format!("{at}/eslint: eslint-disable comments are ESLint's; a php query has none"),
            ),
            (
                "php",
                json!({ "noqa": { "codes": ["F401"] } }),
                format!("{at}/noqa: noqa comments are Ruff's; a php query has none"),
            ),
            (
                "python",
                json!({ "phpstan": { "identifiers": ["variable.undefined"] } }),
                format!(
                    "{at}/phpstan: @phpstan-ignore comments are PHPStan's; a python query has none"
                ),
            ),
            (
                "typescript",
                json!({ "phpstan": { "identifiers": ["variable.undefined"] } }),
                format!(
                    "{at}/phpstan: @phpstan-ignore comments are PHPStan's; a typescript query has none"
                ),
            ),
            (
                "php",
                json!({ "psalm": { "issues": ["all"] } }),
                format!(
                    "{at}/psalm/issues/0 must be a Psalm issue name (letters, digits, `_` or `-`; `all` is suppressAll)"
                ),
            ),
            (
                "php",
                json!({ "psalm": { "issues": ["UndefinedVariable"], "suppressAll": "yes" } }),
                format!("{at}/psalm/suppressAll must be a boolean"),
            ),
            (
                "php",
                json!({ "phpcs": { "sniffs": ["Squiz.PHP"] } }),
                format!(
                    "{at}/phpcs/sniffs/0 must be a PHP_CodeSniffer sniff or message code (Standard.Category.Sniff or Standard.Category.Sniff.Code)"
                ),
            ),
            (
                "php",
                json!({ "phpcs": { "sniffs": ["Squiz.PHP.DiscouragedFunctions"], "version": 2 } }),
                format!("{at}/phpcs/version must be 3 or 4"),
            ),
            (
                "rust",
                json!({ "clippy": { "lints": ["clippy::all"], "msrv": "1" } }),
                format!(
                    "{at}/clippy/msrv must be a Rust version, `major.minor` or `major.minor.patch`"
                ),
            ),
            (
                "python",
                json!({ "psalm": { "issues": ["UndefinedVariable"] } }),
                format!("{at}/psalm: @psalm-suppress tags are Psalm's; a python query has none"),
            ),
            (
                "typescript",
                json!({ "phpcs": { "sniffs": ["Squiz.PHP.DiscouragedFunctions"] } }),
                format!(
                    "{at}/phpcs: phpcs: annotations are PHP_CodeSniffer's; a typescript query has none"
                ),
            ),
        ] {
            assert_eq!(
                rejection(compile(document(language, suppress.clone()))),
                expected,
                "{suppress}"
            );
        }
        let mut regex = document("typescript", json!({ "eslint": { "rules": ["strict"] } }));
        regex["clauses"][0]["forbid"] =
            json!({ "regex": { "source": "file.content", "pattern": "x" } });
        assert_eq!(
            rejection(compile(regex)),
            format!("{at}: suppress removes an ast query's violations")
        );
        for (language, suppress) in [
            (
                "typescript",
                json!({ "eslint": { "rules": ["strict", "rule-to-test/strict"] } }),
            ),
            (
                "typescript",
                json!({ "oxlint": { "rules": ["rules-of-hooks"] } }),
            ),
            (
                "javascript",
                json!({
                    "eslint": { "rules": ["react-hooks/rules-of-hooks"] },
                    "oxlint": { "rules": ["rules-of-hooks"] },
                    "biome": { "rules": ["lint/correctness/useHookAtTopLevel"] }
                }),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["F401", "TC002", "TCH002"], "parent": "statement" } }),
            ),
            (
                "python",
                json!({ "noqa": { "codes": ["S101"] }, "nosec": { "codes": ["B101"] } }),
            ),
            (
                "php",
                json!({ "phpstan": { "identifiers": ["variable.undefined", "echo.nonString"] } }),
            ),
            (
                "php",
                json!({
                    "phpstan": { "identifiers": ["variable.undefined"] },
                    "psalm": { "issues": ["UndefinedVariable", "PossiblyUndefinedVariable"], "suppressAll": true },
                    "phpcs": { "sniffs": ["Squiz.PHP.DiscouragedFunctions.Discouraged"], "version": 3 }
                }),
            ),
        ] {
            compile(document(language, suppress))
                .unwrap_or_else(|error| panic!("{}", error.reason));
        }
    }
