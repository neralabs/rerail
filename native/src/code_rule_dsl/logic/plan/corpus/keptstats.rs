//! The base relations' and exports' statistics, kept by document across a memoised program's runs (decision 0096 §3).
//!
//! A memoised run sees every file's rows of the base relations the cross-file part reads, and of the exports the
//! file-local part hands it: a memo hit's from its entry, an evaluated file's from the entry it is remembered under.
//! Each is kept under its entry's key (`memo::base_key(r)` for a base relation, `r` for an export). Rather than
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

/// One memoised program's statistics, and which content of each file they count.
#[derive(Default)]
pub(super) struct Keeper {
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
    pub(super) counted: (usize, usize),
    /// A session's exports, counted by file as the statistics are (decision 0096 step 3), with the exports they
    /// hold; `building` while a run counts every file in (the first run, or after one gave them up).
    exports: Option<super::exports::Counted>,
    export_rels: Vec<String>,
    building: bool,
    /// With the exports: each base relation's held columns, counted by file (`Counted`, a row emitted once per file
    /// holding it), by (relation, memo key, the columns taken from an entry's row); and the files' tree counts.
    base_rels: Vec<(String, String, Vec<usize>)>,
    base: super::exports::Counted,
    own: super::Own,
}

/// What a session's keeper keeps of its files beyond their statistics (0096 step 3): the exports, and each base
/// relation's held columns as (relation, memo key, the columns an entry's row gives).
pub(super) struct Keep {
    pub(super) exports: Vec<String>,
    pub(super) base: Vec<(String, String, Vec<usize>)>,
}

impl Keeper {
    /// For a run reading `rels`, by entry key: the base relations held (`memo::base_key`, each with every column
    /// `input_columns` names) and the exports (each with its columns `c0…`), and `exports`, each export's column that
    /// names its rows' document (`Keeper::doc_at`).
    pub(super) fn begin(&mut self, rels: &[(String, Vec<String>)], exports: HashMap<String, Option<usize>>, keep: Option<Keep>) {
        self.doc_at = exports;
        self.seen.clear();
        self.failed = None;
        self.counted = (0, 0);
        // A relation new to the keeper (another program's set): counted from scratch.
        if rels.iter().any(|(rel, _)| !self.rels.contains_key(rel)) && !self.docs.is_empty() {
            self.docs.clear();
            self.rels.clear();
        }
        for (rel, columns) in rels {
            self.rels.entry(rel.clone()).or_insert_with(|| RelStats::new(columns.clone()));
        }
        // The exports counted while the files counted are the same ones; else counted again from every file.
        match keep {
            Some(Keep { exports: names, base }) => {
                if self.exports.is_none() || self.export_rels != names || self.base_rels != base || self.docs.is_empty() {
                    self.exports = Some(super::exports::Counted::default());
                    self.base = super::exports::Counted::default();
                    self.own = super::Own::default();
                    self.building = true;
                } else {
                    self.building = false;
                }
                self.export_rels = names;
                self.base_rels = base;
            }
            None => {
                self.exports = None;
                self.building = false;
            }
        }
    }

    /// Whether the counted exports hold this run's rows of the files counted (none built this run, none given up):
    /// a memo hit's export rows need not be held again.
    pub(super) fn exports_held(&self) -> bool {
        self.exports.is_some() && !self.building && self.failed.is_none()
    }

    /// Every file counted, with the content and key it was counted under.
    pub(super) fn counted_files(&self) -> Vec<(String, memo::Content, i64)> {
        self.docs.iter().map(|(p, (c, k))| (p.clone(), *c, *k)).collect()
    }

    /// The files counted that this run has not seen yet, with the content and key they were counted under.
    pub(super) fn unseen(&self) -> Vec<(String, memo::Content, i64)> {
        self.docs.iter().filter(|(p, _)| !self.seen.contains(*p)).map(|(p, (c, k))| (p.clone(), *c, *k)).collect()
    }

    /// The exports counted, after a run whose every file was counted (`end` gave the statistics).
    pub(super) fn counted_exports(&self) -> Option<&super::exports::Counted> {
        self.exports.as_ref().filter(|_| self.failed.is_none())
    }

    /// With `counted_exports`: the base relations' columns counted, by (relation, the columns' order in `base_rels`).
    pub(super) fn counted_base(&self) -> Option<(&super::exports::Counted, &[(String, String, Vec<usize>)])> {
        self.counted_exports().map(|_| (&self.base, self.base_rels.as_slice()))
    }

    /// With `counted_exports`: the counted files' tree counts, summed.
    pub(super) fn counted_own(&self) -> Option<&super::Own> {
        self.counted_exports().map(|_| &self.own)
    }

    /// Whether this run has given the keeper up.
    pub(super) fn failed(&self) -> bool {
        self.failed.is_some()
    }

    pub(super) fn fail(&mut self, why: impl Into<String>) {
        self.failed.get_or_insert_with(|| why.into());
        self.exports = None;
        self.building = false;
    }

    /// File `path`'s rows in this run (`entry`, under `content`, its document `key`): counted in if its content is new to the keeper,
    /// the content it counted before counted out first (read back by `old`).
    pub(super) fn file(&mut self, path: &str, content: Option<memo::Content>, key: i64, entry: &memo::Entry, old: &dyn Fn(memo::Content, i64) -> Option<memo::Entry>) {
        if self.failed.is_some() {
            return;
        }
        let Some(content) = content else {
            return self.fail(format!("{path}: no memo content"));
        };
        self.seen.insert(path.to_owned());
        let now = (content, key);
        match self.docs.get(path).copied() {
            Some(was) if same(was, now) => {
                // Counted already; its exports too, unless they are being counted from every file.
                if self.building {
                    self.patch_exports(path, entry, 1);
                }
                return;
            }
            Some((c, k)) => match {
                if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
                    eprintln!("statkeep {path}: counted under {:?}, now {:?}", (c, k), now);
                }
                old(c, k)
            } {
                Some(before) => {
                    // A relation whose rows the edit left as they were (node ids by definition keep an unchanged
                    // definition's) is counted out and in again to no effect: left as counted. Each relation's
                    // statistics are its own rows', so this is exact; within a relation the rows are counted out and
                    // in whole, as its per-document statistics need.
                    let (before, now_rows) = changed_only(&before, entry);
                    crate::code_rule_dsl::logic::profile::decision("statistics kept by file (decision 0096 §3)", || {
                        format!("an edited file's unchanged relations not counted again: {} of {} changed", now_rows.rows.len(), entry.rows.len())
                    });
                    self.count(&before, -1);
                    if !self.building {
                        self.patch_exports(path, &before, -1);
                    }
                    self.count(&now_rows, 1);
                    self.patch_exports(path, &now_rows, 1);
                    self.docs.insert(path.to_owned(), now);
                    return;
                }
                None => return self.fail(format!("{path}: the memo no longer holds the rows counted")),
            },
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
        let mut ok = self.export_rels.iter().all(|rel| match entry.rows.get(rel) {
            Some((width, values)) => counted.patch(rel, *width, values, sign),
            None => true,
        });
        for (rel, key, from) in &self.base_rels {
            let Some((width, values)) = entry.rows.get(key) else { continue };
            if from.iter().any(|c| c >= width) {
                ok = false;
                break;
            }
            let projected: Vec<i64> = values.chunks_exact(*width).flat_map(|row| from.iter().map(|c| row[*c])).collect();
            ok &= self.base.patch(rel, from.len(), &projected, sign);
        }
        let own = super::Own::taken(&entry.rows);
        if sign > 0 { self.own.add(&own) } else { self.own.sub(&own) }
        if !ok {
            self.fail(format!("{path}: an export row counted out that the counted exports do not hold"));
        }
    }

    fn count(&mut self, entry: &memo::Entry, sign: i64) {
        for (key, stats) in &mut self.rels {
            if let Some((width, values)) = entry.rows.get(key) {
                match self.doc_at.get(key) {
                    Some(at) => stats.rows_by(values, *width, sign, *at),
                    None => stats.documents(values, *width, sign),
                }
            }
        }
        if sign > 0 { self.counted.0 += 1 } else { self.counted.1 += 1 }
    }

    /// The run's end: files not seen are counted out. The statistics by entry key, unless the run could not keep them
    /// (then the reason, and the keeper starts again empty).
    pub(super) fn end(&mut self, old: &dyn Fn(&str, memo::Content, i64) -> Option<memo::Entry>) -> Result<BTreeMap<String, RelStats>, String> {
        if self.failed.is_none() {
            let gone: Vec<String> = self.docs.keys().filter(|p| !self.seen.contains(*p)).cloned().collect();
            for path in gone {
                let (c, k) = self.docs.remove(&path).expect("listed");
                match old(&path, c, k) {
                    Some(before) => {
                        self.count(&before, -1);
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
            self.exports = None;
            return Err(why);
        }
        self.building = false;
        Ok(self.rels.clone())
    }
}

impl Keeper {
    /// The keeper as a run of the tape cache keeps it (decision 0096 §2): one blob, the files' contents it counts
    /// and each relation's statistics. Only stored contents are kept (a session's are its memory's).
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = vec![3u8];
        let int = |out: &mut Vec<u8>, v: i64| out.extend_from_slice(&v.to_le_bytes());
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
        out
    }

    /// The contents kept by digest (a run's new sources, written after its blob was made), as the store holds them
    /// now: `found` gives each digest's source, in one read before the run's first wave.
    pub(super) fn resolve(&mut self, found: impl FnOnce(&[[u8; 32]]) -> Vec<Option<crate::content_store::SourceId>>) {
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
    pub(super) fn decode(bytes: &[u8]) -> Option<Keeper> {
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
        if bytes.first() != Some(&3) {
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
        (at == bytes.len()).then_some(k)
    }
}

pub(super) fn same(a: (memo::Content, i64), b: (memo::Content, i64)) -> bool {
    a.1 == b.1
        && match (a.0, b.0) {
            (memo::Content::Stored(x), memo::Content::Stored(y)) => x == y,
            (memo::Content::Unstored(x), memo::Content::Unstored(y)) | (memo::Content::Held(x), memo::Content::Held(y)) => x == y,
            _ => false,
        }
}

#[cfg(test)]
pub mod tests {
    use super::{Keeper, memo};

    /// A keeper read back from its blob counts the same files the same way: an edit patches it as it would the one
    /// that wrote it, and only stored contents are kept.
    #[test]
    fn a_kept_blob_reads_back_the_files_and_statistics_it_counted() {
        let entry = |key: i64, salt: i64| {
            let mut e = memo::Entry { key, ..memo::Entry::default() };
            e.rows.insert(memo::base_key("r"), (2, (0..20).flat_map(|i| [key << 32 | i, (i + salt) % 3]).collect()));
            e
        };
        let id = |n| memo::Content::Stored(crate::content_store::SourceId::from_raw(n));
        let rels = [(memo::base_key("r"), vec!["n".to_owned(), "v".to_owned()])];
        let mut k = Keeper::default();
        k.begin(&rels, Default::default(), None);
        for (n, path) in ["a.py", "b.py"].iter().enumerate() {
            k.file(path, Some(id(n as i64 + 1)), n as i64 + 1, &entry(n as i64 + 1, 0), &|_, _| None);
        }
        let first = k.end(&|_, _, _| None).unwrap();
        let mut back = Keeper::decode(&k.encode()).expect("a keeper");
        assert_eq!(back.encode(), k.encode());
        // b.py edited: counted out by its old content, in by its new.
        back.begin(&rels, Default::default(), None);
        back.file("a.py", Some(id(1)), 1, &entry(1, 0), &|_, _| None);
        back.file("b.py", Some(id(3)), 2, &entry(2, 1), &|c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 2).then(|| entry(key, 0)));
        let after = back.end(&|_, _, _| None).unwrap();
        assert_eq!(back.counted, (1, 1));
        let r = memo::base_key("r");
        assert_eq!(after[&r].rows(), first[&r].rows());
        assert_ne!(after[&r], first[&r], "b.py's values moved");
    }
}

/// `before` and `now` without the relations whose rows are the same in both: what an edit changed, by relation.
fn changed_only(before: &memo::Entry, now: &memo::Entry) -> (memo::Entry, memo::Entry) {
    let same = |rel: &String| before.rows.get(rel) == now.rows.get(rel);
    let keep = |e: &memo::Entry| memo::Entry { rows: e.rows.iter().filter(|(r, _)| !same(r)).map(|(r, v)| (r.clone(), v.clone())).collect(), key: e.key, nodes: e.nodes, lines: Default::default() };
    (keep(before), keep(now))
}
