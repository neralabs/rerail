//! The base relations' statistics, kept by document across a memoised program's runs (decision 0096 §3).
//!
//! A memoised run sees every file's rows of the base relations the cross-file part reads: a memo hit's from its
//! entry, an evaluated file's from the entry it is remembered under. Rather than measure their concatenation
//! (`syntax::measured`'s sort per column, over every file), the keeper counts a file in when its content is new
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
    /// This run's: the files seen, and whether it could count them all.
    seen: HashSet<String>,
    failed: Option<String>,
    /// Files counted in and out this run.
    pub(super) counted: (usize, usize),
}

impl Keeper {
    /// For a run reading `rels` (the base relations held, each with every column `input_columns` names).
    pub(super) fn begin(&mut self, rels: &[(String, Vec<String>)]) {
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
    }

    pub(super) fn fail(&mut self, why: impl Into<String>) {
        self.failed.get_or_insert_with(|| why.into());
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
            Some(was) if same(was, now) => return,
            Some((c, k)) => match old(c, k) {
                Some(before) => self.count(&before, -1),
                None => return self.fail(format!("{path}: the memo no longer holds the rows counted")),
            },
            None => {}
        }
        self.count(entry, 1);
        self.docs.insert(path.to_owned(), now);
    }

    fn count(&mut self, entry: &memo::Entry, sign: i64) {
        for (rel, stats) in &mut self.rels {
            if let Some((width, values)) = entry.rows.get(&memo::base_key(rel)) {
                stats.documents(values, *width, sign);
            }
        }
        if sign > 0 { self.counted.0 += 1 } else { self.counted.1 += 1 }
    }

    /// The run's end: files not seen are counted out. The statistics by relation, unless the run could not keep them
    /// (then the reason, and the keeper starts again empty).
    pub(super) fn end(&mut self, old: &dyn Fn(&str, memo::Content, i64) -> Option<memo::Entry>) -> Result<BTreeMap<String, RelStats>, String> {
        if self.failed.is_none() {
            let gone: Vec<String> = self.docs.keys().filter(|p| !self.seen.contains(*p)).cloned().collect();
            for path in gone {
                let (c, k) = self.docs.remove(&path).expect("listed");
                match old(&path, c, k) {
                    Some(before) => self.count(&before, -1),
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
            return Err(why);
        }
        Ok(self.rels.clone())
    }
}

impl Keeper {
    /// The keeper as a run of the tape cache keeps it (decision 0096 §2): one blob, the files' contents it counts
    /// and each relation's statistics. Only stored contents are kept (a session's are its memory's).
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut out = vec![1u8];
        let int = |out: &mut Vec<u8>, v: i64| out.extend_from_slice(&v.to_le_bytes());
        let mut docs: Vec<(&String, i64, i64)> = self.docs.iter().filter_map(|(p, (c, k))| match c { memo::Content::Stored(id) => Some((p, id.raw(), *k)), memo::Content::Held(_) => None }).collect();
        // In path order: the same keeper is the same blob.
        docs.sort();
        int(&mut out, docs.len() as i64);
        for (p, id, k) in docs {
            int(&mut out, p.len() as i64);
            out.extend_from_slice(p.as_bytes());
            int(&mut out, id);
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
        if bytes.first() != Some(&1) {
            return None;
        }
        let mut k = Keeper::default();
        for _ in 0..int(&mut at)? {
            let path = text(&mut at)?;
            let id = crate::content_store::SourceId::from_raw(int(&mut at)?);
            k.docs.insert(path, (memo::Content::Stored(id), int(&mut at)?));
        }
        for _ in 0..int(&mut at)? {
            let rel = text(&mut at)?;
            k.rels.insert(rel, RelStats::decode(bytes, &mut at)?);
        }
        (at == bytes.len()).then_some(k)
    }
}

fn same(a: (memo::Content, i64), b: (memo::Content, i64)) -> bool {
    a.1 == b.1
        && match (a.0, b.0) {
            (memo::Content::Stored(x), memo::Content::Stored(y)) => x == y,
            (memo::Content::Held(x), memo::Content::Held(y)) => x == y,
            _ => false,
        }
}

#[cfg(test)]
mod tests {
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
        let rels = [("r".to_owned(), vec!["n".to_owned(), "v".to_owned()])];
        let mut k = Keeper::default();
        k.begin(&rels);
        for (n, path) in ["a.py", "b.py"].iter().enumerate() {
            k.file(path, Some(id(n as i64 + 1)), n as i64 + 1, &entry(n as i64 + 1, 0), &|_, _| None);
        }
        let first = k.end(&|_, _, _| None).unwrap();
        let mut back = Keeper::decode(&k.encode()).expect("a keeper");
        assert_eq!(back.encode(), k.encode());
        // b.py edited: counted out by its old content, in by its new.
        back.begin(&rels);
        back.file("a.py", Some(id(1)), 1, &entry(1, 0), &|_, _| None);
        back.file("b.py", Some(id(3)), 2, &entry(2, 1), &|c, key| matches!(c, memo::Content::Stored(s) if s.raw() == 2).then(|| entry(key, 0)));
        let after = back.end(&|_, _, _| None).unwrap();
        assert_eq!(back.counted, (1, 1));
        assert_eq!(after["r"].rows(), first["r"].rows());
        assert_ne!(after["r"], first["r"], "b.py's values moved");
    }
}
