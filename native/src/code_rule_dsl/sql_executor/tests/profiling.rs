//! Profile configuration is process-wide, so exercise it in an isolated test process.
use super::*;

const PROGRAM: &str = ".decl edge(a:number,b:number,w:number)\n.decl root(n:number)\n.input edge,root\n.decl distance(n:number,d:min<>) widen 4\ndistance(n,0) :- root(n).\ndistance(b,d+w) :- distance(a,d), edge(a,b,w).\n.output distance\n";

#[test]
fn sql_sessions_use_shared_explain_profiles_including_errors_and_native_work() {
    let directory = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "code_rule_dsl::sql_executor::tests::profiling::profiled_session_child",
            "--ignored",
            "--nocapture",
        ])
        .env("RERAIL_SQL_PROFILE_DIR", directory.path().join("sql"))
        .env("RERAIL_PROFILE", directory.path().join("logic"))
        .status()
        .unwrap();
    assert!(status.success());
    let mut native = false;
    let mut cold = false;
    let mut recursive = false;
    let mut query = false;
    let mut columns = false;
    let mut inserts = false;
    let mut failed = false;
    let mut sparse_batch = false;
    let mut files = 0;
    let mut logical = false;
    for entry in std::fs::read_dir(directory.path().join("sql")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|s| s != "json") {
            continue;
        }
        files += 1;
        let catalog: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(catalog["error"].is_null(), "{catalog}");
        assert_eq!(catalog["executionCoverage"]["running"], 0);
        assert_eq!(
            catalog["executionCoverage"]["started"],
            catalog["executionCoverage"]["completed"]
        );
        let queries = catalog["queries"].as_array().unwrap();
        for q in queries.iter().filter(|q| q["kind"] == "sql") {
            assert!(
                q.get("plan").is_some(),
                "unexplained statement at close: {q}"
            );
        }
        let events = std::fs::read_to_string(path.with_extension("jsonl")).unwrap();
        for line in events.lines() {
            let event: Value = serde_json::from_str(line).unwrap();
            failed |= event["outcome"] == "error"
                && event["completed"] == true
                && event["context"]["error"].is_string();
            if event["kind"] == "operator" {
                native = true;
            }
            if event["kind"] != "sql" {
                continue;
            }
            logical |= event["context"]["logicalAnalyses"].as_array().is_some_and(|a| !a.is_empty()) || event["context"]["logicalAnalysis"].is_object();
            let stage = event["context"]["stage"].as_str().unwrap_or("");
            cold |= stage == "cold rule";
            recursive |= event["context"].get("frame").is_some();
            columns |= event["frame"][3].as_u64().unwrap_or(0) > 0;
            inserts |= event["frame"][4].as_u64().unwrap_or(0) > 0;
            let q = &queries[event["query"].as_u64().unwrap() as usize];
            if q["sql"].as_str().unwrap().contains("/* sparse compound */") {
                // Includes native candidate inspections, not only rows returned to SQLite.
                assert!(event["frame"][2].as_u64().unwrap() < 100, "{event}");
                sparse_batch = true;
            }
            if event["context"]["operation"] == "query" {
                query = true;
                let q = &queries[event["query"].as_u64().unwrap() as usize];
                assert!(q["plan"].is_array(), "{q}");
                assert!(q["bytecode"].is_array(), "{q}");
            }
            assert!(
                !queries[event["query"].as_u64().unwrap() as usize]["sql"]
                    .as_str()
                    .unwrap()
                    .starts_with("EXPLAIN ")
            );
        }
    }
    assert!(
        files >= 2
            && native
            && cold
            && recursive
            && query
            && columns
            && inserts
            && failed
            && sparse_batch
            && logical
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let output = std::process::Command::new("node")
        .arg(root.join("scripts/profiling/sql-report.mjs"))
        .arg("--sql-session")
        .arg(directory.path().join("sql"))
        .arg(directory.path().join("report"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("report/profile.json")).unwrap(),
    )
    .unwrap();
    assert!(report["captureCoverage"]["complete"].as_bool().unwrap());
    assert!(
        report["executionInventory"]["phases"]["relationalEvaluation"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_ne!(report["executionBudget"]["accepted"], true);
    let report = std::fs::read_to_string(directory.path().join("logic/profile.txt")).unwrap();
    assert!(
        report.contains("SQL extension request") && report.contains("EXPLAIN ANALYZE"),
        "{report}"
    );
    assert!(
        report.contains("full query") && report.contains("delta query"),
        "{report}"
    );
    assert!(report.contains("logical uniqueness and scalar summaries") && report.contains("support_uniqueness"),"{report}");
}

#[test]
#[ignore = "child process of the SQL profile integration test"]
fn profiled_session_child() {
    assert!(std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some());
    let (_directory, mut e) = executor();
    for (name, columns, data) in [
        (
            "edge",
            json!(["a", "b", "w"]),
            json!([[0, 1, 5], [0, 2, 1], [2, 1, 1], [1, 2, 1]]),
        ),
        ("root", json!(["n"]), json!([[0]])),
    ] {
        e.bind_relation(json!({"name":name,"columns":columns,"rows":data}).to_string())
            .unwrap();
    }
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    let totals = e.sql_profile_totals();
    let explanation: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    assert!(!explanation["executions"][0]["recursive"].as_array().unwrap().is_empty());
    assert_eq!(e.sql_profile_totals(), totals, "explain must not execute SQL");
    let patch = json!([{"name":"edge","delete":[[2,1,1]]}]).to_string();
    e.set_limits(json!({"max_steps":1}).to_string()).unwrap();
    assert!(e.apply_changes(patch.clone(), None).is_err());
    e.set_limits("{}".into()).unwrap();
    e.apply_changes(patch, None).unwrap();
    let result: Value = serde_json::from_str(
        &e.query(
            "WITH selected AS(SELECT * FROM distance WHERE c0=?1) SELECT * FROM selected".into(),
            Some("[1]".into()),
            None,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result["rows"], json!([[1, 5]]));
    e.reset().unwrap();
    let versions = versions::Store::default();
    versions
        .create(&e.db, "batch_rows", versions::Schema::integers(3, 2))
        .unwrap();
    versions.restrict_indexes("batch_rows", &[]).unwrap();
    e.db.execute_batch("WITH RECURSIVE n(i) AS(VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<20000) INSERT INTO batch_rows SELECT 1,i,i%7 FROM n").unwrap();
    assert_eq!(
        e.db.query_row(
            "SELECT count(*) FROM batch_rows WHERE c0=1 AND c1 IN(17,811) /* sparse compound */",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    e.db.profile_checkpoint();
    versions.close(&e.db).unwrap();
    drop(e);
    let store = tempfile::tempdir().unwrap();
    super::super::super::tools::sql(&store.path().join("constant.sqlite"),&json!({"program":".decl answer(n:number)\nanswer(7).\n.output answer\n","queries":[{"sql":"SELECT * FROM answer"}]}).to_string()).unwrap();
    super::super::super::tools::sql(&store.path().join("store.sqlite"), &json!({"relations":[{"name":"edge","columns":["a","b","w"],"rows":[[0,1,2],[1,0,1]]},{"name":"root","columns":["n"],"rows":[[0]]}],"program":PROGRAM,"queries":[{"sql":"SELECT * FROM distance"}]}).to_string()).unwrap();
}
