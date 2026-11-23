use super::*;

#[test]
fn retained_initial_execution_preserves_global_gates_frontiers_and_wave_limits() {
    let p = ".decl root(n:number)\n.decl edge(a:number,b:number)\n.decl bump(x:number)\n.input root,edge,bump\n.decl reach(n:number)\nreach(n) :- root(n).\nreach(b) :- reach(a), edge(a,b).\n.decl amount(n:number,v:max<>) widen 1\namount(0,7).\namount(n,c+extra) :- reach(n), c=count : {reach(_)}, extra=count : {bump(_)}.\namount(b,v) :- amount(a,v), edge(a,b).\n.output reach,amount\n";
    let (_a, mut native) = executor();
    let (_b, mut cold) = executor();
    cold.cold_oracle = true;
    for e in [&mut native, &mut cold] {
        e.bind_relation(json!({"name":"root","columns":["n"],"rows":[[0]]}).to_string())
            .unwrap();
        e.bind_relation(
            json!({"name":"edge","columns":["a","b"],"rows":[[0,1],[1,2],[2,3],[3,1]]}).to_string(),
        )
        .unwrap();
    }
    for e in [&mut native, &mut cold] {
        e.bind_relation(json!({"name":"bump","columns":["x"],"rows":[]}).to_string())
            .unwrap();
    }
    let a: Value =
        serde_json::from_str(&native.run(p.into(), "python".into(), None).unwrap()).unwrap();
    let b: Value =
        serde_json::from_str(&cold.run(p.into(), "python".into(), None).unwrap()).unwrap();
    assert_eq!(a["preparation"]["initial_execution"], "retained");
    assert_eq!(a["waves"], b["waves"]);
    for sql in [
        "SELECT * FROM reach ORDER BY c0",
        "SELECT * FROM amount ORDER BY c0",
        "SELECT * FROM __sql_changes_amount ORDER BY c0",
        "SELECT * FROM wave_delta WHERE rows>0 ORDER BY wave,rel",
    ] {
        assert_eq!(rows(&mut native, sql), rows(&mut cold, sql), "{sql}");
    }
    // An inactive earlier frame accumulates this edit before a later batch opens its gate.
    native
        .apply_changes(json!([{"name":"bump","insert":[[1]]}]).to_string(), None)
        .unwrap();
    cold.bind_relation(json!({"name":"bump","columns":["x"],"rows":[[1]]}).to_string())
        .unwrap();
    cold.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&mut native, "SELECT * FROM amount ORDER BY c0"),
        rows(&mut cold, "SELECT * FROM amount ORDER BY c0")
    );
    let limit = a["waves"].as_u64().unwrap() as u32 - 1;
    assert!(native.run(p.into(), "python".into(), Some(limit)).is_err());
    assert!(cold.run(p.into(), "python".into(), Some(limit)).is_err());
    assert_eq!(
        rows(&mut native, "SELECT * FROM amount ORDER BY c0"),
        rows(&mut cold, "SELECT * FROM amount ORDER BY c0")
    );
    native
        .apply_changes(json!([{"name":"edge","delete":[[1,2]]}]).to_string(), None)
        .unwrap();
    cold.bind_relation(
        json!({"name":"edge","columns":["a","b"],"rows":[[0,1],[2,3],[3,1]]}).to_string(),
    )
    .unwrap();
    cold.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&mut native, "SELECT * FROM amount ORDER BY c0"),
        rows(&mut cold, "SELECT * FROM amount ORDER BY c0")
    );
}
