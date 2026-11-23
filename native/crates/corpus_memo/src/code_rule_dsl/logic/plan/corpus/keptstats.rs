//! The exports' statistics, kept by document across a memoised program's runs (decision 0096 §3). (A base relation's
//! are kept beside its documents' table and patched from the changed documents, decision 0458: no entry holds its
//! rows.)
//!
//! A memoised run sees every file's rows of the exports the file-local part hands it: a memo hit's from its entry, an
//! evaluated file's from the entry it is remembered under. Each is kept under its entry's key (`r`). Rather than
//! measure their concatenation (`syntax::measured`'s sort per column, over every file), the keeper counts a file in when its content is new
//! to it, and the content it had counted out, read back from the memo by that content. A file no longer in the
//! run is counted out the same way. An edit costs its files' rows.
//!
//! A run that cannot say what it counted (a file without a memo content, a chunk whose rows were not attributed to
//! its files, an old entry the memo dropped) gives the keeper up: that run measures as before, and the next one
//! counts every file in again (the backfill).

use std::collections::{BTreeMap, HashMap, HashSet};

use super::memo;
use sha2::Digest as _;
use crate::code_rule_dsl::logic::plan::statkeep::RelStats;

/// `RERAIL_KEPT_DOCS=<export>,…` (a debugging aid, Q20): the rows of those exports a run counted in and out, by the
/// document their location column names. The run's own, in its keeper: read from the setting as the run begins.
#[derive(Default)]
struct Trace {
    /// The exports traced.
    listed: Vec<String>,
    /// Per export traced: the rows counted, signed, by document.
    docs: BTreeMap<String, HashMap<i64, i64>>,
    /// Each counted row of those exports, with the file whose entry gave it and its sign.
    rows: Vec<(String, String, i64, Vec<i64>)>,
    /// The file whose entry is being counted.
    counting: String,
}

impl Trace {
    fn from_env() -> Trace {
        let listed = std::env::var("RERAIL_KEPT_DOCS").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_default();
        Trace { listed, ..Default::default() }
    }
}

/// One memoised program's statistics, and which content of each file they count.
#[derive(Default)]
pub struct Keeper {
    /// Per file: the content counted and its document key.
    docs: HashMap<String, (memo::Content, i64)>,
    rels: BTreeMap<String, RelStats>,
    /// Per export (by key): the column that names its rows' document, its first node column (`None`: no such
    /// column). A base relation's rows name theirs in their first column. Set by `begin`, not kept in the blob.
    doc_at: HashMap<String, Option<usize>>,
    /// This run's: the files seen, and whether it could count them all.
    seen: HashSet<String>,
    failed: Option<String>,
    /// Files counted in and out this run.
    pub counted: (usize, usize),
    /// Per export (by key): each value document its rows name in their location column (`doc_at`), and how many
    /// counted files' entries hold its rows (decision 0117 step 2). A value document's rows go to every file that
    /// asked for it; any other owner's to one file. Kept in the blob.
    holders: BTreeMap<String, rustc_hash::FxHashMap<i64, i64>>,
    /// Per relation (by key): the identity its statistics were counted under (decision 0225 §2), for the relations
    /// that have one (a base relation's rows are the files' alone). Kept in the blob.
    ids: BTreeMap<String, [u8; 32]>,
    /// This run's relations whose identity changed since they were counted (a rule's edit reached them): counted from
    /// scratch, from every file's entry, while the others stay as counted.
    fresh: HashSet<String>,
    /// This run's `RERAIL_KEPT_DOCS` trace (`Trace`): not kept in the blob.
    trace: Trace,
}

impl Keeper {
    /// For a run reading `rels`, by entry key: the exports, each with its columns `c0…`, and `exports`, each export's column that names its rows' document (`Keeper::doc_at`). A relation counted at
    /// other columns before is counted from scratch.
    /// `ids` gives each relation's identity, where it has one: a relation counted under another identity, or new to
    /// the keeper, is counted again from scratch (`fresh`), from every file's entry, and the others stay as counted.
    pub fn begin(&mut self, rels: &[(String, Vec<String>)], ids: &BTreeMap<String, [u8; 32]>, exports: HashMap<String, Option<usize>>) {
        self.doc_at = exports;
        self.seen.clear();
        self.trace = Trace::from_env();
        self.failed = None;
        self.counted = (0, 0);
        self.fresh.clear();
        let changed = |k: &Keeper, rel: &str, columns: &[String]| k.rels.get(rel).is_none_or(|s| s.columns() != columns) || k.ids.get(rel) != ids.get(rel);
        if !self.docs.is_empty() && rels.iter().any(|(rel, columns)| changed(self, rel, columns)) {
            // Every relation counted again: the files are counted from scratch.
            if rels.iter().all(|(rel, columns)| changed(self, rel, columns)) {
                self.docs.clear();
                self.rels.clear();
                self.holders.clear();
            } else {
                let now: HashSet<&str> = rels.iter().map(|(r, _)| r.as_str()).collect();
                self.rels.retain(|r, _| now.contains(r.as_str()));
                self.holders.retain(|r, _| now.contains(r.as_str()));
                for (rel, columns) in rels {
                    if changed(self, rel, columns) {
                        self.rels.insert(rel.clone(), RelStats::new(columns.clone()));
                        self.holders.remove(rel);
                        self.fresh.insert(rel.clone());
                    }
                }
            }
        }
        self.ids = rels.iter().filter_map(|(r, _)| ids.get(r).map(|id| (r.clone(), *id))).collect();
        for (rel, columns) in rels {
            if self.rels.get(rel).is_some_and(|s| s.columns() != columns.as_slice()) {
                self.rels.remove(rel);
            }
            self.rels.entry(rel.clone()).or_insert_with(|| RelStats::new(columns.clone()));
        }
    }

    /// Every file counted, with the content and key it was counted under.
    pub fn counted_files(&self) -> Vec<(String, memo::Content, i64)> {
        self.docs.iter().map(|(p, (c, k))| (p.clone(), *c, *k)).collect()
    }

    /// Whether this run counts some relations again from scratch (`begin`'s `fresh`): a file's entry must hold
    /// their rows, so none is read for its outputs alone.
    pub fn has_fresh(&self) -> bool {
        !self.fresh.is_empty()
    }

    /// Whether this run has given the keeper up.
    pub fn failed(&self) -> bool {
        self.failed.is_some()
    }

    pub fn fail(&mut self, why: impl Into<String>) {
        self.failed.get_or_insert_with(|| why.into());
    }

    /// File `path`'s rows in this run (`entry`, under `content`, its document `key`): counted in if its content is new to the keeper,
    /// the content it counted before counted out first (read back by `old`).
    pub fn file(&mut self, path: &str, content: Option<memo::Content>, key: i64, entry: &memo::Entry, old: &dyn Fn(memo::Content, i64) -> Option<memo::Entry>) {
        if self.failed.is_some() {
            return;
        }
        let Some(content) = content else {
            return self.fail(format!("{path}: no memo content"));
        };
        self.seen.insert(path.to_owned());
        if !self.trace.listed.is_empty() {
            self.trace.counting = path.to_owned();
        }
        let now = (content, key);
        match self.docs.get(path).copied() {
            Some(was) if same(was, now) => {
                // Counted already, but for the relations counted from scratch.
                if !self.fresh.is_empty() {
                    let fresh = std::mem::take(&mut self.fresh);
                    self.count_where(entry, 1, &|r| fresh.contains(r));
                    self.fresh = fresh;
                }
                return;
            }
            Some((c, k)) => {
                if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                    eprintln!("statkeep {path}: counted under {:?}, now {:?}", (c, k), now);
                }
                match old(c, k) {
                Some(before) => {
                    // A relation whose rows the edit left as they were (node ids by definition keep an unchanged
                    // definition's) is counted out and in again to no effect: left as counted. Each relation's
                    // statistics are its own rows', so this is exact; within a relation the rows are counted out and
                    // in whole, as its per-document statistics need.
                    let (before, now_rows) = changed_only(&before, entry);
                    if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                        // What a row-level count would leave out: the rows the old and new share at either end.
                        for (rel, (w, new)) in &now_rows.rows {
                            let Some((_, old)) = before.rows.get(rel) else { continue };
                            let (a, b): (Vec<&[i64]>, Vec<&[i64]>) = (old.chunks_exact((*w).max(1)).collect(), new.chunks_exact((*w).max(1)).collect());
                            let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
                            let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
                            let set: std::collections::HashSet<&[i64]> = a.iter().copied().collect();
                            let kept = b.iter().filter(|r| set.contains(*r)).count();
                            eprintln!("statkeep diff {path} {rel}: old {} new {} rows, same at the ends {head}+{tail}, new rows also old {kept}", a.len(), b.len());
                            // The rows that changed, paired by their first column: which of the others differ.
                            let key = 0;
                            let now: std::collections::HashSet<&[i64]> = b.iter().copied().collect();
                            let gone: std::collections::HashMap<i64, &[i64]> = a.iter().filter(|r| !now.contains(*r)).filter_map(|r| r.get(key).map(|k| (*k, *r))).collect();
                            let fresh: Vec<&[i64]> = b.iter().copied().filter(|r| !set.contains(r)).collect();
                            let mut by_col = vec![0usize; *w];
                            let mut paired = 0;
                            for r in &fresh {
                                let Some(o) = r.get(key).and_then(|k| gone.get(k)) else { continue };
                                paired += 1;
                                for c in (0..*w).filter(|&c| c != key) {
                                    by_col[c] += usize::from(o[c] != r[c]);
                                }
                            }
                            let names: Vec<String> = (0..*w).map(|c| format!("c{c}")).collect();
                            let differ: Vec<String> = (0..*w).filter(|&c| by_col[c] > 0).map(|c| format!("{}={}", names.get(c).map_or("?", |n| n.as_str()), by_col[c])).collect();
                            eprintln!("statkeep churn {rel}: gone {} new {} paired {paired}; columns differing: {}", a.len() - (b.len() - fresh.len()).min(a.len()), fresh.len(), differ.join(" "));
                        }
                    }
                    crate::code_rule_dsl::logic::profile::decision("statistics kept by file (decision 0096 §3)", || {
                        format!("an edited file's unchanged relations not counted again: {} of {} changed", now_rows.rows.len(), entry.rows.len())
                    });
                    // The relations counted from scratch were never counted with its old rows: in from its entry whole.
                    let fresh = std::mem::take(&mut self.fresh);
                    self.count_where(&before, -1, &|r| !fresh.contains(r));
                    self.count_where(&now_rows, 1, &|r| !fresh.contains(r));
                    if !fresh.is_empty() {
                        self.count_where(entry, 1, &|r| fresh.contains(r));
                    }
                    self.fresh = fresh;
                    self.docs.insert(path.to_owned(), now);
                    return;
                }
                None => return self.fail(format!("{path}: the memo no longer holds the rows counted")),
                }
            }
            None => {}
        }
        self.count(entry, 1);
        self.docs.insert(path.to_owned(), now);
    }

    fn count(&mut self, entry: &memo::Entry, sign: i64) {
        self.count_where(entry, sign, &|_| true);
    }

    /// `count`, of the relations `pick` takes.
    fn count_where(&mut self, entry: &memo::Entry, sign: i64, pick: &dyn Fn(&str) -> bool) {
        let values = crate::code_rule_dsl::facts::vtab::syntax::HIDDEN_KEYS as i64;
        let holders = crate::code_rule_dsl::logic::profile::span(|| "statistics: holders".into());
        for (key, at) in &self.doc_at {
            if !pick(key) {
                continue;
            }
            let (Some(at), Some((width, rows))) = (at, entry.rows.get(key)) else { continue };
            if at >= width {
                continue;
            }
            let owners: rustc_hash::FxHashSet<i64> = rows.chunks_exact(*width).map(|r| memo::document_of(r[*at])).filter(|d| *d >= values).collect();
            if owners.is_empty() {
                continue;
            }
            let held = self.holders.entry(key.clone()).or_default();
            for o in owners {
                let n = held.entry(o).or_default();
                *n += sign;
                if *n == 0 {
                    held.remove(&o);
                }
            }
        }
        drop(holders);
        let _rels = crate::code_rule_dsl::logic::profile::span(|| "statistics: relations".into());
        for (key, stats) in &mut self.rels {
            if !pick(key) {
                continue;
            }
            if let Some((width, values)) = entry.rows.get(key) {
                match self.doc_at.get(key) {
                    Some(at) => {
                        if self.trace.listed.iter().any(|k| k == key) {
                            let Trace { docs, rows, counting, .. } = &mut self.trace;
                            let by = docs.entry(key.clone()).or_default();
                            for row in values.chunks_exact((*width).max(1)) {
                                *by.entry(at.map_or(-1, |c| row[c] >> 32)).or_default() += sign;
                                rows.push((key.clone(), counting.clone(), sign, row.to_vec()));
                            }
                        }
                        stats.rows_by(values, *width, sign, *at)
                    }
                    None => stats.documents(values, *width, sign),
                }
            }
        }
        if sign > 0 { self.counted.0 += 1 } else { self.counted.1 += 1 }
    }

    /// The run's end: files not seen are counted out. The statistics by entry key, unless the run could not keep them
    /// (then the reason, and the keeper starts again empty).
    pub fn end(&mut self, old: &dyn Fn(&str, memo::Content, i64) -> Option<memo::Entry>) -> Result<BTreeMap<String, RelStats>, String> {
        if self.failed.is_none() {
            let gone: Vec<String> = self.docs.keys().filter(|p| !self.seen.contains(*p)).cloned().collect();
            for path in gone {
                let (c, k) = self.docs.remove(&path).expect("listed");
                match old(&path, c, k) {
                    Some(before) => {
                        let fresh = std::mem::take(&mut self.fresh);
                        self.count_where(&before, -1, &|r| !fresh.contains(r));
                        self.fresh = fresh;
                    }
                    None => {
                        self.fail(format!("{path}: gone, and the memo no longer holds the rows counted"));
                        break;
                    }
                }
            }
        }
        if let Some(why) = self.failed.take() {
            self.docs.clear();
            self.rels.clear();
            self.holders.clear();
            return Err(why);
        }
        self.fresh.clear();
        Ok(self.rels.clone())
    }
}

impl Keeper {
    /// The keeper as kept facts of the store's corpus statistics' space (t33 step 6; decision 0096 §2 kept it as one
    /// blob): partition `plan_part(plan)` of its fixed relations, every one put, so a write is the partition whole:
    ///
    /// - `keeper_build(format, plan0..3, build0..3)`: what it was counted under. The engine build that counted them
    ///   (Q102): its memo entries are of the build (decision 0077 §4), so another build's statistics name rows its memo
    ///   does not hold, and its next run counts every file in.
    /// - `keeper_name(r, at, word)`: the relation names, each as its bytes (`at` -1 their length).
    /// - `keeper_doc(d, kind, c0..c3, key)` and `keeper_path(d, at, word)`: each file counted, its content (`kind` 0 a
    ///   stored source's id, 1 a digest the store did not hold yet: the next run finds its source, `resolve`) and its
    ///   document key. Only stored contents are kept (a session's are its memory's).
    /// - `keeper_stats(r, at, word)`: relation `r`'s statistics as they encode (`RelStats::encode`), as the documents'
    ///   table keeps a base relation's (`retain::put_base_stats`), but for the exact counts of its symbol columns:
    /// - `keeper_exact(r, c, value: atom, count)`: those counts, their values atoms the store's writer maps to its own
    ///   ids (decision 0170), as `of_kept` maps them back to the reader's. A relation whose symbol columns `syms` does
    ///   not know keeps no exact maps (its constants are then counted from the rows held).
    /// - `keeper_watched(r, c, value: atom, count)`: the constants a symbol column watches (`RelStats::watch`), kept
    ///   and mapped as the exact counts are.
    /// - `keeper_summary(r, c, ndv, sq, sqd)`: each column as the statistics give it (`RelStats::summary`, the exact
    ///   maps in): a later run answers the planner from it and decodes a relation's statistics only to patch them.
    /// - `keeper_holder(r, owner, n)` and `keeper_id(r, i0..i3)`: the holders and identities.
    ///
    /// The partition is held by `program`, the memo program `plan` is made of (`plan_holder_put`): the store's
    /// collection drops it once that program's plan is dead.
    pub fn into_kept(mut self, plan: &[u8; 32], program: &[u8; 32], syms: &dyn Fn(&str, &[String]) -> Option<Vec<bool>>) -> crate::content_store::StoreResult<Vec<crate::content_store::kept_facts::KeptPut>> {
        let part = plan_part(plan);
        // Each symbol column's exact counts and watched values taken out of its statistics, for `keeper_exact` and
        // `keeper_watched`: their values are atoms, which the store's writer maps (decision 0170).
        let mut exact: Vec<(String, usize, Vec<(i64, i64)>)> = Vec::new();
        let mut watched: Vec<(String, usize, Vec<(i64, i64)>)> = Vec::new();
        // Each relation's summary (`RelStats::summary`), with its exact maps in, before they are taken out: a later
        // run answers the planner's statistics from it, its sketches not decoded (t33 K4).
        let summaries: Vec<(String, Vec<(i64, i64, i64)>)> = self.rels.iter().map(|(rel, stats)| (rel.clone(), stats.summary())).collect();
        for (rel, stats) in self.rels.iter_mut() {
            let width = stats.columns().len();
            match syms(rel, stats.columns()).filter(|c| c.len() == width) {
                Some(cols) => {
                    for c in (0..width).filter(|&c| cols[c]) {
                        exact.extend(stats.take_exact(c).map(|pairs| (rel.clone(), c, pairs)));
                        let pairs = stats.take_watched(c);
                        if !pairs.is_empty() {
                            watched.push((rel.clone(), c, pairs));
                        }
                    }
                }
                None => (0..width).for_each(|c| {
                    drop(stats.take_exact(c));
                    drop(stats.take_watched(c));
                }),
            }
        }
        let names: Vec<&String> = self.rels.keys().chain(self.holders.keys()).chain(self.ids.keys()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let index: HashMap<&str, i64> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i as i64)).collect();
        let stats: Vec<(i64, Vec<u8>)> = self
            .rels
            .iter()
            .map(|(rel, stats)| {
                let mut out = Vec::new();
                stats.encode(&mut out);
                (index[rel.as_str()], out)
            })
            .collect();
        let docs: Vec<(&String, memo::Content, i64)> = self.docs.iter().map(|(p, (c, k))| (p, *c, *k)).collect();
        let build: [u8; 32] = sha2::Sha256::digest(crate::code_rule_dsl::build_ids::engine().as_bytes()).into();
        let put = |rel: &str, types: &[u8], n: usize, fill: &mut dyn FnMut(&mut dyn FnMut(i64))| {
            crate::content_store::kept_facts::filled_put(rel, ident(rel), part, types, n, |push| {
                fill(push);
                Ok::<(), crate::content_store::StoreError>(())
            })
        };
        let words = |d: &[u8; 32]| d.as_chunks::<8>().0.iter().map(|w| i64::from_le_bytes(*w)).collect::<Vec<_>>();
        Ok(vec![
            put(BUILD, b"iiiiiiiii", 1, &mut |push| std::iter::once(FORMAT).chain(words(plan)).chain(words(&build)).for_each(&mut *push))?,
            put(NAME, b"iii", names.iter().map(|n| bytes_rows(n.len())).sum(), &mut |push| {
                for (r, n) in names.iter().enumerate() {
                    push_bytes(r as i64, n.as_bytes(), push);
                }
            })?,
            put(DOC, b"iiiiiii", docs.len(), &mut |push| {
                for (d, (_, c, k)) in docs.iter().enumerate() {
                    let (kind, w) = match c {
                        memo::Content::Stored(id) => (0, [id.raw(), 0, 0, 0]),
                        memo::Content::Unstored(digest) => (1, std::array::from_fn(|i| i64::from_le_bytes(digest[i * 8..i * 8 + 8].try_into().expect("8 bytes")))),
                    };
                    [d as i64, kind].into_iter().chain(w).chain([*k]).for_each(&mut *push);
                }
            })?,
            put(PATH, b"iii", docs.iter().map(|(p, _, _)| bytes_rows(p.len())).sum(), &mut |push| {
                for (d, (p, _, _)) in docs.iter().enumerate() {
                    push_bytes(d as i64, p.as_bytes(), push);
                }
            })?,
            put(STATS, b"iii", stats.iter().map(|(_, b)| bytes_rows(b.len())).sum(), &mut |push| {
                for (r, b) in &stats {
                    push_bytes(*r, b, push);
                }
            })?,
            put(EXACT, b"iisi", exact.iter().map(|(_, _, p)| p.len()).sum(), &mut |push| {
                for (rel, c, pairs) in &exact {
                    for &(v, n) in pairs {
                        [index[rel.as_str()], *c as i64, v, n].into_iter().for_each(&mut *push);
                    }
                }
            })?,
            put(WATCHED, b"iisi", watched.iter().map(|(_, _, p)| p.len()).sum(), &mut |push| {
                for (rel, c, pairs) in &watched {
                    for &(v, n) in pairs {
                        [index[rel.as_str()], *c as i64, v, n].into_iter().for_each(&mut *push);
                    }
                }
            })?,
            put(SUMMARY, b"iiiii", summaries.iter().map(|(_, s)| s.len()).sum(), &mut |push| {
                for (rel, summary) in &summaries {
                    for (c, &(ndv, sq, sqd)) in summary.iter().enumerate() {
                        [index[rel.as_str()], c as i64, ndv, sq, sqd].into_iter().for_each(&mut *push);
                    }
                }
            })?,
            put(HOLDERS, b"iii", self.holders.values().map(|h| h.len()).sum(), &mut |push| {
                for (rel, held) in &self.holders {
                    for (&o, &n) in held {
                        [index[rel.as_str()], o, n].into_iter().for_each(&mut *push);
                    }
                }
            })?,
            put(IDS, b"iiiii", self.ids.len(), &mut |push| {
                for (rel, id) in &self.ids {
                    std::iter::once(index[rel.as_str()]).chain(words(id)).for_each(&mut *push);
                }
            })?,
            crate::content_store::kept_facts::plan_holder_put(part, *program),
        ])
    }

    /// The keeper `into_kept` kept under `plan` in `kept`; `None` when none is kept, one of another format, plan or
    /// engine build, or one whose atoms `to_local` refuses. `keeper_exact`'s values, the store's ids, are mapped to this
    /// process's by `to_local` (decision 0170), all in one call. The sketches hash the values as they were counted, and
    /// stay as they are: a run across numberings perturbs their estimates, never a finding.
    pub fn of_kept(kept: &crate::content_store::kept_facts::KeptSpace, plan: &[u8; 32], to_local: &dyn Fn(&mut [i64]) -> crate::content_store::StoreResult<()>) -> Option<Keeper> {
        use rerail_blocks::kept::KeptRead as _;
        let part = plan_part(plan);
        let chain = kept.view();
        // Each relation's columns, each decoded once in order (`Col::for_each`): no row made of its own.
        let cols = |rel: &str| -> Cols {
            let ident = ident(rel);
            chain.get(&rerail_blocks::kept::FactKey { rel, ident: &ident, part }).map_or_else(Cols::default, |e| {
                let cols = (0..e.rows.arity as usize)
                    .map(|c| {
                        let col = e.rows.col(c);
                        let mut v = Vec::with_capacity(col.len());
                        col.for_each(|x| v.push(x));
                        v
                    })
                    .collect();
                Cols { n: e.rows.len(), cols }
            })
        };
        let rows = |rel: &str| -> Vec<Vec<i64>> { cols(rel).rows().collect() };
        let words = |d: &[u8; 32]| d.as_chunks::<8>().0.iter().map(|w| i64::from_le_bytes(*w)).collect::<Vec<_>>();
        let build: [u8; 32] = sha2::Sha256::digest(crate::code_rule_dsl::build_ids::engine().as_bytes()).into();
        let want: Vec<i64> = std::iter::once(FORMAT).chain(words(plan)).chain(words(&build)).collect();
        if rows(BUILD).first() != Some(&want) {
            return None;
        }
        let names: HashMap<i64, String> = bytes_of(&cols(NAME))?.into_iter().map(|(r, b)| Some((r, String::from_utf8(b).ok()?))).collect::<Option<_>>()?;
        let name = |r: i64| names.get(&r).cloned();
        let mut paths = bytes_of(&cols(PATH))?;
        let mut k = Keeper::default();
        for row in rows(DOC) {
            let [d, kind, w0, w1, w2, w3, key] = row[..] else { return None };
            let content = match kind {
                0 => memo::Content::Stored(crate::content_store::SourceId::from_raw(w0)),
                1 => {
                    let mut digest = [0u8; 32];
                    for (i, w) in [w0, w1, w2, w3].into_iter().enumerate() {
                        digest[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
                    }
                    memo::Content::Unstored(digest)
                }
                _ => return None,
            };
            k.docs.insert(String::from_utf8(paths.remove(&d)?).ok()?, (content, key));
        }
        // Each relation's statistics as kept, decoded where first read, its summary answering the planner until then.
        let mut summaries: HashMap<i64, Vec<(i64, (i64, i64, i64))>> = HashMap::new();
        for row in rows(SUMMARY) {
            let [r, c, ndv, sq, sqd] = row[..] else { return None };
            summaries.entry(r).or_default().push((c, (ndv, sq, sqd)));
        }
        for (r, b) in bytes_of(&cols(STATS))? {
            let summary = summaries.remove(&r).map(|mut s| {
                s.sort_unstable_by_key(|(c, _)| *c);
                s.into_iter().map(|(_, x)| x).collect()
            });
            k.rels.insert(name(r)?, RelStats::kept(b.into(), summary)?);
        }
        // The exact counts and the watched values (`keeper_watched`), their atoms this process's.
        let by_column = |rel: &str| -> Option<BTreeMap<(i64, usize), Vec<(i64, i64)>>> {
            let got = cols(rel);
            let mut out: BTreeMap<(i64, usize), Vec<(i64, i64)>> = BTreeMap::new();
            if got.n > 0 {
                let [r, c, values, n] = &got.cols[..] else { return None };
                let mut values = values.clone();
                to_local(&mut values).ok()?;
                for i in 0..got.n {
                    out.entry((r[i], usize::try_from(c[i]).ok()?)).or_default().push((values[i], n[i]));
                }
            }
            Some(out)
        };
        for ((r, c), pairs) in by_column(EXACT)? {
            k.rels.get_mut(&name(r)?)?.put_exact(c, pairs);
        }
        for ((r, c), pairs) in by_column(WATCHED)? {
            k.rels.get_mut(&name(r)?)?.watch(c, pairs);
        }
        for row in rows(HOLDERS) {
            let [r, o, n] = row[..] else { return None };
            k.holders.entry(name(r)?).or_default().insert(o, n);
        }
        for row in rows(IDS) {
            let [r, i0, i1, i2, i3] = row[..] else { return None };
            let mut id = [0u8; 32];
            for (i, w) in [i0, i1, i2, i3].into_iter().enumerate() {
                id[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
            }
            k.ids.insert(name(r)?, id);
        }
        Some(k)
    }

    /// Export `key`'s column `c` watches the values `counts` names, each with its count in the rows counted
    /// (`RelStats::watch`): the constants the rules test there, kept with the statistics from this run on.
    pub fn watch(&mut self, key: &str, c: usize, counts: Vec<(i64, i64)>) {
        if let Some(stats) = self.rels.get_mut(key) {
            stats.watch(c, counts);
        }
    }

    /// Whether this run traces export `key` (`RERAIL_KEPT_DOCS`).
    pub fn traces(&self, key: &str) -> bool {
        self.trace.listed.iter().any(|k| k == key)
    }

    /// `RERAIL_KEPT_DOCS`: export `key`'s rows as this run counted them, by document, against `held`'s (each row
    /// `held` holds, `width` columns): the documents whose counts differ, with the file each is kept under. A run
    /// that counts every file in (a new build's first) counts all of them.
    pub fn compare_documents(&self, key: &str, held: &super::exports::Exports, width: usize) -> Vec<String> {
        let at = self.doc_at.get(key).copied().flatten();
        let mut have: HashMap<i64, i64> = HashMap::new();
        held.each_row(key, width, |row| *have.entry(at.map_or(-1, |c| row[c] >> 32)).or_default() += 1);
        let counted = self.trace.docs.get(key).cloned().unwrap_or_default();
        let mut docs: Vec<i64> = have.keys().chain(counted.keys()).copied().collect();
        docs.sort_unstable();
        docs.dedup();
        let path = |d: i64| self.docs.iter().find(|(_, (_, k))| *k == d).map_or("(no file kept under it)", |(p, _)| p.as_str());
        docs.into_iter()
            .filter_map(|d| {
                let (c, h) = (counted.get(&d).copied().unwrap_or(0), have.get(&d).copied().unwrap_or(0));
                (c != h).then(|| {
                    let mut out = format!("document {d} {}: {c} counted, {h} held", path(d));
                    for (k, p, sign, row) in &self.trace.rows {
                        if k == key && at.is_some_and(|c| row[c] >> 32 == d) {
                            out.push_str(&format!("\n  counted {sign:+} from {p}: {row:?}"));
                        }
                    }
                    held.each_row(key, width, |row| {
                        if at.is_some_and(|c| row[c] >> 32 == d) {
                            out.push_str(&format!("\n  held: {row:?}"));
                        }
                    });
                    out
                })
            })
            .collect()
    }

    /// Export `key`'s value documents and how many counted files hold each (`holders`).
    pub fn holders(&self, key: &str) -> Option<&rustc_hash::FxHashMap<i64, i64>> {
        self.holders.get(key)
    }

    /// The contents kept by digest (a run's new sources, written after its blob was made), as the store holds them
    /// now: `found` gives each digest's source, in one read before the run's first wave.
    pub fn resolve(&mut self, found: impl FnOnce(&[[u8; 32]]) -> Vec<Option<crate::content_store::SourceId>>) {
        let unstored: Vec<(String, [u8; 32])> = self.docs.iter().filter_map(|(p, (c, _))| match c { memo::Content::Unstored(d) => Some((p.clone(), *d)), _ => None }).collect();
        if unstored.is_empty() {
            return;
        }
        let ids = found(&unstored.iter().map(|(_, d)| *d).collect::<Vec<_>>());
        for ((path, _), id) in unstored.into_iter().zip(ids) {
            if let (Some(id), Some(at)) = (id, self.docs.get_mut(&path)) {
                at.0 = memo::Content::Stored(id);
            }
        }
    }
}

/// The corpus statistics' space: the keepers (`Keeper::into_kept`) and the memo spaces' last programs (`put_last`).
pub fn space() -> [u8; 32] {
    sha2::Sha256::digest(b"rerail.corpus-statistics/v1").into()
}

fn ident(rel: &str) -> [u8; 32] {
    sha2::Sha256::digest(format!("rerail.corpus-statistics/v1\0{rel}").as_bytes()).into()
}

/// The partition of what is kept under `key` (a plan, or a memo space's last-program key).
fn plan_part(key: &[u8; 32]) -> i64 {
    i64::from_le_bytes(key[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// The kept form's format, in `keeper_build`'s first column.
const FORMAT: i64 = 1;
const BUILD: &str = "keeper_build";
const NAME: &str = "keeper_name";
const DOC: &str = "keeper_doc";
const PATH: &str = "keeper_path";
const STATS: &str = "keeper_stats";
const EXACT: &str = "keeper_exact";
/// `keeper_watched(r, c, value: atom, count)`: the constants a column watches (`RelStats::watch`), mapped as
/// `keeper_exact`'s are.
const WATCHED: &str = "keeper_watched";
/// `keeper_summary(r, c, ndv, sq, sqd)`: relation `r`'s column `c` as its statistics give it (`RelStats::summary`).
const SUMMARY: &str = "keeper_summary";
const HOLDERS: &str = "keeper_holder";
const IDS: &str = "keeper_id";
const LAST: &str = "last_program";

/// The rows `push_bytes` gives `n` bytes: their length's, and one a word.
fn bytes_rows(n: usize) -> usize {
    1 + n.div_ceil(8)
}

/// `bytes` as rows `(key, at, word)` of `push`: `at` -1 their length, then each word, eight bytes, little-endian, the
/// last padded with zeros.
fn push_bytes(key: i64, bytes: &[u8], push: &mut dyn FnMut(i64)) {
    [key, -1, bytes.len() as i64].into_iter().for_each(&mut *push);
    for (at, c) in bytes.chunks(8).enumerate() {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        [key, at as i64, i64::from_le_bytes(w)].into_iter().for_each(&mut *push);
    }
}

/// A kept relation's columns, each decoded whole (`Keeper::of_kept`).
#[derive(Default)]
struct Cols {
    n: usize,
    cols: Vec<Vec<i64>>,
}

impl Cols {
    fn rows(&self) -> impl Iterator<Item = Vec<i64>> + '_ {
        (0..self.n).map(|i| self.cols.iter().map(|c| c[i]).collect())
    }
}

/// `push_bytes`' rows back, by key; `None` if a key's are not whole. The rows are as `push_bytes` gave them (a kept
/// put keeps them as given): each key's length, then its words in order, so each key's bytes are one pass.
fn bytes_of(rows: &Cols) -> Option<HashMap<i64, Vec<u8>>> {
    let mut out: HashMap<i64, Vec<u8>> = HashMap::new();
    if rows.n == 0 {
        return Some(out);
    }
    let [keys, ats, words] = &rows.cols[..] else { return None };
    let mut i = 0;
    while i < rows.n {
        let (key, n) = (keys[i], usize::try_from(words[i]).ok()?);
        (ats[i] == -1).then_some(())?;
        let nwords = n.div_ceil(8);
        let end = i + 1 + nwords;
        (end <= rows.n).then_some(())?;
        let mut bytes = Vec::with_capacity(nwords * 8);
        for (at, j) in (i + 1..end).enumerate() {
            (keys[j] == key && ats[j] == at as i64).then_some(())?;
            bytes.extend_from_slice(&words[j].to_le_bytes());
        }
        bytes.truncate(n);
        out.insert(key, bytes).is_none().then_some(())?;
        i = end;
    }
    Some(out)
}

/// Keeps `whole` as the last program of the memo space whose key is `key` (`memo::last_program`), `under_way` when a
/// run of it is under way that started from what an ended run of it kept; `[0; 32]`: none. Behind the run.
pub fn put_last(store: &crate::content_store::ContentStore, key: &[u8; 32], whole: [u8; 32], under_way: bool) {
    let words = |d: &[u8; 32]| d.as_chunks::<8>().0.iter().map(|w| i64::from_le_bytes(*w)).collect::<Vec<_>>();
    let put = crate::content_store::kept_facts::filled_put(LAST, ident(LAST), plan_part(key), b"iiiiiiiii", 1, |push| {
        words(key).into_iter().chain(words(&whole)).chain([i64::from(under_way)]).for_each(push);
        Ok::<(), crate::content_store::StoreError>(())
    })
    .expect("one whole row");
    store.queue_kept(space(), vec![put]);
}

/// What `put_last` kept under `key`: the program and whether a run of it is under way.
pub fn last(store: &crate::content_store::ContentStore, key: &[u8; 32]) -> Option<([u8; 32], bool)> {
    use rerail_blocks::kept::KeptRead as _;
    let kept = store.kept_space(&space()).ok()??;
    let chain = kept.view();
    let ident = ident(LAST);
    let e = chain.get(&rerail_blocks::kept::FactKey { rel: LAST, ident: &ident, part: plan_part(key) })?;
    let row = (e.rows.len() == 1).then(|| e.row(0))?;
    let words = |w: &[i64]| -> [u8; 32] {
        let mut d = [0u8; 32];
        for (i, w) in w.iter().enumerate() {
            d[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        d
    };
    (words(&row[..4]) == *key).then(|| (words(&row[4..8]), row[8] != 0))
}

/// The keeper's test that a file's rows are counted already: the same content under the same document key.
pub fn same(a: (memo::Content, i64), b: (memo::Content, i64)) -> bool {
    a.1 == b.1
        && match (a.0, b.0) {
            (memo::Content::Stored(x), memo::Content::Stored(y)) => x == y,
            (memo::Content::Unstored(x), memo::Content::Unstored(y)) => x == y,
            _ => false,
        }
}

/// `before` and `now` without the relations whose rows are the same in both: what an edit changed, by relation.
fn changed_only(before: &memo::Entry, now: &memo::Entry) -> (memo::Entry, memo::Entry) {
    let same = |rel: &String| before.rows.get(rel) == now.rows.get(rel);
    let keep = |e: &memo::Entry| memo::Entry { rows: e.rows.iter().filter(|(r, _)| !same(r)).map(|(r, v)| (r.clone(), v.clone())).collect(), key: e.key, nodes: e.nodes, outputs: Default::default() };
    (keep(before), keep(now))
}

#[cfg(test)]
pub mod tests {
    use super::{Keeper, memo};

    /// A semi-join's statistics counted by document (`corpus`' `Counted::Measured`) are `measured`'s statistics of its rows,
    /// counted by document as every base relation's are (`statkeep::RelStats`), so the documents' table keeps and
    /// patches them as it does the others': every column, the node's a key.
    #[test]
    fn a_semijoins_held_rows_counted_by_document_are_their_measure() {
        // `node(d, n, kind)`-like rows over three documents, kinds repeating within and across them.
        let rows: Vec<[i64; 3]> = (0..3i64).flat_map(|d| (0..7i64).map(move |i| [d, d << 32 | i, (i * d + i / 2) % 4])).collect();
        let names: Vec<String> = ["d", "n", "kind"].iter().map(|s| (*s).to_owned()).collect();
        let mut k = crate::code_rule_dsl::logic::plan::statkeep::RelStats::new(names.clone());
        k.documents(&rows.concat(), 3, 1);
        let columns: Vec<Option<Vec<i64>>> = (0..3).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
        let source = crate::code_rule_dsl::logic::native::setfix::columns_source(names, columns);
        let measured = crate::code_rule_dsl::facts::vtab::syntax::measured(source.as_ref());
        assert_eq!(k.statistics(&|_| true), measured);
        // A document's rows counted out are gone from them: an edit patches them so.
        k.documents(&rows[..7].concat(), 3, -1);
        let rest: Vec<Option<Vec<i64>>> = (0..3).map(|c| Some(rows[7..].iter().map(|r| r[c]).collect())).collect();
        let names: Vec<String> = ["d", "n", "kind"].iter().map(|s| (*s).to_owned()).collect();
        let source = crate::code_rule_dsl::logic::native::setfix::columns_source(names, rest);
        assert_eq!(k.statistics(&|_| true), crate::code_rule_dsl::facts::vtab::syntax::measured(source.as_ref()));
    }

    /// `k` kept as its plan's kept relations in a store of its own (its relations of no symbol column), and read back
    /// from them.
    fn kept_and_read(name: &str, k: Keeper) -> Keeper {
        let dir = std::env::temp_dir().join(format!("rerail-keptstats-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::content_store::ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let plan = [7u8; 32];
        store.queue_kept(super::space(), k.into_kept(&plan, &[8u8; 32], &|_, names| Some(vec![false; names.len()])).unwrap());
        store.written().unwrap();
        let back = Keeper::of_kept(&store.kept_space(&super::space()).unwrap().expect("kept"), &plan, &|_| Ok(())).expect("a keeper");
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
        back
    }

    /// A keeper read back from its kept relations counts the same files the same way: an edit patches it as it would
    /// the one that wrote it, and only stored contents are kept.
    #[test]
    fn a_kept_keeper_reads_back_the_files_and_statistics_it_counted() {
        let entry = |key: i64, salt: i64| {
            let mut e = memo::Entry { key, ..memo::Entry::default() };
            e.rows.insert("r".to_owned(), (2, (0..20).flat_map(|i| [key << 32 | i, (i + salt) % 3]).collect()));
            e
        };
        let id = |n| memo::Content::Stored(crate::content_store::SourceId::from_raw(n));
        let rels = [("r".to_owned(), vec!["n".to_owned(), "v".to_owned()])];
        let mut k = Keeper::default();
        k.begin(&rels, &Default::default(), Default::default());
        for (n, path) in ["a.py", "b.py"].iter().enumerate() {
            k.file(path, Some(id(n as i64 + 1)), n as i64 + 1, &entry(n as i64 + 1, 0), &|_, _| None);
        }
        let first = k.end(&|_, _, _| None).unwrap();
        let mut back = kept_and_read("files", k);
        // b.py edited: counted out by its old content, in by its new.
        back.begin(&rels, &Default::default(), Default::default());
        back.file("a.py", Some(id(1)), 1, &entry(1, 0), &|_, _| None);
        back.file("b.py", Some(id(3)), 2, &entry(2, 1), &|c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 2).then(|| entry(key, 0)));
        let after = back.end(&|_, _, _| None).unwrap();
        assert_eq!(back.counted, (1, 1));
        let r = "r".to_owned();
        assert_eq!(after[&r].rows(), first[&r].rows());
        assert_ne!(after[&r], first[&r], "b.py's values moved");
    }

    /// A symbol column's exact counts are kept as atoms and read back through the reader's mapping (decision 0170), all
    /// in one call; a value column's stay in its statistics, unmapped; a relation whose symbol columns are unknown
    /// keeps none.
    #[test]
    fn a_symbol_columns_counts_are_mapped_as_read_and_only_theirs() {
        let entry = |key: i64| {
            let mut e = memo::Entry { key, ..memo::Entry::default() };
            e.rows.insert("r".to_owned(), (2, (0..6).flat_map(|i| [key << 32 | i, 40 + i % 3]).collect()));
            e.rows.insert("u".to_owned(), (2, (0..6).flat_map(|i| [key << 32 | i, 50 + i % 2]).collect()));
            e
        };
        let names = |c: &[&str]| c.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        let rels = [("r".to_owned(), names(&["n", "v"])), ("u".to_owned(), names(&["n", "w"]))];
        let mut k = Keeper::default();
        k.begin(&rels, &Default::default(), Default::default());
        k.file("a.py", Some(memo::Content::Stored(crate::content_store::SourceId::from_raw(1))), 1, &entry(1), &|_, _| None);
        k.end(&|_, _, _| None).unwrap();
        let dir = std::env::temp_dir().join(format!("rerail-keptstats-syms-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::content_store::ContentStore::open(&dir.join("store.sqlite")).unwrap();
        let plan = [9u8; 32];
        let syms = |rel: &str, _: &[String]| (rel == "r").then(|| vec![false, true]);
        store.queue_kept(super::space(), k.into_kept(&plan, &[8u8; 32], &syms).unwrap());
        store.written().unwrap();
        let calls = std::cell::Cell::new(0);
        let to_local = |v: &mut [i64]| {
            calls.set(calls.get() + 1);
            v.iter_mut().for_each(|x| *x += 1000);
            Ok(())
        };
        let mut back = Keeper::of_kept(&store.kept_space(&super::space()).unwrap().expect("kept"), &plan, &to_local).expect("a keeper");
        assert_eq!(calls.get(), 1, "one call");
        let mut exact = back.rels.get_mut("r").unwrap().take_exact(1).expect("r.v's counts");
        exact.sort();
        assert_eq!(exact, [(1040, 2), (1041, 2), (1042, 2)], "mapped as read");
        assert!(back.rels.get_mut("u").unwrap().take_exact(1).is_none(), "u's symbol columns unknown: no counts kept");
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A rule's edit reaches one relation (its identity changes): the next run counts that relation again from every
    /// file's entry and leaves the other as counted, and the statistics are a keeper's that counted everything from
    /// scratch, with a file edited and one gone in the same run.
    #[test]
    fn a_relation_an_edit_reached_is_counted_again_and_the_others_stay() {
        use std::collections::BTreeMap;
        let entry = |key: i64, salt: i64, e_salt: i64| {
            let mut e = memo::Entry { key, ..memo::Entry::default() };
            e.rows.insert("r".to_owned(), (2, (0..20).flat_map(|i| [key << 32 | i, (i + salt) % 3]).collect()));
            e.rows.insert("e".to_owned(), (2, (0..10).flat_map(|i| [key << 32 | i, (i * e_salt) % 5]).collect()));
            e
        };
        let id = |n| memo::Content::Stored(crate::content_store::SourceId::from_raw(n));
        let names = |c: &[&str]| c.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>();
        let rels = [("r".to_owned(), names(&["n", "v"])), ("e".to_owned(), names(&["c0", "c1"]))];
        let ids = |tag: u8| BTreeMap::from([("e".to_owned(), [tag; 32])]);
        // Run 1: a.py, b.py, c.py, with `e` under identity 1.
        let mut k = Keeper::default();
        k.begin(&rels, &ids(1), Default::default());
        for n in 1..=3 {
            k.file(&format!("{n}.py"), Some(id(n)), n, &entry(n, 0, 1), &|_, _| None);
        }
        k.end(&|_, _, _| None).unwrap();
        let mut k = kept_and_read("edit", k);
        // Run 2: `e` under identity 2 (its rows changed in every file), 2.py edited, 3.py gone.
        k.begin(&rels, &ids(2), Default::default());
        assert!(k.has_fresh());
        k.file("1.py", Some(id(1)), 1, &entry(1, 0, 2), &|_, _| None);
        k.file("2.py", Some(id(4)), 2, &entry(2, 1, 2), &|c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 2).then(|| entry(key, 0, 1)));
        let carried = k.end(&|_, c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 3).then(|| entry(key, 0, 1))).unwrap();
        assert!(!k.has_fresh());
        let mut scratch = Keeper::default();
        scratch.begin(&rels, &ids(2), Default::default());
        scratch.file("1.py", Some(id(1)), 1, &entry(1, 0, 2), &|_, _| None);
        scratch.file("2.py", Some(id(4)), 2, &entry(2, 1, 2), &|_, _| None);
        assert_eq!(carried, scratch.end(&|_, _, _| None).unwrap());
        assert_eq!(carried["e"].rows(), 20);
        assert_eq!(carried["r"].rows(), 40);
    }
}

/// A semi-join's base relation's statistics, counted of its rows at the nodes the exports hold, the chunks' and the
/// hits' (read off the corpus batch by a walk of their documents, `Source::for_rows_keyed`). They are the rows the
/// cross part looks up (decision 0458 binds the whole view, but the cross part reads it only at those nodes), so the
/// cross part's statistics of the relation are theirs, and the view's other rows are never computed to measure it.
/// A node's row is counted once, though it is read twice. Exact: each value's count, and its count in each document
/// (the first column's high half, as `syntax::measured` reads it).
#[derive(Default)]
pub struct SemijoinCounts {
    nodes: rustc_hash::FxHashSet<i64>,
    /// The node's column: every counted row's own node, so a key with no counts.
    node: Option<usize>,
    all: Vec<rustc_hash::FxHashMap<i64, i64>>,
    by_doc: Vec<rustc_hash::FxHashMap<(i64, i64), i64>>,
}

impl SemijoinCounts {
    /// Node `node`'s row, its columns in `input_columns` order with the node at `at`: counted unless counted already.
    /// Whether it was counted now.
    pub fn row(&mut self, node: i64, at: usize, row: &[i64]) -> bool {
        if !self.nodes.insert(node) {
            return false;
        }
        self.node = Some(at);
        if self.all.len() < row.len() {
            self.all.resize_with(row.len(), Default::default);
            self.by_doc.resize_with(row.len(), Default::default);
        }
        let d = row[0] >> 32;
        for (c, &v) in row.iter().enumerate().filter(|&(c, _)| c != at) {
            *self.all[c].entry(v).or_default() += 1;
            *self.by_doc[c].entry((v, d)).or_default() += 1;
        }
        true
    }

    /// The statistics (`syntax::statistics`' rows) of the columns `names`, each named in `input` (the rows' columns):
    /// `None` if one is not, or the rows' document column is not `names`' first.
    pub fn statistics(&self, input: &[&str], names: &[String]) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        if names.first().map(String::as_str) != input.first().copied() {
            return None;
        }
        let rows = self.nodes.len() as i64;
        names
            .iter()
            .map(|name| {
                let c = input.iter().position(|i| i == name)?;
                Some(match (self.all.get(c).filter(|_| self.node != Some(c)), self.by_doc.get(c)) {
                    (Some(all), Some(by_doc)) => {
                        let (ndv, sq, sqd) = (all.len() as i64, all.values().map(|n| n * n).sum(), by_doc.values().map(|n| n * n).sum());
                        // A key column (every value once) reads as `measured`'s does.
                        if ndv == rows { (name.clone(), rows, rows, rows, rows) } else { (name.clone(), rows, ndv, sq, sqd) }
                    }
                    _ => (name.clone(), rows, rows, rows, rows),
                })
            })
            .collect()
    }
}
