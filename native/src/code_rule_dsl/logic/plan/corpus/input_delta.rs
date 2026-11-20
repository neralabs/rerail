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
use std::collections::HashMap;
use std::sync::Arc;

/// The document keys guests take (decision 0055, `memo::doc_keys`): a guest's key is a hash of its host's path, the
/// host node and the grammar, and its rows are in its host's entry alone (`host_of`).
const GUESTS: std::ops::Range<i64> = (1 << 30)..(1 << 31);

/// Each document a run read: its content as the memo knows it, and its document key, by path.
pub(in crate::code_rule_dsl) type Docs = HashMap<String, (Content, i64)>;

/// The documents that differ between two runs: each one's entry before and after (`None`: not read in that run).
/// `None` when an entry is not in the memo (it was emptied, or a chunk's rows were not attributable). `held`: the
/// session's memo, its entries under the memo plan `program` (`memo::Program::id`).
pub(super) fn changed(old: &Docs, new: &Docs, held: &memo::Held, program: &[u8; 32]) -> Option<Vec<(Option<Arc<Entry>>, Option<Arc<Entry>>)>> {
    let held = held.lock().expect("the session memo");
    let live = std::env::var_os("RERAIL_STRATA_LIVE").is_some();
    let entry = |path: &str, (content, key): (Content, i64)| -> Option<Arc<Entry>> {
        let Content::Held(digest) = content else {
            if live {
                eprintln!("RERAIL_STRATA_LIVE changed documents: {path}'s content is not held ({content:?})");
            }
            return None;
        };
        // Held under its store path (`world::store_path`), as the memo publishes it (`remember_all_but`).
        let found = held.get(&(digest, super::world::store_path(path).to_owned(), *program));
        if live && found.is_none_or(|e| e.key != key) {
            eprintln!("RERAIL_STRATA_LIVE changed documents: {path} has no memo entry (held {}, key {key} vs {:?})", held.len(), found.map(|e| e.key));
        }
        found.filter(|e| e.key == key).cloned()
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
    if !out.is_empty() {
        crate::code_rule_dsl::logic::profile::warm_fault("a document changed since the last run", || format!("{} documents", out.len()));
    }
    Some(out)
}

/// The rows gone and new of the input whose entry rows are under `key`, read as the input's columns: `cols[c]` is
/// the entry column the input's column `c` holds, `None` for one it does not hold (it reads as 0). `None` when a
/// row is not its document's alone (no node of that document in it). Each is named where it lies (0364 R2): by its
/// entry among `changes` and its place in that entry's rows, read in place (`Delta::source`), not copied.
pub(super) fn delta(changes: &[(Option<Arc<Entry>>, Option<Arc<Entry>>)], key: &str, cols: &[Option<usize>]) -> Option<Delta> {
    // The entries, each once: an old one at `2 i`, a new one at `2 i + 1`.
    let entries: Vec<Option<Arc<Entry>>> = changes.iter().flat_map(|(w, n)| [w.clone(), n.clone()]).collect();
    let flats = entries.iter().map(|e| e.as_ref().and_then(|e| e.rows.get(key)).map(|(w, f)| Flat { w: *w, at: f.as_ptr(), len: f.len() })).collect();
    let mut out = Delta { entries, flats, cols: cols.to_vec(), gone: Vec::new(), new: Vec::new() };
    for (i, (was, now)) in changes.iter().enumerate() {
        // The same rows under the same key: nothing gone or new (an edit leaves most relations of a file as they were).
        if let (Some(w), Some(n)) = (was, now)
            && w.key == n.key
            && w.rows.get(key) == n.rows.get(key)
        {
            continue;
        }
        let (w, n) = (2 * i as u32, 2 * i as u32 + 1);
        if !out.owned(w) || !out.owned(n) {
            return None;
        }
        // Each side's rows as the input's columns, a row once; then those the other side lacks.
        let (was_rows, now_rows) = (out.distinct(w), out.distinct(n));
        let lacks = |rows: &hashbrown::HashTable<u32>, e: u32, r: u32| {
            let h = out.hash(e, r);
            rows.find(h, |&o| out.same((e, r), (if e == w { n } else { w }, o))).is_none()
        };
        let gone: Vec<(u32, u32)> = was_rows.iter().filter(|&&r| lacks(&now_rows, w, r)).map(|&r| (w, r)).collect();
        let new: Vec<(u32, u32)> = now_rows.iter().filter(|&&r| lacks(&was_rows, n, r)).map(|&r| (n, r)).collect();
        out.gone.extend(gone);
        out.new.extend(new);
    }
    if !out.gone.is_empty() || !out.new.is_empty() {
        crate::code_rule_dsl::logic::profile::warm_fault("a non-empty delta", || format!("input {key}: {} rows gone, {} new", out.gone.len(), out.new.len()));
    }
    Some(out)
}

/// An input's change, read from the changed documents' entries where their rows lie: each row by its entry (an index
/// into `entries`) and its place among that entry's rows of `key`.
pub(super) struct Delta {
    entries: Vec<Option<Arc<Entry>>>,
    /// Each entry's rows of `key`, found once: a cell is read where it lies, with no search of the entry's
    /// relations by name at each read (2.7 s of O125 step 05's edit was those searches).
    flats: Vec<Option<Flat>>,
    cols: Vec<Option<usize>>,
    pub(super) gone: Vec<(u32, u32)>,
    pub(super) new: Vec<(u32, u32)>,
}

/// An entry's rows of one relation: its width and its flat values, where the entry holds them.
#[derive(Clone, Copy)]
struct Flat {
    w: usize,
    at: *const i64,
    len: usize,
}

// SAFETY: `Flat` points into a `Vec<i64>` of an `Entry` its `Delta` holds by an `Arc` (`entries`), and an entry is
// never changed once made: the values stay where they are, readable from any thread, while the delta lives.
unsafe impl Send for Flat {}
unsafe impl Sync for Flat {}

impl Delta {
    fn flat(&self, e: u32) -> Option<(usize, &[i64])> {
        let f = self.flats[e as usize]?;
        // SAFETY: as `Flat`'s: the entry is held by `self.entries[e]`, unchanged.
        Some((f.w, unsafe { std::slice::from_raw_parts(f.at, f.len) }))
    }
    fn rows(&self, e: u32) -> u32 {
        self.flat(e).map_or(0, |(w, f)| f.len().checked_div(w).unwrap_or(0) as u32)
    }
    /// Input column `c` of entry `e`'s row `r`.
    fn value(&self, (e, r): (u32, u32), c: usize) -> i64 {
        let Some((w, flat)) = self.flat(e) else { return 0 };
        self.cols[c].and_then(|x| (x < w).then(|| flat[r as usize * w + x])).unwrap_or(0)
    }
    fn hash(&self, e: u32, r: u32) -> u64 {
        let mut h = rustc_hash::FxHasher::default();
        for c in 0..self.cols.len() {
            std::hash::Hasher::write_i64(&mut h, self.value((e, r), c));
        }
        std::hash::Hasher::finish(&h)
    }
    fn same(&self, a: (u32, u32), b: (u32, u32)) -> bool {
        (0..self.cols.len()).all(|c| self.value(a, c) == self.value(b, c))
    }
    /// Whether each of entry `e`'s rows names a node of its document or of one of its guests: its document's alone.
    fn owned(&self, e: u32) -> bool {
        let Some(entry) = self.entries[e as usize].as_ref() else { return true };
        let Some((w, flat)) = self.flat(e) else { return true };
        if w == 0 {
            return flat.is_empty();
        }
        flat.chunks(w).all(|row| row.iter().any(|&v| v >= 1 << 32 && (v >> 32 == entry.key || GUESTS.contains(&(v >> 32)))))
    }
    /// Entry `e`'s rows as the input's columns, each distinct row once (by its first place).
    fn distinct(&self, e: u32) -> hashbrown::HashTable<u32> {
        let mut t = hashbrown::HashTable::new();
        for r in 0..self.rows(e) {
            let h = self.hash(e, r);
            if t.find(h, |&o| self.same((e, r), (e, o))).is_none() {
                t.insert_unique(h, r, |&o| self.hash(e, o));
            }
        }
        t
    }
    /// The rows of `which` (`gone` or `new`) read in place, as a source of columns `names`.
    pub(super) fn source(self: &Arc<Self>, gone: bool, names: Vec<String>) -> Arc<dyn crate::code_rule_dsl::facts::vtab::Source> {
        Arc::new(DeltaRows { delta: Arc::clone(self), gone, names })
    }
}

/// `Delta::source`'s rows: a change's rows gone or new, read from the entries.
struct DeltaRows {
    delta: Arc<Delta>,
    gone: bool,
    names: Vec<String>,
}

impl DeltaRows {
    fn picks(&self) -> &[(u32, u32)] {
        if self.gone { &self.delta.gone } else { &self.delta.new }
    }
}

impl crate::code_rule_dsl::facts::vtab::Source for DeltaRows {
    fn columns(&self) -> Vec<String> {
        self.names.clone()
    }
    fn len(&self) -> usize {
        self.picks().len()
    }
    fn estimated_matches(&self, _column: usize, _batch: bool) -> usize {
        self.len()
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.delta.value(self.picks()[row], column)
    }
    fn indexed(&self, _column: usize) -> bool {
        false
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        // A change's rows are few: tested in turn.
        Some((0..self.len()).filter(|&r| values.contains(&self.value(r, column))).collect())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn entry(key: i64, rel: &str, rows: &[[i64; 2]]) -> Arc<Entry> {
        Arc::new(Entry { key, rows: [(rel.to_owned(), (2, rows.iter().flatten().copied().collect()))].into(), ..Default::default() })
    }

    /// A change's rows gone and new, read out to compare (`Delta::source`).
    fn delta(changes: &[(Option<Arc<Entry>>, Option<Arc<Entry>>)], key: &str, cols: &[Option<usize>]) -> Option<(Vec<Vec<i64>>, Vec<Vec<i64>>)> {
        use crate::code_rule_dsl::facts::vtab::Source as _;
        let d = Arc::new(super::delta(changes, key, cols)?);
        let read = |gone: bool| {
            let s = d.source(gone, (0..cols.len()).map(|c| format!("c{c}")).collect());
            (0..s.len()).map(|r| (0..cols.len()).map(|c| s.value(r, c)).collect()).collect()
        };
        Some((read(true), read(false)))
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

/// Rows of `now` that `docs` own, ascending: through `now`'s own index of its first column, a document key (`d`) or a
/// node id (`key << 32 | …`), where it has one; else (`None`) the caller reads `now` through. Decision 0458: an
/// edit reads its changed documents' rows, not every row.
pub(super) fn rows_of_documents(now: &dyn crate::code_rule_dsl::facts::vtab::Source, docs: &std::collections::BTreeSet<i64>) -> Option<Vec<u32>> {
    let first = now.columns().into_iter().next()?;
    let mut rows: Vec<u32> = Vec::new();
    if first == "d" {
        if !now.indexed(0) {
            return None;
        }
        rows.extend(now.lookup(0, &docs.iter().copied().collect::<Vec<_>>())?.into_iter().map(|r| r as u32));
    } else {
        if !now.range_indexed(0) {
            return None;
        }
        for d in docs {
            rows.extend(now.range_lookup(0, Some((d << 32, true)), Some(((d + 1) << 32, false)))?.into_iter().map(|r| r as u32));
        }
    }
    rows.sort_unstable();
    rows.dedup();
    Some(rows)
}

/// A changed base relation's version before an edit (decision 0458, 0364's `Old`), read where its rows lie: its rows
/// now but the changed documents' (`skip`, ascending), then the rows those documents had before the edit (`was`, a
/// batch of their old documents). No row is copied.
pub(super) struct Before {
    now: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>,
    skip: Vec<u32>,
    was: Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>,
    kept: usize,
}

impl Before {
    pub(super) fn new(now: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, skip: Vec<u32>, was: Option<Arc<dyn crate::code_rule_dsl::facts::vtab::Source>>) -> Before {
        let kept = now.len() - skip.len();
        Before { now, skip, was, kept }
    }

    /// The id in `now` of its `r`-th row not skipped: `r` plus the skipped ids before it (`skip[j] - j` counts the rows
    /// kept before `skip[j]`, and does not decrease).
    fn now_row(&self, r: usize) -> usize {
        let (mut lo, mut hi) = (0usize, self.skip.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.skip[mid] as usize - mid <= r { lo = mid + 1 } else { hi = mid }
        }
        r + lo
    }
}

impl crate::code_rule_dsl::facts::vtab::Source for Before {
    fn columns(&self) -> Vec<String> {
        self.now.columns()
    }
    fn len(&self) -> usize {
        self.kept + self.was.as_ref().map_or(0, |w| w.len())
    }
    fn estimated_matches(&self, _column: usize, _batch: bool) -> usize {
        self.len()
    }
    fn indexed(&self, _column: usize) -> bool {
        false
    }
    fn lookup(&self, _column: usize, _values: &[i64]) -> Option<Vec<usize>> {
        None
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if row < self.kept {
            self.now.value(self.now_row(row), column)
        } else {
            self.was.as_ref().expect("a row past the kept ones is an old document's").value(row - self.kept, column)
        }
    }
}

#[cfg(test)]
mod before_tests {
    use super::Before;
    use crate::code_rule_dsl::facts::vtab::Source;
    use std::sync::Arc;

    /// The old version reads the rows now that the changed documents do not own, in order, then their old rows.
    #[test]
    fn before_reads_the_kept_rows_then_the_old_documents_rows() {
        let col = |v: Vec<i64>| crate::code_rule_dsl::logic::native::component::columns_source(vec!["n".to_owned()], vec![Some(v)]);
        let now = col(vec![10, 11, 12, 13, 14, 15]);
        let was = col(vec![99, 98]);
        let before = Before::new(now, vec![0, 2, 3], Some(was));
        let rows: Vec<i64> = (0..before.len()).map(|r| before.value(r, 0)).collect();
        assert_eq!(rows, vec![11, 14, 15, 99, 98]);
        let none = Before::new(col(vec![1, 2]), vec![], None);
        assert_eq!((0..none.len()).map(|r| none.value(r, 0)).collect::<Vec<_>>(), vec![1, 2]);
        let _ = Arc::new(none);
    }
}
