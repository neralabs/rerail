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
