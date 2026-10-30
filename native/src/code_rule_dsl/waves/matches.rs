//! The first slice's wave (06-first-slice S4): a scope's lowered Python clauses as one file-local
//! wave program with one step. Per chunk of documents, `demand` selects each clause's candidates
//! with the prefilter (`prefilter::select`, SQL over the tapes' published rows), evaluates every
//! clause's label passes over the chunk's tapes together (`LoweredWave`), and registers the
//! clauses' root programs from those candidates as the chunk's `demand(batch)` (the CPU JIT, over
//! the chunk's files in one execution; it tests each label as one bit of the batch, `PackedBatch::derived`). The step
//! reads the batch's terminal matches into `match(clause, d, node)`, and each matched node's bytes
//! into `span(d, node, start, end)` for the findings layer (`findings`, S5); the planner emits them
//! from `SOURCE`, over two batch inputs (O14, `logic::plan::emit_batch`). The loop records the relation as one
//! fact family per clause (`partitions`) and `statistics` writes their row estimates to
//! `fact_stats`, where the demand relations read them.

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
use crate::code_rule_dsl::transducer::packed::{ExecutionBackend, LoweredMatcher, LoweredWave};
use crate::content_store::{StoreError, StoreResult};
use std::collections::BTreeSet;
use std::sync::Arc;

pub(in crate::code_rule_dsl) const PROGRAM: &str = "matches";
const RELATION: &str = "match";
/// The demand relation the step reads, whose row estimate `statistics` writes.
const TERMINAL: &str = "rerail_terminal_match";
/// The wave's program: a batch's terminal matches by document, and the matched nodes' bytes.
/// `terminal` is the chunk's `rerail_terminal_match` (file, clause, node, start byte, end byte);
/// `document` is `temp.wave_document` (the batch's file index, the document's key).
const SOURCE: &str = "
.decl terminal(f: int, c: int, n: int, s: int, e: int)
.decl document(f: int, d: int)
.input terminal, document
.decl match(c: int, d: int, n: int)
.decl span(d: int, n: int, s: int, e: int)
.output match, span
match($c, $d, $n) :- terminal($f, $c, $n, _, _), document($f, $d).
span($d, $n, $s, $e) :- terminal($f, _, $n, $s, $e), document($f, $d).
";

/// One lowered clause of the wave: its name (`policy/clause`), the prefilter's plan for its roots
/// and its lowered matcher.
pub(in crate::code_rule_dsl) struct Clause {
    pub name: String,
    pub plan: Arc<AnchorPlan>,
    pub matcher: LoweredMatcher,
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
    lowered: LoweredWave,
    lang: u32,
    /// Each chunk's candidates, in order.
    pub chunks: Vec<Chunk>,
}

fn invalid(error: impl std::fmt::Debug) -> StoreError {
    StoreError::Invalid(format!("{error:?}"))
}

impl Wave {
    pub(in crate::code_rule_dsl) fn new(clauses: &[Clause], backend: ExecutionBackend) -> StoreResult<Self> {
        let matchers = clauses.iter().map(|clause| &clause.matcher).collect::<Vec<_>>();
        Ok(Self {
            backend,
            names: clauses.iter().map(|clause| clause.name.clone()).collect(),
            plans: clauses.iter().map(|clause| Arc::clone(&clause.plan)).collect(),
            lowered: LoweredWave::new(&matchers).map_err(invalid)?,
            lang: u32::from(tape_tag(GrammarDialect::Python).expect("Python has a tape")),
            chunks: Vec::new(),
        })
    }

    pub(in crate::code_rule_dsl) fn lowered(&self) -> &LoweredWave {
        &self.lowered
    }

    /// The wave program: the planner's step that reads the chunk's batch into `match`.
    /// File-local, so one wave per chunk. A batch holds at least one match per clause, the
    /// terminal input's estimate; a chunk has documents in the hundreds at most.
    pub(in crate::code_rule_dsl) fn program(&self, store: &ContentStore) -> StoreResult<Program> {
        let batch = HashMap::from([
            (
                "terminal".to_owned(),
                (
                    format!("(SELECT file_index AS c0, producer_id AS c1, node AS c2, start_byte AS c3, end_byte AS c4 FROM {TERMINAL} WHERE batch = :batch)"),
                    self.names.len() as f64,
                ),
            ),
            ("document".to_owned(), ("(SELECT position AS c0, d AS c1 FROM temp.wave_document)".to_owned(), 100.0)),
        ]);
        let intern = &mut |names: &[&str]| store.intern_atoms(names).map_err(|e| crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string()));
        let mut program = emit::emit_batch(SOURCE, PROGRAM, &batch, intern).map_err(invalid)?.program;
        program.file_local = true;
        program.partitions = vec![(RELATION.to_owned(), self.names.clone())];
        Ok(program)
    }

    /// The chunk's batch: its documents' stored tapes, in order, labelled by every clause's
    /// passes, and each clause's root program anchored at its prefilter candidates of its root's
    /// kinds.
    pub(in crate::code_rule_dsl) fn demand(
        &mut self,
        graph: &(impl Graph + ?Sized),
        scope: &str,
        documents: &[Document],
    ) -> StoreResult<Option<PackedDemand>> {
        let tapes = documents
            .iter()
            .map(|document| {
                Families::stored(graph, scope, &document.path)?
                    .ok_or_else(|| StoreError::Invalid(format!("{} has no stored tape", document.path)))
            })
            .collect::<StoreResult<Vec<Arc<Tape>>>>()?;
        let candidates = self.candidates(graph, &tapes)?;
        let docs = tapes.iter().map(|tape| tape.doc.clone()).collect::<Vec<_>>();
        // A clause's labels are needed only where its root program starts.
        let active = candidates.iter().map(|clause| clause.iter().map(|nodes| !nodes.is_empty()).collect()).collect::<Vec<Vec<bool>>>();
        let views = match self.backend {
            #[cfg(target_os = "macos")]
            ExecutionBackend::MetalInterpreter | ExecutionBackend::MetalJit | ExecutionBackend::MetalBackground => {
                self.lowered.labelled_metal(&docs, &active).map_err(invalid)?.0
            }
            _ => self.lowered.labelled(&docs, &active).map_err(invalid)?,
        };
        let programs = self.lowered.programs();
        let anchors = (0..views.len())
            .map(|file| candidates.iter().map(|clause| clause[file].clone()).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let grid = vec![vec![true; programs.len()]; views.len()];
        let limits = vec![u32::MAX; programs.len()];
        let demand =
            PackedDemand::new(self.backend, programs, views, anchors, grid, limits).map_err(invalid)?;
        self.chunks.push(Chunk { candidates });
        Ok(Some(demand))
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
        let rows = crate::code_rule_dsl::prefilter::select(graph.store(), self.lang, &plans, &ids)?;
        let position = |row: i64| ids.iter().position(|id| id.row() == row);
        rows.into_iter()
            .enumerate()
            .map(|(clause, rows)| {
                let mut out = vec![BTreeSet::new(); tapes.len()];
                for (row, node) in rows {
                    let file = position(row).ok_or_else(|| StoreError::Corrupt(format!("candidate of tape {row} outside the chunk")))?;
                    if self.lowered.starts(clause, &tapes[file].doc, node) {
                        out[file].insert(node);
                    }
                }
                Ok(out.into_iter().map(|nodes| nodes.into_iter().collect()).collect())
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
        out.push(Document { key, path: path.clone(), core, digest, facts: Arc::new(facts) });
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
