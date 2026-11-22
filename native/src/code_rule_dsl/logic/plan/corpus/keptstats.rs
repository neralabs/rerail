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
    /// A session's exports, counted by file as the statistics are (decision 0096 step 3), with the exports they
    /// hold; `building` while a run counts every file in (the first run, or after one gave them up).
    exports: Option<super::exports::Counted>,
    export_rels: Vec<String>,
    building: bool,
    own: super::Own,
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

/// What a session's keeper keeps of its files beyond their statistics (0096 step 3): the exports. (A base
/// relation's rows are no entry's: the cross part reads them off the corpus batch, decision 0458.)
pub struct Keep {
    pub exports: Vec<String>,
}

impl Keeper {
    /// For a run reading `rels`, by entry key: the exports, each with its columns `c0…`, and `exports`, each export's column that names its rows' document (`Keeper::doc_at`). A relation counted at
    /// other columns before is counted from scratch.
    /// `ids` gives each relation's identity, where it has one: a relation counted under another identity, or new to
    /// the keeper, is counted again from scratch (`fresh`), from every file's entry, and the others stay as counted. A
    /// run that counts its exports by file too (`keep`) counts everything again instead.
    pub fn begin(&mut self, rels: &[(String, Vec<String>)], ids: &BTreeMap<String, [u8; 32]>, exports: HashMap<String, Option<usize>>, keep: Option<Keep>) {
        self.doc_at = exports;
        self.seen.clear();
        self.trace = Trace::from_env();
        self.failed = None;
        self.counted = (0, 0);
        self.fresh.clear();
        let changed = |k: &Keeper, rel: &str, columns: &[String]| k.rels.get(rel).is_none_or(|s| s.columns() != columns) || k.ids.get(rel) != ids.get(rel);
        if !self.docs.is_empty() && rels.iter().any(|(rel, columns)| changed(self, rel, columns)) {
            // Every relation counted again (or the exports counted by file too): the files are counted from scratch.
            if keep.is_some() || rels.iter().all(|(rel, columns)| changed(self, rel, columns)) {
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
        // The exports counted while the files counted are the same ones; else counted again from every file.
        match keep {
            Some(Keep { exports: names }) => {
                if self.exports.is_none() || self.export_rels != names || self.docs.is_empty() {
                    self.exports = Some(super::exports::Counted::default());
                    self.own = super::Own::default();
                    self.building = true;
                } else {
                    self.building = false;
                }
                self.export_rels = names;
            }
            None => {
                self.exports = None;
                self.building = false;
            }
        }
    }

    /// Whether the counted exports hold this run's rows of the files counted (none built this run, none given up):
    /// a memo hit's export rows need not be held again.
    pub fn exports_held(&self) -> bool {
        self.exports.is_some() && !self.building && self.failed.is_none()
    }

    /// Every file counted, with the content and key it was counted under.
    pub fn counted_files(&self) -> Vec<(String, memo::Content, i64)> {
        self.docs.iter().map(|(p, (c, k))| (p.clone(), *c, *k)).collect()
    }

    /// The files counted that this run has not seen yet, with the content and key they were counted under.
    pub fn unseen(&self) -> Vec<(String, memo::Content, i64)> {
        self.docs.iter().filter(|(p, _)| !self.seen.contains(*p)).map(|(p, (c, k))| (p.clone(), *c, *k)).collect()
    }

    /// The exports counted, after a run whose every file was counted (`end` gave the statistics).
    pub fn counted_exports(&self) -> Option<&super::exports::Counted> {
        self.exports.as_ref().filter(|_| self.failed.is_none())
    }


    /// With `counted_exports`: the counted files' tree counts, summed.
    pub fn counted_own(&self) -> Option<&super::Own> {
        self.counted_exports().map(|_| &self.own)
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
        self.exports = None;
        self.building = false;
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
                // Counted already, but for the relations counted from scratch; its exports too, unless they are being
                // counted from every file.
                if !self.fresh.is_empty() {
                    let fresh = std::mem::take(&mut self.fresh);
                    self.count_where(entry, 1, &|r| fresh.contains(r));
                    self.fresh = fresh;
                }
                if self.building {
                    self.patch_exports(path, entry, 1);
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
                    if !self.building {
                        self.patch_exports(path, &before, -1);
                    }
                    self.count_where(&now_rows, 1, &|r| !fresh.contains(r));
                    if !fresh.is_empty() {
                        self.count_where(entry, 1, &|r| fresh.contains(r));
                    }
                    self.fresh = fresh;
                    self.patch_exports(path, &now_rows, 1);
                    self.docs.insert(path.to_owned(), now);
                    return;
                }
                None => return self.fail(format!("{path}: the memo no longer holds the rows counted")),
                }
            }
            None => {}
        }
        self.count(entry, 1);
        self.patch_exports(path, entry, 1);
        self.docs.insert(path.to_owned(), now);
    }

    /// `entry`'s export rows into the counted exports (`sign` 1) or out (−1); the exports are given up if a row
    /// counted out is not held.
    fn patch_exports(&mut self, path: &str, entry: &memo::Entry, sign: i64) {
        let Some(counted) = self.exports.as_mut() else { return };
        let exports_span = crate::code_rule_dsl::logic::profile::span(|| "statistics: exports counted".into());
        // A file's rows are grouped by the document they name (its location column): its own document's, and each
        // guest's it holds (the same rows in every file that holds that guest). Each group is counted as one
        // (`patch_own`): compared with the rows kept for that document, not hashed a row at a time.
        // `RERAIL_COUNT_OWN=off` counts every row a row at a time, as before decision 0220.
        let by_file = rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var("RERAIL_COUNT_OWN").map_or(true, |v| v != "off"));
        let doc_at = &self.doc_at;
        let ok = self.export_rels.iter().all(|rel| match (entry.rows.get(rel), doc_at.get(rel).copied().flatten()) {
            (Some((width, values)), Some(at)) if by_file && at < *width => {
                by_document(values, *width, |row| memo::document_of(row[at])).iter().all(|(d, rows)| counted.patch_own(rel, *width, *d, Some(at), rows, sign))
            }
            (Some((width, values)), _) => counted.patch(rel, *width, values, sign),
            (None, _) => true,
        });
        drop(exports_span);
        let _own = crate::code_rule_dsl::logic::profile::span(|| "statistics: own counted".into());
        let own = super::Own::taken(&entry.rows);
        if sign > 0 { self.own.add(&own) } else { self.own.sub(&own) }
        if !ok {
            self.fail(format!("{path}: an export row counted out that the counted exports do not hold"));
        }
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
                        if !self.building {
                            self.patch_exports(&path, &before, -1);
                        }
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
            self.exports = None;
            return Err(why);
        }
        self.building = false;
        self.fresh.clear();
        Ok(self.rels.clone())
    }
}

impl Keeper {
    /// The keeper as a run of the tape cache keeps it (decision 0096 §2): one blob, the files' contents it counts
    /// and each relation's statistics. Only stored contents are kept (a session's are its memory's).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![6u8];
        let int = |out: &mut Vec<u8>, v: i64| out.extend_from_slice(&v.to_le_bytes());
        // The engine build that counted them (Q102): its memo entries are of the build (decision 0077 §4), so another
        // build's statistics name rows its memo does not hold, and its next run counts every file in.
        let build = crate::code_rule_dsl::build_ids::engine();
        int(&mut out, build.len() as i64);
        out.extend_from_slice(build.as_bytes());
        // A content the store does not hold yet is kept by its digest: the next run finds its source (`resolve`).
        let mut docs: Vec<(&String, memo::Content, i64)> = self.docs.iter().filter(|(_, (c, _))| !matches!(c, memo::Content::Held(_))).map(|(p, (c, k))| (p, *c, *k)).collect();
        // In path order: the same keeper is the same blob.
        docs.sort_by(|a, b| a.0.cmp(b.0));
        int(&mut out, docs.len() as i64);
        for (p, c, k) in docs {
            int(&mut out, p.len() as i64);
            out.extend_from_slice(p.as_bytes());
            match c {
                memo::Content::Stored(id) => {
                    out.push(0);
                    int(&mut out, id.raw());
                }
                memo::Content::Unstored(d) => {
                    out.push(1);
                    out.extend_from_slice(&d);
                }
                memo::Content::Held(_) => unreachable!("not kept"),
            }
            int(&mut out, k);
        }
        int(&mut out, self.rels.len() as i64);
        for (rel, stats) in &self.rels {
            int(&mut out, rel.len() as i64);
            out.extend_from_slice(rel.as_bytes());
            stats.encode(&mut out);
        }
        int(&mut out, self.holders.len() as i64);
        for (rel, held) in &self.holders {
            int(&mut out, rel.len() as i64);
            out.extend_from_slice(rel.as_bytes());
            let mut pairs: Vec<(i64, i64)> = held.iter().map(|(o, n)| (*o, *n)).collect();
            pairs.sort_unstable();
            int(&mut out, pairs.len() as i64);
            for (o, n) in pairs {
                int(&mut out, o);
                int(&mut out, n);
            }
        }
        int(&mut out, self.ids.len() as i64);
        for (rel, id) in &self.ids {
            int(&mut out, rel.len() as i64);
            out.extend_from_slice(rel.as_bytes());
            out.extend_from_slice(id);
        }
        out
    }

    /// Maps the values its exact maps count in symbol columns with `f` (decision 0170: this process's ids to the
    /// store's as the blob is written, back as it is read), all in one call. `syms` gives a relation's symbol columns
    /// (by its key and width); a relation it does not know keeps no exact maps when `strict`, so its constants are
    /// counted from the rows held. The sketches hash the values as they are counted, and stay as they are: a run
    /// across numberings perturbs their estimates, never a finding.
    /// What `syms` says of each relation's columns (`map_symbols`'s first argument), asked now: for a write that maps
    /// them later, where `syms` is not at hand (the store's writer, decision 0306).
    pub fn symbol_columns(&self, syms: &dyn Fn(&str, &[String]) -> Option<Vec<bool>>) -> BTreeMap<String, Option<Vec<bool>>> {
        self.rels.iter().map(|(rel, stats)| (rel.clone(), syms(rel, stats.columns()))).collect()
    }

    pub fn map_symbols(&mut self, syms: &dyn Fn(&str, &[String]) -> Option<Vec<bool>>, strict: bool, f: impl FnOnce(&mut [i64]) -> crate::content_store::StoreResult<()>) -> crate::content_store::StoreResult<()> {
        let mut taken: Vec<(String, usize, Vec<i64>)> = Vec::new();
        let mut values: Vec<i64> = Vec::new();
        for (rel, stats) in self.rels.iter_mut() {
            let width = stats.columns().len();
            let Some(cols) = syms(rel, stats.columns()).filter(|c| c.len() == width) else {
                if strict {
                    for c in 0..width {
                        stats.take_exact(c);
                    }
                }
                continue;
            };
            for c in (0..width).filter(|&c| cols[c]) {
                if let Some(pairs) = stats.take_exact(c) {
                    let (vs, ns): (Vec<i64>, Vec<i64>) = pairs.into_iter().unzip();
                    values.extend(vs);
                    taken.push((rel.clone(), c, ns));
                }
            }
        }
        if !values.is_empty() {
            f(&mut values)?;
        }
        let mut values = values.into_iter();
        for (rel, c, ns) in taken {
            let pairs = ns.into_iter().map(|n| (values.next().expect("one a count"), n)).collect();
            if let Some(stats) = self.rels.get_mut(&rel) {
                stats.put_exact(c, pairs);
            }
        }
        Ok(())
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

    /// `encode`'s keeper back, `None` if the bytes are not one.
    pub fn decode(bytes: &[u8]) -> Option<Keeper> {
        let mut at = 1;
        let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
            let b = bytes.get(*at..*at + n)?;
            *at += n;
            Some(b)
        };
        let int = |at: &mut usize| -> Option<i64> { Some(i64::from_le_bytes(take(at, 8)?.try_into().ok()?)) };
        let text = |at: &mut usize| -> Option<String> {
            let n = usize::try_from(int(at)?).ok()?;
            String::from_utf8(take(at, n)?.to_vec()).ok()
        };
        if bytes.first() != Some(&6) || text(&mut at)? != crate::code_rule_dsl::build_ids::engine() {
            return None;
        }
        let mut k = Keeper::default();
        for _ in 0..int(&mut at)? {
            let path = text(&mut at)?;
            let content = match take(&mut at, 1)?[0] {
                0 => memo::Content::Stored(crate::content_store::SourceId::from_raw(int(&mut at)?)),
                1 => memo::Content::Unstored(take(&mut at, 32)?.try_into().ok()?),
                _ => return None,
            };
            k.docs.insert(path, (content, int(&mut at)?));
        }
        for _ in 0..int(&mut at)? {
            let rel = text(&mut at)?;
            k.rels.insert(rel, RelStats::decode(bytes, &mut at)?);
        }
        for _ in 0..int(&mut at)? {
            let rel = text(&mut at)?;
            let mut held = rustc_hash::FxHashMap::default();
            for _ in 0..int(&mut at)? {
                let o = int(&mut at)?;
                held.insert(o, int(&mut at)?);
            }
            k.holders.insert(rel, held);
        }
        for _ in 0..int(&mut at)? {
            let rel = text(&mut at)?;
            k.ids.insert(rel, take(&mut at, 32)?.try_into().ok()?);
        }
        (at == bytes.len()).then_some(k)
    }
}

/// The keeper's test that a file's rows are counted already: the same content under the same document key.
pub fn same(a: (memo::Content, i64), b: (memo::Content, i64)) -> bool {
    a.1 == b.1
        && match (a.0, b.0) {
            (memo::Content::Stored(x), memo::Content::Stored(y)) => x == y,
            (memo::Content::Unstored(x), memo::Content::Unstored(y)) | (memo::Content::Held(x), memo::Content::Held(y)) => x == y,
            _ => false,
        }
}

/// `flat`'s rows (`width` a row) grouped by the document `doc` says each names, in the order the documents first
/// come. A document's rows mostly come together, so its group is looked up once a run of them.
fn by_document(flat: &[i64], width: usize, doc: impl Fn(&[i64]) -> i64) -> Vec<(i64, Vec<i64>)> {
    let mut groups: Vec<(i64, Vec<i64>)> = Vec::new();
    let mut at: rustc_hash::FxHashMap<i64, usize> = Default::default();
    let mut last: Option<(i64, usize)> = None;
    for row in flat.chunks_exact(width) {
        let d = doc(row);
        let g = match last {
            Some((l, g)) if l == d => g,
            _ => {
                let g = *at.entry(d).or_insert_with(|| {
                    groups.push((d, Vec::new()));
                    groups.len() - 1
                });
                last = Some((d, g));
                g
            }
        };
        groups[g].1.extend_from_slice(row);
    }
    groups
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

    /// A keeper read back from its blob counts the same files the same way: an edit patches it as it would the one
    /// that wrote it, and only stored contents are kept.
    #[test]
    fn a_kept_blob_reads_back_the_files_and_statistics_it_counted() {
        let entry = |key: i64, salt: i64| {
            let mut e = memo::Entry { key, ..memo::Entry::default() };
            e.rows.insert("r".to_owned(), (2, (0..20).flat_map(|i| [key << 32 | i, (i + salt) % 3]).collect()));
            e
        };
        let id = |n| memo::Content::Stored(crate::content_store::SourceId::from_raw(n));
        let rels = [("r".to_owned(), vec!["n".to_owned(), "v".to_owned()])];
        let mut k = Keeper::default();
        k.begin(&rels, &Default::default(), Default::default(), None);
        for (n, path) in ["a.py", "b.py"].iter().enumerate() {
            k.file(path, Some(id(n as i64 + 1)), n as i64 + 1, &entry(n as i64 + 1, 0), &|_, _| None);
        }
        let first = k.end(&|_, _, _| None).unwrap();
        let mut back = Keeper::decode(&k.encode()).expect("a keeper");
        assert_eq!(back.encode(), k.encode());
        // b.py edited: counted out by its old content, in by its new.
        back.begin(&rels, &Default::default(), Default::default(), None);
        back.file("a.py", Some(id(1)), 1, &entry(1, 0), &|_, _| None);
        back.file("b.py", Some(id(3)), 2, &entry(2, 1), &|c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 2).then(|| entry(key, 0)));
        let after = back.end(&|_, _, _| None).unwrap();
        assert_eq!(back.counted, (1, 1));
        let r = "r".to_owned();
        assert_eq!(after[&r].rows(), first[&r].rows());
        assert_ne!(after[&r], first[&r], "b.py's values moved");
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
        k.begin(&rels, &ids(1), Default::default(), None);
        for n in 1..=3 {
            k.file(&format!("{n}.py"), Some(id(n)), n, &entry(n, 0, 1), &|_, _| None);
        }
        k.end(&|_, _, _| None).unwrap();
        let mut k = Keeper::decode(&k.encode()).expect("a keeper");
        // Run 2: `e` under identity 2 (its rows changed in every file), 2.py edited, 3.py gone.
        k.begin(&rels, &ids(2), Default::default(), None);
        assert!(k.has_fresh());
        k.file("1.py", Some(id(1)), 1, &entry(1, 0, 2), &|_, _| None);
        k.file("2.py", Some(id(4)), 2, &entry(2, 1, 2), &|c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 2).then(|| entry(key, 0, 1)));
        let carried = k.end(&|_, c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 3).then(|| entry(key, 0, 1))).unwrap();
        assert!(!k.has_fresh());
        let mut scratch = Keeper::default();
        scratch.begin(&rels, &ids(2), Default::default(), None);
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
