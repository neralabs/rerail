use super::*;

#[test]
#[cfg(unix)]
fn replay_preserves_rows_empty_results_and_cross_query_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(include_str!(
        "../../../../native/src/code_rule_dsl/matcher/anchors/sql/schema.sql"
    ))
    .unwrap();
    db.execute_batch("INSERT INTO file(fid,nodes,path,digest,lang,src,rec,spans,checkpoints,parb,sizb,flags) VALUES(1,1,'a.ts',x'01',1,x'78',zeroblob(6),zeroblob(2),zeroblob(6),x'',x'',x'02'); INSERT INTO ftsk(rowid,k) VALUES(1,'k1')").unwrap();
    drop(db);
    for (number, term) in [(0, "k1"), (1, "absent")] {
        let stages:Vec<_>=["left_hits","right_hits"].iter().map(|name|serde_json::json!({
            "name":name,"sql":format!("CREATE TEMP TABLE \"{name}\" AS SELECT CAST(i.doc AS INTEGER) AS fid,CAST(i.offset AS INTEGER) AS pre FROM ftsk_inst i JOIN request_file q ON q.fid=i.doc WHERE i.term=?1"),
            "index":"fid,pre","temporary":true,"relationId":7+number,"class":"File","columns":"fid,pre","body":"SELECT doc,offset FROM ftsk_inst WHERE term=?1"
        })).collect();
        let mut input = serde_json::json!({"format":"rerail.candidate-replay/v1","variant":"normal","database":"input.sqlite","files":[1],"fileScope":true,"fileVersions":[[1,1,"01"]],"parameters":[{"kind":"text","value":term}],"stages":stages,"sql":"SELECT 0,fid,pre FROM left_hits UNION ALL SELECT 1,fid,pre FROM right_hits ORDER BY 1,2,3"});
        if number == 1 {
            input["format"] = serde_json::json!("rerail.candidate-replay/v2");
            input.as_object_mut().unwrap().remove("sql");
            input["queries"] = serde_json::json!([
                "SELECT 0,fid,pre FROM left_hits ORDER BY 1,2,3",
                "SELECT 1,fid,pre FROM right_hits ORDER BY 1,2,3"
            ]);
        }
        std::fs::write(
            dir.path().join(format!("candidate-0-{number}.json")),
            serde_json::to_vec(&input).unwrap(),
        )
        .unwrap();
    }
    // The older, unoptimized description must not enter the production-layout comparison.
    std::fs::write(
        dir.path().join("candidate-0-2.json"),
        br#"{"variant":"uncompacted","sql":"invalid SQL"}"#,
    )
    .unwrap();
    let results = replay::experiment(dir.path(), 0);
    assert_eq!(results.len(), 3);
    for result in &results {
        assert_eq!(result["rows"], 2);
    }
    let cached = results.iter().find(|r| r["mode"] == "shared").unwrap();
    assert_eq!(cached["reusedConstructions"], 2);
    assert_eq!(cached["retainedRows"], 1);
}

#[test]
fn cache_budget_replay_preserves_bags_and_requested_file_scope() {
    let dir = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("input.sqlite")).unwrap();
    db.execute_batch(include_str!(
        "../../../../native/src/code_rule_dsl/matcher/anchors/sql/schema.sql"
    ))
    .unwrap();
    db.execute_batch("INSERT INTO ftsk(rowid,k) VALUES(1,'k1'),(2,'k1')")
        .unwrap();
    drop(db);
    let capture = dir.path().join("candidate.json");
    let data = serde_json::json!({
        "database":"input.sqlite", "files":[1], "fileScope":true,
        "parameters":[{"kind":"text","value":"k1"}],
        "stages":[
            {"name":"hits","sql":"CREATE TEMP TABLE hits AS SELECT CAST(i.doc AS INTEGER) fid,CAST(i.offset AS INTEGER) pre FROM ftsk_inst i JOIN request_file q ON q.fid=i.doc WHERE i.term=?1","temporary":true,"index":"fid,pre"},
            {"name":"twice","sql":"CREATE TEMP TABLE twice AS SELECT fid,pre FROM hits UNION ALL SELECT fid,pre FROM hits","temporary":true,"index":null}
        ],
        "queries":["SELECT 0,fid,pre FROM hits","SELECT 1,fid,pre FROM twice"]
    });
    std::fs::write(&capture, serde_json::to_vec(&data).unwrap()).unwrap();
    let mut control_counters = None;
    for (name, main, temp) in [("control", 1024, 512), ("larger", 65536, 16384)] {
        let output = dir.path().join(format!("{name}.json"));
        residency::run_cache(&capture, main, temp, &output).unwrap();
        let rows: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.with_extension("rows.json")).unwrap())
                .unwrap();
        assert_eq!(rows, serde_json::json!([[0, 1, 0], [1, 1, 0], [1, 1, 0]]));
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        if let Some(control) = &control_counters {
            assert_eq!(&report["statementCounters"], control);
        } else {
            control_counters = Some(report["statementCounters"].clone());
        }
        assert!(report["statementCounters"]["vmSteps"].as_u64().unwrap() > 0);
        assert_eq!(report["observation"], "cache-counters");
        assert_eq!(
            report["cacheBudgetKiB"],
            serde_json::json!({"main":main,"temp":temp})
        );
        let last = report["snapshots"].as_array().unwrap().last().unwrap();
        for field in [
            "cacheHits",
            "cacheMisses",
            "cacheWrites",
            "cacheSpills",
            "cacheBytes",
        ] {
            assert!(last["sample"]["sqlite"][field].as_i64().unwrap() >= 0);
        }
    }
}
