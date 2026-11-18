//! The source summary phase uses the same request and retention machinery as other rules.

use super::{Error, Result, front};
use crate::code_rule_dsl::logic::plan::output::{Outputs, Value};
use crate::content_store::{ContentStore, HashKey, StoreError};
use std::{collections::HashMap, path::Path, sync::{Arc, OnceLock}};

pub(super) const INPUT: &str = "reference_source";
pub(super) const REQUEST: &str = "reference_summary_request";
pub(super) const ANSWER: &str = "reference_summary_of";
pub(super) const LINK_REQUEST: &str = "reference_link_request";
pub(super) const LINK_ANSWER: &str = "reference_link_of";
pub(super) const LINK_INPUT: &str = "reference_link_input";
const STAMPS: &str = "reference_binding_stamp";
const DEPENDENCIES: &str = "reference_link_dependency";
const NAMES: &str = "reference_link_name";
const CONTRACTS: &str = "reference_link_contracts";

pub(super) fn is_input(relation: &str) -> bool {
    matches!(relation, INPUT | STAMPS | DEPENDENCIES | NAMES | CONTRACTS | LINK_INPUT)
}

#[derive(Clone, Copy, Default)]
pub(super) enum Input<'a> {
    #[default]
    None,
    Sources(&'a [Source]),
    Bindings(&'a Links<'a>),
    Links(&'a [Vec<front::Value>], &'a (dyn Fn(&str) -> String + Sync)),
}

pub struct Links<'a> {
    pub observations: &'a [(String, String)],
    pub dependencies: &'a [(String, String)],
    pub requests: &'a [(String, String)],
    pub constructors: &'a str,
    pub resolve: &'a (dyn Fn(&str) -> String + Sync),
}

impl Links<'_> {
    pub(super) fn facts(&self) -> impl Iterator<Item = (String, Vec<Vec<front::Value>>)> {
        use front::Value::Str;
        [
            (STAMPS, self.observations),
            (DEPENDENCIES, self.dependencies),
            (NAMES, self.requests),
        ].into_iter().map(|(relation, tuples)| (relation.to_owned(), tuples.iter()
            .map(|(left, right)| vec![Str(left.as_str().into()), Str(right.as_str().into())]).collect()))
            .chain([(CONTRACTS.to_owned(), vec![vec![Str(self.constructors.into())]])])
    }
}

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
    let image = super::run_fact_image_with_references(&program, root, &options, Input::Sources(sources))?;
    let rows = Outputs::of(&image).map_err(Error::Plan)?;
    rows.rows("reference_source_summary").into_iter().map(|row| {
        let (Value::Text(path), Value::Text(bytes)) = (row.get(0), row.get(1))
            else { return Err(Error::Plan("invalid reference summary fact".into())) };
        Ok((path.to_owned(), bytes.to_owned()))
    }).collect()
}

pub fn link(root: &Path, links: &Links<'_>) -> Result<HashMap<String, String>> {
    if links.requests.is_empty() { return Ok(HashMap::new()); }
    let derive_keys = crate::code_rule_dsl::logic::rr::parse(concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rr"), "\n",
        include_str!("../../../typing/programs/reference-link-keys.rr"),
    )).map_err(|err| Error::Plan(err.to_string()))?;
    let options = serde_json::json!({ "reuse": false, "inputFiles": [] });
    let image = super::run_fact_image_with_references(&derive_keys, root, &options, Input::Bindings(links))?;
    let rows = Outputs::of(&image).map_err(Error::Plan)?;
    let keys = rows.rows(LINK_INPUT).iter().map(|row| (0..3).map(|column| match row.get(column) {
        Value::Text(value) => Ok(front::Value::Str(value.into())),
        _ => Err(Error::Plan("invalid reference link input".into())),
    }).collect::<Result<Vec<_>>>()).collect::<Result<Vec<_>>>()?;
    let program = crate::code_rule_dsl::logic::rr::parse(concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rr"), "\n",
        include_str!("../../../typing/programs/reference-links.rr"),
    )).map_err(|err| Error::Plan(err.to_string()))?;
    let image = super::run_fact_image_with_references(&program, root, &options, Input::Links(&keys, links.resolve))?;
    let rows = Outputs::of(&image).map_err(Error::Plan)?;
    rows.rows("reference_name_result").into_iter().map(|row| {
        let (Value::Text(name), Value::Text(result)) = (row.get(0), row.get(1))
            else { return Err(Error::Plan("invalid reference link fact".into())) };
        Ok((name.to_owned(), result.to_owned()))
    }).collect()
}
