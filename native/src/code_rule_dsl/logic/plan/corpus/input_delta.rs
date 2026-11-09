//! A session's input deltas, read from the documents that changed (decision 0090): an input's rows gone and new are
//! the changed documents' old and new rows of it, not a diff of its whole old and new rows.
//!
//! ```text
//! r__minus(x) :- r__old(x), !r(x).     was: every row of r and r__old read, each run
//! r__plus(x)  :- r(x), !r__old(x).
//!
//! now:  for each document d whose content changed (or that came or went):
//!         old(d) = d's rows of r in the memo entry of its old content
//!         new(d) = d's rows of r in the entry of its new content
//!       r__minus = ∪ old(d) \ new(d)        r__plus = ∪ new(d) \ old(d)
//! ```
//!
//! The memo (decision 0052, a session's in memory) holds each document's rows per content, so both entries are
//! there: the work is the changed documents' rows.
//!
//! This holds only for a relation whose rows are each one document's. A row of `d`'s entry names a node of `d`
//! (`key << 32 | …`, by preorder or by definition) or of one of its guests, and no other document's entry holds one:
//! the file-local part sees one document. A relation with a row that names neither (a text test's strings, a loaded
//! document's rows, which go to every file that asked) has its delta made by the rules, as does one whose row counts
//! do not add up (`old - gone + new = rows`), which another source fed.

use super::memo::{self, Content, Entry};
use rustc_hash::FxHashSet;
use std::collections::HashMap;

/// The document keys guests take (decision 0055, `memo::doc_keys`): a guest's key is a hash of its host's path, the
/// host node and the grammar, and its rows are in its host's entry alone (`host_of`).
const GUESTS: std::ops::Range<i64> = (1 << 30)..(1 << 31);

/// Each document a run read: its content as the memo knows it, and its document key, by path.
pub(in crate::code_rule_dsl) type Docs = HashMap<String, (Content, i64)>;

/// The documents that differ between two runs: each one's entry before and after (`None`: not read in that run).
/// `None` when an entry is not in the memo (it was emptied, or a chunk's rows were not attributable).
pub(super) fn changed(old: &Docs, new: &Docs, held: &memo::Held, program: &[u8; 32]) -> Option<Vec<(Option<Entry>, Option<Entry>)>> {
    let held = held.lock().expect("the session memo");
    let entry = |path: &str, (content, key): (Content, i64)| -> Option<Entry> {
        let Content::Held(digest) = content else { return None };
        held.get(&(digest, path.to_owned(), *program)).filter(|e| e.key == key).cloned()
    };
    let mut out = Vec::new();
    for (path, &now) in new {
        match old.get(path) {
            Some(&was) if was == now => {}
            Some(&was) => out.push((Some(entry(path, was)?), Some(entry(path, now)?))),
            None => out.push((None, Some(entry(path, now)?))),
        }
    }
    for (path, &was) in old.iter().filter(|(p, _)| !new.contains_key(*p)) {
        out.push((Some(entry(path, was)?), None));
    }
    Some(out)
}

/// The rows gone and new of the input whose entry rows are under `key`, read as the input's columns: `cols[c]` is
/// the entry column the input's column `c` holds, `None` for one it does not hold (it reads as 0). `None` when a
/// row is not its document's alone (no node of that document in it).
pub(super) fn delta(changes: &[(Option<Entry>, Option<Entry>)], key: &str, cols: &[Option<usize>]) -> Option<(Vec<Vec<i64>>, Vec<Vec<i64>>)> {
    let rows_of = |e: &Option<Entry>| -> Option<FxHashSet<Vec<i64>>> {
        let mut out = FxHashSet::default();
        let Some(e) = e else { return Some(out) };
        let Some((width, flat)) = e.rows.get(key) else { return Some(out) };
        if *width == 0 {
            return if flat.is_empty() { Some(out) } else { None };
        }
        for row in flat.chunks(*width) {
            if !row.iter().any(|&v| v >= 1 << 32 && (v >> 32 == e.key || GUESTS.contains(&(v >> 32)))) {
                return None;
            }
            out.insert(cols.iter().map(|c| c.and_then(|c| row.get(c).copied()).unwrap_or(0)).collect());
        }
        Some(out)
    };
    let (mut gone, mut new) = (Vec::new(), Vec::new());
    for (was, now) in changes {
        // The same rows under the same key: nothing gone or new (an edit leaves most relations of a file as they were).
        if let (Some(w), Some(n)) = (was, now)
            && w.key == n.key
            && w.rows.get(key) == n.rows.get(key)
        {
            continue;
        }
        let (was, now) = (rows_of(was)?, rows_of(now)?);
        gone.extend(was.difference(&now).cloned());
        new.extend(now.difference(&was).cloned());
    }
    Some((gone, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: i64, rel: &str, rows: &[[i64; 2]]) -> Entry {
        Entry { key, rows: [(rel.to_owned(), (2, rows.iter().flatten().copied().collect()))].into(), ..Default::default() }
    }

    /// A changed document's rows gone and new, read as the input's columns; its unchanged relations are skipped.
    #[test]
    fn a_changed_documents_rows_are_the_delta() {
        let (d, n) = (7i64, |i: i64| (7i64 << 32) | i);
        let was = entry(d, "r", &[[n(1), 10], [n(2), 20]]);
        let now = entry(d, "r", &[[n(2), 20], [n(3), 30]]);
        let (gone, new) = delta(&[(Some(was.clone()), Some(now.clone()))], "r", &[Some(0), Some(1)]).unwrap();
        assert_eq!((gone, new), (vec![vec![n(1), 10]], vec![vec![n(3), 30]]));
        // Read as an input holding its first column only: the second reads as 0.
        let (gone, new) = delta(&[(Some(was.clone()), Some(now))], "r", &[Some(0), None]).unwrap();
        assert_eq!((gone, new), (vec![vec![n(1), 0]], vec![vec![n(3), 0]]));
        // A document that went: every row gone. One the same: nothing.
        assert_eq!(delta(&[(Some(was.clone()), None)], "r", &[Some(0), Some(1)]).unwrap().0.len(), 2);
        assert_eq!(delta(&[(Some(was.clone()), Some(was))], "r", &[Some(0), Some(1)]).unwrap(), (vec![], vec![]));
    }

    /// A guest's nodes are its host's; a row naming no node of the document (another's, or none) is not its alone.
    #[test]
    fn a_row_not_its_documents_alone_is_left_to_the_rules() {
        let guest = (1i64 << 30) + 5;
        let host = entry(7, "r", &[[(guest << 32) | 1, 0]]);
        assert!(delta(&[(None, Some(host))], "r", &[Some(0), Some(1)]).is_some());
        let other = entry(7, "r", &[[(8i64 << 32) | 1, 0]]);
        assert!(delta(&[(None, Some(other))], "r", &[Some(0), Some(1)]).is_none());
        let strings = entry(7, "r", &[[3, 4]]);
        assert!(delta(&[(None, Some(strings))], "r", &[Some(0), Some(1)]).is_none());
    }
}
