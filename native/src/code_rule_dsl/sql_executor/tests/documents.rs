use super::*;

fn snapshot(files: &[(&str, &str)]) -> String {
    json!({"format":"rerail.repository-snapshot/v1","revision":1,"files":files.iter().map(|(p,s)| json!({"path":p,"language":"python","source":s})).collect::<Vec<_>>()} ).to_string()
}
const NAMES: &str = ".decl name(n:node,s:symbol)\n.input name\n.decl names(s:symbol)\nnames(s) :- name(_,s).\nnames(s) :- names(s).\n.output names\n";

#[test]
fn source_edits_retain_results_and_match_a_fresh_snapshot() {
    let (_dir, mut e) = executor();
    let first = snapshot(&[("a.py", "alpha = 1\n"), ("b.py", "beta = 1\n")]);
    e.bind_snapshot(first.clone()).unwrap();
    e.run(NAMES.into(), "python".into(), None).unwrap();
    e.create_index("names".into(), vec![0]).unwrap();
    e.bind_relation(json!({"name":"external","columns":["v"],"rows":[[7]]}).to_string())
        .unwrap();
    let root = rows(
        &e,
        "SELECT rootpage FROM temp.sqlite_schema WHERE name='names'",
    );
    let retained = Arc::clone(&e.documents.as_ref().unwrap()["b.py"].sources["node"]);
    for files in [
        vec![("a.py", "gamma = 2\n"), ("b.py", "beta = 1\n")],
        vec![("b.py", "beta = 1\n"), ("c.py", "delta = 3\n")],
        vec![],
    ] {
        let next = snapshot(&files);
        let report: Value =
            serde_json::from_str(&e.apply_snapshot(next.clone(), None).unwrap()).unwrap();
        assert_eq!(report["programs"][0]["program"], 0);
        assert_eq!(
            rows(
                &e,
                "SELECT rootpage FROM temp.sqlite_schema WHERE name='names'"
            ),
            root
        );
        assert_eq!(rows(&e, "SELECT * FROM external"), json!([[7]]));
        if files.iter().any(|(p, _)| *p == "b.py") {
            assert!(Arc::ptr_eq(
                &retained,
                &e.documents.as_ref().unwrap()["b.py"].sources["node"]
            ));
        }
        let (_cold_dir, mut cold) = executor();
        cold.bind_snapshot(next).unwrap();
        cold.run(NAMES.into(), "python".into(), None).unwrap();
        // Symbol ids are store-local: compare their values.
        assert_eq!(
            rows(&e, "SELECT sym_text(c0) FROM names ORDER BY 1"),
            rows(&cold, "SELECT sym_text(c0) FROM names ORDER BY 1")
        );
        assert_eq!(
            rows(
                &e,
                "SELECT sym_text(v),sym_text(t) FROM value_type ORDER BY 1,2"
            ),
            rows(
                &cold,
                "SELECT sym_text(v),sym_text(t) FROM value_type ORDER BY 1,2"
            )
        );
    }
}

#[test]
fn native_edits_are_atomic_and_identical_snapshots_are_noops() {
    let (_dir, mut e) = executor();
    let first = snapshot(&[("a.py", "alpha = 1\n")]);
    e.bind_snapshot(first.clone()).unwrap();
    e.run(NAMES.into(), "python".into(), None).unwrap();
    let before = rows(&e, "SELECT * FROM names");
    assert!(
        e.apply_snapshot(snapshot(&[("a.py", "def broken(\n")]), None)
            .is_err()
    );
    assert_eq!(rows(&e, "SELECT * FROM names"), before);
    e.set_limits("{\"max_steps\":1}".into()).unwrap();
    assert!(
        e.apply_snapshot(snapshot(&[("a.py", "beta = 2\n")]), None)
            .is_err()
    );
    e.set_limits("{}".into()).unwrap();
    assert_eq!(rows(&e, "SELECT * FROM names"), before);
    assert_eq!(rows(&e, "SELECT sym_text(s) FROM name"), json!([["alpha"]]));
    let noop: Value = serde_json::from_str(&e.apply_snapshot(first, None).unwrap()).unwrap();
    assert_eq!(noop["rebuilt_files"], 0);
    assert_eq!(noop["programs"], json!([]));
    e.apply_snapshot(snapshot(&[("a.py", "beta = 2\n")]), None)
        .unwrap();
    assert_eq!(
        rows(&e, "SELECT sym_text(c0) FROM names"),
        json!([["beta"]])
    );
}

#[test]
fn native_snapshot_shared_literal_types_survive_one_document_deletion() {
    let (_dir, mut e) = executor();
    e.bind_snapshot(snapshot(&[("a.py", "a = 1\n"), ("b.py", "b = 1\n")]))
        .unwrap();
    let program = ".decl value_type(v:symbol,t:symbol)\n.input value_type\n.decl types(v:symbol,t:symbol)\ntypes(v,t) :- value_type(v,t).\n.output types\n";
    e.run(program.into(), "python".into(), None).unwrap();
    let before = rows(&e, "SELECT * FROM types");
    assert_eq!(rows(&e, "SELECT count(*) FROM value_type"), json!([[1]]));
    e.apply_snapshot(snapshot(&[("b.py", "b = 1\n")]), None)
        .unwrap();
    assert_eq!(rows(&e, "SELECT * FROM types"), before);
    e.apply_snapshot(snapshot(&[]), None).unwrap();
    assert_eq!(rows(&e, "SELECT * FROM types"), json!([]));
}

#[test]
fn native_lazy_tree_lookups_work_across_repeated_edits() {
    let (_dir, mut e) = executor();
    for source in [
        "def a():\n    return 1\n",
        "def b():\n    if True:\n        return 2\n",
        "def c():\n    return 3\n",
    ] {
        let next = snapshot(&[("a.py", source), ("b.py", "x = 1\n")]);
        if e.documents.is_none() {
            e.bind_snapshot(next.clone()).unwrap();
        } else {
            e.apply_snapshot(next.clone(), None).unwrap();
        }
        let (_dir, mut cold) = executor();
        cold.bind_snapshot(next).unwrap();
        let q = "SELECT count(*) FROM node n JOIN within w ON w.n=n.n JOIN node a ON a.n=w.a WHERE kind_name('python',n.kind)='return_statement'";
        let actual = rows(&e, q);
        assert!(actual[0][0].as_u64().unwrap() > 0);
        assert_eq!(actual, rows(&cold, q));
    }
}

#[test]
fn native_shards_match_the_original_batch_and_both_probe_directions() {
    let (_dir, mut e) = executor();
    let input = snapshot(&[
        ("a.py", "def a(x):\n    return service.run(x, flag=x)\nother(2)\n"),
        ("b.py", "b = 1\n"),
    ]);
    e.bind_snapshot(input.clone()).unwrap();
    let (snapshot, _) = super::super::super::eval::parse_snapshot(&input, false).unwrap();
    let documents = snapshot
        .files
        .iter()
        .map(|file| {
            let Built::Facts(facts) = core::build_cached(file, &e.store).unwrap() else {
                panic!()
            };
            syntax::Document {
                key: e.store.intern_atoms(&[&file.path]).unwrap()[0],
                path: file.path.clone(),
                core: String::new(),
                digest: [0; 32],
                facts: Arc::new(facts),
                guest: None,
                module: None,
                tape: None,
            }
        })
        .collect();
    let reference = syntax::with_nearest(
        &e.store,
        documents,
        &[],
        &Default::default(),
        super::super::super::typing::file_local::Binding::Cpu,
        &syntax::Projection::core(),
    )
    .unwrap();
    for (name, source) in reference {
        let actual = e.inputs[&name].as_ref();
        let tuples = |s: &dyn Source| {
            (0..s.len())
                .map(|r| {
                    (0..s.columns().len())
                        .map(|c| s.value(r, c))
                        .collect::<Vec<_>>()
                })
                .collect::<BTreeSet<_>>()
        };
        let expected = tuples(source.as_ref());
        assert_eq!(tuples(actual), expected, "{name}");
        for c in 0..source.columns().len() {
            let values = expected.iter().map(|r| r[c]).collect::<BTreeSet<_>>();
            for v in values {
                if let Some(found) = actual.lookup(c, &[v]) {
                    let rows = found
                        .into_iter()
                        .map(|r| {
                            (0..actual.columns().len())
                                .map(|c| actual.value(r, c))
                                .collect::<Vec<_>>()
                        })
                        .filter(|r| r[c] == v)
                        .collect::<BTreeSet<_>>();
                    assert_eq!(
                        rows,
                        expected.iter().filter(|r| r[c] == v).cloned().collect(),
                        "{name}[{c}]={v}"
                    );
                }
            }
        }
    }
}

fn patch(e: &mut CodeRuleSqlExecutor, upsert: &[(&str, &str)], delete: &[&str]) -> Value {
    let batch = json!({"upsert":upsert.iter().map(|(p,s)|json!({"path":p,"language":"python","source":s})).collect::<Vec<_>>(),"delete":delete});
    serde_json::from_str(&e.apply_file_changes(batch.to_string(), None).unwrap()).unwrap()
}

#[test]
fn file_patches_are_atomic_and_interoperate_with_complete_snapshots() {
    let (_dir, mut e) = executor();
    e.bind_snapshot(snapshot(&[("a.py", "alpha = 1\n"), ("b.py", "beta = 1\n")]))
        .unwrap();
    e.run(NAMES.into(), "python".into(), None).unwrap();
    let stable = Arc::clone(&e.documents.as_ref().unwrap()["b.py"].sources["node"]);
    let report = patch(
        &mut e,
        &[("a.py", "gamma = 2\n"), ("c.py", "delta = 1\n")],
        &[],
    );
    assert_eq!(report["examined_files"], 2);
    assert_eq!(report["rebuilt_files"], 2);
    assert!(Arc::ptr_eq(
        &stable,
        &e.documents.as_ref().unwrap()["b.py"].sources["node"]
    ));
    let before = rows(&e, "SELECT sym_text(c0) FROM names ORDER BY 1");
    assert!(e.apply_file_changes(json!({"upsert":[{"path":"a.py","language":"python","source":"bad(\n"}],"delete":["b.py"]}).to_string(),None).is_err());
    e.set_limits("{\"max_steps\":1}".into()).unwrap();
    assert!(
        e.apply_file_changes(json!({"delete":["b.py"]}).to_string(), None)
            .is_err()
    );
    e.set_limits("{}".into()).unwrap();
    assert_eq!(
        rows(&e, "SELECT sym_text(c0) FROM names ORDER BY 1"),
        before
    );
    let noop = patch(&mut e, &[("b.py", "beta = 1\n")], &["missing.py"]);
    assert_eq!(noop["metadata_nodes"], 0);
    assert_eq!(noop["programs"], json!([]));
    patch(&mut e, &[], &["b.py"]);
    assert_eq!(
        rows(&e, "SELECT sym_text(c0) FROM names ORDER BY 1"),
        json!([["delta"], ["gamma"]])
    );
    e.apply_snapshot(snapshot(&[("c.py", "delta = 1\n")]), None)
        .unwrap();
    assert_eq!(
        rows(&e, "SELECT sym_text(c0) FROM names"),
        json!([["delta"]])
    );
    patch(&mut e, &[("z.py", "zeta = 1\n")], &["c.py"]);
    assert_eq!(
        rows(&e, "SELECT sym_text(c0) FROM names"),
        json!([["zeta"]])
    );
    for batch in [
        json!({"upsert":[{"path":"z.py","source":"x=1","language":"python"}],"delete":["z.py"]}),
        json!({"delete":["z.py","z.py"]}),
        json!({"unknown":[]}),
    ] {
        assert!(e.apply_file_changes(batch.to_string(), None).is_err());
    }
    assert_eq!(
        rows(&e, "SELECT sym_text(c0) FROM names"),
        json!([["zeta"]])
    );
}

#[test]
fn shared_literal_reference_indexes_handle_partial_deletions_and_retry() {
    let (_dir, mut e) = executor();
    e.bind_snapshot(snapshot(&[
        ("a.py", "a = 1\n"),
        ("b.py", "b = 1\n"),
        ("c.py", "c = 2\n"),
    ]))
    .unwrap();
    e.run(".decl value_type(v:symbol,t:symbol)\n.input value_type\n.decl result(v:symbol,t:symbol)\nresult(v,t) :- value_type(v,t).\n.output result\n".into(),"python".into(),None).unwrap();
    patch(&mut e, &[("a.py", "a = 2\n")], &[]);
    assert_eq!(rows(&e, "SELECT count(*) FROM value_type"), json!([[2]]));
    e.set_limits("{\"max_steps\":1}".into()).unwrap();
    assert!(
        e.apply_file_changes(json!({"delete":["b.py"]}).to_string(), None)
            .is_err()
    );
    e.set_limits("{}".into()).unwrap();
    assert_eq!(rows(&e, "SELECT count(*) FROM result"), json!([[2]]));
    patch(&mut e, &[], &["b.py"]);
    assert_eq!(rows(&e, "SELECT sym_text(c0) FROM result"), json!([["2"]]));
    patch(&mut e, &[], &["a.py"]);
    assert_eq!(
        rows(
            &e,
            "SELECT sym_text(v) FROM value_type WHERE t=sym_atom('int')"
        ),
        json!([["2"]])
    );
    patch(&mut e, &[], &["c.py"]);
    assert_eq!(rows(&e, "SELECT * FROM result"), json!([]));
}

#[test]
fn a_file_patch_updates_logarithmic_metadata_and_does_not_visit_the_inventory() {
    let mut work = vec![];
    for n in [100, 1000] {
        let (_dir, mut e) = executor();
        let files = (0..n)
            .map(|i| (format!("m{i}.py"), format!("value{i} = {i}\n")))
            .collect::<Vec<_>>();
        let all = files
            .iter()
            .map(|(p, s)| (p.as_str(), s.as_str()))
            .collect::<Vec<_>>();
        e.bind_snapshot(snapshot(&all)).unwrap();
        e.run(NAMES.into(), "python".into(), None).unwrap();
        let first = patch(&mut e, &[("m0.py", "new_value = 10001\n")], &[]);
        let second = patch(&mut e, &[("m0.py", "final_value = 10002\n")], &[]);
        assert_eq!(first["examined_files"], 1);
        assert_eq!(second["examined_files"], 1);
        assert_eq!(second["rebuilt_files"], 1);
        assert!(
            second["metadata_nodes"].as_u64().unwrap() < 2000,
            "{second}"
        );
        assert_eq!(rows(&e, "SELECT count(*) FROM names"), json!([[n]]));
        assert_eq!(rows(&e, "SELECT count(*) FROM value_type"), json!([[n]]));
        work.push(second["metadata_nodes"].as_u64().unwrap());
    }
    assert!(
        work[1] < work[0] * 2,
        "metadata must follow search depth, not inventory size: {work:?}"
    );
}

#[test]
fn mixed_file_patches_match_cold_execution_with_shared_values_and_lazy_queries() {
    let (_dir, mut e) = executor();
    e.bind_snapshot(snapshot(&[])).unwrap();
    e.run(NAMES.into(), "python".into(), None).unwrap();
    let mut files = BTreeMap::<String, String>::new();
    let mut seed = 91u32;
    for step in 0..40 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let path = format!("m{}.py", seed % 13);
        if seed % 4 == 0 {
            files.remove(&path);
            patch(&mut e, &[], &[&path]);
        } else {
            let source = format!("def f{}():\n    return {}\n", step % 7, step % 3);
            files.insert(path.clone(), source.clone());
            patch(&mut e, &[(&path, &source)], &[]);
        }
        let (_cold_dir, mut cold) = executor();
        let all = files
            .iter()
            .map(|(p, s)| (p.as_str(), s.as_str()))
            .collect::<Vec<_>>();
        cold.bind_snapshot(snapshot(&all)).unwrap();
        cold.run(NAMES.into(), "python".into(), None).unwrap();
        for q in [
            "SELECT sym_text(c0) FROM names ORDER BY 1",
            "SELECT sym_text(v),sym_text(t) FROM value_type ORDER BY 1,2",
            "SELECT count(*) FROM within",
            "SELECT count(*) FROM span_within",
        ] {
            assert_eq!(rows(&e, q), rows(&cold, q), "step {step}: {q}");
        }
    }
}

#[test]
fn standalone_file_changes_maintain_rules_and_reject_unknown_patch_fields() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("content.sqlite");
    let request=json!({"snapshot":{"files":[]},"program":NAMES,"file_changes":[{"upsert":[{"path":"a.py","language":"python","source":"alpha = 1\n"}]},{"upsert":[{"path":"b.py","language":"python","source":"beta = 2\n"}],"delete":["a.py"]}],"queries":[{"sql":"SELECT sym_text(c0) FROM names"}]}).to_string();
    let result: Value =
        serde_json::from_str(&super::super::super::tools::sql(&store, &request).unwrap()).unwrap();
    assert_eq!(result["queries"][0]["rows"], json!([["beta"]]));
    assert_eq!(result["updates"][1]["examined_files"], 2);
    assert_eq!(result["updates"][1]["rebuilt_files"], 1);
    let invalid = json!({"snapshot":{"files":[]},"file_changes":[{"unknown":[]}]}).to_string();
    assert!(super::super::super::tools::sql(&store, &invalid).is_err());
}
