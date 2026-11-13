//! The cone of requests maintained by delta (decision 0145).
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
use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Rule, Term};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    /// The relations answers add rows to, other than the per-file ones: `loaded`, and the file system's relations
    /// (`fs_input`) in place of the answers they make.
    produced: Vec<(String, usize)>,
    /// Each answer of the file system (`rr::WORLD`'s `dir_entry`, `file_line`, `file_digest`) the program reads, with
    /// its request: a join of the two, `dir_entry(d, n, k) :- rr_dir_req(d), rr_fs_dir_entry(d, n, k)`.
    fs: Vec<(String, String)>,
}

/// The relation of the file system an answer is the join of its request with (decision 0145, t33's K2): its rows are
/// the file system's where asked, kept as inputs, and the answer is derived.
pub(in crate::code_rule_dsl) fn fs_input(answer: &str) -> String {
    format!("rr_fs_{answer}")
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
        // The file system's answers as joins: each request with a relation of the file system, the input.
        let mut fs = Vec::new();
        for (answer, req) in crate::code_rule_dsl::logic::rr::WORLD {
            let Some(width) = produced.iter().find(|(r, _)| r == answer).map(|(_, w)| *w) else { continue };
            if !load.answers.contains_key(req) || load.decls.get(req).is_none_or(|d| d.len() != 1) {
                continue;
            }
            let input = fs_input(answer);
            program.decls.insert(input.clone(), load.decls[answer].clone());
            let vars: Vec<Term> = (0..width).map(|c| Term::Var(format!("rr_c{c}"))).collect();
            program.rules.push(Rule {
                head: Atom { rel: (answer).to_owned().into(), args: vars.clone(), neg: false, gated: false },
                body: vec![
                    Literal::Atom(Atom { rel: (req).to_owned().into(), args: vec![vars[0].clone()], neg: false, gated: false }),
                    Literal::Atom(Atom { rel: input.clone().into(), args: vars, neg: false, gated: false }),
                ],
            });
            // Read back by the rest of the cross part: kept, not inlined.
            program.qualifiers.entry((answer).to_owned()).or_default().insert("no_inline".to_owned());
            program.inputs.retain(|r| r != answer);
            fs.push(((answer).to_owned(), input));
        }
        let mut produced = produced;
        for (answer, input) in &fs {
            for p in produced.iter_mut().filter(|(r, _)| r == answer) {
                p.0 = input.clone();
            }
        }
        for rel in gates.iter().map(|(d, _)| d.clone()).chain(produced.iter().map(|(r, _)| r.clone())) {
            if !program.inputs.contains(&rel) {
                program.inputs.push(rel);
            }
        }
        let requests = load.answers.keys().filter_map(|r| Some((r.clone(), load.decls.get(r)?.len()))).collect();
        program.answers.clear();
        program.gates.clear();
        // The rounds hand on rows (requests to the driver, the cone's relations to the rest), never output lines: so
        // each is kept in the frames as any derived relation, with its rows.
        program.outputs.clear();
        front::strata(&program).ok()?;
        Some(RoundProgram { program, gates, requests, produced, fs })
    }

    /// The relations a round's caller reads: its requests and asks.
    pub(in crate::code_rule_dsl) fn read(&self) -> BTreeSet<String> {
        self.requests.iter().map(|(r, _)| r.clone()).chain(self.gates.iter().filter_map(|(_, a)| a.clone())).collect()
    }

    /// Whether `kept` is a final round of this program: it holds every request and every input (else it is what
    /// other programs kept of relations it shares, and the rounds start from the fixpoint).
    pub(in crate::code_rule_dsl) fn holds(&self, kept: &retain::Retained) -> bool {
        self.missing(kept).is_empty()
    }

    /// What `kept` lacks of a final round: the requests and inputs it does not hold.
    pub(in crate::code_rule_dsl) fn missing(&self, kept: &retain::Retained) -> Vec<String> {
        let requests = self.requests.iter().map(|(r, _)| r).filter(|r| !kept.holds(r, &self.program));
        requests.chain(self.program.inputs.iter().filter(|r| !kept.has_input(r))).cloned().collect()
    }

}

pub(in crate::code_rule_dsl) fn of_rows<'a>(width: usize, rows: impl Iterator<Item = &'a Vec<i64>>) -> Arc<dyn Source> {
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
    /// The corpus's documents.
    pub(in crate::code_rule_dsl) corpus: &'a HashSet<i64>,
    /// The run's constants the cone reads (the observer's facts, an earlier phase's exports): bound as they are.
    pub(in crate::code_rule_dsl) constants: &'a BTreeMap<String, Arc<dyn Source>>,
    /// The loads answered and taken back, between the rounds and the thread that answers them.
    pub(in crate::code_rule_dsl) ledger: &'a std::sync::Mutex<LoadLedger>,
}

/// What the rounds and the thread answering their loads share (decision 0145, a load taken back).
#[derive(Default)]
pub(in crate::code_rule_dsl) struct LoadLedger {
    /// Each file a load answered (its path's atom, `loaded`'s first column) with its document: what a load taken
    /// back takes with it. Filled by the answering thread.
    pub(in crate::code_rule_dsl) documents: HashMap<i64, i64>,
    /// The files the rounds took back, which the answering thread has not yet unloaded (`World::unload`): their
    /// places in the load budget are free again.
    pub(in crate::code_rule_dsl) taken_back: Vec<i64>,
    /// The rounds are asking (set by the cone's thread while they run): their loads are answered at once, not in
    /// the budget's levels.
    pub(in crate::code_rule_dsl) rounds: bool,
    /// A load the rounds asked was past the budget: what they load then depends on their order, so they give up and
    /// the cone runs fresh, in levels (decision 0145).
    pub(in crate::code_rule_dsl) bound: bool,
    /// The rounds gave up and the cone runs fresh: what was answered to them is not the run's answers.
    pub(in crate::code_rule_dsl) restarted: bool,
    /// The cone runs again in the budget's levels: the answering thread takes back what was loaded from beyond the
    /// corpus (`World::take_back_libraries`) and answers the loads asked again level by level.
    pub(in crate::code_rule_dsl) relevel: bool,
}

impl Inputs<'_> {
    /// The constants' digests: the inputs a round binds as they are, which the frames' reads record.
    pub(in crate::code_rule_dsl) fn constant_digests(&self) -> HashMap<String, u64> {
        self.constants.iter().map(|(rel, source)| (rel.clone(), retain::digest(source.as_ref()))).collect()
    }
}

/// What the answers gave: each request asked and the rows answers added, by relation.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Answers {
    asked: HashSet<(String, Vec<i64>)>,
    rows: BTreeMap<String, Vec<Vec<i64>>>,
}

impl Answers {
    /// Takes back the loads `stale` (keys of `world::LOAD`, `(file, module, grammar)`), none of which `asked_now`
    /// holds: the `loaded` rows of their module and grammar that no key still asked shares, and the rows of the
    /// per-file relations located at their files' documents (`documents`, as the answers recorded them; by
    /// `per_file`'s location column). Nothing is matched by value. Their documents go into `withdrawn`; the relations
    /// whose rows changed are returned.
    fn withdraw(&mut self, stale: &[Vec<i64>], asked_now: &HashSet<Vec<i64>>, per_file: &[(String, Arc<dyn Source>, usize)], ledger: &mut LoadLedger, withdrawn: &mut HashSet<i64>) -> BTreeSet<String> {
        let live: HashSet<(i64, i64)> = asked_now.iter().filter(|k| k.len() == 3).map(|k| (k[1], k[2])).collect();
        let gone: HashSet<(i64, i64)> = stale.iter().filter(|k| k.len() == 3).map(|k| (k[1], k[2])).filter(|mg| !live.contains(mg)).collect();
        for k in stale {
            self.asked.remove(&(super::world::LOAD.to_owned(), k.clone()));
        }
        let files: HashSet<i64> = self.rows.get(super::world::LOADED).into_iter().flatten().filter(|r| r.len() == 3 && gone.contains(&(r[1], r[2]))).map(|r| r[0]).collect();
        // Their documents, as the answers recorded them. A file loaded under a module still asked keeps its `loaded`
        // row and its document.
        let at: HashMap<&str, usize> = per_file.iter().map(|(r, _, at)| (r.as_str(), *at)).collect();
        let still: HashSet<i64> = self.rows.get(super::world::LOADED).into_iter().flatten().filter(|r| r.len() == 3 && !gone.contains(&(r[1], r[2]))).map(|r| r[0]).collect();
        let taken: Vec<i64> = files.iter().filter(|f| !still.contains(*f)).copied().collect();
        let docs: HashSet<i64> = taken.iter().filter_map(|f| ledger.documents.get(f).copied()).collect();
        ledger.taken_back.extend(taken);
        // A load this run asked and was not given (past the budget, say) is asked again: a place may be free now.
        let given: HashSet<(i64, i64)> = self.rows.get(super::world::LOADED).into_iter().flatten().filter(|r| r.len() == 3 && !gone.contains(&(r[1], r[2]))).map(|r| (r[1], r[2])).collect();
        if !docs.is_empty() {
            self.asked.retain(|(rel, k)| !(rel == super::world::LOAD && k.len() == 3 && !given.contains(&(k[1], k[2]))));
        }
        withdrawn.extend(docs.iter().copied());
        let mut touched = BTreeSet::new();
        for (rel, rows) in self.rows.iter_mut() {
            let before = rows.len();
            let of_doc = |r: &Vec<i64>| at.get(rel.as_str()).is_some_and(|c| r.get(*c).is_some_and(|v| docs.contains(&super::memo::document_of(*v))));
            let loaded_gone = |r: &Vec<i64>| rel == super::world::LOADED && r.len() == 3 && gone.contains(&(r[1], r[2]));
            rows.retain(|r| !(loaded_gone(r) || of_doc(r)));
            if rows.len() != before {
                touched.insert(rel.clone());
            }
        }
        touched
    }

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
    /// The strata derived again, by why, with one of their relations.
    pub(in crate::code_rule_dsl) again_why: BTreeMap<&'static str, BTreeSet<String>>,
    /// Requests of the file system kept as the last run answered them (`On::still`).
    pub(in crate::code_rule_dsl) still: usize,
    /// The file system's answers as the last round bound them (`world_digest`).
    pub(in crate::code_rule_dsl) world: u64,
    /// The documents loaded this run and taken back: the run drops them (`Answers::withdraw`).
    pub(in crate::code_rule_dsl) withdrawn: HashSet<i64>,
    /// Rows written back to the frames.
    pub(in crate::code_rule_dsl) written: usize,
    /// Each produced relation's rows as the last round bound them (what the rest of the cross part reads).
    pub(in crate::code_rule_dsl) produced: Vec<(String, Vec<Vec<i64>>)>,
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
    /// The store the rounds are kept in, as frames of fact space `space` (decision 0103): read back as a round needs
    /// them, written back behind the run.
    pub(in crate::code_rule_dsl) store: &'a crate::content_store::ContentStore,
    pub(in crate::code_rule_dsl) space: [u8; 32],
    pub(in crate::code_rule_dsl) db: &'a Connection,
    pub(in crate::code_rule_dsl) modules: &'a Modules,
    pub(in crate::code_rule_dsl) lowered: &'a lower::Lowered,
    pub(in crate::code_rule_dsl) key: [u8; 32],
    pub(in crate::code_rule_dsl) stats: &'a crate::code_rule_dsl::logic::plan::Stats,
    pub(in crate::code_rule_dsl) keep: &'a BTreeSet<String>,
    pub(in crate::code_rule_dsl) modes: &'a str,
    /// Whether a carried request of the file system (`rr::WORLD`) still has the answer the last run kept: the
    /// journal names no change at its path since (decision 0105). Such a key is not asked again.
    pub(in crate::code_rule_dsl) still: Option<&'a dyn Fn(&str, &[i64]) -> bool>,
}

/// The digest of the file system's answers `kept` holds as inputs (`rr::WORLD`'s relations): what a record of the
/// journal's cursors is checked against, so its cursors are trusted only over the answers kept with them.
pub(in crate::code_rule_dsl) fn world_digest(digest: impl Fn(&str) -> Option<u64>) -> u64 {
    crate::code_rule_dsl::logic::rr::WORLD.iter().fold(0x5157_0001u64, |h, (rel, _)| h.rotate_left(17) ^ digest(&fs_input(rel)).unwrap_or(0))
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
        // The answers recorded before the rounds (the gated fixpoint's), kept where the rounds keep them.
        answers.rows = std::mem::take(&mut answers.rows).into_iter().fold(BTreeMap::new(), |mut m: BTreeMap<String, Vec<Vec<i64>>>, (r, rows)| {
            m.entry(self.kept_as(&r)).or_default().extend(rows);
            m
        });
        let mut provisional: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
        let mut still_n = 0;
        // The documents loaded this run and taken back (`Answers::withdraw`).
        let mut withdrawn: HashSet<i64> = HashSet::new();
        let mut carried: HashSet<(String, Vec<i64>)> = HashSet::new();
        // Each gate's keys open: the last run's, or (closing) every key the fixpoint asked.
        let mut open: BTreeMap<String, Vec<Vec<i64>>> = BTreeMap::new();
        if let Some(asks) = &closing {
            for (done, ask) in &self.gates {
                open.insert(done.clone(), match ask {
                    Some(ask) => asks.get(ask).cloned().unwrap_or_default(),
                    None => vec![vec![0; self.program.decls[done].len()]],
                });
            }
        }
        let closing = closing.is_some();
        // What the store holds of the program before the rounds: what the rounds write back is compared with it.
        let was = match &kept {
            Some(k) => k.frames().cloned(),
            None => retain::Retained::from_frames(on.store, on.space, on.key, &self.program, &crate::code_rule_dsl::logic::front::strata(&self.program).map_err(|e| super::Error::Plan(e.to_string()))?.0, &inputs.constant_digests())?.frames().cloned(),
        };
        if let Some(k) = &kept {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "rounds/the last run's, from the frames".into());
            let program = &self.program;
            // A cone's program has no split: its inputs' rows are grouped by their first node column (`group_col`).
            let no_locations = retain::Locations::new();
            for (done, _) in &self.gates {
                open.insert(done.clone(), k.read_input(on.store, program, &no_locations, done, None)?.unwrap_or_default());
            }
            for (rel, _) in &self.produced {
                provisional.insert(rel.clone(), k.read_input(on.store, program, &no_locations, rel, None)?.unwrap_or_default());
            }
            // The loaded documents' rows of each per-file relation: the kept rows not of the corpus's documents
            // (only their granules read, where the documents are the column the frames group its rows by).
            for (rel, _, at) in inputs.per_file {
                let loaded = |d: i64| !inputs.corpus.contains(&d);
                let rows = k.read_input(on.store, program, &no_locations, rel, (*at == retain::group_col(program, &no_locations, rel)).then_some(&loaded as &dyn Fn(i64) -> bool))?.unwrap_or_default();
                provisional.insert(rel.clone(), rows.into_iter().filter(|r| loaded(super::memo::document_of(r[*at]))).collect());
            }
            for (req, _) in &self.requests {
                carried.extend(k.read_derived(on.store, req)?.unwrap_or_default().into_iter().map(|r| (req.clone(), r)));
            }
            // A request of the file system the journal names no change for keeps its answer: its rows of the kept
            // answers are its answer again, and it is not asked.
            if let Some(still) = on.still {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "rounds/answers the journal keeps".into());
                for (answer, req) in crate::code_rule_dsl::logic::rr::WORLD {
                    let produced = self.kept_as(answer);
                    let Some(rows) = provisional.get(&produced) else { continue };
                    let mut by: HashMap<i64, Vec<&Vec<i64>>> = HashMap::new();
                    for r in rows.iter().filter(|r| !r.is_empty()) {
                        by.entry(r[0]).or_default().push(r);
                    }
                    let keys: Vec<&Vec<i64>> = carried.iter().filter(|(q, k)| q == req && k.len() == 1 && still(q, k)).map(|(_, k)| k).collect();
                    let given: Vec<Vec<i64>> = keys.iter().flat_map(|k| by.get(&k[0]).into_iter().flatten().map(|r| (*r).clone())).collect();
                    still_n += keys.len();
                    answers.asked.extend(keys.into_iter().map(|k| ((*req).to_owned(), k.clone())));
                    answers.rows.entry(produced).or_default().extend(given);
                }
            }
        }
        let mut kept = kept;
        let (mut rounds, mut strata, mut maintained_n, mut again) = (0, (0, 0), 0, 0);
        let mut again_why: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
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
                let source = if let Some(c) = inputs.constants.get(&rel) {
                    Arc::clone(c)
                } else if let Some(keys) = open.get(&rel) {
                    of_rows(width, keys.iter())
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
            let ran = maintained::run(Some(on.store), on.db, on.modules, on.lowered, on.key, &given, kept.take(), &mut stats, on.keep, on.modes)?;
            strata = (strata.0 + ran.strata.0, ran.strata.1);
            maintained_n += ran.maintained;
            again += ran.derived_again;
            for (rel, why) in &ran.again {
                again_why.entry(why).or_default().insert(rel.clone());
            }
            let next = ran.next;
            let _s = crate::code_rule_dsl::logic::profile::span(|| "rounds/requests".into());
            // What the round asks that is not answered, by request; and of that, what is not carried.
            let mut unanswered: Vec<(String, Vec<Vec<i64>>, Vec<Vec<i64>>)> = Vec::new();
            for (req, _) in &self.requests {
                let asked = next.read_derived(on.store, req)?.ok_or_else(|| Gave::Up(format!("{req} is not kept")))?;
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
                    let given: Vec<(String, Vec<Vec<i64>>)> = answer(&req, &keys)?.into_iter().map(|(r, rows)| (self.kept_as(&r), rows)).collect();
                    if inputs.ledger.lock().unwrap_or_else(|e| e.into_inner()).bound {
                        return Err(Gave::Up("a load past the budget: where it binds, the loads are taken level by level, in a fresh cone".into()));
                    }
                    // A document taken back and loaded again is the run's again.
                    if req == super::world::LOAD && !withdrawn.is_empty() {
                        let ledger = inputs.ledger.lock().unwrap_or_else(|e| e.into_inner());
                        for (_, rows) in given.iter().filter(|(r, _)| r == super::world::LOADED) {
                            for r in rows {
                                if let Some(d) = r.first().and_then(|f| ledger.documents.get(f)) {
                                    withdrawn.remove(d);
                                }
                            }
                        }
                    }
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
                    Some(ask) => next.read_derived(on.store, ask)?.ok_or_else(|| Gave::Up(format!("{ask} is not kept")))?,
                    None => vec![vec![0; self.program.decls[done].len()]],
                };
                opened.insert(done.clone(), keys);
            }
            let same = |a: &BTreeMap<String, Vec<Vec<i64>>>, b: &BTreeMap<String, Vec<Vec<i64>>>| {
                a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| v.iter().collect::<BTreeSet<_>>() == w.iter().collect::<BTreeSet<_>>()))
            };
            if same(&opened, &open) {
                // Quiet with its gates as the run's own: a load answered this run that nothing asks for now was
                // asked from the last run's rows or gates alone. It is taken back, with every row its answer gave
                // (those naming its file or a node of it), and the rounds run on to quiet: what only it asked for
                // goes in turn.
                let asked_now: HashSet<Vec<i64>> = next.read_derived(on.store, super::world::LOAD)?.unwrap_or_default().into_iter().collect();
                let stale: Vec<Vec<i64>> = answers.asked.iter().filter(|(r, k)| r == super::world::LOAD && !asked_now.contains(k)).map(|(_, k)| k.clone()).collect();
                if stale.is_empty() {
                    break;
                }
                if closing {
                    return Err(Gave::Up("the closing round takes back a load".to_owned()));
                }
                let touched = answers.withdraw(&stale, &asked_now, inputs.per_file, &mut inputs.ledger.lock().unwrap_or_else(|e| e.into_inner()), &mut withdrawn);
                dirty.extend(touched);
                continue;
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
        let next = kept.expect("a round ran");
        // Written back as any kept part is (t33 K4): each relation whose rows or reads differ from its frame, each
        // input's granules whose documents' rows changed. What no round derived again stays as stored.
        let written = match &was {
            Some(was) => {
                let derived: BTreeSet<String> = self.program.rules.iter().map(|r| r.head.rel.to_string()).collect();
                let clean = derived.into_iter().filter(|r| next.kept(r).is_none()).collect();
                let cut = retain::Cut { clean, strata: (0, 0), changed: Vec::new() };
                let sources: Vec<(String, Arc<dyn Source>)> = bound.iter().map(|(r, (s, _))| (r.clone(), Arc::clone(s))).collect();
                let digests: std::collections::HashMap<String, u64> = bound.iter().map(|(r, (_, d))| (r.clone(), *d)).collect();
                next.write_frames(on.store, was, &self.program, &retain::Locations::new(), &crate::code_rule_dsl::logic::front::strata(&self.program).map_err(|e| super::Error::Plan(e.to_string()))?.0, &cut, &sources, &digests, &BTreeSet::new(), &HashMap::new())?
            }
            None => 0,
        };
        let mut produced: Vec<(String, Vec<Vec<i64>>)> = self.produced.iter().filter(|(rel, _)| !self.fs.iter().any(|(_, i)| i == rel)).map(|(rel, _)| (rel.clone(), bound.get(rel).map(|(s, _)| rows(s.as_ref())).unwrap_or_default())).collect();
        // The file system's answers as the last round derived them: each request's rows of the file system.
        for (answer, _) in &self.fs {
            produced.push((answer.clone(), next.read_derived(on.store, answer)?.ok_or_else(|| Gave::Up(format!("{answer} is not kept")))?));
        }
        let world = world_digest(|rel| bound.get(rel).map(|(_, d)| *d));
        Ok(Done { next, rounds, strata, maintained: maintained_n, derived_again: again, again_why, still: still_n, world, withdrawn, written, produced, answered })
    }

    /// Where the rounds keep an answer's rows: the file system's relation for one of its answers (`fs`), else the
    /// answer's own.
    fn kept_as(&self, rel: &str) -> String {
        self.fs.iter().find(|(a, _)| a == rel).map_or_else(|| rel.to_owned(), |(_, i)| i.clone())
    }

    /// The ask relations, by gate.
    pub(in crate::code_rule_dsl) fn asks(&self) -> impl Iterator<Item = &String> {
        self.gates.iter().filter_map(|(_, a)| a.as_ref())
    }
}
