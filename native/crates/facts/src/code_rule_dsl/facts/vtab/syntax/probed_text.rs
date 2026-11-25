//! `text` read by node alone (decision 0689): the cross part's `text`, where each of its atoms reads a node, or a
//! `source` or `src_line` span, that its rule binds (`Projection::text_probed`, `text::cross_text_by_node`). A probe
//! reads its row off its document's tape as a group makes it: a node's (`CoreRows::text_row`), a line's off the source
//! (`line_text`, as `Extra::document`'s and `LongTexts::text_any`'s). No document's `text` rows are made, made document
//! or not. A read of every row (an edit's versions of the changed documents, `Before`'s scan) reads each id's in turn;
//! the corpus batch is read by probes alone. Its rows are of those ids alone: no rule of the cross part reads another's
//! (a token's, a comment's or a match's span). Its texts are the batch's strings, as a made row's were, for a text test
//! of them (`strings`, the batch's vocabulary).

use super::made::MadeDoc;
use super::ranks::Item;
use super::{Batch, Dialect, TEXT, granules};
use crate::code_rule_dsl::facts::vtab::Source;
use rerail_blocks::vector::Vector;
use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

/// Document `i`'s grammar and projection, where it has nodes: none for a text-only or withheld document.
fn doc(b: &Batch, i: usize) -> Option<(&Dialect, &MadeDoc)> {
    let tag = b.docs.tag(i);
    let dialect = if tag == TEXT { None } else { b.dialects.get(tag) }?;
    let made = b.docs.made(i);
    (!made.withheld).then_some((dialect, made))
}

fn rows(b: &Batch) -> &super::core_rr::CoreRows {
    b.text_rows.get_or_init(|| super::core_rows_for(&b.projection))
}

/// Id `n`'s text as a group makes its `text` row: a node's (`text_row`), or a line's (`line_text`). None: it has no
/// row.
fn text_of(b: &Batch, n: i64) -> Option<String> {
    let i = b.docs.position((n >> 32) as u32)?;
    let low = n as u32;
    if let Some(l) = granules::line_of(low) {
        return line_text(b, i, l);
    }
    let (dialect, made) = doc(b, i)?;
    let p = &made.p;
    let local = p.local_of(low).filter(|&l| l < p.len())?;
    let x = p.ranks().select(Item::Node, local)?;
    rows(b).text_row(p, dialect, x).map(str::to_owned)
}

/// Whether document `i`'s line `l` (0 its source) has a `text` row (`line_text`), none read: where the projection
/// reads their text (`wants_span_text`), of a document's own text (a guest's lines are its file's).
fn has_line(b: &Batch, i: usize, l: u32) -> bool {
    use crate::code_rule_dsl::logic::front::ast::SpanIn;
    b.projection.wants_span_text(if l == 0 { SpanIn::Source } else { SpanIn::Line }) && b.docs.guest(i).is_none() && l as usize <= b.docs.made(i).p.f().lines.count()
}

/// Document `i`'s line `l`'s text, 0 its source, as `Extra::document` makes the `text` rows of its `source` and
/// `src_line` spans (`has_line`).
fn line_text(b: &Batch, i: usize, l: u32) -> Option<String> {
    if !has_line(b, i, l) {
        return None;
    }
    let f = b.docs.made(i).p.f();
    let source = f.source.as_str();
    if l == 0 {
        return Some(source.to_owned());
    }
    let [a, z] = f.lines.text_of(l as usize, source.as_bytes());
    source.get(a as usize..z as usize).map(str::to_owned)
}

/// The ids of document `i` that may have a `text` row: its nodes, its source's span and its lines'.
fn ids_of(b: &Batch, i: usize) -> Vec<i64> {
    let p = &b.docs.made(i).p;
    let key = i64::from(p.key()) << 32;
    let mut ids: Vec<i64> = if doc(b, i).is_some() { p.node_ids().map(|(_, _, n)| n).collect() } else { Vec::new() };
    let line = |l: u32| key | i64::from(rerail_clif::ids::LINE_SLOT << rerail_clif::ids::INDEX_BITS | l);
    ids.extend((0..=p.f().lines.count() as u32).map(line));
    ids
}

/// Each text `text` read by node alone may give over `b` (its rows' strings, none made), for the batch's vocabulary:
/// a text test of a text read so is a test of one of the batch's strings, as it was of a made row's.
pub(super) fn strings(b: &Batch, each: &mut dyn FnMut(&str)) {
    if !b.projection.text_probed {
        return;
    }
    for i in 0..b.docs.len() {
        for n in ids_of(b, i) {
            if let Some(t) = text_of(b, n) {
                each(&t);
            }
        }
    }
}

/// The batch's `text` as probes read it: its rows those read so far, each id's at most once, in the order read.
pub(super) struct ProbedText {
    batch: Arc<Batch>,
    /// Its rows read, `[n, s]`, and each id's row (none: the id has no `text` row).
    read: parking_lot::Mutex<(Vec<[i64; 2]>, rustc_hash::FxHashMap<i64, Option<u32>>)>,
    /// Its rows, counted on the tape: each id `ids_of` gives that has a row.
    counted: OnceLock<usize>,
    /// Whether every row is read (`every`).
    whole: OnceLock<()>,
    /// Its statistics, measured on a sample of the batch's documents (`statistics`).
    sampled: OnceLock<Option<Vec<(String, i64, i64, i64, i64)>>>,
}

impl ProbedText {
    pub(super) fn new(batch: &Arc<Batch>) -> Self {
        Self { batch: Arc::clone(batch), read: Default::default(), counted: OnceLock::new(), whole: OnceLock::new(), sampled: OnceLock::new() }
    }

    /// The row of id `n`, read once: none where it has none.
    fn probe(&self, n: i64) -> Option<u32> {
        if let Some(&row) = self.read.lock().1.get(&n) {
            return row;
        }
        let atom = text_of(&self.batch, n).map(|t| match self.batch.atoms.intern(&t) {
            Ok(a) => i64::from(a),
            Err(e) => {
                self.batch.docs.fail(format!("text of {n}: {e}"));
                0
            }
        });
        let mut read = self.read.lock();
        let (rows, by_id) = &mut *read;
        if let Some(&row) = by_id.get(&n) {
            return row;
        }
        let row = atom.map(|a| {
            rows.push([n, a]);
            (rows.len() - 1) as u32
        });
        by_id.insert(n, row);
        row
    }

    /// Every row read: each document's ids probed in turn.
    fn every(&self) {
        self.whole.get_or_init(|| {
            crate::code_rule_dsl::logic::profile::decision("text read by node alone (decision 0689)", || format!("every row of {} documents read", self.batch.docs.len()));
            for i in 0..self.batch.docs.len() {
                for n in ids_of(&self.batch, i) {
                    self.probe(n);
                }
            }
        });
    }
}

impl Source for ProbedText {
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        Some(Arc::clone(&self.batch) as _)
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn columns(&self) -> Vec<String> {
        vec!["n".to_owned(), "s".to_owned()]
    }
    /// Its rows: read, where every one is; else as the kept statistics give them (`given_rows`, a made relation's
    /// count: no document made, Q332); else counted on the tape (no text read), each id `ids_of` gives that has a row.
    fn len(&self) -> usize {
        if self.whole.get().is_some() {
            return self.read.lock().0.len();
        }
        if let Some(n) = self.batch.given_rows("text") {
            return n;
        }
        *self.counted.get_or_init(|| {
            let b = &*self.batch;
            let rows = rows(b);
            (0..b.docs.len())
                .map(|i| {
                    let nodes = doc(b, i).map_or(0, |(dialect, made)| made.p.node_ids().filter(|&(_, x, _)| rows.text_row(&made.p, dialect, x).is_some()).count());
                    let lines = b.docs.made(i).p.f().lines.count() as u32;
                    nodes + (0..=lines).filter(|&l| has_line(b, i, l)).count()
                })
                .sum()
        })
    }
    /// Without making a document (Q332): its rows read, or given, where they are; else wherever the batch has a
    /// document, as a made relation's.
    fn may_hold_rows(&self) -> bool {
        if self.whole.get().is_some() {
            return !self.read.lock().0.is_empty();
        }
        match self.batch.given_rows("text") {
            Some(n) => n > 0,
            None => self.batch.docs.len() > 0,
        }
    }
    /// The batch's nodes, none of their texts read: a probe's estimate, never a scan's.
    fn estimated_len(&self) -> usize {
        self.batch.nodes().total()
    }
    /// Measured on the documents a made relation's are (`Batch::sample_groups`'s, `SAMPLE_EVERY`), their ids probed,
    /// and scaled to the batch's documents, as a made relation's (`made::sampled`): a string's rows are many, so a
    /// read by string is priced so.
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.sampled
            .get_or_init(|| {
                let b = &*self.batch;
                let n = b.docs.len();
                if n == 0 {
                    return None;
                }
                let k = n.min(super::made::SAMPLE_DOCS.max(n / super::made::SAMPLE_EVERY));
                let rows: Vec<[i64; 2]> = (0..k).flat_map(|i| ids_of(b, i)).filter_map(|id| self.probe(id)).map(|r| self.read.lock().0[r as usize]).collect();
                crate::code_rule_dsl::logic::profile::decision("text read by node alone (decision 0689)", || format!("measured on {k} of {n} documents, {} rows", rows.len()));
                let measured: Vec<(String, i64, i64, i64, i64)> = self
                    .columns()
                    .into_iter()
                    .enumerate()
                    .map(|(c, name)| {
                        let mut pairs: Vec<[i64; 2]> = rows.iter().map(|r| [r[c], r[0] >> 32]).collect();
                        pairs.sort_unstable();
                        let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
                        for value in pairs.chunk_by(|a, b| a[0] == b[0]) {
                            ndv += 1;
                            sq += (value.len() * value.len()) as i64;
                            for doc in value.chunk_by(|a, b| a[1] == b[1]) {
                                sqd += (doc.len() * doc.len()) as i64;
                            }
                        }
                        (name, rows.len() as i64, ndv, sq, sqd)
                    })
                    .collect();
                if rows.is_empty() {
                    return Some(measured);
                }
                let all = (rows.len() as u128 * n as u128 / k as u128) as usize;
                Some(super::scaled_statistics(&measured, rows.len(), all))
            })
            .clone()
    }
    fn estimated_matches(&self, column: usize, _batch: bool) -> usize {
        if column == 0 { 1 } else { self.estimated_len() }
    }
    /// A row a scan gives is its node's text read off its tape and interned, where a made relation's is read from its
    /// block: several derived rows' work (decision 0312's units). A probe's alone is meant.
    fn scan_cost(&self) -> f64 {
        64.0
    }
    fn indexed(&self, column: usize) -> bool {
        column == 0
    }
    /// Its rows are read per node looked up: a read by its string reads every one (`every`), off every document's
    /// tape, and is priced so (`Stats::probed_by`).
    fn made_on_lookup(&self) -> bool {
        true
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vector<usize>> {
        if column != 0 {
            return None;
        }
        let mut rows: Vector<usize> = values.iter().filter_map(|&n| self.probe(n)).map(|r| r as usize).collect();
        rows.sort_unstable();
        rows.dedup();
        Some(rows)
    }
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        if column != 0 {
            return false;
        }
        if let Some(row) = self.probe(value) {
            let got = self.read.lock().0[row as usize];
            let v: Vec<i64> = columns.iter().map(|&c| got[c]).collect();
            each(row as usize, &v);
        }
        true
    }
    /// The rows of the nodes `keys` names (a semi-join's, decision 0458), each probed, in row order.
    fn for_rows_keyed(&self, column: usize, keys: &rustc_hash::FxHashSet<i64>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) -> bool {
        if column != 0 {
            return false;
        }
        let mut rows: Vec<u32> = keys.iter().filter_map(|&n| self.probe(n)).collect();
        rows.sort_unstable();
        let mut v = vec![0i64; columns.len()];
        for r in rows {
            let got = self.read.lock().0[r as usize];
            for (o, &c) in v.iter_mut().zip(columns) {
                *o = got[c];
            }
            if !each(r as usize, &v) {
                break;
            }
        }
        true
    }
    /// The changed documents' rows (an edit's `Before`): each of their ids probed.
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<Vector<usize>> {
        let mut rows: Vector<usize> = Vector::new();
        for &key in docs {
            let Some(i) = self.batch.docs.position(key) else { continue };
            rows.extend(ids_of(&self.batch, i).into_iter().filter_map(|n| self.probe(n)).map(|r| r as usize));
        }
        rows.sort_unstable();
        Some(rows)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        if row >= self.read.lock().0.len() {
            self.every();
        }
        let read = self.read.lock();
        match read.0.get(row) {
            Some(r) => r[column],
            None => {
                drop(read);
                self.batch.docs.fail(format!("text: row {row} of {} read", self.len()));
                -1
            }
        }
    }
}
