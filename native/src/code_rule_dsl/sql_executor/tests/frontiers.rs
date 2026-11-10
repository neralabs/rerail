//! Cold adapters preserve SQL rule gates without candidate or scheduler tables.
use super::*;
fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], values: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":values}).to_string())
        .unwrap();
}

fn private_schema(e: &CodeRuleSqlExecutor) -> Value {
    rows(
        e,
        "SELECT name FROM temp.sqlite_schema WHERE name GLOB 'new_*' OR name GLOB 'delta_*' OR name GLOB '__sql_changes_*' OR name GLOB '__rr_update_*' OR name GLOB '__sql_state_*' OR name IN('wave_step','wave_read','wave_pending','wave_after','wave_document') ORDER BY name",
    )
}

#[test]
fn cold_closure_uses_native_frontiers_through_failure_edits_and_reset() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "edge",
        &["src", "dst"],
        json!([[1, 2], [2, 3], [3, 4]]),
    );
    let p = ".decl edge(src:number,dst:number)\n.input edge\n.decl reach(a:number,b:number)\nreach(a,b) :- edge(a,b).\nreach(a,c) :- reach(a,b),edge(b,c).\n.output reach\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(private_schema(&e), json!([]));
    for patch in [
        json!([{"name":"edge","delete":[[2,3]]}]),
        json!([{"name":"edge","insert":[[2,3]]}]),
    ] {
        let before = rows(&e, "SELECT * FROM reach ORDER BY c0,c1");
        assert!(e.apply_changes(patch.to_string(), Some(1)).is_err());
        assert_eq!(rows(&e, "SELECT * FROM reach ORDER BY c0,c1"), before);
        e.apply_changes(patch.to_string(), None).unwrap();
        assert_eq!(private_schema(&e), json!([]));
    }
    assert_eq!(rows(&e, "SELECT count(*) FROM reach"), json!([[6]]));
    e.reset().unwrap();
    e.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(private_schema(&e), json!([]));
}

#[test]
fn opaque_producer_gates_merges_and_widening_match_independent_sql_frontiers() {
    let p = ".decl root(n:number)\n.decl edge(a:number,b:number,w:number)\n.input root,edge\n.decl allowed(n:number)\n.input allowed(IO=sql, query=\"SELECT n FROM root\")\n.decl distance(n:number,d:min<>) widen 2\ndistance(n,0) :- allowed(n).\ndistance(b,d+w) :- distance(a,d),edge(a,b,w).\n.output distance\n";
    let (_a, mut e) = executor();
    let (_b, mut oracle) = executor();
    oracle.cold_oracle = true;
    for target in [&mut e, &mut oracle] {
        bind(target, "root", &["n"], json!([[0], [10]]));
        bind(
            target,
            "edge",
            &["a", "b", "w"],
            json!([[0, 1, 5], [0, 2, 1], [2, 1, 1], [1, 3, 2], [10, 11, 1]]),
        );
    }
    let result: Value =
        serde_json::from_str(&e.run(p.into(), "python".into(), None).unwrap()).unwrap();
    let expected: Value =
        serde_json::from_str(&oracle.run(p.into(), "python".into(), None).unwrap()).unwrap();
    assert_eq!(result["waves"], expected["waves"]);
    for sql in [
        "SELECT * FROM distance ORDER BY c0",
        "SELECT * FROM __sql_changes_distance ORDER BY c0",
        "SELECT * FROM wave_delta WHERE rows>0 ORDER BY wave,rel",
    ] {
        assert_eq!(rows(&e, sql), rows(&oracle, sql), "{sql}");
    }
    assert_eq!(private_schema(&e), json!([]));
    let patch = json!([{"name":"edge","delete":[[2,1,1]]}]).to_string();
    let before = rows(&e, "SELECT * FROM distance ORDER BY c0");
    assert!(e.apply_changes(patch.clone(), Some(1)).is_err());
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0"), before);
    e.apply_changes(patch, None).unwrap();
    bind(
        &mut oracle,
        "edge",
        &["a", "b", "w"],
        rows(&e, "SELECT * FROM edge"),
    );
    oracle.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&e, "SELECT * FROM distance ORDER BY c0"),
        rows(&oracle, "SELECT * FROM distance ORDER BY c0")
    );
    assert_eq!(
        rows(&e, "SELECT * FROM __sql_changes_distance ORDER BY c0"),
        rows(&oracle, "SELECT * FROM __sql_changes_distance ORDER BY c0")
    );
    assert_eq!(private_schema(&e), json!([]));
}

#[test]
fn opaque_producers_keep_generated_projections_native_across_document_edits() {
    let (_d, mut e) = executor();
    let program = include_str!("../../logic/plan/golden/programs/workload.rr").to_owned()
        + "\n.decl gate(n:number)\n.input gate(IO=sql, query=\"SELECT 1\")\n.output gate\n";
    let snapshot=json!({"files":[{"path":"a.py","language":"python","source":"def f():\n    x = 1\n    return x\n"}]}).to_string();
    e.bind_snapshot(snapshot.clone()).unwrap();
    e.run(program.clone(), "python".into(), None).unwrap();
    let query = "SELECT name FROM temp.sqlite_schema WHERE name GLOB '__sql_*' AND type='table'";
    assert_eq!(rows(&e, query), json!([]));
    let patch=json!({"upsert":[{"path":"a.py","language":"python","source":"def g():\n    x = 2\n    return x\n"}]}).to_string();
    assert!(e.apply_file_changes(patch.clone(), Some(1)).is_err());
    e.apply_file_changes(patch, None).unwrap();
    assert_eq!(rows(&e, query), json!([]));
    e.apply_snapshot(snapshot, None).unwrap();
    assert_eq!(rows(&e, query), json!([]));
    e.reset().unwrap();
    e.run(program, "python".into(), None).unwrap();
    assert_eq!(rows(&e, query), json!([]));
}
