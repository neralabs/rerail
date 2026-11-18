//! The source summary phase uses the same request and retention machinery as other rules.

use super::{Error, Result, front};
use crate::code_rule_dsl::logic::plan::output::{Outputs, Value};
use crate::content_store::{ContentStore, HashKey, StoreError};
use std::{collections::HashMap, path::Path, sync::{Arc, OnceLock}};

pub(super) const INPUT: &str = "reference_source";
pub(super) const REQUEST: &str = "reference_summary_request";
pub(super) const ANSWER: &str = "reference_summary_of";

#[derive(Clone)]
pub struct Source {
    pub path: String,
    pub digest: String,
    pub module: String,
    pub stub: bool,
    pub extractor: u32,
}

type Summarize = fn(crate::code_rule_dsl::tape::TapeDoc, &str, bool) -> Result<String>;
static SUMMARIZE: OnceLock<Summarize> = OnceLock::new();

pub fn install(summarize: Summarize) {
    let _ = SUMMARIZE.set(summarize);
}

pub(super) fn inputs(sources: &[Source]) -> Vec<Vec<front::Value>> {
    use front::Value::{Int, Str};
    sources.iter().map(|source| vec![Str(source.path.as_str().into()), Str(source.digest.as_str().into()),
        Str(source.module.as_str().into()), Int(i64::from(source.stub)), Int(i64::from(source.extractor))]).collect()
}

pub(super) fn answer(store: &ContentStore, root: &Path, path: &str, digest: &str, module: &str, stub: bool) -> Result<String> {
    use crate::code_rule_dsl::facts::core;
    use crate::content_store::observe::unhex;
    let digest = unhex(digest).ok_or_else(|| Error::Plan("invalid reference source digest".into()))?;
    let wanted = HashKey::from_sha256_digest(digest);
    let source = match store.source_by_hash(wanted)? {
        Some(id) => String::from_utf8(store.get_source(id)?).map_err(|err| Error::Cache(StoreError::Invalid(err.to_string())))?,
        None => {
            let source = crate::git_workspace::snapshot::read_to_string(root.join(path))
                .map_err(|err| Error::Plan(format!("{path}: {err}")))?;
            if HashKey::sha256(source.as_bytes()) != wanted {
                return Err(Error::Plan(format!("{path}: reference source changed during analysis")));
            }
            source
        }
    };
    let file = core::Source { language: "python".to_owned(), source: Arc::new(source) };
    let tape = core::restore_tape(&file, store, digest)?
        .ok_or_else(|| Error::Cache(StoreError::Invalid(format!("{path}: parsed tape is unavailable for reference analysis"))))?;
    SUMMARIZE.get().ok_or_else(|| Error::Plan("reference extractor is unavailable".into()))?(tape, module, stub)
}

pub fn run(root: &Path, sources: &[Source]) -> Result<HashMap<String, String>> {
    if sources.is_empty() { return Ok(HashMap::new()); }
    let program = crate::code_rule_dsl::logic::rr::parse(concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rr"), "\n",
        include_str!("../../../typing/programs/reference-summaries.rr"),
    )).map_err(|err| Error::Plan(err.to_string()))?;
    let options = serde_json::json!({ "reuse": false, "inputFiles": [] });
    let image = super::run_fact_image_with_references(&program, root, &options, sources)?;
    let rows = Outputs::of(&image).map_err(Error::Plan)?;
    rows.rows("reference_source_summary").into_iter().map(|row| {
        let (Value::Text(path), Value::Text(bytes)) = (row.get(0), row.get(1))
            else { return Err(Error::Plan("invalid reference summary fact".into())) };
        Ok((path.to_owned(), bytes.to_owned()))
    }).collect()
}
