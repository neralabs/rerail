//! The syntax tests' fixtures: the planner fixture's corpus (CPython 3.13 modules), its facts, and the corpus as a
//! batch of documents. The syntax tests and the planner's and schedule's tests read them.

use super::*;
use crate::code_rule_dsl::facts::core::{self, Built};
use crate::code_rule_dsl::model::FileInput;
use std::path::PathBuf;

pub fn root() -> PathBuf {
    crate::source_tree::native().join("src/code_rule_dsl")
}
/// CPython 3.13's `fnmatch`, `ftplib`, `glob`, `json/` and `shlex`: the planner fixture's corpus, in its `doc` order.
pub fn corpus() -> Vec<(String, String)> {
    let raw = zstd::decode_all(std::fs::File::open(root().join("facts/vtab/golden/corpus.json.zst")).unwrap()).unwrap();
    serde_json::from_slice(&raw).unwrap()
}
pub fn facts(path: &str, source: &str) -> Arc<Facts> {
    let file = FileInput::new(path.to_owned(), "python".to_owned(), Arc::new(source.to_owned().into()), None, None, "core").unwrap();
    match core::build(&file).unwrap() {
        Built::Facts(f) => Arc::new(f),
        Built::Unparsed => panic!("{path} did not parse"),
    }
}
/// The corpus as a batch, document `d` keyed `d`.
pub fn documents() -> Vec<Document> {
    corpus()
        .into_iter()
        .enumerate()
        .map(|(d, (path, source))| {
            let facts = facts(&path, &source);
            Document { key: d as u32, core: format!("/w#core:{path}"), digest: core::digest(&facts), path, facts, tape: None, guest: None, module: None, unpackaged: false }
        })
        .collect()
}
