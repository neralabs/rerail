//! Y3: Y1's maintained graph and Y2's local rules wired into one cross-file path, with the
//! symbol table maintained in Y1's style (`sym`) instead of the dataflow. `wire` is the per-edit
//! path, `world` the model of the layers and worktrees. See facts-engine's
//! docs/facts/reports/explore-y3-wire.md.
//!
//! As copied from facts-b, less the three small plans (`m2`, P3a's M2 over facts-b's `gen`), the
//! `symcheck` and `replay` drivers (`replay.rs` not copied) and `spec`: the replay is the
//! dead-code producer's workload test (`facts::deadcode`).
pub mod input;
pub mod sym;
pub mod wire;
pub mod world;

use std::path::{Path, PathBuf};

pub fn bench() -> PathBuf {
    crate::code_rule_dsl::facts::ygraph::bench()
}

/// One replay event (S3's `events.jsonl`).
pub struct Event {
    pub seq: u64,
    pub worktree: u32,
    pub group: String,
    pub kind: String,
    pub changes: Vec<(String, Option<String>, Option<String>)>,
}

pub fn read_events(p: &Path) -> Vec<Event> {
    std::fs::read_to_string(p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let j: serde_json::Value = serde_json::from_str(l).unwrap();
            Event {
                seq: j["seq"].as_u64().unwrap(),
                worktree: j["worktree"].as_u64().unwrap() as u32,
                group: j["group"].as_str().unwrap().to_owned(),
                kind: j["kind"].as_str().unwrap_or("").to_owned(),
                changes: j["changes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        let c = c.as_array().unwrap();
                        (c[0].as_str().unwrap().to_owned(), c[1].as_str().map(str::to_owned), c[2].as_str().map(str::to_owned))
                    })
                    .collect(),
            }
        })
        .collect()
}
