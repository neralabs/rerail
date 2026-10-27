//! Replays captured candidate statements on their captured stores: the rows and the work of the
//! statement as it ran, the baseline the lab's re-lowering is compared with.
//!
//! RERAIL_LAB_CAPTURES=<dir> cargo test --release --lib -- --ignored candidate_replay --nocapture
//!
//! From Pulling-it-all-together's `matcher/anchors/sql/replay.rs`. P also replayed each capture
//! under three layouts of retained (file, pre) relations (`lab_node`, `lab_ready`,
//! `lab_revision`) for `retention_spikes.rs`; those relations were decoded ones, which
//! transducers now produce, so that experiment, its schema and its modes are not kept.
use super::super::{lower::lifetime::Action, tests::{open, temp}};
use rusqlite::{Result, params_from_iter, types::Value};
use serde_json::Value as Json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

const STEP: i32 = 1000;
type CandidateRows = Vec<(i64, i64, i64)>;

pub(super) fn replay(capture: &std::path::Path) -> (Json, CandidateRows) {
    let data: Json = serde_json::from_slice(&std::fs::read(capture).unwrap()).unwrap();
    let database = capture.parent().unwrap().join(data["database"].as_str().unwrap());
    let copy = temp();
    std::fs::copy(database, &copy).unwrap();
    let store = open(&copy);
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
    let files: Vec<i64> = data["files"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    let counter = Arc::new(AtomicU64::new(0));
    let started = super::cpu_ns();
    let (rows, temp_bytes) = store
        .candidate_transaction(true, |db| {
            let observed = Arc::clone(&counter);
            db.progress_handler(STEP, Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }))?;
            super::super::execute::request(db, &files)?;
            for sql in data["setup"].as_array().unwrap() {
                db.execute_batch(sql.as_str().unwrap())?;
            }
            // In the captured schedule, as `execute::rows` runs it: members of an owner slot have no
            // SQL of their own. Releases are left to the end, so temp bytes are the peak.
            let stages = data["stages"].as_array().unwrap();
            let queries = data["queries"].as_array().unwrap();
            let schedule: Vec<Action> = serde_json::from_value(data["schedule"].clone()).unwrap();
            let mut rows = Vec::new();
            for action in schedule {
                match action {
                    Action::Prepare(i) => {
                        let stage = &stages[i];
                        let name = stage["name"].as_str().unwrap();
                        let mut q = db.prepare(stage["sql"].as_str().unwrap())?;
                        let n = q.parameter_count();
                        q.execute(params_from_iter(&parameters[..n])).unwrap_or_else(|e| panic!("{name}: {e}"));
                        drop(q);
                        if let Some(columns) = stage["index"].as_str() {
                            db.execute_batch(&format!("CREATE INDEX temp.\"{name}_keys\" ON \"{name}\"({columns})"))?;
                        }
                    }
                    Action::Query(q) => {
                        let mut q = db.prepare(queries[q].as_str().unwrap())?;
                        let n = q.parameter_count();
                        rows.extend(
                            q.query_map(params_from_iter(&parameters[..n]), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                                .collect::<Result<Vec<_>>>()?,
                        );
                    }
                    Action::Release(_) => {}
                }
            }
            let temp_bytes = db.query_row(
                "SELECT (SELECT page_count FROM pragma_page_count('temp'))*(SELECT page_size FROM pragma_page_size('temp'))",
                [],
                |r| r.get::<_, i64>(0),
            )?;
            db.progress_handler(0, None::<fn() -> bool>)?;
            Ok((rows, temp_bytes))
        })
        .unwrap();
    let cpu_ms = (super::cpu_ns() - started) as f64 / 1e6;
    let mut sorted = rows.clone();
    sorted.sort_unstable();
    (
        serde_json::json!({"capture":capture,"cpuMs":cpu_ms,"vmSteps":counter.load(Ordering::Relaxed) * STEP as u64,"rows":rows.len(),"tempReservedBytes":temp_bytes}),
        sorted,
    )
}

#[test]
#[ignore = "requires captured candidate databases"]
fn candidate_replay() {
    let dir = std::env::var("RERAIL_LAB_CAPTURES").expect("RERAIL_LAB_CAPTURES");
    let mut captures: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|s| s == "json") && p.file_name().unwrap().to_string_lossy().starts_with("candidate-")
        })
        .collect();
    captures.sort();
    assert!(!captures.is_empty());
    for capture in captures {
        println!("{}", replay(&capture).0);
    }
}
