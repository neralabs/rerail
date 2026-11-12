    use super::*;
    use crate::code_rule_dsl::fixtures::oracle;

    fn kinds(text: &str, parenthesized: bool) -> Option<String> {
        fn describe(node: &PNode, text: &str) -> String {
            let children: Vec<String> = node
                .children
                .iter()
                .filter(|child| child.named)
                .map(|child| describe(child, text))
                .collect();
            if children.is_empty() {
                format!("{}:{}", kind_name(node.kind), &text[node.start..node.end])
            } else {
                format!("{}({})", kind_name(node.kind), children.join(" "))
            }
        }
        expression(text, parenthesized).map(|root| describe(&root, text))
    }

    #[test]
    fn annotations_are_expressions_as_ruff_reads_them() {
        let statement = |inner: &str| format!("expression_statement({inner})");
        assert_eq!(
            kinds("List[int]", false),
            Some(statement("subscript(identifier:List identifier:int)"))
        );
        assert_eq!(kinds("int ", false), Some(statement("identifier:int")));
        assert_eq!(kinds("int # x", false), Some(statement("identifier:int")));
        assert_eq!(
            kinds("a, b", false).as_deref(),
            Some("expression_statement(identifier:a identifier:b)")
        );
        assert_eq!(
            kinds("\nint |\n  str", true),
            Some(statement("binary_operator(identifier:int identifier:str)"))
        );
        assert_eq!(kinds(" int", true), Some(statement("identifier:int")));
        assert_eq!(
            kinds("(int)", true),
            Some(statement("parenthesized_expression(identifier:int)"))
        );
        for refused in [
            " int",
            "async",
            "print x",
            "a b",
            "(int",
            "int\\",
            "x := 1",
            "*a",
            "yield x",
            "await",
            "await await x",
            "f(yield x)",
            "",
            " ",
            "1 +",
            "a if b",
            "(a) or (b",
            "int |\nstr",
            "a = 1",
        ] {
            assert_eq!(kinds(refused, false), None, "{refused:?}");
        }
        assert_eq!(
            kinds("lambda: 1", false),
            Some(statement("lambda(integer:1)"))
        );
        assert_eq!(kinds("match", false), Some(statement("identifier:match")));
        assert_eq!(
            kinds("await x", false),
            Some(statement("await(identifier:x)"))
        );
        assert_eq!(
            kinds("(yield)", false),
            Some(statement("parenthesized_expression(yield:yield)"))
        );
    }

    /// Every case of `fixtures/python/literal/annotation.expected.json`, written in its host and read through the
    /// scope table: a refused literal where Ruff 0.16.9's parser reports F722 on the whole
    /// literal, and otherwise an `ERROR` node after the expression for each semantic syntax error
    /// its checker reports, at the same range with the same message. In a complex (escaped)
    /// string Ruff relocates every expression to the whole literal but leaves a lambda
    /// parameter's, a keyword argument's and a comprehension clause's range at its offset in the
    /// value, which it then reads as a file offset; the tree maps every node to the whole
    /// literal, so the escaped cases' reports are read there.
    #[test]
    fn annotations_report_what_ruff_reports() {
        use super::super::SubDoc;
        use crate::code_rule_dsl::scope::{Reading, ScopeTable};
        use ast_grep_core::AstGrep;
        use std::sync::Arc;
        let oracle: serde_json::Value =
            serde_json::from_str(oracle!("python/literal/annotation.expected.json"))
                .expect("oracle");
        let cases = oracle["cases"].as_array().expect("cases");
        assert!(cases.len() > 2_000);
        let hosts: std::collections::HashMap<&str, &str> = [
            ("module", "x: {}\n"),
            ("function", "def f():\n    x: {}\n"),
            ("async", "async def f():\n    x: {}\n"),
            ("class", "class C:\n    x: {}\n"),
            ("parameter", "async def f(p: {}): pass\n"),
            ("global", "def f():\n    x: {}\n    global a, b\n"),
        ]
        .into_iter()
        .collect();
        let mut mismatches = Vec::new();
        for case in cases {
            let value = case[0].as_str().expect("value");
            let style = case[1].as_str().expect("style");
            let literal = case[2].as_str().expect("literal");
            let host = case[3].as_str().expect("host");
            let template = hosts[host];
            let source = template.replace("{}", literal);
            let at = template.find("{}").expect("placeholder");
            let whole = literal.len() as i64;
            let mut theirs: Vec<String> = case[4]
                .as_array()
                .expect("reports")
                .iter()
                .map(|report| {
                    let (mut start, mut end) =
                        (report[0].as_i64().unwrap(), report[1].as_i64().unwrap());
                    if style == "escaped" {
                        (start, end) = (0, whole);
                    }
                    format!("{start}..{end} {}", report[2].as_str().unwrap())
                })
                .collect();
            let root = GrammarDialect::Python.ast_grep(&source);
            let table = ScopeTable::build_with(
                root.root(),
                GrammarDialect::Python,
                usize::MAX,
                &|| false,
                Reading::default(),
            )
            .expect("table builds");
            let Some(annotation) = table.annotations.iter().find(|found| found.start == at) else {
                mismatches.push(format!("{host} {style} {value:?}: no type definition"));
                continue;
            };
            let tree = Arc::clone(&annotation.tree);
            let mut ours: Vec<String> = if tree.is_error() {
                vec![format!("0..{whole} syntax")]
            } else {
                let document = AstGrep::doc(SubDoc {
                    tree: Arc::clone(&tree),
                });
                document
                    .root()
                    .children()
                    .filter(|child| child.kind() == "ERROR")
                    .map(|child| {
                        let range = crate::code_rule_dsl::origin::Origin::of_node(
                            &*tree,
                            child.node_id(),
                            child.range(),
                        )
                        .expect("range")
                        .primary();
                        let message = match tree.value(child.node_id()) {
                            Some(Value::Str(message)) => message.clone(),
                            other => format!("{other:?}"),
                        };
                        format!("{}..{} {message}", range.start - at, range.end - at)
                    })
                    .collect()
            };
            // Ruff's parser reports once, on the whole literal, with its own message.
            if tree.is_error()
                && theirs.len() == 1
                && theirs[0].starts_with(&format!("0..{whole} "))
            {
                theirs = vec![format!("0..{whole} syntax")];
            }
            ours.sort();
            theirs.sort();
            if ours != theirs {
                mismatches.push(format!(
                    "{host} {style} {value:?}: Ruff {theirs:?}, Rerail {ours:?}"
                ));
            }
        }
        assert_eq!(
            mismatches,
            Vec::<String>::new(),
            "\n{}",
            mismatches.join("\n")
        );
    }
