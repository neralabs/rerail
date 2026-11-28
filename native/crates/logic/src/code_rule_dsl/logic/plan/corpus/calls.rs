//! Calls to callable per-file relations, answered (the "Callable Per-file Relations" page; a spike, CPU only).
//!
//! The caller's fixpoint pauses when a component makes request rows (`R__call`, decision 0065 §2a); the run holds
//! every callable's requests until the caller waits, and hands them here together. [`Calls::answer_all`] groups them
//! by what holds the file column 0 names (a request's first column: the callable's own column 0, or the document its
//! router names, `by H`) (its document: a chunk's relations, which hold its files' rows together), and enters each
//! holder once for them all: it binds the holder's relations as they stand ([`FileState`]) and every callable's
//! requests of its files, and runs the callees (every callable's rules, each reading its request first, one program)
//! on a connection of its own. A request binds the file, so a callee reads only its rows, wherever the holder keeps
//! other files'. The answers are each R's rows for those requests, values only.
//! Each request asked is kept for the run (the memo), its key alone: asked again, it enters no file and adds nothing,
//! for its answers are already R's rows in the caller, the one place they lie (decision 0353: no second structure of
//! them here). A call writes nothing of its file's: the callee's only rules are R's.

use rerail_blocks::vector::Vector;
use super::super::{Error, Result, Stats, lower, run_atoms_native_over};
use super::preparation::callable::{Callee, Split};
use crate::code_rule_dsl::facts::vtab::{Modules, Source};
use crate::code_rule_dsl::logic::native::component::{self, Asked};
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as _;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Where a call finds its file's relations as they stand.
pub trait FileState {
    /// What holds the rows of the file that location value `at` names (its document's chunk); `None` where the run
    /// holds no such file.
    fn holder(&self, at: i64) -> Option<u64>;
    /// Holder `holder`'s rows of `rels`, each read where it lies (a relation it holds no rows of: none).
    fn rows(&self, holder: u64, rels: &[String]) -> Result<Vec<(String, Arc<dyn Source>)>>;
}

/// What the calls of a run did: batches answered, requests in them, those answered from the memo, files asked in
/// them, holders entered (each once a batch), requests whose file the run has not, answer rows, and the time spent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallCounts {
    pub batches: usize,
    pub asked: usize,
    pub memo_hits: usize,
    pub files: usize,
    pub entered: usize,
    pub missing_files: usize,
    pub answers: usize,
    pub seconds: f64,
}

impl CallCounts {
    /// The profile's line.
    pub fn text(&self) -> String {
        format!(
            "{} batches, {} requests ({} from the memo), {} files asked, {} holders entered ({} requests of no file), {} answer rows, {:.0} µs ({:.0} µs a holder entered)",
            self.batches, self.asked, self.memo_hits, self.files, self.entered, self.missing_files, self.answers, self.seconds * 1e6, self.seconds * 1e6 / self.entered.max(1) as f64
        )
    }
}

/// `rel`'s statistics, measured on `source` where it lies, into `stats`: the planner's for a relation bound as an input.
pub fn measured(stats: &mut Stats, rel: &str, source: &dyn Source) {
    let m = crate::code_rule_dsl::facts::vtab::syntax::statistics(source);
    stats.measured(rel, m.into_iter().map(|(c, n, d, q, qd)| (c, [n, d, q, qd])), std::iter::empty());
}

/// A callable's request, as its calls answer it.
struct Prepared {
    rel: String,
    width: usize,
    /// Its request's width: the bound columns', after the document where a router names it.
    key: usize,
    bound: Vec<usize>,
}

impl Prepared {
    /// Where a request's bound values start: after the document a router names (`by H`).
    fn bound_at(&self) -> usize {
        self.key - self.bound.len()
    }
}

/// An empty source of `width` columns: a relation a holder has no rows of.
fn empty(width: usize) -> Arc<dyn Source> {
    component::columns_source((0..width).map(|c| format!("c{c}")).collect(), vec![Some(Vector::new()); width])
}

/// A run's callees and their memo (module docs).
pub struct Calls {
    by_request: BTreeMap<String, Prepared>,
    /// Every callee's rules in one program (`Split::program`), lowered once for the run: a holder entered runs it
    /// once for every callable's requests of its files.
    lowered: lower::Lowered,
    /// The relations any callee reads, and each one's width.
    reads: Vec<String>,
    widths: BTreeMap<String, usize>,
    db: Connection,
    modules: Modules,
    modes: String,
    /// Each request asked, by its relation and row.
    memo: BTreeSet<(String, Vector<i64>)>,
    pub counts: CallCounts,
}

impl Calls {
    /// The callees of a split and their one program (`program`, `Split::program`), lowered as the run's program is
    /// (`ops`, `dialect`), run on `db` (a connection of the callees' own, on the run's store) with the native modes
    /// `modes`.
    pub fn new(callees: &[Callee], program: &Arc<crate::code_rule_dsl::logic::program_image::Version>, ops: [i64; 3], dialect: crate::code_rule_dsl::language::GrammarDialect, db: Connection, modes: &str) -> Result<Calls> {
        let domains = lower::domains_of(program).map_err(|e| Error::Plan(format!("the callees' program: {e}")))?;
        let lowered = lower::Lowered::of_placed(super::super::body::Placed::whole(program.clone()), domains, ops, dialect);
        let mut reads: Vec<String> = Vec::new();
        for r in callees.iter().flat_map(|c| c.reads.iter()) {
            if !reads.contains(r) {
                reads.push(r.clone());
            }
        }
        let widths = reads.iter().map(|r| (r.clone(), program.columns_of(r).map_or(1, |cs| cs.len()))).collect();
        let by_request = callees
            .iter()
            .map(|c| (c.callable.request.clone(), Prepared { rel: c.callable.rel.clone(), width: c.callable.width, key: c.callable.request_width(), bound: c.callable.bound.clone() }))
            .collect();
        Ok(Calls { by_request, lowered, reads, widths, db, modules: Modules::default(), modes: modes.to_owned(), memo: BTreeSet::new(), counts: CallCounts::default() })
    }

    /// For a split's callees.
    pub fn of(split: &Split, db: Connection, modes: &str) -> Result<Calls> {
        Calls::new(&split.callees, &split.program, [0; 3], crate::code_rule_dsl::language::GrammarDialect::Python, db, modes)
    }

    /// Whether `request` is a callable's.
    pub fn answers(&self, request: &str) -> bool {
        self.by_request.contains_key(request)
    }

    /// The memo's size: the requests asked so far.
    pub fn asked(&self) -> usize {
        self.memo.len()
    }

    /// The answer to one batch of request `request`'s rows (`answer_all` of it alone).
    pub fn answer(&mut self, request: &str, rows: &Asked, state: &dyn FileState) -> Result<Vec<(String, Vec<Vector<i64>>)>> {
        Ok(self.answer_all(&[(request, rows)], state)?.pop().unwrap_or_default())
    }

    /// The answers to every request the caller made before it waited (`asks`, each a request's batch of rows, any
    /// callable's), at once: the new requests grouped by what holds the file column 0 names (a request's first
    /// column: the callable's own column 0, or the document its router names, `by H`), every callable's together,
    /// and each holder entered once: it binds the holder's relations every callee reads as they stand
    /// ([`FileState`]) and every request's rows of its files, and runs the callees' one program on a connection of
    /// its own. A request binds its file, so the callee reads only its rows, wherever the holder keeps other files'.
    /// Each batch's answer is R's rows for its own requests, values only.
    pub fn answer_all(&mut self, asks: &[(&str, &Asked)], state: &dyn FileState) -> Result<Vec<Vec<(String, Vec<Vector<i64>>)>>> {
        let t0 = std::time::Instant::now();
        if let Some((request, _)) = asks.iter().find(|(r, _)| !self.by_request.contains_key(*r)) {
            return Err(Error::Plan(format!("{request}: not a callable's request")));
        }
        self.counts.batches += 1;
        // The new requests, by their file's holder and their request, in order; each batch's bound values (its own
        // answers); and the files they ask.
        let mut by_holder: BTreeMap<u64, BTreeMap<&str, Vec<Vector<i64>>>> = BTreeMap::new();
        let mut wanted: Vec<BTreeSet<Vector<i64>>> = vec![BTreeSet::new(); asks.len()];
        let mut files: BTreeSet<i64> = BTreeSet::new();
        for (i, &(request, rows)) in asks.iter().enumerate() {
            let at = self.by_request[request].bound_at();
            let mut seen: BTreeSet<Vector<i64>> = BTreeSet::new();
            for row in rows.to_vec() {
                if !seen.insert(row.clone()) {
                    continue;
                }
                self.counts.asked += 1;
                if !self.memo.insert((request.to_owned(), row.clone())) {
                    self.counts.memo_hits += 1;
                    continue;
                }
                wanted[i].insert(row[at..].iter().copied().collect());
                match state.holder(row[0]) {
                    Some(h) => {
                        files.insert(row[0]);
                        by_holder.entry(h).or_default().entry(request).or_default().push(row);
                    }
                    None => self.counts.missing_files += 1,
                }
            }
        }
        self.counts.files += files.len();
        // Each callable's rows made, over every holder entered.
        let mut made: BTreeMap<String, Vec<Vector<i64>>> = BTreeMap::new();
        for (holder, keys) in by_holder {
            self.counts.entered += 1;
            let sources = state.rows(holder, &self.reads)?;
            let mut stats = Stats::empty_of_lowered(&self.lowered)?;
            let bound: BTreeSet<&str> = sources.iter().map(|(r, _)| r.as_str()).collect();
            let none: Vec<(String, Arc<dyn Source>)> = self.reads.iter().filter(|r| !bound.contains(r.as_str())).map(|r| (r.clone(), empty(self.widths[r]))).collect();
            // Every callable's request, its rows of this holder's files (a callable asked nothing of them: none).
            let requests: Vec<(String, Arc<dyn Source>)> = self
                .by_request
                .iter()
                .map(|(request, p)| {
                    let source = match keys.get(request.as_str()) {
                        Some(rows) => component::columns_source((0..p.key).map(|c| format!("c{c}")).collect(), (0..p.key).map(|c| Some(rows.iter().map(|k| k[c]).collect())).collect()),
                        None => empty(p.key),
                    };
                    (request.clone(), source)
                })
                .collect();
            for (rel, source) in sources.into_iter().chain(none).chain(requests) {
                measured(&mut stats, &rel, source.as_ref());
                self.modules.bind(&self.db, &rel, source).map_err(|e| Error::Plan(format!("the callees: binding {rel}: {e}")))?;
            }
            let outputs: BTreeSet<String> = keys.keys().map(|r| self.by_request[*r].rel.clone()).collect();
            run_atoms_native_over(&self.lowered, &self.db, "main", stats, outputs.clone(), &self.modes)?;
            for rel in &outputs {
                let width = self.by_request.values().find(|p| &p.rel == rel).map_or(0, |p| p.width);
                let held = component::derived(&self.db, rel).ok_or_else(|| Error::Plan(format!("{rel}: the call's answer is not held")))?;
                made.entry(rel.clone()).or_default().extend((0..held.len()).map(|i| (0..width).map(|c| held.value(i, c)).collect::<Vector<i64>>()));
            }
            component::release(&self.db, &outputs.into_iter().collect::<Vec<_>>());
        }
        // Each batch its own requests' answers: R's rows whose bound columns are one of its requests'.
        let out: Vec<Vec<(String, Vec<Vector<i64>>)>> = asks
            .iter()
            .zip(&wanted)
            .map(|(&(request, _), wanted)| {
                let p = &self.by_request[request];
                let rows: Vec<Vector<i64>> = made.get(&p.rel).into_iter().flatten().filter(|r| wanted.contains(&p.bound.iter().map(|&c| r[c]).collect::<Vector<i64>>())).cloned().collect();
                vec![(p.rel.clone(), rows)]
            })
            .collect();
        self.counts.answers += out.iter().flatten().map(|(_, rows)| rows.len()).sum::<usize>();
        self.counts.seconds += t0.elapsed().as_secs_f64();
        Ok(out)
    }
}

/// A corpus run's state for calls: each chunk's relations a callable reads, held where its engine made them (their
/// blocks, `component::derived`: read in place, decision 0353), by the documents the chunk evaluated, to the end of
/// the cross part. A file is entered by its chunk: the request binds the file, so the callee reads its rows alone.
#[derive(Default)]
pub struct Held {
    by_doc: std::collections::HashMap<i64, u64>,
    chunks: Vec<Vec<(String, Arc<dyn Source>)>>,
    unheld: std::collections::HashSet<i64>,
}

/// A file's rows of a relation as its memo entry held them, flat (`width` values a row), read where they lie: the
/// entry's own values, moved, not copied.
pub struct Flat {
    width: usize,
    values: Vector<i64>,
}

impl Source for Flat {
    fn columns(&self) -> Vec<String> {
        (0..self.width).map(|c| format!("c{c}")).collect()
    }
    fn len(&self) -> usize {
        self.values.len() / self.width.max(1)
    }
    fn estimated_matches(&self, _: usize, _: bool) -> usize {
        self.len()
    }
    fn indexed(&self, _: usize) -> bool {
        false
    }
    fn lookup(&self, _: usize, _: &[i64]) -> Option<Vector<usize>> {
        None
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.values[row * self.width + column]
    }
}

impl Held {
    /// A chunk's relations `rels`, holding the rows of documents `docs`.
    pub fn add(&mut self, docs: impl IntoIterator<Item = i64>, rels: Vec<(String, Arc<dyn Source>)>) {
        let h = self.chunks.len() as u64;
        self.chunks.push(rels);
        for d in docs {
            self.by_doc.insert(d, h);
        }
    }
    /// A file's rows the memo gave (a full hit): its entry's rows of `rels`, moved here (`Flat`).
    pub fn add_entry(&mut self, doc: i64, rels: Vec<(String, (usize, Vector<i64>))>) {
        self.add([doc], rels.into_iter().map(|(r, (w, v))| (r, Arc::new(Flat { width: w, values: v }) as Arc<dyn Source>)).collect());
    }
    /// Documents whose rows it does not hold (a partial memo hit's, decision 0225 §2): a call into one is refused.
    pub fn add_unheld(&mut self, docs: impl IntoIterator<Item = i64>) {
        self.unheld.extend(docs);
    }
    /// The first of `rows`' files (column 0) whose rows it does not hold, where one is.
    pub fn unheld_of(&self, rows: &Asked) -> Option<i64> {
        rows.iter().map(|r| r[0]).find(|v| self.unheld.contains(&super::memo::document_of(*v)))
    }
    /// How many documents' rows it holds.
    pub fn documents(&self) -> usize {
        self.by_doc.len()
    }
}

impl FileState for Held {
    fn holder(&self, at: i64) -> Option<u64> {
        self.by_doc.get(&super::memo::document_of(at)).copied()
    }
    fn rows(&self, holder: u64, rels: &[String]) -> Result<Vec<(String, Arc<dyn Source>)>> {
        Ok(self.chunks[holder as usize].iter().filter(|(r, _)| rels.contains(r)).map(|(r, s)| (r.clone(), Arc::clone(s))).collect())
    }
}

#[cfg(test)]
mod tests;
