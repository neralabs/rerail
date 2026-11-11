use super::super::super::tests::{executor, rows};
use super::*;

#[test]
fn wide_budgeted_lattice_and_scalar_feedback_replays_atomically() {
    let (_dir, mut e) = executor();
    let mut program = String::from(
        ".decl seed(g:number,v:number)\n.decl owner(g:number)\n.input seed,owner\n\
         .decl item(g:number,v:number)\n.decl amount(g:number,v:max<>) widen 20\n",
    );
    for i in 0..36 {
        program.push_str(&format!("item(g,v) :- seed(g,v), v >= {i}.\n"));
    }
    program.push_str(
        "item(g,v) :- amount(g,v), v < 4.\n\
         amount(g,n) :- owner(g), n=count : {item(g,_)}.\n\
         .output item,amount\n",
    );
    e.set_limits(json!({"max_submissions":100}).to_string())
        .unwrap();
    let bind = |e: &mut CodeRuleSqlExecutor, seeds: Value| {
        e.bind_relation(json!({"name":"seed","columns":["g","v"],"rows":seeds}).to_string())
            .unwrap();
        e.bind_relation(json!({"name":"owner","columns":["g"],"rows":[[0],[1]]}).to_string())
            .unwrap();
    };
    bind(&mut e, json!([[0, 0], [1, 3]]));
    e.run(program.clone(), "python".into(), Some(32)).unwrap();
    let before = rows(&mut e, "SELECT * FROM amount ORDER BY c0");
    assert!(e.executions[0].temporal[&0].batch.is_some());
    let edit = json!([{"name":"seed","delete":[[1,3]],"insert":[[0,9]]}]).to_string();
    e.set_limits(json!({"max_submissions":100,"max_steps":1}).to_string())
        .unwrap();
    assert!(e.apply_changes(edit.clone(), Some(32)).is_err());
    e.set_limits(json!({"max_submissions":100}).to_string())
        .unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM amount ORDER BY c0"), before);
    e.apply_changes(edit, Some(32)).unwrap();
    let (_dir, mut oracle) = executor();
    oracle.cold_oracle = true;
    bind(&mut oracle, json!([[0, 0], [0, 9]]));
    oracle.run(program, "python".into(), Some(32)).unwrap();
    for relation in ["item", "amount", "__sql_changes_amount"] {
        let sql = format!("SELECT * FROM {relation} ORDER BY 1,2");
        assert_eq!(rows(&mut e, &sql), rows(&mut oracle, &sql), "{relation}");
    }
    let history = e.executions[0].temporal[&0].batch.as_ref().unwrap();
    assert_eq!(history.explain()["scope"], "warm");
    assert_eq!(history.explain()["planned_submissions"], 1);
    assert!(e.db.admission().snapshot()["executed"].as_u64().unwrap() <= 100);
}

fn projection_program() -> String {
    let mut program = String::from(
        ".decl seed(g:number,id:number,v:number)\n\
         .decl enabled(g:number)\n.input seed,enabled\n\
         .decl item(g:number,v:number)\n.decl total(n:number)\n",
    );
    for i in 0..36 {
        program.push_str(&format!(
            "item(g,v) :- seed(g,_,v), enabled(g), v >= {i}.\n"
        ));
    }
    program.push_str(
        "item(g,v) :- seed(g,_,v).\n\
         total(n) :- n=count : {item(_,_)}.\n\
         total(n) :- total(n), n=count : {item(_,_)}.\n\
         .decl paired(g:number)\npaired(g) :- item(g,_),item(g,_).\n\
         .output item,total,paired\n",
    );
    program
}

fn projected_fixture(groups: i64) -> (tempfile::TempDir, CodeRuleSqlExecutor) {
    let (dir, mut e) = executor();
    e.set_limits(json!({"max_submissions":100}).to_string())
        .unwrap();
    e.bind_relation(
        json!({"name":"seed","columns":["g","id","v"],
        "rows":(0..groups).flat_map(|g|[vec![g,0,40],vec![g,1,40]]).collect::<Vec<_>>()})
        .to_string(),
    )
    .unwrap();
    e.bind_relation(
        json!({"name":"enabled","columns":["g"],
        "rows":(0..groups).map(|g|vec![g]).collect::<Vec<_>>()})
        .to_string(),
    )
    .unwrap();
    e.run(projection_program(), "python".into(), Some(32))
        .unwrap();
    assert!(e.executions[0].temporal[&0].batch.is_some());
    (dir, e)
}

#[test]
fn signed_batches_preserve_duplicate_supports_simultaneous_edits_and_rollback() {
    let (_dir, mut e) = projected_fixture(256);
    let (_dir, mut full) = projected_fixture(256);
    let edits = [
        json!([{"name":"seed","delete":[[3,0,40]]}]),
        json!([{"name":"seed","delete":[[3,1,40]]}]),
        json!([{"name":"seed","insert":[[3,0,41],[3,1,41]],"delete":[[4,0,40],[4,1,40]]},
            {"name":"enabled","insert":[[3]],"delete":[[4]]}]),
        json!([{"name":"seed","insert":[[4,2,42]]},{"name":"enabled","insert":[[4]]}]),
    ];
    for edit in edits {
        e.set_limits("{}".into()).unwrap();
        full.set_limits("{}".into()).unwrap();
        incremental::FORCE_FULL.set(true);
        let expected = full.apply_changes(edit.to_string(), Some(32));
        incremental::FORCE_FULL.set(false);
        expected.unwrap();
        e.apply_changes(edit.to_string(), Some(32)).unwrap();
        for relation in ["item", "total", "paired"] {
            let sql = format!("SELECT * FROM {relation} ORDER BY 1,2");
            // The scalar relation has one column.
            let sql = if relation != "item" {
                format!("SELECT * FROM {relation} ORDER BY 1")
            } else {
                sql
            };
            assert_eq!(rows(&mut e, &sql), rows(&mut full, &sql), "{edit}");
        }
        let history = e.executions[0].temporal[&0].batch.as_ref().unwrap();
        assert!(
            history.explain()["signed_rule_results"].as_u64().unwrap() > 0,
            "{}",
            history.explain()
        );
        assert_eq!(history.explain()["planned_submissions"], 1);
    }
    let before = rows(&mut e, "SELECT * FROM item ORDER BY 1,2");
    e.set_limits(json!({"max_steps":200}).to_string()).unwrap();
    let edit =
        json!([{"name":"seed","insert":[[5,2,43]],"delete":[[5,0,40],[5,1,40]]}]).to_string();
    assert!(e.apply_changes(edit.clone(), Some(32)).is_err());
    e.set_limits("{}".into()).unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM item ORDER BY 1,2"), before);
    e.apply_changes(edit, Some(32)).unwrap();
    assert_eq!(
        rows(&mut e, "SELECT * FROM item WHERE c0=5 ORDER BY c1"),
        json!([[5, 43]])
    );
}

#[test]
fn signed_batches_reduce_small_edit_vm_work_under_the_same_session_cap() {
    let (_dir, mut sparse) = projected_fixture(1024);
    let (_dir, mut full) = projected_fixture(1024);
    let edit = json!([{"name":"seed","delete":[[7,0,40]],"insert":[[7,2,41]]}]).to_string();
    let sparse_ledger = sparse.db.admission();
    let full_ledger = full.db.admission();
    let _sparse_measurement = sparse_ledger.measure();
    let _full_measurement = full_ledger.measure();
    let sparse_before = sparse_ledger.counters();
    sparse.apply_changes(edit.clone(), Some(32)).unwrap();
    let sparse_after = sparse_ledger.counters();
    let full_before = full_ledger.counters();
    incremental::FORCE_FULL.set(true);
    let result = full.apply_changes(edit, Some(32));
    incremental::FORCE_FULL.set(false);
    result.unwrap();
    let full_after = full_ledger.counters();
    let sparse_vm = sparse_after.1 - sparse_before.1;
    let full_vm = full_after.1 - full_before.1;
    println!(
        "signed replay {sparse_vm} VM steps, full replay {full_vm} VM steps; submissions {} and {}",
        sparse_after.0 - sparse_before.0,
        full_after.0 - full_before.0
    );
    assert!(sparse_vm < full_vm, "signed={sparse_vm}, full={full_vm}");
    assert_eq!(
        sparse_after.0 - sparse_before.0,
        full_after.0 - full_before.0
    );
    assert!(sparse_after.0 <= 100 && full_after.0 <= 100);
    assert_eq!(
        rows(&mut sparse, "SELECT * FROM item ORDER BY 1,2"),
        rows(&mut full, "SELECT * FROM item ORDER BY 1,2")
    );
}

fn normalized_state(e: &CodeRuleSqlExecutor) -> BTreeMap<String, Vec<String>> {
    let mut relations = e
        .verification_relations()
        .unwrap()
        .into_iter()
        .map(|(name, width)| {
            (
                name,
                (0..width).map(|c| format!("c{c}")).collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    relations.extend(e.verification_inputs());
    let types = e.verification_types();
    let width = relations.values().map(Vec::len).max().unwrap();
    let mut state = relations
        .keys()
        .map(|n| (n.clone(), Vec::new()))
        .collect::<BTreeMap<String, Vec<String>>>();
    let arms = relations
        .iter()
        .enumerate()
        .map(|(tag, (name, columns))| {
            let fields = (0..width)
                .map(|c| columns.get(c).map_or_else(|| "NULL".into(), |c| quote(c)))
                .collect::<Vec<_>>()
                .join(",");
            format!("SELECT {tag},{fields} FROM {}", quote(name))
        })
        .collect::<Vec<_>>();
    let names = relations.keys().collect::<Vec<_>>();
    for row in e.verification_rows(&arms.join(" UNION ALL ")).unwrap() {
        let name = names[row[0].unwrap() as usize];
        let values = row[1..=relations[name].len()]
            .iter()
            .enumerate()
            .map(|(at, &v)| {
                let ty = types
                    .get(name)
                    .and_then(|cols| cols.get(at))
                    .map_or("int", String::as_str);
                e.verification_cell(ty, v).unwrap()
            })
            .collect::<Vec<_>>();
        state
            .get_mut(name)
            .unwrap()
            .push(serde_json::to_string(&values).unwrap());
    }
    for rows in state.values_mut() {
        rows.sort();
    }
    state
}

#[test]
#[ignore = "complete CPython replay comparison with the full batch kernel"]
fn signed_cpython_replays_match_full_batches_within_the_session_cap() {
    let raw = zstd::decode_all(&include_bytes!("../../../facts/vtab/golden/corpus.json.zst")[..])
        .unwrap();
    let corpus: Vec<(String, String)> = serde_json::from_slice(&raw).unwrap();
    let snapshot=json!({"files":corpus.iter().map(|(p,s)|json!({"path":p,"source":s,"language":"python"})).collect::<Vec<_>>()}).to_string();
    let (path, source) = corpus.iter().find(|(p, _)| p == "json/decoder.py").unwrap();
    let edited=json!({"upsert":[{"path":path,"source":format!("{source}\n\ndef debug_extra(text):\n    return eval(text)\n"),"language":"python"}]}).to_string();
    let original =
        json!({"upsert":[{"path":path,"source":source,"language":"python"}]}).to_string();
    for program in [
        include_str!("../../../logic/plan/golden/programs/workload.rr"),
        include_str!("../../../logic/plan/golden/programs/calls.rr"),
    ] {
        let (_dir, mut sparse) = executor();
        let (_dir, mut full) = executor();
        for e in [&mut sparse, &mut full] {
            e.set_limits(json!({"max_submissions":100}).to_string())
                .unwrap();
            e.bind_snapshot(snapshot.clone()).unwrap();
            e.run(program.into(), "python".into(), None).unwrap();
        }
        assert_eq!(normalized_state(&sparse), normalized_state(&full));
        for change in [edited.clone(), edited.clone(), original.clone()] {
            let measure = |e: &mut CodeRuleSqlExecutor, force: bool| {
                let ledger = e.db.admission();
                let _measurement = ledger.measure();
                let before = ledger.counters();
                let cpu = crate::code_rule_dsl::sql_profile::thread_cpu_ns();
                incremental::FORCE_FULL.set(force);
                let result = e.apply_file_changes(change.clone(), None);
                incremental::FORCE_FULL.set(false);
                result.unwrap();
                let after = ledger.counters();
                let cpu = cpu
                    .zip(crate::code_rule_dsl::sql_profile::thread_cpu_ns())
                    .map(|(start, end)| end.saturating_sub(start) as f64 / 1e9);
                (after.0 - before.0, after.1 - before.1, cpu)
            };
            let signed = measure(&mut sparse, false);
            let baseline = measure(&mut full, true);
            println!("CPython replay signed={signed:?}, full={baseline:?}");
            assert!(
                signed.1 <= baseline.1,
                "signed batch must preserve indexed small-edit work: signed={signed:?}, full={baseline:?}"
            );
            assert_eq!(normalized_state(&sparse), normalized_state(&full));
            assert!(sparse.db.admission().counters().0 <= 100);
            assert!(full.db.admission().counters().0 <= 100);
            let batch = sparse.executions[0].temporal[&0]
                .batch
                .as_ref()
                .unwrap()
                .explain();
            println!(
                "signed={} full={} reused={} affected={}",
                batch["signed_rule_results"],
                batch["evaluated_rule_results"],
                batch["reused_rule_results"],
                batch["affected_heads"]
            );
        }
    }
}

#[test]
fn signed_gate_statistics_reach_the_background_cost_store() {
    let dir = tempfile::tempdir().unwrap();
    let result = std::process::Command::new("nice")
        .args(["-n", "19"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "code_rule_dsl::sql_executor::temporal::batch::tests::signed_feedback_child",
            "--ignored",
        ])
        .env("RERAIL_COSTS", dir.path().join("costs.sqlite"))
        .env("RERAIL_THREADS", "2")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
#[ignore = "child process of signed SQL feedback test"]
fn signed_feedback_child() {
    let (_dir, mut e) = projected_fixture(128);
    e.apply_changes(
        json!([{"name":"seed","delete":[[7,0,40]]}]).to_string(),
        Some(32),
    )
    .unwrap();
    let stage = &e.last_planning.as_ref().unwrap().stages[0];
    assert_eq!(stage.strategy, planning::Strategy::SignedWaveBatch);
    assert!(
        stage.gate_statistics[0]["signed_rule_results"]
            .as_u64()
            .unwrap()
            > 0
    );
    let path = std::env::var_os("RERAIL_COSTS").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let count = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .and_then(|db| {
            db.query_row(
                "SELECT count(*) FROM sql_plan_observations WHERE operation='apply_changes'
                 AND json_extract(report,'$.stages[0].strategy')='signed_wave_batch'
                 AND json_extract(report,'$.stages[0].gate_statistics[0].signed_rule_results')>0",
                [],
                |row| row.get::<_, i64>(0),
            )
        });
        if matches!(count, Ok(1)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "signed gate feedback did not reach the cost store"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
