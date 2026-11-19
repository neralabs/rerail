    use super::PhpstanIndex;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::language::adapter::ast_grep;
    use crate::code_rule_dsl::lines::{LineTable, Terminators};
    use crate::code_rule_dsl::fixtures::oracle;
    use serde_json::Value;

    const ORACLE: &str = oracle!("php/suppress/phpstan.expected.json");

    fn reports(file: &Value, key: &str) -> Vec<(String, usize)> {
        file[key]
            .as_array()
            .expect("reports")
            .iter()
            .map(|report| {
                (
                    report[0].as_str().expect("identifier").to_owned(),
                    report[1].as_u64().expect("start") as usize,
                )
            })
            .collect()
    }

    #[test]
    fn phpstan_comments_suppress_as_phpstan_honours_them() {
        let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
        assert_eq!(
            oracle["phpstan"],
            "PHPStan - PHP Static Analysis Tool 2.2.16"
        );
        for (name, source) in [
            ("ignores.php", oracle!("php/suppress/phpstan/ignores.php")),
            (
                "terminators.php",
                oracle!("php/suppress/phpstan/terminators.php"),
            ),
            ("tokens.php", oracle!("php/suppress/phpstan/tokens.php")),
        ] {
            // The tree the engine reads: the PHP view's (inline HTML and mid-file tags blanked),
            // its texts the file's.
            let root = ast_grep(GrammarDialect::Php, source);
            let lines = LineTable::new(source, Terminators::Newlines);
            let index = PhpstanIndex::read(&root.root(), source, &lines);
            // One PHPStan run's errors are one kind: they spend the same listed identifiers.
            let ours = reports(&oracle[name], "all")
                .into_iter()
                .filter(|(identifier, at)| {
                    !index.suppresses("phpstan", std::slice::from_ref(identifier), *at, &lines)
                })
                .collect::<Vec<_>>();
            // `ignore.*` reports are PHPStan's about the comments themselves.
            let kept = reports(&oracle[name], "kept")
                .into_iter()
                .filter(|(identifier, _)| !identifier.starts_with("ignore."))
                .collect::<Vec<_>>();
            assert_eq!(ours, kept, "{name}");
        }
    }
