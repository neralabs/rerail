//! The wave's rows of a document, memoised by content (05 Phase 1, the facts memo row, on the
//! slice): a document whose blob was evaluated under the same rules before, in any scope and at any
//! path of the same dialect, takes its match and span rows from the memo instead of the wave.
//!
//! The rows are a function of the document's tape (its blob and dialect) and the rules, so the memo
//! key is the blob's content, and the plan is the rules' digest with the dialect's label. Each entry
//! also holds the core digest it was computed from: a hit whose core digest differs from the
//! document's (a tape or core format change) is a miss, so a memo never answers for other facts.
//!
//! The entries are kept facts of the store (t33 step 3, decisions 0104 and 0456), where a facts-memo row held each
//! one encoded: one space, a partition per (content, plan), its relations `slice_memo_core(d0, d1, d2, d3)`,
//! `slice_memo_match(clause, node)` and `slice_memo_span(node, start, end)`, held by the blob's source and the plan
//! (`HOLDER`): collection drops an entry once either goes, as the memo's row died with them.

use crate::code_rule_dsl::facts::families::blob_source_id;
use crate::code_rule_dsl::facts::observe::path_name;
use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::language::label_of_path;
use crate::code_rule_dsl::waves::owner::Lease;
use crate::content_store::kept_facts::{KeptPut, KeptSpace, KeptWhat, holder_put};
use crate::content_store::{ContentStore, StoreResult};
use rerail_blocks::kept::{FactKey, KeptRead as _};
use sha2::{Digest as _, Sha256};

const CORE: &str = "slice_memo_core";
const MATCH: &str = "slice_memo_match";
const SPAN: &str = "slice_memo_span";

/// One document's wave rows without its key: matches (clause, node) and spans (node, start, end).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Rows {
    pub matches: Vec<(i64, i64)>,
    pub spans: Vec<(i64, i64, i64)>,
}

fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.slice-memo-space/v1").into()
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.slice-memo/v1\0{rel}").as_bytes()).into()
}

/// The memo's plan for `path` under the rules `digest`.
fn plan(rules: &[u8; 32], path: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.slice-wave/v1\0");
    hash.update(rules);
    hash.update(label_of_path(path).unwrap_or("").as_bytes());
    hash.finalize().into()
}

/// The partition of `content`'s entry under `plan`.
fn part(content: &[u8; 32], plan: &[u8; 32]) -> i64 {
    let d = Sha256::new().chain_update(content).chain_update(plan).finalize();
    i64::from_le_bytes(d[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// The SHA-256 key of the blob's source the document's path binds, if stored.
fn content(store: &ContentStore, scope: &str, document: &Document) -> StoreResult<Option<[u8; 32]>> {
    let Some(blob) = store.binding(&path_name(scope, &document.path))? else { return Ok(None) };
    let Some(source) = blob_source_id(store, blob)? else { return Ok(None) };
    store.source_sha256(source).map(Some)
}

/// The memo's kept entries, opened once for a run's lookups (`lookup`).
pub(super) fn open(store: &ContentStore) -> StoreResult<Option<KeptSpace>> {
    store.kept_space(&space())
}

/// The memoised rows of `document` under the rules `digest` in `kept` (`open`), when its blob was evaluated with
/// the same core facts.
pub(super) fn lookup(store: &ContentStore, kept: Option<&KeptSpace>, scope: &str, rules: &[u8; 32], document: &Document) -> StoreResult<Option<Rows>> {
    let Some(kept) = kept else { return Ok(None) };
    let Some(content) = content(store, scope, document)? else { return Ok(None) };
    let plan = plan(rules, &document.path);
    let part = part(&content, &plan);
    let chain = kept.view();
    let get = |rel: &str| chain.get(&FactKey { rel, ident: &ident(rel), part });
    let Some(core) = get(CORE).filter(|e| e.rows.len() == 1) else { return Ok(None) };
    let core = core.row(0);
    let mut digest = [0u8; 32];
    for (i, w) in core.iter().take(4).enumerate() {
        digest[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    // A wave that reads its rows uses the plan as much as one that writes them.
    hold(store, plan)?;
    if digest != document.digest {
        return Ok(None);
    }
    let mut rows = Rows::default();
    if let Some(e) = get(MATCH) {
        rows.matches = (0..e.rows.len()).map(|i| e.row(i)).map(|r| (r[0], r[1])).collect();
    }
    if let Some(e) = get(SPAN) {
        rows.spans = (0..e.rows.len()).map(|i| e.row(i)).map(|r| (r[0], r[1], r[2])).collect();
    }
    Ok(Some(rows))
}

/// Memoises the rows the wave left in `match` and `span` for each of `documents`, behind the run.
pub(super) fn remember(lease: &Lease<'_>, store: &ContentStore, scope: &str, rules: &[u8; 32], documents: &[Document]) -> StoreResult<()> {
    let mut matches = lease.prepare("SELECT c0, c2 FROM matched WHERE c1 = ?1")?;
    let mut spans = lease.prepare("SELECT c1, c2, c3 FROM span WHERE c0 = ?1")?;
    let mut puts = Vec::new();
    for document in documents {
        let Some(content) = content(store, scope, document)? else { continue };
        let key = i64::from(document.key);
        let plan = plan(rules, &document.path);
        hold(store, plan)?;
        let part = part(&content, &plan);
        let mut core = Vec::with_capacity(4);
        core.extend(document.digest.as_chunks::<8>().0.iter().map(|w| i64::from_le_bytes(*w)));
        let mut m = Vec::new();
        let mut rows = matches.query([key])?;
        while let Some(row) = rows.next()? {
            m.extend([row.get::<_, i64>(0)?, row.get(1)?]);
        }
        let mut s = Vec::new();
        let mut rows = spans.query([key])?;
        while let Some(row) = rows.next()? {
            s.extend([row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?]);
        }
        puts.push(put(CORE, part, b"iiii", core));
        puts.push(put(MATCH, part, b"ii", m));
        puts.push(put(SPAN, part, b"iii", s));
        puts.push(holder_put(part, content, Some(plan)));
    }
    store.queue_kept(space(), puts);
    Ok(())
}

/// `rel`'s rows of partition `part` (`flat`, `types.len()` a row), or, with none, the partition's rows gone.
fn put(rel: &str, part: i64, types: &[u8], flat: Vec<i64>) -> KeptPut {
    let width = types.len();
    let what = if flat.is_empty() {
        KeptWhat::Gone
    } else {
        let n = flat.len() / width;
        KeptWhat::Flat { widths: vec![8; width], rows: flat, n, levels: None, given: true }
    };
    let digest = match &what {
        KeptWhat::Flat { rows, .. } => {
            let mut h = Sha256::new();
            for v in rows {
                h.update(v.to_le_bytes());
            }
            u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"))
        }
        _ => 0,
    };
    KeptPut { rel: rel.to_owned(), ident: ident(rel), part, digest, reads: Vec::new(), types: types.to_vec(), what }
}

/// How long a plan stays live after the last wave that wrote under it (`hold`).
const HOLD_SECS: i64 = 86_400;

/// Holds `plan` live for `HOLD_SECS` (`ContentStore::hold_corpus_plan`), renewed at most hourly per process. The
/// rows of a plan no wave has written under for that long (rules since changed) are collected by the store's
/// corpus pass (decision 0053), as a corpus run's dead plans are; their sources may live on in later snapshots.
fn hold(store: &ContentStore, plan: [u8; 32]) -> StoreResult<()> {
    #[derive(Default)]
    struct HeldPlans(std::sync::Mutex<Option<std::collections::HashMap<[u8; 32], i64>>>);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let held_ = crate::code_rule_dsl::logic::profile::shared::<HeldPlans>();
    let mut held = held_.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
