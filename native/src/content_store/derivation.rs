//! The derivation graph (docs/final-shape/03-architecture.md §14): every fact family is the output
//! of a derivation, and the dependency engine and the fact engine are one mechanism.
//!
//! - A name (a workspace path, a Git ref, a buffer, a derivation's own output) is bound to a
//!   32-byte digest (`name_bindings`). Only observers rebind names; rebinding a name to the digest
//!   it already has changes nothing.
//! - A derivation records the names it read with the digest each had then (`derivation_inputs`),
//!   absent names included (bound to `ABSENT`), so it is valid while every one still binds to that
//!   digest: one join. An input without a name is an immutable digest (a content hash).
//! - A derivation's output is itself a name, bound to the output's digest when it is recorded. A
//!   recomputed derivation whose digest is unchanged therefore rebinds nothing, and nothing that
//!   read it is enqueued (early cutoff, `facts::propagate`).
//! - The readers of a name are found through `derivation_inputs_by_name`: the next wave's frontier.

use super::rows::{hash_params, intern_string};
use super::{ContentStore, HashKey, STRING_FACT_NAME, StoreError, StoreResult};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::BTreeSet;

/// The digest an unbound name reads as: a derivation that probed a name nobody bound records it,
/// and is invalidated when the name is first bound.
pub const ABSENT: [u8; 32] = [0; 32];

/// A name and the digest an observer saw it bound to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    pub name: String,
    pub digest: [u8; 32],
}

/// One input of a derivation: a name and the digest it read, or an immutable digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivationInput {
    pub name: Option<String>,
    pub digest: [u8; 32],
}

/// A derivation as run: its output name, its producer (plan identity, revision included), what it
/// read, the digest of what it produced, its status and its cost.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivationRecord {
    pub output: String,
    pub producer: [u8; 32],
    pub inputs: Vec<DerivationInput>,
    pub digest: [u8; 32],
    pub status: i64,
    pub cost: i64,
}

/// A stored derivation, as the frontier names it.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Derivation {
    pub output: String,
    pub producer: [u8; 32],
    pub status: i64,
    pub cost: i64,
}

impl ContentStore {
    /// Binds each name to its digest, in one transaction; returns the names whose digest changed
    /// (newly bound, or bound to another digest), in input order.
    pub fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>> {
        self.immediate_transaction(|transaction| {
            let mut changed = Vec::new();
            for binding in bindings {
                let name = intern_string(transaction, STRING_FACT_NAME, &binding.name)?;
                if bind(transaction, name, &binding.digest)? {
                    changed.push(binding.name.clone());
                }
            }
            Ok(changed)
        })
    }

    /// Records each derivation (replacing its inputs) and binds its output name to its digest, in
    /// one transaction; returns the outputs whose digest changed, in input order.
    pub fn record_derivations(&self, records: &[DerivationRecord]) -> StoreResult<Vec<String>> {
        self.immediate_transaction(|transaction| {
            let mut changed = Vec::new();
            for record in records {
                let output = intern_string(transaction, STRING_FACT_NAME, &record.output)?;
                transaction
                    .prepare_cached(
                        "INSERT INTO derivations(output_string_id,producer,status,cost) VALUES (?1,?2,?3,?4) \
                         ON CONFLICT(output_string_id) DO UPDATE \
                         SET producer=excluded.producer,status=excluded.status,cost=excluded.cost",
                    )?
                    .execute(params![output, &record.producer[..], record.status, record.cost])?;
                transaction
                    .prepare_cached("DELETE FROM derivation_inputs WHERE output_string_id=?1")?
                    .execute([output])?;
                let mut insert = transaction.prepare_cached(
                    "INSERT INTO derivation_inputs(output_string_id,ordinal,name_string_id,digest) \
                     VALUES (?1,?2,?3,?4)",
                )?;
                for (ordinal, input) in record.inputs.iter().enumerate() {
                    let name = match &input.name {
                        Some(name) => Some(intern_string(transaction, STRING_FACT_NAME, name)?),
                        None => None,
                    };
                    insert.execute(params![output, ordinal as i64, name, &input.digest[..]])?;
                }
                if bind(transaction, output, &record.digest)? {
                    changed.push(record.output.clone());
                }
            }
            Ok(changed)
        })
    }

    /// The digest `name` is bound to, if any.
    pub fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let Some(id) = name_id(&connection, name)? else {
            return Ok(None);
        };
        connection
            .prepare_cached("SELECT digest FROM name_bindings WHERE name_string_id=?1")?
            .query_row([id], |row| row.get::<_, Vec<u8>>(0))
            .optional()?
            .map(|bytes| digest(&bytes))
            .transpose()
    }

    /// Every derivation that read one of `names`: the frontier those names' changes dirty, each
    /// once, ordered by output name.
    pub fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut readers = connection.prepare_cached(
            "SELECT s.value,d.producer,d.status,d.cost FROM derivation_inputs i INDEXED BY derivation_inputs_by_name \
             JOIN derivations d ON d.output_string_id=i.output_string_id \
             JOIN strings s ON s.string_id=d.output_string_id WHERE i.name_string_id=?1",
        )?;
        let mut frontier = BTreeSet::new();
        for name in names {
            let Some(id) = name_id(&connection, name)? else {
                continue;
            };
            let rows = readers.query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get(2)?, row.get(3)?))
            })?;
            for row in rows {
                let (output, producer, status, cost) = row?;
                frontier.insert(Derivation { output, producer: digest(&producer)?, status, cost });
            }
        }
        Ok(frontier.into_iter().collect())
    }

    /// Whether `output`'s derivation is stored and every name it read still binds to the digest
    /// it read (an unbound name to `ABSENT`).
    pub fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let Some(id) = name_id(&connection, output)? else {
            return Ok(false);
        };
        connection
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM derivations WHERE output_string_id=?1) \
                 AND NOT EXISTS (SELECT 1 FROM derivation_inputs i \
                 LEFT JOIN name_bindings b ON b.name_string_id=i.name_string_id \
                 WHERE i.output_string_id=?1 AND i.name_string_id IS NOT NULL \
                 AND coalesce(b.digest,?2)<>i.digest)",
            )?
            .query_row(params![id, &ABSENT[..]], |row| row.get(0))
            .map_err(StoreError::from)
    }
}

/// Binds the interned `name` to `digest`; whether its digest changed.
fn bind(transaction: &Transaction<'_>, name: i64, digest: &[u8; 32]) -> StoreResult<bool> {
    let held = transaction
        .prepare_cached("SELECT digest FROM name_bindings WHERE name_string_id=?1")?
        .query_row([name], |row| row.get::<_, Vec<u8>>(0))
        .optional()?;
    if held.as_deref() == Some(&digest[..]) {
        return Ok(false);
    }
    transaction
        .prepare_cached(
            "INSERT INTO name_bindings(name_string_id,digest,revision) VALUES (?1,?2,0) \
             ON CONFLICT(name_string_id) DO UPDATE SET digest=excluded.digest,revision=revision+1",
        )?
        .execute(params![name, &digest[..]])?;
    // A first binding of a name read while absent is a change only if it differs from ABSENT.
    Ok(held.is_some() || digest != &ABSENT)
}

/// The string id of fact name `name`, without interning it.
fn name_id(connection: &Connection, name: &str) -> StoreResult<Option<i64>> {
    let [algorithm, lane0, lane1, lane2, lane3] = hash_params(HashKey::sha256(name.as_bytes()));
    connection
        .prepare_cached(
            "SELECT s.string_id FROM hashes h INDEXED BY hashes_by_prefix \
             JOIN strings s ON s.content_hash_id=h.hash_id \
             WHERE h.algorithm_id=?1 AND h.lane0=?2 AND h.lane1=?3 AND h.lane2=?4 AND h.lane3=?5 \
             AND s.domain_id=?6 AND s.value=?7",
        )?
        .query_row(
            params![algorithm, lane0, lane1, lane2, lane3, STRING_FACT_NAME, name],
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)
}

fn digest(bytes: &[u8]) -> StoreResult<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt("derivation digest is not 32 bytes".to_owned()))
}

#[cfg(test)]
mod tests;
