    use super::*;

    /// Each `ast` atom without captures reads its matcher's relation, one per distinct matcher; `docof` joins the node
    /// to its document only where the rule reads the document; an atom with captures is left as written.
    /// `kind`, `field`, `kind` with the field's node read nowhere else becomes one matcher; a field read again stays.
    #[test]
    fn tree_shaped_parts_become_matchers() {
        let src = r#"
.decl a(c: node)
.decl b(c: node, f: node)
.output a, b
a(c) :- kind(c, "call"), field(c, "function", f), kind(f, "attribute").
b(c, f) :- kind(c, "call"), field(c, "function", f), kind(f, "attribute").
"#;
        let mut prog = crate::code_rule_dsl::logic::rr::parse(src).unwrap();
        assert_eq!(absorb(&mut prog.clone(), false), 1);
        let mut captured = prog.clone();
        assert_eq!(absorb(&mut captured, true), 2, "with captures, the field the head reads is part of the matcher");
        let b = captured.rules.iter().find(|r| r.head.rel == "b").unwrap().body.clone();
        assert_eq!(b.len(), 1);
        let Literal::Ast { captures, matcher, node, .. } = &b[0] else { panic!("not a matcher: {:?}", b[0]) };
        assert_eq!(node, "c");
        assert_eq!(captures, &vec![("C1".to_owned(), "f".to_owned())], "the field's node captured");
        let m: serde_json::Value = serde_json::from_str(matcher).unwrap();
        assert_eq!(m["rule"]["all"][1]["has"]["pattern"], "$C1");
        assert_eq!(absorb(&mut prog, false), 1);
        let body = |head: &str| prog.rules.iter().find(|r| r.head.rel == head).unwrap().body.clone();
        let a = body("a");
        assert_eq!(a.len(), 1);
        let Literal::Ast { matcher, node, .. } = &a[0] else { panic!("not a matcher: {:?}", a[0]) };
        assert_eq!(node, "c");
        let m: serde_json::Value = serde_json::from_str(matcher).unwrap();
        assert_eq!(m["rule"]["all"][1]["has"]["field"], "function");
        assert_eq!(m["rule"]["all"][1]["has"]["kind"], "attribute");
        assert_eq!(body("b").len(), 3, "a field the head reads stays a join");
    }

    /// `any`'s arms bind into the environment only when they match, so two arms binding one name share nothing; a
    /// name two parts of one `all` bind is a join the labels do not carry, and stays refused.
    // O-split-logic-tests: reads a crate above rerail-logic; compiled only once the split hosts it in the engine.
    #[cfg(any())]
    #[test]
    fn any_arms_binding_one_name_lower() {
        let lower = |rule: &str| crate::code_rule_dsl::waves::policies::lower_matcher(&format!(r#"{{"language": "python", "rule": {rule}}}"#), None).map(|_| ());
        lower(r#"{"any": [{"all": [{"kind": "call"}, {"pattern": "$X"}]}, {"all": [{"kind": "attribute"}, {"pattern": "$X"}]}]}"#).unwrap();
        let err = lower(r#"{"all": [{"pattern": "$X"}, {"has": {"pattern": "$X", "stopBy": "neighbor"}}]}"#).unwrap_err();
        assert!(err.contains("shared binding"), "{err}");
    }

    /// A comparison of captures, which the labels do not carry, is dropped where that only widens the matcher; one
    /// under an `any` arm left with nothing is not.
    // O-split-logic-tests: reads a crate above rerail-logic; compiled only once the split hosts it in the engine.
    #[cfg(any())]
    #[test]
    fn a_matcher_relaxed_where_it_only_widens() {
        use crate::code_rule_dsl::waves::policies::relaxed;
        let m = |rule: &str| serde_json::from_str::<serde_json::Value>(&format!(r#"{{"language": "python", "rule": {rule}}}"#)).unwrap();
        let got = relaxed(&m(r#"{"all": [{"kind": "call"}, {"pattern": "$A"}, {"not": {"compare": {"left": "A", "right": "A", "op": "ne"}}}]}"#), false).unwrap();
        assert_eq!(got["rule"], serde_json::json!({"all": [{"kind": "call"}, {"pattern": "$A"}]}));
        let got = relaxed(&m(r#"{"kind": "call", "has": {"compare": {"left": "A", "right": "B", "op": "eq"}, "stopBy": "neighbor"}}"#), false).unwrap();
        assert_eq!(got["rule"], serde_json::json!({"kind": "call"}));
        assert!(relaxed(&m(r#"{"any": [{"kind": "call"}, {"compare": {"left": "A", "right": "B", "op": "eq"}}]}"#), false).is_none());
        assert!(relaxed(&m(r#"{"kind": "call"}"#), false).is_none());
        // A path test is W's to confirm; an `any` with an arm left matching every node goes with it.
        let got = relaxed(&m(r#"{"all": [{"kind": "call"}, {"any": [{"filePath": {"regex": "x"}}, {"kind": "call"}]}]}"#), false).unwrap();
        assert_eq!(got["rule"], serde_json::json!({"all": [{"kind": "call"}]}));
        // Every negation in a positive place, only where asked.
        let not = m(r#"{"all": [{"kind": "call"}, {"not": {"kind": "identifier"}}]}"#);
        assert!(relaxed(&not, false).is_none());
        assert_eq!(relaxed(&not, true).unwrap()["rule"], serde_json::json!({"all": [{"kind": "call"}]}));
        // The relaxed matcher lowers; its roots are confirmed by W's.
        let (_, _, captor) = crate::code_rule_dsl::waves::policies::lower_matcher(
            r#"{"language": "python", "rule": {"all": [{"kind": "call"}, {"has": {"pattern": "$A", "field": "function", "stopBy": "neighbor"}}, {"not": {"compare": {"left": "A", "right": "A", "op": "ne", "text": "node"}}}]}}"#,
            None,
        )
        .unwrap();
        assert!(captor.verify);
    }

    /// A field's kinds and a kind's leafhood from the grammar: what a long-text twin is made for.
    #[test]
    fn field_kinds_and_leaves_from_the_grammar() {
        assert_eq!(field_kinds("attribute"), Some(BTreeSet::from(["identifier".to_owned()])));
        assert!(field_kinds("function").is_some_and(|k| k.contains("call") && k.contains("attribute")));
        assert_eq!(field_kinds("no_such_field"), None);
        assert!(leaf_kind("identifier") && leaf_kind("integer"));
        assert!(!leaf_kind("call") && !leaf_kind("decorated_definition"));
    }

    /// A field's multiplicity from the grammar: `import os, sys` holds two names, a subscript several indexes.
    #[test]
    fn single_fields_from_the_grammar() {
        assert!(single_field("call", "function"));
        assert!(single_field("attribute", "object"));
        assert!(single_field("function_definition", "name"));
        assert!(!single_field("import_statement", "name"));
        assert!(!single_field("subscript", "subscript"));
        assert!(!single_field("identifier", "name"), "an alias target");
    }

    #[test]
    fn ast_atoms_read_their_matchers_relations() {
        let src = r#"
.decl a(n: node)
.decl b(n: node, d: number)
.decl c(n: node, x: node)
.output a, b, c
a(n) :- ast(d, n, {"language": "python", "rule": {"kind": "call"}}).
b(n, d) :- ast(d, n, {"language": "python", "rule": {"kind": "call"}}).
c(n, x) :- ast(d, n, {"language": "python", "rule": {"pattern": "f($X)"}}, X: x).
"#;
        let mut prog = crate::code_rule_dsl::logic::rr::parse(src).unwrap();
        let found = lower(&mut prog);
        assert_eq!(found.len(), 2, "one distinct matcher without captures, one with");
        let rel = &found[0].0;
        let body = |head: &str| prog.rules.iter().find(|r| r.head.rel == head).unwrap().body.clone();
        let reads = |head: &str| body(head).iter().filter_map(|l| l.atom().map(|a| a.rel.clone())).collect::<Vec<_>>();
        assert_eq!(reads("a"), vec![rel.clone()]);
        assert_eq!(reads("b"), vec![rel.clone(), "docof".to_owned()]);
        // A captured matcher's relation has the root and a column a capture (decision 0107).
        assert_eq!(found[1].0, "rr_ast_1_1");
        assert_eq!(found[1].2, vec!["X".to_owned()]);
        assert_eq!(reads("c"), vec!["rr_ast_1_1".to_owned()]);
        let Some(Literal::Atom(c)) = body("c").into_iter().next() else { panic!("not an atom") };
        assert_eq!(c.args, vec![Term::Var("n".into()), Term::Var("x".into())]);
        assert_eq!(columns("rr_ast_1_1"), &["n", "x1"]);
        assert_eq!(columns("rr_ast_0"), &["n"]);
        assert!(prog.inputs.contains(rel) && prog.decls.contains_key(rel));
    }

    /// A `filePath` test below a matcher's root conjuncts (here in an `any`) is the document's: the matcher holds in
    /// a file its path passes, by that branch, and elsewhere only by the other.
    // O-split-logic-tests: reads a crate above rerail-logic; compiled only once the split hosts it in the engine.
    #[cfg(any())]
    #[test]
    fn a_nested_file_path_is_the_documents() {
        let root = crate::code_rule_dsl::fixtures::temp().with_extension("nested-file-path");
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        std::fs::write(root.join("a/x.py"), "f()\n").unwrap();
        std::fs::write(root.join("b/y.py"), "f()\nzz()\n").unwrap();
        let policy = root.with_extension("policy.rr");
        let rule = r#"{"language": "python", "rule": {"all": [{"kind": "call"}, {"any": [{"filePath": {"regex": "^a/"}}, {"has": {"field": "function", "regex": "^zz"}}]}]}}"#;
        std::fs::write(&policy, format!(".language python\n.violation hit(at: node) code \"X1\" \"hit\"\nhit(n) :- ast(d, n, {rule}).\n")).unwrap();
        let report = crate::code_rule_dsl::tools::run_policies(&[policy.clone()], &root, &root.join("out"), &serde_json::Value::Null).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&policy);
        let mut hits: Vec<(String, i64)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["file"].as_str().or(d["path"].as_str()).unwrap_or("").rsplit('/').next().unwrap_or("").to_owned(), d["line"].as_i64().or(d["start_line"].as_i64()).unwrap_or(-1)))
            .collect();
        hits.sort();
        assert_eq!(hits.iter().map(|h| h.0.as_str()).collect::<Vec<_>>(), ["x.py", "y.py"], "{report:#}");
    }
