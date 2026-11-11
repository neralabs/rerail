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
//! wave's frontier: the pending steps of every stratum with no pending stratum upstream of it
//! (`temp.wave_after`, the strata a stratum reads from, transitively). Independent strata share
//! waves, so a program takes about its dependency depth in waves, not one per stratum. Rust executes the statements
//! SQL selected and folds each `new_` table with fixed SQL; it never runs per row (ledger 17).
//!
//! Step contract:
//! - A step's statement inserts into `new_<output>`.
//! - It reads `delta_<r>` for a relation `r` of its own stratum (semi-naive) and whole
//!   relations otherwise.
//! - `reads` lists every derived relation the statement reads.
//! - A stratum runs only when no stratum it reads from (transitively) has pending steps, so
//!   negation and aggregation read complete relations.
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

use super::owner::{Database, Lease};
use super::owner::sql::{SqlOwner, Borrowed};
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::facts::vtab::syntax::{Document, fresh, sources};
use crate::content_store::{DerivationInput, DerivationRecord, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
mod lattice;
pub(in crate::code_rule_dsl) mod sets;
pub(in crate::code_rule_dsl) use lattice::Operations as LatticeOperations;

/// One statement of a wave program.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Step {
    pub output: String,
    pub stratum: u32,
    /// The relations the statement reads: derived ones by name (whole or `delta_`), base ones by
    /// table.
    pub reads: Vec<String>,
    pub sql: String,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
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
    /// Flat-lattice relations (`logic::plan`'s `merge_lattice`): (relation, key columns, top). The
    /// last column is the value: a key's equal values stay, different ones (or top) become top. A
    /// lattice relation's rows are keyed by its keys, and its fold merges by key.
    pub lattices: Vec<(String, usize, i64)>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lattice_operations: BTreeMap<String, LatticeOperations>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub column_types: BTreeMap<String, Vec<String>>,
}

impl Program {
    /// The longest chain of strata its steps read through: the waves a chunk of a program without
    /// recursion takes (independent strata share a wave).
    pub(in crate::code_rule_dsl) fn strata(&self) -> u32 {
        let mut depth = BTreeMap::<&str, u32>::new();
        let mut steps = self.steps.iter().collect::<Vec<_>>();
        steps.sort_by_key(|step| step.stratum);
        for step in steps {
            let below = step.reads.iter().filter_map(|read| depth.get(read.trim_start_matches("delta_"))).max().copied().unwrap_or(0);
            let at = depth.entry(step.output.as_str()).or_insert(0);
            *at = (*at).max(below + 1);
        }
        depth.values().max().copied().unwrap_or(1)
    }

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
        for (rel, keys, top) in &self.lattices {
            hash.update(format!("lattice {rel}/{keys}/{top}\0").as_bytes());
        }
        if !self.column_types.is_empty() {
            hash.update(serde_json::to_vec(&self.column_types).expect("column types serialize"));
        }
        if !self.lattice_operations.is_empty() {
            hash.update(serde_json::to_vec(&self.lattice_operations).expect("lattice SQL operations serialize"));
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
    // No documents is one chunk all the same: a warm run's kept rows are a wave's (06 S6).
    if !file_local || documents.is_empty() {
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

const CONTROL: [(&str, &str); 6] = [
    ("wave_step", "CREATE TEMP TABLE wave_step(step INTEGER PRIMARY KEY, stratum INTEGER NOT NULL, output TEXT NOT NULL, sql TEXT NOT NULL)"),
    ("wave_read", "CREATE TEMP TABLE wave_read(rel TEXT NOT NULL, step INTEGER NOT NULL, PRIMARY KEY (rel, step)) WITHOUT ROWID"),
    ("wave_delta", "CREATE TEMP TABLE wave_delta(wave INTEGER NOT NULL, rel TEXT NOT NULL, rows INTEGER NOT NULL, PRIMARY KEY (wave, rel)) WITHOUT ROWID"),
    ("wave_pending", "CREATE TEMP TABLE wave_pending(step INTEGER PRIMARY KEY)"),
    ("wave_after", "CREATE TEMP TABLE wave_after(stratum INTEGER NOT NULL, before INTEGER NOT NULL, PRIMARY KEY (stratum, before)) WITHOUT ROWID"),
    ("wave_document", "CREATE TEMP TABLE wave_document(position INTEGER PRIMARY KEY, d INTEGER NOT NULL)"),
];

fn sql(e: rusqlite::Error) -> StoreError {
    StoreError::Database(e)
}

/// Declares the program's relations, their `new_` and `delta_` tables and the loop's control
/// tables, empty, and loads the steps and their reads.
fn declare(lease: &impl SqlOwner, program: &Program, existing:bool) -> rusqlite::Result<()> {
    let mut storage = lease.storage().borrow_mut();
    for (name, definition) in CONTROL {
        let definition=if existing {definition.replacen("CREATE TEMP TABLE ","CREATE TEMP TABLE IF NOT EXISTS ",1)} else {definition.to_owned()};
        storage.declare(lease, name, definition, true)?;
        storage.written(name, true);
    }
    drop(storage);
    declare_relations(lease, program, existing, false)?;
    let mut step = lease.prepare_cached("INSERT INTO temp.wave_step(step, stratum, output, sql) VALUES (?1, ?2, ?3, ?4)")?;
    let mut read = lease.prepare_cached("INSERT OR IGNORE INTO temp.wave_read(rel, step) VALUES (?1, ?2)")?;
    for (i, s) in program.steps.iter().enumerate() {
        step.execute(rusqlite::params![i as i64, s.stratum, s.output, s.sql])?;
        for r in &s.reads {
            read.execute(rusqlite::params![r, i as i64])?;
        }
    }
    // A stratum waits on the strata whose outputs its steps read, and on theirs.
    lease.execute_batch(
        "INSERT INTO temp.wave_after WITH RECURSIVE \
           e(stratum, before) AS (SELECT DISTINCT s.stratum, p.stratum FROM temp.wave_read r \
             JOIN temp.wave_step s USING (step) JOIN temp.wave_step p ON p.output = r.rel WHERE p.stratum <> s.stratum), \
           c(stratum, before) AS (SELECT * FROM e UNION SELECT c.stratum, e.before FROM c JOIN e ON e.stratum = c.before) \
         SELECT * FROM c WHERE stratum <> before",
    )?;
    Ok(())
}

fn declare_relations(lease: &impl SqlOwner, program: &Program, existing: bool, public_only: bool) -> rusqlite::Result<()> {
    let mut storage = lease.storage().borrow_mut();
    for (rel, arity) in &program.relations {
        let cols = (0..*arity).map(|i| format!("c{i}")).collect::<Vec<_>>();
        let typed = cols.iter().enumerate().map(|(i,c)| format!("{c} {}", program.column_types.get(rel).and_then(|t|t.get(i)).map_or("INTEGER",String::as_str))).collect::<Vec<_>>().join(", ");
        // A lattice relation and its delta hold one value per key; its `new_` rows, every candidate.
        let keys = lattice(program, rel).map_or(*arity, |(keys, _)| keys);
        for (table, keys) in [(rel.clone(), keys), (format!("new_{rel}"), *arity), (format!("delta_{rel}"), keys)] {
            if public_only && table != *rel { continue; }
            let definition = format!("CREATE TEMP TABLE {}{table}({typed}, PRIMARY KEY ({})) WITHOUT ROWID",if existing {"IF NOT EXISTS "}else{""}, cols[..keys].join(", "));
            storage.declare(lease, &table, definition, true)?;
        }
        if !public_only && program.lattice_operations.get(rel).is_some_and(|ops| ops.after.is_some()) {
            let typed = cols[..keys].iter().enumerate().map(|(i,c)| format!("{c} {}",program.column_types.get(rel).and_then(|t|t.get(i)).map_or("INTEGER",String::as_str))).collect::<Vec<_>>().join(", ");
            let table = format!("__sql_changes_{rel}");
            storage.declare(lease, &table, format!("CREATE TEMP TABLE {}{table}({typed}, changes INTEGER NOT NULL, PRIMARY KEY ({})) WITHOUT ROWID",if existing {"IF NOT EXISTS "}else{""},cols[..keys].join(", ")),true)?;
        }
    }
    Ok(())
}

/// Retained waves need public outputs and wave diagnostics; cold control tables are deferred.
pub(in crate::code_rule_dsl) fn declare_outputs(db: &Database, program: &Program) -> StoreResult<()> {
    {
        let mut storage = db.storage().borrow_mut();
        let (name, definition) = CONTROL[2];
        storage.declare(db, name, definition.to_owned(), true)?;
    }
    declare_relations(db, program, false, true)?;
    Ok(())
}

fn lattice(program: &Program, rel: &str) -> Option<(usize, i64)> {
    program.lattices.iter().find(|(name, ..)| name == rel).map(|(_, keys, top)| (*keys, *top))
}

/// A lattice relation's fold (`merge_lattice`): the candidates of `new_<rel>` joined per key, those
/// that change the relation's value as its delta (a value that differs from one already there is
/// top), merged into the relation. Returns the delta's rows.
fn fold_lattice(lease: &impl SqlOwner, rel: &str, arity: usize, keys: usize, top: i64) -> rusqlite::Result<usize> {
    let key = (0..keys).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
    let v = format!("c{}", arity - 1);
    let on = (0..keys).map(|i| format!("r.c{i} = j.c{i}")).collect::<Vec<_>>().join(" AND ");
    let same = (0..keys).map(|i| format!("r.c{i} = delta_{rel}.c{i}")).collect::<Vec<_>>().join(" AND ");
    let joined = format!(
        "SELECT {key}, CASE WHEN count(DISTINCT {v}) > 1 OR max({v}) = {top} THEN {top} ELSE min({v}) END AS {v} FROM new_{rel} GROUP BY {key}"
    );
    lease.execute_batch(&format!("DELETE FROM delta_{rel}"))?;
    let n = lease.execute(
        &format!("INSERT INTO delta_{rel} SELECT j.* FROM ({joined}) j LEFT JOIN {rel} r ON {on} WHERE r.{v} IS NULL OR (r.{v} <> {top} AND r.{v} <> j.{v})"),
        [],
    )?;
    lease.execute_batch(&format!(
        "UPDATE delta_{rel} SET {v} = {top} WHERE EXISTS (SELECT 1 FROM {rel} r WHERE {same} AND r.{v} <> delta_{rel}.{v});
         INSERT OR REPLACE INTO {rel} SELECT * FROM delta_{rel}; DELETE FROM new_{rel}"
    ))?;
    Ok(n)
}

/// Runs one chunk's waves to the fixpoint: every step is pending at first, and after each wave
/// the readers of every relation that changed. Returns the number of waves.
/// `empty`: the chunk has no documents, so a step reading its batch reads nothing and is skipped.
fn waves(lease: &impl SqlOwner, program: &Program, token: Option<i64>, max_waves: u32, empty: bool) -> StoreResult<u32> {
    waves_with(lease, program, token, max_waves, empty, &mut |_| Ok(None))
}

/// The frontier has completed every upstream stratum before asking for a step.
/// An optional replacement may construct reusable state and supply equivalent SQL.
fn waves_with(lease: &impl SqlOwner, program: &Program, token: Option<i64>, max_waves: u32, empty: bool,
    before: &mut dyn FnMut(usize) -> StoreResult<Option<String>>) -> StoreResult<u32> {
    crate::code_rule_dsl::logic::profile::stratum(&format!("{}: cold execution",program.name),"SQL wave kernel",||"pending gates and SQL frontier folds".into());
    let derived: BTreeMap<&str, usize> = program.relations.iter().map(|(r, arity)| (r.as_str(), *arity)).collect();
    lease.execute_batch("DELETE FROM temp.wave_delta; DELETE FROM temp.wave_pending; INSERT INTO temp.wave_pending SELECT step FROM temp.wave_step")?;
    let mut wave = 0;
    loop {
        let pending: i64 = lease.query_row("SELECT count(*) FROM temp.wave_pending", [], |r| r.get(0))?;
        if pending == 0 {
            break;
        }
        if wave == max_waves {
            return Err(StoreError::Invalid(format!("{}: no fixpoint in {max_waves} waves", program.name)));
        }
        wave += 1;
        let frontier: Vec<(i64, String, String)> = lease
            .prepare_cached(
                "SELECT s.step, s.output, s.sql FROM temp.wave_pending p JOIN temp.wave_step s USING (step) \
                 WHERE NOT EXISTS (SELECT 1 FROM temp.wave_after a JOIN temp.wave_step u ON u.stratum = a.before \
                   JOIN temp.wave_pending q ON q.step = u.step WHERE a.stratum = s.stratum) ORDER BY s.step",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut clear = lease.prepare_cached("DELETE FROM temp.wave_pending WHERE step = ?1")?;
        for (step, ..) in &frontier {
            clear.execute([step])?;
        }
        drop(clear);
        let mut written = BTreeSet::new();
        for (id, output, text) in &frontier {
            let _profile = lease.profile_scope(||serde_json::json!({"stage":"cold rule","executionPhase":"relationalEvaluation","wave":wave,"ruleStep":id,"relation":output,"stratum":program.steps[*id as usize].stratum}));
            let _cpu = crate::code_rule_dsl::logic::profile::span(||format!("cold rule {output}"));
            let replacement = before(*id as usize)?;
            let text = replacement.as_ref().unwrap_or(text);
            let mut statement = lease.prepare_step(text)?;
            if empty && token.is_none() && statement.parameter_index(":batch")?.is_some() {
                continue;
            }
            if let Some(i) = statement.parameter_index(":batch")? {
                let token = token.ok_or_else(|| StoreError::Invalid(format!("{}: a step reads demand(batch) without a batch", program.name)))?;
                statement.raw_bind_parameter(i, token)?;
            }
            // Each statement's time, by its output (`RERAIL_PROF`); `RERAIL_PROF_SLOW_MS=<ms>` prints the
            // SQL of each statement slower than that.
            let started = std::time::Instant::now();
            crate::code_rule_dsl::prof::time(&format!("step.{output}"), || statement.raw_execute())?;
            if let Some(slow) = std::env::var("RERAIL_PROF_SLOW_MS").ok().and_then(|ms| ms.parse::<u128>().ok()) {
                if started.elapsed().as_millis() >= slow {
                    eprintln!("slow step {output} ({:?}): {text}", started.elapsed());
                }
            }
            lease.storage().borrow_mut().written(&format!("new_{output}"), true);
            written.insert(output.as_str());
        }
        // Fold: the rows not seen before are the relation's delta for the next wave.
        let mut storage = lease.storage().borrow_mut();
        let mut record = lease.prepare_cached("INSERT INTO temp.wave_delta(wave, rel, rows) VALUES (?1, ?2, ?3)")?;
        for (rel, arity) in &derived {
            let _profile = lease.profile_scope(||serde_json::json!({"stage":"cold merge","executionPhase":"relationalEvaluation","wave":wave,"relation":rel}));
            let had = storage.rows(&format!("delta_{rel}")) != 0;
            if !written.contains(rel) && !had {
                continue;
            }
            let n = if let Some((keys, top)) = lattice(program, rel) {
                if let Some(ops) = program.lattice_operations.get(*rel) {
                    lattice::fold(lease, rel, *arity, keys, ops)?
                } else { fold_lattice(lease, rel, *arity, keys, top)? }
            } else {
                sets::fold(lease,rel,*arity,&format!("new_{rel}"),&format!("delta_{rel}"))?
            };
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
        lease.storage().borrow_mut().statistics_boundary(lease)?;
    }
    Ok(wave)
}

/// Executes a lowered fact/rule program on an existing owner connection. Inputs may be virtual
/// tables or SQL relations; outputs and wave diagnostics remain queryable on this connection.
/// AST matching stays outside this loop and supplies relations like any other producer.
pub(in crate::code_rule_dsl) fn execute_sql(db: &Database, program: &Program, max_waves: u32) -> StoreResult<u32> {
    execute_sql_with(db, program, max_waves, || Ok(()))
}
pub(in crate::code_rule_dsl) fn declare_sql(db:&Database,program:&Program)->StoreResult<()> {
    declare(db,program,false)?;Ok(())
}
pub(in crate::code_rule_dsl) fn execute_declared(db:&Database,program:&Program,max_waves:u32)->StoreResult<u32> {
    waves(db,program,None,max_waves,false)
}

pub(in crate::code_rule_dsl) fn execute_declared_with(db:&Database, program:&Program, max_waves:u32,
    mut before: impl FnMut(usize) -> StoreResult<Option<String>>) -> StoreResult<u32> {
    waves_with(db,program,None,max_waves,false,&mut before)
}

pub(in crate::code_rule_dsl) fn execute_sql_with(db: &Database, program: &Program, max_waves: u32, setup: impl FnOnce() -> rusqlite::Result<()>) -> StoreResult<u32> {
    declare(db, program,false)?;
    setup()?;
    waves(db, program, None, max_waves, false)
}

/// Run a component whose full tables already exist (corpus strata and maintained sessions).
/// Only the frontier/candidate tables are reset. Full rows and their indexes remain in place.
pub(in crate::code_rule_dsl) fn execute_existing(db:&rusqlite::Connection,program:&Program,max_waves:u32)->StoreResult<u32>{
    let present:BTreeSet<String>=db.prepare("SELECT name FROM temp.sqlite_schema WHERE type='table'")?.query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
    // Reuse control schemas; the wave loop clears pending/delta rows itself.
    for (name,_) in CONTROL {if present.contains(name)&&!matches!(name,"wave_delta"|"wave_pending"){db.execute_batch(&format!("DELETE FROM temp.{name}"))?;}}
    let owner=Borrowed::new(db);
    declare(&owner,program,true)?;
    for (rel,_) in &program.relations {for prefix in ["new_","delta_"]{let table=format!("{prefix}{rel}");if present.contains(&table){db.execute_batch(&format!("DELETE FROM {table}"))?;}}}
    waves(&owner,program,None,max_waves,false)
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

mod batches;
pub(in crate::code_rule_dsl) use batches::{layer, run_over};
#[cfg(test)]
pub(in crate::code_rule_dsl) use batches::run;


#[cfg(test)]
mod tests;
