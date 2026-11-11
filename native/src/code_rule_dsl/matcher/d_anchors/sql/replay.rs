//! Captured-input SQL layout laboratory, shared by the native tests and the small replay host.
use super::store::Store;
use rusqlite::{Result, params_from_iter, types::Value};
use serde_json::Value as Json;
use std::collections::{HashMap, HashSet};
type CandidateRows = Vec<Vec<(i64, i64, i64)>>;

pub(super) const SCHEMA:&str="
CREATE TABLE lab_epoch(value INTEGER); INSERT INTO lab_epoch VALUES(0);
CREATE TABLE lab_revision(fid INTEGER PRIMARY KEY REFERENCES file(fid) ON DELETE CASCADE,revision INTEGER NOT NULL,UNIQUE(fid,revision));
INSERT INTO lab_revision SELECT fid,row_number() OVER(ORDER BY fid) FROM file;
UPDATE lab_epoch SET value=(SELECT count(*) FROM file);
CREATE TRIGGER lab_revision_added AFTER INSERT ON file BEGIN
 UPDATE lab_epoch SET value=value+1;
 INSERT INTO lab_revision SELECT NEW.fid,value FROM lab_epoch;
END;
CREATE TABLE lab_node(relation_id INTEGER NOT NULL,fid INTEGER NOT NULL,revision INTEGER NOT NULL,pre INTEGER NOT NULL,
 PRIMARY KEY(relation_id,fid,pre),FOREIGN KEY(fid,revision) REFERENCES lab_revision(fid,revision) ON DELETE CASCADE) WITHOUT ROWID;
CREATE TABLE lab_ready(relation_id INTEGER NOT NULL,fid INTEGER NOT NULL,revision INTEGER NOT NULL,
 PRIMARY KEY(relation_id,fid,revision),FOREIGN KEY(fid,revision) REFERENCES lab_revision(fid,revision) ON DELETE CASCADE) WITHOUT ROWID;
CREATE TABLE lab_request(fid INTEGER PRIMARY KEY);
";

fn run_sql(store: &Store, sql: &str, parameters: &[Value]) -> Result<usize> {
    store.readers.clear();
    let mut q = store.db.prepare(sql)?;
    let n = q.parameter_count();
    let changed = q.execute(params_from_iter(&parameters[..n]));
    drop(q);
    store.readers.clear();
    changed
}

fn replay(captures: &[std::path::PathBuf], mode: &str) -> (Json, CandidateRows) {
    let inputs: Vec<Json> = captures
        .iter()
        .map(|p| serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap())
        .collect();
    let data = &inputs[0];
    let database = captures[0]
        .parent()
        .unwrap()
        .join(data["database"].as_str().unwrap());
    let temporary = tempfile::NamedTempFile::new().unwrap();
    std::fs::copy(database, temporary.path()).unwrap();
    let started = crate::code_rule_dsl::profile::thread_cpu_ns().unwrap();
    let store = Store::open(temporary.path()).unwrap();
    let mut counts = HashMap::new();
    for data in &inputs {
        for stage in data["stages"].as_array().unwrap() {
            if let Some(id) = stage["relationId"].as_u64() {
                *counts.entry(id).or_insert(0) += 1;
            }
        }
    }
    let mut reused = 0;
    let mut retained = HashSet::new();
    let mut bags = HashSet::new();
    store.db.execute_batch("BEGIN").unwrap();
    if mode != "temporary" {
        store.db.execute_batch(SCHEMA).unwrap();
        let files = data["files"].as_array().unwrap();
        for files in files.chunks(128) {
            let sql = format!(
                "INSERT INTO lab_request VALUES {}",
                std::iter::repeat_n("(?)", files.len())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            store
                .db
                .execute(
                    &sql,
                    params_from_iter(files.iter().map(|v| v.as_i64().unwrap())),
                )
                .unwrap();
        }
    }
    let mut output = Vec::new();
    let mut temp_bytes = 0;
    for data in &inputs {
        let mut views = HashSet::new();
        if data["fileScope"] == true {
            super::session::request(
                &store.db,
                &data["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
        let parameters: Vec<_> = data["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| match v["kind"].as_str().unwrap() {
                "integer" => Value::Integer(v["value"].as_i64().unwrap()),
                "real" => Value::Real(v["value"].as_f64().unwrap()),
                "text" => Value::Text(v["value"].as_str().unwrap().to_owned()),
                "null" => Value::Null,
                _ => panic!("unexpected capture parameter"),
            })
            .collect();
        let stages = data["stages"].as_array().unwrap();
        for stage in stages {
            let name = stage["name"].as_str().unwrap();
            let sql = stage["sql"].as_str().unwrap();
            let id = stage["relationId"].as_u64();
            let eligible = stage["temporary"] == true
                && stage["columns"] == "fid,pre"
                && stage["class"] == "File"
                && id.is_some()
                && match mode {
                    "postings" => stage["body"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("ftsk_inst"),
                    "shared" => counts[&id.unwrap()] > 1,
                    _ => false,
                };
            if eligible && !bags.contains(&id.unwrap()) {
                let id = id.unwrap();
                let mut ready = retained.contains(&id);
                if ready {
                    reused += 1;
                } else {
                    let select = sql
                        .strip_prefix(&format!("CREATE TEMP TABLE \"{name}\" AS "))
                        .unwrap();
                    let insert = format!(
                        "INSERT INTO lab_node SELECT {id},q.fid,r.revision,q.pre FROM ({select}) q JOIN lab_revision r ON r.fid=q.fid"
                    );
                    match run_sql(&store, &insert, &parameters) {
                        Ok(_) => {
                            store.db.execute(&format!("INSERT INTO lab_ready SELECT {id},r.fid,r.revision FROM lab_request q JOIN lab_revision r ON r.fid=q.fid"),[]).unwrap();
                            retained.insert(id);
                            ready = true;
                        }
                        Err(rusqlite::Error::SqliteFailure(e, _))
                            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY =>
                        {
                            bags.insert(id);
                        }
                        Err(e) => panic!("retained construction {name}: {e}"),
                    }
                }
                if ready {
                    store.db.execute_batch(&format!("CREATE TEMP VIEW \"{name}\" AS SELECT n.fid,n.pre FROM lab_node n JOIN lab_ready r USING(relation_id,fid,revision) JOIN lab_request q ON q.fid=n.fid WHERE n.relation_id={id}")).unwrap();
                    views.insert(name.to_owned());
                    continue;
                }
            }
            run_sql(&store, sql, &parameters).unwrap_or_else(|e| panic!("{name}: {e}"));
            if let Some(columns) = stage["index"].as_str() {
                store
                    .db
                    .execute_batch(&format!(
                        "CREATE INDEX temp.\"{name}_keys\" ON \"{name}\"({columns})"
                    ))
                    .unwrap();
            }
        }
        let queries: Vec<_> = data
            .get("queries")
            .and_then(Json::as_array)
            .map(|queries| queries.iter().map(|q| q.as_str().unwrap()).collect())
            .unwrap_or_else(|| vec![data["sql"].as_str().unwrap()]);
        let mut rows = Vec::new();
        for sql in queries {
            let mut q = store.db.prepare(sql).unwrap();
            let n = q.parameter_count();
            rows.extend(
                q.query_map(params_from_iter(&parameters[..n]), |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>>>()
                .unwrap(),
            );
            drop(q);
        }
        store.readers.clear();
        temp_bytes=temp_bytes.max(store.db.query_row("SELECT (SELECT page_count FROM pragma_page_count('temp'))*(SELECT page_size FROM pragma_page_size('temp'))",[],|r|r.get::<_,i64>(0)).unwrap());
        for stage in stages.iter().rev().filter(|s| s["temporary"] == true) {
            let name = stage["name"].as_str().unwrap();
            store
                .db
                .execute_batch(&format!(
                    "DROP {} \"{name}\"",
                    if views.contains(name) {
                        "VIEW"
                    } else {
                        "TABLE"
                    }
                ))
                .unwrap();
        }
        output.push(rows);
    }
    store.db.execute_batch("COMMIT").unwrap();
    let cpu_ms = (crate::code_rule_dsl::profile::thread_cpu_ns().unwrap() - started) as f64 / 1e6;
    let pages: i64 = store
        .db
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap();
    let retained_rows = if mode == "temporary" {
        0
    } else {
        store
            .db
            .query_row("SELECT count(*) FROM lab_node", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    (
        serde_json::json!({"captures":captures,"mode":mode,"cpuMs":cpu_ms,"rows":output.iter().map(Vec::len).sum::<usize>(),"retainedRelations":retained.len(),"reusedConstructions":reused,"bagRelations":bags,"retainedRows":retained_rows,"databasePages":pages,"tempReservedBytes":temp_bytes}),
        output,
    )
}

pub(crate) fn experiment(dir: &std::path::Path, trial: usize) -> Vec<Json> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|s| s == "json")
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("candidate-")
        })
        .collect();
    files.retain(|file| {
        serde_json::from_slice::<Json>(&std::fs::read(file).unwrap()).unwrap()["variant"]
            == "normal"
    });
    files.sort_by_key(|p| {
        p.file_stem()
            .unwrap()
            .to_string_lossy()
            .rsplit('-')
            .next()
            .unwrap()
            .parse::<usize>()
            .unwrap()
    });
    assert!(!files.is_empty());
    let mut groups = std::collections::BTreeMap::new();
    for file in files {
        let data: Json = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let mut fids: Vec<_> = data["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        fids.sort();
        groups
            .entry((fids, serde_json::to_string(&data["fileVersions"]).unwrap()))
            .or_insert_with(Vec::new)
            .push(file);
    }
    assert!(trial < 3);
    let mut results = Vec::new();
    for group in groups.values() {
        let (_, expected) = replay(group, "temporary");
        for step in 0..3 {
            let mode = ["temporary", "postings", "shared"][(trial + step) % 3];
            let (mut result, rows) = replay(group, mode);
            assert_eq!(rows, expected, "candidate parity {mode}");
            result["trial"] = trial.into();
            results.push(result);
        }
    }
    std::fs::write(
        dir.join(format!("retention-results-{trial}.json")),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
    println!(
        "{}",
        serde_json::json!({"trials":results.len(),"parity":"passed"})
    );

    results
}
