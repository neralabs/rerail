use super::*;
mod documents;
mod recovery;
mod updates;
mod scaling;

fn executor() -> (tempfile::TempDir, CodeRuleSqlExecutor) {
    let dir = tempfile::tempdir().unwrap();
    let executor = CodeRuleSqlExecutor::new(
        dir.path()
            .join("content.sqlite")
            .to_string_lossy()
            .into_owned(),
    )
    .unwrap();
    (dir, executor)
}

fn rows(executor: &CodeRuleSqlExecutor, sql: &str) -> Value {
    serde_json::from_str::<Value>(&executor.query(sql.to_owned(), None, None).unwrap()).unwrap()["rows"].clone()
}

fn edge(executor: &mut CodeRuleSqlExecutor, values: Value) {
    executor
        .bind_relation(json!({"name":"edge", "columns":["src", "dst"], "rows": values}).to_string())
        .unwrap();
}

const CLOSURE: &str = r#"
.decl edge(src: number, dst: number)
.input edge
.decl reach(src: number, dst: number)
reach(x, y) :- edge(x, y).
reach(x, z) :- reach(x, y), edge(y, z).
.output reach
"#;

#[test]
fn recursive_rules_lower_to_sql_and_run_in_waves_over_virtual_facts() {
    let (_dir, mut executor) = executor();
    edge(&mut executor, json!([[1, 2], [2, 3], [3, 4], [4, 2]]));
    let plan: Value =
        serde_json::from_str(&executor.lower(CLOSURE.into(), "python".into()).unwrap()).unwrap();
    assert!(
        plan["program"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["sql"].as_str().unwrap().contains("delta_reach"))
    );
    let run: Value =
        serde_json::from_str(&executor.run(CLOSURE.into(), "python".into(), None).unwrap())
            .unwrap();
    assert!(run["waves"].as_u64().unwrap() > 1);
    assert_eq!(
        rows(
            &executor,
            "SELECT c0, c1 FROM reach WHERE c0 = 1 ORDER BY c1"
        ),
        json!([[1, 2], [1, 3], [1, 4]])
    );
    assert!(
        rows(&executor, "SELECT count(*) FROM wave_delta WHERE rows > 0")[0][0]
            .as_u64()
            .unwrap()
            > 1
    );
    assert_eq!(
        rows(
            &executor,
            "SELECT count(*) FROM sqlite_schema WHERE name = 'edge'"
        ),
        json!([[0]])
    );
    // A second SQL program can consume the first program's materialized result.
    executor.run(".decl reach(x: number, y: number)\n.input reach\n.decl ends(x: number)\nends(y) :- reach(1, y).\n.output ends\n".into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&executor, "SELECT c0 FROM ends ORDER BY c0"),
        json!([[2], [3], [4]])
    );
    // Rebinding invalidates all derived rows but keeps the SQL connection and native modules.
    edge(&mut executor, json!([[9, 10]]));
    assert!(
        executor
            .query("SELECT * FROM reach".into(), None, None)
            .is_err()
    );
    executor.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&executor, "SELECT * FROM reach"), json!([[9, 10]]));
}

#[test]
fn core_facts_are_queryable_with_atoms_and_can_feed_rule_sql() {
    let (_dir, mut executor) = executor();
    executor
        .bind_snapshot(
            json!({"format":"rerail.repository-snapshot/v1", "revision":0, "files":[
                {"path":"a.py", "language":"python", "source":"def hello():\n    return 42\n"}
            ]})
            .to_string(),
        )
        .unwrap();
    assert_eq!(
        rows(&executor, "SELECT sym_text(path) FROM doc"),
        json!([["a.py"]])
    );
    assert_eq!(
        rows(
            &executor,
            "SELECT sym_text(s) FROM name WHERE sym_text(s) = 'hello'"
        ),
        json!([["hello"]])
    );
    let program = ".decl node(d: number, n: node, kind: symbol, start: number, end: number)\n.input node\n.decl functions(n: node)\nfunctions(n) :- node(_, n, \"function_definition\", _, _).\n.output functions\n";
    executor.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&executor, "SELECT count(*) FROM functions"),
        json!([[1]])
    );
    // A failed binding does not replace the previous fact snapshot or its successful outputs.
    assert!(
        executor
            .bind_snapshot(
                json!({"files":[{"path":"bad.py","language":"python","source":"def ("}]})
                    .to_string()
            )
            .is_err()
    );
    assert_eq!(
        rows(&executor, "SELECT count(*) FROM functions"),
        json!([[1]])
    );
    executor
        .bind_snapshot(
            json!({"files":[{"path":"b.py","language":"python","source":"x = 1\n"}]}).to_string(),
        )
        .unwrap();
    assert_eq!(
        rows(&executor, "SELECT sym_text(path) FROM doc"),
        json!([["b.py"]])
    );
    executor.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&executor, "SELECT count(*) FROM functions"),
        json!([[0]])
    );
}

#[test]
fn negation_aggregates_symbols_and_native_functions_preserve_sql_semantics() {
    let (_dir, mut executor) = executor();
    let program = r#"
.decl item(k: number, s: symbol)
item(1, "HELLO"). item(1, "WORLD"). item(2, "NO").
.decl blocked(k: number)
blocked(2).
.decl allowed(k: number, s: symbol)
allowed(k, @lower(s)) :- item(k, s), !blocked(k).
.decl counts(k: number, n: number)
counts(k, n) :- allowed(k, _), n = count : { allowed(k, _) }.
.output allowed, counts
"#;
    executor.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(
            &executor,
            "SELECT c0, sym_text(c1) FROM allowed ORDER BY c1"
        ),
        json!([[1, "hello"], [1, "world"]])
    );
    assert_eq!(rows(&executor, "SELECT * FROM counts"), json!([[1, 2]]));
    assert_eq!(
        rows(
            &executor,
            "SELECT sym_text(sym_atom('é')), lower('É'), sha256('abc')"
        ),
        json!([[
            "é",
            "é",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ]])
    );
}

#[test]
fn queries_are_parameterized_bounded_and_failures_do_not_publish_partial_results() {
    let (_dir, mut executor) = executor();
    edge(&mut executor, json!([[1, 2], [2, 3], [3, 4]]));
    let result: Value = serde_json::from_str(
        &executor
            .query(
                "SELECT dst FROM edge WHERE src >= ?1 ORDER BY src".into(),
                Some("[2]".into()),
                Some(1),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["rows"], json!([[3]]));
    assert_eq!(result["truncated"], true);
    assert!(
        executor
            .query("DELETE FROM atoms".into(), None, None)
            .is_err()
    );
    assert!(
        executor
            .run(CLOSURE.into(), "python".into(), Some(1))
            .is_err()
    );
    assert!(
        executor
            .query("SELECT * FROM reach".into(), None, None)
            .is_err()
    );
    executor.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&executor, "SELECT count(*) FROM reach"), json!([[6]]));
    executor.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&executor, "SELECT count(*) FROM reach"), json!([[6]]));
}

#[test]
fn sessions_isolate_bindings_and_reject_invalid_relation_shapes() {
    let (dir, mut first) = executor();
    let mut second = CodeRuleSqlExecutor::new(
        dir.path()
            .join("content.sqlite")
            .to_string_lossy()
            .into_owned(),
    )
    .unwrap();
    edge(&mut first, json!([[1, 2]]));
    edge(&mut second, json!([[7, 8]]));
    assert_eq!(rows(&first, "SELECT * FROM edge"), json!([[1, 2]]));
    assert_eq!(rows(&second, "SELECT * FROM edge"), json!([[7, 8]]));
    for invalid in [
        json!({"name":"edge", "columns":["x"], "rows":[[1]]}),
        json!({"name":"wave_step", "columns":["x"], "rows":[]}),
        json!({"name":"atoms", "columns":["id", "name"], "rows":[]}),
        json!({"name":"x", "columns":["a","A"], "rows":[]}),
        json!({"name":"x", "columns":["a"], "rows":[[1,2]]}),
    ] {
        assert!(first.bind_relation(invalid.to_string()).is_err());
    }
    assert_eq!(rows(&first, "SELECT * FROM edge"), json!([[1, 2]]));
}

#[test]
fn rule_files_use_the_core_prelude_and_unused_inputs_need_no_binding() {
    let (dir, mut executor) = executor();
    executor.bind_snapshot(json!({"files":[{"path":"a.py", "language":"python", "source":"def f():\n    pass\n"}]}).to_string()).unwrap();
    let file = dir.path().join("functions.rr");
    std::fs::write(&file, ".decl functions(n: node)\nfunctions(n) :- node(_, n, \"function_definition\", _, _).\n.output functions\n").unwrap();
    executor
        .run_file(file.to_string_lossy().into_owned(), "python".into(), None)
        .unwrap();
    assert_eq!(
        rows(&executor, "SELECT count(*) FROM functions"),
        json!([[1]])
    );
}

#[test]
fn standalone_requests_and_stored_integer_relations_use_the_same_executor() {
    let (dir, executor) = executor();
    executor.store.open_connection().unwrap().execute_batch("CREATE TABLE stored_edge(a INTEGER, b INTEGER); INSERT INTO stored_edge VALUES (5,6),(6,7)").unwrap();
    let program = CLOSURE
        .replace("edge(src:", "stored_edge(src:")
        .replace(".input edge", ".input stored_edge")
        .replace("edge(x, y)", "stored_edge(x, y)")
        .replace("edge(y, z)", "stored_edge(y, z)");
    let request = json!({"program":program,"queries":[{"sql":"SELECT c0, c1 FROM reach ORDER BY c0, c1"},{"sql":"SELECT name FROM sqlite_schema WHERE name = ?1", "parameters":["stored_edge"]}]}).to_string();
    let response: Value = serde_json::from_str(
        &crate::code_rule_dsl::tools::sql(&dir.path().join("content.sqlite"), &request).unwrap(),
    )
    .unwrap();
    assert_eq!(
        response["queries"][0]["rows"],
        json!([[5, 6], [5, 7], [6, 7]])
    );
    assert_eq!(response["queries"][1]["rows"], json!([["stored_edge"]]));
    assert!(response["execution"]["waves"].as_u64().unwrap() > 1);
}

#[test]
fn duplicate_facts_are_sets_and_flat_lattice_conflicts_become_top() {
    let (_dir, mut executor) = executor();
    edge(&mut executor, json!([[1, 2], [1, 2], [1, 3]]));
    let program = ".decl edge(a: number, b: number)\n.input edge\n .decl size(k: number, n: number)\nsize(k, n) :- edge(k, _), n = count : { edge(k, _) }.\n.decl joined(k: number, v: flat<>)\njoined(k, v) :- edge(k, v).\n.output size, joined\n";
    executor.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&executor, "SELECT * FROM size"), json!([[1, 2]]));
    assert_eq!(
        rows(&executor, "SELECT * FROM joined"),
        json!([[1, wire::integer(super::super::logic::native::TOP)]])
    );
}

#[test]
fn corpus_sql_mode_matches_native_rule_execution() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corpus");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.py"), "answer = 42\n").unwrap();
    let program = CLOSURE.replace(".input edge", "edge(1, 2). edge(2, 3). edge(3, 4).");
    for (name, modes) in [("sql", ""), ("native", "program")] {
        let report = crate::code_rule_dsl::tools::logic(
            &[program.clone()],
            &root,
            None,
            &dir.path().join(name),
            modes,
            false,
            1024,
            &[],
        )
        .unwrap();
        if modes.is_empty() {
            assert!(report.contains("0 components native (sql,"), "{report}");
        }
    }
    let output =
        |name: &str| std::fs::read_to_string(dir.path().join(name).join("reach.csv")).unwrap();
    assert_eq!(output("sql"), output("native"));
    assert_eq!(output("sql").lines().count(), 6);
}

#[test]
fn unsupported_lattices_fail_explicitly_and_query_created_symbols_survive_close() {
    let (dir, executor) = executor();
    let unsupported = ".lattice custom<number> { Lub -> @custom }\n.decl v(k: number, x: custom<>) widen 2\nv(1, 2).\n.output v\n";
    let failure = executor
        .lower(unsupported.into(), "python".into())
        .unwrap_err();
    assert!(
        failure.reason.contains("register SQL operations"),
        "{failure}"
    );
    assert!(
        executor
            .query("SELECT 1; SELECT 2".into(), None, None)
            .is_err()
    );
    assert!(
        executor
            .query(
                "SELECT ?1".into(),
                Some("[18446744073709551615]".into()),
                None
            )
            .is_err()
    );
    let value = rows(&executor, "SELECT sym_atom('extension-persisted-symbol')");
    drop(executor);
    let reopened = CodeRuleSqlExecutor::new(
        dir.path()
            .join("content.sqlite")
            .to_string_lossy()
            .into_owned(),
    )
    .unwrap();
    assert_eq!(
        rows(&reopened, &format!("SELECT sym_text({})", value[0][0])),
        json!([["extension-persisted-symbol"]])
    );
}

#[test]
fn stored_core_facts_bind_without_source_files_and_rebinding_refreshes_them() {
    let (dir, mut executor) = executor();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.py"), "def stored():\n    pass\n").unwrap();
    let root =
        crate::code_rule_dsl::facts::observe::roots::directory_root(&executor.store, &source)
            .unwrap();
    let mut families = crate::code_rule_dsl::facts::families::Families::default();
    families
        .push(&executor.store, "/stored", root, 8, |_| Ok(Vec::new()))
        .unwrap();
    // The SQL context uses only the store, even after the source file disappears from disk.
    std::fs::remove_file(source.join("a.py")).unwrap();
    executor.bind_stored("/stored".into(), None).unwrap();
    assert_eq!(
        rows(&executor, "SELECT sym_text(path) FROM doc"),
        json!([["a.py"]])
    );
    assert_eq!(
        rows(
            &executor,
            "SELECT sym_text(s) FROM name WHERE sym_text(s) = 'stored'"
        ),
        json!([["stored"]])
    );
    assert!(
        executor
            .bind_stored("/stored".into(), Some(vec!["missing.py".into()]))
            .is_err()
    );
    assert_eq!(rows(&executor, "SELECT count(*) FROM doc"), json!([[1]]));
    std::fs::write(source.join("a.py"), "def refreshed():\n    pass\n").unwrap();
    let root =
        crate::code_rule_dsl::facts::observe::roots::directory_root(&executor.store, &source)
            .unwrap();
    families
        .push(&executor.store, "/stored", root, 8, |_| Ok(Vec::new()))
        .unwrap();
    executor
        .bind_stored("/stored".into(), Some(vec!["a.py".into()]))
        .unwrap();
    assert_eq!(
        rows(
            &executor,
            "SELECT sym_text(s) FROM name WHERE sym_text(s) = 'refreshed'"
        ),
        json!([["refreshed"]])
    );
    assert_eq!(
        rows(
            &executor,
            "SELECT count(*) FROM name WHERE sym_text(s) = 'stored'"
        ),
        json!([[0]])
    );
}

mod reductions;

#[test]
fn the_catalog_lists_rules_relations_and_plans_and_names_their_columns() {
    let (_dir, mut executor) = executor();
    edge(&mut executor, json!([[1, 2], [2, 3]]));
    executor.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&executor, "SELECT name, kind, arity, output, view FROM rr_relation ORDER BY name"),
        json!([["edge", "input", 2, 0, "rr_rel_edge"], ["reach", "derived", 2, 1, "rr_rel_reach"]])
    );
    assert_eq!(
        rows(&executor, "SELECT relation, ord, name FROM rr_column WHERE relation = 'reach' ORDER BY ord"),
        json!([["reach", 0, "src"], ["reach", 1, "dst"]])
    );
    // Typed views read the positional relations under their declared column names.
    assert_eq!(
        rows(&executor, "SELECT src, dst FROM rr_rel_reach WHERE src = 1 ORDER BY dst"),
        json!([[1, 2], [1, 3]])
    );
    assert_eq!(rows(&executor, "SELECT count(*) FROM rr_rel_edge"), json!([[2]]));
    // Which rules read `edge`, and through which literal.
    assert_eq!(
        rows(&executor, "SELECT r.ord, a.ord, a.kind FROM rr_atom a JOIN rr_rule r USING (rule) WHERE a.relation = 'edge' ORDER BY r.ord"),
        json!([[0, 0, "atom"], [1, 1, "atom"]])
    );
    assert!(
        rows(&executor, "SELECT count(*) FROM rr_plan WHERE output = 'reach' AND sql LIKE '%delta_reach%'")[0][0]
            .as_u64()
            .unwrap()
            > 0
    );
    // Programs cannot take the catalog's names, and a reset empties it.
    assert!(executor.run(".decl rr_x(a: number)\nrr_x(1).\n.output rr_x\n".into(), "python".into(), None).is_err());
    executor.reset().unwrap();
    assert_eq!(rows(&executor, "SELECT count(*) FROM rr_relation"), json!([[0]]));
    assert_eq!(rows(&executor, "SELECT count(*) FROM temp.sqlite_schema WHERE name = 'rr_rel_reach'"), json!([[0]]));
}

#[test]
fn sql_producers_read_tables_and_derived_relations_and_refuse_cycles() {
    let (_dir, mut executor) = executor();
    edge(&mut executor, json!([[1, 2], [2, 3], [3, 4], [1, 5]]));
    executor
        .connection()
        .execute_batch("CREATE TEMP TABLE team_api(name TEXT, owner TEXT); INSERT INTO team_api VALUES ('reach', 'core'), ('ends', 'web');")
        .unwrap();
    // Kind 1 reads an ordinary table; its symbol column is interned and joins a rule's constant.
    // Kind 2 reads `reach` once it is complete, and a rule builds on the aggregate.
    let program = r#"
.decl edge(src: number, dst: number)
.input edge
.decl reach(src: number, dst: number)
reach(x, y) :- edge(x, y).
reach(x, z) :- reach(x, y), edge(y, z).
.decl owner(rel: symbol, team: symbol)
.input owner(IO=sql, query="SELECT name, owner FROM team_api")
.decl fanout(src: number, n: number)
.input fanout(IO=sql, query="SELECT src, count(*) FROM rr_rel_reach GROUP BY src")
.decl wide(src: number)
wide(s) :- fanout(s, n), n >= 3.
.decl core_owned(rel: symbol)
core_owned(r) :- owner(r, "core").
.output wide, core_owned
"#;
    executor.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&executor, "SELECT src, n FROM rr_rel_fanout ORDER BY src"), json!([[1, 4], [2, 2], [3, 1]]));
    assert_eq!(rows(&executor, "SELECT src FROM rr_rel_wide"), json!([[1]]));
    assert_eq!(rows(&executor, "SELECT sym_text(rel) FROM rr_rel_core_owned"), json!([["reach"]]));
    assert_eq!(
        rows(&executor, "SELECT name, kind FROM rr_relation WHERE name IN ('owner', 'fanout') ORDER BY name"),
        json!([["fanout", "sql"], ["owner", "sql"]])
    );
    assert_eq!(
        rows(&executor, "SELECT reads FROM rr_plan WHERE output = 'fanout'"),
        json!([["[\"reach\"]"]])
    );
    // Tuple maintenance cannot maintain a producer: the change fails and the session is unchanged.
    let change = json!([{"name": "edge", "insert": [[5, 6]], "delete": []}]).to_string();
    assert!(executor.apply_changes(change, None).unwrap_err().to_string().contains("SQL producers"));
    assert_eq!(rows(&executor, "SELECT src, n FROM rr_rel_fanout WHERE src = 1"), json!([[1, 4]]));
    // Recursion through a producer is refused.
    let cycle = r#"
.decl a(x: number)
.decl b(x: number)
.input b(IO=sql, query="SELECT x + 1 FROM rr_rel_a WHERE x < 3")
a(1).
a(x) :- b(x).
.output a
"#;
    let err = executor.run(cycle.into(), "python".into(), None).unwrap_err().to_string();
    assert!(err.contains("no recursion through SQL"), "{err}");
}
