//! The cone of requests maintained by delta (decision 0108).
//!
//! With its answers and its gates taken as inputs, the cone's rules stratify: each relation is a stratum of its own,
//! and all of the cone's recursion is through the answers. So the cone is run as rounds of one stratified program,
//! the *round program*, each maintained from the last (`maintained::run`), and between rounds the requests it
//! raised are answered:
//!
//! ```text
//! round:      inputs (per-file rows, answers, open gate keys) ──► the round program ──► requests, asks
//! between:    new directory and file requests answered;  else new loads answered;
//!             else (quiet) every gate's keys opened: done_N := ask_N;  else the cone is done
//! ```
//!
//! This is the gated fixpoint's own order (`answers::gate`): answers as they are asked, gates opened only once
//! nothing is unanswered. A run that kept the last run's final round starts from it: its answers and its loaded
//! documents' rows are *provisional*, standing in for this run's until the run is quiet; then every key still
//! asked is answered for real (directories and files, then loads), and the provisional rows are dropped. A load the
//! last run made that this run no longer asks is never asked of the main thread.
//!
//! A run with nothing kept runs the gated fixpoint, recording its answers, and then one closing round over them,
//! which is kept for the next run (and asks nothing new, or the rounds are not kept).

use super::{Result, lower, maintained, retain};
use crate::code_rule_dsl::facts::vtab::{Modules, Source};
use crate::code_rule_dsl::logic::front::{self, Literal};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

/// Rounds past this many go back to the gated fixpoint.
const LIMIT: usize = 64;

/// The cone's program as rounds: its answers and gates as inputs.
pub(in crate::code_rule_dsl) struct RoundProgram {
    pub(in crate::code_rule_dsl) program: front::Program,
    /// Each gate's `done` relation and its `ask` (none: the whole relation's gate, `done_N(0)`).
    gates: Vec<(String, Option<String>)>,
    /// Each request relation and its width.
    requests: Vec<(String, usize)>,
    /// The relations answers add rows to, other than the per-file ones.
    produced: Vec<(String, usize)>,
}

impl RoundProgram {
    /// The round program of `load` (the gated cone, `answers::split`), when it stratifies with its answers and
    /// gates as inputs; `per_file` the per-file relations a load adds rows to (inputs already).
    pub(in crate::code_rule_dsl) fn of(load: &front::Program, per_file: &[String]) -> Option<RoundProgram> {
        let mut program = load.clone();
        let gates: Vec<(String, Option<String>)> = load.gates.iter().map(|(done, g)| (done.clone(), g.src.clone())).collect();
        let names: BTreeSet<&str> = gates.iter().map(|(d, _)| d.as_str()).collect();
        program.rules.retain(|r| !names.contains(r.head.rel.as_str()));
        for rule in &mut program.rules {
            for l in &mut rule.body {
                if let Literal::Atom(a) = l {
                    a.gated = false;
                }
            }
        }
        let heads: BTreeSet<&str> = program.rules.iter().map(|r| r.head.rel.as_str()).collect();
        let produced: Vec<(String, usize)> = load.answers.values().flatten().filter(|r| !per_file.contains(r)).filter_map(|r| Some((r.clone(), load.decls.get(r)?.len()))).collect::<BTreeSet<_>>().into_iter().collect();
        // An answer's relation that rules derive too is not an input.
        if produced.iter().any(|(r, _)| heads.contains(r.as_str())) {
            return None;
        }
        for rel in gates.iter().map(|(d, _)| d.clone()).chain(produced.iter().map(|(r, _)| r.clone())) {
            if !program.inputs.contains(&rel) {
                program.inputs.push(rel);
            }
        }
        let requests = load.answers.keys().filter_map(|r| Some((r.clone(), load.decls.get(r)?.len()))).collect();
        program.answers.clear();
        program.gates.clear();
        front::strata(&program).ok()?;
        Some(RoundProgram { program, gates, requests, produced })
    }

    /// The relations a round's caller reads: its requests and asks.
    pub(in crate::code_rule_dsl) fn read(&self) -> BTreeSet<String> {
        self.requests.iter().map(|(r, _)| r.clone()).chain(self.gates.iter().filter_map(|(_, a)| a.clone())).collect()
    }

    /// Whether `kept` is a final round of this program: it holds every request and every input (else it is what
    /// other programs kept of relations it shares, and the rounds start from the fixpoint).
    pub(in crate::code_rule_dsl) fn holds(&self, kept: &retain::Retained) -> bool {
        self.requests.iter().all(|(r, _)| kept.kept(r).is_some()) && self.program.inputs.iter().all(|r| kept.input_source(r).is_some())
    }

    /// The inputs a store keeps of the program, as empty sources of their widths (`kept_strata::load`).
    pub(in crate::code_rule_dsl) fn input_shapes(&self) -> Vec<(String, Arc<dyn Source>)> {
        self.program.inputs.iter().filter_map(|r| Some((r.clone(), empty(self.program.decls.get(r)?.len())))).collect()
    }
}

fn empty(width: usize) -> Arc<dyn Source> {
    of_rows(width, std::iter::empty())
}

fn of_rows<'a>(width: usize, rows: impl Iterator<Item = &'a Vec<i64>>) -> Arc<dyn Source> {
    let mut columns: Vec<Vec<i64>> = vec![Vec::new(); width];
    for row in rows {
        for (c, v) in row.iter().enumerate() {
            columns[c].push(*v);
        }
    }
    crate::code_rule_dsl::logic::native::component::columns_source((0..width).map(|c| format!("c{c}")).collect(), columns.into_iter().map(Some).collect())
}

fn rows(source: &dyn Source) -> Vec<Vec<i64>> {
    let width = source.columns().len();
    (0..source.len()).map(|i| (0..width).map(|c| source.value(i, c)).collect()).collect()
}

/// What the rounds need of the run.
pub(in crate::code_rule_dsl) struct Inputs<'a> {
    /// The corpus's rows of each per-file relation, and its location column.
    pub(in crate::code_rule_dsl) per_file: &'a [(String, Arc<dyn Source>, usize)],
    /// The cone's constants (the observer's facts, an earlier phase's exports), bound in every round as given.
    pub(in crate::code_rule_dsl) constants: &'a [(String, Arc<dyn Source>)],
    /// The corpus's documents.
    pub(in crate::code_rule_dsl) corpus: &'a HashSet<i64>,
}

/// What the answers gave: each request asked and the rows answers added, by relation.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Answers {
    asked: HashSet<(String, Vec<i64>)>,
    rows: BTreeMap<String, Vec<Vec<i64>>>,
}

impl Answers {
    /// Records an answer: `rel`'s `keys` asked, and what was given.
    pub(in crate::code_rule_dsl) fn record(&mut self, rel: &str, keys: &[Vec<i64>], given: &[(String, Vec<Vec<i64>>)]) {
        self.asked.extend(keys.iter().map(|k| (rel.to_owned(), k.clone())));
        for (r, rows) in given {
            self.rows.entry(r.clone()).or_default().extend(rows.iter().cloned());
        }
    }
}

/// What the rounds did.
pub(in crate::code_rule_dsl) struct Done {
    pub(in crate::code_rule_dsl) next: retain::Retained,
    pub(in crate::code_rule_dsl) rounds: usize,
    /// Rounds' strata run (summed) and of all; maintained, derived again.
    pub(in crate::code_rule_dsl) strata: (usize, usize),
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    /// Requests answered (by relation, keys).
    pub(in crate::code_rule_dsl) answered: BTreeMap<String, usize>,
}

/// Why the rounds gave up: the caller runs the gated fixpoint.
pub(in crate::code_rule_dsl) enum Gave {
    Up(String),
    Err(super::Error),
}

impl From<super::Error> for Gave {
    fn from(e: super::Error) -> Gave {
        Gave::Err(e)
    }
}

/// The run's machinery the rounds run on.
pub(in crate::code_rule_dsl) struct On<'a> {
    pub(in crate::code_rule_dsl) db: &'a Connection,
    pub(in crate::code_rule_dsl) modules: &'a Modules,
    pub(in crate::code_rule_dsl) lowered: &'a lower::Lowered,
    pub(in crate::code_rule_dsl) key: [u8; 32],
    pub(in crate::code_rule_dsl) stats: &'a crate::code_rule_dsl::logic::plan::Stats,
    pub(in crate::code_rule_dsl) keep: &'a BTreeSet<String>,
    pub(in crate::code_rule_dsl) modes: &'a str,
}

/// Where the rounds start.
pub(in crate::code_rule_dsl) enum Start {
    /// From the last run's final round.
    Kept(retain::Retained),
    /// The closing round over the gated fixpoint's answers, every gate's keys open: each ask relation's rows.
    Closing(BTreeMap<String, Vec<Vec<i64>>>),
}

impl RoundProgram {
    /// The rounds from `start`, answering through `answer`; `answers` what was answered before them.
    pub(in crate::code_rule_dsl) fn run(
        &self,
        on: &On<'_>,
        inputs: &Inputs<'_>,
        start: Start,
        mut answers: Answers,
        answer: &mut dyn FnMut(&str, &[Vec<i64>]) -> Result<Vec<(String, Vec<Vec<i64>>)>>,
    ) -> std::result::Result<Done, Gave> {
        let (kept, closing) = match start {
            Start::Kept(k) => (Some(k), None),
            Start::Closing(asks) => (None, Some(asks)),
        };
        let mut provisional: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
        let mut carried: HashSet<(String, Vec<i64>)> = HashSet::new();
        // Each gate's keys open: the last run's, or (closing) every key the fixpoint asked.
        let mut open: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
        if let Some(asks) = &closing {
            for (done, ask) in &self.gates {
                open.insert(done.clone(), match ask {
                    Some(ask) => asks.get(ask).cloned().unwrap_or_default(),
                    None => vec![vec![0]],
                });
            }
        }
        let closing = closing.is_some();
        if let Some(k) = &kept {
            for (done, _) in &self.gates {
                open.insert(done.clone(), k.input_source(done).map(|s| rows(s.as_ref())).unwrap_or_default());
            }
            for (rel, _) in &self.produced {
                provisional.insert(rel.clone(), k.input_source(rel).map(|s| rows(s.as_ref())).unwrap_or_default());
            }
            // The loaded documents' rows of each per-file relation: the kept rows not of the corpus's documents.
            for (rel, _, at) in inputs.per_file {
                let rows: Vec<Vec<i64>> = k.input_source(rel).map(|s| rows(s.as_ref())).unwrap_or_default();
                provisional.insert(rel.clone(), rows.into_iter().filter(|r| !inputs.corpus.contains(&super::memo::document_of(r[*at]))).collect());
            }
            for (req, _) in &self.requests {
                carried.extend(maintained::rows_of(k, req).unwrap_or_default().into_iter().map(|r| (req.clone(), r)));
            }
        }
        let mut kept = kept;
        let (mut rounds, mut strata, mut maintained_n, mut again) = (0, (0, 0), 0, 0);
        let mut stats = on.stats.clone();
        // Each input as the last round bound it, with its digest; the relations whose rows changed since (all, at
        // first) are made and bound again.
        let mut bound: BTreeMap<String, (Arc<dyn Source>, u64)> = BTreeMap::new();
        let mut dirty: BTreeSet<String> = self.program.inputs.iter().cloned().collect();
        let corpus_rows: BTreeMap<&str, Vec<Vec<i64>>> = inputs.per_file.iter().map(|(r, s, _)| (r.as_str(), rows(s.as_ref()))).collect();
        loop {
            rounds += 1;
            if rounds > LIMIT {
                return Err(Gave::Up(format!("past {LIMIT} rounds")));
            }
            let s = crate::code_rule_dsl::logic::profile::span(|| "rounds/inputs".into());
            let fresh = std::mem::take(&mut dirty);
            for rel in fresh.iter().cloned() {
                let width = self.program.decls[&rel].len();
                let source = if let Some(keys) = open.get(&rel) {
                    of_rows(width, keys.iter())
                } else if let Some((_, given)) = inputs.constants.iter().find(|(r, _)| *r == rel) {
                    Arc::clone(given)
                } else {
                    let mut seen: HashSet<&Vec<i64>> = HashSet::new();
                    let all = corpus_rows.get(rel.as_str()).into_iter().flatten().chain(provisional.get(&rel).into_iter().flatten()).chain(answers.rows.get(&rel).into_iter().flatten()).filter(|r| seen.insert(*r));
                    of_rows(width, all)
                };
                let digest = retain::digest(source.as_ref());
                bound.insert(rel, (source, digest));
            }
            let given: Vec<(String, Arc<dyn Source>, u64, bool)> = bound.iter().map(|(r, (s, d))| (r.clone(), Arc::clone(s), *d, !fresh.contains(r))).collect();
            drop(s);
            let ran = maintained::run(on.db, on.modules, on.lowered, on.key, &given, kept.take(), &mut stats, on.keep, on.modes)?;
            strata = (strata.0 + ran.strata.0, ran.strata.1);
            maintained_n += ran.maintained;
            again += ran.derived_again;
            let next = ran.next;
            let _s = crate::code_rule_dsl::logic::profile::span(|| "rounds/requests".into());
            // What the round asks that is not answered, by request; and of that, what is not carried.
            let mut unanswered: Vec<(String, Vec<Vec<i64>>, Vec<Vec<i64>>)> = Vec::new();
            for (req, _) in &self.requests {
                let asked = maintained::rows_of(&next, req).ok_or_else(|| Gave::Up(format!("{req} is not kept")))?;
                let all: Vec<Vec<i64>> = asked.iter().filter(|r| !answers.asked.contains(&(req.clone(), (*r).clone()))).cloned().collect();
                let fresh: Vec<Vec<i64>> = all.iter().filter(|r| provisional.is_empty() || !carried.contains(&(req.clone(), (*r).clone()))).cloned().collect();
                unanswered.push((req.clone(), all, fresh));
            }
            if closing && unanswered.iter().any(|(_, all, _)| !all.is_empty()) {
                return Err(Gave::Up(format!("the closing round asks what the fixpoint did not: {}", unanswered.iter().filter(|(_, a, _)| !a.is_empty()).map(|(r, a, _)| format!("{r} ×{}", a.len())).collect::<Vec<_>>().join(", "))));
            }
            kept = Some(next);
            let is_load = |req: &str| req == super::world::LOAD;
            // New directory and file requests first, then new loads; once the run is quiet, the carried keys still
            // asked, answered for real (the same order).
            let pick = |fresh: bool, loads: bool| -> Vec<(String, Vec<Vec<i64>>)> {
                unanswered.iter().filter(|(r, ..)| is_load(r) == loads).map(|(r, all, f)| (r.clone(), if fresh { f.clone() } else { all.clone() })).filter(|(_, k)| !k.is_empty()).collect()
            };
            let batch = [pick(true, false), pick(true, true), pick(false, false), pick(false, true)].into_iter().find(|b| !b.is_empty());
            if let Some(batch) = batch {
                for (req, keys) in batch {
                    let given = answer(&req, &keys)?;
                    dirty.extend(given.iter().filter(|(_, rows)| !rows.is_empty()).map(|(r, _)| r.clone()));
                    answers.record(&req, &keys, &given);
                }
                continue;
            }
            // Quiet, and every key asked answered for real: the provisional rows go.
            if !provisional.is_empty() {
                dirty.extend(std::mem::take(&mut provisional).into_keys());
                continue;
            }
            let next = kept.as_ref().expect("a round ran");
            let mut opened: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
            for (done, ask) in &self.gates {
                let keys = match ask {
                    Some(ask) => maintained::rows_of(next, ask).ok_or_else(|| Gave::Up(format!("{ask} is not kept")))?,
                    None => vec![vec![0]],
                };
                opened.insert(done.clone(), keys);
            }
            let same = |a: &BTreeMap<String, Vec<Vec<i64>>>, b: &BTreeMap<String, Vec<Vec<i64>>>| {
                a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| v.iter().collect::<BTreeSet<_>>() == w.iter().collect::<BTreeSet<_>>()))
            };
            if same(&opened, &open) {
                break;
            }
            if closing {
                return Err(Gave::Up("the closing round opens gates the fixpoint did not".to_owned()));
            }
            dirty.extend(opened.keys().cloned());
            open = opened;
        }
        let mut answered: BTreeMap<String, usize> = BTreeMap::new();
        for (req, _) in &answers.asked {
            *answered.entry(req.clone()).or_insert(0) += 1;
        }
        Ok(Done { next: kept.expect("a round ran"), rounds, strata, maintained: maintained_n, derived_again: again, answered })
    }

    /// The ask relations, by gate.
    pub(in crate::code_rule_dsl) fn asks(&self) -> impl Iterator<Item = &String> {
        self.gates.iter().filter_map(|(_, a)| a.as_ref())
    }
}
