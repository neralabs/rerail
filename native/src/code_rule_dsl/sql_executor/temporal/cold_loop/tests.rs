use serde_json::json;
use super::super::super::tests::{executor, rows};
use super::*;

struct FullLoop(bool);
impl Drop for FullLoop {
    fn drop(&mut self) {
        FORCE_LOOP.set(self.0);
    }
}
fn full_loop() -> FullLoop {
    FullLoop(FORCE_LOOP.replace(true))
}

const PROGRAM: &str = ".decl seed(g:number,n:number)\n.decl edge(g:number,a:number,b:number)\n.input seed,edge\n.decl a(g:number,n:number)\n.decl b(g:number,n:number)\n.decl observed(g:number,n:number)\na(g,n) :- seed(g,n).\nb(g,y) :- a(g,x),edge(g,x,y).\na(g,n) :- b(g,n).\nobserved(g,n) :- b(g,n).\n.output a,b,observed\n";
fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], values: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":values}).to_string())
        .unwrap();
}
fn check(e: &mut CodeRuleSqlExecutor) {
    let (_dir, mut oracle) = executor();
    oracle.cold_oracle = true;
    for (name, columns) in [("seed", vec!["g", "n"]), ("edge", vec!["g", "a", "b"])] {
        let values = rows(e, &format!("SELECT * FROM {name}"));
        bind(&mut oracle, name, &columns, values);
    }
    oracle
        .run(PROGRAM.into(), "python".into(), Some(256))
        .unwrap();
    for name in ["a", "b", "observed"] {
        let query = format!("SELECT * FROM {name} ORDER BY c0,c1");
        assert_eq!(rows(e, &query), rows(&mut oracle, &query), "{name}");
    }
}
#[test]
fn retained_sql_loop_keeps_intermediate_frontiers_and_warm_supports() {
    let _full = full_loop();
    let (_dir, mut e) = executor();
    bind(&mut e, "seed", &["g", "n"], json!([[0, 0], [1, 0]]));
    bind(
        &mut e,
        "edge",
        &["g", "a", "b"],
        json!(
            (0..2)
                .flat_map(|g| (0..18).map(move |n| [g, n, (n + 1) % 18]))
                .collect::<Vec<_>>()
        ),
    );
    e.run(PROGRAM.into(), "python".into(), Some(128)).unwrap();
    let trace = &e.executions[0].temporal[&0];
    assert!(trace.frames.len() > 18);
    assert_eq!(
        trace.frames.iter().map(|f| f.phase_batches).sum::<usize>(),
        1,
        "{}",
        trace.explain()
    );
    assert_eq!(
        trace.frames[0].observation["sql_version_loop"]["planned_submissions"],
        1
    );
    check(&mut e);
    assert_eq!(rows(&mut e, "SELECT count(*) FROM observed"), json!([[36]]));
    assert!(
        e.db.prepare("SELECT * FROM __rr_retained_loop_rows")
            .is_err()
    );
    for patch in [
        json!([{"name":"edge","delete":[[0,5,6]]}]),
        json!([{"name":"seed","delete":[[1,0]],"insert":[[1,9]]}]),
        json!([{"name":"seed","insert":[[0,6]]}]),
        json!([{"name":"seed","delete":[[0,0]]}]),
        json!([{"name":"edge","insert":[[0,5,6]]}]),
    ] {
        e.apply_changes(patch.to_string(), Some(128)).unwrap();
        check(&mut e);
    }
    let before = rows(&mut e, "SELECT * FROM a ORDER BY c0,c1");
    let patch = json!([{"name":"seed","delete":[[0,6]]}]).to_string();
    assert!(e.apply_changes(patch.clone(), Some(1)).is_err());
    assert_eq!(rows(&mut e, "SELECT * FROM a ORDER BY c0,c1"), before);
    e.apply_changes(patch, Some(128)).unwrap();
    check(&mut e);
}

#[test]
fn cold_loop_wave_failure_rolls_back_private_history_and_can_retry() {
    let _full = full_loop();
    let (_dir, mut e) = executor();
    bind(&mut e, "seed", &["g", "n"], json!([[0, 0]]));
    bind(
        &mut e,
        "edge",
        &["g", "a", "b"],
        json!([[0, 0, 1], [0, 1, 2], [0, 2, 3]]),
    );
    assert!(
        e.run(PROGRAM.into(), "python".into(), Some(1))
            .unwrap_err()
            .reason
            .contains("no fixpoint")
    );
    assert!(e.executions.is_empty());
    assert!(
        e.db.prepare("SELECT * FROM __rr_retained_loop_rows")
            .is_err()
    );
    e.run(PROGRAM.into(), "python".into(), Some(128)).unwrap();
    check(&mut e);
    assert_eq!(rows(&mut e, "SELECT count(*) FROM a"), json!([[4]]));
}

#[test]
fn sparse_iteration_differences_keep_ids_and_skip_unchanged_index_branches() {
    let old = versions::Version::empty(Arc::new(versions::Schema::integers(2, 2)))
        .loaded(
            (0..10000).map(|i| vec![Some(i), Some(i + 1)]).collect(),
            false,
            true,
        )
        .unwrap();
    let new = old
        .inserted_rows(vec![vec![Some(10000), Some(10001)]], false)
        .unwrap();
    let (plus, minus, visited) = old.difference_roots(&new).unwrap();
    assert_eq!(plus.rows(), vec![vec![Some(10000), Some(10001)]]);
    assert_eq!(plus.row_id(0), 10001);
    assert_eq!(minus.len(), 0);
    assert!(visited < 200, "visited {visited} unchanged branches");
    let (same, removed, visited) = new.difference_roots(&new).unwrap();
    assert_eq!(same.len() + removed.len() + visited, 0);
}

#[test]
#[ignore = "debug host/SQL comparison helper, run with SQL profiling enabled"]
fn compare_complete_cold_sessions() {
    struct Host(bool, bool);
    impl Drop for Host {
        fn drop(&mut self) {
            FORCE_HOST.set(self.0);
            FORCE_LOOP.set(self.1);
        }
    }
    let regions = std::env::var("RERAIL_SQL_LOOP_REGIONS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(8);
    let nodes = std::env::var("RERAIL_SQL_LOOP_NODES")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(64);
    let mut results = vec![];
    let order = if std::env::var_os("RERAIL_SQL_LOOP_FIRST").is_some() {
        ["sql", "adaptive", "host"]
    } else {
        ["host", "adaptive", "sql"]
    };
    for strategy in order {
        let host = strategy == "host";
        let (_dir, mut e) = executor();
        bind(
            &mut e,
            "seed",
            &["g", "n"],
            json!((0..regions).map(|g| [g, 0]).collect::<Vec<_>>()),
        );
        bind(
            &mut e,
            "edge",
            &["g", "a", "b"],
            json!(
                (0..regions)
                    .flat_map(|g| (0..nodes).map(move |n| [g, n, (n + 1) % nodes]))
                    .collect::<Vec<_>>()
            ),
        );
        let _host = Host(
            FORCE_HOST.replace(host),
            FORCE_LOOP.replace(strategy == "sql"),
        );
        let before = e.sql_profile_totals();
        let cpu = crate::code_rule_dsl::sql_profile::thread_cpu_ns();
        let wall = std::time::Instant::now();
        e.run(PROGRAM.into(), "python".into(), Some(256)).unwrap();
        let wall_ns = wall.elapsed().as_nanos();
        let cpu_ns = cpu
            .zip(crate::code_rule_dsl::sql_profile::thread_cpu_ns())
            .map(|(start, end)| end - start);
        let totals = before
            .zip(e.sql_profile_totals())
            .map(|(before, after)| std::array::from_fn::<_, 6, _>(|i| after[i] - before[i]));
        let trace = &e.executions[0].temporal[&0];
        let phases = trace.frames.iter().map(|f| f.phase_batches).sum::<usize>();
        if !host {
            if strategy == "sql" {
                assert_eq!(phases, 1);
            } else {
                assert!(phases <= HOST_PHASE_LIMIT + 1);
            }
            if let Some(totals) = totals {
                assert!(
                    totals[0] <= 100,
                    "complete cold session exceeded allowance: {totals:?}"
                );
            }
        }
        let mut sorted = BTreeMap::new();
        for name in ["a", "b", "observed"] {
            sorted.insert(
                name,
                serde_json::from_str::<Value>(
                    &e.query(
                        format!("SELECT * FROM {name} ORDER BY c0,c1"),
                        None,
                        Some(u32::MAX),
                    )
                    .unwrap(),
                )
                .unwrap()["rows"]
                    .clone(),
            );
        }
        assert_eq!(
            sorted["a"].as_array().unwrap().len(),
            (regions * nodes) as usize
        );
        results.push(json!({"host":host,"strategy":strategy,"totals":totals,"cpu_ns":cpu_ns,"wall_ns":wall_ns,"phases":phases,"rows":sorted}));
    }
    for other in results.iter().skip(1) {
        assert_eq!(results[0]["rows"], other["rows"]);
    }
    for result in &mut results {
        result.as_object_mut().unwrap().remove("rows");
    }
    println!(
        "{}",
        json!({"case":"mutual recursion and a downstream frontier consumer","regions":regions,"nodes_per_region":nodes,"results":results})
    );
}

#[test]
fn snapshot_edits_replay_the_sql_trajectory_and_restore_publication_failure() {
    let _full = full_loop();
    let (_dir, mut e) = executor();
    let program = ".decl name(n:node,s:symbol)\n.input name\n.decl a(n:node)\n.decl b(n:node)\na(n) :- name(n,_).\nb(n) :- a(n).\na(n) :- b(n).\n.output a,b\n";
    let file = |source: &str| json!({"path":"a.py","language":"python","source":source});
    let source = "first = 1\nsecond = 2\n";
    e.bind_snapshot(json!({"files":[file(source)]}).to_string())
        .unwrap();
    e.run(program.into(), "python".into(), Some(64)).unwrap();
    assert_eq!(
        e.executions[0].temporal[&0].frames[0].observation["sql_version_loop"]["planned_submissions"],
        1
    );
    for source in ["third = 3\n", "", source] {
        e.apply_file_changes(json!({"upsert":[file(source)]}).to_string(), Some(64))
            .unwrap();
        let (_dir, mut oracle) = executor();
        oracle.cold_oracle = true;
        oracle
            .bind_snapshot(json!({"files":[file(source)]}).to_string())
            .unwrap();
        oracle
            .run(program.into(), "python".into(), Some(64))
            .unwrap();
        for name in ["a", "b"] {
            let query = format!("SELECT * FROM {name} ORDER BY c0");
            assert_eq!(rows(&mut e, &query), rows(&mut oracle, &query));
        }
    }
    let before = rows(&mut e, "SELECT * FROM a ORDER BY c0");
    e.connection().execute_batch("CREATE TEMP TRIGGER fail_loop_publication BEFORE DELETE ON a BEGIN SELECT RAISE(ABORT,'stop loop publication'); END").unwrap();
    let patch = json!({"upsert":[file("")]}).to_string();
    assert!(
        e.apply_file_changes(patch.clone(), Some(64))
            .unwrap_err()
            .reason
            .contains("stop loop publication")
    );
    assert_eq!(rows(&mut e, "SELECT * FROM a ORDER BY c0"), before);
    e.connection()
        .execute_batch("DROP TRIGGER fail_loop_publication")
        .unwrap();
    e.apply_file_changes(patch, Some(64)).unwrap();
    assert_eq!(rows(&mut e, "SELECT count(*) FROM a"), json!([[0]]));
}

#[test]
fn adaptive_handoff_reuses_prefix_contexts_and_keeps_warm_edits_correct() {
    let (_dir, mut e) = executor();
    bind(&mut e, "seed", &["g", "n"], json!([[0, 0]]));
    bind(
        &mut e,
        "edge",
        &["g", "a", "b"],
        json!((0..64).map(|n| [0, n, (n + 1) % 64]).collect::<Vec<_>>()),
    );
    e.run(PROGRAM.into(), "python".into(), Some(256)).unwrap();
    let trace = &e.executions[0].temporal[&0];
    assert_eq!(
        trace.frames.iter().map(|f| f.phase_batches).sum::<usize>(),
        HOST_PHASE_LIMIT + 1
    );
    assert!(
        trace
            .frames
            .iter()
            .any(|f| f.observation["sql_version_loop"]["prefix_sql_phases"] == HOST_PHASE_LIMIT)
    );
    check(&mut e);
    for patch in [
        json!([{"name":"edge","delete":[[0,5,6]]}]),
        json!([{"name":"seed","insert":[[0,6]]}]),
        json!([{"name":"edge","insert":[[0,5,6]]}]),
    ] {
        e.apply_changes(patch.to_string(), Some(256)).unwrap();
        check(&mut e);
    }
}

#[test]
fn short_recursions_keep_the_host_sql_schedule() {
    let (_dir, mut e) = executor();
    bind(&mut e, "seed", &["g", "n"], json!([[0, 0]]));
    bind(
        &mut e,
        "edge",
        &["g", "a", "b"],
        json!((0..8).map(|n| [0, n, (n + 1) % 8]).collect::<Vec<_>>()),
    );
    e.run(PROGRAM.into(), "python".into(), Some(256)).unwrap();
    let trace = &e.executions[0].temporal[&0];
    assert!(
        trace
            .frames
            .iter()
            .all(|f| f.observation["sql_version_loop"].is_null())
    );
    assert!(trace.frames.iter().map(|f| f.phase_batches).sum::<usize>() < HOST_PHASE_LIMIT);
    check(&mut e);
}

fn loop_reports(e: &CodeRuleSqlExecutor) -> Vec<Value> {
    e.executions[0].temporal[&0]
        .frames
        .iter()
        .map(|f| f.observation["sql_version_loop"].clone())
        .filter(|v| !v.is_null())
        .collect()
}

#[test]
fn mixed_feedback_keeps_frozen_scalars_and_opaque_consumers_at_their_gates() {
    let (_dir, mut e) = executor();
    let program = PROGRAM
        .replace(
            ".input seed,edge",
            ".decl quota(n:number)\n.input seed,edge,quota",
        )
        .replace(
            "b(g,y) :- a(g,x),edge(g,x,y).",
            "b(g,y) :- a(g,x),edge(g,x,y),n=count : {quota(_)},n>0.",
        )
        + ".decl summary(g:number,n:number)\nsummary(g,n) :- seed(g,_), n=count : {a(g,_)}.\n.output summary\n"
        + ".decl observed_sql(n:number)\n.input observed_sql(IO=sql,query=\"SELECT count(*) FROM temp.a\")\n.output observed_sql\n";
    bind(&mut e, "seed", &["g", "n"], json!([[0, 0]]));
    bind(&mut e, "quota", &["n"], json!([[1]]));
    bind(
        &mut e,
        "edge",
        &["g", "a", "b"],
        json!((0..80).map(|n| [0, n, (n + 1) % 80]).collect::<Vec<_>>()),
    );
    e.run(program.clone(), "python".into(), Some(256)).unwrap();
    let reports = loop_reports(&e);
    assert!(
        reports
            .iter()
            .any(|r| {
                r["owner_gates"] == json!(["observed_sql", "summary"])
                    && r["rule_stages"].as_array().unwrap().iter().any(|s| {
                        s["relation"] == "b" && s["frozen_scalar_inputs"] == json!(["quota"])
                    })
            }),
        "{reports:?}"
    );
    assert_eq!(rows(&mut e, "SELECT * FROM observed_sql"), json!([[80]]));
    let cut = json!([{"name":"edge","delete":[[0,63,64]]}]).to_string();
    let err = e.apply_changes(cut.clone(), Some(50)).unwrap_err();
    assert!(err.reason.contains("no fixpoint"), "{err}");
    assert_eq!(rows(&mut e, "SELECT * FROM observed_sql"), json!([[80]]));
    e.apply_changes(cut, Some(256)).unwrap();
    assert!(loop_reports(&e).iter().any(|r| r["scope"] == "warm"));
    assert_eq!(rows(&mut e, "SELECT * FROM observed_sql"), json!([[64]]));
    for patch in [
        json!([{"name":"quota","delete":[[1]]}]),
        json!([{"name":"quota","insert":[[2]]}]),
        json!([{"name":"edge","insert":[[0,63,64]]}]),
    ] {
        let (_dir, mut oracle) = executor();
        oracle.cold_oracle = true;
        e.apply_changes(patch.to_string(), Some(256)).unwrap();
        for (r, c) in [
            ("seed", &["g", "n"][..]),
            ("edge", &["g", "a", "b"][..]),
            ("quota", &["n"][..]),
        ] {
            bind(
                &mut oracle,
                r,
                c,
                rows(&mut e, &format!("SELECT * FROM {r}")),
            );
        }
        oracle
            .run(program.clone(), "python".into(), Some(256))
            .unwrap();
        for r in ["a", "b", "observed", "summary", "observed_sql"] {
            let q = format!("SELECT * FROM {r} ORDER BY 1");
            let mut got = rows(&mut e, &q).as_array().unwrap().clone();
            let mut want = rows(&mut oracle, &q).as_array().unwrap().clone();
            got.sort_by_key(|r| r.to_string());
            want.sort_by_key(|r| r.to_string());
            assert_eq!(got, want, "{r}");
        }
    }
}

#[test]
fn long_cold_and_warm_replays_fit_one_session_allowance() {
    let (_dir, mut e) = executor();
    e.set_limits(json!({"max_submissions":100}).to_string())
        .unwrap();
    bind(&mut e, "seed", &["g", "n"], json!([[0, 0]]));
    bind(
        &mut e,
        "edge",
        &["g", "a", "b"],
        json!((0..80).map(|n| [0, n, (n + 1) % 80]).collect::<Vec<_>>()),
    );
    e.run(PROGRAM.into(), "python".into(), Some(256)).unwrap();
    let cold = e.db.admission().snapshot();
    assert_eq!(rows(&mut e, "SELECT count(*) FROM observed"), json!([[80]]));
    e.apply_changes(
        json!([{"name":"edge","delete":[[0,63,64]]}]).to_string(),
        Some(256),
    )
    .unwrap();
    let warm = e.db.admission().snapshot();
    assert!(
        loop_reports(&e).iter().any(|r| r["scope"] == "warm"),
        "{warm}"
    );
    assert_eq!(rows(&mut e, "SELECT count(*) FROM observed"), json!([[63]]));
    let checked = e.db.admission().snapshot();
    assert!(checked["executed"].as_u64().unwrap() <= 100, "{checked}");
    println!("complete session: cold={cold} warm={warm} verified={checked}");
}
