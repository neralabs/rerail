//! The derivation graph (docs/final-shape/03-architecture.md §14): every fact family is the output
//! of a derivation, and the dependency engine and the fact engine are one mechanism.
//!
//! - A name (a workspace path, a Git ref, a buffer, a derivation's own output) is an atom
//!   (atoms.rs), bound to a 32-byte digest (`name_bindings`). Only observers rebind names; rebinding a name to the digest
//!   it already has changes nothing.
//! - A derivation records the names it read with the digest each had then (`derivation_inputs`),
//!   absent names included (bound to `ABSENT`), so it is valid while every one still binds to that
//!   digest: one join. An input without a name is an immutable digest (a content hash).
//! - A derivation's output is itself a name, bound to the output's digest when it is recorded. A
//!   recomputed derivation whose digest is unchanged therefore rebinds nothing, and nothing that
//!   read it is enqueued (early cutoff, `facts::propagate`).
//! - The readers of a name are found through `derivation_inputs_by_name`: the next wave's frontier.

use super::{ContentStore, StoreError, StoreResult, Sym};
use rusqlite::{OptionalExtension, Transaction, params};
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
        let names = self.intern_atoms(&bindings.iter().map(|b| b.name.as_str()).collect::<Vec<_>>())?;
        self.immediate_transaction(|transaction| {
            let mut changed = Vec::new();
            for (binding, &name) in bindings.iter().zip(&names) {
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
        let mut values = Vec::new();
        for record in records {
            values.push(record.output.as_str());
            values.extend(record.inputs.iter().filter_map(|input| input.name.as_deref()));
        }
        let atoms = self.intern_atoms(&values)?;
        self.immediate_transaction(|transaction| {
            let mut changed = Vec::new();
            let mut atoms = atoms.iter().copied();
            for record in records {
                let output = atoms.next().expect("one atom per output");
                let inputs = record
                    .inputs
                    .iter()
                    .map(|input| (input.name.as_ref().map(|_| atoms.next().expect("one atom per named input")), input.digest))
                    .collect();
                let derivation =
                    StoredDerivation { producer: record.producer, status: record.status, cost: record.cost, inputs };
                write_derivation(transaction, output, &derivation)?;
                if bind(transaction, output, &record.digest)? {
                    changed.push(record.output.clone());
                }
            }
            Ok(changed)
        })
    }

    /// The digest `name` is bound to, if any.
    pub fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        let Some(id) = self.atom(name)? else {
            return Ok(None);
        };
        let connection = self.connection.lock().expect("content store mutex poisoned");
        connection
            .prepare_cached("SELECT digest FROM name_bindings WHERE name_atom=?1")?
            .query_row([id], |row| row.get::<_, Vec<u8>>(0))
            .optional()?
            .map(|bytes| digest(&bytes))
            .transpose()
    }

    /// Every derivation that read one of `names`: the frontier those names' changes dirty, each
    /// once, ordered by output name.
    pub fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>> {
        let mut ids = Vec::with_capacity(names.len());
        for name in names {
            ids.extend(self.atom(name)?);
        }
        let mut rows = Vec::new();
        {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut readers = connection.prepare_cached(
                "SELECT a.value,d.producer,d.status,d.cost FROM derivation_inputs i INDEXED BY derivation_inputs_by_name \
                 JOIN derivations d ON d.output_atom=i.output_atom \
                 JOIN atoms a ON a.atom=d.output_atom WHERE i.name_atom=?1",
            )?;
            for id in ids {
                let found = readers.query_map([id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get(2)?, row.get(3)?))
                })?;
                for row in found {
                    rows.push(row?);
                }
            }
        }
        let mut frontier = BTreeSet::new();
        for (output, producer, status, cost) in rows {
            frontier.insert(Derivation { output, producer: digest(&producer)?, status, cost });
        }
        Ok(frontier.into_iter().collect())
    }

    /// Whether `output`'s derivation is stored and every name it read still binds to the digest
    /// it read (an unbound name to `ABSENT`).
    pub fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        let Some(id) = self.atom(output)? else {
            return Ok(false);
        };
        let connection = self.connection.lock().expect("content store mutex poisoned");
        connection
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM derivations WHERE output_atom=?1) \
                 AND NOT EXISTS (SELECT 1 FROM derivation_inputs i \
                 LEFT JOIN name_bindings b ON b.name_atom=i.name_atom \
                 WHERE i.output_atom=?1 AND i.name_atom IS NOT NULL \
                 AND coalesce(b.digest,?2)<>i.digest)",
            )?
            .query_row(params![id, &ABSENT[..]], |row| row.get(0))
            .map_err(StoreError::from)
    }

    /// The digest the atom `name` is bound to, if any.
    pub fn binding_of(&self, name: Sym) -> StoreResult<Option<[u8; 32]>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        connection
            .prepare_cached("SELECT digest FROM name_bindings WHERE name_atom=?1")?
            .query_row([name], |row| row.get::<_, Vec<u8>>(0))
            .optional()?
            .map(|bytes| digest(&bytes))
            .transpose()
    }

    /// The stored derivation whose output is the atom `output`, with its inputs in order.
    pub fn derivation_of(&self, output: Sym) -> StoreResult<Option<StoredDerivation>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let Some((producer, status, cost)) = connection
            .prepare_cached("SELECT producer,status,cost FROM derivations WHERE output_atom=?1")?
            .query_row([output], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get(1)?, row.get(2)?)))
            .optional()?
        else {
            return Ok(None);
        };
        let inputs = connection
            .prepare_cached("SELECT name_atom,digest FROM derivation_inputs WHERE output_atom=?1 ORDER BY ordinal")?
            .query_map([output], |row| Ok((row.get::<_, Option<Sym>>(0)?, row.get::<_, Vec<u8>>(1)?)))?
            .map(|row| {
                let (name, bytes) = row?;
                Ok((name, digest(&bytes)?))
            })
            .collect::<StoreResult<_>>()?;
        Ok(Some(StoredDerivation { producer: digest(&producer)?, status, cost, inputs }))
    }

    /// The outputs of the stored derivations that read the atom `name`, ascending.
    pub fn readers_of(&self, name: Sym) -> StoreResult<Vec<Sym>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut readers = connection
            .prepare_cached(
                "SELECT DISTINCT output_atom FROM derivation_inputs INDEXED BY derivation_inputs_by_name \
                 WHERE name_atom=?1 ORDER BY output_atom",
            )?
            .query_map([name], |row| row.get(0))?
            .collect::<Result<Vec<Sym>, _>>()?;
        readers.dedup();
        Ok(readers)
    }

    /// Writes one overlay layer in one transaction: each binding (a rebinding bumps its revision)
    /// and each derivation with its inputs replaced. The layer's own rows are already final, so
    /// nothing here compares or reports changes.
    pub fn write_derivation_layer(
        &self,
        bindings: &[(Sym, [u8; 32])],
        derivations: &[(Sym, StoredDerivation)],
    ) -> StoreResult<()> {
        self.immediate_transaction(|transaction| {
            for (name, digest) in bindings {
                bind(transaction, *name, digest)?;
            }
            for (output, derivation) in derivations {
                write_derivation(transaction, *output, derivation)?;
            }
            Ok(())
        })
    }
}

/// A derivation by atoms, as the overlay and the writer hold it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDerivation {
    pub producer: [u8; 32],
    pub status: i64,
    pub cost: i64,
    pub inputs: Vec<(Option<Sym>, [u8; 32])>,
}

fn write_derivation(transaction: &Transaction<'_>, output: Sym, derivation: &StoredDerivation) -> StoreResult<()> {
    transaction
        .prepare_cached(
            "INSERT INTO derivations(output_atom,producer,status,cost) VALUES (?1,?2,?3,?4) \
             ON CONFLICT(output_atom) DO UPDATE \
             SET producer=excluded.producer,status=excluded.status,cost=excluded.cost",
        )?
        .execute(params![output, &derivation.producer[..], derivation.status, derivation.cost])?;
    transaction
        .prepare_cached("DELETE FROM derivation_inputs WHERE output_atom=?1")?
        .execute([output])?;
    let mut insert = transaction.prepare_cached(
        "INSERT INTO derivation_inputs(output_atom,ordinal,name_atom,digest) VALUES (?1,?2,?3,?4)",
    )?;
    for (ordinal, (name, digest)) in derivation.inputs.iter().enumerate() {
        insert.execute(params![output, ordinal as i64, name, &digest[..]])?;
    }
    Ok(())
}

/// Binds the atom `name` to `digest`; whether its digest changed.
fn bind(transaction: &Transaction<'_>, name: Sym, digest: &[u8; 32]) -> StoreResult<bool> {
    let held = transaction
        .prepare_cached("SELECT digest FROM name_bindings WHERE name_atom=?1")?
        .query_row([name], |row| row.get::<_, Vec<u8>>(0))
        .optional()?;
    if held.as_deref() == Some(&digest[..]) {
        return Ok(false);
    }
    transaction
        .prepare_cached(
            "INSERT INTO name_bindings(name_atom,digest,revision) VALUES (?1,?2,0) \
             ON CONFLICT(name_atom) DO UPDATE SET digest=excluded.digest,revision=revision+1",
        )?
        .execute(params![name, &digest[..]])?;
    // A first binding of a name read while absent is a change only if it differs from ABSENT.
    Ok(held.is_some() || digest != &ABSENT)
}

fn digest(bytes: &[u8]) -> StoreResult<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt("derivation digest is not 32 bytes".to_owned()))
}

#[cfg(test)]
mod tests;
