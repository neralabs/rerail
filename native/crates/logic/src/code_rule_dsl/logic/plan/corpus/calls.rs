//! Calls to callable per-file relations, answered (the "Callable Per-file Relations" page; a spike, CPU only).
//!
//! The caller's fixpoint pauses when a component makes request rows (`R__call`, decision 0065 §2a), and hands them
//! here as one batch. [`Calls::answer`] groups the batch by the file column 0 names, enters each file once: it binds
//! the file's relations as they stand ([`FileState`]) and the file's requests, and runs the callee (R's rules, each
//! reading its request first) on a connection of its own. The answers are R's rows for those requests, values only.
//! Each request asked is kept for the run (the memo), its key alone: asked again, it enters no file and adds nothing,
//! for its answers are already R's rows in the caller, the one place they lie (decision 0353: no second structure of
//! them here). A call writes nothing of its file's: the callee's only rules are R's.

use rerail_blocks::vector::Vector;
use super::super::{Error, Result, Stats, lower, run_atoms_native_over};
use super::preparation::callable::{Callee, Split};
use crate::code_rule_dsl::facts::vtab::{Modules, Source};
use crate::code_rule_dsl::logic::native::component::{self, Asked};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Where a call finds its file's relations as they stand.
pub trait FileState {
    /// The rows of `rels` in the file whose path is atom `path`, each read where it lies; `None` where the run has
    /// no file of that path.
    fn rows(&self, path: i64, rels: &[String]) -> Result<Option<Vec<(String, Arc<dyn Source>)>>>;
}

/// What the calls of a run did: batches answered, requests in them, those answered from the memo, files entered,
/// requests whose file the run has not, answer rows, and the time spent answering.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallCounts {
    pub batches: usize,
    pub asked: usize,
    pub memo_hits: usize,
    pub files_entered: usize,
    pub missing_files: usize,
    pub answers: usize,
    pub seconds: f64,
}

impl CallCounts {
    /// The profile's line.
    pub fn text(&self) -> String {
        format!(
            "{} batches, {} requests ({} from the memo), {} files entered ({} requests of no file), {} answer rows, {:.3} s",
            self.batches, self.asked, self.memo_hits, self.files_entered, self.missing_files, self.answers, self.seconds
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
    /// The callees of `split`, each run on `db` (a connection of the callees' own, on the run's store) with the
    /// native modes `modes`.
    pub fn new(callees: &[Callee], db: Connection, modes: &str) -> Result<Calls> {
        let mut by_request = BTreeMap::new();
        for c in callees {
            let domains = lower::domains_of(&c.program).map_err(|e| Error::Plan(format!("callable {}: {e}", c.callable.rel)))?;
            let lowered = lower::Lowered::of_placed(super::super::body::Placed::whole(c.program.clone()), domains, [0; 3], crate::code_rule_dsl::language::GrammarDialect::Python);
            by_request.insert(
                c.callable.request.clone(),
                Prepared { rel: c.callable.rel.clone(), width: c.callable.width, key: c.callable.bound.len(), bound: c.callable.bound.clone(), lowered, reads: c.reads.clone() },
            );
        }
        Ok(Calls { by_request, db, modules: Modules::default(), modes: modes.to_owned(), memo: BTreeSet::new(), counts: CallCounts::default() })
    }

    /// For a split's callees.
    pub fn of(split: &Split, db: Connection, modes: &str) -> Result<Calls> {
        Calls::new(&split.callees, db, modes)
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
        // The batch's new requests, by file (column 0), in order.
        let mut by_file: BTreeMap<i64, Vec<Vector<i64>>> = BTreeMap::new();
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
            by_file.entry(row[0]).or_default().push(row);
        }
        for (path, keys) in by_file {
            match state.rows(path, &p.reads)? {
                None => {
                    self.counts.missing_files += keys.len();
                }
                Some(sources) => {
                    self.counts.files_entered += 1;
                    let mut stats = Stats::empty_of_lowered(&p.lowered)?;
                    let columns: Vec<Option<Vector<i64>>> = (0..p.key).map(|c| Some(keys.iter().map(|k| k[c]).collect())).collect();
                    let asked = component::columns_source((0..p.key).map(|c| format!("c{c}")).collect(), columns);
                    for (rel, source) in sources.into_iter().chain([(request.to_owned(), asked)]) {
                        measured(&mut stats, &rel, source.as_ref());
                        self.modules.bind(&self.db, &rel, source).map_err(|e| Error::Plan(format!("{request}: binding {rel}: {e}")))?;
                    }
                    run_atoms_native_over(&p.lowered, &self.db, "main", stats, BTreeSet::from([p.rel.clone()]), &self.modes)?;
                    let held = component::derived(&self.db, &p.rel).ok_or_else(|| Error::Plan(format!("{}: the call's answer is not held", p.rel)))?;
                    out.extend((0..held.len()).map(|i| (0..p.width).map(|c| held.value(i, c)).collect::<Vector<i64>>()));
                    component::release(&self.db, std::slice::from_ref(&p.rel));
                }
            }
        }
        self.counts.answers += out.len();
        self.counts.seconds += t0.elapsed().as_secs_f64();
        Ok(vec![(p.rel.clone(), out)])
    }
}

#[cfg(test)]
mod tests;
