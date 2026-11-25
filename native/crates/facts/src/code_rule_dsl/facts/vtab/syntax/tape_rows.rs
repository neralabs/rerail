//! Relations whose rows are the tape's answers, read where the tape is (decision 0353 step 3): a row per item of a
//! document (a node, or a node or token) that has one, its values read off the tape when the row is read.
//!
//! `core_rr` and `tree` made each as rows for every document of a group, which a rule that read one whole made for
//! the batch with every other relation: `rr_token_named` and the four token neighbours were 40 of the 85 MB of each
//! chunk's batch of sources at the M3's 300-file peak (ab200). Each is now a `TapeRows`, served by one source
//! (`RowsView`): which items have a row, the row of each, and the lookups the tape answers.
//!
//! Which items have a row is an item of the projection's rank index (`ranks::Item`), marked by its walk: the numbering
//! the kernels and the JIT read too (`rerail_clif::ids::Numbering`, `num_is`, `num_rank`, `num_select`), so no
//! relation's rows are the host's alone. A row's number is its item's rank, its item the rank's select. What a row
//! reads that the tape holds no index of (a gap's or a digest's atom, a child's place among its parent's tape
//! children, the visit order) is a derived column of the projection (`Derived`), made on its first read into the
//! projection's block, which the document's view entry points at (`ViewDoc::gaps`, `named`, `visits`, `digests`).
//! `doc`'s rows are a batch's documents themselves, one each, of what the documents are given.

use super::cells::Cells;
use super::made::ViewCounts;
use super::ranks::Item;
use super::tokens::TokRel;
use super::{Batch, Projected, Shape};
use crate::code_rule_dsl::facts::core::NONE;
use crate::code_rule_dsl::facts::vtab::Source;
use rerail_blocks::vector::Vector;
use rerail_clif::view_reads::{TAPE, TOK};
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

/// A projection's derived columns of the tape's rows, each made on its first read into the projection's block, which
/// the document's view entry points at (`ViewDoc`'s runs) for the device.
#[derive(Default)]
pub struct Derived {
    /// `structural_before`'s and `structural_after`'s gaps' atoms, by the row's `Item::Child` rank.
    gaps: [OnceLock<Cells<u32>>; 2],
    /// `rr_token_named`'s: each row's parent (its local id) and place among its parent's tape children (anonymous ones
    /// counted), by the row's `Item::Child` rank (the root's, which is none of its rows, unused).
    named: OnceLock<Cells<[u32; 2]>>,
    /// Each node's place in `visit_index`'s order, by local id.
    visits: OnceLock<Cells<u32>>,
    /// Each node's `tree_digest` atom, by local id.
    digests: OnceLock<Cells<u32>>,
}

impl Derived {
    /// The columns made so far, as the view entry's runs take them: `[gaps before, gaps after, visits, digests]`, and
    /// `named`.
    pub(super) fn made(&self) -> ([Option<&Cells<u32>>; 4], Option<&Cells<[u32; 2]>>) {
        ([self.gaps[0].get(), self.gaps[1].get(), self.visits.get(), self.digests.get()], self.named.get())
    }
}

/// Which of a document's items a `TapeRows` has a row of.
#[derive(Clone, Copy)]
pub(super) enum Rows {
    None,
    /// The document itself: one row, its item 0.
    Doc,
    /// The tape nodes its projection's rank index marks `item`, past the first `skip` of them: a row's item is its
    /// tape node, its number the node's rank less `skip`.
    Ranked { item: Item, skip: u32 },
}

impl Rows {
    fn count(self, p: Option<&Projected>) -> u32 {
        match (self, p) {
            (Rows::Doc, _) => 1,
            (Rows::Ranked { item, skip }, Some(p)) => p.ranks().count(item).saturating_sub(skip),
            _ => 0,
        }
    }
    /// Whether `x` has a row, and its number.
    fn rank(self, p: &Projected, x: u32) -> Option<u32> {
        match self {
            Rows::None => None,
            Rows::Doc => (x == 0).then_some(0),
            Rows::Ranked { item, skip } => p.ranks().local(item, x).and_then(|r| r.checked_sub(skip)),
        }
    }
    /// Row `k`'s item.
    fn select(self, p: Option<&Projected>, k: u32) -> u32 {
        match (self, p) {
            (Rows::Ranked { item, skip }, Some(p)) => p.ranks().select(item, k + skip).expect("a row of the view"),
            _ => 0,
        }
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
/// one (a tape node of an item of its rank index; a document itself), its values read where they are.
pub(super) trait TapeRows: Send + Sync + 'static {
    /// What a document's rows read beyond its projection, made once per document.
    type Doc: Send + Sync;
    fn name(&self) -> &'static str;
    /// Its slot of the batch's views' directory, which holds its first rows by document (`Batch::firsts_at`,
    /// `view_reads::TAPE`, `TOK`).
    fn slot(&self) -> usize;
    fn columns(&self) -> &'static [&'static str];
    /// The column holding the row's own item's id, which a lookup addresses.
    fn own(&self) -> usize;
    /// A column holding a node whose rows the tape lists (a parent's children: `under`), if any.
    fn parent(&self) -> Option<usize> {
        None
    }
    /// Which items of the document have rows.
    fn rows(&self, at: &At) -> Rows;
    /// Its rows in document `doc`, whose views' counts `counts` gives (`Docs::counts`: those kept, else its
    /// projection's, made for them), as `rows` has them: no document made where its counts are kept (decision 0494).
    fn counted(&self, batch: &Batch, doc: usize, counts: &dyn Fn() -> ViewCounts) -> u32;
    fn doc(&self, at: &At) -> Self::Doc;
    /// Item `item`'s row, the document's `k`th, every column.
    fn row(&self, at: &At, d: &Self::Doc, item: u32, k: u32, out: &mut [i64]);
    /// The tape nodes under tape node `x` that may have a row whose `parent()` column is `x`, in order.
    fn under(&self, _at: &At, _x: u32, _each: &mut dyn FnMut(u32)) {}
    /// Whether a lookup on `column` finds its items (`find`).
    fn keyed(&self, column: usize) -> bool {
        column == self.own() || Some(column) == self.parent()
    }
    /// The items (each with its document) a lookup of `value` on a `keyed` column may find, with a row or not: by the
    /// row's own id, its tape node; by a parent's, the nodes under it.
    fn find(&self, batch: &Batch, column: usize, value: i64, each: &mut dyn FnMut(usize, u32)) {
        let Some(doc) = batch.doc(value >> 32, &mut 0) else { return };
        let at = At { batch, doc };
        let p = at.p();
        let Some(local) = p.local_of((value & 0xffff_ffff) as u32) else { return };
        if column == self.own() {
            let x = p.place_node(local);
            if x != NONE {
                each(doc, x);
            }
        } else if local < p.len() {
            self.under(&at, p.node(local), &mut |x| each(doc, x));
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
    rows: Rows,
}

/// The most columns a `TapeRows` has.
const COLUMNS: usize = 8;

/// A `TapeRows` over a batch's documents: each document's rows at its offset, in item order.
pub(super) struct RowsView<R: TapeRows> {
    batch: Arc<Batch>,
    rows: R,
    per: Box<[OnceLock<Per<R::Doc>>]>,
}

impl<R: TapeRows> RowsView<R> {
    pub(super) fn new(batch: &Arc<Batch>, rows: R) -> Self {
        let per = (0..batch.docs.len()).map(|_| OnceLock::new()).collect();
        RowsView { batch: Arc::clone(batch), rows, per }
    }

    fn at(&self, doc: usize) -> At<'_> {
        At { batch: &self.batch, doc }
    }

    /// Document `doc`'s rows: their count checked by its first rows, which its kept counts may have given.
    fn per(&self, doc: usize) -> &Per<R::Doc> {
        self.per[doc].get_or_init(|| {
            let at = self.at(doc);
            let rows = self.rows.rows(&at);
            let made = rows.count(matches!(rows, Rows::Ranked { .. }).then(|| at.p()));
            let given = self.firsts().range(doc).len();
            if made as usize != given {
                self.batch.docs.fail(format!("{}: {made} rows of document {doc} read, its counts gave {given} (decision 0494)", self.rows.name()));
            }
            Per { doc: self.rows.doc(&at), rows }
        })
    }

    /// Its first rows by document, in the batch's views' directory (decision 0353 step 3), from each document's counts
    /// (decision 0494): those kept, else its projection's (`count`).
    fn firsts(&self) -> super::made::Firsts<'_> {
        self.batch.firsts_at(self.rows.slot())
    }

    /// Row `row`'s document and its number there (the directory's search).
    fn locate(&self, row: usize) -> (usize, u32) {
        let firsts = self.firsts();
        let doc = firsts.doc_of(row, 0);
        (doc, (row - firsts.at(doc)) as u32)
    }

    /// Document `doc`'s row `k`, every column.
    fn read(&self, doc: usize, k: u32, out: &mut [i64; COLUMNS]) {
        let per = self.per(doc);
        let at = self.at(doc);
        let item = per.rows.select(matches!(per.rows, Rows::Ranked { .. }).then(|| at.p()), k);
        self.rows.row(&at, &per.doc, item, k, out);
    }
}

impl<R: TapeRows> Source for RowsView<R> {
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        Some(Arc::clone(&self.batch) as _)
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
        if !self.batch.firsts_written()
            && let Some(n) = given.or_else(|| self.rows.statistics(&self.batch).map(|s| s[0].1 as usize))
        {
            return n;
        }
        let len = self.firsts().total();
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
    /// Document after document, each one's rows in its items' order: no row's place searched.
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        let (mut all, mut row) = ([0; COLUMNS], vec![0; columns.len()]);
        let firsts = self.firsts();
        for doc in 0..self.per.len() {
            let (per, at, first) = (self.per(doc), self.at(doc), firsts.at(doc));
            let mut emit = |k: u32, item: u32| {
                self.rows.row(&at, &per.doc, item, k, &mut all);
                for (o, &c) in row.iter_mut().zip(columns) {
                    *o = all[c];
                }
                each(first + k as usize, &row);
            };
            match per.rows {
                Rows::None => {}
                Rows::Doc => emit(0, 0),
                Rows::Ranked { item, skip } => {
                    for (r, x) in at.p().ranks().iter(item).skip(skip as usize) {
                        emit(r - skip, x);
                    }
                }
            }
        }
    }
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<Vector<usize>> {
        let firsts = self.firsts();
        Some(docs.iter().filter_map(|&k| self.batch.docs.position(k)).flat_map(|i| firsts.range(i)).collect())
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
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vector<usize>> {
        if !self.rows.keyed(column) {
            return None;
        }
        let firsts = self.firsts();
        let mut rows = Vector::new();
        for &v in values {
            self.rows.find(&self.batch, column, v, &mut |doc, x| {
                let per = self.per(doc);
                let at = self.at(doc);
                let rank = match per.rows {
                    Rows::Ranked { .. } => per.rows.rank(at.p(), x),
                    Rows::Doc => (x == 0).then_some(0),
                    Rows::None => None,
                };
                if let Some(k) = rank {
                    rows.push(firsts.at(doc) + k as usize);
                }
            });
        }
        rows.sort_unstable();
        rows.dedup();
        Some(rows)
    }
}

/// The token relations' names (`TokenNeighbour`), core.rerail's: `first_token(n, t)`, `last_token(n, t)`,
/// `token_before(x, t)`, `token_after(x, t)` (`tokens::VIEWS`).
pub const TOKENS: [&str; 4] = super::tokens::VIEWS;

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

/// The views' counts' places (`made::view_counts`): the nodes, the token order's places (with a token, one before,
/// one after), and the nodes with a numbered tape parent and the root (`Item::Child`).
const NODES: usize = 0;
const LEADS: usize = 6;
const CHILDREN: usize = 10;

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
        for rel in [TokRel::First, TokRel::Last, TokRel::Before, TokRel::After] {
            out.push((TOKENS[rel as usize], Arc::new(RowsView::new(batch, TokenNeighbour(rel)))));
        }
        for side in [Side::Before, Side::After] {
            out.push((STRUCTURAL[side as usize], Arc::new(RowsView::new(batch, Gap(side)))));
        }
    }
    out
}

/// The directory's slots whose rows a document's views' counts do not give: `doc`'s, a row a file (decision 0494: a
/// read of `doc` alone makes no document). `Batch::firsts_at` writes them apart (`count_free`), and the views' fill
/// leaves them as written.
pub(super) fn free_slots() -> [usize; 1] {
    [DocRow.slot()]
}

/// Document `doc`'s rows of each relation at a slot of `free_slots`, in that order, none of its counts asked.
pub(super) fn count_free(batch: &Batch, doc: usize) -> [usize; 1] {
    [DocRow.counted(batch, doc, &|| unreachable!("doc's rows are counted by its files, not by its views")) as usize]
}

/// Document `doc`'s rows of each relation `views` serves over `batch`, at its slot (`TapeRows::slot`) of `by`, a row of
/// the views' directory's first rows (`Batch::firsts_at`), from its views' counts `counts`.
pub(super) fn count(batch: &Batch, doc: usize, counts: &ViewCounts, by: &mut [usize]) {
    fn put<R: TapeRows>(r: R, batch: &Batch, doc: usize, counts: &ViewCounts, by: &mut [usize]) {
        by[r.slot()] = r.counted(batch, doc, &|| *counts) as usize;
    }
    put(DocRow, batch, doc, counts, by);
    put(NamedChild, batch, doc, counts, by);
    put(VisitIndex, batch, doc, counts, by);
    if batch.shape == Shape::Core {
        put(TreeDigest, batch, doc, counts, by);
        for rel in [TokRel::First, TokRel::Last, TokRel::Before, TokRel::After] {
            put(TokenNeighbour(rel), batch, doc, counts, by);
        }
        for side in [Side::Before, Side::After] {
            put(Gap(side), batch, doc, counts, by);
        }
    }
}

/// Document `doc`'s derived columns that its program reads, made before a device reads its view entry (`ViewDoc`'s
/// runs point at them; an executor reads them as the host does).
pub(super) fn derive(batch: &Batch, doc: usize) {
    let at = At { batch, doc };
    let wants = |rel: &str| batch.projection.wants(rel);
    if batch.shape == Shape::Core {
        for side in [Side::Before, Side::After] {
            if wants(STRUCTURAL[side as usize]) {
                Gap(side).atoms(&at);
            }
        }
        if wants(DIGEST) {
            TreeDigest::atoms(&at);
        }
    }
    if wants(NAMED) {
        NamedChild::places(&at);
    }
    if wants(VISIT) {
        VisitIndex::places(&at);
    }
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

/// Atoms made into a projection's block: `texts` interned, in order.
fn interned(at: &At, texts: &[&str], what: &str) -> Cells<u32> {
    let syms = at.batch.atoms.intern_all(texts).unwrap_or_else(|e| panic!("{what} interned: {e}"));
    // copy: derived (the texts' atoms: the tape holds the texts, not their atoms)
    Cells::build(at.p().block(), syms.len().max(1), |out: &mut [u32]| {
        out[..syms.len()].copy_from_slice(&syms);
        syms.len().max(1)
    })
    .expect("a projection's block holds a view's atoms")
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
/// one of its nodes (`Item::Child`). `n` is the tape's; `s` is the gap's text interned, once per row on the
/// document's first read (a derived column: the tape holds the gap, not its atom). Made as rows they were 27 MB of
/// each chunk's block of sources at the M3's 300-file peak (ab217), `n` and `s` a row each.
pub(super) struct Gap(pub(super) Side);

impl Gap {
    /// Each row's gap, `(tape node, from, to)` in bytes of the source, in the tape's order of their parents.
    fn walk(&self, p: &Projected, each: &mut dyn FnMut(u32, u32, u32)) {
        let f = p.f();
        if !f.is_empty() && p.local_of_node(0) != NONE {
            match self.0 {
                Side::Before => each(0, 0, f.start(0)),
                Side::After => each(0, f.end(0), p.facts.source.len() as u32),
            }
        }
        for (_, q) in p.nodes() {
            let children = f.children(q);
            for (k, &c) in children.iter().enumerate() {
                if p.local_of_node(c) == NONE {
                    continue;
                }
                match self.0 {
                    Side::Before => each(c, if k == 0 { f.start(q) } else { f.end(children[k - 1]) }, f.start(c)),
                    Side::After => each(c, f.end(c), children.get(k + 1).map_or(f.end(q), |&d| f.start(d))),
                }
            }
        }
    }

    /// Each row's gap's atom, by its `Item::Child` rank: made on the document's first read, in one walk.
    fn atoms<'a>(&self, at: &'a At) -> &'a Cells<u32> {
        let p = at.p();
        p.derived.gaps[self.0 as usize].get_or_init(|| {
            let source = p.facts.source.as_str();
            let ranks = p.ranks();
            let mut texts: Vec<&str> = vec![""; ranks.count(Item::Child) as usize];
            self.walk(p, &mut |x, a, b| {
                if a < b {
                    texts[ranks.rank(Item::Child, x) as usize] = &source[a as usize..b as usize];
                }
            });
            interned(at, &texts, "a document's gaps")
        })
    }
}

impl TapeRows for Gap {
    type Doc = ();
    fn name(&self) -> &'static str {
        STRUCTURAL[self.0 as usize]
    }
    fn slot(&self) -> usize {
        TAPE + self.0 as usize
    }
    fn columns(&self) -> &'static [&'static str] {
        &["n", "s"]
    }
    fn own(&self) -> usize {
        0
    }
    fn rows(&self, _: &At) -> Rows {
        Rows::Ranked { item: Item::Child, skip: 0 }
    }
    fn counted(&self, _: &Batch, _: usize, counts: &dyn Fn() -> ViewCounts) -> u32 {
        counts()[CHILDREN]
    }
    fn doc(&self, _: &At) {}
    fn row(&self, at: &At, _: &(), x: u32, k: u32, out: &mut [i64]) {
        let p = at.p();
        out[0] = p.global(p.local_of_node(x));
        out[1] = i64::from(self.atoms(at)[k as usize]);
    }
}

/// core.rerail's tokens of a node or token (ESLint's token store, `tokens`): the first and last inside it, the one
/// before it and the one after it, read off the rank index (a rank and a select of the lexemes). Its rows are the
/// places that have one (`TokRel::rows`: `Item::Leads`, `Follows`, `Precedes`), marked by the projection's walk where
/// its program reads one.
pub(super) struct TokenNeighbour(pub(super) TokRel);

impl TapeRows for TokenNeighbour {
    type Doc = ();
    fn name(&self) -> &'static str {
        TOKENS[self.0 as usize]
    }
    fn slot(&self) -> usize {
        TOK + self.0 as usize
    }
    fn columns(&self) -> &'static [&'static str] {
        self.0.columns()
    }
    fn own(&self) -> usize {
        0
    }
    fn rows(&self, _: &At) -> Rows {
        Rows::Ranked { item: self.0.rows(), skip: 0 }
    }
    fn counted(&self, batch: &Batch, _: usize, counts: &dyn Fn() -> ViewCounts) -> u32 {
        let at = LEADS
            + match self.0.rows() {
                Item::Leads => 0,
                Item::Follows => 1,
                _ => 2,
            };
        if batch.shape == Shape::Core && batch.projection.wants(self.name()) { counts()[at] } else { 0 }
    }
    fn doc(&self, _: &At) {}
    fn row(&self, at: &At, _: &(), x: u32, _: u32, out: &mut [i64]) {
        let p = at.p();
        let place = p.place_of(x);
        out[0] = p.global(place);
        out[1] = p.token_of(self.0, place);
    }
}

/// `rr_token_named(p, i, c, start, end)`: node `p`'s child `c`, its `i`th on the tape (anonymous ones counted), a named
/// node of the projection, and its byte span: the named part of `token`'s answer (decision 0174), whose anonymous
/// part (`rr_token_anon`) holds their texts. A row per node whose tape parent is a node of the projection
/// (`Item::Child` past the root).
pub(super) struct NamedChild;

impl NamedChild {
    /// The root's `Item::Child` mark, which is none of its rows.
    fn skip(p: &Projected) -> u32 {
        u32::from(p.ranks().is(Item::Child, 0))
    }

    /// Each row's parent (its local id) and `i`, by its `Item::Child` rank: made on the first read of a row, in one
    /// walk of the nodes' children (the tape holds no child's place among its siblings, and a node's parent is a
    /// search back on format 15).
    fn places<'a>(at: &'a At) -> &'a Cells<[u32; 2]> {
        let p = at.p();
        p.derived.named.get_or_init(|| {
            let ranks = p.ranks();
            let n = ranks.count(Item::Child) as usize;
            // copy: derived (each row's parent and place among its tape siblings, computed once here)
            Cells::build(p.block(), n.max(1), |out: &mut [[u32; 2]]| {
                for (lq, q) in p.nodes() {
                    let mut i = 0;
                    p.f().each_child(q, |c| {
                        if let Some(r) = ranks.local(Item::Child, c) {
                            out[r as usize] = [lq, i];
                        }
                        i += 1;
                    });
                }
                n.max(1)
            })
            .expect("a projection's block holds a view's places")
        })
    }
}

impl TapeRows for NamedChild {
    type Doc = ();
    fn name(&self) -> &'static str {
        NAMED
    }
    fn slot(&self) -> usize {
        TAPE + 2
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
    fn rows(&self, at: &At) -> Rows {
        Rows::Ranked { item: Item::Child, skip: Self::skip(at.p()) }
    }
    /// Its `Item::Child` marks but the root's, which the walk marks where tape node 0 is numbered (`skip`): a file's
    /// own tree's, and a guest's whose root is its tape's node 0 (a program on a tape of its own; not a fragment's
    /// expression inside its wrapper). A guest is walked with no anchors, so each numbered node but its first has a
    /// numbered parent: its marks are its nodes less one, and the root's. So the counts tell, no document made.
    fn counted(&self, batch: &Batch, doc: usize, counts: &dyn Fn() -> ViewCounts) -> u32 {
        let counts = counts();
        let root = counts[NODES] > 0 && (batch.docs.guest(doc).is_none() || counts[CHILDREN] >= counts[NODES]);
        counts[CHILDREN].saturating_sub(u32::from(root))
    }
    fn doc(&self, _: &At) {}
    fn row(&self, at: &At, _: &(), x: u32, k: u32, out: &mut [i64]) {
        let p = at.p();
        let [parent, i] = Self::places(at)[(k + Self::skip(p)) as usize];
        out[0] = p.global(parent);
        out[1] = i64::from(i);
        out[2] = p.global(p.local_of_node(x));
        out[3] = i64::from(p.f().start(x));
        out[4] = i64::from(p.f().end(x));
    }
    fn under(&self, at: &At, x: u32, each: &mut dyn FnMut(u32)) {
        at.p().f().each_child(x, each);
    }
}

/// core.rerail's and T's `doc(d, path, module)`: a row a file of the batch (a tape value, decision 0431, is no file's),
/// its key, its path and its module (`syntax::doc_module`), what its documents are given: none made for it. Made as
/// rows, a scan of it made every document's every relation (ab203: `exists_25`'s `doc($d, $p, _)` led its text
/// test's probe, and made the chunk's 260 documents whole, 61 MB of the block of sources at the M3's peak).
pub(super) struct DocRow;

impl DocRow {
    fn new(_: &Batch) -> Self {
        DocRow
    }

    /// Whether document `doc` is a file's: a tape value is none.
    fn is_file(batch: &Batch, doc: usize) -> bool {
        !batch.docs.guest(doc).is_some_and(|g| matches!(g.made, super::Made::Tape { .. }))
    }
}

impl TapeRows for DocRow {
    /// Its path's and its module's atoms.
    type Doc = [i64; 2];
    fn name(&self) -> &'static str {
        DOC
    }
    fn slot(&self) -> usize {
        TAPE + 5
    }
    fn columns(&self) -> &'static [&'static str] {
        &["d", "path", "module"]
    }
    fn own(&self) -> usize {
        0
    }
    fn rows(&self, at: &At) -> Rows {
        if Self::is_file(at.batch, at.doc) { Rows::Doc } else { Rows::None }
    }
    fn counted(&self, batch: &Batch, doc: usize, _: &dyn Fn() -> ViewCounts) -> u32 {
        u32::from(Self::is_file(batch, doc))
    }
    fn doc(&self, at: &At) -> [i64; 2] {
        let docs = &at.batch.docs;
        let module = super::doc_module(docs.path(at.doc), docs.module(at.doc), at.batch.shape == Shape::Core);
        // copy: derived (a document's path and module, interned: the atoms `make_rows` gave them)
        let atoms = at.batch.atoms.intern_all(&[docs.path(at.doc), module.as_str()]).expect("a document's path and module interned");
        [i64::from(atoms[0]), i64::from(atoms[1])]
    }
    fn row(&self, at: &At, d: &[i64; 2], _: u32, _: u32, out: &mut [i64]) {
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

/// `visit_index(n, i)`: node `n`'s place in the language's visit order, from 0 per document (`tree::visit_order`:
/// CPython's `ast` order in a Python document, the tape's elsewhere), of every document with a grammar, a guest's too
/// (decision 0597). A row a node (`Item::Node`); the order is a walk of the tape that the tape holds no index of: each
/// node's place is kept, 4 bytes a node of the projection, made on the document's first read (`// copy: derived`).
/// Made as rows it was 13.7 MB of each chunk's block of sources at the M3's 300-file peak (ab200), `n` and `i` a row
/// each.
pub(super) struct VisitIndex;

impl VisitIndex {
    fn places<'a>(at: &'a At) -> &'a Cells<u32> {
        let p = at.p();
        p.derived.visits.get_or_init(|| {
            let dialect = at.batch.dialects.get(p.tag()).expect("a document with rows has a grammar");
            let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python) == Some(p.tag());
            let n = p.len() as usize;
            // copy: derived (each node's place in the visit order, a walk the tape holds no index of)
            Cells::build(p.block(), n.max(1), |out: &mut [u32]| {
                out.fill(NONE);
                super::tree::visit_order(p, dialect, python, &mut |l, i| out[l as usize] = i);
                n.max(1)
            })
            .expect("a projection's block holds a view's places")
        })
    }
}

impl TapeRows for VisitIndex {
    type Doc = ();
    fn name(&self) -> &'static str {
        VISIT
    }
    fn slot(&self) -> usize {
        TAPE + 3
    }
    fn columns(&self) -> &'static [&'static str] {
        &["n", "i"]
    }
    fn own(&self) -> usize {
        0
    }
    fn rows(&self, at: &At) -> Rows {
        if at.batch.dialects.get(at.batch.docs.tag(at.doc)).is_some() { Rows::Ranked { item: Item::Node, skip: 0 } } else { Rows::None }
    }
    fn counted(&self, batch: &Batch, doc: usize, counts: &dyn Fn() -> ViewCounts) -> u32 {
        if batch.dialects.get(batch.docs.tag(doc)).is_some() { counts()[NODES] } else { 0 }
    }
    fn doc(&self, _: &At) {}
    fn row(&self, at: &At, _: &(), _: u32, k: u32, out: &mut [i64]) {
        out[0] = at.p().global(k);
        out[1] = i64::from(Self::places(at)[k as usize]);
    }
}

/// `tree_digest(n, h)` (`@tree_digest(n)`): a structural hash of each node's subtree (`core_rr::digests`), of the
/// nodes of a document with a grammar that parsed (a withheld one numbers none). A row a node (`Item::Node`); a hash
/// is of the whole subtree, which the tape holds no digest of: each node's is kept as its atom, 4 bytes a node,
/// interned on the document's first read (`// copy: derived`). Made as rows it was 6.8 MB of each chunk's block of
/// sources at the M3's 300-file peak (ab200).
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
        let (hash, _) = super::core_rr::digests(p.f(), dialect, true, false);
        let mut text = [0u8; 64];
        for (local, id, _) in p.node_ids() {
            each(local, super::core_rr::hex_into(&hash[id as usize], &mut text));
        }
    }

    /// Each node's digest's atom, by local id: made on the document's first read of a row.
    fn atoms<'a>(at: &'a At) -> &'a Cells<u32> {
        let p = at.p();
        p.derived.digests.get_or_init(|| {
            // Every node's 64 digits in one buffer, each a slice of it.
            let mut digits: Vec<u8> = vec![b'0'; 64 * p.len() as usize];
            Self::hexes(at, &mut |l, h| digits[64 * l as usize..64 * (l as usize + 1)].copy_from_slice(h.as_bytes()));
            let texts: Vec<&str> = digits.as_chunks::<64>().0.iter().map(|d| std::str::from_utf8(d).expect("hex digits")).collect();
            interned(at, &texts, "a document's digests")
        })
    }
}

impl TapeRows for TreeDigest {
    type Doc = ();
    fn name(&self) -> &'static str {
        DIGEST
    }
    fn slot(&self) -> usize {
        TAPE + 4
    }
    fn columns(&self) -> &'static [&'static str] {
        &["n", "s"]
    }
    fn own(&self) -> usize {
        0
    }
    fn rows(&self, at: &At) -> Rows {
        VisitIndex.rows(at)
    }
    fn counted(&self, batch: &Batch, doc: usize, counts: &dyn Fn() -> ViewCounts) -> u32 {
        VisitIndex.counted(batch, doc, counts)
    }
    fn doc(&self, _: &At) {}
    fn row(&self, at: &At, _: &(), _: u32, k: u32, out: &mut [i64]) {
        out[0] = at.p().global(k);
        out[1] = i64::from(Self::atoms(at)[k as usize]);
    }
}
