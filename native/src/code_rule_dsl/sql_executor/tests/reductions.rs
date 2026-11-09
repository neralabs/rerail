use super::*;

#[test]
fn string_group_edits_skip_unrelated_groups() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["g", "k", "v"],
        json!((0..20000).map(|g| vec![g, 0, g]).collect::<Vec<_>>()),
    );
    bind(
        &mut e,
        "owner",
        &["g"],
        json!((0..20000).map(|g| vec![g]).collect::<Vec<_>>()),
    );
    let p = ".decl event(g:number,k:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl joined(g:number,s:symbol)\njoined(g,s) :- owner(g), s = join v by k sep \"|\" : {event(g,k,v)}.\n.output joined\n";
    e.run(p.into(), "python".into(), None).unwrap();
    e.set_limits(json!({"max_steps":30000}).to_string())
        .unwrap();
    let (result, work) = super::scaling::profile(&mut e, |e| {
        e.apply_changes(
            json!([{"name":"event","insert":[[17,1,30]]}]).to_string(),
            None,
        )
    });
    let report: Value =
        serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
    assert_eq!(report["programs"][0]["refreshed_strata"], json!([]));
    assert_eq!(report["programs"][0]["context_rows"], 1);
    assert!(work.data_scans.get() < 100, "{work:?}");
    eprintln!("string group: {work:?}");
    e.set_limits("{}".into()).unwrap();
    assert_eq!(
        rows(&e, "SELECT sym_text(c1) FROM joined WHERE c0=17"),
        json!([["17|30"]])
    );
}

#[test]
fn dependent_aggregate_keys_follow_old_and_new_dependencies_without_scanning_contexts() {
    let (_d, mut e) = executor();
    let p = ".decl event(g:number,v:number)\n.decl bucket(k:number,v:number)\n.decl owner(g:number)\n.input event,bucket,owner\n.decl result(g:number,n:number,s:number,m:number)\nresult(g,n,s,m) :- owner(g), n = max v : {event(g,v)}, s = sum v : {bucket(n,v)}, m = count : {bucket(s,_)}.\n.output result\n";
    bind(
        &mut e,
        "event",
        &["g", "v"],
        json!((0..20000).map(|g| vec![g, g]).collect::<Vec<_>>()),
    );
    bind(
        &mut e,
        "owner",
        &["g"],
        json!((0..20000).map(|g| vec![g]).collect::<Vec<_>>()),
    );
    bind(
        &mut e,
        "bucket",
        &["k", "v"],
        json!((0..20000).map(|g| vec![g, g]).collect::<Vec<_>>()),
    );
    e.run(p.into(), "python".into(), None).unwrap();
    for patch in [
        json!([{"name":"event","delete":[[1,1]],"insert":[[1,21000]]}]),
        json!([{"name":"bucket","insert":[[21000,22000]]}]),
        json!([{"name":"bucket","insert":[[22000,1]]}]),
        json!([{"name":"bucket","delete":[[1,1]],"insert":[[1,4]]}]),
        json!([{"name":"event","delete":[[1,21000]],"insert":[[1,1]]},{"name":"bucket","delete":[[1,4]],"insert":[[1,3]]}]),
        json!([{"name":"owner","delete":[[1]]}]),
        json!([{"name":"owner","insert":[[1]]},{"name":"event","delete":[[1,1]]}]),
    ] {
        e.set_limits(json!({"max_steps":30000}).to_string())
            .unwrap();
        let (result, work) =
            super::scaling::profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        let report: Value =
            serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert!(work.data_scans.get() < 100, "{work:?}");
        assert!(
            report["programs"][0]["context_rows"].as_u64().unwrap() < 10,
            "{report}"
        );
        e.set_limits("{}".into()).unwrap();
        compare(
            &e,
            p,
            &[
                ("event", &["g", "v"]),
                ("bucket", &["k", "v"]),
                ("owner", &["g"]),
            ],
            &["result"],
        );
    }
    e.connection().execute_batch("CREATE TEMP TRIGGER stop_dependent BEFORE INSERT ON result BEGIN SELECT RAISE(ABORT,'stop dependent'); END").unwrap();
    let before = ordered(&e, "result");
    let patch = json!([{"name":"event","insert":[[1,17]]}]);
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("stop dependent")
    );
    assert_eq!(ordered(&e, "result"), before);
    e.connection()
        .execute_batch("DROP TRIGGER stop_dependent")
        .unwrap();
    change(&mut e, patch);
    compare(
        &e,
        p,
        &[
            ("event", &["g", "v"]),
            ("bucket", &["k", "v"]),
            ("owner", &["g"]),
        ],
        &["result"],
    );
}

fn change(e: &mut CodeRuleSqlExecutor, patch: Value) -> Value {
    serde_json::from_str(
        &e.apply_changes(patch.to_string(), None)
            .unwrap_or_else(|e| panic!("{patch}: {e}")),
    )
    .unwrap()
}
fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], values: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":values}).to_string())
        .unwrap();
}
fn all(e: &CodeRuleSqlExecutor, sql: &str) -> Value {
    let result: Value =
        serde_json::from_str(&e.query(sql.into(), None, Some(300000)).unwrap()).unwrap();
    assert_eq!(result["truncated"], false);
    result["rows"].clone()
}
fn ordered(e: &CodeRuleSqlExecutor, table: &str) -> Value {
    let width = e
        .connection()
        .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
        .unwrap()
        .column_count();
    all(
        e,
        &format!(
            "SELECT * FROM {table} ORDER BY {}",
            (1..=width)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
    )
}
fn compare(e: &CodeRuleSqlExecutor, program: &str, inputs: &[(&str, &[&str])], outputs: &[&str]) {
    let (_d, mut fresh) = executor();
    for (name, columns) in inputs {
        bind(
            &mut fresh,
            name,
            columns,
            all(e, &format!("SELECT * FROM {name}")),
        );
    }
    fresh.run(program.into(), "python".into(), None).unwrap();
    for out in outputs {
        assert_eq!(ordered(e, out), ordered(&fresh, out), "{out}");
    }
}
const AGG: &str = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl totals(g:number,n:number,s:number,lo:number,hi:number)\ntotals(g,n,s,lo,hi) :- owner(g), n = count : {event(_,g,_)}, s = sum v : {event(_,g,v)}, lo = min v : {event(_,g,v)}, hi = max v : {event(_,g,v)}.\n.decl global(n:number,s:number)\nglobal(n,s) :- n = count : {event(_,_,_)}, s = sum v : {event(_,_,v)}.\n.decl empty()\nempty() :- !event(_,_,_).\n.output totals,global,empty\n";

#[test]
fn ordered_string_groups_preserve_text_order_duplicates_defaults_and_rollback() {
    let (_d, mut e) = executor();
    let p = ".decl part(id:number,g:number,k:symbol,v:symbol)\n.decl owner(g:number)\n.input part,owner\n.decl joined(g:number,s:symbol)\njoined(g,s) :- owner(g), s = join v by (k,g) sep \"|\" : {part(_,g,k,v)}.\n.output joined\n";
    bind(
        &mut e,
        "part",
        &["id", "g", "k", "v"],
        json!([
            [1, 1, "2", "b"],
            [2, 1, "10", "a"],
            [3, 1, "2", "b"],
            [4, 2, "x", "é"]
        ]),
    );
    bind(&mut e, "owner", &["g"], json!([[1], [2], [3]]));
    e.run(p.into(), "python".into(), None).unwrap();
    let decoded =
        |e: &CodeRuleSqlExecutor| all(e, "SELECT c0,sym_text(c1) FROM joined ORDER BY c0");
    assert_eq!(decoded(&e), json!([[1, "a|b"], [2, "é"], [3, ""]]));
    for patch in [
        json!([{"name":"part","delete":[[1,1,"2","b"]]}]),
        json!([{"name":"part","delete":[[3,1,"2","b"]],"insert":[[5,1,"10","c"],[6,2,"a","z"]]}]),
        json!([{"name":"part","delete":[[2,1,"10","a"],[5,1,"10","c"],[4,2,"x","é"],[6,2,"a","z"]]}]),
        json!([{"name":"part","insert":[[7,3,"b","new"]]},{"name":"owner","delete":[[3]]}]),
        json!([{"name":"owner","insert":[[3]]}]),
    ] {
        let report = change(&mut e, patch);
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        let (_d, mut fresh) = executor();
        bind(
            &mut fresh,
            "part",
            &["id", "g", "k", "v"],
            all(&e, "SELECT id,g,sym_text(k),sym_text(v) FROM part"),
        );
        bind(&mut fresh, "owner", &["g"], all(&e, "SELECT * FROM owner"));
        fresh.run(p.into(), "python".into(), None).unwrap();
        assert_eq!(decoded(&e), decoded(&fresh));
    }
    let before = decoded(&e);
    e.connection().execute_batch("CREATE TEMP TRIGGER stop_string BEFORE INSERT ON joined BEGIN SELECT RAISE(ABORT,'stop string'); END").unwrap();
    let patch = json!([{"name":"part","insert":[[8,3,"a","first"]]}]);
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("stop string")
    );
    assert_eq!(decoded(&e), before);
    e.connection()
        .execute_batch("DROP TRIGGER stop_string")
        .unwrap();
    change(&mut e, patch);
    assert_eq!(decoded(&e), json!([[1, ""], [2, ""], [3, "first|new"]]));
}

#[test]
fn count_sum_and_extrema_keep_multiplicities_empty_groups_and_global_defaults() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 10, 3], [2, 10, 3], [3, 10, 8], [4, 20, -2]]),
    );
    bind(&mut e, "owner", &["g"], json!([[10], [20], [30]]));
    e.run(AGG.into(), "python".into(), None).unwrap();
    for batch in [
        json!([{"name":"event","delete":[[1,10,3]]}]),
        json!([{"name":"event","delete":[[2,10,3]],"insert":[[5,10,9],[6,10,-4]]}]),
        json!([{"name":"event","delete":[[3,10,8],[4,20,-2],[5,10,9],[6,10,-4]]}]),
        json!([{"name":"event","insert":[[7,30,5]]},{"name":"owner","delete":[[30]]}]),
        json!([{"name":"owner","insert":[[30]]}]),
    ] {
        let report = change(&mut e, batch);
        assert!(
            report["programs"][0]["refreshed_strata"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{report}"
        );
        compare(
            &e,
            AGG,
            &[("event", &["id", "g", "v"]), ("owner", &["g"])],
            &["totals", "global", "empty"],
        );
    }
}

#[test]
fn negation_changes_only_at_zero_crossings_and_preserves_alternate_rule_support() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "candidate",
        &["id", "g"],
        json!([[1, 10], [2, 10], [3, 20]]),
    );
    bind(&mut e, "blocked", &["id", "g"], json!([[1, 10], [2, 10]]));
    bind(&mut e, "direct", &["g"], json!([[10]]));
    let p = ".decl candidate(id:number,g:number)\n.decl blocked(id:number,g:number)\n.decl direct(g:number)\n.input candidate,blocked,direct\n.decl allowed(g:number)\nallowed(g) :- candidate(_,g), !blocked(_,g).\nallowed(g) :- direct(g).\n.decl seen(g:number)\nseen(g) :- allowed(g).\n.output allowed,seen\n";
    e.run(p.into(), "python".into(), None).unwrap();
    for batch in [
        json!([{"name":"blocked","insert":[[4,10]]}]),
        json!([{"name":"blocked","delete":[[1,10],[2,10]]}]),
        json!([{"name":"direct","delete":[[10]]}]),
        json!([{"name":"blocked","delete":[[4,10]]}]),
        json!([{"name":"candidate","delete":[[1,10]]}]),
        json!([{"name":"blocked","insert":[[5,10]]},{"name":"candidate","delete":[[2,10]],"insert":[[4,30]]}]),
    ] {
        let report = change(&mut e, batch);
        assert!(
            report["programs"][0]["refreshed_strata"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{report}"
        );
        compare(
            &e,
            p,
            &[
                ("candidate", &["id", "g"]),
                ("blocked", &["id", "g"]),
                ("direct", &["g"]),
            ],
            &["allowed", "seen"],
        );
    }
}

#[test]
fn joined_contexts_handle_simultaneous_inserts_deletes_and_reducer_filters() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "lefts",
        &["a", "b"],
        json!([[1, 2], [1, 3], [2, 3]]),
    );
    bind(&mut e, "rights", &["b", "g"], json!([[2, 10], [3, 10]]));
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 10, 4], [2, 10, 4]]),
    );
    let p = ".decl lefts(a:number,b:number)\n.decl rights(b:number,g:number)\n.decl event(id:number,g:number,v:number)\n.input lefts,rights,event\n.decl accepted(a:number,s:number)\naccepted(a,s) :- lefts(a,b), rights(b,g), n = count : {event(_,g,_)}, n>1, s = sum v : {event(_,g,v)}.\n.output accepted\n";
    e.run(p.into(), "python".into(), None).unwrap();
    for batch in [
        json!([{"name":"lefts","delete":[[1,2]]},{"name":"rights","delete":[[2,10]]}]),
        json!([{"name":"lefts","insert":[[1,2]]},{"name":"rights","insert":[[2,10]]},{"name":"event","delete":[[1,10,4]]}]),
        json!([{"name":"event","insert":[[3,10,8],[4,20,9]]},{"name":"rights","insert":[[2,20]]}]),
    ] {
        let r = change(&mut e, batch);
        assert!(
            r["programs"][0]["refreshed_strata"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{r}"
        );
        compare(
            &e,
            p,
            &[
                ("lefts", &["a", "b"]),
                ("rights", &["b", "g"]),
                ("event", &["id", "g", "v"]),
            ],
            &["accepted"],
        );
    }
}

#[test]
fn small_warm_updates_do_not_scan_large_groups_or_all_outer_candidates() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!((0..100000).map(|i| vec![i, 1, i]).collect::<Vec<_>>()),
    );
    bind(
        &mut e,
        "owner",
        &["g"],
        json!((0..100000).map(|i| vec![i]).collect::<Vec<_>>()),
    );
    let p = format!(
        "{AGG}\n.decl vacant(g:number)\nvacant(g) :- owner(g), !event(_,g,_).\n.output vacant\n"
    );
    e.run(p.clone(), "python".into(), None).unwrap();
    change(&mut e, json!([{"name":"event","insert":[[100000,1,17]]}]));
    e.set_limits(json!({"max_steps":60000}).to_string())
        .unwrap();
    for batch in [
        json!([{"name":"event","delete":[[100000,1,17]]}]),
        json!([{"name":"event","delete":[[0,1,0]]}]),
        json!([{"name":"event","insert":[[200000,2,99]]}]),
        json!([{"name":"event","delete":[[200000,2,99]]}]),
    ] {
        let report = change(&mut e, batch);
        let r = &report["programs"][0];
        assert!(r["refreshed_strata"].as_array().unwrap().is_empty(), "{r}");
        assert!(r["context_rows"].as_u64().unwrap() <= 5, "{r}");
    }
    e.set_limits("{}".into()).unwrap();
    compare(
        &e,
        &p,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["totals", "global", "empty", "vacant"],
    );
}

#[test]
fn unused_sum_groups_keep_wide_support_and_fail_only_when_read() {
    let (_d, mut e) = executor();
    let max = wire::integer(i64::MAX);
    let min = wire::integer(i64::MIN);
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 1, 2], [2, 2, max], [3, 2, 1]]),
    );
    bind(&mut e, "owner", &["g"], json!([[1]]));
    let p = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl total(g:number,v:number)\ntotal(g,v) :- owner(g), v=sum x : {event(_,g,x)}.\n.output total\n";
    e.run(p.into(), "python".into(), None).unwrap();
    change(&mut e, json!([{"name":"event","insert":[[4,2,1]]}]));
    assert!(
        e.apply_changes(json!([{"name":"owner","insert":[[2]]}]).to_string(), None)
            .unwrap_err()
            .reason
            .contains("integer overflow")
    );
    assert_eq!(rows(&e, "SELECT * FROM owner"), json!([[1]]));
    change(
        &mut e,
        json!([{"name":"event","delete":[[2,2,max]]},{"name":"owner","insert":[[2]]}]),
    );
    compare(
        &e,
        p,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["total"],
    );
    change(
        &mut e,
        json!([{"name":"owner","delete":[[2]]},{"name":"event","delete":[[3,2,1],[4,2,1]],"insert":[[5,2,min],[6,2,-1]]}]),
    );
    assert!(
        e.apply_changes(json!([{"name":"owner","insert":[[2]]}]).to_string(), None)
            .unwrap_err()
            .reason
            .contains("integer overflow")
    );
    assert_eq!(rows(&e, "SELECT * FROM total"), json!([[1, 2]]));
    change(
        &mut e,
        json!([{"name":"event","delete":[[6,2,-1]]},{"name":"owner","insert":[[2]]}]),
    );
    assert_eq!(rows(&e, "SELECT c1 FROM total WHERE c0=2"), json!([[min]]));
    compare(
        &e,
        p,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["total"],
    );
}

#[test]
fn overflow_and_budget_failures_restore_reducer_state_and_allow_retry() {
    let (_d, mut e) = executor();
    let large = wire::integer(i64::MAX);
    bind(&mut e, "event", &["id", "g", "v"], json!([[1, 1, large]]));
    bind(&mut e, "owner", &["g"], json!([[1]]));
    e.run(AGG.into(), "python".into(), None).unwrap();
    // Keep cancellation deterministic as the optimizer reduces VM work.
    e.set_limits(json!({"max_steps":1}).to_string()).unwrap();
    assert!(
        e.apply_changes(json!([{"name":"owner","insert":[[2]]}]).to_string(), None)
            .is_err()
    );
    e.set_limits("{}".into()).unwrap();
    change(&mut e, json!([{"name":"owner","insert":[[2]]}]));
    assert!(
        e.apply_changes(
            json!([{"name":"event","insert":[[2,1,1]]}]).to_string(),
            None
        )
        .is_err()
    );
    assert_eq!(rows(&e, "SELECT * FROM global"), json!([[1, large]]));
    assert_eq!(rows(&e, "SELECT count(*) FROM event"), json!([[1]]));
    e.set_limits(json!({"max_steps":1}).to_string()).unwrap();
    let interrupted = e
        .apply_changes(
            json!([{"name":"event","insert":[[2,1,0]]}]).to_string(),
            None,
        )
        .unwrap_err();
    assert!(interrupted.reason.contains("interrupt"), "{interrupted}");
    e.set_limits("{}".into()).unwrap();
    compare(
        &e,
        AGG,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["totals", "global", "empty"],
    );

    change(
        &mut e,
        json!([{"name":"event","delete":[[1,1,large]],"insert":[[1,1,wire::integer(i64::MAX-1)]]}]),
    );
    compare(
        &e,
        AGG,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["totals", "global", "empty"],
    );
}

#[test]
fn deterministic_mixed_edits_match_fresh_execution_with_composite_keys_and_filters() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 1, 3], [2, 1, -2], [3, 2, 7]]),
    );
    bind(&mut e, "owner", &["g"], json!([[0], [1], [2], [3]]));
    let p = format!(
        "{AGG}\n.decl filtered(g:number,n:number)\nfiltered(g,n) :- owner(g), n = count : {{event(_,g,7)}}, n>0.\n.decl unblocked(g:number)\nunblocked(g) :- owner(g), !event(_,g,7).\n.decl pairs(g:number,v:number,n:number)\npairs(g,v,n) :- event(_,g,v), n = count : {{event(_,g,v)}}.\n.output filtered,unblocked,pairs\n"
    );
    e.run(p.clone(), "python".into(), None).unwrap();
    let mut random = 89u64;
    for step in 0..40 {
        let mut insert = Vec::new();
        let mut delete = Vec::new();
        for _ in 0..3 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let row = vec![
                (random % 12) as i64,
                ((random >> 8) % 4) as i64,
                ((random >> 16) % 11) as i64 - 3,
            ];
            if random & 128 == 0 {
                insert.push(row)
            } else {
                delete.push(row)
            }
        }
        let current = all(&e, "SELECT * FROM event");
        if step % 3 == 0
            && let Some(row) = current.as_array().unwrap().first()
        {
            delete.push(serde_json::from_value(row.clone()).unwrap());
        }
        let patch = json!([{"name":"event","insert":insert,"delete":delete}]);
        let report = change(&mut e, patch);
        for program in report["programs"].as_array().unwrap() {
            assert!(
                program["refreshed_strata"].as_array().unwrap().is_empty(),
                "edit {step}: {report}"
            );
        }
        compare(
            &e,
            &p,
            &[("event", &["id", "g", "v"]), ("owner", &["g"])],
            &[
                "totals",
                "global",
                "empty",
                "filtered",
                "unblocked",
                "pairs",
            ],
        );
    }
}

#[test]
fn dependent_aggregate_correlations_are_retained_without_refresh() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 1, 5], [2, 1, 8], [3, 2, 9]]),
    );
    bind(&mut e, "owner", &["g"], json!([[1], [2]]));
    let p = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl nested(g:number,m:number)\nnested(g,m) :- owner(g), n = count : {event(_,g,_)}, m = count : {event(_,n,_)}.\n.output nested\n";
    e.run(p.into(), "python".into(), None).unwrap();
    let report = change(&mut e, json!([{"name":"event","delete":[[1,1,5]]}]));
    assert_eq!(report["programs"][0]["refreshed_strata"], json!([]));
    compare(
        &e,
        p,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["nested"],
    );
}

#[test]
fn retained_support_is_owned_by_the_program_and_recreated_after_reset() {
    let (_d, mut e) = executor();
    bind(&mut e, "event", &["id", "g", "v"], json!([[1, 1, 5]]));
    bind(&mut e, "owner", &["g"], json!([[1]]));
    e.run(AGG.into(), "python".into(), None).unwrap();
    change(&mut e, json!([{"name":"event","insert":[[2,1,8]]}]));
    assert!(rows(&e,"SELECT count(*) FROM temp.sqlite_schema WHERE type='table' AND name LIKE '__sql_state_%'")[0][0].as_u64().unwrap()>0);
    e.reset().unwrap();
    assert_eq!(
        rows(
            &e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name LIKE '__sql_state_%'"
        ),
        json!([[0]])
    );
    e.run(AGG.into(), "python".into(), None).unwrap();
    change(&mut e, json!([{"name":"event","delete":[[1,1,5]]}]));
    compare(
        &e,
        AGG,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["totals", "global", "empty"],
    );
    bind(&mut e, "event", &["id", "g", "v"], json!([[3, 1, 9]]));
    assert_eq!(
        rows(
            &e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name LIKE '__sql_state_%'"
        ),
        json!([[0]])
    );
    e.run(AGG.into(), "python".into(), None).unwrap();
    change(&mut e, json!([{"name":"event","insert":[[4,1,4]]}]));
    compare(
        &e,
        AGG,
        &[("event", &["id", "g", "v"]), ("owner", &["g"])],
        &["totals", "global", "empty"],
    );
}

#[test]
fn initial_execution_prepares_reducers_and_edits_skip_unrelated_state() {
    let (_d, mut e) = executor();
    bind(&mut e, "small", &["id"], json!([[1]]));
    bind(
        &mut e,
        "large",
        &["id"],
        json!((0..20000).map(|i| vec![i]).collect::<Vec<_>>()),
    );
    let p = ".decl small(id:number)\n.decl large(id:number)\n.input small,large\n.decl count_small(n:number)\ncount_small(n) :- n = count : {small(_)}.\n.decl count_large(n:number)\ncount_large(n) :- n = count : {large(_)}.\n.output count_small,count_large\n";
    let initial: Value =
        serde_json::from_str(&e.run(p.into(), "python".into(), None).unwrap()).unwrap();
    assert_eq!(
        initial["preparation"]["initializations"],
        json!(["count_large", "count_small"])
    );
    e.set_limits(json!({"max_steps":10000}).to_string())
        .unwrap();
    let report = change(&mut e, json!([{"name":"small","insert":[[2]]}]));
    assert_eq!(
        report["programs"][0]["incremental_strata"],
        json!(["count_small"])
    );
    assert_eq!(report["initializations"], json!([]));
    assert_eq!(rows(&e, "SELECT * FROM count_large"), json!([[20000]]));
    e.set_limits("{}".into()).unwrap();
    change(&mut e, json!([{"name":"large","insert":[[20000]]}]));
    e.set_limits(json!({"max_steps":10000}).to_string())
        .unwrap();
    let warm = change(&mut e, json!([{"name":"large","delete":[[20000]]}]));
    assert_eq!(warm["initializations"], json!([]));
    e.set_limits("{}".into()).unwrap();
    compare(
        &e,
        p,
        &[("small", &["id"]), ("large", &["id"])],
        &["count_small", "count_large"],
    );
}

#[test]
fn recursive_producers_feed_actual_tuple_deltas_into_dependent_reducers() {
    let (_d, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3], [3, 1], [1, 4]]));
    bind(
        &mut e,
        "candidate",
        &["n"],
        json!([[1], [2], [3], [4], [5]]),
    );
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    let consumer = ".decl reach(a:number,b:number)\n.decl candidate(n:number)\n.input reach,candidate\n.decl total(n:number)\ntotal(n) :- n = count : {reach(_,_)}.\n.decl no_incoming(n:number)\nno_incoming(n) :- candidate(n), !reach(_,n).\n.output total,no_incoming\n";
    e.run(consumer.into(), "python".into(), None).unwrap();
    for batch in [
        json!([{"name":"edge","delete":[[2,3]]}]),
        json!([{"name":"edge","insert":[[2,5],[5,3]]}]),
        json!([{"name":"edge","delete":[[3,1],[1,4]],"insert":[[5,4]]}]),
    ] {
        let report = change(&mut e, batch);
        let dependent = report["programs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["program"] == 1)
            .unwrap();
        assert!(
            dependent["refreshed_strata"].as_array().unwrap().is_empty(),
            "{report}"
        );
        compare(
            &e,
            consumer,
            &[("reach", &["c0", "c1"]), ("candidate", &["n"])],
            &["total", "no_incoming"],
        );
    }
}
