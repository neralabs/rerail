use super::*;
use rusqlite::Connection;

#[test]
fn cached_statements_count_each_execution_and_leave_a_rejected_statement_inside_the_cap() {
    let db = Connection::open_in_memory().unwrap();
    let ledger = Ledger::new();
    let _observer = unsafe { observer::Observer::install(&db, ledger.clone(), None) };
    ledger.configure(Some(6)).unwrap();
    let _scope = ledger.enter("query").unwrap();
    let mut statement = db.prepare_cached("SELECT 1").unwrap();
    for _ in 0..5 {
        assert_eq!(statement.query_row([], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
    assert!(statement.query_row([], |r| r.get::<_, i64>(0)).is_err());
    assert_eq!(ledger.snapshot()["executed"], 6);
    assert!(ledger.enter("query").is_err());
    ledger.configure(Some(9)).unwrap();
    statement.query_row([], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(
        ledger.snapshot()["executed"],
        7,
        "Increasing a cap does not reset the session"
    );
}

#[test]
fn reservations_share_one_ledger_and_return_unused_credits() {
    let ledger = Ledger::new();
    ledger.configure(Some(100)).unwrap();
    let _scope = ledger.enter("run").unwrap();
    let reservation = ledger.reserve(80).unwrap();
    assert!(ledger.reserve(12).is_err());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let reservation = reservation.clone();
            scope.spawn(move || {
                for _ in 0..10 {
                    assert!(reservation.charge());
                }
            });
        }
    });
    assert_eq!(ledger.snapshot()["executed"], 80);
    assert_eq!(ledger.snapshot()["reserved"], 0);
    drop(reservation);
    let unused = ledger.reserve(5).unwrap();
    drop(unused);
    assert_eq!(ledger.snapshot()["reserved"], 0);
    assert_eq!(ledger.available(), 11);
}

#[test]
fn exhaustion_rolls_back_inside_the_same_allowance() {
    let db = Connection::open_in_memory().unwrap();
    let ledger = Ledger::new();
    let _observer = unsafe { observer::Observer::install(&db, ledger.clone(), None) };
    ledger.configure(Some(16)).unwrap();
    let _scope = ledger.enter("run").unwrap();
    db.execute_batch("CREATE TABLE r(v); SAVEPOINT run")
        .unwrap();
    for i in 0..5 {
        db.execute("INSERT INTO r VALUES(?1)", [i]).unwrap();
    }
    assert!(db.execute("INSERT INTO r VALUES(99)", []).is_err());
    let _ = db.execute_batch("ROLLBACK TO run; RELEASE run");
    assert!(db.is_autocommit());
    assert!(ledger.snapshot()["executed"].as_u64().unwrap() <= 16);
    drop(_scope);
    ledger.configure(None).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM r", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn session_owner_and_workers_report_one_budget_without_profiling() {
    use crate::code_rule_dsl::sql_executor::CodeRuleSqlExecutor;
    let dir = tempfile::tempdir().unwrap();
    let mut e = CodeRuleSqlExecutor::new(
        dir.path()
            .join("data.sqlite")
            .to_string_lossy()
            .into_owned(),
    )
    .unwrap();
    let used: serde_json::Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    let used = used["sql_budget"]["executed"].as_u64().unwrap();
    e.set_limits(serde_json::json!({"max_submissions":used+15}).to_string())
        .unwrap();
    e.query_plan(serde_json::json!({"nodes":[{"name":"a","sql":"SELECT 1","width":1,"reads":[]},{"name":"b","sql":"SELECT 2","width":1,"reads":[]}],"outputs":["a","b"]}).to_string()).unwrap();
    let after: serde_json::Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    assert!(after["sql_budget"]["executed"].as_u64().unwrap() > used);
    assert_eq!(after["sql_budget"]["reserved"], 0);
    let mut failed = false;
    for _ in 0..20 {
        if e.query("SELECT 1".into(), None, None).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    let final_state: serde_json::Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    assert!(final_state["sql_budget"]["executed"].as_u64().unwrap() <= used + 15);
    e.set_limits("{}".into()).unwrap();
    e.query("SELECT 1".into(), None, None).unwrap();
}

#[test]
fn interrupted_run_does_not_publish_and_can_retry_without_resetting_usage() {
    use crate::code_rule_dsl::sql_executor::CodeRuleSqlExecutor;
    use serde_json::{Value, json};
    let dir = tempfile::tempdir().unwrap();
    let mut e =
        CodeRuleSqlExecutor::new(dir.path().join("store").to_string_lossy().into()).unwrap();
    let budget = |e: &CodeRuleSqlExecutor| {
        serde_json::from_str::<Value>(&e.explain().unwrap()).unwrap()["sql_budget"].clone()
    };
    e.bind_relation(json!({"name":"seed","columns":["n"],"rows":[[1]]}).to_string())
        .unwrap();
    let used = budget(&e)["executed"].as_u64().unwrap();
    let limit = used + 13;
    e.set_limits(json!({"max_submissions":limit}).to_string())
        .unwrap();
    let program =
        ".decl seed(n:number)\n.input seed\n.decl a(n:number)\na(n) :- seed(n).\n.output a\n";
    let err = e.run(program.into(), "python".into(), None).unwrap_err();
    assert!(
        err.reason.contains("submission allowance exhausted"),
        "{err}"
    );
    assert!(e.connection().is_autocommit());
    assert_eq!(budget(&e)["exhausted"], true);
    assert!(
        budget(&e)["executed"].as_u64().unwrap() <= limit,
        "{}",
        budget(&e)
    );
    assert!(e.connection().prepare("SELECT * FROM a").is_err());
    let consumed = budget(&e)["executed"].as_u64().unwrap();
    e.set_limits("{}".into()).unwrap();
    e.run(program.into(), "python".into(), None).unwrap();
    assert!(budget(&e)["executed"].as_u64().unwrap() > consumed);
    assert_eq!(
        e.connection()
            .query_row("SELECT c0 FROM a", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn cleanup_admission_preserves_results_when_reset_and_rebind_cannot_fit() {
    use crate::code_rule_dsl::sql_executor::CodeRuleSqlExecutor;
    use serde_json::{Value, json};
    let dir = tempfile::tempdir().unwrap();
    let mut e =
        CodeRuleSqlExecutor::new(dir.path().join("store").to_string_lossy().into()).unwrap();
    let budget = |e: &CodeRuleSqlExecutor| {
        serde_json::from_str::<Value>(&e.explain().unwrap()).unwrap()["sql_budget"].clone()
    };
    let seed = |n| json!({"name":"seed","columns":["n"],"rows":[[n]]}).to_string();
    e.bind_relation(seed(1)).unwrap();
    e.run(".decl seed(n:number)\n.input seed\n.decl a(n:number)\n.decl b(n:number)\na(n) :- seed(n).\nb(n) :- seed(n).\n.output a,b\n".into(),"python".into(),None).unwrap();
    let before = budget(&e)["executed"].as_u64().unwrap();
    e.set_limits(json!({"max_submissions":before+11}).to_string())
        .unwrap();
    assert!(
        e.reset()
            .unwrap_err()
            .reason
            .contains("owner operation needs")
    );
    assert_eq!(budget(&e)["executed"], before);
    assert!(
        e.bind_relation(seed(2))
            .unwrap_err()
            .reason
            .contains("owner operation needs")
    );
    for relation in ["seed", "a", "b"] {
        let sql = format!("SELECT * FROM {relation}");
        assert_eq!(
            e.connection()
                .query_row(&sql, [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
