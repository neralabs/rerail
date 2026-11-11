//! A program's waves over a batch of documents: the chunks the transducer's demand drives (`demand::driver::with_batch`),
//! their families and derivation records.

use super::*;
use crate::code_rule_dsl::demand::vtab::PackedDemand;

type Built = StoreResult<(Vec<Document>, Vec<(&'static str, Arc<dyn Source>)>)>;

/// Runs `program` over `documents` of `scope` on `lease` to its fixpoint, then records each derived
/// relation as a family. `demand` gives a chunk's packed batch, if its steps read one. The
/// relations stay readable on the lease until it is dropped.
pub(in crate::code_rule_dsl) fn run(
    lease: &Lease<'_>,
    graph: &(impl Graph + ?Sized),
    program: &Program,
    scope: &str,
    documents: Vec<Document>,
    limits: &Limits,
    demand: &mut dyn FnMut(&[Document]) -> StoreResult<Option<PackedDemand>>,
) -> StoreResult<Outcome> {
    let inputs = documents.iter().map(|d| DerivationInput { name: Some(d.core.clone()), digest: d.digest }).collect();
    run_over(lease, graph, program, scope, documents, inputs, limits, demand)
}

/// `run` over `documents`, its families recorded as read from `inputs`: a warm run evaluates the
/// documents that changed, its relations holding the others' rows as kept (06 S6), and its
/// families read every document.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn run_over(
    lease: &Lease<'_>,
    graph: &(impl Graph + ?Sized),
    program: &Program,
    scope: &str,
    documents: Vec<Document>,
    mut inputs: Vec<DerivationInput>,
    limits: &Limits,
    demand: &mut dyn FnMut(&[Document]) -> StoreResult<Option<PackedDemand>>,
) -> StoreResult<Outcome> {
    declare(&**lease, program,false)?;
    // A program that reads a `py.*` family reads it as a fact of its own (decision 0029).
    if program.steps.iter().any(|step| step.reads.iter().any(|read| crate::code_rule_dsl::facts::vtab::syntax::is_family(read))) {
        inputs.extend(crate::code_rule_dsl::facts::families::derived_inputs(graph, scope, &documents)?);
    }
    inputs.sort_by(|a, b| a.name.cmp(&b.name));
    inputs.dedup_by(|a, b| a.name == b.name);
    let planned = chunks(documents, limits.chunk_bytes, program.file_local);
    let mut outcome = Outcome { chunks: planned.len(), ..Outcome::default() };
    let store = graph.store();
    // Chunk k+1's sources are built as a job on the pool while chunk k runs; the run takes them once built (running
    // the pool's jobs meanwhile), and starts chunk k+2's.
    let next: parking_lot::Mutex<Option<Built>> = parking_lot::Mutex::new(None);
    let mut planned = planned.into_iter();
    crate::threads::scope(|jobs| -> StoreResult<()> {
        let next = &next;
        let start = |chunk: Vec<Document>| {
            jobs.spawn("a wave chunk's sources", move |_| {
                let built = sources(store, chunk.clone()).map(|s| (chunk, s));
                *next.lock() = Some(built);
            })
        };
        let mut started = planned.next().map(start).is_some();
        while started {
            let built = rerail_accel::exec::help_until_as("a wave chunk: its sources", || next.lock().take());
            started = planned.next().map(start).is_some();
            let (docs, relations) = built?;
            fresh(graph, &docs)?;
            lease.execute_batch("DELETE FROM temp.wave_document")?;
            {
                let mut position = lease.prepare_cached("INSERT INTO temp.wave_document(position, d) VALUES (?1, ?2)")?;
                for (at, d) in docs.iter().enumerate() {
                    position.execute(rusqlite::params![at as i64, d.key])?;
                }
            }
            for (name, source) in relations {
                lease.bind(name, source).map_err(sql)?;
            }
            // The chunk's batch through the one driver (`demand::driver`, list step 20): a batch
            // the steps read that did not complete fails its chunk.
            let reads = program.steps.iter().any(|step| step.sql.contains(":batch"));
            let status = reads.then_some((&**lease as &rusqlite::Connection, program.name.as_str()));
            outcome.waves += crate::code_rule_dsl::demand::driver::with_batch(crate::code_rule_dsl::demand::vtab::registry_of(lease.attached.as_deref())?, demand(&docs)?, status, |token| {
                waves(&**lease, program, token, limits.max_waves, docs.is_empty())
            })?;
        }
        Ok(())
    })?;
    record(lease, graph, program, scope, inputs, outcome)
}

/// Runs `program`, whose steps read only tables already on `lease` (no documents' sources, no
/// batch), once to its fixpoint, then records each derived relation as a family read from
/// `inputs`: a layer over another program's relations (06 S5's findings, architecture §8).
pub(in crate::code_rule_dsl) fn layer(
    lease: &Lease<'_>,
    graph: &(impl Graph + ?Sized),
    program: &Program,
    scope: &str,
    mut inputs: Vec<DerivationInput>,
    limits: &Limits,
) -> StoreResult<Outcome> {
    declare(&**lease, program,false)?;
    inputs.sort_by(|a, b| a.name.cmp(&b.name));
    let waves = waves(&**lease, program, None, limits.max_waves, false)?;
    record(lease, graph, program, scope, inputs, Outcome { waves, chunks: 1, ..Outcome::default() })
}

/// Records each derived relation of `program` (and each part of a partitioned one) as a family
/// read from `inputs`; `outcome.changed` names those whose digest changed.
fn record(
    lease: &Lease<'_>,
    graph: &(impl Graph + ?Sized),
    program: &Program,
    scope: &str,
    inputs: Vec<DerivationInput>,
    mut outcome: Outcome,
) -> StoreResult<Outcome> {
    outcome.rows = lease.storage().borrow_mut().statistics_boundary(lease)?;
    let producer = program.digest();
    let record = |output: String, (digest, rows): ([u8; 32], i64)| DerivationRecord {
        output,
        producer,
        inputs: inputs.clone(),
        digest,
        status: 0,
        cost: rows,
    };
    let mut records = Vec::new();
    for (rel, arity) in &program.relations {
        records.push(record(family_name(scope, &program.name, rel), rows_digest(lease, rel, *arity, None)?));
        for (_, names) in program.partitions.iter().filter(|(partitioned, _)| partitioned == rel) {
            for (part, name) in names.iter().enumerate() {
                let rows = rows_digest(lease, rel, *arity, Some(part as i64))?;
                records.push(record(family_name(scope, &program.name, &format!("{rel}:{name}")), rows));
            }
        }
    }
    outcome.changed = graph.record_derivations(&records)?;
    Ok(outcome)
}