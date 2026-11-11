//! The first slice's wave (06-first-slice S4): a scope's lowered Python clauses as one file-local
//! wave program with one step. Per chunk of documents, `demand` selects each clause's candidates
//! with the prefilter (`prefilter::select`, SQL over the tapes' published rows), evaluates every
//! clause's label passes over the chunk's tapes together (`LoweredWave`), and registers the
//! clauses' root programs from those candidates as the chunk's `demand(batch)` (the CPU JIT, over
//! the chunk's files in one execution; it tests each label as one bit of the batch, `PackedBatch::derived`). The step
//! reads the batch's terminal matches into `matched(clause, d, node)`, and each matched node's bytes
//! into `span(d, node, start, end)` for the findings layer (`findings`, S5); the planner emits them
//! from `SOURCE`, over two batch inputs (O14, `logic::plan::emit_batch`). The loop records the relation as one
//! fact family per clause (`partitions`) and `statistics` writes their row estimates to
//! `fact_stats`, where the demand relations read them.

use crate::code_rule_dsl::logic::sql::select::Source;
use super::owner::Lease;
use super::schedule::Program;
use crate::code_rule_dsl::logic::plan::emit;
use crate::content_store::ContentStore;
use std::collections::HashMap;
use crate::code_rule_dsl::demand::vtab::PackedDemand;
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::core;
use crate::code_rule_dsl::facts::families::{Families, Tape, core_name};
use crate::code_rule_dsl::facts::observe::path_name;
use crate::code_rule_dsl::facts::vtab::syntax::Document;
use crate::code_rule_dsl::language::{GrammarDialect, tape_tag};
use crate::code_rule_dsl::matcher::AnchorPlan;
use crate::code_rule_dsl::transducer::packed::{ExecutionBackend, LoweredMatcher, LoweredWave, PlanId};
use crate::content_store::{StoreError, StoreResult};
#[cfg(test)]
use std::collections::BTreeSet;
use std::sync::Arc;

pub(in crate::code_rule_dsl) const PROGRAM: &str = "matches";
const RELATION: &str = "matched";
/// The demand relation the step reads, whose row estimate `statistics` writes.
const TERMINAL: &str = "rerail_terminal_match";
/// The wave's program: a batch's terminal matches by document, and the matched nodes' bytes.
/// `terminal` is the chunk's `rerail_terminal_match` (file, clause, node, start byte, end byte);
/// `document` is `temp.wave_document` (the batch's file index, the document's key).
const SOURCE: &str = "
.decl terminal(f: number, c: number, n: number, s: number, e: number)
.decl document(f: number, d: number)
.input terminal, document
.decl matched(c: number, d: number, n: number)
.decl span(d: number, n: number, s: number, e: number)
.output matched, span
matched(c, d, n) :- terminal(f, c, n, _, _), document(f, d).
span(d, n, s, e) :- terminal(f, _, n, s, e), document(f, d).
";
/// A warm run's rules besides `SOURCE`'s (06 S6): the rows of the documents it does not evaluate,
/// kept from the run before (`KEPT_MATCH`, `KEPT_SPAN`), so its relations, and the families
/// recorded from them, are of every document.
const KEPT: &str = "
.decl kept_match(c: number, d: number, n: number)
.decl kept_span(d: number, n: number, s: number, e: number)
.input kept_match, kept_span
matched(c, d, n) :- kept_match(c, d, n).
span(d, n, s, e) :- kept_span(d, n, s, e).
";
/// The kept rows a warm run reads, filled by its caller (`slice`).
pub(in crate::code_rule_dsl) const KEPT_MATCH: &str = "temp.wave_kept_match";
pub(in crate::code_rule_dsl) const KEPT_SPAN: &str = "temp.wave_kept_span";

/// One lowered clause of the wave: its name (`policy/clause`), the prefilter's plan for its roots
/// and its lowered matcher.
pub(in crate::code_rule_dsl) struct Clause {
    pub name: String,
    /// Its policy's id and its own: the key of its violations (`violation("<policy>", "<clause>", …)`).
    pub key: (String, String),
    pub plan: Arc<AnchorPlan>,
    pub matcher: LoweredMatcher,
    /// The conditions on the document's path its rule's root holds (`gated_match_tree`): the clause
    /// runs only on documents whose path meets every one.
    pub paths: Vec<crate::code_rule_dsl::matcher::DocumentGate>,
}

/// What `demand` did for one chunk, for the gates.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Chunk {
    /// Per clause, per document of the chunk: the candidates the root program starts from.
    pub candidates: Vec<Vec<Vec<u32>>>,
}

pub(in crate::code_rule_dsl) struct Wave {
    /// Where the labels and root programs run: the CPU or Metal, compiled now or in the
    /// background (`compilation`).
    backend: ExecutionBackend,
    names: Vec<String>,
    plans: Vec<Arc<AnchorPlan>>,
    /// Each clause's conditions on the document's path (`Clause::paths`).
    paths: Vec<Vec<crate::code_rule_dsl::matcher::DocumentGate>>,
    lowered: LoweredWave,
    /// The lowered wave's plan identity (`LoweredWave::identity`): its roots' batches are its parts.
    plan: PlanId,
    lang: u32,
    /// How the candidates are selected (`Candidates`).
    pub(in crate::code_rule_dsl) route: Candidates,
    /// The policies' rules over the clauses, and their demand (`policies`, list step 21).
    logic: super::policies::Logic,
    /// The demand statement, planned on the run's store when its program is (`Logic::demand_sql`).
    demand_sql: std::sync::Mutex<Option<Option<String>>>,
    /// Each chunk's candidates, in order.
    pub chunks: Vec<Chunk>,
}

/// How a wave selects its candidates: the plans' sets over the batch's node space on the wave's
/// device (decision 0024 §2), or the pure-SQL fallback (03 §17, `prefilter::fallback`): the same
/// plans lowered to SQLite over `nera_tree`, `RERAIL_CANDIDATES=sql`. Both are supersets of the
/// roots, exact where the plans are; the walks confirm them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum Candidates {
    Sets,
    Sql,
}

impl Candidates {
    fn from_environment() -> Self {
        if std::env::var("RERAIL_CANDIDATES").is_ok_and(|route| route == "sql") { Self::Sql } else { Self::Sets }
    }
}

/// Each clause's candidates kept only on the documents it is demanded on.
fn restrict(candidates: &mut [Vec<Vec<u32>>], demanded: Option<&[Vec<bool>]>) {
    let Some(demanded) = demanded else { return };
    for (clause, on) in candidates.iter_mut().zip(demanded) {
        for (nodes, &on) in clause.iter_mut().zip(on) {
            if !on {
                *nodes = Vec::new();
            }
        }
    }
}

fn invalid(error: impl std::fmt::Debug) -> StoreError {
    StoreError::Invalid(format!("{error:?}"))
}


/// The process's planned wave programs, in memory before the store (`stats::plans`' bounded cache).
static WAVE_PLANS: std::sync::LazyLock<std::sync::Mutex<crate::code_rule_dsl::stats::plans::Plans>> = std::sync::LazyLock::new(Default::default);

impl Wave {
    /// The wave of `clauses`, each its own forbid rule demanded everywhere (`Logic::per_clause`).
    pub(in crate::code_rule_dsl) fn new(clauses: &[Clause], backend: ExecutionBackend) -> StoreResult<Self> {
        Self::with_logic(clauses, super::policies::Logic::per_clause(clauses), backend)
    }

    /// Whether the demand statement reads a `py.*` family: the wave then reads it before its batch.
    pub(in crate::code_rule_dsl) fn demand_reads_py(&self) -> bool {
        let py = |rel: &str| crate::code_rule_dsl::facts::vtab::syntax::is_family(rel);
        self.logic.demand.as_ref().is_some_and(|p| p.rules.iter().any(|r| r.body.iter().any(|l| l.atom().is_some_and(|a| py(&a.rel)))))
    }

    /// The wave of `clauses` running the policies' rules `logic` (`policies::lower`).
    pub(in crate::code_rule_dsl) fn with_logic(clauses: &[Clause], logic: super::policies::Logic, backend: ExecutionBackend) -> StoreResult<Self> {
        #[cfg(target_os = "macos")]
        if matches!(backend, ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit | ExecutionBackend::MetalBackground) {
            LoweredWave::warm_metal();
        }
        let matchers = clauses.iter().map(|clause| &clause.matcher).collect::<Vec<_>>();
        let lowered = LoweredWave::new(&matchers).map_err(invalid)?;
        Ok(Self {
            backend,
            names: clauses.iter().map(|clause| clause.name.clone()).collect(),
            plans: clauses.iter().map(|clause| Arc::clone(&clause.plan)).collect(),
            paths: clauses.iter().map(|clause| clause.paths.clone()).collect(),
            plan: lowered.identity(),
            lowered,
            lang: u32::from(tape_tag(GrammarDialect::Python).expect("Python has a tape")),
            route: Candidates::from_environment(),
            logic,
            demand_sql: std::sync::Mutex::new(None),
            chunks: Vec::new(),
        })
    }

    pub(in crate::code_rule_dsl) fn lowered(&self) -> &LoweredWave {
        &self.lowered
    }

    /// The wave as `rir` (list step 18, `rerail engine dump`), its kinds and fields named by
    /// its grammar: the lowered wave (`LoweredWave::dump`; with `fused` its plan, packs and
    /// groups), and with `setplan` its candidates' set plan (`prefilter::sets::dump`).
    pub(in crate::code_rule_dsl) fn dump(&self, fused: bool, setplan: bool) -> StoreResult<String> {
        let grammar = crate::code_rule_dsl::transducer::packed::rir::Names::of(GrammarDialect::Python);
        let mut out = self.lowered.dump(&self.names, Some(&grammar), fused).map_err(invalid)?;
        if setplan {
            let plans = self.plans.iter().map(|plan| plan.as_ref()).collect::<Vec<_>>();
            out.push_str(&crate::code_rule_dsl::prefilter::sets::dump(&plans, Some(&grammar)));
        }
        Ok(out)
    }

    /// The wave program: the planner's step that reads the chunk's batch into `match`.
    /// File-local, so one wave per chunk. A batch holds at least one match per clause, the
    /// terminal input's estimate; a chunk has documents in the hundreds at most.
    pub(in crate::code_rule_dsl) fn program(&self, store: &ContentStore) -> StoreResult<Program> {
        self.emit(store, false)
    }

    /// The wave program of a warm run: `program`'s, with each relation also holding the kept rows
    /// of the documents it does not evaluate (`KEPT_MATCH`, `KEPT_SPAN`).
    pub(in crate::code_rule_dsl) fn kept_program(&self, store: &ContentStore) -> StoreResult<Program> {
        self.emit(store, true)
    }

    /// The planned program, from `compiled_plans` while its decisions hold (list step 20): it is
    /// keyed by a `CompiledPlanId` over the planner's inputs, and it reads each batch input's row
    /// estimate, valid while each stays in its log2 bucket. Otherwise it is planned and recorded.
    fn emit(&self, store: &ContentStore, kept: bool) -> StoreResult<Program> {
        // The run's demand statement, its atoms this store's.
        *self.demand_sql.lock().expect("demand statement mutex poisoned") = Some(self.logic.demand_sql(store)?);
        let (source, batch) = self.inputs(kept);
        let mut identity = crate::code_rule_dsl::stats::identity::CompiledPlanId::builder()
            .text("rerail.wave-program/v1")
            .text(PROGRAM)
            .text(&source)
            .u64(crate::code_rule_dsl::stats::plans::COMPILER_VERSION as u64);
        let mut inputs = batch.iter().collect::<Vec<_>>();
        inputs.sort_by(|a, b| a.0.cmp(b.0));
        let mut reads = std::collections::BTreeMap::new();
        for (name, (from, estimate)) in &inputs {
            identity = identity.text(name).text(&from.render());
            reads.insert(format!("batch.{name}"), *estimate as u64);
        }
        for name in &self.names {
            identity = identity.text(name);
        }
        let id = *identity.finish().as_bytes();
        let held = store.candidate_transaction(false, |db| WAVE_PLANS.lock().expect("wave plans mutex poisoned").valid(db, &[id], &reads))?;
        if let Some(held) = held.get(&id) {
            // The SQL holds this store's atom ids: a plan is reused only where the atoms it was
            // planned with have the same ids here (its decisions record them).
            let atoms = serde_json::from_str::<Vec<(String, u32)>>(&held.decisions).unwrap_or_default();
            let names = atoms.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
            let same = store.intern_atoms(&names)?.into_iter().eq(atoms.iter().map(|(_, atom)| *atom));
            if let (true, Ok(program)) = (same, serde_json::from_slice::<Program>(&held.program)) {
                crate::code_rule_dsl::prof::count("wave.plan.reused", 1);
                return Ok(program);
            }
        }
        let mut atoms = Vec::<(String, u32)>::new();
        let intern = &mut |names: &[&str]| {
            let found = store.intern_atoms(names).map_err(|e| crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string()))?;
            atoms.extend(names.iter().map(|name| (*name).to_owned()).zip(found.iter().copied()));
            Ok(found)
        };
        let mut program = emit::emit_batch(&source, PROGRAM, &batch, intern).map_err(invalid)?.program;
        program.file_local = true;
        program.partitions = vec![(RELATION.to_owned(), self.names.clone())];
        let bytes = serde_json::to_vec(&program).map_err(|error| invalid(error.to_string()))?;
        atoms.sort();
        atoms.dedup();
        let decisions = serde_json::to_string(&atoms).map_err(|error| invalid(error.to_string()))?;
        store.candidate_transaction(true, |db| {
            let compiled = crate::code_rule_dsl::stats::plans::Compiled { decisions: decisions.clone(), program: bytes.clone() };
            WAVE_PLANS.lock().expect("wave plans mutex poisoned").put(db, id, compiled, &reads).map(|_| ())
        })?;
        crate::code_rule_dsl::prof::count("wave.plan.compiled", 1);
        Ok(program)
    }

    /// Per clause, per document of the chunk: whether the policies' rules demand the clause there
    /// (`policies`: the magic rules seeded by the chunk's documents in `temp.wave_document`, run
    /// before the batch). `None`: every clause everywhere.
    fn demanded(&self, lease: &Lease<'_>, store: &ContentStore, documents: &[Document]) -> StoreResult<Option<Vec<Vec<bool>>>> {
        let sql = {
            let mut held = self.demand_sql.lock().expect("demand statement mutex poisoned");
            if held.is_none() {
                *held = Some(self.logic.demand_sql(store)?);
            }
            held.clone().flatten()
        };
        let Some(sql) = sql else { return Ok(None) };
        let position = documents.iter().enumerate().map(|(at, d)| (i64::from(d.key), at)).collect::<HashMap<_, _>>();
        let mut demanded = vec![vec![false; documents.len()]; self.plans.len()];
        let mut statement = lease.prepare_cached(&sql)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let (clause, d): (i64, i64) = (row.get(0)?, row.get(1)?);
            if let (Some(clause), Some(&file)) = (demanded.get_mut(clause as usize), position.get(&d)) {
                clause[file] = true;
            }
        }
        crate::code_rule_dsl::prof::count("demand.pairs", demanded.iter().flatten().filter(|&&on| on).count());
        Ok(Some(demanded))
    }

    /// Each clause kept off the documents that fail one of its document gates (`Clause::paths`).
    fn gate(&self, demanded: &mut Option<Vec<Vec<bool>>>, documents: &[Document], tapes: &[Arc<Tape>]) {
        if self.paths.iter().all(Vec::is_empty) {
            return;
        }
        let demanded = demanded.get_or_insert_with(|| vec![vec![true; documents.len()]; self.plans.len()]);
        let sources = tapes.iter().map(|tape| ast_grep_core::Doc::get_source(&tape.doc)).collect::<Vec<_>>();
        for (on, gates) in demanded.iter_mut().zip(&self.paths) {
            for ((on, document), source) in on.iter_mut().zip(documents).zip(&sources) {
                *on = *on && gates.iter().all(|gate| gate.holds(&document.path, source.as_str()));
            }
        }
    }

    /// The planner's source and the batch inputs it reads, each with its source and estimate.
    fn inputs(&self, kept: bool) -> (String, HashMap<String, (Source, f64)>) {
        let mut batch = HashMap::from([
            (
                "terminal".to_owned(),
                (
                    Source::positional(TERMINAL, &["file_index", "producer_id", "node", "start_byte", "end_byte"]).batched(),
                    self.names.len() as f64,
                ),
            ),
            ("document".to_owned(), (Source::positional("temp.wave_document", &["position", "d"]), 100.0)),
        ]);
        // The relations the rules read as bound with the chunk, priced at a chunk's size: a few hundred
        // documents, their nodes in the hundreds of thousands. Priced low, the planner scans one first
        // and probes the rules' relations per row.
        for (rel, columns) in &self.logic.bound {
            let rows = match rel.as_str() {
                "doc" | "pkg" => 300.0,
                "intval" | "op" => 1e4,
                rel if rel.starts_with("py_") => 1e5,
                _ => 1e6,
            };
            batch.insert(rel.clone(), (Source::named(&format!("main.{rel}"), columns), rows));
        }
        let mut source = SOURCE.to_owned();
        source.push_str(&self.logic.rules);
        if kept {
            source.push_str(KEPT);
            batch.insert("kept_match".to_owned(), (Source::positional(KEPT_MATCH, &["c0", "c1", "c2"]), 1000.0));
            batch.insert("kept_span".to_owned(), (Source::positional(KEPT_SPAN, &["c0", "c1", "c2", "c3"]), 1000.0));
        }
        (source, batch)
    }

    /// The chunk's batch: its documents' stored tapes, in order, labelled by every clause's
    /// passes, and each clause's root program anchored at its prefilter candidates of its root's
    /// kinds.
    pub(in crate::code_rule_dsl) fn demand(
        &mut self,
        lease: &Lease<'_>,
        graph: &(impl Graph + ?Sized),
        scope: &str,
        documents: &[Document],
    ) -> StoreResult<Option<PackedDemand>> {
        // An empty chunk (a warm run with nothing to evaluate) has no batch.
        if documents.is_empty() {
            return Ok(None);
        }
        let mut demanded = crate::code_rule_dsl::prof::time("demand.magic", || self.demanded(lease, graph.store(), documents))?;
        let tapes = crate::code_rule_dsl::prof::time("demand.tapes", || {
            documents
                .iter()
                .map(|document| {
                    Families::stored(graph, scope, &document.path)?
                        .ok_or_else(|| StoreError::Invalid(format!("{} has no stored tape", document.path)))
                })
                .collect::<StoreResult<Vec<Arc<Tape>>>>()
        })?;
        crate::code_rule_dsl::prof::time("demand.gates", || self.gate(&mut demanded, documents, &tapes));
        crate::code_rule_dsl::prof::release();
        crate::code_rule_dsl::prof::peak("1.tapes");
        // Each read at its path, which a `filePath` test inside a matcher tests.
        let docs = tapes.iter().zip(documents).map(|(tape, document)| tape.doc.with_path(&document.path)).collect::<Vec<_>>();
        // The walks' text tests from the term index (`R[tid]`, list step 17).
        let docs = crate::code_rule_dsl::prof::time("demand.texts", || self.texts(graph.store(), &tapes, docs))?;
        let metal = cfg!(target_os = "macos")
            && matches!(self.backend, ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit | ExecutionBackend::MetalBackground);
        let programs = self.lowered.programs();
        let mut found = None;
        let (candidates, views) = if metal {
            // On Metal the candidates come first (set steps on the device, ~150 ms), then every
            // clause's labels and its roots from those candidates in one run: the roots read the
            // labels in place and come back as match records (decision 0024 §3).
            let mut candidates = crate::code_rule_dsl::prof::time("demand.candidates", || self.candidates(graph, &tapes))?;
            restrict(&mut candidates, demanded.as_deref());
            crate::code_rule_dsl::prof::peak("2a.candidates");
            // Wave preparation (PIAT §20.3): what the residency lacks of the chunk's tapes is read
            // from the store in one query and decompressed into slabs; the label run then finds
            // every document resident.
            #[cfg(target_os = "macos")]
            crate::code_rule_dsl::prof::time("demand.residency", || prepare(graph.store(), &docs))?;
            crate::code_rule_dsl::prof::peak("2b.residency");
            let active = vec![vec![true; docs.len()]; self.plans.len()];
            let views = crate::code_rule_dsl::prof::time("demand.labels", || {
                #[cfg(target_os = "macos")]
                {
                    let jobs = crate::code_rule_dsl::transducer::packed::RootJobs { programs: programs.iter().collect(), starts: candidates.clone() };
                    let (views, records) = labelled_metal(&self.lowered, &docs, &active, &jobs).map_err(invalid)?;
                    found = Some(records);
                    Ok::<_, StoreError>(views)
                }
                #[cfg(not(target_os = "macos"))]
                self.lowered.labelled(&docs, &active).map_err(invalid)
            })?;
            (candidates, views)
        } else {
            let mut candidates = crate::code_rule_dsl::prof::time("demand.candidates", || self.candidates(graph, &tapes))?;
            restrict(&mut candidates, demanded.as_deref());
            crate::code_rule_dsl::prof::release();
            crate::code_rule_dsl::prof::peak("2.candidates");
            // A clause's labels are needed only where its root program starts.
            let active = candidates.iter().map(|clause| clause.iter().map(|nodes| !nodes.is_empty()).collect()).collect::<Vec<Vec<bool>>>();
            let views = crate::code_rule_dsl::prof::time("demand.labels", || self.lowered.labelled(&docs, &active).map_err(invalid))?;
            (candidates, views)
        };
        crate::code_rule_dsl::prof::release();
        crate::code_rule_dsl::prof::peak("3.labels");
        let anchors = (0..views.len())
            .map(|file| candidates.iter().map(|clause| clause[file].clone()).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let grid = vec![vec![true; programs.len()]; views.len()];
        let limits = vec![u32::MAX; programs.len()];
        // The roots run on the CPU whatever labelled the tapes: they start only at the prefilter's
        // candidates, few and scattered, and carry captures and outputs, where the labels are a pass
        // over every node (decision 0017; 06 S7: 1.2 s on the CPU's JIT, 4.3 s on Metal's).
        // Bound as the wave's: compiled now, or interpreted until the background binding of the
        // wave's plan is ready (list step 16).
        let roots = match self.backend {
            ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit => ExecutionBackend::CpuJit,
            ExecutionBackend::MetalBackground => ExecutionBackend::CpuBackground,
            backend => backend,
        };
        let mut demand = PackedDemand::new(roots, programs, views, anchors, grid, limits).map_err(invalid)?.with_plan(self.plan);
        if let Some(found) = found {
            demand = demand.with_records(found).map_err(invalid)?;
        }
        self.chunks.push(Chunk { candidates });
        Ok(Some(demand))
    }

    /// `docs` with every text test the wave's walks make read from the term index (03 §16 item
    /// 4, `prefilter::terms`): one slot per distinct test (`Micro::slot_key`). `RERAIL_TEXTS=scan`
    /// tests each node's text instead.
    fn texts(&self, store: &ContentStore, tapes: &[Arc<Tape>], docs: Vec<crate::code_rule_dsl::tape::TapeDoc>) -> StoreResult<Vec<crate::code_rule_dsl::tape::TapeDoc>> {
        use crate::code_rule_dsl::prefilter::terms::{TextSlot, rows};
        use crate::code_rule_dsl::transducer::packed::Micro;
        if std::env::var("RERAIL_TEXTS").is_ok_and(|mode| mode == "scan") {
            return Ok(docs);
        }
        let walks = self.lowered.walked_programs().into_iter().filter_map(|program| Micro::of(program).map(|walk| (walk, program))).collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        let mut slots = Vec::new();
        for (walk, program) in &walks {
            for slot in 0..walk.texts.len() {
                let key = walk.slot_key(program, slot);
                let Some(lowered) = walk.lowered(program, slot) else { continue };
                if !seen.insert(key.clone()) {
                    continue;
                }
                slots.push(TextSlot { key, lowered });
            }
        }
        let ids = tapes.iter().map(|tape| tape.id.row()).zip(&docs).collect::<Vec<_>>();
        let rows = rows(store, self.lang, &ids, &slots)?;
        Ok(docs.iter().zip(rows).map(|(doc, rows)| if rows.is_empty() { doc.clone() } else { doc.with_texts(rows) }).collect())
    }

    /// Per clause, per tape: the prefilter's candidates (one statement for every clause's plan over
    /// the tapes) that have the kinds the clause's root program starts from, ascending.
    pub(in crate::code_rule_dsl) fn candidates(
        &self,
        graph: &(impl Graph + ?Sized),
        tapes: &[Arc<Tape>],
    ) -> StoreResult<Vec<Vec<Vec<u32>>>> {
        let ids = tapes.iter().map(|tape| tape.id).collect::<Vec<_>>();
        let plans = self.plans.iter().map(Arc::as_ref).collect::<Vec<_>>();
        if self.route == Candidates::Sql {
            let rows = crate::code_rule_dsl::prof::time("candidates.sql", || {
                crate::code_rule_dsl::prefilter::fallback(graph.store(), self.lang, &plans, &ids)
            })?;
            // Rows are (file, preorder), ascending: each file's starts stay in order.
            return Ok(rows
                .into_iter()
                .enumerate()
                .map(|(clause, (rows, _exact))| {
                    let mut out = vec![Vec::new(); tapes.len()];
                    for (file, node) in rows {
                        if self.lowered.starts(clause, &tapes[file as usize].doc, node) {
                            out[file as usize].push(node);
                        }
                    }
                    crate::code_rule_dsl::prof::count("candidates.kept", out.iter().map(Vec::len).sum());
                    out
                })
                .collect());
        }
        // The plans' sets over the batch's node space, on the wave's device (decision 0024 §2).
        use crate::code_rule_dsl::prefilter::sets::Device;
        let device = match self.backend {
            #[cfg(target_os = "macos")]
            ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit | ExecutionBackend::MetalBackground => Device::Metal,
            _ => Device::Cpu,
        };
        let (space, sets) = crate::code_rule_dsl::prof::time("candidates.select", || {
            crate::code_rule_dsl::prefilter::select_sets(graph.store(), self.lang, &plans, &ids, device)
        })?;
        crate::code_rule_dsl::prof::peak("2a3.selected");
        let _compact = crate::code_rule_dsl::prof::Stage::start("candidates.compact");
        use rayon::prelude::*;
        // Each clause's starts per file: its set's nodes where its root program starts, ascending
        // (the set's order). Held through the labels and the roots: no growth slack.
        sets.into_par_iter()
            .enumerate()
            .map(|(clause, set)| {
                let out = (0..tapes.len())
                    .map(|file| {
                        let mut nodes = set.file_nodes(&space, file).filter(|&node| self.lowered.starts(clause, &tapes[file].doc, node)).collect::<Vec<_>>();
                        nodes.shrink_to_fit();
                        nodes
                    })
                    .collect::<Vec<_>>();
                crate::code_rule_dsl::prof::count("candidates.kept", out.iter().map(Vec::len).sum());
                Ok(out)
            })
            .collect()
    }

    /// Each clause family's row estimate in `fact_stats`, and the terminal relation's (their
    /// most), replacing the last run's: the demand relations read `max(rows)` per table.
    pub(in crate::code_rule_dsl) fn statistics(&self, lease: &Lease<'_>, scope: &str) -> StoreResult<()> {
        let mut most = 0_i64;
        for (part, name) in self.names.iter().enumerate() {
            let (rows, documents): (i64, i64) = lease.query_row(
                &format!("SELECT count(*), count(DISTINCT c1) FROM {RELATION} WHERE c0 = ?1"),
                [part as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            most = most.max(rows);
            let table = super::schedule::family_name(scope, PROGRAM, &format!("{RELATION}:{name}"));
            lease.execute(
                "INSERT OR REPLACE INTO fact_stats(tbl, col, rows, ndv, sq, sqd) VALUES (?1, 'd', ?2, ?3, 0, 0)",
                rusqlite::params![table, rows, documents],
            )?;
        }
        lease.execute(
            "INSERT OR REPLACE INTO fact_stats(tbl, col, rows, ndv, sq, sqd) VALUES (?1, 'producer_id', ?2, ?3, 0, 0)",
            rusqlite::params![TERMINAL, most.max(1), self.names.len() as i64],
        )?;
        Ok(())
    }
}

/// The documents of `paths` in `scope` whose core facts are bound, each keyed by its path name's
/// atom, its facts rebuilt from its stored tape.
pub(in crate::code_rule_dsl) fn documents(graph: &(impl Graph + ?Sized), scope: &str, paths: &[String]) -> StoreResult<Vec<Document>> {
    let mut out = Vec::new();
    for path in paths {
        let core = core_name(scope, path);
        let Some(digest) = graph.binding(&core)? else { continue };
        let Some(tape) = Families::stored(graph, scope, path)? else { continue };
        let facts = core::from_tape(&tape.doc, Arc::clone(&tape.lines)).map_err(StoreError::Invalid)?;
        if core::digest(&facts) != digest {
            return Err(StoreError::Invalid(format!("stale core facts for {core}")));
        }
        let key = graph.store().atom(&path_name(scope, path))?.ok_or_else(|| StoreError::Corrupt(format!("{path} has no atom")))?;
        out.push(Document { key, path: path.clone(), core, digest, facts: Arc::new(facts), tape: None, guest: None, module: None });
    }
    Ok(out)
}

/// The tapes labelled on Metal, all at once or, where their label levels outgrow a workspace
/// (06 S7: 250 files in one chunk) or the evaluation's launch bound, each half on its own: a file's labels read only that file.
/// Their roots' records come with them (`RootsFound`): a half's program that did not run on the
/// device makes the program's records `None` for both.
#[cfg(target_os = "macos")]
fn labelled_metal(
    lowered: &LoweredWave,
    docs: &[crate::code_rule_dsl::tape::TapeDoc],
    active: &[Vec<bool>],
    jobs: &crate::code_rule_dsl::transducer::packed::RootJobs<'_>,
) -> Result<(Vec<crate::code_rule_dsl::tape::TapeDoc>, crate::code_rule_dsl::transducer::packed::RootsFound), crate::code_rule_dsl::transducer::packed::Error> {
    use crate::code_rule_dsl::transducer::packed::{Error as PackedError, RootJobs};
    match lowered.labelled_metal(docs, active, Some(jobs)) {
        // A pool or the launch bound the plan cannot fit (list step 13): the batch is split, still
        // on Metal (a half's chains, and so its rounds, are no longer than the whole's).
        Err(PackedError::BackendCapacity | PackedError::BackendLaunchLimit) if docs.len() > 1 => {
            let half = docs.len() / 2;
            let split = |range: std::ops::Range<usize>| active.iter().map(|clause| clause[range.clone()].to_vec()).collect::<Vec<_>>();
            let part = |range: std::ops::Range<usize>| RootJobs {
                programs: jobs.programs.clone(),
                starts: jobs.starts.iter().map(|clause| clause[range.clone()].to_vec()).collect(),
            };
            let (mut views, left) = labelled_metal(lowered, &docs[..half], &split(0..half), &part(0..half))?;
            let (right_views, right) = labelled_metal(lowered, &docs[half..], &split(half..docs.len()), &part(half..docs.len()))?;
            views.extend(right_views);
            let records = left
                .into_iter()
                .zip(right)
                .map(|(left, right)| left.zip(right).map(|(mut left, right)| {
                    left.extend(right);
                    left
                }))
                .collect();
            Ok((views, records))
        }
        labelled => labelled.map(|(views, outcome)| {
            let records = if outcome.records.is_empty() { vec![None; jobs.programs.len()] } else { outcome.records };
            (views, records)
        }),
    }
}

#[cfg(test)]
mod tests;

#[cfg(target_os = "macos")]
/// The chunk's documents admitted from the store by their tape digests (decision 0032). A grammar
/// is the dialect the store's tag names. Leases are not kept: the label run takes its own.
fn prepare(store: &ContentStore, docs: &[crate::code_rule_dsl::tape::TapeDoc]) -> StoreResult<()> {
    use crate::code_rule_dsl::{language, residency::slabs, tape::Grammar};
    // The stored bytes are read in place.
    let digests = docs.iter().map(|doc| doc.resident_key().map_err(StoreError::Invalid)).collect::<StoreResult<Vec<_>>>()?;
    let grammar = |id: i64| Grammar::new(u8::try_from(id).ok().and_then(language::from_tape_tag).ok_or("unknown grammar")?);
    slabs::admit(store, &digests, &grammar).map(drop).map_err(invalid)
}
