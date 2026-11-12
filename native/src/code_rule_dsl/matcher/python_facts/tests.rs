    use super::*;
    use crate::code_rule_dsl::scope::options::DEFAULT_PYTHON_MINOR;
    use ast_grep_language::LanguageExt;

    fn batch(source: &str, module: Option<&ModulePath>, texts: &[&str]) -> Batch {
        let doc = GrammarDialect::Python.ast_grep(source);
        let table =
            ScopeTable::build(doc.root(), GrammarDialect::Python, usize::MAX, &|| false).unwrap();
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        let nodes: Vec<_> = doc
            .root()
            .dfs()
            .filter(|n| {
                matches!(
                    n.kind().as_ref(),
                    "identifier" | "attribute" | "parenthesized_expression" | "call"
                ) && texts.contains(&n.text().as_ref())
            })
            .collect();
        project(
            &table,
            Config {
                module,
                python: &python,
            },
            nodes,
        )
        .unwrap()
    }
    fn at<'a>(facts: &'a Batch, start: usize, text: &str) -> &'a NodeFacts {
        &facts.by_range[&(start, start + text.len())][0]
    }
    fn qualified(row: &NodeFacts) -> &str {
        let Fact::Known(Qualification::Qualified { name, .. }) = &row.qualification else {
            panic!("expected qualified name: {:?}", row.qualification)
        };
        name
    }
    fn bound(row: &NodeFacts) -> &BindingProjection {
        let Fact::Known(Some(binding)) = &row.head_bindings.default else {
            panic!("expected binding: {:?}", row.head_bindings.default)
        };
        binding
    }
    fn text<'a>(source: &'a str, target: Option<Target>) -> &'a str {
        let (start, end) = target.expect("binding target").range;
        &source[start..end]
    }

    #[test]
    fn aliases_shadowing_builtins_and_normalized_attributes() {
        let source = "import os as platform\nfrom pathlib import Path as P\nplatform.path.join('a', 'b')\nP('x')\nlen([])\ndef f(platform):\n    platform.path.join('a', 'b')\n";
        let facts = batch(source, None, &["platform.path.join", "P", "len"]);
        assert_eq!(
            qualified(at(
                &facts,
                source.find("platform.path.join").unwrap(),
                "platform.path.join"
            )),
            "os.path.join"
        );
        assert_eq!(
            qualified(at(&facts, source.rfind("P(").unwrap(), "P")),
            "pathlib.Path"
        );
        let last = at(
            &facts,
            source.rfind("platform.path.join").unwrap(),
            "platform.path.join",
        );
        assert!(matches!(
            last.qualification,
            Fact::Known(Qualification::Local(_))
        ));
        assert!(!last.binding_eligible);
        let len = at(&facts, source.find("len([])").unwrap(), "len");
        assert!(
            matches!(&len.qualification, Fact::Known(Qualification::Unshadowed { name, builtin: true, declared_placeholder: false }) if name == "len")
        );
        let source = "import pkg\n(pkg.K)\n";
        let facts = batch(source, None, &["(pkg.K)"]);
        assert_eq!(
            qualified(at(&facts, source.find("(pkg.K)").unwrap(), "(pkg.K)")),
            "pkg.K"
        );
    }

    #[test]
    fn relative_imports_and_definitions_use_explicit_module_only() {
        let source = "from .helpers import wait as w\ndef run():\n    pass\nw()\nrun()\n";
        let module = ModulePath::dotted("pkg.worker").unwrap();
        let facts = batch(source, Some(&module), &["w", "run"]);
        assert_eq!(
            qualified(at(&facts, source.rfind("w()").unwrap(), "w")),
            "pkg.helpers.wait"
        );
        let run = at(&facts, source.rfind("run()").unwrap(), "run");
        assert_eq!(qualified(run), "pkg.worker.run");
        assert!(matches!(
            run.qualification,
            Fact::Known(Qualification::Qualified {
                origin: QualifiedOrigin::Definition,
                ..
            })
        ));
        let without = batch(source, None, &["w"]);
        assert!(matches!(
            at(&without, source.rfind("w()").unwrap(), "w").qualification,
            Fact::NotApplicable(Reason::MissingModule)
        ));
    }

    #[test]
    fn binding_targets_preserve_assignment_annotation_unpacking_and_with_site() {
        let source = "from pathlib import Path\np: Path = Path('first')\na, b = (Path('a'), Path('b'))\nwith open('f') as handle:\n    handle.read()\np\na\nb\n";
        let facts = batch(source, None, &["p", "a", "b", "handle"]);
        let p = bound(at(&facts, source.rfind("\np\n").unwrap() + 1, "p"));
        assert_eq!(text(source, p.value), "Path('first')");
        assert_eq!(text(source, p.annotation), "Path");
        assert_eq!(text(source, p.site), "p: Path = Path('first')");
        for (name, expected) in [("a", "Path('a')"), ("b", "Path('b')")] {
            let start = source.rfind(&format!("\n{name}\n")).unwrap() + 1;
            assert_eq!(text(source, bound(at(&facts, start, name)).value), expected);
        }
        let handle = bound(at(&facts, source.find("handle.read").unwrap(), "handle"));
        assert_eq!(handle.form, Form::With);
        assert_eq!(text(source, handle.site), "open('f') as handle");
        assert!(handle.value.is_none());
    }

    #[test]
    fn default_and_stop_global_remain_distinct_and_duplicates_are_deduplicated() {
        let source = "x = 1\ndef f():\n    global x\n    return x\n";
        let doc = GrammarDialect::Python.ast_grep(source);
        let table =
            ScopeTable::build(doc.root(), GrammarDialect::Python, usize::MAX, &|| false).unwrap();
        let node = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "identifier" && n.range().start == source.rfind('x').unwrap())
            .unwrap();
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        let facts = project(
            &table,
            Config {
                module: None,
                python: &python,
            },
            [node.clone(), node],
        )
        .unwrap();
        assert_eq!(
            (
                facts.stats.requested,
                facts.stats.distinct,
                facts.stats.qualified_lookups,
                facts.stats.binding_lookups
            ),
            (2, 1, 1, 1)
        );
        let row = at(&facts, source.rfind('x').unwrap(), "x");
        assert_eq!(text(source, bound(row).value), "1");
        let Fact::Known(Some(stopped)) = &row.head_bindings.stop_at_global else {
            panic!("global placeholder")
        };
        assert!(stopped.is_global_declaration);
        assert!(stopped.value.is_none());
        assert_ne!(stopped.id, bound(row).id);
        assert!(matches!(
            row.head_bindings.get(Lookup {
                only: true,
                stop_at_global: false
            }),
            Fact::Unsupported(Reason::LookupNotProjected)
        ));
    }

    #[test]
    fn qualification_eligibility_rejects_non_name_heads_without_reading_scopes() {
        let source = "factory().open()\nhandlers['open']()\nPath('f').open()\n(real.name)()\n";
        let doc = GrammarDialect::Python.ast_grep(source);
        for (callee, eligible) in [
            ("factory().open", false),
            ("handlers['open']", false),
            ("Path('f').open", false),
            ("(real.name)", true),
        ] {
            let node = doc
                .root()
                .dfs()
                .filter(|n| n.kind() == "call")
                .filter_map(|n| n.field("function"))
                .find(|n| n.text() == callee)
                .unwrap();
            let result = qualification_eligibility(&node);
            if eligible {
                assert!(matches!(result, Fact::Known(())));
            } else {
                assert!(matches!(
                    result,
                    Fact::NotApplicable(Reason::UnsupportedExpression)
                ));
            }
        }
        let malformed = GrammarDialect::Python.ast_grep("broken = (\n");
        assert!(
            malformed
                .root()
                .dfs()
                .any(|n| n.is_error() || n.is_missing())
        );
        assert!(matches!(
            qualification_eligibility(&malformed.root()),
            Fact::Unknown(Reason::SyntaxError)
        ));
    }

    #[test]
    fn valid_line_continuation_parentheses_are_not_applicable_not_unknown() {
        let source = "import pkg\n(pkg \\\n).open()\n";
        let doc = GrammarDialect::Python.ast_grep(source);
        assert!(!doc.root().dfs().any(|n| n.is_error() || n.is_missing()));
        let parens = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "parenthesized_expression")
            .unwrap();
        assert!(
            parens
                .children()
                .filter(|n| n.is_named() && n.kind() != "comment")
                .count()
                > 1,
            "fixture must retain the named line continuation native unparenthesizing stops at"
        );
        assert!(matches!(
            qualification_eligibility(&parens),
            Fact::NotApplicable(Reason::UnsupportedExpression)
        ));
        let attribute = doc.root().dfs().find(|n| n.kind() == "attribute").unwrap();
        assert!(matches!(
            qualification_eligibility(&attribute),
            Fact::NotApplicable(Reason::UnsupportedExpression)
        ));
    }

    #[test]
    fn unsupported_callee_is_not_a_semantic_unknown_and_bad_configuration_is_rejected() {
        let source = "from pathlib import Path\nPath('f').open()\n";
        let facts = batch(source, None, &["Path('f').open", "Path"]);
        assert!(matches!(
            at(
                &facts,
                source.find("Path('f').open").unwrap(),
                "Path('f').open"
            )
            .qualification,
            Fact::NotApplicable(Reason::UnsupportedExpression)
        ));
        assert_eq!(
            qualified(at(&facts, source.find("Path('f')").unwrap(), "Path")),
            "pathlib.Path"
        );
        let doc = GrammarDialect::Python.ast_grep("x\n");
        let table =
            ScopeTable::build(doc.root(), GrammarDialect::Python, usize::MAX, &|| false).unwrap();
        let bad = PythonOptions::new(u8::MAX);
        assert_eq!(
            project(
                &table,
                Config {
                    module: None,
                    python: &bad
                },
                [doc.root()]
            )
            .unwrap_err(),
            ProjectionError::UnsupportedPythonVersion
        );
        let incompatible = PythonOptions::new(14);
        assert_eq!(
            project(
                &table,
                Config {
                    module: None,
                    python: &incompatible
                },
                [doc.root()]
            )
            .unwrap_err(),
            ProjectionError::ReadingMismatch
        );
        let other = GrammarDialect::Python.ast_grep("y\n");
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        assert_eq!(
            project(
                &table,
                Config {
                    module: None,
                    python: &python
                },
                [doc.root(), other.root()]
            )
            .unwrap_err(),
            ProjectionError::MixedDocuments
        );
    }
