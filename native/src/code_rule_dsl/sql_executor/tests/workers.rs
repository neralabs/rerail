use super::*;
const PROGRAM: &str = ".decl seed(n:number)\n.input seed\n.decl a(n:number)\n.decl b(n:number)\na(n) :- seed(n).\nb(n) :- seed(n).\n.output a,b\n";
#[test]
fn independent_checked_transitions_use_workers_and_publish_atomically() {
    if rerail_accel::exec::threads() < 2 {
        return;
    }
    let (_dir, mut e) = executor();
    e.bind_relation(
        json!({"name":"seed","columns":["n"],"rows":(0..1024).map(|n|[n]).collect::<Vec<_>>()} )
            .to_string(),
    )
    .unwrap();
    let receipts = e.db.admission().receipts();
    let profiled_before = e.sql_profile_totals();
    let used_before = e.db.admission().snapshot()["executed"].as_u64().unwrap();
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    let explain: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    assert!(
        explain["checked_sql_work_plan"]["groups"]
            .as_array()
            .unwrap()
            .len()
            >= 2,
        "{explain}"
    );
    assert!(
        receipts
            .lock()
            .iter()
            .any(|r| r.sql.starts_with("WITH RECURSIVE")),
        "Worker statements share owner accounting"
    );
    if let Some((before, after)) = profiled_before.zip(e.sql_profile_totals()) {
        assert_eq!(
            after[0] - before[0],
            explain["sql_budget"]["executed"].as_u64().unwrap() - used_before
        );
    }
    for name in ["a", "b"] {
        assert_eq!(
            rows(&mut e, &format!("SELECT count(*) FROM {name}")),
            json!([[1024]])
        );
    }
    e.apply_changes(
        json!([{"name":"seed","delete":[[0]],"insert":[[2048]]}]).to_string(),
        None,
    )
    .unwrap();
    for name in ["a", "b"] {
        assert_eq!(
            rows(
                &mut e,
                &format!("SELECT c0 FROM {name} WHERE c0 IN(0,2048)")
            ),
            json!([[2048]])
        );
    }
}
#[test]
fn operation_vm_limits_keep_checked_work_on_the_limited_owner() {
    let (_dir, mut e) = executor();
    e.bind_relation(
        json!({"name":"seed","columns":["n"],"rows":(0..1024).map(|n|[n]).collect::<Vec<_>>()} )
            .to_string(),
    )
    .unwrap();
    e.set_limits(json!({"max_steps":1000000}).to_string())
        .unwrap();
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    assert!(e.scratch.worker_plan.lock().is_none());
    assert_eq!(rows(&mut e, "SELECT count(*) FROM a"), json!([[1024]]));
}
