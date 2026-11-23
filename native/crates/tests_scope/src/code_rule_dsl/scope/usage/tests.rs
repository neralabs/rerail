    use super::lang_python::{Exports, dunder_all};
    use super::*;
    use crate::code_rule_dsl::scope::ANNOTATION_RANGE;

    #[test]
    fn before_local_keeps_synthetic_annotation_offsets() {
        let usage = Usage {
            facts: Vec::new(),
            sites: Vec::new(),
            exports: Vec::new(),
            comment_exports: BTreeSet::new(),
            occurrences: HashMap::new(),
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local: vec![(ANNOTATION_RANGE, true)],
            cost: 0,
        };
        assert_eq!(usage.before_local(ANNOTATION_RANGE), Some(true));
    }

    /// The module `__all__` definitions of a Python source, read as a rule reads them.
    fn python_exports(source: &str) -> Exports {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let root = GrammarDialect::Python.ast_grep(source);
        let table = ScopeTable::build(root.root(), GrammarDialect::Python, usize::MAX, &|| false)
            .expect("table builds");
        let mut steps = Steps {
            cost: 0,
            cap: usize::MAX,
            deadline: &|| false,
        };
        dunder_all(&table, &root.root(), &mut steps).expect("exports build")
    }

    /// The entry the string literal `text` (quotes included) makes, where it first occurs.
    fn entry(source: &str, text: &str) -> (usize, usize, String) {
        let start = source
            .find(text)
            .unwrap_or_else(|| panic!("{text} in {source:?}"));
        let name = text.trim_matches(|c| c == '\'' || c == '"').to_owned();
        (start, start + text.len(), name)
    }

    fn names(exports: &Exports) -> Vec<&str> {
        exports.names.iter().map(String::as_str).collect()
    }

    /// Ruff's AST has no parentheses, so a parenthesised string entry, however deep, is the string
    /// (`add_to_names` in ruff_python_semantic's `model/all.rs`), recorded at the string's own
    /// range, where the rule's string node is.
    #[test]
    fn python_dunder_all_reads_parenthesised_entries() {
        let source = "__all__: list = ['b', \"c\", ((('a')))]\n__all__ += [(\"d\")]\n";
        let exports = python_exports(source);
        assert!(exports.complete);
        assert_eq!(names(&exports), ["a", "b", "c", "d"]);
        assert_eq!(
            exports.entries,
            [
                entry(source, "'b'"),
                entry(source, "\"c\""),
                entry(source, "'a'"),
                entry(source, "\"d\""),
            ]
        );
    }

    /// A bare tuple on the right of `=`, `+=` or `: T =` (tree-sitter's `expression_list`) is
    /// Ruff's `Expr::Tuple`, read like a tuple display.
    #[test]
    fn python_dunder_all_reads_bare_tuples() {
        let source = "__all__ = (\"a\"), \"b\"\n__all__ += \"d\", \"e\"\n__all__: list = \"f\",\n";
        let exports = python_exports(source);
        assert!(exports.complete);
        assert_eq!(names(&exports), ["a", "b", "d", "e", "f"]);
        assert_eq!(
            exports.entries,
            [
                entry(source, "\"a\""),
                entry(source, "\"b\""),
                entry(source, "\"d\""),
                entry(source, "\"e\""),
                entry(source, "\"f\""),
            ]
        );
    }

    /// A bare tuple's string entries are read and a non-string one makes the exports incomplete
    /// (Ruff's `INVALID_OBJECT`), as in a display.
    #[test]
    fn python_dunder_all_bare_tuple_with_a_non_string_entry_is_incomplete() {
        let source = "__all__ = \"foo\", \"an\" \"implicit\", not_a_string_literal\n";
        let exports = python_exports(source);
        assert!(!exports.complete);
        assert_eq!(names(&exports), ["animplicit", "foo"]);
        assert_eq!(
            exports.entries,
            [
                entry(source, "\"foo\""),
                (
                    source.find("\"an\"").expect("concatenation"),
                    source.find(", not_a").expect("end"),
                    "animplicit".to_owned()
                ),
            ]
        );
    }
