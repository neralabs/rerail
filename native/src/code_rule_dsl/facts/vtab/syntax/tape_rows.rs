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
    /// The members of items `0..n`, each given to `fill`'s callback (in any order, at most once each).
    fn build(block: &Arc<Block>, n: u32, fill: impl FnOnce(&mut dyn FnMut(u32))) -> Marked {
        let words = (n as usize).div_ceil(64).max(1);
        // copy: derived (which of a document's items have a view's row: made once on its first read)
        let bits = Cells::build(block, words, |out: &mut [u64]| {
            fill(&mut |i| {
                debug_assert!(i < n, "a member past the items");
                out[(i / 64) as usize] |= 1 << (i % 64);
            });
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

/// One of a batch's documents, as a `TapeRows` reads it: its projection made only where a relation asks for it
/// (`p`), so a relation of what the documents are given (`DocRow`) makes none.
pub(super) struct At<'a> {
    pub(super) batch: &'a Batch,
    pub(super) doc: usize,
}

impl At<'_> {
    /// Its projection: its document made, if it is not.
    pub(super) fn p(&self) -> &Projected {
        &self.batch.docs[self.doc]
    }
}

/// A relation of the tape's answers, or of what a batch's documents are given: a row per item of a document that has
/// one (a node, a node or token: its local id; a document itself), its values read where they are.
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
    fn doc(&self, at: &At) -> Self::Doc;
    /// The block a document's marks are kept in: its projection's.
    fn block(&self, at: &At) -> Arc<Block> {
        Arc::clone(at.p().block())
    }
    /// The items the rows are of: `0..items`.
    fn items(&self, at: &At, d: &Self::Doc) -> u32;
    /// Each item with a row, once, in any order: a walk of the tape that finds them, not a test of each item (a test
    /// that reads a node's parent searches the tape for it, on format 15).
    fn members(&self, at: &At, d: &Self::Doc, each: &mut dyn FnMut(u32));
    /// Item `item`'s row (its `row`th in the document, `marked` its rows), every column.
    fn row(&self, at: &At, d: &Self::Doc, marked: &Marked, item: u32, row: u32, out: &mut [i64]);
    /// The items whose `parent()` column is node `local`, in order (a member or not: `RowsView` keeps its rows').
    fn under(&self, _at: &At, _local: u32, _each: &mut dyn FnMut(u32)) {}
    /// Whether a lookup on `column` finds its items (`find`).
    fn keyed(&self, column: usize) -> bool {
        column == self.own() || Some(column) == self.parent()
    }
    /// The items (each with its document) a lookup of `value` on a `keyed` column may find, a member or not: by the
    /// row's own id, its item; by a parent's, the items under it.
    fn find(&self, batch: &Batch, column: usize, value: i64, each: &mut dyn FnMut(usize, u32)) {
        let Some(doc) = batch.doc(value >> 32) else { return };
        let at = At { batch, doc };
        let Some(local) = at.p().local_of((value & 0xffff_ffff) as u32) else { return };
        if column == self.own() {
            each(doc, local);
        } else if local < at.p().len() {
            self.under(&at, local, &mut |item| each(doc, item));
        }
    }
    /// Its statistics without its rows read (as `syntax::measured` gives them), where it knows them.
    fn statistics(&self, _batch: &Batch) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        None
    }
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

    fn at(&self, doc: usize) -> At<'_> {
        At { batch: &self.batch, doc }
    }

    fn per(&self, doc: usize) -> &Per<R::Doc> {
        self.per[doc].get_or_init(|| {
            let at = self.at(doc);
            let d = self.rows.doc(&at);
            let marked = Marked::build(&self.rows.block(&at), self.rows.items(&at, &d), |each| self.rows.members(&at, &d, each));
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
        self.rows.row(&self.at(doc), &per.doc, &per.marked, item, k, out);
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
    /// The rows as the run's kept statistics count them, or its own statistics do, until a read makes them (decision
    /// 0494: a batch given its counts makes no document for a `len`); then checked by the kept ones.
    fn len(&self) -> usize {
        let given = self.batch.given_rows(self.rows.name());
        if self.offsets.get().is_none()
            && let Some(n) = given.or_else(|| self.rows.statistics(&self.batch).map(|s| s[0].1 as usize))
        {
            return n;
        }
        let len = *self.offsets().last().unwrap();
        if let Some(n) = given.filter(|&n| n != len) {
            self.batch.docs.fail(format!("{}: {len} rows read, its kept statistics counted {n} (decision 0494)", self.rows.name()));
        }
        len
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.rows.statistics(&self.batch)
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
            let (per, at) = (self.per(doc), self.at(doc));
            for (k, item) in per.marked.iter().enumerate() {
                self.rows.row(&at, &per.doc, &per.marked, item, k as u32, &mut all);
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
        } else if self.rows.keyed(column) {
            4
        } else {
            (self.len() / 64).max(1)
        };
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        self.rows.keyed(column)
    }
    /// The items `find` gives that have rows, each its row.
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        if !self.rows.keyed(column) {
            return None;
        }
        let offsets = self.offsets();
        let mut rows = Vec::new();
        for &v in values {
            self.rows.find(&self.batch, column, v, &mut |doc, item| {
                let marked = &self.per(doc).marked;
                if marked.is(item) {
                    rows.push(offsets[doc] + marked.rank(item) as usize);
                }
            });
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

/// `doc(d, path, module)` (`DocRow`).
pub const DOC: &str = "doc";

/// core.rerail's `structural_before(n, s)`, `structural_after(n, s)` (`Gap`).
pub const STRUCTURAL: [&str; 2] = ["structural_before", "structural_after"];

/// `visit_index(n, i)` (`VisitIndex`).
pub const VISIT: &str = "visit_index";

/// `tree_digest(n, s)` (`TreeDigest`).
pub const DIGEST: &str = "tree_digest";

/// The relations `RowsView` serves, a batch of `shape`'s: none of them made as rows.
pub(super) fn served(shape: Shape) -> impl Iterator<Item = &'static str> {
    [DOC, NAMED, VISIT].into_iter().chain(TOKENS.into_iter().chain(STRUCTURAL).chain([DIGEST]).filter(move |_| shape == Shape::Core))
}

/// The views `served` names over `batch`.
pub(super) fn views(batch: &Arc<Batch>) -> Vec<(&'static str, Arc<dyn Source>)> {
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = vec![
        (DOC, Arc::new(RowsView::new(batch, DocRow::new(batch)))),
        (NAMED, Arc::new(RowsView::new(batch, NamedChild))),
        (VISIT, Arc::new(RowsView::new(batch, VisitIndex))),
    ];
    if batch.shape == Shape::Core {
        out.push((DIGEST, Arc::new(RowsView::new(batch, TreeDigest))));
    }
    if batch.shape == Shape::Core {
        for which in [Neighbour::First, Neighbour::Last, Neighbour::Before, Neighbour::After] {
            out.push((TOKENS[which as usize], Arc::new(RowsView::new(batch, TokenNeighbour(which)))));
        }
        for side in [Side::Before, Side::After] {
            out.push((STRUCTURAL[side as usize], Arc::new(RowsView::new(batch, Gap(side)))));
        }
    }
    out
}

/// The strings the views of `batch` that its projection reads hold (each interned when read), of every document: a
/// text test's scan tests the batch's strings (`Batch::vocabulary`), which a relation made as rows put in its strings
/// as it was made. Without them, a scan missed what a probe found: `match(…, g)` over `structural_before(n, g)`
/// found 463 or 467 violations by the plan the run chose (ab218).
pub(super) fn strings(batch: &Batch, wants: &dyn Fn(&str) -> bool, each: &mut dyn FnMut(&str)) {
    let doc = wants(DOC);
    let gaps: Vec<Gap> = [Side::Before, Side::After].into_iter().filter(|&s| batch.shape == Shape::Core && wants(STRUCTURAL[s as usize])).map(Gap).collect();
    for d in 0..batch.docs.len() {
        if doc {
            let docs = &batch.docs;
            each(docs.path(d));
            each(&super::doc_module(docs.path(d), docs.module(d), batch.shape == Shape::Core));
        }
        if gaps.is_empty() {
            continue;
        }
        let p = At { batch, doc: d };
        let p = p.p();
        let source = p.facts.source.as_str();
        for g in &gaps {
            g.walk(p, &mut |_, a, b| each(if a < b { &source[a as usize..b as usize] } else { "" }));
        }
    }
    if batch.shape == Shape::Core && wants(DIGEST) {
        for d in 0..batch.docs.len() {
            TreeDigest::hexes(&At { batch, doc: d }, &mut |_, h| each(h));
        }
    }
}

/// Which side of a node its gap is (`Gap`).
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Side {
    Before,
    After,
}

/// core.rerail's `structural_before(n, s)` and `structural_after(n, s)`: the source between node `n` and its
/// neighbour on that side (the sibling before or after it on the tape, anonymous ones too, else its parent's edge); a
/// root's, between it and the source's edge. A row per node of the projection that is the tape's root or a child of
/// one of its nodes. `n` is the tape's; `s` is the gap's text interned, once per row on the document's first read
/// (a derived fact: the tape holds the gap, not its atom). Made as rows they were 27 MB of each chunk's block of
/// sources at the M3's 300-file peak (ab217), `n` and `s` a row each.
pub(super) struct Gap(pub(super) Side);

impl Gap {
    /// Each member's gap, `(local, from, to)` in bytes of the source, in the tape's order of their parents.
    fn walk(&self, p: &Projected, each: &mut dyn FnMut(u32, u32, u32)) {
        let f = &*p.facts;
        if f.len() > 0 {
            let root = p.local_of_node(0);
            if root != NONE {
                match self.0 {
                    Side::Before => each(root, 0, f.start(0)),
                    Side::After => each(root, f.end(0), f.source.len() as u32),
                }
            }
        }
        for (_, q) in p.nodes() {
            let children = f.children(q);
            for (k, &c) in children.iter().enumerate() {
                let l = p.local_of_node(c);
                if l == NONE {
                    continue;
                }
                match self.0 {
                    Side::Before => each(l, if k == 0 { f.start(q) } else { f.end(children[k - 1]) }, f.start(c)),
                    Side::After => each(l, f.end(c), children.get(k + 1).map_or(f.end(q), |&d| f.start(d))),
                }
            }
        }
    }
}

impl TapeRows for Gap {
    /// Each row's gap's atom, in row order: made on the document's first read of a row, in one walk.
    type Doc = OnceLock<Cells<u32>>;
    fn name(&self) -> &'static str {
        STRUCTURAL[self.0 as usize]
    }
    fn columns(&self) -> &'static [&'static str] {
        &["n", "s"]
    }
    fn own(&self) -> usize {
        0
    }
    fn doc(&self, _: &At) -> Self::Doc {
        OnceLock::new()
    }
    fn items(&self, at: &At, _: &Self::Doc) -> u32 {
        at.p().len()
    }
    fn members(&self, at: &At, _: &Self::Doc, each: &mut dyn FnMut(u32)) {
        self.walk(at.p(), &mut |l, _, _| each(l));
    }
    fn row(&self, at: &At, d: &Self::Doc, marked: &Marked, item: u32, row: u32, out: &mut [i64]) {
        let p = at.p();
        let atoms = d.get_or_init(|| {
            let source = p.facts.source.as_str();
            let mut texts: Vec<&str> = vec![""; marked.count() as usize];
            self.walk(p, &mut |l, a, b| {
                if a < b {
                    texts[marked.rank(l) as usize] = &source[a as usize..b as usize];
                }
            });
            let syms = at.batch.atoms.intern_all(&texts).expect("a document's gaps interned");
            // copy: derived (each row's gap's text interned: the tape holds the gap, not its atom)
            Cells::build(p.block(), syms.len().max(1), |out: &mut [u32]| {
                out[..syms.len()].copy_from_slice(&syms);
                syms.len().max(1)
            })
            .expect("a projection's block holds a view's atoms")
        });
        out[0] = p.global(item);
        out[1] = i64::from(atoms[row as usize]);
    }
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
    fn doc(&self, at: &At) -> Option<Marked> {
        let (batch, p) = (at.batch, at.p());
        let dialect = batch.dialects.get(p.tag)?;
        let (f, ranks) = (&*p.facts, p.ranks());
        Some(Marked::build(p.block(), f.len() as u32, |each| {
            for x in 0..f.len() as u32 {
                if ranks.is(Item::Token, x) || ranks.is(Item::Node, x) && f.subtree_end(x) == x + 1 && f.end(x) > f.start(x) && !super::core_rr::is_comment(dialect.name(f, x)) {
                    each(x);
                }
            }
        }))
    }
    fn items(&self, at: &At, d: &Option<Marked>) -> u32 {
        let p = at.p();
        if d.is_some() { p.len() + p.ranks().count(Item::Token) } else { 0 }
    }
    fn members(&self, at: &At, d: &Option<Marked>, each: &mut dyn FnMut(u32)) {
        let Some(tokens) = d else { return };
        let p = at.p();
        for item in 0..self.items(at, d) {
            if self.token(p, tokens, item).is_some() {
                each(item);
            }
        }
    }
    fn row(&self, at: &At, d: &Option<Marked>, _: &Marked, item: u32, _: u32, out: &mut [i64]) {
        let p = at.p();
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
    /// Each row's parent (its local id) and `i`, made on the first read of a row, in one walk of the nodes' children:
    /// the tape holds no child's place among its siblings, and a node's parent is a search back on format 15.
    type Doc = OnceLock<(Cells<u32>, Cells<u32>)>;
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
    fn doc(&self, _: &At) -> Self::Doc {
        OnceLock::new()
    }
    fn items(&self, at: &At, _: &Self::Doc) -> u32 {
        at.p().len()
    }
    /// The named children of each node of the projection that are nodes of it.
    fn members(&self, at: &At, _: &Self::Doc, each: &mut dyn FnMut(u32)) {
        let p = at.p();
        for (_, q) in p.nodes() {
            p.facts.each_child(q, |c| {
                let l = p.local_of_node(c);
                if l != NONE && p.facts.named(c) {
                    each(l);
                }
            });
        }
    }
    fn row(&self, at: &At, d: &Self::Doc, marked: &Marked, item: u32, row: u32, out: &mut [i64]) {
        let p = at.p();
        let (parents, places) = d.get_or_init(|| {
            let n = marked.count().max(1) as usize;
            let walk = |out: &mut [u32], of: &dyn Fn(u32, u32) -> u32| {
                for (lq, q) in p.nodes() {
                    let mut i = 0;
                    p.facts.each_child(q, |c| {
                        let l = p.local_of_node(c);
                        if l != NONE && marked.is(l) {
                            out[marked.rank(l) as usize] = of(lq, i);
                        }
                        i += 1;
                    });
                }
                n
            };
            // copy: derived (each row's parent and place among its children on the tape, computed once here)
            let parents = Cells::build(p.block(), n, |out| walk(out, &|lq, _| lq)).expect("a projection's block holds a view's parents");
            // copy: derived (as above)
            let places = Cells::build(p.block(), n, |out| walk(out, &|_, i| i)).expect("a projection's block holds a view's places");
            (parents, places)
        });
        let x = p.node(item);
        out[0] = p.global(parents[row as usize]);
        out[1] = i64::from(places[row as usize]);
        out[2] = p.global(item);
        out[3] = i64::from(p.facts.start(x));
        out[4] = i64::from(p.facts.end(x));
    }
    fn under(&self, at: &At, local: u32, each: &mut dyn FnMut(u32)) {
        let p = at.p();
        p.facts.each_child(p.node(local), |c| {
            let l = p.local_of_node(c);
            if l != NONE {
                each(l);
            }
        });
    }
}

/// core.rerail's and T's `doc(d, path, module)`: a row a file of the batch (a tape value, decision 0431, is no file's),
/// its key, its path and its module (`syntax::doc_module`), what its documents are given: none made for it. Made as
/// rows, a scan of it made every document's every relation (ab203: `exists_25`'s `doc($d, $p, _)` led its text
/// test's probe, and made the chunk's 260 documents whole, 61 MB of the block of sources at the M3's peak).
pub(super) struct DocRow {
    /// Its documents' marks: its own, as it makes no projection.
    block: Arc<Block>,
}

impl DocRow {
    fn new(batch: &Batch) -> Self {
        // A word of marks and two of their ranks a document, with room to spare.
        DocRow { block: super::cells::block(64 * batch.docs.len() + 4096) }
    }
}

impl TapeRows for DocRow {
    /// Its path's and its module's atoms.
    type Doc = [i64; 2];
    fn name(&self) -> &'static str {
        DOC
    }
    fn columns(&self) -> &'static [&'static str] {
        &["d", "path", "module"]
    }
    fn own(&self) -> usize {
        0
    }
    fn block(&self, _: &At) -> Arc<Block> {
        Arc::clone(&self.block)
    }
    fn doc(&self, at: &At) -> [i64; 2] {
        let docs = &at.batch.docs;
        let module = super::doc_module(docs.path(at.doc), docs.module(at.doc), at.batch.shape == Shape::Core);
        // copy: derived (a document's path and module, interned: the atoms `make_rows` gave them)
        let atoms = at.batch.atoms.intern_all(&[docs.path(at.doc), module.as_str()]).expect("a document's path and module interned");
        [i64::from(atoms[0]), i64::from(atoms[1])]
    }
    fn items(&self, _: &At, _: &[i64; 2]) -> u32 {
        1
    }
    fn members(&self, at: &At, _: &[i64; 2], each: &mut dyn FnMut(u32)) {
        if !at.batch.docs.guest(at.doc).is_some_and(|g| matches!(g.made, super::Made::Tape { .. })) {
            each(0);
        }
    }
    fn row(&self, at: &At, d: &[i64; 2], _: &Marked, _: u32, _: u32, out: &mut [i64]) {
        out[0] = i64::from(at.batch.docs.key(at.doc));
        out[1] = d[0];
        out[2] = d[1];
    }
    fn keyed(&self, column: usize) -> bool {
        column < 3
    }
    /// By its key, its document; by its path or module, the documents the batch's interned paths and modules name.
    fn find(&self, batch: &Batch, column: usize, value: i64, each: &mut dyn FnMut(usize, u32)) {
        let keys = match column {
            0 => u32::try_from(value).ok().into_iter().collect(),
            1 => batch.keys_of_paths(&[value]).unwrap_or_default(),
            _ => batch.keys_of_modules(&[value]).unwrap_or_default(),
        };
        for k in keys {
            if let Some(doc) = batch.docs.position(k) {
                each(doc, 0);
            }
        }
    }
    fn statistics(&self, batch: &Batch) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        batch.doc_statistics()
    }
}

/// `visit_index(n, i)`: node `n`'s place in CPython's `ast` visit order, from 0 per document, of a file's own Python
/// documents (`tree::visit_order`). The order is a walk of the tape that the tape holds no index of: each node's place
/// is kept, 4 bytes a node of the projection, made on the document's first read (`// copy: derived`). Made as rows it
/// was 13.7 MB of each chunk's block of sources at the M3's 300-file peak (ab200), `n` and `i` a row each.
pub(super) struct VisitIndex;

impl TapeRows for VisitIndex {
    /// Each node's place (by its local id), `NONE` for a node with none; `None` for a document with no order.
    type Doc = Option<Cells<u32>>;
    fn name(&self) -> &'static str {
        VISIT
    }
    fn columns(&self) -> &'static [&'static str] {
        &["n", "i"]
    }
    fn own(&self) -> usize {
        0
    }
    fn doc(&self, at: &At) -> Option<Cells<u32>> {
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        if at.batch.docs.guest(at.doc).is_some() {
            return None;
        }
        let p = at.p();
        if python != Some(p.tag) {
            return None;
        }
        let dialect = at.batch.dialects.get(p.tag)?;
        let n = p.len() as usize;
        // copy: derived (each node's place in CPython's visit order, a walk the tape holds no index of)
        Some(
            Cells::build(p.block(), n.max(1), |out: &mut [u32]| {
                out.fill(NONE);
                super::tree::visit_order(p, dialect, &mut |l, i| out[l as usize] = i);
                n.max(1)
            })
            .expect("a projection's block holds a view's places"),
        )
    }
    fn items(&self, at: &At, d: &Self::Doc) -> u32 {
        if d.is_some() { at.p().len() } else { 0 }
    }
    fn members(&self, _: &At, d: &Self::Doc, each: &mut dyn FnMut(u32)) {
        if let Some(places) = d {
            places.iter().enumerate().filter(|&(_, &i)| i != NONE).for_each(|(l, _)| each(l as u32));
        }
    }
    fn row(&self, at: &At, d: &Self::Doc, _: &Marked, item: u32, _: u32, out: &mut [i64]) {
        out[0] = at.p().global(item);
        out[1] = i64::from(d.as_ref().expect("a row's document has an order")[item as usize]);
    }
}

/// `tree_digest(n, h)` (`@tree_digest(n)`): a structural hash of each node's subtree (`core_rr::digests`), of the
/// nodes of a document with a grammar that parsed. A hash is of the whole subtree, which the tape holds no digest of:
/// each node's is kept as its atom, 4 bytes a node, interned on the document's first read (`// copy: derived`). Made
/// as rows it was 6.8 MB of each chunk's block of sources at the M3's 300-file peak (ab200).
pub(super) struct TreeDigest;

impl TreeDigest {
    /// Each node of the projection of `at` (by local id, in order) with its digest's symbol; none for a document
    /// with no grammar, or withheld.
    fn hexes(at: &At, each: &mut dyn FnMut(u32, &str)) {
        let Some(dialect) = at.batch.dialects.get(at.batch.docs.tag(at.doc)) else { return };
        if at.batch.docs.withheld(at.doc) {
            return;
        }
        let p = at.p();
        let (hash, _) = super::core_rr::digests(&p.facts, dialect, true, false);
        let mut text = [0u8; 64];
        for (local, id, _) in p.node_ids() {
            each(local, super::core_rr::hex_into(&hash[id as usize], &mut text));
        }
    }
}

impl TapeRows for TreeDigest {
    /// Each node's digest's atom, by local id: made on the document's first read of a row.
    type Doc = OnceLock<Cells<u32>>;
    fn name(&self) -> &'static str {
        DIGEST
    }
    fn columns(&self) -> &'static [&'static str] {
        &["n", "s"]
    }
    fn own(&self) -> usize {
        0
    }
    fn doc(&self, _: &At) -> Self::Doc {
        OnceLock::new()
    }
    fn items(&self, at: &At, _: &Self::Doc) -> u32 {
        at.p().len()
    }
    fn members(&self, at: &At, _: &Self::Doc, each: &mut dyn FnMut(u32)) {
        if at.batch.dialects.get(at.batch.docs.tag(at.doc)).is_some() && !at.batch.docs.withheld(at.doc) {
            (0..at.p().len()).for_each(each);
        }
    }
    fn row(&self, at: &At, d: &Self::Doc, _: &Marked, item: u32, _: u32, out: &mut [i64]) {
        let p = at.p();
        let atoms = d.get_or_init(|| {
            // Every node's 64 digits in one buffer, each a slice of it.
            let mut digits: Vec<u8> = vec![b'0'; 64 * p.len() as usize];
            Self::hexes(at, &mut |l, h| digits[64 * l as usize..64 * (l as usize + 1)].copy_from_slice(h.as_bytes()));
            let texts: Vec<&str> = digits.chunks_exact(64).map(|d| std::str::from_utf8(d).expect("hex digits")).collect();
            let syms = at.batch.atoms.intern_all(&texts).expect("a document's digests interned");
            // copy: derived (each node's subtree's digest, interned: the tape holds the subtree, not its hash)
            Cells::build(p.block(), syms.len().max(1), |out: &mut [u32]| {
                out[..syms.len()].copy_from_slice(&syms);
                syms.len().max(1)
            })
            .expect("a projection's block holds a view's atoms")
        });
        out[0] = p.global(item);
        out[1] = i64::from(atoms[item as usize]);
    }
}
