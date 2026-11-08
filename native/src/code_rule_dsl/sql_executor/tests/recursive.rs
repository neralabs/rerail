use super::scaling::profile;

#[test]
fn nonrecursive_lattice_walks_keep_aggregate_and_negative_blocks_incremental() {
    let (_d, mut e) = executor();
    let files=json!({"files":[{"path":"a.py","language":"python","source":"def first():\n    work()\ndef second():\n    other()\n"}]}).to_string();
    e.bind_snapshot(files.clone()).unwrap();
    bind(&mut e, "blocked", &["n"], json!([]));
    let p = ".decl node(d:number,n:node,k:symbol,s:number,e:number)\n.decl tape(n:node,p:node,s:number,d:number)\n.decl field(p:node,f:symbol,c:node)\n.decl blocked(n:node)\n.input node,tape,field,blocked\n.decl scope(k:symbol)\nscope(\"function_definition\").\n.decl score(n:node,v:min<>) widen 4\nscore(n,c) :- node(_,n,\"call\",_,_), !blocked(n), nearest(n,a,scope), c=count : {field(a,_,_)}.\n.output score\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(
        e.executions[0]
            .emitted
            .queries
            .iter()
            .flatten()
            .all(|q| q.ctes.is_empty())
    );
    let call = rows(&e, "SELECT c0 FROM score ORDER BY c0")[0][0].clone();
    for insert in [true, false, true, false] {
        let patch = if insert {
            json!([{"name":"blocked","insert":[[call]]}])
        } else {
            json!([{"name":"blocked","delete":[[call]]}])
        };
        let report: Value =
            serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        let (_d, mut fresh) = executor();
    fresh.cold_oracle=true;
        fresh.bind_snapshot(files.clone()).unwrap();
        bind(
            &mut fresh,
            "blocked",
            &["n"],
            rows(&e, "SELECT * FROM blocked"),
        );
        fresh.run(p.into(), "python".into(), None).unwrap();
        assert_eq!(
            rows(&e, "SELECT * FROM score ORDER BY c0"),
            rows(&fresh, "SELECT * FROM score ORDER BY c0")
        );
    }
}

#[test]
fn recursive_dependencies_that_cross_partition_keys_match_fresh_execution() {
    let (_d, mut e) = executor();
    bind(&mut e, "seed", &["g", "v"], json!([[17, 42]]));
    bind(&mut e, "owner", &["g"], json!([[17], [18]]));
    let p = ".decl seed(g:number,v:number)\n.decl owner(g:number)\n.input seed,owner\n.decl amount(g:number,v:max<>) widen 20\n.decl item(g:number,v:number)\nitem(g,v) :- seed(g,v).\nitem(g,v) :- amount(g,v), v < 2.\namount(g,n) :- owner(g), n=count : {item(_,_)}.\n.output amount,item\n";
    e.run(p.into(), "python".into(), None).unwrap();
    let before = rows(&e, "SELECT c1 FROM amount WHERE c0=18");
    for patch in [
        json!([{"name":"owner","delete":[[17]]}]),
        json!([{"name":"seed","delete":[[17,42]],"insert":[[19,47]]}]),
        json!([{"name":"owner","insert":[[17],[19]]}]),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        compare(
            &e,
            p,
            &[("seed", &["g", "v"]), ("owner", &["g"])],
            "SELECT * FROM amount ORDER BY c0",
        );
        compare(
            &e,
            p,
            &[("seed", &["g", "v"]), ("owner", &["g"])],
            "SELECT * FROM item ORDER BY c0,c1",
        );
    }
    assert_ne!(rows(&e, "SELECT c1 FROM amount WHERE c0=18"), before);
}

#[test]
fn recursive_iterations_restore_simultaneous_sql_changes_and_retry_atomically() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "source",
        &["id", "g", "a", "b", "w"],
        json!([[1, 17, 0, 1, 5], [2, 17, 1, 2, 3], [3, 18, 0, 1, 2]]),
    );
    bind(&mut e, "active", &["g"], json!([[17], [18]]));
    let producer = ".decl source(id:number,g:number,a:number,b:number,w:number)\n.decl active(g:number)\n.input source,active\n.decl weighted(g:number,a:number,b:number,w:min<>) widen 20\nweighted(g,a,b,w) :- source(_,g,a,b,w).\n.decl root(g:number)\nroot(g) :- active(g).\n.output weighted,root\n";
    e.run(producer.into(), "python".into(), None).unwrap();
    e.run(PATHS.into(), "python".into(), None).unwrap();
    let initial = rows(&e, "SELECT * FROM distance ORDER BY c0,c1");
    let patch = json!([{"name":"source","delete":[[1,17,0,1,5],[2,17,1,2,3]],"insert":[[4,17,0,1,9],[5,17,1,2,1]]},{"name":"active","delete":[[17]]}]);
    e.connection().execute_batch("CREATE TEMP TRIGGER stop_old_recursive BEFORE DELETE ON distance WHEN OLD.c0=17 BEGIN SELECT RAISE(ABORT,'stop old recursive input'); END").unwrap();
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("stop old recursive input")
    );
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0,c1"), initial);
    assert_eq!(
        rows(&e, "SELECT * FROM root ORDER BY c0"),
        json!([[17], [18]])
    );
    e.connection()
        .execute_batch("DROP TRIGGER stop_old_recursive")
        .unwrap();
    for patch in [
        patch,
        json!([{"name":"active","insert":[[17]]},{"name":"source","delete":[[4,17,0,1,9]],"insert":[[6,17,0,1,4]]}]),
    ] {
        let report: Value =
            serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap();
        assert_eq!(
            report["programs"][1]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert!(
            report["programs"][1]["recursive_frames"].as_u64().unwrap() > 0,
            "{report}"
        );
        let (_d, mut fresh) = executor();
    fresh.cold_oracle=true;
        bind(
            &mut fresh,
            "source",
            &["id", "g", "a", "b", "w"],
            rows(&e, "SELECT * FROM source"),
        );
        bind(
            &mut fresh,
            "active",
            &["g"],
            rows(&e, "SELECT * FROM active"),
        );
        fresh.run(producer.into(), "python".into(), None).unwrap();
        fresh.run(PATHS.into(), "python".into(), None).unwrap();
        assert_eq!(
            rows(&e, "SELECT * FROM distance ORDER BY c0,c1"),
            rows(&fresh, "SELECT * FROM distance ORDER BY c0,c1")
        );
    }
}
use super::*;

#[test]
fn recursive_counts_wake_full_queries_and_retract_independent_cycles() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "seed",
        &["g", "v"],
        json!((0..3000).map(|g| vec![g, 42]).collect::<Vec<_>>()),
    );
    bind(
        &mut e,
        "owner",
        &["g"],
        json!((0..3000).map(|g| vec![g]).collect::<Vec<_>>()),
    );
    let p = ".decl seed(g:number,v:number)\n.decl owner(g:number)\n.input seed,owner\n.decl amount(g:number,v:max<>) widen 20\n.decl item(g:number,v:number)\nitem(g,v) :- seed(g,v).\nitem(g,v) :- amount(g,v), v < 4.\namount(g,n) :- owner(g), n=count : {item(g,_)}.\n.output amount,item\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&e, "SELECT * FROM amount WHERE c0=17"),
        json!([[17, 5]])
    );
    assert_eq!(
        rows(&e, "SELECT * FROM item WHERE c0=17 ORDER BY c1"),
        json!([[17, 0], [17, 1], [17, 2], [17, 3], [17, 42]])
    );
    assert!(!e.executions[0].temporal.is_empty());
    for patch in [
        json!([{"name":"owner","delete":[[17]]},{"name":"seed","delete":[[17,42]]}]),
        json!([{"name":"owner","insert":[[17]]}]),
        json!([{"name":"seed","insert":[[17,42],[17,47]]}]),
        json!([{"name":"seed","delete":[[17,42],[17,47]]}]),
    ] {
        e.set_limits(json!({"max_steps":50000}).to_string())
            .unwrap();
        let (result, work) = profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        let report: Value =
            serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert!(
            report["programs"][0]["recursive_frames"].as_u64().unwrap() > 0,
            "{report}"
        );
        assert!(work.data_scans.get() < 1000, "{work:?}");
        e.set_limits("{}".into()).unwrap();
        compare(
            &e,
            p,
            &[("seed", &["g", "v"]), ("owner", &["g"])],
            "SELECT * FROM amount ORDER BY c0",
        );
        compare(
            &e,
            p,
            &[("seed", &["g", "v"]), ("owner", &["g"])],
            "SELECT * FROM item ORDER BY c0,c1",
        );
    }
}
fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], rows: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":rows}).to_string())
        .unwrap();
}
fn all(e: &CodeRuleSqlExecutor, sql: &str) -> Value {
    let result: Value =
        serde_json::from_str(&e.query(sql.into(), None, Some(300000)).unwrap()).unwrap();
    assert_eq!(result["truncated"], false);
    result["rows"].clone()
}

#[test]
fn recursive_widening_retracts_top_with_a_nonleading_partition_and_preserves_other_counters() {
    let (_d, mut e) = executor();
    bind(&mut e, "root", &["g"], json!([[1], [2]]));
    bind(
        &mut e,
        "weighted",
        &["g", "a", "b", "w"],
        json!([[1, 0, 1, 0], [1, 1, 1, -1], [2, 0, 1, 0], [2, 1, 1, -1]]),
    );
    let p = ".decl root(g:number)\n.decl weighted(g:number,a:number,b:number,w:number)\n.input root,weighted\n.decl distance(n:number,g:number,v:min<>) widen 2\ndistance(0,g,0) :- root(g).\ndistance(b,g,d+w) :- distance(a,g,d), weighted(g,a,b,w).\n.output distance\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    assert_eq!(
        rows(&e, "SELECT c2 < -1000000 FROM distance WHERE c0=1 AND c1=1"),
        json!([[1]])
    );
    let other = rows(
        &e,
        "SELECT * FROM __sql_changes_distance WHERE c1=2 ORDER BY c0",
    );
    for patch in [
        json!([{"name":"weighted","delete":[[1,1,1,-1]]}]),
        json!([{"name":"weighted","insert":[[1,1,1,-1]]}]),
        json!([{"name":"root","delete":[[1]]}]),
        json!([{"name":"root","insert":[[1]]},{"name":"weighted","delete":[[1,1,1,-1]]}]),
    ] {
        let report: Value =
            serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert!(report["programs"][0]["recursive_frames"].as_u64().unwrap() > 0);
        assert_eq!(
            rows(
                &e,
                "SELECT * FROM __sql_changes_distance WHERE c1=2 ORDER BY c0"
            ),
            other
        );
        compare(
            &e,
            p,
            &[("root", &["g"]), ("weighted", &["g", "a", "b", "w"])],
            "SELECT * FROM distance ORDER BY c0,c1",
        );
    }
}
const PATHS: &str = ".decl root(g:number)\n.decl weighted(g:number,a:number,b:number,w:number)\n.input root,weighted\n.decl distance(g:number,n:number,v:min<>) widen 100\ndistance(g,0,0) :- root(g).\ndistance(g,b,d+w) :- distance(g,a,d), weighted(g,a,b,w).\n.output distance\n";
fn compare(e: &CodeRuleSqlExecutor, p: &str, inputs: &[(&str, &[&str])], query: &str) {
    let (_d, mut fresh) = executor();
    fresh.cold_oracle=true;
    for (name, columns) in inputs {
        bind(
            &mut fresh,
            name,
            columns,
            all(e, &format!("SELECT * FROM {name}")),
        );
    }
    fresh.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(all(e, query), all(&fresh, query));
}

#[test]
fn partitioned_recursive_lattices_remove_cycles_with_bounded_work_and_match_fresh() {
    let (_d, mut e) = executor();
    let edge_rows = (0..20000)
        .flat_map(|g| {
            [
                [g, 0, 1, 5],
                [g, 0, 2, 1],
                [g, 2, 1, 1],
                [g, 1, 3, 2],
                [g, 3, 2, 1],
            ]
        })
        .collect::<Vec<_>>();
    bind(&mut e, "weighted", &["g", "a", "b", "w"], json!(edge_rows));
    bind(
        &mut e,
        "root",
        &["g"],
        json!((0..20000).map(|g| vec![g]).collect::<Vec<_>>()),
    );
    e.run(PATHS.into(), "python".into(), None).unwrap();
    assert!(!e.executions[0].temporal.is_empty());
    let inputs = [
        ("weighted", &["g", "a", "b", "w"][..]),
        ("root", &["g"][..]),
    ];
    e.connection().execute_batch("CREATE TEMP TRIGGER untouched_recursive_delete BEFORE DELETE ON distance WHEN OLD.c0<>17 BEGIN SELECT RAISE(ABORT,'unrelated recursive deletion'); END; CREATE TEMP TRIGGER untouched_recursive_insert BEFORE INSERT ON distance WHEN NEW.c0<>17 BEGIN SELECT RAISE(ABORT,'unrelated recursive insertion'); END").unwrap();
    for patch in [
        json!([{"name":"weighted","delete":[[17,2,1,1]]}]),
        json!([{"name":"weighted","delete":[[17,0,1,5],[17,0,2,1]]}]),
        json!([{"name":"weighted","insert":[[17,0,3,1]]}]),
        json!([{"name":"root","delete":[[17]]}]),
        json!([{"name":"root","insert":[[17]]},{"name":"weighted","delete":[[17,0,3,1]],"insert":[[17,0,2,9]]}]),
    ] {
        e.set_limits(json!({"max_steps":50000}).to_string())
            .unwrap();
        let (result, work) = profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        let report: Value =
            serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert!(
            report["programs"][0]["recursive_frames"].as_u64().unwrap() > 0,
            "{report}"
        );
        assert!(work.data_scans.get() < 1000, "{work:?}");
        eprintln!("recursive partition: {work:?}");
        e.set_limits("{}".into()).unwrap();
        compare(&e, PATHS, &inputs, "SELECT * FROM distance ORDER BY c0,c1");
    }
    let before = rows(&e, "SELECT * FROM distance ORDER BY c0,c1");
    let patch = json!([{"name":"weighted","insert":[[17,0,1,1]]}]);
    assert!(e.apply_changes(patch.to_string(), Some(1)).is_err());
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0,c1"), before);
    e.connection().execute_batch("CREATE TEMP TRIGGER stop_recursive BEFORE INSERT ON distance BEGIN SELECT RAISE(ABORT,'stop recursive'); END").unwrap();
    let patch = json!([{"name":"weighted","insert":[[17,0,1,1]]}]);
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("stop recursive")
    );
    assert_eq!(rows(&e, "SELECT * FROM distance ORDER BY c0,c1"), before);
    e.connection()
        .execute_batch("DROP TRIGGER stop_recursive")
        .unwrap();
    e.apply_changes(patch.to_string(), None).unwrap();
    compare(&e, PATHS, &inputs, "SELECT * FROM distance ORDER BY c0,c1");
}

#[test]
fn constant_facts_and_nonmonotone_rules_share_retained_support() {
    let (_d, mut e) = executor();
    bind(&mut e, "event", &["g", "v"], json!([[1, 5], [2, 9]]));
    let p = ".decl event(g:number,v:number)\n.input event\n.decl least(g:number,v:min<>) widen 8\nleast(1,3).\nleast(g,v) :- event(g,v).\n.decl total(n:number)\ntotal(1).\ntotal(n) :- n=count : {event(_,_)}.\n.output least,total\n";
    e.run(p.into(), "python".into(), None).unwrap();
    for patch in [
        json!([{"name":"event","delete":[[1,5]]}]),
        json!([{"name":"event","delete":[[2,9]]}]),
        json!([{"name":"event","insert":[[1,1]]}]),
    ] {
        let report: Value =
            serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        compare(
            &e,
            p,
            &[("event", &["g", "v"])],
            "SELECT * FROM least ORDER BY c0",
        );
        compare(
            &e,
            p,
            &[("event", &["g", "v"])],
            "SELECT * FROM total ORDER BY c0",
        );
    }
}
