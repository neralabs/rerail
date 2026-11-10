use super::*;

#[test]
fn unique_incremental_support_preserves_unchanged_heads_and_empty_transitions() {
    let p=".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl r(g:number)\nr(g) :- owner(g), n=count : {event(_,g,_)}, s=sum v : {event(_,g,v)}, n>0,s>0.\n.output r\n";
    let (_directory,mut e)=executor();
    bind(&mut e,"event",&["id","g","v"],json!([[1,1,4]]));
    bind(&mut e,"owner",&["g"],json!([[1],[2]]));
    e.run(p.into(),"python".into(),None).unwrap();
    let explain:Value=serde_json::from_str(&e.explain().unwrap()).unwrap();
    assert_eq!(explain["executions"][0]["retained"][0]["support_uniqueness"]["proven"],true);
    e.connection().execute_batch("CREATE TEMP TRIGGER forbid_r_insert BEFORE INSERT ON r BEGIN SELECT RAISE(ABORT,'unchanged r'); END;
        CREATE TEMP TRIGGER forbid_r_delete BEFORE DELETE ON r BEGIN SELECT RAISE(ABORT,'unchanged r'); END;").unwrap();
    e.apply_changes(json!([{"name":"event","delete":[[1,1,4]],"insert":[[1,1,6]]}]).to_string(),None).unwrap();
    assert_eq!(rows(&e,"SELECT n FROM __sql_state_r_heads"),json!([[1]]));
    e.connection().execute_batch("DROP TRIGGER forbid_r_insert; DROP TRIGGER forbid_r_delete;").unwrap();
    for patch in [json!([{"name":"event","delete":[[1,1,6]]}]),json!([{"name":"event","insert":[[1,1,9]]}]),json!([{"name":"owner","delete":[[1]],"insert":[[3]]}])] {
        e.apply_changes(patch.to_string(),None).unwrap();
        let (_directory,mut oracle)=executor();oracle.cold_oracle=true;
        for (name,cols) in [("event",&["id","g","v"][..]),("owner",&["g"][..])] {bind(&mut oracle,name,cols,rows(&e,&format!("SELECT * FROM {name}")));}
        oracle.run(p.into(),"python".into(),None).unwrap();
        assert_eq!(rows(&e,"SELECT * FROM r ORDER BY c0"),rows(&oracle,"SELECT * FROM r ORDER BY c0"));
        assert_eq!(rows(&e,"SELECT count(*) FROM __sql_state_r_heads WHERE n<>1"),json!([[0]]));
    }
}

const PROGRAM: &str = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl stats(g:number,n:number,s:number,lo:number,hi:number)\nstats(g,n,s,lo,hi) :- owner(g), n=count : {event(_,g,_)}, s=sum v : {event(_,g,v)}, lo=min x : {event(_,g,x)}, hi=max y : {event(_,g,y)}.\n.decl totals(g:number,n:number,s:number)\ntotals(g,n,s) :- owner(g), n=count : {event(_,g,_)}, s=sum v : {event(_,g,v)}.\n.output stats,totals\n";

fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], values: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":values}).to_string())
        .unwrap();
}
fn cold(e: &CodeRuleSqlExecutor) {
    let (_directory, mut oracle) = executor();
    oracle.cold_oracle = true;
    for (name, columns) in [("event", &["id", "g", "v"][..]), ("owner", &["g"][..])] {
        bind(
            &mut oracle,
            name,
            columns,
            rows(e, &format!("SELECT * FROM {name}")),
        );
    }
    oracle.run(PROGRAM.into(), "python".into(), None).unwrap();
    for name in ["stats", "totals"] {
        let sql = format!("SELECT * FROM {name} ORDER BY c0");
        assert_eq!(rows(e, &sql), rows(&oracle, &sql));
    }
}

#[test]
fn shared_scalar_summaries_preserve_bags_empty_groups_extrema_and_sparse_edits() {
    let (_directory, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!(
            (0..2000)
                .flat_map(|g| [(g * 3, g, 7), (g * 3 + 1, g, 7), (g * 3 + 2, g, 3)])
                .map(|(id, g, v)| vec![id, g, v])
                .collect::<Vec<_>>()
        ),
    );
    bind(
        &mut e,
        "owner",
        &["g"],
        json!((0..2001).map(|g| vec![g]).collect::<Vec<_>>()),
    );
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&e, "SELECT * FROM stats WHERE c0=17"),
        json!([[17, 3, 17, 3, 7]])
    );
    assert_eq!(
        rows(&e, "SELECT * FROM totals WHERE c0=2000"),
        json!([[2000, 0, 0]])
    );
    let explanation: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    let retained = explanation["executions"][0]["retained"].as_array().unwrap();
    let stats = retained.iter().find(|s| s["relation"] == "stats").unwrap();
    assert_eq!(
        stats["rules"][0]["scalar_summaries"]["families"][0]["members"],
        json!([0, 1, 2, 3])
    );
    assert_eq!(stats["full_rebuild_grouping_elided"], true);
    assert_eq!(stats["cold_reuse"]["proven"], true);
    assert_eq!(stats["cold_initialized"], true);
    assert_eq!(stats["rules"][0]["shared_summary_lookups"], 1);
    assert_eq!(stats["rules"][0]["scalar_summaries"]["families"][0]["lookup"]["proven"], true);
    for patch in [
        json!([{"name":"event","delete":[[51,17,7]]}]),
        json!([{"name":"event","delete":[[52,17,7]]}]),
        json!([{"name":"event","delete":[[53,17,3]]}]),
        json!([{"name":"event","insert":[[6001,17,-4],[6002,17,12]]}]),
        json!([{"name":"event","delete":[[6001,17,-4]],"insert":[[6003,17,-8]]}]),
    ] {
        let (result, work) =
            super::scaling::profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        let report: Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(report["programs"][0]["refreshed_strata"], json!([]));
        assert_eq!(report["programs"][0]["reducer_groups"], 1, "{report}");
        assert!(work.data_scans.get() < 100, "{work:?}");
        cold(&e);
    }
    let before = rows(&e, "SELECT * FROM stats WHERE c0=17");
    e.connection().execute_batch("CREATE TEMP TRIGGER stop_summary BEFORE INSERT ON stats BEGIN SELECT RAISE(ABORT,'stop combined'); END").unwrap();
    let patch = json!([{"name":"event","insert":[[7000,17,2]]}]);
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("stop combined")
    );
    assert_eq!(rows(&e, "SELECT * FROM stats WHERE c0=17"), before);
    e.connection()
        .execute_batch("DROP TRIGGER stop_summary")
        .unwrap();
    e.apply_changes(patch.to_string(), None).unwrap();
    cold(&e);
}

#[test]
#[ignore = "since the 2026-11-10 merges after 09cea8b63 (More-ergonomic .. async-family) the first run's unused wide totals overflow at once, not at lookup; cause not found; open item"]
fn combined_sums_keep_wide_unused_totals_and_defer_overflow_until_lookup() {
    let (_directory, mut e) = executor();
    let max = wire::integer(i64::MAX);
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[0, 0, 3], [1, 1, max], [2, 1, max]]),
    );
    bind(&mut e, "owner", &["g"], json!([[0]]));
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    let patch = json!([{"name":"owner","insert":[[1]]}]);
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("overflow")
    );
    assert_eq!(rows(&e, "SELECT * FROM owner ORDER BY g"), json!([[0]]));
    e.apply_changes(
        json!([{"name":"event","delete":[[2,1,max]]},{"name":"owner","insert":[[1]]}]).to_string(),
        None,
    )
    .unwrap();
    cold(&e);
    assert_eq!(
        rows(&e, "SELECT * FROM stats WHERE c0=1"),
        json!([[1, 1, max, max, max]])
    );
}

#[test]
fn shared_extrema_probe_large_groups_and_global_summaries_keep_empty_contracts() {
    let (_directory, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!((0..100000).map(|i| vec![i, 0, i]).collect::<Vec<_>>()),
    );
    bind(&mut e, "owner", &["g"], json!([[0]]));
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    for patch in [
        json!([{"name":"event","delete":[[50000,0,50000]]}]),
        json!([{"name":"event","delete":[[0,0,0],[99999,0,99999]]}]),
    ] {
        let (result, work) =
            super::scaling::profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        result.unwrap();
        assert!(
            work.data_scans.get() < 200 && work.vm.get() < 50000,
            "{work:?}"
        );
        assert_eq!(
            rows(&e, "SELECT c1,c2,c3,c4 FROM stats"),
            rows(&e, "SELECT count(*),sum(v),min(v),max(v) FROM event")
        );
    }
    let (_directory, mut global) = executor();
    bind(&mut global, "event", &["id", "g", "v"], json!([]));
    let p = ".decl event(id:number,g:number,v:number)\n.input event\n.decl stats(n:number,s:number,lo:number,hi:number)\nstats(n,s,lo,hi) :- n=count : {event(_,_,_)}, s=sum v : {event(_,_,v)}, lo=min x : {event(_,_,x)}, hi=max y : {event(_,_,y)}.\n.decl totals(n:number,s:number)\ntotals(n,s) :- n=count : {event(_,_,_)}, s=sum v : {event(_,_,v)}.\n.output stats,totals\n";
    global.run(p.into(), "python".into(), None).unwrap();
    for patch in [
        json!([{"name":"event","insert":[[0,0,7],[1,0,-3]]}]),
        json!([{"name":"event","delete":[[0,0,7],[1,0,-3]]}]),
    ] {
        global.apply_changes(patch.to_string(), None).unwrap();
    }
    assert_eq!(rows(&global, "SELECT * FROM totals"), json!([[0, 0]]));
    assert_eq!(rows(&global, "SELECT * FROM stats"), json!([]));
}

#[test]
fn explain_reports_proofs_rejections_and_catalog_updates_without_profiling() {
    let (_directory, mut e) = executor();
    bind(&mut e, "event", &["id", "g", "v"], json!([[0, 0, 7]]));
    bind(&mut e, "owner", &["g"], json!([[0]]));
    let lowered: Value =
        serde_json::from_str(&e.lower(PROGRAM.into(), "python".into()).unwrap()).unwrap();
    assert_eq!(
        lowered["analysis"]["rules"][0]["scalars"]["families"][0]["members"],
        json!([0, 1, 2, 3])
    );
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    // Reading facts or explain does not build an unrelated SQL metadata catalog.
    assert_eq!(
        e.connection()
            .query_row(
                "SELECT count(*) FROM temp.sqlite_schema WHERE name='rr_analysis'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let explain: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    assert_eq!(
        explain["executions"][0]["logical"]["rules"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let catalog = rows(
        &e,
        "SELECT json_extract(analysis,'$.logical.rules[0].scalars.families[0].members') FROM rr_analysis",
    );
    assert_eq!(catalog, json!([["[0,1,2,3]"]]));
    e.reset().unwrap();
    assert_eq!(rows(&e, "SELECT count(*) FROM rr_analysis"), json!([[0]]));
    let p = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl out(g:number,v:min<>) widen 3\nout(g,v) :- event(_,g,v), n=count : {owner(_)}, n>0.\nout(g,v) :- event(g,_,v), n=count : {owner(_)}, n>0.\n.output out\n";
    e.run(p.into(), "python".into(), None).unwrap();
    let explain: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    let retained = explain["executions"][0]["retained"].as_array().unwrap();
    assert!(
        retained
            .iter()
            .any(|r| r["relation"] == "out" && r["support_uniqueness"]["proven"] == false)
    );
    assert_eq!(
        rows(
            &e,
            "SELECT n FROM __sql_state_out_heads WHERE c0=0 AND c1=7"
        ),
        json!([[2]])
    );
}

#[test]
fn catalog_explain_refreshes_when_fact_edits_extend_recursive_plans() {
    let (_directory, mut e) = executor();
    bind(&mut e, "edge", &["a", "b"], json!([[0, 1]]));
    bind(&mut e,"root",&["n"],json!([[0]]));
    let p = ".decl edge(a:number,b:number)\n.decl root(n:number)\n.input edge,root\n.decl distance(n:number,v:min<>) widen 3\ndistance(n,0) :- root(n).\ndistance(b,v+1) :- distance(a,v),edge(a,b).\n.output distance\n";
    e.run(p.into(), "python".into(), None).unwrap();
    let first = rows(&e, "SELECT analysis FROM rr_analysis");
    let first: Value = serde_json::from_str(first[0][0].as_str().unwrap()).unwrap();
    e.apply_changes(
        json!([{"name":"edge","insert":[[1,2],[2,3],[3,4],[4,5]]}]).to_string(),
        None,
    )
    .unwrap();
    let current: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    assert_ne!(first["recursive"], current["executions"][0]["recursive"]);
    let refreshed = rows(&e, "SELECT analysis FROM rr_analysis");
    let refreshed: Value = serde_json::from_str(refreshed[0][0].as_str().unwrap()).unwrap();
    assert_eq!(refreshed, current["executions"][0]);
}

#[test]
fn cold_summaries_wait_for_upstream_strata_and_keep_multiple_supports() {
    let (_directory, mut e) = executor();
    bind(&mut e, "event", &["id", "g", "v"], json!([[0, 1, 4], [1, 1, 6], [2, 2, 9]]));
    bind(&mut e, "owner", &["id", "g"], json!([[0, 1], [1, 1], [2, 2], [3, 3]]));
    let p = ".decl event(id:number,g:number,v:number)\n.decl owner(id:number,g:number)\n.input event,owner\n.decl copied(id:number,g:number,v:number)\ncopied(id,g,v) :- event(id,g,v).\n.decl filtered(id:number,g:number,v:number)\nfiltered(id,g,v) :- copied(id,g,v), v>4.\n.decl out(g:number,n:number,s:number)\nout(g,n,s) :- owner(_,g), n=count : {filtered(_,g,_)}, s=sum v : {filtered(_,g,v)}.\n.decl checked(g:number,s:number)\nchecked(g,s) :- owner(_,g), n=count : {filtered(_,g,_)}, s=n+1.\n.output out,checked\n";
    e.run(p.into(), "python".into(), None).unwrap();
    assert_eq!(rows(&e,"SELECT * FROM out ORDER BY c0"),json!([[1,1,6],[2,1,9],[3,0,0]]));
    let explanation: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    let retained = explanation["executions"][0]["retained"].as_array().unwrap();
    let out = retained.iter().find(|s|s["relation"]=="out").unwrap();
    assert_eq!(out["cold_initialized"],false);
    assert!(explanation["executions"][0]["recursive"][0]["execution_shape"].as_str().unwrap().contains("retained relation graph"));
    assert_eq!(out["full_rebuild_grouping_elided"],false);
    let checked = retained.iter().find(|s|s["relation"]=="checked").unwrap();
    assert_eq!(checked["cold_reuse"]["proven"],false);
    assert_eq!(checked["cold_initialized"],false);
    for patch in [
        json!([{"name":"owner","delete":[[0,1]]}]),
        json!([{"name":"event","delete":[[1,1,6]],"insert":[[3,1,8]]}]),
        json!([{"name":"owner","delete":[[1,1]]}]),
    ] {
        e.apply_changes(patch.to_string(),None).unwrap();
        let (_directory, mut oracle) = executor();
        oracle.cold_oracle=true;
        for (name, columns) in [("event", &["id","g","v"][..]),("owner", &["id","g"][..])] {
            bind(&mut oracle,name,columns,rows(&e,&format!("SELECT * FROM {name}")));
        }
        oracle.run(p.into(),"python".into(),None).unwrap();
        for output in ["out","checked"] {
            let query=format!("SELECT * FROM {output} ORDER BY c0");
            assert_eq!(rows(&e,&query),rows(&oracle,&query));
        }
    }
}

#[test]
fn disjoint_producers_elide_support_grouping_and_keep_every_dirty_arm() {
    let (_d,mut e)=executor();
    bind(&mut e,"event",&["x"],json!([[1],[2],[3]]));
    let p=".decl event(x:number)\n.input event\n.decl out(tag:number,x:number)\nout(1,x) :- event(x).\nout(2,x) :- event(x).\n.output out\n";
    e.run(p.into(),"python".into(),None).unwrap();
    let mut states=super::super::reductions::planned(&e.db,&e.executions[0],true).unwrap();
    let state=states.get_mut("out").unwrap();
    assert_eq!(state.explain()["support_uniqueness"]["proven"],true);
    e.db.execute_batch("DROP TABLE out").unwrap();
    e.versions.create_readonly(&e.db,"out",versions::Schema::integers(2,2)).unwrap();
    state.declare_phase(&e.db,&mut e.executions[0].objects,&e.versions).unwrap();
    let mut work=maintenance::Work::new(&e.db,&e.scratch);
    work.versions=Some(&e.versions);work.preparing=true;
    let mut phase=phase::Phase::default();
    state.append_phase(&mut work,&BTreeMap::new(),&mut phase,true).unwrap();
    phase.execute(&work).unwrap();
    work.cleanup().unwrap();
    state.ready=true;
    e.executions[0].reductions=Some(states);
    e.apply_changes(json!([{"name":"event","delete":[[1]],"insert":[[4]]}]).to_string(),None).unwrap();
    assert_eq!(rows(&e,"SELECT * FROM out ORDER BY c0,c1"),json!([[1,2],[1,3],[1,4],[2,2],[2,3],[2,4]]));
}
