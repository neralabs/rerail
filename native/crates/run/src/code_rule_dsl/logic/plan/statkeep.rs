//! Planner statistics kept by delta, in memory (decision 0096 §2–§3).
//!
//! A relation's statistics are what `syntax::measured` gives, `(column, rows, ndv, sq, sqd)`, but kept rather than
//! measured: a document's rows are counted in when it enters and out when it leaves or changes, so an edit costs
//! its own rows, not the repository's. Per column:
//!
//! ```text
//! sqd         Σ over (document, value) of count²      exact: a document's term is its own, subtracted whole
//! local ndv   Σ over documents of their distinct values  exact ndv while no value crosses documents (node columns)
//! exact       value → count, while ≤ 256 values         exact ndv and sq for few-valued columns
//! sketch      the 0096 sketch of value → count           ndv and sq within a small factor, for the rest
//! ```
//!
//! A row's document is its first column's high half (`>> 32`), as `syntax::measured` reads it. A column is a node
//! column, exact by sums, while every value names its row's document (a node `d << 32 | i`, or the document key
//! `d` itself): `crossing` counts the documents counted in that hold a value naming another.
//!
//! `delta` patches by rows that are not whole documents (a phase export's plus and minus, a maintained relation's
//! rows gone and new): per-document sums are then not additive, so the column's sums are given up and its exact map
//! or sketch answers, with `sqd` as `sq`, or as a sketch of the (document, value) pairs' counts where one is kept
//! (`paired`, decision 0096 step 4): the sketch is linear, so a row's delete takes out what its insert put in.

use std::collections::BTreeMap;
use rustc_hash::FxHashMap;
use rerail_blocks::vector::Vector;

use super::sketch::Sketch;

/// Values a column's exact map holds at most; past it the map is dropped for the sketch (0096 §1).
const EXACT: usize = 256;

/// A column's exact counts, value → count, in a block (decision 0478 R5, Q338): open addressing, each slot
/// `[value, count]`. A count is never 0 (a value whose count goes back to 0 is taken out, the slots after it shifted
/// back), so a count of 0 is an empty slot. It starts at 16 slots and doubles past half full.
#[derive(Clone, Default)]
struct Exact {
    slots: rerail_blocks::vector::Vector<[i64; 2]>,
    len: usize,
}
impl Exact {
    /// Empty, with room for `n` values: its slots made once, not grown as they are put.
    fn with_room(n: usize) -> Exact {
        let mut e = Exact { slots: Default::default(), len: 0 };
        if n > 0 {
            e.slots.resize((2 * (n + 1)).next_power_of_two().max(16), [0, 0]);
        }
        e
    }
    fn home(&self, v: i64) -> usize {
        (super::sketch::mix_of(v) as usize) & (self.slots.len() - 1)
    }
    /// The slot holding `v`, or the empty one it would go in.
    fn slot(&self, v: i64) -> usize {
        let m = self.slots.len() - 1;
        let mut at = self.home(v);
        while self.slots[at][1] != 0 && self.slots[at][0] != v {
            at = (at + 1) & m;
        }
        at
    }
    fn room(&mut self) {
        if !self.slots.is_empty() && 2 * (self.len + 1) <= self.slots.len() {
            return;
        }
        let mut grown = Exact { slots: crate::code_rule_dsl::facts::rows::vector_in(None, 0), len: 0 };
        grown.slots.resize((2 * self.slots.len()).max(16), [0, 0]);
        for e in self.slots.iter().filter(|e| e[1] != 0) {
            let at = grown.slot(e[0]);
            grown.slots[at] = *e;
            grown.len += 1;
        }
        *self = grown;
    }
    /// `v`'s count set to `n` (0: taken out).
    fn insert(&mut self, v: i64, n: i64) {
        if n == 0 {
            self.add(v, -self.get(v));
            return;
        }
        self.room();
        let at = self.slot(v);
        if self.slots[at][1] == 0 {
            self.len += 1;
        }
        self.slots[at] = [v, n];
    }
    fn get(&self, v: i64) -> i64 {
        if self.len == 0 { 0 } else { self.slots[self.slot(v)][1] }
    }
    /// `d` added to `v`'s count; a count back at 0 is taken out.
    fn add(&mut self, v: i64, d: i64) {
        if d == 0 {
            return;
        }
        self.room();
        let at = self.slot(v);
        if self.slots[at][1] == 0 {
            self.slots[at] = [v, d];
            self.len += 1;
            return;
        }
        self.slots[at][1] += d;
        if self.slots[at][1] != 0 {
            return;
        }
        // Taken out: each entry after it in its run moves back into the hole when its home allows.
        self.len -= 1;
        let m = self.slots.len() - 1;
        let (mut hole, mut at) = (at, (at + 1) & m);
        self.slots[hole] = [0, 0];
        while self.slots[at][1] != 0 {
            let home = self.home(self.slots[at][0]);
            if (at.wrapping_sub(home) & m) >= (at.wrapping_sub(hole) & m) {
                self.slots[hole] = self.slots[at];
                self.slots[at] = [0, 0];
                hole = at;
            }
            at = (at + 1) & m;
        }
    }
    fn len(&self) -> usize {
        self.len
    }
    fn iter(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.slots.iter().filter(|e| e[1] != 0).map(|e| (e[0], e[1]))
    }
    fn values(&self) -> impl Iterator<Item = i64> + '_ {
        self.iter().map(|(_, n)| n)
    }
}
impl FromIterator<(i64, i64)> for Exact {
    fn from_iter<I: IntoIterator<Item = (i64, i64)>>(pairs: I) -> Self {
        let mut e = Exact::default();
        for (v, n) in pairs {
            e.insert(v, n);
        }
        e
    }
}
/// Equal when they count the same values alike, wherever their slots put them.
impl PartialEq for Exact {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().all(|(v, n)| other.get(v) == n)
    }
}
impl Eq for Exact {}
impl std::fmt::Debug for Exact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut pairs: Vec<(i64, i64)> = self.iter().collect();
        pairs.sort_unstable();
        f.debug_map().entries(pairs).finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    /// The within-document sum of squares of the rows whose first column names a node (each document a group).
    pub sqd: i64,
    /// The rows whose first column is a document key (`doc32` 0): `syntax::measured` counts them all as one group,
    /// so their part of the within-document sum, `zero_sq`, is each value's count over every such row, squared. No
    /// sum by document gives it (a document counted out takes out less than it put in), so each value's count is
    /// kept (`zero`) and `zero_sq` moved by its square's change; past `EXACT` values the map is given up and a
    /// document's own squares are taken (as before 0458's patching).
    zero: Option<Exact>,
    zero_sq: i64,
    pub local_ndv: i64,
    /// Documents counted in that hold a value naming another document.
    pub crossing: i64,
    exact: Option<Exact>,
    sketch: Sketch,
    /// `paired`: the 0096 sketch of each (document, value) pair's count, whose `sq` is the column's `sqd`.
    pub pairs: Option<Sketch>,
    /// The values watched (`watch`): the constants the rules test in the column, each counted exactly whatever the
    /// column's values (a count may be 0), so an edit's run takes them as kept rather than counting the rows held.
    watched: FxHashMap<i64, i64>,
}

/// One relation's statistics, kept by delta.
#[derive(Clone, Debug)]
pub struct RelStats {
    columns: Vec<String>,
    pub rows: i64,
    /// Patched by rows that are not whole documents (`delta`): the per-document sums no longer hold.
    partial: bool,
    cols: Cols,
}

impl PartialEq for RelStats {
    fn eq(&self, other: &Self) -> bool {
        self.columns == other.columns && self.rows == other.rows && self.partial == other.partial && self.cols() == other.cols()
    }
}

impl Eq for RelStats {}

/// A relation's columns: made, or read from a store as kept (`RelStats::kept`) and decoded where first read (t33 K4): a
/// run reads every kept relation's statistics and patches those whose files changed, so a relation no file of which
/// changed is answered from its summary and written back as it was read, its sketches never decoded.
#[derive(Clone, Debug, Default)]
struct Cols {
    decoded: std::sync::OnceLock<Vec<Column>>,
    kept: Option<Kept>,
}

/// A relation's columns as kept: where `encode`'s bytes lie, and what was given beside them.
#[derive(Clone)]
struct Kept {
    source: std::sync::Arc<dyn KeptSource>,
    /// Where the columns start in the bytes, and their format (`encode`'s first byte).
    at: usize,
    format: u8,
    /// Per column, whether the bytes hold an exact map, and watched values.
    has_exact: Vec<bool>,
    has_watched: Vec<bool>,
    /// Exact maps and watched values given apart: the source's (`KeptSource::apart`), read where first asked, then as
    /// put or taken (`put_exact`, `watch`), the bytes' replaced as decoding applies them.
    apart: std::sync::OnceLock<Apart>,
    /// Each column's `(ndv, sq, sqd)` as `statistics` gives them with every column held, kept with the bytes.
    summary: Option<std::sync::Arc<[(i64, i64, i64)]>>,
}

impl std::fmt::Debug for Kept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kept").field("bytes", &self.source.len()).field("at", &self.at).field("format", &self.format).field("apart", &self.apart.get()).finish_non_exhaustive()
    }
}

impl Kept {
    fn apart(&self) -> &Apart {
        self.apart.get_or_init(|| self.source.apart())
    }

    fn apart_mut(&mut self) -> &mut Apart {
        self.apart();
        self.apart.get_mut().expect("read")
    }
}

/// A relation's statistics where a store keeps them (t33 K4): `encode`'s bytes, and the exact maps and watched values
/// kept apart from them, read where they lie as the relation is decoded or written back, never held of their own
/// (decision 0353).
pub trait KeptSource: Send + Sync {
    /// The bytes' length.
    fn len(&self) -> usize;
    /// `f`'s answer, given a reader of the bytes: one lookup of where they lie for all its reads.
    fn read(&self, f: &mut dyn FnMut(&dyn ReadAt) -> Option<()>) -> Option<()>;
    /// The exact maps and watched values kept apart, by column, their values this process's atoms.
    fn apart(&self) -> Apart;
}

/// Bytes read at an offset (`KeptSource::read`).
pub trait ReadAt {
    fn len(&self) -> usize;
    /// The bytes at `at..at + out.len()`, into `out`; `None` past their end.
    fn read_at(&self, at: usize, out: &mut [u8]) -> Option<()>;
}

impl ReadAt for [u8] {
    fn len(&self) -> usize {
        <[u8]>::len(self)
    }
    fn read_at(&self, at: usize, out: &mut [u8]) -> Option<()> {
        out.copy_from_slice(self.get(at..at.checked_add(out.len())?)?);
        Some(())
    }
}

/// Exact maps and watched values given apart from the bytes, by column (`KeptSource::apart`).
#[derive(Clone, Debug, Default)]
pub struct Apart {
    pub exact: BTreeMap<usize, Vec<(i64, i64)>>,
    pub watched: BTreeMap<usize, Vec<(i64, i64)>>,
}

/// `encode`'s bytes held in memory (`RelStats::kept`), nothing kept apart.
struct InMemory(std::sync::Arc<[u8]>);

impl KeptSource for InMemory {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn read(&self, f: &mut dyn FnMut(&dyn ReadAt) -> Option<()>) -> Option<()> {
        f(self)
    }
    fn apart(&self) -> Apart {
        Apart::default()
    }
}

impl ReadAt for InMemory {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn read_at(&self, at: usize, out: &mut [u8]) -> Option<()> {
        self.0[..].read_at(at, out)
    }
}

impl Cols {
    fn of(cols: Vec<Column>) -> Self {
        Cols { decoded: std::sync::OnceLock::from(cols), kept: None }
    }
}

/// The rows `RelStats::documents_scanned` counts at once, about 48 bytes each while counted: past it, a window ends
/// where the document of its last row does.
const WINDOW: usize = 1 << 20;

/// The end of the window of `source`'s rows from `lo`: `window` rows on, then on to the end of the last one's document
/// (a row's document is its first column's, as `count` reads it).
fn window_end(source: &dyn crate::code_rule_dsl::facts::vtab::Source, lo: usize, n: usize, window: usize) -> usize {
    let hi = (lo + window).min(n);
    if hi == n {
        return n;
    }
    let mut first = [0i64];
    source.row_at(hi - 1, &[0], &mut first);
    let last = document_of(first[0]);
    let mut end = n;
    source.for_rows_in(hi..n, &[0], &mut |r, row| {
        if document_of(row[0]) == last {
            return true;
        }
        end = r;
        false
    });
    end
}

/// The document a row's first column names: as `syntax::measured` reads it.
pub fn doc32(first: i64) -> i64 {
    first >> 32
}

/// The document a location value names: a node (`key << 32 | index`) or a document key (as `memo::document_of`).
pub fn document_of(v: i64) -> i64 {
    if v >= 1 << 32 { v >> 32 } else { v }
}

/// The key a (document, value) pair is counted under in a column's `pairs` sketch: the sketch hashes it again.
pub fn pair(doc: i64, v: i64) -> i64 {
    v ^ doc.wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64).rotate_left(29)
}

/// The group `syntax::measured` sums a row's `sqd` within: its first column's document, all document keys (`doc32`
/// 0) one group.
fn group(first: i64) -> i64 {
    if doc32(first) == 0 { 0 } else { document_of(first) }
}

/// Whether value `v` names document `d` (its first column's value `first`): a node of it, or its key.
pub fn names_doc(v: i64, first: i64) -> bool {
    let d = doc32(first);
    (d != 0 && v >> 32 == d) || v == document_of(first)
}

/// `pairs` sorted by value, each value's counts summed into one pair.
fn summed(pairs: &mut Vec<(i64, i64)>) {
    pairs.sort_unstable_by_key(|p| p.0);
    pairs.dedup_by(|b, a| {
        let same = a.0 == b.0;
        if same {
            a.1 += b.1;
        }
        same
    });
}

impl RelStats {
    /// No rows yet, of these columns.
    pub fn new(columns: Vec<String>) -> Self {
        let cols = columns.iter().map(|_| Column { exact: Some(Exact::default()), zero: Some(Exact::default()), ..Column::default() }).collect();
        RelStats { columns, rows: 0, partial: false, cols: Cols::of(cols) }
    }

    /// `new`, keeping each column's (document, value) pairs sketched too: for a relation patched by rows that are
    /// not whole documents (a maintained relation's delta), whose `sqd` the per-document sums no longer give.
    pub fn paired(columns: Vec<String>) -> Self {
        let mut s = Self::new(columns);
        for c in s.cols_mut() {
            c.pairs = Some(Sketch::default());
        }
        s
    }

    /// Statistics a source answers from its structure (`Source::statistics`, `within`'s from the tree), as kept ones:
    /// each column's `(name, rows, ndv, sq, sqd)` within documents (no value crossing one), so `statistics` gives them
    /// back and a changed document patches them by its rows as any kept relation's. Kept, the next run reads them
    /// rather than the structure, which in a run whose documents are pending builds every one (Q332).
    pub fn of_statistics(columns: Vec<String>, known: &[(String, i64, i64, i64, i64)]) -> Self {
        let rows = known.first().map_or(0, |m| m.1);
        let cols = columns
            .iter()
            .map(|c| {
                let (ndv, sqd) = known.iter().find(|m| m.0 == *c).map_or((rows, rows), |m| (m.2, m.4));
                Column { local_ndv: ndv, sqd, ..Column::default() }
            })
            .collect();
        RelStats { columns, rows, partial: false, cols: Cols::of(cols) }
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// The same statistics, of columns named `columns` (as many).
    pub fn renamed(&self, columns: Vec<String>) -> Self {
        debug_assert_eq!(columns.len(), self.columns.len());
        RelStats { columns, ..self.clone() }
    }

    pub fn rows(&self) -> i64 {
        self.rows
    }

    /// Whole documents' rows in (`sign` 1) or out (−1): `flat` row-major, `width` values a row (columns past it, in
    /// a row narrower than the relation, are not counted). Every row of each document they name must be in `flat`.
    pub fn documents(&mut self, flat: &[i64], width: usize, sign: i64) {
        self.rows_of(flat, width, sign, None, None)
    }

    /// `documents`, where this relation's columns are some of the rows' (`from[j]`: the row's column that is this
    /// relation's `j`): a base relation counted at the columns its rules read. A row's document is still its own
    /// first column's.
    pub fn documents_at(&mut self, flat: &[i64], width: usize, from: &[usize], sign: i64) {
        debug_assert_eq!(from.len(), self.columns.len());
        self.rows_of(flat, width, sign, None, Some(from))
    }

    /// Rows in or out, as `documents`, where a row's document is the one its column `at` names (a node,
    /// `key << 32 | index`), or one document for all when `at` is `None`: as the chunks' totals read an export's
    /// rows (its first node column), whose first column may hold a value many documents share.
    pub fn rows_by(&mut self, flat: &[i64], width: usize, sign: i64, at: Option<usize>) {
        self.rows_of(flat, width, sign, Some(at), None)
    }

    /// `rows_by` of the rows `skip` does not take: a file's rows less those of the value documents another file holds
    /// (`keptstats::Keeper::count_where`), read where they lie by their indices, not copied.
    pub fn rows_by_except(&mut self, flat: &[i64], width: usize, sign: i64, at: Option<usize>, skip: &dyn Fn(&[i64]) -> bool) {
        if width == 0 || flat.is_empty() {
            return;
        }
        let rows: Vec<usize> = (0..flat.len() / width).filter(|&r| !skip(&flat[r * width..(r + 1) * width])).collect();
        if rows.len() == flat.len() / width {
            return self.rows_by(flat, width, sign, at);
        }
        self.count(rows.len(), width, &mut |c, each| rows.iter().enumerate().for_each(|(i, &r)| each(i, flat[r * width + c])), sign, Some(at), None)
    }

    /// `documents_at` over `n` rows read where they lie (`value(row, column)`, a source's), not a flat copy of them:
    /// a base relation's documents counted off the corpus batch (decision 0458).
    pub fn documents_read(&mut self, n: usize, width: usize, value: &dyn Fn(usize, usize) -> i64, from: &[usize], sign: i64) {
        debug_assert_eq!(from.len(), self.columns.len());
        self.count(n, width, &mut |c, each| (0..n).for_each(|r| each(r, value(r, c))), sign, None, Some(from))
    }

    /// `documents_read` of a source's rows, each column read by the source's walk (`Source::for_rows`: a view's,
    /// document by document) rather than a row at a time, which a view answers by a rank and a select (Q183): a
    /// base relation counted off the corpus batch, cold (decision 0458).
    pub fn documents_scanned(&mut self, source: &dyn crate::code_rule_dsl::facts::vtab::Source, from: &[usize], sign: i64) {
        self.documents_scanned_in(source, from, sign, WINDOW);
    }

    /// `documents_scanned` of rows `lo..hi` of `source`, which hold whole documents: one chunk's own files' rows of a
    /// relation its batch holds (decision 0458's statistics, counted where the chunk's rows lie).
    pub fn documents_scanned_range(&mut self, source: &dyn crate::code_rule_dsl::facts::vtab::Source, lo: usize, hi: usize, from: &[usize], sign: i64) {
        debug_assert_eq!(from.len(), self.cols().len());
        let width = source.columns().len();
        let mut at = lo;
        while at < hi {
            let end = window_end(source, at, hi, WINDOW).min(hi);
            self.count(
                end - at,
                width,
                &mut |c, each| {
                    source.for_rows_in(at..end, &[c], &mut |r, row| {
                        each(r - at, row[0]);
                        true
                    });
                },
                sign,
                None,
                Some(from),
            );
            at = end;
        }
    }

    /// Another set of documents' statistics in (none of them counted here already): each sum adds, each value's count
    /// adds, and the sketches add. The same as counting both sets' rows here, whole documents at a time. `false`, and
    /// nothing changed, when their columns differ.
    pub fn merge(&mut self, other: &RelStats) -> bool {
        if self.columns != other.columns {
            return false;
        }
        self.rows += other.rows;
        self.partial |= other.partial;
        for (a, b) in self.cols_mut().iter_mut().zip(other.cols()) {
            a.sqd += b.sqd;
            match &b.zero {
                Some(z) => {
                    let pairs: Vec<(i64, i64)> = z.iter().collect();
                    Self::zeros(a, &pairs, 1);
                }
                None => {
                    a.zero = None;
                    a.zero_sq += b.zero_sq;
                }
            }
            a.local_ndv += b.local_ndv;
            a.crossing += b.crossing;
            match (&mut a.exact, &b.exact) {
                (Some(x), Some(y)) => {
                    for (v, n) in y.iter() {
                        x.add(v, n);
                    }
                    if x.len() > EXACT {
                        a.exact = None;
                    }
                }
                _ => a.exact = None,
            }
            a.sketch.merge(&b.sketch, 1);
            match (&mut a.pairs, &b.pairs) {
                (Some(x), Some(y)) => x.merge(y, 1),
                _ => a.pairs = None,
            }
        }
        true
    }

    /// `documents_scanned`, `window` rows at a time and on to the end of a window's last document.
    pub fn documents_scanned_in(&mut self, source: &dyn crate::code_rule_dsl::facts::vtab::Source, from: &[usize], sign: i64, window: usize) {
        debug_assert_eq!(from.len(), self.columns.len());
        let (n, width) = (source.len(), source.columns().len());
        if n <= window {
            self.count(
                n,
                width,
                &mut |c, each| {
                    let mut next = 0;
                    source.for_rows(&[c], &mut |r, row| {
                        debug_assert_eq!(r, next, "a source's walk reads its rows in order");
                        next = r + 1;
                        each(r, row[0]);
                    });
                    debug_assert_eq!(next, n, "a source's walk reads every row");
                },
                sign,
                None,
                Some(from),
            );
            return;
        }
        // Whole documents at a time: `count` lists each row's document and value (about 48 bytes a row), and its
        // counts add over whole documents (as a patch's documents in and out do), so a window of rows that ends where
        // a document does is counted alone. `node` listed whole was 509 MB of a 1,000-file run.
        let mut lo = 0;
        while lo < n {
            let hi = window_end(source, lo, n, window);
            self.count(
                hi - lo,
                width,
                &mut |c, each| {
                    let mut next = lo;
                    source.for_rows_in(lo..hi, &[c], &mut |r, row| {
                        debug_assert_eq!(r, next, "a source's walk reads its rows in order");
                        next = r + 1;
                        each(r - lo, row[0]);
                        true
                    });
                    debug_assert_eq!(next, hi, "a source's walk reads every row of the window");
                },
                sign,
                None,
                Some(from),
            );
            lo = hi;
        }
    }

    pub fn rows_of(&mut self, flat: &[i64], width: usize, sign: i64, at: Option<Option<usize>>, from: Option<&[usize]>) {
        if width == 0 || flat.is_empty() {
            return;
        }
        self.count(flat.len() / width, width, &mut |c, each| (0..flat.len() / width).for_each(|r| each(r, flat[r * width + c])), sign, at, from)
    }

    /// Rows in or out, read a column at a time: `column(c, each)` hands each row's value of column `c`, in row order.
    fn count(&mut self, n: usize, width: usize, column: &mut dyn FnMut(usize, &mut dyn FnMut(usize, i64)), sign: i64, at: Option<Option<usize>>, from: Option<&[usize]>) {
        if width == 0 || n == 0 {
            return;
        }
        // The column a row's document is read from: its first, else `at`'s; read once.
        let keyed = match at {
            None => Some(0),
            Some(at) => at,
        };
        let mut key = vec![0i64; if keyed.is_some() { n } else { 0 }];
        if let Some(k) = keyed {
            column(k, &mut |r, v| key[r] = v);
        }
        // A row's document (as `doc32` and `document_of` read its first column, else by `at`), and whether `v` names it.
        let of = |r: usize| match at {
            None => (doc32(key[r]), document_of(key[r])),
            Some(at) => {
                let d = at.map_or(-1, |_| key[r] >> 32);
                (d, d)
            }
        };
        let names = |v: i64, r: usize| match at {
            None => names_doc(v, key[r]),
            Some(at) => at.is_some_and(|_| v >> 32 == key[r] >> 32),
        };
        self.rows += sign * n as i64;
        // One pass a column: each row's (document, value, whether its `doc32` is 0) is listed; in order (node
        // columns mostly are), each (document, value) is a run, else they are counted in one map kept across columns. `syntax::measured` groups sqd by `doc32`: every row whose first column is a document key is one
        // group, so those rows' counts are summed by value across documents (`zero`). A map a document and one a
        // value, an entry a row each, were most of a file's count (O125 cold: 0.15 s and 363 MB allocated).
        let docs: Vec<(i64, i64)> = (0..n).map(of).collect();
        let mut keys: Vec<(i64, i64, bool)> = Vec::with_capacity(n);
        let mut values: Vec<(i64, i64)> = Vec::new();
        let mut zero: Vec<(i64, i64)> = Vec::new();
        let mut crossed: FxHashMap<i64, bool> = FxHashMap::default();
        let mut counts: FxHashMap<(i64, i64), (i64, i64)> = FxHashMap::default();
        for (j, col) in self.cols_mut().iter_mut().enumerate() {
            let c = from.map_or(j, |f| f[j]);
            if c >= width {
                continue;
            }
            keys.clear();
            values.clear();
            zero.clear();
            crossed.clear();
            // Whether a document holds a value naming another: its rows mostly come together, so it is looked up
            // once a run of them.
            let mut at: Option<(i64, bool)> = None;
            column(c, &mut |r, v| {
                let (d32, doc) = docs[r];
                keys.push((doc, v, d32 == 0));
                let crosses = !names(v, r);
                match &mut at {
                    Some((d, x)) if *d == doc => *x |= crosses,
                    _ => {
                        if let Some((d, x)) = at.take() {
                            *crossed.entry(d).or_default() |= x;
                        }
                        at = Some((doc, crosses));
                    }
                }
            });
            if let Some((d, x)) = at.take() {
                *crossed.entry(d).or_default() |= x;
            }
            let (mut sqd, mut local) = (0i64, 0i64);
            let mut each = |d: i64, v: i64, small: i64, large: i64| {
                local += 1;
                sqd += large * large;
                if large > 0
                    && let Some(p) = &mut col.pairs
                {
                    p.add(pair(d, v), sign * large);
                }
                values.push((v, small + large));
                if small > 0 {
                    zero.push((v, small));
                }
            };
            if keys.is_sorted() {
                // In order: each (document, value) is a run.
                let mut i = 0;
                while i < keys.len() {
                    let (d, v, _) = keys[i];
                    let (mut small, mut large) = (0i64, 0i64);
                    while i < keys.len() && keys[i].0 == d && keys[i].1 == v {
                        if keys[i].2 { small += 1 } else { large += 1 }
                        i += 1;
                    }
                    each(d, v, small, large);
                }
            } else {
                // Not in order: counted in one map, kept across the columns (a sort cost more than the map).
                counts.clear();
                for &(d, v, z) in &keys {
                    let e = counts.entry((d, v)).or_default();
                    if z { e.0 += 1 } else { e.1 += 1 }
                }
                for (&(d, v), &(small, large)) in &counts {
                    each(d, v, small, large);
                }
            }
            // By value across documents: within one document each value is one pair already.
            if crossed.len() > 1 {
                summed(&mut values);
                summed(&mut zero);
            }
            if let Some(p) = &mut col.pairs {
                for &(v, k) in &zero {
                    p.add(pair(0, v), sign * k);
                }
            }
            col.sqd += sign * sqd;
            Self::zeros(col, &zero, sign);
            col.local_ndv += sign * local;
            col.crossing += sign * crossed.values().filter(|x| **x).count() as i64;
            Self::values(col, &values, sign);
        }
    }

    /// `paired` statistics of a whole relation of `rows` rows, read from its postings rather than its rows: for each
    /// column `c`, `postings(c, visit)` hands each value with the first-column values of the rows holding it (their
    /// documents), and is `false` when it has none. The same as `documents` over the rows, at a hash a distinct value
    /// rather than a row. `None` when a column has no postings.
    pub fn from_postings(columns: Vec<String>, rows: i64, postings: &mut dyn FnMut(usize, &mut dyn FnMut(i64, &[i64])) -> bool) -> Option<Self> {
        let mut s = Self::paired(columns);
        s.rows = rows;
        let mut docs: Vec<(i64, bool, bool)> = Vec::new();
        let mut crossing: rustc_hash::FxHashSet<i64> = Default::default();
        for (c, col) in s.cols_mut().iter_mut().enumerate() {
            crossing.clear();
            let (mut sqd, mut local) = (0i64, 0i64);
            let pairs = col.pairs.as_mut().expect("paired");
            let mut exact = col.exact.take();
            let mut zeros: Vec<(i64, i64)> = Vec::new();
            let found = postings(c, &mut |v, firsts| {
                col.sketch.add(v, firsts.len() as i64);
                if let Some(e) = &mut exact {
                    e.insert(v, firsts.len() as i64);
                    if e.len() > EXACT {
                        exact = None;
                    }
                }
                // Each row's document, whether its first column is a document key (`doc32` 0: summed across
                // documents, as `syntax::measured` groups them), and whether `v` names another document.
                docs.clear();
                docs.extend(firsts.iter().map(|&f| (document_of(f), doc32(f) == 0, !names_doc(v, f))));
                if docs.len() > 1 {
                    docs.sort_unstable();
                }
                let mut small = 0i64;
                let mut at = 0;
                while at < docs.len() {
                    let d = docs[at].0;
                    let end = at + docs[at..].partition_point(|x| x.0 == d);
                    let zero = docs[at..end].iter().filter(|x| x.1).count() as i64;
                    let large = (end - at) as i64 - zero;
                    local += 1;
                    small += zero;
                    sqd += large * large;
                    if large > 0 {
                        pairs.add(pair(d, v), large);
                    }
                    if docs[at..end].iter().any(|x| x.2) {
                        crossing.insert(d);
                    }
                    at = end;
                }
                if small > 0 {
                    zeros.push((v, small));
                    pairs.add(pair(0, v), small);
                }
            });
            if !found {
                return None;
            }
            col.exact = exact;
            col.sqd = sqd;
            Self::zeros(col, &zeros, 1);
            col.local_ndv = local;
            col.crossing = crossing.len() as i64;
        }
        Some(s)
    }

    /// Rows in and out that are not whole documents (a phase export's delta), `plus` then `minus`, each row-major.
    pub fn delta(&mut self, plus: &[i64], minus: &[i64], width: usize) {
        if width == 0 {
            return;
        }
        let each = |sign: i64, f: &mut dyn FnMut(&[i64])| (if sign < 0 { minus } else { plus }).chunks_exact(width).for_each(f);
        self.delta_each((plus.len() / width, minus.len() / width), width, &each);
    }

    /// `delta`, its rows handed by `each(sign, f)`: every row added (`sign` 1) or gone (−1) given to `f`, read where it
    /// lies (`DeltaIds::each`), none copied (0364 R2). `counts`: how many are added and gone.
    pub(in crate::code_rule_dsl) fn delta_each(&mut self, counts: (usize, usize), width: usize, each: &(dyn Fn(i64, &mut dyn FnMut(&[i64])) + Sync)) {
        if width == 0 {
            return;
        }
        self.partial = true;
        // Out before in: a value whose rows are replaced never counts past them, so an exact map is not given up on
        // the way. Each row goes into the sketches as it comes (they are linear: the same counters as its value's
        // count added once), not through a map of the batch's values: two map entries a row a column were most of a
        // kept relation's count (O125's cold run: 13% of its CPU). Only an exact map is a map.
        self.rows += counts.0 as i64 - counts.1 as i64;
        let one = |c: usize, col: &mut Column| {
            for sign in [-1i64, 1] {
                each(sign, &mut |row| {
                    col.sketch.add(row[c], sign);
                    if let Some(p) = &mut col.pairs {
                        p.add(pair(group(row[0]), row[c]), sign);
                    }
                });
                if !col.watched.is_empty() {
                    each(sign, &mut |row| {
                        if let Some(w) = col.watched.get_mut(&row[c]) {
                            *w += sign;
                        }
                    });
                }
                if let Some(exact) = &mut col.exact {
                    let mut over = false;
                    each(sign, &mut |row| {
                        if over {
                            return;
                        }
                        exact.add(row[c], sign);
                        if exact.len() > EXACT {
                            over = true;
                        }
                    });
                    if over || exact.len() > EXACT {
                        col.exact = None;
                    }
                }
            }
        };
        // Many rows: a job a column (decision 0294); each column takes its rows in the same order either way.
        if counts.0 + counts.1 >= 1 << 16 && crate::threads::max() > 1 {
            crate::threads::scope(|jobs| {
                for (c, col) in self.cols_mut().iter_mut().enumerate().take(width) {
                    jobs.spawn("a column's statistics", move |_| one(c, col));
                }
            });
        } else {
            for (c, col) in self.cols_mut().iter_mut().enumerate().take(width) {
                one(c, col);
            }
        }
    }

    /// The document-key rows' counts by value (`zero`, one pair a value) in or out of their group (`Column::zero`).
    pub fn zeros(col: &mut Column, zero: &[(i64, i64)], sign: i64) {
        if zero.is_empty() {
            return;
        }
        let Some(map) = &mut col.zero else {
            col.zero_sq += sign * zero.iter().map(|(_, k)| k * k).sum::<i64>();
            return;
        };
        for &(v, k) in zero {
            let old = map.get(v);
            let new = old + sign * k;
            col.zero_sq += new * new - old * old;
            map.add(v, sign * k);
        }
        if map.len() > EXACT {
            col.zero = None;
        }
    }

    /// Each distinct value's count (`values`, one pair a value) into the sketch and the exact map.
    pub fn values(col: &mut Column, values: &[(i64, i64)], sign: i64) {
        for &(v, k) in values {
            col.sketch.add(v, sign * k);
        }
        if !col.watched.is_empty() {
            for &(v, k) in values {
                if let Some(w) = col.watched.get_mut(&v) {
                    *w += sign * k;
                }
            }
        }
        // Counted in, the map would hold every one of these values at least: past `EXACT`, it is given up unread.
        if sign > 0 && values.len() > EXACT {
            col.exact = None;
        }
        if let Some(exact) = &mut col.exact {
            for &(v, k) in values {
                exact.add(v, sign * k);
            }
            if exact.len() > EXACT {
                col.exact = None;
            }
        }
    }

    /// The statistics, as `syntax::measured` gives them. A column not `held` (no rule reads it; it reads as 0) is
    /// stated as a key, as `columns_source`'s own measure states it.
    pub fn statistics(&self, held: &dyn Fn(&str) -> bool) -> Vec<(String, i64, i64, i64, i64)> {
        let n = self.rows.max(0);
        // As kept and not decoded: its summary, which is these with every column held.
        if self.cols.decoded.get().is_none()
            && let Some(summary) = self.cols.kept.as_ref().and_then(|k| k.summary.as_ref()).filter(|s| s.len() == self.columns.len())
        {
            return self.columns.iter().zip(summary.iter()).map(|(name, &(ndv, sq, sqd))| if !held(name) || n == 0 { (name.clone(), n, n, n, n) } else { (name.clone(), n, ndv, sq, sqd) }).collect();
        }
        self.columns
            .iter()
            .zip(self.cols())
            .map(|(name, col)| {
                if !held(name) || n == 0 {
                    return (name.clone(), n, n, n, n);
                }
                let within = col.sqd + col.zero_sq;
                let (ndv, sq) = if !self.partial && col.crossing == 0 {
                    (col.local_ndv, within)
                } else if let Some(exact) = &col.exact {
                    (exact.len() as i64, exact.values().map(|k| k * k).sum())
                } else {
                    (col.sketch.ndv().round() as i64, col.sketch.sq().round() as i64)
                };
                let ndv = ndv.clamp(1, n);
                let sqd = match (&col.pairs, self.partial) {
                    (_, false) => within,
                    (Some(p), true) => p.sq().round() as i64,
                    (None, true) => sq,
                };
                // Within its bounds: every row once at least (a key), every row one value at most, and no less
                // than within documents.
                let sq = sq.clamp(n.max(sqd.min(n * n)), n.saturating_mul(n));
                (name.clone(), n, ndv, sq, sqd.clamp(n, sq))
            })
            .collect()
    }

    /// Column `c`'s count of each value, while it has few enough to keep them exactly (`EXACT`).
    /// Column `c`'s exact map taken out as (value, count) pairs, to be mapped and put back (`put_exact`).
    pub fn take_exact(&mut self, c: usize) -> Option<Vec<(i64, i64)>> {
        if let Some(k) = self.undecoded_mut() {
            if let Some(pairs) = k.apart_mut().exact.remove(&c) {
                return Some(pairs);
            }
            if !k.has_exact.get(c).copied().unwrap_or(false) {
                return None;
            }
        }
        self.cols_mut().get_mut(c)?.exact.take().map(|e| e.iter().collect())
    }

    /// Column `c`'s exact map from (value, count) pairs.
    pub fn put_exact(&mut self, c: usize, pairs: Vec<(i64, i64)>) {
        if let Some(k) = self.undecoded_mut() {
            if c < k.has_exact.len() {
                k.apart_mut().exact.insert(c, pairs);
            }
            return;
        }
        if let Some(col) = self.cols_mut().get_mut(c) {
            col.exact = Some(pairs.into_iter().collect());
        }
    }

    pub fn exact(&self, c: usize) -> Option<Box<dyn Iterator<Item = (i64, i64)> + '_>> {
        if let Some(k) = self.undecoded() {
            if let Some(pairs) = k.apart().exact.get(&c) {
                return Some(Box::new(pairs.iter().copied()));
            }
            if !k.has_exact.get(c).copied().unwrap_or(false) {
                return None;
            }
        }
        self.cols().get(c)?.exact.as_ref().map(|e| Box::new(e.iter()) as Box<dyn Iterator<Item = (i64, i64)> + '_>)
    }

    /// Column `c`'s count of `v`, where `v` is watched (`watch`).
    pub fn watched(&self, c: usize, v: i64) -> Option<i64> {
        if let Some(k) = self.undecoded() {
            if let Some(pairs) = k.apart().watched.get(&c) {
                return pairs.iter().find(|p| p.0 == v).map(|p| p.1);
            }
            if !k.has_watched.get(c).copied().unwrap_or(false) {
                return None;
            }
        }
        self.cols().get(c)?.watched.get(&v).copied()
    }

    /// Column `c` watches exactly the values `counts` names, each with its count in the rows counted: from then on
    /// each is counted in and out with the rows. A value no longer named is no longer watched.
    pub fn watch(&mut self, c: usize, counts: impl IntoIterator<Item = (i64, i64)>) {
        if let Some(k) = self.undecoded_mut() {
            if c < k.has_watched.len() {
                k.apart_mut().watched.insert(c, counts.into_iter().collect());
            }
            return;
        }
        if let Some(col) = self.cols_mut().get_mut(c) {
            col.watched = counts.into_iter().collect();
        }
    }

    /// Column `c`'s watched values taken out as (value, count) pairs, to be mapped and put back (`watch`).
    pub fn take_watched(&mut self, c: usize) -> Vec<(i64, i64)> {
        if let Some(k) = self.undecoded_mut() {
            if let Some(pairs) = k.apart_mut().watched.remove(&c) {
                return pairs;
            }
            if !k.has_watched.get(c).copied().unwrap_or(false) {
                return Vec::new();
            }
        }
        self.cols_mut().get_mut(c).map(|col| std::mem::take(&mut col.watched).into_iter().collect()).unwrap_or_default()
    }

    /// Its columns, decoded from the bytes they were kept as where they were (`kept`).
    fn cols(&self) -> &[Column] {
        self.cols.decoded.get_or_init(|| {
            let k = self.cols.kept.as_ref().expect("columns made or kept");
            crate::code_rule_dsl::prof::count("statkeep.kept_decoded", 1);
            // Decoded where they lie: `columns_of` reads the bytes through the source's reader, nothing copied out
            // first (decision 0353 step 8).
            let (mut cols, mut at) = (None, k.at);
            k.source.read(&mut |r| {
                cols = Self::columns_of(r, &mut at, k.format, self.columns.len());
                Some(())
            });
            let mut cols = cols.unwrap_or_else(|| vec![Column::default(); self.columns.len()]);
            let apart = k.apart();
            for (c, pairs) in &apart.exact {
                if let Some(col) = cols.get_mut(*c) {
                    col.exact = Some(pairs.iter().copied().collect());
                }
            }
            for (c, pairs) in &apart.watched {
                if let Some(col) = cols.get_mut(*c) {
                    col.watched = pairs.iter().copied().collect();
                }
            }
            cols
        })
    }

    /// Its columns, decoded, to be changed: no longer as kept (a test's patch of them, too).
    pub fn cols_mut(&mut self) -> &mut Vec<Column> {
        self.cols();
        self.cols.kept = None;
        self.cols.decoded.get_mut().expect("decoded")
    }

    /// As kept, its columns not decoded yet.
    fn undecoded(&self) -> Option<&Kept> {
        self.cols.kept.as_ref().filter(|_| self.cols.decoded.get().is_none())
    }

    fn undecoded_mut(&mut self) -> Option<&mut Kept> {
        if self.cols.decoded.get().is_some() {
            return None;
        }
        self.cols.kept.as_mut()
    }

    /// Each column's `(ndv, sq, sqd)` as `statistics` gives them with every column held: kept with the statistics, so
    /// a later run answers `statistics` without decoding them (`kept`).
    pub fn summary(&self) -> Vec<(i64, i64, i64)> {
        self.statistics(&|_| true).into_iter().map(|(_, _, ndv, sq, sqd)| (ndv, sq, sqd)).collect()
    }

    /// Bytes `encode` takes, near enough (the exact maps and sketches dominate).
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        if let Some(k) = self.undecoded() {
            return k.source.len() + 16 * k.apart.get().map_or(0, |a| a.exact.values().chain(a.watched.values()).map(Vec::len).sum::<usize>());
        }
        self.cols().iter().map(|c| 32 + [&c.exact, &c.zero].iter().map(|e| e.as_ref().map_or(0, |e| 16 * e.len())).sum::<usize>() + 16 * c.watched.len() + c.sketch.bytes() + c.pairs.as_ref().map_or(0, Sketch::bytes)).sum::<usize>() + self.columns.iter().map(|c| c.len() + 4).sum::<usize>()
    }

    /// As stored in the wave's blob (0096 §2): a format byte, the columns, the rows, then per column its sums, its
    /// exact map (or none), its sketches and its watched values. Integers little-endian.
    pub fn encode(&self, out: &mut Vec<u8>) {
        let int = |out: &mut Vec<u8>, v: i64| out.extend_from_slice(&v.to_le_bytes());
        let exact = |out: &mut Vec<u8>, e: Option<&Exact>| match e {
            None => int(out, -1),
            Some(e) => {
                int(out, e.len() as i64);
                let mut pairs: Vec<(i64, i64)> = e.iter().collect();
                pairs.sort();
                for (v, k) in pairs {
                    int(out, v);
                    int(out, k);
                }
            }
        };
        out.push(4);
        int(out, self.columns.len() as i64);
        for c in &self.columns {
            int(out, c.len() as i64);
            out.extend_from_slice(c.as_bytes());
        }
        int(out, self.rows);
        out.push(u8::from(self.partial));
        // As kept, not decoded, and every map given apart taken out again: its columns' bytes as they were read.
        if let Some(k) = self.undecoded().filter(|k| k.format == 4 && k.apart().exact.is_empty() && k.apart().watched.is_empty()) {
            let n = out.len();
            out.resize(n + k.source.len().saturating_sub(k.at), 0);
            if k.source.read(&mut |r| r.read_at(k.at, &mut out[n..])).is_some() {
                return;
            }
            out.truncate(n);
        }
        for col in self.cols() {
            int(out, col.sqd);
            int(out, col.zero_sq);
            exact(out, col.zero.as_ref());
            int(out, col.local_ndv);
            int(out, col.crossing);
            exact(out, col.exact.as_ref());
            let s = col.sketch.encode();
            int(out, s.len() as i64);
            out.extend_from_slice(&s);
            match &col.pairs {
                None => int(out, -1),
                Some(p) => {
                    let s = p.encode();
                    int(out, s.len() as i64);
                    out.extend_from_slice(&s);
                }
            }
            let mut watched: Vec<(i64, i64)> = col.watched.iter().map(|(v, n)| (*v, *n)).collect();
            watched.sort_unstable();
            int(out, watched.len() as i64);
            for (v, n) in watched {
                int(out, v);
                int(out, n);
            }
        }
    }

    /// `encode`'s statistics back from `bytes` at `*at`, which moves past them; `None` if they are not.
    pub fn decode(bytes: &[u8], at: &mut usize) -> Option<RelStats> {
        RelStats::decode_in(bytes, at)
    }

    /// `decode` of bytes read where they lie (`ReadAt`): a blob in a kept entry's words is not read into bytes first
    /// (decision 0353 step 8).
    pub fn decode_in<R: ReadAt + ?Sized>(bytes: &R, at: &mut usize) -> Option<RelStats> {
        // Format 3: format 2's `sqd` held the document-key rows' part, summed by document (not to be patched).
        // Format 4: format 3 with each column's watched values; a format 3 column watches none.
        let (format, columns, rows, partial) = Self::header(bytes, at)?;
        let cols = Self::columns_of(bytes, at, format, columns.len())?;
        Some(RelStats { columns, rows, partial, cols: Cols::of(cols) })
    }

    /// `encode`'s statistics in `bytes`, whole, as kept (t33 K4): their columns decoded where first read, `summary`
    /// (each column's, `summary`) answering `statistics` until then. `None` if they are not statistics.
    pub fn kept(bytes: std::sync::Arc<[u8]>, summary: Option<Vec<(i64, i64, i64)>>) -> Option<RelStats> {
        Self::kept_in(std::sync::Arc::new(InMemory(bytes)), summary)
    }

    /// `encode`'s statistics where `source` keeps them, as kept (`kept`): only their header read here, and the
    /// lengths that lead past their columns.
    pub fn kept_in(source: std::sync::Arc<dyn KeptSource>, summary: Option<Vec<(i64, i64, i64)>>) -> Option<RelStats> {
        let mut read = None;
        source.read(&mut |r| {
            let mut at = 0;
            let (format, columns, rows, partial) = Self::header(r, &mut at)?;
            let start = at;
            let (has_exact, has_watched) = Self::walk(r, &mut at, format, columns.len())?;
            (at == r.len()).then_some(())?;
            read = Some((format, columns, rows, partial, start, has_exact, has_watched));
            Some(())
        })?;
        let (format, columns, rows, partial, at, has_exact, has_watched) = read?;
        let kept = Kept { source, at, format, has_exact, has_watched, apart: std::sync::OnceLock::new(), summary: summary.map(Into::into) };
        Some(RelStats { columns, rows, partial, cols: Cols { decoded: std::sync::OnceLock::new(), kept: Some(kept) } })
    }

    /// `encode`'s statistics where `source` keeps them, their header given apart from them (`columns`, `rows`,
    /// `partial`: what `encode` wrote first) and every exact map and watched value apart (`KeptSource::apart`): nothing
    /// of the bytes read until they are decoded or written back.
    pub fn kept_with(source: std::sync::Arc<dyn KeptSource>, columns: Vec<String>, rows: i64, partial: bool, summary: Option<Vec<(i64, i64, i64)>>) -> RelStats {
        // Past `encode`'s format byte, its columns' count and names, its rows and its partial byte.
        let at = 1 + 8 + columns.iter().map(|c| 8 + c.len()).sum::<usize>() + 8 + 1;
        let k = columns.len();
        let kept = Kept { source, at, format: 4, has_exact: vec![false; k], has_watched: vec![false; k], apart: std::sync::OnceLock::new(), summary: summary.map(Into::into) };
        RelStats { columns, rows, partial, cols: Cols { decoded: std::sync::OnceLock::new(), kept: Some(kept) } }
    }

    /// Whether it was patched by rows that are not whole documents (`partial`).
    pub fn partial(&self) -> bool {
        self.partial
    }

    /// `encode`'s first part: its format, columns, rows and whether it is partial.
    fn header<R: ReadAt + ?Sized>(bytes: &R, at: &mut usize) -> Option<(u8, Vec<String>, i64, bool)> {
        let take = |at: &mut usize, out: &mut [u8]| -> Option<()> {
            bytes.read_at(*at, out)?;
            *at += out.len();
            Some(())
        };
        let int = |at: &mut usize| -> Option<i64> {
            let mut b = [0u8; 8];
            take(at, &mut b)?;
            Some(i64::from_le_bytes(b))
        };
        let byte = |at: &mut usize| -> Option<u8> {
            let mut b = [0u8; 1];
            take(at, &mut b)?;
            Some(b[0])
        };
        let format = byte(at)?;
        if format != 3 && format != 4 {
            return None;
        }
        let k = usize::try_from(int(at)?).ok()?;
        let mut columns = Vec::with_capacity(k.min(1024));
        for _ in 0..k {
            let l = usize::try_from(int(at)?).ok()?;
            let mut name = vec![0u8; l.min(bytes.len())];
            (name.len() == l).then_some(())?;
            take(at, &mut name)?;
            columns.push(String::from_utf8(name).ok()?);
        }
        let rows = int(at)?;
        let partial = byte(at)? != 0;
        Some((format, columns, rows, partial))
    }

    /// `k` columns' bytes at `*at` walked over, nothing made: whether each holds an exact map, and watched values.
    fn walk<R: ReadAt + ?Sized>(bytes: &R, at: &mut usize, format: u8, k: usize) -> Option<(Vec<bool>, Vec<bool>)> {
        let skip = |at: &mut usize, n: usize| -> Option<()> {
            (at.checked_add(n)? <= bytes.len()).then_some(())?;
            *at += n;
            Some(())
        };
        let int = |at: &mut usize| -> Option<i64> {
            let mut b = [0u8; 8];
            bytes.read_at(*at, &mut b)?;
            *at += 8;
            Some(i64::from_le_bytes(b))
        };
        let pairs = |at: &mut usize| -> Option<bool> {
            let e = int(at)?;
            if e < 0 {
                return Some(false);
            }
            skip(at, usize::try_from(e).ok()?.checked_mul(16)?)?;
            Some(true)
        };
        let (mut has_exact, mut has_watched) = (Vec::with_capacity(k), Vec::with_capacity(k));
        for _ in 0..k {
            skip(at, 16)?;
            pairs(at)?;
            skip(at, 16)?;
            has_exact.push(pairs(at)?);
            let l = usize::try_from(int(at)?).ok()?;
            skip(at, l)?;
            if let Ok(l) = usize::try_from(int(at)?) {
                skip(at, l)?;
            }
            // Watched values, where there are any: an empty list (the keeper's, which keeps them apart) is none, and
            // asking one of its values decodes nothing.
            has_watched.push(format == 4 && {
                let e = int(at)?;
                skip(at, usize::try_from(e).ok()?.checked_mul(16)?)?;
                e > 0
            });
        }
        Some((has_exact, has_watched))
    }

    /// `k` columns of format `format` decoded from `bytes` at `*at`, which moves past them: read where they lie, a
    /// sketch by position (`Sketch::decode_at`).
    fn columns_of<R: ReadAt + ?Sized>(bytes: &R, at: &mut usize, format: u8, k: usize) -> Option<Vec<Column>> {
        let n = bytes.len();
        let take = |at: &mut usize, l: usize| -> Option<usize> {
            let start = *at;
            (start.checked_add(l)? <= n).then_some(())?;
            *at += l;
            Some(start)
        };
        let int = |at: &mut usize| -> Option<i64> {
            let mut b = [0u8; 8];
            bytes.read_at(*at, &mut b)?;
            *at += 8;
            Some(i64::from_le_bytes(b))
        };
        let byte = |i: usize| -> u8 {
            let mut b = [0u8; 1];
            bytes.read_at(i, &mut b).map_or(0, |()| b[0])
        };
        let exact = |at: &mut usize| -> Option<Option<Exact>> {
            let e = int(at)?;
            if e < 0 {
                return Some(None);
            }
            let mut m = Exact::with_room(usize::try_from(e).ok()?.min(n / 16));
            for _ in 0..e {
                let v = int(at)?;
                m.insert(v, int(at)?);
            }
            Some(Some(m))
        };
        let mut cols = Vec::with_capacity(k);
        for _ in 0..k {
            let (sqd, zero_sq) = (int(at)?, int(at)?);
            let zero = exact(at)?;
            let (local_ndv, crossing) = (int(at)?, int(at)?);
            let exact = exact(at)?;
            let l = usize::try_from(int(at)?).ok()?;
            let sketch = Sketch::decode_at(&byte, take(at, l)?, l)?;
            let pairs = match int(at)? {
                l if l < 0 => None,
                l => {
                    let l = usize::try_from(l).ok()?;
                    Some(Sketch::decode_at(&byte, take(at, l)?, l)?)
                }
            };
            let mut watched = FxHashMap::default();
            if format == 4 {
                for _ in 0..int(at)? {
                    let v = int(at)?;
                    watched.insert(v, int(at)?);
                }
            }
            cols.push(Column { sqd, zero, zero_sq, local_ndv, crossing, exact, sketch, pairs, watched });
        }
        Some(cols)
    }
}

#[cfg(test)]
mod exact_tests {
    use super::Exact;

    /// Decision 0478 R5 (Q338): a column's exact counts are slots in a block, and stay a value → count map under
    /// adds and takes-out in any order, whatever the growth and the shifts after a value taken out.
    #[test]
    fn a_columns_exact_counts_are_slots_in_a_block() {
        let mut e = Exact::default();
        let mut want = std::collections::BTreeMap::<i64, i64>::new();
        let mut x = 12345u64;
        for step in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let v = (x % 300) as i64 - 150;
            let d = if step % 3 == 0 { -1 } else { 1 };
            e.add(v, d);
            let n = want.entry(v).or_default();
            *n += d;
            if *n == 0 {
                want.remove(&v);
            }
            assert_eq!(e.len(), want.len());
        }
        assert!(rerail_blocks::maps::Placed::place(&e.slots).is_some(), "its slots in a block");
        let mut got: Vec<(i64, i64)> = e.iter().collect();
        got.sort_unstable();
        assert_eq!(got, want.into_iter().collect::<Vec<_>>());
        assert_eq!(e.iter().collect::<Exact>(), e);
    }
}

// cratify: `tests` is rerail-setfix's (it mounts it: they measure setfix's column sources)
