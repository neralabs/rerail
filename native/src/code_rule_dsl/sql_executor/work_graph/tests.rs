use super::*;

fn node(name: &str, sql: &str, width: usize, reads: &[&str]) -> Node {
    Node {
        name: name.into(),
        sql: sql.into(),
        width,
        reads: reads.iter().map(|s| (*s).into()).collect(),
    }
}
fn request(nodes: Vec<Node>, outputs: &[&str], max_submissions: usize) -> Request {
    Request {
        nodes,
        outputs: outputs.iter().map(|n| (*n).into()).collect(),
        max_submissions,
    }
}
fn sources() -> BTreeMap<String, Arc<dyn Source>> {
    let mut edges = Relation::new(&["src", "dst"]);
    for edge in [[1, 2], [2, 3], [3, 4], [4, 2], [2, 5], [5, 6]] {
        edges.push(&edge);
    }
    BTreeMap::from([("edges".into(), Arc::new(edges) as Arc<dyn Source>)])
}

#[test]
fn recursive_join_diamond_fuses_once_and_independent_groups_use_sql() {
    let graph = request(
        vec![
            node(
                "closure",
                "WITH RECURSIVE walk(a,b) AS (SELECT src,dst FROM edges UNION SELECT w.a,e.dst FROM walk w JOIN edges e ON e.src=w.b) SELECT a,b FROM walk",
                2,
                &["edges"],
            ),
            node(
                "left_path",
                "SELECT c0,count(*) FROM closure GROUP BY c0",
                2,
                &["closure"],
            ),
            node(
                "right_path",
                "SELECT c0,max(c1) FROM closure GROUP BY c0",
                2,
                &["closure"],
            ),
            node(
                "joined",
                "SELECT l.c0,l.c1+r.c1 FROM left_path l JOIN right_path r ON r.c0=l.c0",
                2,
                &["left_path", "right_path"],
            ),
            node(
                "independent",
                "SELECT src,sum(dst) FROM edges GROUP BY src",
                2,
                &["edges"],
            ),
            node("unobserved", "SELECT abs(-9223372036854775808)", 1, &[]),
        ],
        &["joined", "independent"],
        2,
    );
    let plan = planner::Plan::new(&graph, &sources(), 2).unwrap();
    assert_eq!(plan.groups.len(), 2);
    assert!(plan.groups.iter().all(|g| g.dependencies.is_empty()));
    assert_eq!(
        plan.groups.iter().map(|g| g.outputs.len()).sum::<usize>(),
        2
    );
    let result = execute(graph, sources(), &runtime::Limits::default(), None).unwrap();
    assert_eq!(
        result.submissions,
        if rerail_accel::exec::threads() > 1 {
            2
        } else {
            1
        }
    );
    let mut joined = result.rows["joined"].rows.clone();
    joined.sort();
    assert_eq!(
        joined,
        vec![
            vec![Some(1), Some(11)],
            vec![Some(2), Some(11)],
            vec![Some(3), Some(11)],
            vec![Some(4), Some(11)],
            vec![Some(5), Some(7)]
        ]
    );
    assert!(result.vm_steps > 0);
    assert_eq!(result.explain["executor"], "sql");
}

#[test]
fn large_dependent_graph_exports_cut_edges_without_recomputation() {
    let mut nodes = vec![node(
        "stage_0",
        "SELECT 1,NULL UNION ALL SELECT 1,NULL",
        2,
        &[],
    )];
    for n in 1..240 {
        nodes.push(node(
            &format!("stage_{n}"),
            &format!("SELECT c0+1,c1 FROM stage_{}", n - 1),
            2,
            &[&format!("stage_{}", n - 1)],
        ));
    }
    let graph = request(nodes, &["stage_239"], 100);
    let plan = planner::Plan::new(&graph, &BTreeMap::new(), 4).unwrap();
    assert!(plan.groups.len() > 1 && plan.groups.len() < 100);
    assert_eq!(
        plan.groups.iter().map(|g| g.nodes.len()).sum::<usize>(),
        240
    );
    let result = execute(graph, BTreeMap::new(), &runtime::Limits::default(), None).unwrap();
    assert_eq!(result.submissions, plan.groups.len());
    assert_eq!(
        result.rows["stage_239"].rows,
        vec![vec![Some(240), None]; 2]
    );
}

#[test]
fn plan_admission_covers_all_groups_and_packs_independent_work_to_fit() {
    let nodes = (0..64)
        .map(|n| node(&format!("work_{n}"), &format!("SELECT {n}"), 1, &[]))
        .collect::<Vec<_>>();
    let names = (0..64).map(|n| format!("work_{n}")).collect::<Vec<_>>();
    let outputs = names.iter().map(String::as_str).collect::<Vec<_>>();
    let graph = request(nodes, &outputs, 1);
    let result = execute(graph, BTreeMap::new(), &runtime::Limits::default(), None).unwrap();
    assert_eq!(result.submissions, 1);
    assert_eq!(result.rows.len(), 64);
    assert!(
        execute(
            request(vec![node("one", "SELECT 1", 1, &[])], &["one"], 0),
            BTreeMap::new(),
            &runtime::Limits::default(),
            None
        )
        .unwrap_err()
        .to_string()
        .contains("requires 1 submissions")
    );
}

#[test]
fn invalid_dependencies_opaque_functions_and_writes_are_rejected() {
    for graph in [
        request(
            vec![
                node("a", "SELECT c0 FROM b", 1, &["b"]),
                node("b", "SELECT c0 FROM a", 1, &["a"]),
            ],
            &["a"],
            100,
        ),
        request(
            vec![
                node("a", "SELECT 1", 1, &[]),
                node("b", "SELECT c0 FROM \"a\"", 1, &[]),
            ],
            &["a", "b"],
            100,
        ),
        request(vec![node("a", "SELECT random()", 1, &[])], &["a"], 100),
        request(
            vec![node("a", "DELETE FROM edges RETURNING src", 1, &["edges"])],
            &["a"],
            100,
        ),
        request(vec![node("a", "SELECT ?1", 1, &[])], &["a"], 100),
    ] {
        assert!(execute(graph, sources(), &runtime::Limits::default(), None).is_err());
    }
    // Quoted strings and comments containing graph names do not create phantom reads.
    let graph = request(
        vec![node("a", "SELECT length('a') /* a */ -- a\n", 1, &[])],
        &["a"],
        1,
    );
    assert_eq!(
        execute(graph, BTreeMap::new(), &runtime::Limits::default(), None)
            .unwrap()
            .rows["a"]
            .rows,
        vec![vec![Some(1)]]
    );
}

#[test]
fn workers_share_vm_limits_and_failed_queries_do_not_publish() {
    let expensive = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<50) SELECT sum(x) FROM n";
    let one = execute(
        request(vec![node("a", expensive, 1, &[])], &["a"], 1),
        BTreeMap::new(),
        &runtime::Limits::default(),
        None,
    )
    .unwrap();
    let graph = request(
        vec![node("a", expensive, 1, &[]), node("b", expensive, 1, &[])],
        &["a", "b"],
        2,
    );
    // Each query fits, but their total exceeds this operation's shared allowance.
    let limits = runtime::Limits {
        max_steps: Some(one.vm_steps + one.vm_steps / 2),
        timeout_ms: Some(5000),
    };
    let started = Instant::now();
    assert!(execute(graph, BTreeMap::new(), &limits, None).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    // Failure never poisons later jobs or grants any worker its own fresh allowance.
    let ok = request(vec![node("a", "SELECT 9", 1, &[])], &["a"], 1);
    assert_eq!(
        execute(ok, BTreeMap::new(), &runtime::Limits::default(), None)
            .unwrap()
            .submissions,
        1
    );
}

#[test]
fn worker_profiles_cover_every_statement_and_session_totals() {
    for threads in [1_u64, 2] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("nice")
            .args(["-n", "19"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "code_rule_dsl::sql_executor::work_graph::tests::profiled_graph_child",
                "--ignored",
            ])
            .env("RERAIL_SQL_PROFILE_DIR", directory.path())
            .env("RERAIL_THREADS", threads.to_string())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(15) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("SQL graph stalled with {threads} worker(s)");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success());
        let mut statements = 0;
        let mut connections = 0;
        for entry in std::fs::read_dir(directory.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let profile: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert!(profile["error"].is_null(), "{profile}");
            assert_eq!(profile["executionCoverage"]["running"], 0);
            let mut worker = false;
            // Events retain the worker label; every connection has an explained SQL catalogue.
            for query in profile["queries"].as_array().unwrap() {
                if query["sql"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("WITH RECURSIVE \""))
                {
                    worker = true;
                    assert!(query.get("plan").is_some(), "{query}");
                }
            }
            if worker {
                connections += 1;
                statements += profile["executionCoverage"]["started"].as_u64().unwrap();
            }
        }
        assert_eq!(
            (connections, statements),
            (threads as usize, threads),
            "worker binding/setup must submit no helper SQL"
        );
    }
}

#[test]
#[ignore = "child process of SQL graph profiling test"]
fn profiled_graph_child() {
    let (_dir, mut executor) = super::super::tests::executor();
    executor
        .bind_relation(
            json!({"name":"input_rows","columns":["k"],"rows":[[1],[2],[3]]}).to_string(),
        )
        .unwrap();
    let expected = rerail_accel::exec::threads().min(2) as u64;
    let before = executor.sql_profile_totals().unwrap();
    let result: Value = serde_json::from_str(
        &executor
            .query_plan(
                json!({"nodes":[
        {"name":"a","sql":"SELECT sum(k) FROM input_rows","width":1,"reads":["input_rows"]},
        {"name":"b","sql":"SELECT max(k) FROM input_rows","width":1,"reads":["input_rows"]}
    ],"outputs":["a","b"],"max_submissions":2})
                .to_string(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["submissions"], expected);
    let after = executor.sql_profile_totals().unwrap();
    assert_eq!(
        after[0] - before[0],
        expected,
        "owner totals include every worker statement"
    );
    let score = super::super::super::logic::lanes::sql_executor().score();
    let sql = &score.units[Lane::Sql.index()];
    assert_eq!(
        (sql.jobs, sql.failed, sql.running, sql.queued),
        (expected, 0, 0, 0)
    );
    assert_eq!(score.units[Lane::Cpu.index()].jobs, 0);
}

#[test]
fn public_query_plan_reads_pinned_facts_and_explains_its_groups() {
    let (_dir, mut executor) = super::super::tests::executor();
    executor
        .bind_relation(
            json!({"name":"input_rows","columns":["k","v"],"rows":[[1,2],[1,4],[2,7]]}).to_string(),
        )
        .unwrap();
    let result:Value=serde_json::from_str(&executor.query_plan(json!({"nodes":[
        {"name":"totals","sql":"SELECT k,sum(v) FROM input_rows GROUP BY k","width":2,"reads":["input_rows"]},
        {"name":"answer","sql":"SELECT c0,c1*2 FROM totals","width":2,"reads":["totals"]}
    ],"outputs":["answer"],"max_submissions":1}).to_string()).unwrap()).unwrap();
    assert_eq!(result["rows"]["answer"], json!([[1, 12], [2, 14]]));
    assert_eq!(result["submissions"], 1);
    assert_eq!(
        result["plan"]["groups"][0]["nodes"],
        json!(["totals", "answer"])
    );
    let before = executor.sql_profile_totals();
    let explanation: Value = serde_json::from_str(&executor.explain().unwrap()).unwrap();
    assert_eq!(explanation["sql_work_plan"], result["plan"]);
    assert_eq!(executor.sql_profile_totals(), before);
}

#[test]
fn actual_sql_readers_overlap_and_a_shared_intermediate_reads_storage_once() {
    struct Observed {
        entered: AtomicUsize,
        active: AtomicUsize,
        peak: AtomicUsize,
        cells: AtomicUsize,
        overlap: bool,
    }
    impl Source for Observed {
        fn columns(&self) -> Vec<String> {
            vec!["v".into()]
        }
        fn len(&self) -> usize {
            4
        }
        fn indexed(&self, _: usize) -> bool {
            false
        }
        fn estimated_matches(&self, _: usize, _: bool) -> usize {
            4
        }
        fn lookup(&self, _: usize, _: &[i64]) -> Option<Vec<usize>> {
            None
        }
        fn value(&self, r: usize, _: usize) -> i64 {
            self.cells.fetch_add(1, Ordering::Relaxed);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let entry = self.entered.fetch_add(1, Ordering::SeqCst);
            if self.overlap && entry < 2 && rerail_accel::exec::threads() > 1 {
                let start = Instant::now();
                while self.peak.load(Ordering::SeqCst) < 2
                    && start.elapsed() < Duration::from_secs(2)
                {
                    std::thread::yield_now();
                }
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            r as i64 + 1
        }
    }
    let storage = |overlap| {
        Arc::new(Observed {
            entered: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            cells: AtomicUsize::new(0),
            overlap,
        })
    };
    let input = storage(true);
    let inputs = BTreeMap::from([("values_in".into(), input.clone() as Arc<dyn Source>)]);
    let graph = request(
        vec![
            node("a", "SELECT sum(v) FROM values_in", 1, &["values_in"]),
            node("b", "SELECT max(v) FROM values_in", 1, &["values_in"]),
        ],
        &["a", "b"],
        2,
    );
    let result = execute(graph, inputs, &runtime::Limits::default(), None).unwrap();
    assert_eq!(result.rows["a"].rows, vec![vec![Some(10)]]);
    assert_eq!(result.rows["b"].rows, vec![vec![Some(4)]]);
    if rerail_accel::exec::threads() > 1 {
        assert!(input.peak.load(Ordering::SeqCst) >= 2);
    }
    let input = storage(false);
    let inputs = BTreeMap::from([("values_in".into(), input.clone() as Arc<dyn Source>)]);
    let graph = request(
        vec![
            node("common", "SELECT sum(v) FROM values_in", 1, &["values_in"]),
            node("a", "SELECT c0+1 FROM common", 1, &["common"]),
            node("b", "SELECT c0+2 FROM common", 1, &["common"]),
        ],
        &["a", "b"],
        2,
    );
    assert_eq!(
        execute(graph, inputs, &runtime::Limits::default(), None)
            .unwrap()
            .submissions,
        1
    );
    assert_eq!(
        input.cells.load(Ordering::Relaxed),
        4,
        "shared SQL storage scan ran once"
    );
}

#[test]
fn ten_thousand_regions_and_512_operators_fit_four_submissions() {
    let mut regions = Relation::new(&["k", "v"]);
    for k in 0..10_000 {
        regions.push(&[k, k]);
    }
    let inputs = BTreeMap::from([("regions".into(), Arc::new(regions) as Arc<dyn Source>)]);
    let mut nodes = vec![];
    let mut outputs = vec![];
    for chain in 0..4 {
        for stage in 0..128 {
            let name = format!("chain_{chain}_{stage}");
            if stage == 0 {
                nodes.push(node(&name, "SELECT k,v+1 FROM regions", 2, &["regions"]));
            } else {
                let prior = format!("chain_{chain}_{}", stage - 1);
                nodes.push(node(
                    &name,
                    &format!("SELECT c0,c1+1 FROM {prior}"),
                    2,
                    &[&prior],
                ));
            }
        }
        outputs.push(format!("chain_{chain}_127"));
    }
    let names = outputs.iter().map(String::as_str).collect::<Vec<_>>();
    let start = Instant::now();
    let result = execute(
        request(nodes, &names, 4),
        inputs,
        &runtime::Limits::default(),
        None,
    )
    .unwrap();
    assert_eq!(result.submissions, 4);
    for output in result.rows.values() {
        assert_eq!(output.rows.len(), 10_000);
        assert!(output.rows.iter().all(|r| r[1] == r[0].map(|k| k + 128)));
    }
    eprintln!(
        "SQL work graph: 10,000 regions, 512 operators, {} submissions, {} VM steps, {:.3}s wall",
        result.submissions,
        result.vm_steps,
        start.elapsed().as_secs_f64()
    );
}
