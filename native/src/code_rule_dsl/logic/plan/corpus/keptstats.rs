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

fn same(a: (memo::Content, i64), b: (memo::Content, i64)) -> bool {
    a.1 == b.1
        && match (a.0, b.0) {
            (memo::Content::Stored(x), memo::Content::Stored(y)) => x == y,
            (memo::Content::Held(x), memo::Content::Held(y)) => x == y,
            _ => false,
        }
}
