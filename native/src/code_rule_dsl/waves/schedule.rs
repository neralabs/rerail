//! The frontier loop (decision 0010; 03 §4, §7, §10 answer 1), replacing Data-layer-rule-
//! expressivity's `relational/batch.rs`.
//!
//! D drove each consumer from Rust. `Session::advance` returned `Sql`, `Native(demand)` or `Done`;
//! the batch looped over sessions, grouped native demands by footprint, and ran
//! `statistics_boundary` between frontiers. D also admitted sessions against 256 MB of SQL
//! residency. Of that, the frontier order is kept: populate, refresh statistics once, then run the
//! readers.
//!
//! What runs next is now a query. The steps, their reads and each wave's deltas are rows of
//! `temp.wave_step`, `temp.wave_read` and `temp.wave_delta`, and two fixed statements select a
//! wave's frontier: the pending steps of the lowest pending stratum. Rust executes the statements
//! SQL selected and folds each `new_` table with fixed SQL; it never runs per row (ledger 17).
//!
//! Step contract:
//! - A step's statement inserts into `new_<output>`.
//! - It reads `delta_<r>` for a relation `r` of its own stratum (semi-naive) and whole
//!   relations otherwise.
//! - A stratum runs only when no lower stratum has pending steps, so negation and aggregation
//!   read complete relations.
//! - A statement may read the chunk's `demand(batch)` relations through `:batch`. A batch's
//!   files are the chunk's documents in order: `temp.wave_document(position, d)` maps a
//!   relation's `file_index` to the document's key.
//!
//! Chunks:
//! - Documents run in chunks bounded by bytes (500 MB pools, 03 §10 answer 1).
//! - A worker thread builds chunk k+1's vtab sources while chunk k runs; the channel is a
//!   rendezvous, so at most two chunks are resident (E12).
//! - Only file-local programs are chunked.
//!
//! When the loop ends, each derived relation is recorded as a fact family: its digest over the
//! batch, read from the documents' core facts (03 §2, §14).

use super::owner::Lease;
use crate::code_rule_dsl::demand::vtab::PackedDemand;
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::facts::vtab::syntax::{Document, fresh, sources};
use crate::content_store::{DerivationInput, DerivationRecord, StoreError, StoreResult};
use rusqlite::OptionalExtension as _;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// One statement of a wave program.
pub(in crate::code_rule_dsl) struct Step {
    pub output: String,
    pub stratum: u32,
    /// The relations the statement reads: derived ones by name (whole or `delta_`), base ones by
    /// table.
    pub reads: Vec<String>,
    pub sql: String,
}

pub(in crate::code_rule_dsl) struct Program {
    pub name: String,
    /// Derived relations and their arity. Every column is an integer (decision 0009), and the
    /// whole row is the key.
    pub relations: Vec<(String, usize)>,
    pub steps: Vec<Step>,
    /// Every step joins rows of one document at a time, so chunks of documents run apart.
    pub file_local: bool,
    /// Relations recorded as one family per value of their first column besides their own: the
    /// i-th name's family holds the rows whose first column is i (a match-fact family per rule
    /// clause, 06 S4).
    pub partitions: Vec<(String, Vec<String>)>,
}

impl Program {
    /// The producer of the program's families: its relations and statements.
    fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"rerail.wave/v1\0");
        for (rel, arity) in &self.relations {
            hash.update(format!("{rel}/{arity}\0").as_bytes());
        }
        for s in &self.steps {
            hash.update(format!("{}\0{}\0{}\0{}\0", s.output, s.stratum, s.reads.join(","), s.sql).as_bytes());
        }
        hash.finalize().into()
    }
}

pub(in crate::code_rule_dsl) struct Limits {
    /// Bytes of core facts and their projection one chunk may hold. Two chunks are resident at
    /// once, so this is half the scheduler's pool.
    pub chunk_bytes: usize,
    pub max_waves: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { chunk_bytes: 125 * 1024 * 1024, max_waves: 1024 }
    }
}

#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct Outcome {
    /// Waves over all chunks.
    pub waves: u32,
    pub chunks: usize,
    /// `rows.<relation>` at the last statistics boundary.
    pub rows: BTreeMap<String, u64>,
    /// The families whose digest changed.
    pub changed: Vec<String>,
}

/// The family of relation `rel` of program `program` over a batch of `scope`.
pub(in crate::code_rule_dsl) fn family_name(scope: &str, program: &str, rel: &str) -> String {
    format!("{scope}#wave:{program}:{rel}")
}

/// A document's resident cost in a chunk: its core facts, the batch's projection of them and its
/// source.
fn bytes(d: &Document) -> usize {
    2 * d.facts.bytes() + d.facts.source.len()
}

/// Documents in chunks of at most `budget` bytes, in order; a document over the budget is a chunk
/// of its own. A program that is not file-local is one chunk.
fn chunks(documents: Vec<Document>, budget: usize, file_local: bool) -> Vec<Vec<Document>> {
    if !file_local {
        return vec![documents];
    }
    let mut out: Vec<Vec<Document>> = Vec::new();
    let mut used = 0;
    for d in documents {
        let b = bytes(&d);
        match out.last_mut() {
            Some(chunk) if used + b <= budget => chunk.push(d),
            _ => {
                out.push(vec![d]);
                used = 0;
            }
        }
        used += b;
    }
    out
}

const CONTROL: [(&str, &str); 5] = [
    ("wave_step", "CREATE TEMP TABLE wave_step(step INTEGER PRIMARY KEY, stratum INTEGER NOT NULL, output TEXT NOT NULL, sql TEXT NOT NULL)"),
    ("wave_read", "CREATE TEMP TABLE wave_read(rel TEXT NOT NULL, step INTEGER NOT NULL, PRIMARY KEY (rel, step)) WITHOUT ROWID"),
    ("wave_delta", "CREATE TEMP TABLE wave_delta(wave INTEGER NOT NULL, rel TEXT NOT NULL, rows INTEGER NOT NULL, PRIMARY KEY (wave, rel)) WITHOUT ROWID"),
    ("wave_pending", "CREATE TEMP TABLE wave_pending(step INTEGER PRIMARY KEY)"),
    ("wave_document", "CREATE TEMP TABLE wave_document(position INTEGER PRIMARY KEY, d INTEGER NOT NULL)"),
];

fn sql(e: rusqlite::Error) -> StoreError {
    StoreError::Database(e)
}

/// Declares the program's relations, their `new_` and `delta_` tables and the loop's control
/// tables, empty, and loads the steps and their reads.
fn declare(lease: &Lease<'_>, program: &Program) -> rusqlite::Result<()> {
    let mut storage = lease.storage.borrow_mut();
    for (name, definition) in CONTROL {
        storage.declare(lease, name, definition.to_owned(), true)?;
        storage.written(name, true);
    }
    for (rel, arity) in &program.relations {
        let cols = (0..*arity).map(|i| format!("c{i}")).collect::<Vec<_>>();
        let typed = cols.iter().map(|c| format!("{c} INTEGER")).collect::<Vec<_>>().join(", ");
        for table in [rel.clone(), format!("new_{rel}"), format!("delta_{rel}")] {
            let definition = format!("CREATE TEMP TABLE {table}({typed}, PRIMARY KEY ({})) WITHOUT ROWID", cols.join(", "));
            storage.declare(lease, &table, definition, true)?;
        }
    }
    let mut step = lease.prepare_cached("INSERT INTO temp.wave_step(step, stratum, output, sql) VALUES (?1, ?2, ?3, ?4)")?;
    let mut read = lease.prepare_cached("INSERT OR IGNORE INTO temp.wave_read(rel, step) VALUES (?1, ?2)")?;
    for (i, s) in program.steps.iter().enumerate() {
        step.execute(rusqlite::params![i as i64, s.stratum, s.output, s.sql])?;
        for r in &s.reads {
            read.execute(rusqlite::params![r, i as i64])?;
        }
    }
    Ok(())
}

/// Runs one chunk's waves to the fixpoint: every step is pending at first, and after each wave
/// the readers of every relation that changed. Returns the number of waves.
fn waves(lease: &Lease<'_>, program: &Program, token: Option<i64>, max_waves: u32) -> StoreResult<u32> {
    let derived: BTreeSet<&str> = program.relations.iter().map(|(r, _)| r.as_str()).collect();
    lease.execute_batch("DELETE FROM temp.wave_delta; DELETE FROM temp.wave_pending; INSERT INTO temp.wave_pending SELECT step FROM temp.wave_step")?;
    let mut wave = 0;
    loop {
        let stratum: Option<i64> = lease.query_row(
            "SELECT min(s.stratum) FROM temp.wave_pending p JOIN temp.wave_step s USING (step)",
            [],
            |r| r.get(0),
        )?;
        let Some(stratum) = stratum else { break };
        if wave == max_waves {
            return Err(StoreError::Invalid(format!("{}: no fixpoint in {max_waves} waves", program.name)));
        }
        wave += 1;
        let frontier: Vec<(i64, String, String)> = lease
            .prepare_cached(
                "SELECT s.step, s.output, s.sql FROM temp.wave_pending p JOIN temp.wave_step s USING (step) \
                 WHERE s.stratum = ?1 ORDER BY s.step",
            )?
            .query_map([stratum], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        lease.execute(
            "DELETE FROM temp.wave_pending WHERE step IN (SELECT step FROM temp.wave_step WHERE stratum = ?1)",
            [stratum],
        )?;
        let mut written = BTreeSet::new();
        for (_, output, text) in &frontier {
            let mut statement = lease.prepare_execution(text)?;
            if let Some(i) = statement.parameter_index(":batch")? {
                let token = token.ok_or_else(|| StoreError::Invalid(format!("{}: a step reads demand(batch) without a batch", program.name)))?;
                statement.raw_bind_parameter(i, token)?;
            }
            statement.raw_execute()?;
            lease.storage.borrow_mut().written(&format!("new_{output}"), true);
            written.insert(output.as_str());
        }
        // Fold: the rows not seen before are the relation's delta for the next wave.
        let mut storage = lease.storage.borrow_mut();
        let mut record = lease.prepare_cached("INSERT INTO temp.wave_delta(wave, rel, rows) VALUES (?1, ?2, ?3)")?;
        for rel in &derived {
            let had = storage.rows(&format!("delta_{rel}")) != 0;
            if !written.contains(rel) && !had {
                continue;
            }
            lease.execute_batch(&format!("DELETE FROM delta_{rel}"))?;
            let n = lease.execute(&format!("INSERT INTO delta_{rel} SELECT * FROM new_{rel} EXCEPT SELECT * FROM {rel}"), [])?;
            lease.execute_batch(&format!("INSERT INTO {rel} SELECT * FROM delta_{rel}; DELETE FROM new_{rel}"))?;
            storage.populated(&format!("delta_{rel}"), n);
            storage.grew(rel, n);
            record.execute(rusqlite::params![wave, rel, n as i64])?;
        }
        drop(storage);
        lease.execute(
            "INSERT OR IGNORE INTO temp.wave_pending SELECT r.step FROM temp.wave_delta d JOIN temp.wave_read r USING (rel) \
             WHERE d.wave = ?1 AND d.rows > 0",
            [wave],
        )?;
        lease.storage.borrow_mut().statistics_boundary(lease)?;
    }
    Ok(wave)
}

/// The digest of a relation's rows in key order, and their count: all of them, or those whose
/// first column is `part`.
fn rows_digest(lease: &Lease<'_>, rel: &str, arity: usize, part: Option<i64>) -> rusqlite::Result<([u8; 32], i64)> {
    let order = (0..arity).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
    let mut hash = Sha256::new();
    hash.update((arity as u64).to_le_bytes());
    let filter = if part.is_some() { "WHERE c0 = ?1" } else { "" };
    let mut statement = lease.prepare(&format!("SELECT * FROM {rel} {filter} ORDER BY {order}"))?;
    let mut rows = match part {
        Some(part) => statement.query([part])?,
        None => statement.query([])?,
    };
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        for i in 0..arity {
            hash.update(row.get::<_, i64>(i)?.to_le_bytes());
        }
    }
    Ok((hash.finalize().into(), count))
}

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
    declare(lease, program)?;
    let mut inputs: Vec<DerivationInput> =
        documents.iter().map(|d| DerivationInput { name: Some(d.core.clone()), digest: d.digest }).collect();
    inputs.sort_by(|a, b| a.name.cmp(&b.name));
    let planned = chunks(documents, limits.chunk_bytes, program.file_local);
    let mut outcome = Outcome { chunks: planned.len(), ..Outcome::default() };
    let store = graph.store();
    std::thread::scope(|threads| -> StoreResult<()> {
        // A rendezvous: chunk k+1 is built while chunk k runs, and waits there until it is taken.
        let (send, receive) = std::sync::mpsc::sync_channel::<Built>(0);
        threads.spawn(move || {
            for chunk in planned {
                let built = sources(store, chunk.clone()).map(|s| (chunk, s));
                if send.send(built).is_err() {
                    break;
                }
            }
        });
        for built in receive {
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
            let token = demand(&docs)?.map(|d| lease.registry.register(d));
            let waves = waves(lease, program, token, limits.max_waves).and_then(|waves| match token {
                // A batch the steps read that did not complete fails its chunk: its relations
                // would be silently short.
                Some(token) if program.steps.iter().any(|step| step.sql.contains(":batch")) => {
                    let failed: Option<String> = lease
                        .query_row(
                            "SELECT coalesce(error_code, 'incomplete') FROM rerail_batch_status WHERE batch = ?1 AND complete = 0",
                            [token],
                            |row| row.get(0),
                        )
                        .optional()?;
                    match failed {
                        Some(error) => Err(StoreError::Invalid(format!("{}: demand(batch) failed: {error}", program.name))),
                        None => Ok(waves),
                    }
                }
                _ => Ok(waves),
            });
            if let Some(token) = token {
                lease.registry.remove_batch(token);
            }
            outcome.waves += waves?;
        }
        Ok(())
    })?;
    outcome.rows = lease.storage.borrow_mut().statistics_boundary(lease)?;
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

#[cfg(test)]
mod tests;
