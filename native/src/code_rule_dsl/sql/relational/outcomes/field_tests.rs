//! Native differential coverage for the bounded neighbor-field contract.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl, matcher,
    sql::{exec, profiling, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

fn policy(rule: Value, utils: Value) -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"field-neighbor","revision":1,
            "mode":"steer","clauses":[{"id":"c0","forbid":{"ast":{
                "language":"python","rule":rule,"utils":utils}},
                "violation":{"code":"C0","message":"match","anchor":"witness.span"}}]})
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}

fn parity(rule: Value, source: &str) -> Value {
    let p = policy(rule, json!({}));
    super::super::tests::measure(
        &grammar_matchers(&p)[0],
        &[
            ("a.py".into(), source.into()),
            ("empty.py".into(), String::new()),
        ],
    )
}

#[test]
fn field_neighbors_select_only_the_first_child_in_the_requested_field() {
    let source = "import a, b\nfrom x import a, b\nf(a)\ng(b)\n";
    for (rule, expected) in [
        (
            json!({"any":[{"kind":"import_statement"},{"kind":"import_from_statement"}],
                "has":{"field":"name","regex":"^a$"}}),
            2,
        ),
        (
            json!({"any":[{"kind":"import_statement"},{"kind":"import_from_statement"}],
                "has":{"field":"name","regex":"^b$"}}),
            0,
        ),
        (
            json!({"kind":"dotted_name","regex":"^a$","inside":{"field":"name",
                "any":[{"kind":"import_statement"},{"kind":"import_from_statement"}]}}),
            2,
        ),
        (
            json!({"kind":"dotted_name","regex":"^b$","inside":{"field":"name",
                "any":[{"kind":"import_statement"},{"kind":"import_from_statement"}]}}),
            0,
        ),
        (
            json!({"kind":"call","has":{"field":"function","regex":"^a$"}}),
            0,
        ),
        (
            json!({"kind":"identifier","has":{"field":"body","regex":".*"}}),
            0,
        ),
        (
            json!({"kind":"argument_list","inside":{"field":"arguments","kind":"call"}}),
            2,
        ),
        (
            json!({"kind":"identifier","regex":"^a$","inside":{"field":"arguments","kind":"call"}}),
            0,
        ),
    ] {
        assert_eq!(parity(rule.clone(), source)["matches"], expected, "{rule}");
    }
}

#[test]
fn field_neighbors_preserve_caller_nodes_bindings_and_transactional_exports() {
    for rule in [
        json!({"kind":"call","all":[{"pattern":"$ROOT"},
            {"has":{"field":"function","pattern":"$NAME"}},
            {"has":{"field":"arguments","pattern":"$ARGS"}}]}),
        json!({"kind":"call","all":[{"pattern":"f($X)"},
            {"has":{"field":"function","pattern":"$X"}}]}),
        json!({"kind":"call","pattern":"$ROOT","any":[
            {"all":[{"has":{"field":"function","pattern":"$LEAK"}},{"regex":"^never$"}]},
            {"has":{"field":"arguments","pattern":"$ARGS"}}]}),
        json!({"kind":"call","has":{"field":"arguments","has":{
            "kind":"call","has":{"field":"function","pattern":"$INNER"}}}}),
        json!({"kind":"identifier","pattern":"$NAME","inside":{
            "field":"name","kind":"function_definition","pattern":"$PARENT"}}),
    ] {
        let result = parity(rule, "f(f)\nf(g)\nf(g(x))\ndef fun():\n    pass\n");
        assert!(result["matches"].as_u64().unwrap() > 0);
        assert_eq!(result["matchingStatements"], 1);
        assert_eq!(result["framePublications"], 0);
    }
}

#[test]
fn field_neighbors_preserve_unknowns_and_skip_unreached_value_failures() {
    for rule in [
        json!({"kind":"assignment","has":{"field":"right","all":[
            {"pattern":"$X"},{"testUnknown":"^u$"},{"value":{"gt":0}}]}}),
        json!({"kind":"assignment","all":[
            {"has":{"field":"right","pattern":"$X","testUnknown":"^u$"}},
            {"has":{"field":"left","pattern":"$Y"}}]}),
        json!({"kind":"identifier","has":{"field":"body","value":{"gt":0}}}),
        json!({"kind":"identifier","inside":{"field":"left","kind":"assignment",
            "testUnknown":"^u","has":{"field":"right","value":{"gt":0}}}}),
    ] {
        let p = policy(rule.clone(), json!({}));
        for allowance in [0, 1, 2, 8, 64] {
            for fault in [None, Some("2")] {
                check_work(
                    &p,
                    std::slice::from_ref(&rule),
                    &["u = u\na = 1\nb = 2\n", "c = 3\n"],
                    fault,
                    allowance,
                );
            }
        }
    }
}

#[test]
fn field_neighbor_utility_memos_use_the_selected_node_and_preserve_work() {
    let utils = json!({"seen":{"any":[{"value":{"constant":true}},{"regex":".*"}]}});
    let rule = json!({"kind":"assignment","all":[
        {"matches":"seen"},
        {"has":{"field":"left","matches":"seen"}},
        {"has":{"field":"right","matches":"seen"}},
        {"has":{"field":"left","matches":"seen"}},
        {"matches":"seen"}
    ]});
    let p = policy(rule.clone(), utils.clone());
    for allowance in [0, 1, 2, 3, 4, 8, 16, 64, 1024] {
        check_work(
            &p,
            std::slice::from_ref(&rule),
            &["x = 1\ny = 2\n", "x = 1\n"],
            None,
            allowance,
        );
    }
    // No field candidate means no child utility charge and no value call.
    let absent = json!({"kind":"identifier","has":{"field":"body","matches":"seen"}});
    let p = policy(absent.clone(), utils);
    for allowance in [0, 1, 8] {
        let result = check_work(
            &p,
            std::slice::from_ref(&absent),
            &["a\nb\n"],
            Some("a"),
            allowance,
        );
        assert_eq!(result["nativeTraces"][0]["remaining"], allowance);
        assert_eq!(result["nativeTraces"][0]["calls"], json!([]));
    }
}

#[test]
fn field_neighbors_preserve_two_valued_candidate_kind_filtering() {
    let rule = json!({"kind":"call","has":{"field":"function","matches":"wanted"}});
    let utils = json!({"wanted":{"kind":"attribute"}});
    let p = policy(rule.clone(), utils);
    // Bare callees are rejected before entering the utility; attribute callees spend one step.
    for allowance in [0, 1, 2, 8] {
        check_work(
            &p,
            std::slice::from_ref(&rule),
            &["f()\ng()\nobj.f()\n", "h()\n"],
            None,
            allowance,
        );
    }
}

#[test]
fn field_neighbor_catalog_b303_matches_native_on_real_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("packs/python");
    let source = std::fs::read_to_string(root.join("rules/bandit-b303-md5.json")).unwrap();
    let p = CodeRuleDslRuntime::compile(source, "json".into(), CAPABILITIES.into()).unwrap();
    let sources: Vec<_> = ["crypto-md5.py", "pycrypto.py"]
        .into_iter()
        .map(|name| {
            std::fs::read_to_string(root.join(format!("upstream/bandit/fixtures/{name}"))).unwrap()
        })
        .collect();
    // Resolution is not a ValueTest: compare native rows/work directly rather than using
    // check_work's ValueTest-only reached-call oracle.
    let grammar = &grammar_matchers(&p)[0];
    let docs: Vec<_> = sources.iter().map(|s| tape(grammar.language, s)).collect();
    let mut expected = vec![];
    for (file, doc) in docs.iter().enumerate() {
        let tree = AstGrep::doc(doc.clone());
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        let mut work = i64::MAX as usize;
        matcher::with_file_memo(&tree, || {
            matcher::run_clause(
                &tree,
                grammar,
                None,
                doc.transducer_len(),
                &mut work,
                |node, _, left| {
                    expected.push((file as u32, node, *left as i64));
                    Ok::<_, ()>(true)
                },
            )
            .unwrap()
            .unwrap_or_else(|_| panic!("unbounded native field oracle stopped"));
        });
    }
    assert!(!expected.is_empty());
    let compiled = super::compile_job(&[grammar], i64::MAX).unwrap();
    let ((actual, executions), frames) = profiling::frame_counts(|| {
        exec::with_terminals(
            docs.into_iter()
                .map(|doc| (doc, Default::default()))
                .collect(),
            compiled.terminals,
            |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                let mut statement = db.prepare(&compiled.sql).unwrap();
                let trace = super::super::tests::ExecutionTrace::start(db);
                let actual = statement
                    .query_map([], |row| {
                        assert_eq!(row.get::<_, i64>(4).unwrap(), 0);
                        assert_eq!(row.get::<_, String>(3).unwrap(), "[]");
                        Ok((
                            row.get::<_, u32>(1)?,
                            row.get::<_, u32>(2)?,
                            row.get::<_, i64>(5)?,
                        ))
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                (actual, trace.count())
            },
        )
    });
    assert_eq!(actual, expected);
    assert_eq!(executions, 1);
    assert_eq!(frames[4], 0);
}

#[test]
fn field_contract_refuses_rule_stopped_walks() {
    for direction in ["has", "inside"] {
        let p = policy(
            json!({direction:{"field":"function","stopBy":{"kind":"identifier"},"kind":"identifier"}}),
            json!({}),
        );
        assert!(super::compile(&grammar_matchers(&p)[0]).is_err());
    }
}
