use std::path::Path;

/// T's facts for the planner fixture (`logic/plan/golden/facts.sql.zst`) as the exporter's
/// `.facts` files, and every strategy checked against T's `workload` goldens.
#[test]
fn every_strategy_reproduces_t_workload_goldens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic/plan/golden");
    let dump = zstd::decode_all(std::fs::File::open(root.join("facts.sql.zst")).unwrap()).unwrap();
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rerail-rounds-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for rel in ["doc", "node", "tape", "child", "field", "text", "intval", "op"] {
        let mut stmt = db.prepare(&format!("SELECT * FROM {rel}")).unwrap();
        let width = stmt.column_count();
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                (0..width)
                    .map(|i| {
                        Ok(match r.get_ref(i)? {
                            rusqlite::types::ValueRef::Integer(v) => v.to_string(),
                            rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                            other => panic!("{rel}: {other:?}"),
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map(|v| v.join("\t"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        std::fs::write(dir.join(format!("{rel}.facts")), rows.join("\n") + "\n").unwrap();
    }
    super::main(dir.to_str().unwrap(), root.join("workload").to_str().unwrap());
}
