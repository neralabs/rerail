use super::*;
use std::time::{Duration, Instant};

const WRITER: &str = "code_rule_dsl::sql_executor::tests::planning::feedback_child";

#[test]
fn sql_plan_feedback_writes_in_background_and_loads_in_a_fresh_process() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("costs.sqlite");
    for mode in ["write", "read"] {
        let child = std::process::Command::new("nice")
            .args(["-n", "19"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", WRITER, "--ignored", "--nocapture"])
            .env("RERAIL_COSTS", &db)
            .env("RERAIL_THREADS", "2")
            .env("RERAIL_TEST_SQL_PLAN_FEEDBACK", mode)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr),
        );
    }
}

#[test]
#[ignore = "child process of SQL planning feedback test"]
fn feedback_child() {
    use crate::code_rule_dsl::logic::costs::sql_plans::{self, Operation, Outcome, Strategy};
    let (_dir, mut e) = executor();
    let base = (0..32).flat_map(|i| [[i * 4, i * 4 + 1], [i * 4 + 1, i * 4 + 2]]);
    edge(&mut e, json!(base.collect::<Vec<_>>()));
    let program = format!(
        "{CLOSURE}\n.decl total(n:number)\n\
         total(n) :- n=count : {{edge(_,_)}}.\n\
         total(n) :- total(n), n=count : {{edge(_,_)}}.\n.output total\n"
    );
    e.run(program, "python".into(), None).unwrap();
    let change = |e: &mut CodeRuleSqlExecutor, kind: &str| {
        let before = e.db.admission().counters();
        e.apply_changes(json!([{"name":"edge", (kind):[[2,3]]}]).to_string(), None)
            .unwrap();
        let after = e.db.admission().counters();
        let report = e.last_planning.as_ref().unwrap();
        assert_eq!(report.outcome, Outcome::Completed);
        assert_eq!(report.metrics.submissions, after.0 - before.0);
        assert_eq!(report.metrics.vm_steps, after.1 - before.1);
        assert!(report.metrics.vm_steps > 0);
        assert_eq!(report.stages.len(), 1);
        assert_eq!(report.stages[0].strategy, Strategy::SparseRetained);
        assert_eq!(report.stages[0].inputs.changed_relations, 1);
        assert!(report.stages[0].metrics.submissions <= report.metrics.submissions);
    };
    let mode = std::env::var("RERAIL_TEST_SQL_PLAN_FEEDBACK").unwrap();
    if mode == "write" {
        for _ in 0..5 {
            change(&mut e, "insert");
            change(&mut e, "delete");
        }
        let used = e.db.admission().counters().0;
        e.set_limits(json!({"max_submissions":used + 8}).to_string())
            .unwrap();
        assert!(
            e.apply_changes(r#"[{"name":"edge","insert":[[2,3]]}]"#.into(), None)
                .is_err()
        );
        assert_eq!(
            e.last_planning.as_ref().unwrap().outcome,
            Outcome::SubmissionLimit
        );
        assert_eq!(rows_after_reset(&mut e), json!([[96]]));
        let db = std::env::var_os("RERAIL_COSTS").unwrap();
        wait_until(|| {
            let Ok(db) = rusqlite::Connection::open_with_flags(
                &db,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                return false;
            };
            let counts: rusqlite::Result<(i64, i64)> = db.query_row(
                "SELECT count(*),sum(outcome='submission_limit') FROM sql_plan_observations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            );
            matches!(counts, Ok((12, 1)))
        });
    } else {
        change(&mut e, "insert");
        let inputs = e.last_planning.as_ref().unwrap().stages[0].inputs.clone();
        wait_until(|| {
            sql_plans::estimate(Operation::ApplyChanges, &inputs, Strategy::SparseRetained).samples
                >= 5
        });
        change(&mut e, "delete");
        change(&mut e, "insert");
        let explanation = e.explain_value();
        let forecast = &explanation["sql_planning"]["last_operation"]["stages"][0]["forecast"];
        assert!(forecast["max_observed_submissions"].as_u64().unwrap() > 0);
        assert!(forecast["median_vm_steps"].as_u64().unwrap() > 0);
        assert_eq!(rows(&mut e, "SELECT count(*) FROM reach"), json!([[99]]));
    }
}

fn rows_after_reset(e: &mut CodeRuleSqlExecutor) -> Value {
    e.set_limits("{}".into()).unwrap();
    rows(e, "SELECT count(*) FROM reach")
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "SQL plan feedback did not arrive"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
