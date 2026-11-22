//! The slice's last run per scope as kept facts of the content store (t33 step 3, decisions 0104 and 0456), where
//! four SQLite tables (`slice_rules`, `slice_document`, `slice_match`, `slice_span`) held it. One space a scope:
//!
//! - `slice_rules(d0, d1, d2, d3)`, one partition: the clauses' identity (`Rules::digest`), the run's rows are of.
//! - `slice_document(path, d, c0, c1, c2, c3)`, a partition per document `d`: its path and core facts' digest.
//! - `slice_match(c, d, n)` and `slice_span(d, n, s, e)`, a partition per document: the wave's rows of it.
//!
//! A run writes only the partitions of the documents it evaluated, took from the memo, or lost (`fresh`); the
//! others' are as kept. The wave reads the kept rows where they lie, through the `kept_facts` table (`views`): no
//! table holds them again.

use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::waves::matches::{KEPT_MATCH, KEPT_SPAN};
use crate::code_rule_dsl::waves::owner::Lease;
use crate::content_store::kept_facts::{KeptPut, KeptWhat};
use crate::content_store::{ContentStore, StoreResult};
use rerail_blocks::kept::{FactKey, KeptRead as _};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const RULES: &str = "slice_rules";
const DOCUMENT: &str = "slice_document";
const MATCH: &str = "slice_match";
const SPAN: &str = "slice_span";

/// The memo's rows of this run's reused documents, beside the kept ones (`slice::memo::keep` writes them).
pub(super) const REUSED_MATCH: &str = "temp.slice_reused_match";
pub(super) const REUSED_SPAN: &str = "temp.slice_reused_span";

/// The scope's space.
pub(super) fn space(scope: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.slice-space/v1\0{scope}").as_bytes()).into()
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.slice/v1\0{rel}").as_bytes()).into()
}

fn words(digest: &[u8; 32]) -> [i64; 4] {
    std::array::from_fn(|i| i64::from_le_bytes(digest[i * 8..i * 8 + 8].try_into().expect("8 bytes")))
}

fn digest_of_words(w: &[i64]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, v) in w.iter().take(4).enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn flat_digest(flat: &[i64]) -> u64 {
    let mut h = Sha256::new();
    for v in flat {
        h.update(v.to_le_bytes());
    }
    u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"))
}

/// What the scope's last run kept: its rules' identity, and each document's key and core digest by path.
pub(super) struct Last {
    pub rules: Option<[u8; 32]>,
    pub documents: BTreeMap<String, (i64, [u8; 32])>,
}

/// The scope's last run, read once every write of the space this process queued is written.
pub(super) fn last(store: &ContentStore, space: &[u8; 32]) -> StoreResult<Last> {
    let mut last = Last { rules: None, documents: BTreeMap::new() };
    let Some(kept) = store.kept_space(space)? else { return Ok(last) };
    let chain = kept.view();
    if let Some(rules) = chain.get(&FactKey { rel: RULES, ident: &ident(RULES), part: 0 })
        && rules.rows.len() == 1
    {
        last.rules = Some(digest_of_words(&rules.row(0)));
    }
    for entry in chain.of(DOCUMENT) {
        for i in 0..entry.rows.len() {
            let row = entry.row(i);
            let Some(path) = u32::try_from(row[0]).ok().and_then(|a| store.atom_value(a).ok().flatten()) else { continue };
            last.documents.insert(path, (row[1], digest_of_words(&row[2..6])));
        }
    }
    Ok(last)
}

/// The wave's kept inputs (`KEPT_MATCH`, `KEPT_SPAN`): views of the space's rows but those of the documents `fresh`
/// (in `temp.slice_fresh`), with the memo's rows of this run's reused documents (`REUSED_*`). None of them when
/// `cold`.
pub(super) fn views(lease: &Lease<'_>, space: &[u8; 32], fresh: &[i64], cold: bool) -> StoreResult<()> {
    let hex: String = space.iter().map(|b| format!("{b:02x}")).collect();
    let kept = if cold { "0" } else { "part NOT IN (SELECT d FROM temp.slice_fresh)" };
    lease.execute_batch(&format!(
        "DROP VIEW IF EXISTS {KEPT_MATCH}; DROP TABLE IF EXISTS {KEPT_MATCH};
         DROP VIEW IF EXISTS {KEPT_SPAN}; DROP TABLE IF EXISTS {KEPT_SPAN};
         DROP TABLE IF EXISTS {REUSED_MATCH}; CREATE TEMP TABLE {}(c0 INTEGER, c1 INTEGER, c2 INTEGER);
         DROP TABLE IF EXISTS {REUSED_SPAN}; CREATE TEMP TABLE {}(c0 INTEGER, c1 INTEGER, c2 INTEGER, c3 INTEGER);
         DROP TABLE IF EXISTS temp.slice_fresh; CREATE TEMP TABLE slice_fresh(d INTEGER PRIMARY KEY);
         CREATE TEMP VIEW {} AS
           SELECT c0, c1, c2 FROM kept_facts WHERE space = X'{hex}' AND rel = '{MATCH}' AND {kept}
           UNION ALL SELECT c0, c1, c2 FROM {REUSED_MATCH};
         CREATE TEMP VIEW {} AS
           SELECT c0, c1, c2, c3 FROM kept_facts WHERE space = X'{hex}' AND rel = '{SPAN}' AND {kept}
           UNION ALL SELECT c0, c1, c2, c3 FROM {REUSED_SPAN};",
        unqualified(REUSED_MATCH),
        unqualified(REUSED_SPAN),
        unqualified(KEPT_MATCH),
        unqualified(KEPT_SPAN),
    ))?;
    let mut insert = lease.prepare("INSERT OR IGNORE INTO temp.slice_fresh(d) VALUES (?1)")?;
    for d in fresh {
        insert.execute([d])?;
    }
    Ok(())
}

/// The run of `scope` kept for its next: the rules' identity, the documents whose entry changed, and the wave's rows
/// (`matched`, `span`) of the documents `fresh`; a document gone, or one kept before a `cold` run, no longer kept.
pub(super) fn remember(lease: &Lease<'_>, store: &ContentStore, space: [u8; 32], rules: &[u8; 32], documents: &[Document], last: &Last, fresh: &[i64], cold: bool) -> StoreResult<()> {
    let mut puts = Vec::new();
    if last.rules != Some(*rules) {
        let flat = words(rules).to_vec();
        puts.push(put(RULES, 0, b"iiii", flat, 1, true));
    }
    // The documents, a partition each: written where its entry changed, gone where it went.
    let current: BTreeSet<i64> = documents.iter().map(|d| i64::from(d.key)).collect();
    let paths: Vec<&str> = documents.iter().map(|d| d.path.as_str()).collect();
    let syms = store.intern_atoms(&paths)?;
    for (d, sym) in documents.iter().zip(syms) {
        if !cold && last.documents.get(&d.path) == Some(&(i64::from(d.key), d.digest)) {
            continue;
        }
        let mut flat = vec![i64::from(sym), i64::from(d.key)];
        flat.extend(words(&d.digest));
        puts.push(put(DOCUMENT, i64::from(d.key), b"siiiii", flat, 1, true));
    }
    let gone: BTreeSet<i64> = last.documents.values().map(|(d, _)| *d).filter(|d| !current.contains(d)).collect();
    for d in &gone {
        puts.push(gone_put(DOCUMENT, *d, b"siiiii"));
    }
    // The wave's rows of the fresh documents (and, before a cold run, of every document kept): one partition each.
    let mut parts: BTreeSet<i64> = fresh.iter().copied().collect();
    if cold {
        parts.extend(last.documents.values().map(|(d, _)| *d));
    }
    for (rel, table, doc, width, types) in [(MATCH, "matched", "c1", 3usize, &b"iii"[..]), (SPAN, "span", "c0", 4, &b"iiii"[..])] {
        let columns = (0..width).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
        let mut by: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        let mut st = lease.prepare(&format!("SELECT {columns} FROM {table} WHERE {doc} IN (SELECT d FROM temp.slice_fresh)"))?;
        let doc_at = if doc == "c1" { 1 } else { 0 };
        let mut rows = st.query([])?;
        while let Some(row) = rows.next()? {
            let values: Vec<i64> = (0..width).map(|i| row.get(i)).collect::<rusqlite::Result<_>>()?;
            by.entry(values[doc_at]).or_default().extend(values);
        }
        for d in &parts {
            match by.remove(d) {
                Some(flat) => {
                    let n = flat.len() / width;
                    puts.push(put(rel, *d, types, flat, n, false));
                }
                None => puts.push(gone_put(rel, *d, types)),
            }
        }
    }
    store.queue_kept(space, puts);
    Ok(())
}

fn put(rel: &str, part: i64, types: &[u8], flat: Vec<i64>, n: usize, given: bool) -> KeptPut {
    KeptPut {
        rel: rel.to_owned(),
        ident: ident(rel),
        part,
        digest: flat_digest(&flat),
        reads: Vec::new(),
        types: types.to_vec(),
        what: KeptWhat::Flat { widths: vec![8; types.len()], rows: flat, n, levels: None, given },
    }
}

fn gone_put(rel: &str, part: i64, types: &[u8]) -> KeptPut {
    KeptPut { rel: rel.to_owned(), ident: ident(rel), part, digest: 0, reads: Vec::new(), types: types.to_vec(), what: KeptWhat::Gone }
}

/// A temp table's name without its schema.
fn unqualified(table: &str) -> &str {
    table.strip_prefix("temp.").unwrap_or(table)
}

/// The kept spans of document `d` in `scope` starting at `start` moved one byte earlier, as a put of the document's
/// partition: how many moved (a test's mutation of the kept rows).
#[cfg(test)]
pub(super) fn shift_spans(store: &ContentStore, scope: &str, d: i64, start: i64) -> usize {
    let space = space(scope);
    let Some(kept) = store.kept_space(&space).unwrap() else { return 0 };
    let chain = kept.view();
    let Some(entry) = chain.get(&FactKey { rel: SPAN, ident: &ident(SPAN), part: d }) else { return 0 };
    let mut moved = 0;
    let mut flat = Vec::new();
    for i in 0..entry.rows.len() {
        let mut row = entry.row(i);
        if row[2] == start {
            row[2] -= 1;
            moved += 1;
        }
        flat.extend(row);
    }
    let n = entry.rows.len();
    drop(chain);
    if moved > 0 {
        store.queue_kept(space, vec![put(SPAN, d, b"iiii", flat, n, false)]);
        store.settle_kept(&space);
    }
    moved
}

/// The rows of `rel` kept for `scope` (`slice_match` or `slice_span`), through `kept_facts`.
#[cfg(test)]
pub(super) fn count(store: &ContentStore, scope: &str, rel: &str) -> i64 {
    store.settle_kept(&space(scope));
    store.query_rows("SELECT count(*) FROM kept_facts WHERE space = ?1 AND rel = ?2", rusqlite::params![&space(scope)[..], rel], |row| row.get::<_, i64>(0)).unwrap()[0]
}
