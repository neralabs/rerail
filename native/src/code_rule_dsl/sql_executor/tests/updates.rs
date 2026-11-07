use super::*;

fn change(e: &mut CodeRuleSqlExecutor, value: Value) -> Value {
    serde_json::from_str(&e.apply_changes(value.to_string(), None).unwrap()).unwrap()
}
fn reachable(e: &CodeRuleSqlExecutor) -> Value {
    rows(e, "SELECT * FROM reach ORDER BY c0,c1")
}

#[test]
fn insertions_deletions_and_noops_preserve_indexes_and_propagate_between_programs() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3]]));
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    e.run(".decl reach(a:number,b:number)\n.input reach\n.decl ends(n:number)\nends(n) :- reach(1,n).\n.output ends\n".into(),"python".into(),None).unwrap();
    e.run(
        ".decl unrelated(n:number)\nunrelated(42).\n.output unrelated\n".into(),
        "python".into(),
        None,
    )
    .unwrap();
    e.create_index("reach".into(), vec![1]).unwrap();
    let root = rows(
        &e,
        "SELECT rootpage FROM temp.sqlite_schema WHERE name='reach'",
    );
    let report = change(&mut e, json!([{"name":"edge","insert":[[3,4]]}]));
    assert_eq!(report["inserted_facts"], 1);
    assert_eq!(
        rows(&e, "SELECT rows FROM wave_stats WHERE tbl='reach'"),
        json!([[6]])
    );
    assert_eq!(
        rows(&e, "SELECT * FROM ends ORDER BY c0"),
        json!([[2], [3], [4]])
    );
    assert_eq!(
        rows(
            &e,
            "SELECT rootpage FROM temp.sqlite_schema WHERE name='reach'"
        ),
        root
    );
    assert_eq!(
        rows(
            &e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name='__sql_index_reach_1'"
        ),
        json!([[1]])
    );
    change(&mut e, json!([{"name":"edge","delete":[[2,3]]}]));
    assert_eq!(reachable(&e), json!([[1, 2], [3, 4]]));
    assert_eq!(rows(&e, "SELECT * FROM ends"), json!([[2]]));
    let noop = change(
        &mut e,
        json!([{"name":"edge","delete":[[99,100]],"insert":[[1,2],[1,2]]}]),
    );
    assert!(noop["programs"].as_array().unwrap().is_empty());
    assert_eq!(rows(&e, "SELECT * FROM unrelated"), json!([[42]]));
    // Rerun after a projection became a tracked table; ownership is by name, not object kind.
    e.run(".decl reach(a:number,b:number)\n.input reach\n.decl ends(n:number)\nends(n) :- reach(1,n).\n.output ends\n".into(),"python".into(),None).unwrap();
    assert_eq!(rows(&e, "SELECT * FROM ends"), json!([[2]]));
}

#[test]
fn recursive_deletion_removes_unsupported_cycles_and_preserves_alternate_support() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["n"],"rows":[[1],[2]]}).to_string())
        .unwrap();
    edge(&mut e, json!([[1, 2], [2, 1], [2, 3], [3, 3]]));
    let program = ".decl seed(n:number)\n.decl edge(a:number,b:number)\n.input seed,edge\n.decl live(n:number)\nlive(n) :- seed(n).\nlive(b) :- live(a),edge(a,b).\n.output live\n";
    e.run(program.into(), "python".into(), None).unwrap();
    let r = change(&mut e, json!([{"name":"seed","delete":[[1]]}]));
    assert_eq!(
        rows(&e, "SELECT * FROM live ORDER BY c0"),
        json!([[1], [2], [3]])
    );
    assert_eq!(r["programs"][0]["outputs"]["live"]["deleted"], 0);
    change(&mut e, json!([{"name":"seed","delete":[[2]]}]));
    assert_eq!(rows(&e, "SELECT * FROM live"), json!([]));
    change(&mut e, json!([{"name":"seed","insert":[[3]]}]));
    assert_eq!(rows(&e, "SELECT * FROM live"), json!([[3]]));
}

#[test]
fn deterministic_edit_sequences_match_fresh_execution_including_multi_atom_batches() {
    use std::collections::BTreeSet;
    let (_dir, mut e) = executor();
    let mut graph = BTreeSet::<Vec<i64>>::from([vec![0, 1], vec![1, 2], vec![2, 0], vec![2, 3]]);
    edge(&mut e, json!(graph));
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    let mut random = 41u64;
    for step in 0..60 {
        let mut insert = Vec::new();
        let mut delete = Vec::new();
        for _ in 0..2 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let pair = vec![(random % 7) as i64, ((random >> 16) % 7) as i64];
            if random & 256 == 0 {
                insert.push(pair);
            } else {
                delete.push(pair);
            }
        }
        for r in &delete {
            graph.remove(r);
        }
        for r in &insert {
            graph.insert(r.clone());
        }
        change(
            &mut e,
            json!([{"name":"edge","insert":insert,"delete":delete}]),
        );
        let (_fresh_dir, mut fresh) = executor();
        edge(&mut fresh, json!(graph));
        fresh.run(CLOSURE.into(), "python".into(), None).unwrap();
        assert_eq!(reachable(&e), reachable(&fresh), "edit {step}");
    }
}

#[test]
fn projections_keep_alternate_support_and_aggregates_negation_and_lattices_refresh_only_their_strata()
 {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"event","columns":["id","owner","value"],"rows":[[1,10,5],[2,10,3],[3,20,8]]}).to_string()).unwrap();
    let program = ".decl event(id:number,o:number,v:number)\n.input event\n.decl owner(o:number)\nowner(o) :- event(_,o,_).\n.decl totals(o:number,n:number)\ntotals(o,n) :- owner(o), n = count : {event(_,o,_)}.\n.decl least(o:number,v:min<>) widen 100\nleast(o,v) :- event(_,o,v).\n.decl absent(o:number)\nabsent(10) :- !owner(10).\n.output owner,totals,least,absent\n";
    e.run(program.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&e, "SELECT * FROM owner ORDER BY c0"),
        json!([[10], [20]])
    );
    for patch in [
        json!({"name":"event","delete":[[2,10,3]]}),
        json!({"name":"event","delete":[[1,10,5]],"insert":[[4,30,1]]}),
        json!({"name":"event","delete":[[4,30,1],[3,20,8]]}),
    ] {
        change(&mut e, json!([patch]));
        let current = rows(&e, "SELECT * FROM event ORDER BY id");
        let (_d, mut fresh) = executor();
        fresh
            .bind_relation(
                json!({"name":"event","columns":["id","owner","value"],"rows":current}).to_string(),
            )
            .unwrap();
        fresh.run(program.into(), "python".into(), None).unwrap();
        for table in ["owner", "totals", "least", "absent"] {
            assert_eq!(
                rows(&e, &format!("SELECT * FROM {table} ORDER BY 1")),
                rows(&fresh, &format!("SELECT * FROM {table} ORDER BY 1")),
                "{table}"
            );
        }
    }
}

#[test]
fn failed_updates_roll_back_facts_projection_migration_and_outputs_and_can_retry() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3]]));
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    e.run(".decl reach(a:number,b:number)\n.input reach\n.decl ends(n:number)\nends(n) :- reach(1,n).\n.output ends\n".into(),"python".into(),None).unwrap();
    let old_ends = rows(&e, "SELECT * FROM ends ORDER BY c0");
    assert_eq!(
        rows(&e, "SELECT type FROM temp.sqlite_schema WHERE name='ends'"),
        json!([["view"]])
    );
    let old = reachable(&e);
    e.set_limits(json!({"max_steps":100}).to_string()).unwrap();
    assert!(
        e.apply_changes(json!([{"name":"edge","insert":[[3,4]]}]).to_string(), None)
            .is_err()
    );
    e.set_limits("{}".into()).unwrap();
    assert_eq!(reachable(&e), old);
    assert_eq!(rows(&e, "SELECT * FROM ends ORDER BY c0"), old_ends);
    assert_eq!(
        rows(&e, "SELECT type FROM temp.sqlite_schema WHERE name='ends'"),
        json!([["view"]])
    );
    assert_eq!(
        rows(&e, "SELECT * FROM edge ORDER BY src"),
        json!([[1, 2], [2, 3]])
    );
    assert!(
        e.apply_changes(
            json!([{"name":"edge","insert":[[3,4],[4,5],[5,6]]}]).to_string(),
            Some(1)
        )
        .is_err()
    );
    assert_eq!(reachable(&e), old);
    assert_eq!(rows(&e, "SELECT * FROM ends ORDER BY c0"), old_ends);
    assert_eq!(
        rows(&e, "SELECT type FROM temp.sqlite_schema WHERE name='ends'"),
        json!([["view"]])
    );
    assert_eq!(
        rows(&e, "SELECT * FROM edge ORDER BY src"),
        json!([[1, 2], [2, 3]])
    );
    change(&mut e, json!([{"name":"edge","insert":[[3,4]]}]));
    assert_eq!(rows(&e, "SELECT count(*) FROM reach"), json!([[6]]));
    assert!(
        e.apply_changes(json!([{"name":"edge","insert":[[4]]}]).to_string(), None)
            .is_err()
    );
    assert_eq!(rows(&e, "SELECT count(*) FROM reach"), json!([[6]]));
    e.connection().execute_batch("BEGIN").unwrap();
    assert!(e.apply_changes("[]".into(), None).is_err());
    e.connection().execute_batch("ROLLBACK").unwrap();
}

#[test]
fn small_positive_edits_have_bounded_sql_work_over_a_large_retained_relation() {
    let (_dir, mut e) = executor();
    edge(
        &mut e,
        json!((0..511).map(|n| vec![n, n + 1]).collect::<Vec<_>>()),
    );
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    e.set_limits(json!({"max_steps":30000}).to_string())
        .unwrap();
    for pair in [[10000, 10001], [20000, 20001]] {
        let r = change(&mut e, json!([{"name":"edge","insert":[pair]}]));
        assert_eq!(r["programs"][0]["outputs"]["reach"]["inserted"], 1);
        assert!(
            r["programs"][0]["refreshed_strata"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let r = change(&mut e, json!([{"name":"edge","delete":[[10000,10001]]}]));
    assert_eq!(r["programs"][0]["outputs"]["reach"]["deleted"], 1);
}

#[test]
fn boolean_and_exact_symbol_facts_use_the_same_update_encoding() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"flag","columns":[],"rows":[]}).to_string())
        .unwrap();
    e.run(
        ".decl flag()\n.input flag\n.decl yes()\nyes() :- flag().\n.output yes\n".into(),
        "python".into(),
        None,
    )
    .unwrap();
    change(&mut e, json!([{"name":"flag","insert":[[]]}]));
    assert_eq!(rows(&e, "SELECT EXISTS(SELECT 1 FROM yes)"), json!([[1]]));
    change(&mut e, json!([{"name":"flag","delete":[[]]}]));
    assert_eq!(rows(&e, "SELECT EXISTS(SELECT 1 FROM yes)"), json!([[0]]));
    e.bind_relation(
        json!({"name":"label","columns":["id","s"],"types":["number","symbol"],"rows":[]})
            .to_string(),
    )
    .unwrap();
    change(
        &mut e,
        json!([{"name":"label","insert":[[{"integer":"9007199254740993"},"hello"]]}]),
    );
    assert_eq!(
        rows(&e, "SELECT id,sym_text(s) FROM label"),
        json!([[{"integer":"9007199254740993"},"hello"]])
    );
}

#[test]
fn shortest_path_retractions_and_global_aggregate_changes_match_fresh_runs() {
    let (_dir, mut e) = executor();
    let initial = json!([
        [0, 1, 5],
        [0, 2, 1],
        [2, 1, 1],
        [1, 3, 2],
        [2, 3, 10],
        [3, 2, 1]
    ]);
    e.bind_relation(json!({"name":"weighted","columns":["a","b","w"],"rows":initial}).to_string())
        .unwrap();
    let program = ".decl weighted(a:number,b:number,w:number)\n.input weighted\n.decl distance(n:number,v:min<>) widen 100\ndistance(0,0).\ndistance(b,d+w) :- distance(a,d), weighted(a,b,w).\n.decl total(n:number)\ntotal(n) :- n = count : {weighted(_,_,_)}.\n.decl conflict(k:number,v:flat<>)\nconflict(a,w) :- weighted(a,_,w).\n.output distance,total,conflict\n";
    e.run(program.into(), "python".into(), None).unwrap();
    for patch in [
        json!({"name":"weighted","delete":[[2,1,1]]}),
        json!({"name":"weighted","insert":[[0,3,1]],"delete":[[0,1,5],[0,2,1]]}),
    ] {
        change(&mut e, json!([patch]));
        let current = rows(&e, "SELECT * FROM weighted ORDER BY a,b,w");
        let (_d, mut fresh) = executor();
        fresh
            .bind_relation(
                json!({"name":"weighted","columns":["a","b","w"],"rows":current}).to_string(),
            )
            .unwrap();
        fresh.run(program.into(), "python".into(), None).unwrap();
        for r in ["distance", "total", "conflict"] {
            assert_eq!(
                rows(&e, &format!("SELECT * FROM {r} ORDER BY 1")),
                rows(&fresh, &format!("SELECT * FROM {r} ORDER BY 1")),
                "{r}"
            );
        }
    }
}

#[test]
fn nonlinear_recursion_and_simultaneous_multi_input_deletions_keep_set_semantics() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 1], [2, 3], [1, 4], [4, 3]]));
    let program = ".decl edge(a:number,b:number)\n.input edge\n.decl path(a:number,b:number)\npath(a,b) :- edge(a,b).\npath(a,c) :- path(a,b),path(b,c).\n.output path\n";
    e.run(program.into(), "python".into(), None).unwrap();
    for patch in [
        json!({"name":"edge","delete":[[1,2],[2,1]]}),
        json!({"name":"edge","insert":[[3,1]],"delete":[[4,3],[2,3]]}),
        json!({"name":"edge","delete":[[3,1],[1,4]]}),
    ] {
        change(&mut e, json!([patch]));
        let current = rows(&e, "SELECT * FROM edge ORDER BY src,dst");
        let (_d, mut fresh) = executor();
        edge(&mut fresh, current);
        fresh.run(program.into(), "python".into(), None).unwrap();
        assert_eq!(
            rows(&e, "SELECT * FROM path ORDER BY c0,c1"),
            rows(&fresh, "SELECT * FROM path ORDER BY c0,c1")
        );
    }
    for name in ["lefts", "rights"] {
        e.bind_relation(json!({"name":name,"columns":["a","b"],"rows":[[1,2],[2,3]]}).to_string())
            .unwrap();
    }
    e.run(".decl lefts(a:number,b:number)\n.decl rights(a:number,b:number)\n.input lefts,rights\n.decl pair(a:number,c:number)\npair(a,c) :- lefts(a,b),rights(b,c).\n.output pair\n".into(),"python".into(),None).unwrap();
    assert_eq!(rows(&e, "SELECT * FROM pair"), json!([[1, 3]]));
    change(
        &mut e,
        json!([{"name":"lefts","delete":[[1,2]]},{"name":"rights","delete":[[2,3]]}]),
    );
    assert_eq!(rows(&e, "SELECT * FROM pair"), json!([]));
    change(
        &mut e,
        json!([{"name":"lefts","insert":[[1,2]]},{"name":"rights","insert":[[2,3]]}]),
    );
    assert_eq!(rows(&e, "SELECT * FROM pair"), json!([[1, 3]]));
}
