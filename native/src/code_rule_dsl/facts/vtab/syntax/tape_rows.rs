//! Relations whose rows are the tape's answers, read where the tape is (decision 0353 step 3): a row per item of a
//! document (a node, or a node or token) that has one, its values read off the tape when the row is read.
//!
//! `core_rr` and `tree` made each as rows for every document of a group, which a rule that read one whole made for
//! the batch with every other relation: `rr_token_named` and the four token neighbours were 40 of the 85 MB of each
//! chunk's batch of sources at the M3's 300-file peak (ab200). Each is now a `TapeRows`, served by one source
//! (`RowsView`): which items have a row (`Marked`, a document's bits and their counts, made once on its first read
//! into its projection's block), the row of each, and the lookups the tape answers.

use super::cells::{Block, Cells};
use super::ranks::Item;
use super::{Batch, Projected, Shape};
use crate::code_rule_dsl::facts::core::NONE;
use crate::code_rule_dsl::facts::vtab::Source;
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

/// Which of `n` items are members, with each member's rank: a word of bits and the members before it, a word per 64
/// items.
pub(super) struct Marked {
    bits: Cells<u64>,
    before: Cells<u32>,
    n: u32,
    count: u32,
}

impl Marked {
    /// The members of items `0..n`, `has` asked of each once.
    fn build(block: &Arc<Block>, n: u32, mut has: impl FnMut(u32) -> bool) -> Marked {
        let words = (n as usize).div_ceil(64).max(1);
        // copy: derived (which of a document's items have a view's row: made once on its first read)
        let bits = Cells::build(block, words, |out: &mut [u64]| {
            for i in 0..n {
                if has(i) {
                    out[(i / 64) as usize] |= 1 << (i % 64);
                }
            }
            words
        })
        .expect("a projection's block holds a view's marks");
        let mut count = 0u32;
        // copy: derived (the members before each word of the marks)
        let before = Cells::build(block, words + 1, |out: &mut [u32]| {
            for (w, b) in bits.iter().enumerate() {
                out[w] = count;
                count += b.count_ones();
            }
            out[words] = count;
            words + 1
        })
        .expect("a projection's block holds a view's marks");
        Marked { bits, before, n, count }
    }

    pub(super) fn count(&self) -> u32 {
        self.count
    }

    /// Whether item `i` is a member.
    pub(super) fn is(&self, i: u32) -> bool {
        i < self.n && self.bits[(i / 64) as usize] >> (i % 64) & 1 == 1
    }

    /// The members before item `i`.
    pub(super) fn rank(&self, i: u32) -> u32 {
        if i >= self.n {
            return self.count;
        }
        let w = (i / 64) as usize;
        self.before[w] + (self.bits[w] & ((1u64 << (i % 64)) - 1)).count_ones()
    }

    /// Member number `r`.
    pub(super) fn select(&self, r: u32) -> Option<u32> {
        if r >= self.count {
            return None;
        }
        // The last word with at most `r` members before it holds member `r`.
        let w = self.before.partition_point(|&b| b <= r) - 1;
        let mut bits = self.bits[w];
        for _ in 0..r - self.before[w] {
            bits &= bits - 1;
        }
        Some(w as u32 * 64 + bits.trailing_zeros())
    }

    /// The first member at or after item `i`.
    pub(super) fn next_at(&self, i: u32) -> Option<u32> {
        self.select(self.rank(i))
    }

    /// The last member before item `i`.
    pub(super) fn prev_before(&self, i: u32) -> Option<u32> {
        self.rank(i).checked_sub(1).and_then(|r| self.select(r))
    }

    /// Every member, in order.
    fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.bits.iter().enumerate().flat_map(|(w, &b)| {
            let mut m = b;
            std::iter::from_fn(move || {
                (m != 0).then(|| {
                    let j = m.trailing_zeros();
                    m &= m - 1;
                    w as u32 * 64 + j
                })
            })
        })
    }
}

/// A relation of the tape's answers: a row per item of a document that has one (a node, or a node or token: its
/// local id), its values read off the tape.
pub(super) trait TapeRows: Send + Sync + 'static {
    /// What a document's rows read beyond its projection, made once per document.
    type Doc: Send + Sync;
    fn name(&self) -> &'static str;
    fn columns(&self) -> &'static [&'static str];
    /// The column holding the row's own item's id, which a lookup addresses.
    fn own(&self) -> usize;
    /// A column holding a node whose rows the tape lists (a parent's children: `under`), if any.
    fn parent(&self) -> Option<usize> {
        None
    }
    fn doc(&self, batch: &Batch, p: &Projected) -> Self::Doc;
    /// The items the rows are of: `0..items`.
    fn items(&self, p: &Projected, d: &Self::Doc) -> u32;
    fn has(&self, p: &Projected, d: &Self::Doc, item: u32) -> bool;
    /// Item `item`'s row (its `row`th in the document, `marked` its rows), every column.
    fn row(&self, p: &Projected, d: &Self::Doc, marked: &Marked, item: u32, row: u32, out: &mut [i64]);
    /// The items whose `parent()` column is node `local`, in order (a member or not: `RowsView` keeps its rows').
    fn under(&self, _p: &Projected, _d: &Self::Doc, _local: u32, _each: &mut dyn FnMut(u32)) {}
}

/// A document's rows of a `TapeRows`.
struct Per<D> {
    doc: D,
    marked: Marked,
}

/// The most columns a `TapeRows` has.
const COLUMNS: usize = 8;

/// A `TapeRows` over a batch's documents: each document's rows at its offset, in item order.
pub(super) struct RowsView<R: TapeRows> {
    batch: Arc<Batch>,
    rows: R,
    per: Box<[OnceLock<Per<R::Doc>>]>,
    offsets: OnceLock<Vec<usize>>,
}

impl<R: TapeRows> RowsView<R> {
    pub(super) fn new(batch: &Arc<Batch>, rows: R) -> Self {
        let per = (0..batch.docs.len()).map(|_| OnceLock::new()).collect();
        RowsView { batch: Arc::clone(batch), rows, per, offsets: OnceLock::new() }
    }

    fn per(&self, doc: usize) -> &Per<R::Doc> {
        self.per[doc].get_or_init(|| {
            let p = &self.batch.docs[doc];
            let d = self.rows.doc(&self.batch, p);
            let marked = Marked::build(p.block(), self.rows.items(p, &d), |i| self.rows.has(p, &d, i));
            Per { doc: d, marked }
        })
    }

    /// The rows' offsets by document: every document's marks made on the first read.
    fn offsets(&self) -> &[usize] {
        self.offsets.get_or_init(|| {
            let mut offsets = Vec::with_capacity(self.per.len() + 1);
            offsets.push(0);
            for doc in 0..self.per.len() {
                offsets.push(offsets.last().unwrap() + self.per(doc).marked.count() as usize);
            }
            offsets
        })
    }

    /// Row `row`'s document and its number there.
    fn locate(&self, row: usize) -> (usize, u32) {
        let offsets = self.offsets();
        let doc = offsets.partition_point(|&o| o <= row) - 1;
        (doc, (row - offsets[doc]) as u32)
    }

    /// Document `doc`'s row `k`, every column.
    fn read(&self, doc: usize, k: u32, out: &mut [i64; COLUMNS]) {
        let per = self.per(doc);
        let item = per.marked.select(k).expect("a row of the view");
        self.rows.row(&self.batch.docs[doc], &per.doc, &per.marked, item, k, out);
    }

    /// The document and item of id `id`: a node's, or a token's, of the batch.
    fn item_of(&self, id: i64) -> Option<(usize, u32)> {
        let doc = self.batch.doc(id >> 32)?;
        let local = self.batch.docs[doc].local_of((id & 0xffff_ffff) as u32)?;
        Some((doc, local))
    }
}

impl<R: TapeRows> Source for RowsView<R> {
    fn python_batch(&self) -> Option<Arc<Batch>> {
        Some(Arc::clone(&self.batch))
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn columns(&self) -> Vec<String> {
        self.rows.columns().iter().map(|c| (*c).to_owned()).collect()
    }
    /// The rows as the run's kept statistics count them, until a read makes them (decision 0494: a batch given its
    /// counts makes no document for a `len`); then checked by them.
    fn len(&self) -> usize {
        let given = self.batch.given_rows(self.rows.name());
        match (given, self.offsets.get()) {
            (Some(n), None) => n,
            _ => {
                let len = *self.offsets().last().unwrap();
                if let Some(n) = given.filter(|&n| n != len) {
                    self.batch.docs.fail(format!("{}: {len} rows read, its kept statistics counted {n} (decision 0494)", self.rows.name()));
                }
                len
            }
        }
    }
    fn failure(&self) -> Option<String> {
        self.batch.docs.failure()
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let (doc, k) = self.locate(row);
        let mut out = [0; COLUMNS];
        self.read(doc, k, &mut out);
        out[column]
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let (doc, k) = self.locate(row);
        let mut all = [0; COLUMNS];
        self.read(doc, k, &mut all);
        for (o, &c) in out.iter_mut().zip(columns) {
            *o = all[c];
        }
    }
    /// Document after document, each one's rows from its marks in order: no row's place searched.
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        let (mut all, mut row) = ([0; COLUMNS], vec![0; columns.len()]);
        let offsets = self.offsets();
        for doc in 0..self.per.len() {
            let (per, p) = (self.per(doc), &self.batch.docs[doc]);
            for (k, item) in per.marked.iter().enumerate() {
                self.rows.row(p, &per.doc, &per.marked, item, k as u32, &mut all);
                for (o, &c) in row.iter_mut().zip(columns) {
                    *o = all[c];
                }
                each(offsets[doc] + k, &row);
            }
        }
    }
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<Vec<usize>> {
        let offsets = self.offsets();
        Some(docs.iter().filter_map(|&k| self.batch.docs.position(k)).flat_map(|i| offsets[i]..offsets[i + 1]).collect())
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let per = if column == self.rows.own() {
            1
        } else if Some(column) == self.rows.parent() {
            4
        } else {
            (self.len() / 64).max(1)
        };
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        column == self.rows.own() || Some(column) == self.rows.parent()
    }
    /// By the row's own id, its item's mark; by a parent's, the items the tape lists under it.
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let own = column == self.rows.own();
        if !own && Some(column) != self.rows.parent() {
            return None;
        }
        let offsets = self.offsets();
        let mut rows = Vec::new();
        for &v in values {
            let Some((doc, local)) = self.item_of(v) else { continue };
            let (per, p) = (self.per(doc), &self.batch.docs[doc]);
            if own {
                if per.marked.is(local) {
                    rows.push(offsets[doc] + per.marked.rank(local) as usize);
                }
            } else if local < p.len() {
                self.rows.under(p, &per.doc, local, &mut |item| {
                    if per.marked.is(item) {
                        rows.push(offsets[doc] + per.marked.rank(item) as usize);
                    }
                });
            }
        }
        rows.sort_unstable();
        rows.dedup();
        Some(rows)
    }
}

/// The token relations' names (`TokenNeighbour`), core.rerail's: `first_token(n, t)`, `last_token(n, t)`,
/// `token_before(x, t)`, `token_after(x, t)`.
pub const TOKENS: [&str; 4] = ["first_token", "last_token", "token_before", "token_after"];

/// `rr_token_named(p, i, c, start, end)` (`NamedChild`).
pub const NAMED: &str = "rr_token_named";

/// The relations `RowsView` serves, a batch of `shape`'s: none of them made as rows.
pub(super) fn served(shape: Shape) -> impl Iterator<Item = &'static str> {
    std::iter::once(NAMED).chain(TOKENS.into_iter().filter(move |_| shape == Shape::Core))
}

/// The views `served` names over `batch`.
pub(super) fn views(batch: &Arc<Batch>) -> Vec<(&'static str, Arc<dyn Source>)> {
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = vec![(NAMED, Arc::new(RowsView::new(batch, NamedChild)))];
    if batch.shape == Shape::Core {
        for which in [Neighbour::First, Neighbour::Last, Neighbour::Before, Neighbour::After] {
            out.push((TOKENS[which as usize], Arc::new(RowsView::new(batch, TokenNeighbour(which)))));
        }
    }
    out
}

/// Which token of a node or token (`TokenNeighbour`).
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Neighbour {
    /// The first token inside it (a token's, itself).
    First,
    /// The last token inside it.
    Last,
    /// The last token before it.
    Before,
    /// The first token after it.
    After,
}

/// core.rerail's tokens of a node or token (ESLint's token store): its leaves, named or anonymous, other than comments,
/// in source order. Its rows are of the projection's nodes and tokens (`Projected::locs` up to the spans). The tokens
/// are a set of the document's tape nodes (`Marked`): a node's are those its subtree holds, the one before it the last
/// before it on the tape, the one after it the first past its subtree.
pub(super) struct TokenNeighbour(pub(super) Neighbour);

impl TokenNeighbour {
    /// Item `item`'s tape node: a node's, or a token's.
    fn tape(p: &Projected, item: u32) -> u32 {
        match item.checked_sub(p.len()) {
            None => p.node(item),
            Some(t) => p.ranks().select(Item::Token, t).expect("a numbered token"),
        }
    }

    /// The token of `item` this relation names, as a tape node.
    fn token(&self, p: &Projected, tokens: &Marked, item: u32) -> Option<u32> {
        let x = Self::tape(p, item);
        let end = p.facts.subtree_end(x);
        match self.0 {
            Neighbour::First => tokens.next_at(x).filter(|&t| t < end),
            Neighbour::Last => tokens.prev_before(end).filter(|&t| t >= x),
            Neighbour::Before => tokens.prev_before(x),
            Neighbour::After => tokens.next_at(end),
        }
    }
}

impl TapeRows for TokenNeighbour {
    /// The document's tokens, a set of its tape nodes: `None` for a document of no dialect (no node, no token).
    type Doc = Option<Marked>;
    fn name(&self) -> &'static str {
        TOKENS[self.0 as usize]
    }
    fn columns(&self) -> &'static [&'static str] {
        match self.0 {
            Neighbour::First | Neighbour::Last => &["n", "t"],
            Neighbour::Before | Neighbour::After => &["x", "t"],
        }
    }
    fn own(&self) -> usize {
        0
    }
    fn doc(&self, batch: &Batch, p: &Projected) -> Option<Marked> {
        let dialect = batch.dialects.get(p.tag)?;
        let (f, ranks) = (&*p.facts, p.ranks());
        Some(Marked::build(p.block(), f.len() as u32, |x| {
            ranks.is(Item::Token, x) || ranks.is(Item::Node, x) && f.subtree_end(x) == x + 1 && f.end(x) > f.start(x) && !super::core_rr::is_comment(dialect.name(f, x))
        }))
    }
    fn items(&self, p: &Projected, d: &Option<Marked>) -> u32 {
        if d.is_some() { p.len() + p.ranks().count(Item::Token) } else { 0 }
    }
    fn has(&self, p: &Projected, d: &Option<Marked>, item: u32) -> bool {
        d.as_ref().is_some_and(|tokens| self.token(p, tokens, item).is_some())
    }
    fn row(&self, p: &Projected, d: &Option<Marked>, _: &Marked, item: u32, _: u32, out: &mut [i64]) {
        let tokens = d.as_ref().expect("a row's document has tokens");
        let t = self.token(p, tokens, item).expect("a row's token");
        let ranks = p.ranks();
        let local = match ranks.local(Item::Node, t) {
            Some(l) => l,
            None => p.len() + ranks.rank(Item::Token, t),
        };
        out[0] = p.global(item);
        out[1] = p.global(local);
    }
}

/// `rr_token_named(p, i, c, start, end)`: node `p`'s child `c`, its `i`th on the tape (anonymous ones counted), a named
/// node of the projection, and its byte span: the named part of `token`'s answer (decision 0174), whose anonymous
/// part (`rr_token_anon`) holds their texts. A row per node whose tape parent is a node of the projection.
pub(super) struct NamedChild;

impl TapeRows for NamedChild {
    /// Each row's `i`, made on the first read of one: the tape holds no child's place among its siblings, and a search
    /// along them a row made a wide node's rows quadratic.
    type Doc = OnceLock<Cells<u32>>;
    fn name(&self) -> &'static str {
        NAMED
    }
    fn columns(&self) -> &'static [&'static str] {
        &["p", "i", "c", "start", "end_"]
    }
    fn own(&self) -> usize {
        2
    }
    fn parent(&self) -> Option<usize> {
        Some(0)
    }
    fn doc(&self, _: &Batch, _: &Projected) -> Self::Doc {
        OnceLock::new()
    }
    fn items(&self, p: &Projected, _: &Self::Doc) -> u32 {
        p.len()
    }
    fn has(&self, p: &Projected, _: &Self::Doc, item: u32) -> bool {
        let x = p.node(item);
        p.facts.named(x) && p.facts.parent(x).is_some_and(|q| p.local_of_node(q) != NONE)
    }
    fn row(&self, p: &Projected, d: &Self::Doc, marked: &Marked, item: u32, row: u32, out: &mut [i64]) {
        let x = p.node(item);
        let q = p.facts.parent(x).expect("a row's node has a parent");
        let idx = d.get_or_init(|| {
            let f = &*p.facts;
            // copy: derived (each row's place among its parent's children on the tape, computed once here)
            Cells::build(p.block(), marked.count().max(1) as usize, |out| {
                for (_, xq) in p.nodes() {
                    let mut i = 0;
                    f.each_child(xq, |c| {
                        let l = p.local_of_node(c);
                        if l != NONE && marked.is(l) {
                            out[marked.rank(l) as usize] = i;
                        }
                        i += 1;
                    });
                }
                marked.count().max(1) as usize
            })
            .expect("a projection's block holds a view's places")
        });
        out[0] = p.global(p.local_of_node(q));
        out[1] = i64::from(idx[row as usize]);
        out[2] = p.global(item);
        out[3] = i64::from(p.facts.start(x));
        out[4] = i64::from(p.facts.end(x));
    }
    fn under(&self, p: &Projected, _: &Self::Doc, local: u32, each: &mut dyn FnMut(u32)) {
        p.facts.each_child(p.node(local), |c| {
            let l = p.local_of_node(c);
            if l != NONE {
                each(l);
            }
        });
    }
}
