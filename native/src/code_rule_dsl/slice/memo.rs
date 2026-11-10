//! The wave's rows of a document, memoised by content (05 Phase 1, the facts memo row, on the
//! slice): a document whose blob was evaluated under the same rules before, in any scope and at any
//! path of the same dialect, takes its match and span rows from the memo instead of the wave.
//!
//! The rows are a function of the document's tape (its blob and dialect) and the rules, so the memo
//! key is the blob's source, and the plan is the rules' digest with the dialect's label. Each entry
//! also holds the core digest it was computed from: a hit whose core digest differs from the
//! document's (a tape or core format change) is a miss, so a memo never answers for other facts.

use crate::code_rule_dsl::facts::families::blob_source_id;
use crate::code_rule_dsl::facts::observe::path_name;
use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::language::label_of_path;
use crate::code_rule_dsl::waves::owner::Lease;
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, SourceId, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};

/// The memo's fact schema: bumped when the rows' meaning changes.
const SCHEMA: u32 = 2;

/// One document's wave rows without its key: matches (clause, node) and spans (node, start, end).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Rows {
    pub matches: Vec<(i64, i64)>,
    pub spans: Vec<(i64, i64, i64)>,
}

/// The memo's plan for `path` under the rules `digest`.
fn plan(rules: &[u8; 32], path: &str) -> FactsPlanSet {
    let mut hash = Sha256::new();
    hash.update(b"rerail.slice-wave/v1\0");
    hash.update(rules);
    hash.update(label_of_path(path).unwrap_or("").as_bytes());
    FactsPlanSet::new(vec![hash.finalize().into()])
}

/// The blob source the document's path binds, if stored.
fn source(store: &ContentStore, scope: &str, document: &Document) -> StoreResult<Option<SourceId>> {
    match store.binding(&path_name(scope, &document.path))? {
        Some(blob) => blob_source_id(store, blob),
        None => Ok(None),
    }
}

/// The memoised rows of `document` under the rules `digest`, when its blob was evaluated with the
/// same core facts.
pub(super) fn lookup(store: &ContentStore, scope: &str, rules: &[u8; 32], document: &Document) -> StoreResult<Option<Rows>> {
    let Some(source) = source(store, scope, document)? else { return Ok(None) };
    let set = plan(rules, &document.path);
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: &document.path })?;
    let Some(result) = memo.results.get(&set.plans[0]) else { return Ok(None) };
    // A wave that reads its rows uses the plan as much as one that writes them.
    hold(store, set.plans[0])?;
    let (core, rows) = decode(&result.entry)?;
    Ok((core == document.digest).then_some(rows))
}

/// Memoises the rows the wave left in `match` and `span` for each of `documents`.
pub(super) fn remember(lease: &Lease<'_>, store: &ContentStore, scope: &str, rules: &[u8; 32], documents: &[Document]) -> StoreResult<()> {
    let mut matches = lease.prepare("SELECT c0, c2 FROM matched WHERE c1 = ?1")?;
    let mut spans = lease.prepare("SELECT c1, c2, c3 FROM span WHERE c0 = ?1")?;
    for document in documents {
        let Some(source) = source(store, scope, document)? else { continue };
        let key = i64::from(document.key);
        let rows = Rows {
            matches: matches.query_map([key], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?,
            spans: spans.query_map([key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.collect::<rusqlite::Result<_>>()?,
        };
        let set = plan(rules, &document.path);
        hold(store, set.plans[0])?;
        let result = FactsResult { plan: set.plans[0], path_dependent: false, entry: encode(&document.digest, &rows) };
        store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: &document.path }, &set, &[result], &[])?;
    }
    Ok(())
}

/// How long a plan stays live after the last wave that wrote under it (`hold`).
const HOLD_SECS: i64 = 86_400;

/// Holds `plan` live for `HOLD_SECS` (`ContentStore::hold_corpus_plan`), renewed at most hourly per process. The
/// rows of a plan no wave has written under for that long (rules since changed) are collected by the store's
/// corpus pass (decision 0053), as a corpus run's dead plans are; their sources may live on in later snapshots.
fn hold(store: &ContentStore, plan: [u8; 32]) -> StoreResult<()> {
    static HELD: std::sync::Mutex<Option<std::collections::HashMap<[u8; 32], i64>>> = std::sync::Mutex::new(None);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let mut held = HELD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let held = held.get_or_insert_with(Default::default);
    if held.get(&plan).is_some_and(|&at| now - at < 3_600) {
        return Ok(());
    }
    store.hold_corpus_plan(plan, "slice-memo", now + HOLD_SECS)?;
    held.insert(plan, now);
    Ok(())
}

/// Inserts `rows` as document `key`'s into the wave's kept relations.
pub(super) fn keep(lease: &Lease<'_>, key: u32, rows: &Rows, kept_match: &str, kept_span: &str) -> StoreResult<()> {
    let key = i64::from(key);
    let mut insert = lease.prepare(&format!("INSERT INTO {kept_match}(c0, c1, c2) VALUES (?1, ?2, ?3)"))?;
    for &(clause, node) in &rows.matches {
        insert.execute([clause, key, node])?;
    }
    let mut insert = lease.prepare(&format!("INSERT INTO {kept_span}(c0, c1, c2, c3) VALUES (?1, ?2, ?3, ?4)"))?;
    for &(node, start, end) in &rows.spans {
        insert.execute([key, node, start, end])?;
    }
    Ok(())
}

fn encode(core: &[u8; 32], rows: &Rows) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32 + 16 + rows.matches.len() * 16 + rows.spans.len() * 24);
    bytes.extend_from_slice(core);
    bytes.extend_from_slice(&(rows.matches.len() as u64).to_le_bytes());
    for &(a, b) in &rows.matches {
        bytes.extend_from_slice(&a.to_le_bytes());
        bytes.extend_from_slice(&b.to_le_bytes());
    }
    bytes.extend_from_slice(&(rows.spans.len() as u64).to_le_bytes());
    for &(a, b, c) in &rows.spans {
        for value in [a, b, c] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn decode(bytes: &[u8]) -> StoreResult<([u8; 32], Rows)> {
    let invalid = || StoreError::Corrupt("a slice memo entry is truncated".to_owned());
    let mut at = 0;
    let mut take = |n: usize| -> StoreResult<&[u8]> {
        let slice = bytes.get(at..at + n).ok_or_else(invalid)?;
        at += n;
        Ok(slice)
    };
    let core: [u8; 32] = take(32)?.try_into().expect("32 bytes");
    let mut word = || -> StoreResult<i64> { Ok(i64::from_le_bytes(take(8)?.try_into().expect("8 bytes"))) };
    let mut rows = Rows::default();
    for _ in 0..word()? {
        rows.matches.push((word()?, word()?));
    }
    for _ in 0..word()? {
        rows.spans.push((word()?, word()?, word()?));
    }
    Ok((core, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memo_entry_round_trips() {
        let rows = Rows { matches: vec![(1, 7), (3, -1)], spans: vec![(7, 10, 20)] };
        let core = [9; 32];
        assert_eq!(decode(&encode(&core, &rows)).unwrap(), (core, rows));
        assert!(decode(&encode(&core, &Rows::default())[..40]).is_err());
    }
}
