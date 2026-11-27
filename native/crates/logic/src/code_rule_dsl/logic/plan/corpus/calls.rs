//! Calls to callable per-file relations, answered (the "Callable Per-file Relations" page; a spike, CPU only).
//!
//! The caller's fixpoint pauses when a component makes request rows (`R__call`, decision 0065 §2a), and hands them
//! here as one batch. [`Calls::answer`] groups the batch by what holds the file column 0 names (its document: a
//! chunk's relations, which hold its files' rows together), enters each holder once: it binds the holder's relations
//! as they stand ([`FileState`]) and its files' requests, and runs the callee (R's rules, each reading its request
//! first) on a connection of its own. The request binds the file, so the callee reads only its rows, wherever the
//! holder keeps other files'. The answers are R's rows for those requests, values only.
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

/// A callee, lowered once for the run.
struct Prepared {
    rel: String,
    width: usize,
    /// Its request's width: the bound columns'.
    key: usize,
    bound: Vec<usize>,
    lowered: lower::Lowered,
    reads: Vec<String>,
    /// Each read's width.
    widths: BTreeMap<String, usize>,
}

/// A run's callees and their memo (module docs).
pub struct Calls {
    by_request: BTreeMap<String, Prepared>,
    db: Connection,
    modules: Modules,
    modes: String,
    /// Each request asked, by its relation and row.
    memo: BTreeSet<(String, Vector<i64>)>,
    pub counts: CallCounts,
}

impl Calls {
    /// The callees of `split`, lowered as the run's program is (`ops`, `dialect`), each run on `db` (a connection of
    /// the callees' own, on the run's store) with the native modes `modes`.
    pub fn new(callees: &[Callee], ops: [i64; 3], dialect: crate::code_rule_dsl::language::GrammarDialect, db: Connection, modes: &str) -> Result<Calls> {
        let mut by_request = BTreeMap::new();
        for c in callees {
            let domains = lower::domains_of(&c.program).map_err(|e| Error::Plan(format!("callable {}: {e}", c.callable.rel)))?;
            let lowered = lower::Lowered::of_placed(super::super::body::Placed::whole(c.program.clone()), domains, ops, dialect);
            let widths = c.reads.iter().map(|r| (r.clone(), c.program.columns_of(r).map_or(1, |cs| cs.len()))).collect();
            by_request.insert(
                c.callable.request.clone(),
                Prepared { rel: c.callable.rel.clone(), width: c.callable.width, key: c.callable.bound.len(), bound: c.callable.bound.clone(), lowered, reads: c.reads.clone(), widths },
            );
        }
        Ok(Calls { by_request, db, modules: Modules::default(), modes: modes.to_owned(), memo: BTreeSet::new(), counts: CallCounts::default() })
    }

    /// For a split's callees.
    pub fn of(split: &Split, db: Connection, modes: &str) -> Result<Calls> {
        Calls::new(&split.callees, [0; 3], crate::code_rule_dsl::language::GrammarDialect::Python, db, modes)
    }

    /// Whether `request` is a callable's.
    pub fn answers(&self, request: &str) -> bool {
        self.by_request.contains_key(request)
    }

    /// The memo's size: the requests asked so far.
    pub fn asked(&self) -> usize {
        self.memo.len()
    }

    /// The answer to a batch of request `request`'s rows: R's rows for those not asked before, by entering each of
    /// their files once (module docs).
    pub fn answer(&mut self, request: &str, rows: &Asked, state: &dyn FileState) -> Result<Vec<(String, Vec<Vector<i64>>)>> {
        let t0 = std::time::Instant::now();
        let p = self.by_request.get(request).ok_or_else(|| Error::Plan(format!("{request}: not a callable's request")))?;
        self.counts.batches += 1;
        let mut out: Vec<Vector<i64>> = Vec::new();
        // The batch's new requests, by their file's holder, in order; and the files they ask.
        let mut by_holder: BTreeMap<u64, Vec<Vector<i64>>> = BTreeMap::new();
        let mut seen: BTreeSet<Vector<i64>> = BTreeSet::new();
        let mut files: BTreeSet<i64> = BTreeSet::new();
        for row in rows.to_vec() {
            if !seen.insert(row.clone()) {
                continue;
            }
            self.counts.asked += 1;
            if !self.memo.insert((request.to_owned(), row.clone())) {
                self.counts.memo_hits += 1;
                continue;
            }
            match state.holder(row[0]) {
                Some(h) => {
                    files.insert(row[0]);
                    by_holder.entry(h).or_default().push(row);
                }
                None => self.counts.missing_files += 1,
            }
        }
        self.counts.files += files.len();
        for (holder, keys) in by_holder {
            self.counts.entered += 1;
            let sources = state.rows(holder, &p.reads)?;
            let mut stats = Stats::empty_of_lowered(&p.lowered)?;
            let columns: Vec<Option<Vector<i64>>> = (0..p.key).map(|c| Some(keys.iter().map(|k| k[c]).collect())).collect();
            let asked = component::columns_source((0..p.key).map(|c| format!("c{c}")).collect(), columns);
            let bound: BTreeSet<&str> = sources.iter().map(|(r, _)| r.as_str()).collect();
            let none: Vec<(String, Arc<dyn Source>)> = p.reads.iter().filter(|r| !bound.contains(r.as_str())).map(|r| (r.clone(), component::columns_source((0..p.widths[r]).map(|c| format!("c{c}")).collect(), vec![Some(Vector::new()); p.widths[r]]))).collect();
            for (rel, source) in sources.into_iter().chain(none).chain([(request.to_owned(), asked)]) {
                measured(&mut stats, &rel, source.as_ref());
                self.modules.bind(&self.db, &rel, source).map_err(|e| Error::Plan(format!("{request}: binding {rel}: {e}")))?;
            }
            run_atoms_native_over(&p.lowered, &self.db, "main", stats, BTreeSet::from([p.rel.clone()]), &self.modes)?;
            let held = component::derived(&self.db, &p.rel).ok_or_else(|| Error::Plan(format!("{}: the call's answer is not held", p.rel)))?;
            out.extend((0..held.len()).map(|i| (0..p.width).map(|c| held.value(i, c)).collect::<Vector<i64>>()));
            component::release(&self.db, std::slice::from_ref(&p.rel));
        }
        self.counts.answers += out.len();
        self.counts.seconds += t0.elapsed().as_secs_f64();
        Ok(vec![(p.rel.clone(), out)])
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
