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
        ("a.py", "def a(x):\n    return x + 1\n"),
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
