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

use super::{ContentStore, StoreError, StoreResult, Sym, bounded_insert};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use std::collections::{BTreeSet, HashMap};

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
        // Its atoms are the store's (`syms_to_store`), so it is written after a fork too (decision 0170).
        super::atoms::mapped(|| self.immediate_transaction(|transaction| {
            let mut names = names.clone();
            self.syms_to_store(transaction, &mut names)?;
            let named: Vec<(Sym, [u8; 32])> = names.iter().copied().zip(bindings.iter().map(|b| b.digest)).collect();
            let changed = bind_all(transaction, &named)?;
            Ok(bindings.iter().zip(changed).filter(|(_, c)| *c).map(|(b, _)| b.name.clone()).collect())
        }))
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
        let mut atoms = atoms.iter().copied();
        let mut derivations = Vec::with_capacity(records.len());
        let mut bindings = Vec::with_capacity(records.len());
        for record in records {
            let output = atoms.next().expect("one atom per output");
            let inputs = record
                .inputs
                .iter()
                .map(|input| (input.name.as_ref().map(|_| atoms.next().expect("one atom per named input")), input.digest))
                .collect();
            derivations.push((output, StoredDerivation { producer: record.producer, status: record.status, cost: record.cost, inputs }));
            bindings.push((output, record.digest));
        }
        // Its atoms are the store's (`syms_to_store`), so it is written after a fork too (decision 0170).
        super::atoms::mapped(|| self.immediate_transaction(|transaction| {
            let (derivations, bindings) = self.derivations_to_store(transaction, &derivations, &bindings)?;
            write_derivations(transaction, &derivations)?;
            let changed = bind_all(transaction, &bindings)?;
            Ok(records.iter().zip(changed).filter(|(_, c)| *c).map(|(r, _)| r.output.clone()).collect())
        }))
    }

    /// The digest `name` is bound to, if any.
    pub fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        let Some(id) = self.atom(name)?.map(|id| self.stored_sym(id)).transpose()?.flatten() else {
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
        // The names' atoms in one lookup, as the store numbers them, and their readers in one statement.
        let ids: Vec<Sym> = self.atoms.lookup_all(&names.iter().map(String::as_str).collect::<Vec<_>>())?.into_iter().flatten().collect();
        let ids: Vec<Sym> = self.atoms.stored(&ids)?.into_iter().flatten().collect();
        let mut rows = Vec::new();
        if !ids.is_empty() {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            let mut readers = connection.prepare_cached(
                "SELECT DISTINCT d.output_atom,d.producer,d.status,d.cost FROM derivation_inputs i INDEXED BY derivation_inputs_by_name \
                 JOIN derivations d ON d.output_atom=i.output_atom WHERE i.name_atom IN (SELECT value FROM json_each(?1))",
            )?;
            let ids = serde_json::to_string(&ids).expect("ids encode");
            for row in readers.query_map([ids], |row| Ok((row.get::<_, Sym>(0)?, row.get::<_, Vec<u8>>(1)?, row.get(2)?, row.get(3)?)))? {
                rows.push(row?);
            }
        }
        let mut frontier = BTreeSet::new();
        for (output, producer, status, cost) in self.named_outputs(rows)? {
            frontier.insert(Derivation { output, producer: digest(&producer)?, status, cost });
        }
        Ok(frontier.into_iter().collect())
    }

    /// Whether `output`'s derivation is stored and every name it read still binds to the digest
    /// it read (an unbound name to `ABSENT`).
    pub fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        let Some(id) = self.atom(output)?.map(|id| self.stored_sym(id)).transpose()?.flatten() else {
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

    /// Marks a push begun, committed before the push writes anything (open-items O1).
    pub fn begin_push(&self) -> StoreResult<()> {
        self.immediate_transaction(|transaction| {
            transaction.execute("INSERT OR IGNORE INTO push_pending(pending) VALUES (1)", [])?;
            Ok(())
        })
    }

    /// Marks the push ended: called only once every write of the push is in SQLite.
    pub fn end_push(&self) -> StoreResult<()> {
        self.immediate_transaction(|transaction| {
            transaction.execute("DELETE FROM push_pending", [])?;
            Ok(())
        })
    }

    /// Whether every push begun on this store ended; false after a crash mid-push.
    pub fn push_clean(&self) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        connection
            .prepare_cached("SELECT NOT EXISTS (SELECT 1 FROM push_pending)")?
            .query_row([], |row| row.get(0))
            .map_err(StoreError::from)
    }

    /// Every stored derivation that is not valid (the validity join over all derivations), ordered
    /// by output: what an unclean push left behind. One scan of `derivation_inputs`.
    pub fn invalid_derivations(&self) -> StoreResult<Vec<Derivation>> {
        let rows = {
            let connection = self.connection.lock().expect("content store mutex poisoned");
            connection
                .prepare_cached(
                    "SELECT d.output_atom,d.producer,d.status,d.cost FROM derivations d \
                     WHERE EXISTS (SELECT 1 FROM derivation_inputs i \
                     LEFT JOIN name_bindings b ON b.name_atom=i.name_atom \
                     WHERE i.output_atom=d.output_atom AND i.name_atom IS NOT NULL \
                     AND coalesce(b.digest,?1)<>i.digest)",
                )?
                .query_map([&ABSENT[..]], |row| {
                    Ok((row.get::<_, Sym>(0)?, row.get::<_, Vec<u8>>(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut rows = self.named_outputs(rows)?;
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows.into_iter()
            .map(|(output, producer, status, cost)| Ok(Derivation { output, producer: digest(&producer)?, status, cost }))
            .collect()
    }

    /// The digest the atom `name` is bound to, if any.
    pub fn binding_of(&self, name: Sym) -> StoreResult<Option<[u8; 32]>> {
        let Some(name) = self.stored_sym(name)? else { return Ok(None) };
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
        let Some(output) = self.stored_sym(output)? else { return Ok(None) };
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
            .collect::<StoreResult<Vec<(Option<Sym>, [u8; 32])>>>()?;
        drop(connection);
        let mut inputs = inputs;
        if self.atoms.forked() {
            let mut names: Vec<i64> = inputs.iter().filter_map(|(n, _)| n.map(i64::from)).collect();
            self.atoms.to_local(&mut names)?;
            let mut names = names.into_iter();
            for (name, _) in &mut inputs {
                if name.is_some() {
                    *name = Some(names.next().expect("one per named input") as Sym);
                }
            }
        }
        Ok(Some(StoredDerivation { producer: digest(&producer)?, status, cost, inputs }))
    }

    /// The outputs of the stored derivations that read the atom `name`, ascending.
    pub fn readers_of(&self, name: Sym) -> StoreResult<Vec<Sym>> {
        let Some(name) = self.stored_sym(name)? else { return Ok(Vec::new()) };
        let connection = self.connection.lock().expect("content store mutex poisoned");
        let mut readers = connection
            .prepare_cached(
                "SELECT DISTINCT output_atom FROM derivation_inputs INDEXED BY derivation_inputs_by_name \
                 WHERE name_atom=?1 ORDER BY output_atom",
            )?
            .query_map([name], |row| row.get(0))?
            .collect::<Result<Vec<Sym>, _>>()?;
        drop(connection);
        if self.atoms.forked() {
            let mut local: Vec<i64> = readers.iter().map(|&r| i64::from(r)).collect();
            self.atoms.to_local(&mut local)?;
            readers = local.into_iter().map(|r| r as Sym).collect();
            readers.sort_unstable();
        }
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
        // Its atoms are the store's (`syms_to_store`), so it is written after a fork too (decision 0170).
        super::atoms::mapped(|| self.immediate_transaction(|transaction| {
            let (derivations, bindings) = self.derivations_to_store(transaction, derivations, bindings)?;
            bind_all(transaction, &bindings)?;
            write_derivations(transaction, &derivations)?;
            Ok(())
        }))
    }
}

/// The derivation tables name atoms by the store's ids. A process whose atoms forked from the store's (decision
/// 0170) maps its own to the store's when it writes them, and back when it reads them.
impl ContentStore {
    /// The store's id of `atom` for a read: `None` when the store does not hold it (then nothing is bound to it).
    fn stored_sym(&self, atom: Sym) -> StoreResult<Option<Sym>> {
        Ok(self.atoms.stored(&[atom])?[0])
    }

    /// `atoms` as the store's, in `transaction` (a write): a pending atom gets its store id first.
    fn syms_to_store(&self, transaction: &Connection, atoms: &mut [Sym]) -> StoreResult<()> {
        if !self.atoms.forked() {
            return Ok(());
        }
        let mut values: Vec<i64> = atoms.iter().map(|&a| i64::from(a)).collect();
        self.atoms.to_store(transaction, &mut values)?;
        for (atom, value) in atoms.iter_mut().zip(values) {
            *atom = value as Sym;
        }
        Ok(())
    }

    /// Derivations and bindings with the store's ids, in `transaction`.
    #[allow(clippy::type_complexity)]
    fn derivations_to_store(
        &self,
        transaction: &Connection,
        derivations: &[(Sym, StoredDerivation)],
        bindings: &[(Sym, [u8; 32])],
    ) -> StoreResult<(Vec<(Sym, StoredDerivation)>, Vec<(Sym, [u8; 32])>)> {
        let (mut derivations, mut bindings) = (derivations.to_vec(), bindings.to_vec());
        if !self.atoms.forked() {
            return Ok((derivations, bindings));
        }
        let mut atoms: Vec<Sym> = Vec::new();
        for (output, derivation) in &derivations {
            atoms.push(*output);
            atoms.extend(derivation.inputs.iter().filter_map(|(name, _)| *name));
        }
        atoms.extend(bindings.iter().map(|(name, _)| *name));
        self.syms_to_store(transaction, &mut atoms)?;
        let mut atoms = atoms.into_iter();
        for (output, derivation) in &mut derivations {
            *output = atoms.next().expect("an output");
            for (name, _) in &mut derivation.inputs {
                if name.is_some() {
                    *name = atoms.next();
                }
            }
        }
        for (name, _) in &mut bindings {
            *name = atoms.next().expect("a binding");
        }
        Ok((derivations, bindings))
    }

    /// Rows keyed by an output atom (the store's id), with the output's string.
    fn named_outputs<T>(&self, rows: Vec<(Sym, T, i64, i64)>) -> StoreResult<Vec<(String, T, i64, i64)>> {
        let mut ids: Vec<i64> = rows.iter().map(|r| i64::from(r.0)).collect();
        self.atoms.to_local(&mut ids)?;
        rows.into_iter()
            .zip(ids)
            .map(|((_, a, b, c), id)| {
                let name = self.atoms.value(id as Sym)?.ok_or_else(|| StoreError::Corrupt(format!("derivation output atom {id} is no atom")))?;
                Ok((name, a, b, c))
            })
            .collect()
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

/// Writes each derivation, its inputs replaced, as a few statements whatever their number (a derivation listed twice
/// is its last).
fn write_derivations(transaction: &Connection, derivations: &[(Sym, StoredDerivation)]) -> StoreResult<()> {
    if derivations.is_empty() {
        return Ok(());
    }
    let mut last: HashMap<Sym, &StoredDerivation> = HashMap::with_capacity(derivations.len());
    for (output, derivation) in derivations {
        last.insert(*output, derivation);
    }
    let mut outputs: Vec<Sym> = last.keys().copied().collect();
    outputs.sort_unstable();
    bounded_insert::insert(
        transaction,
        "INSERT INTO derivations(output_atom,producer,status,cost)",
        "ON CONFLICT(output_atom) DO UPDATE SET producer=excluded.producer,status=excluded.status,cost=excluded.cost",
        outputs.iter().map(|o| {
            let d = last[o];
            [Value::Integer(i64::from(*o)), Value::Blob(d.producer.to_vec()), Value::Integer(d.status), Value::Integer(d.cost)]
        }),
    )?;
    transaction
        .prepare_cached("DELETE FROM derivation_inputs WHERE output_atom IN (SELECT value FROM json_each(?1))")?
        .execute([serde_json::to_string(&outputs).expect("integers")])?;
    bounded_insert::insert(
        transaction,
        "INSERT INTO derivation_inputs(output_atom,ordinal,name_atom,digest)",
        "",
        outputs.iter().flat_map(|o| {
            last[o].inputs.iter().enumerate().map(move |(ordinal, (name, digest))| {
                [Value::Integer(i64::from(*o)), Value::Integer(ordinal as i64), name.map_or(Value::Null, |n| Value::Integer(i64::from(n))), Value::Blob(digest.to_vec())]
            })
        }),
    )?;
    Ok(())
}

/// Binds each atom to its digest, in order, as a few statements whatever their number: one read of the digests held,
/// the changes found in memory, one upsert of the names whose digest moved (a rebinding bumps its revision). Whether
/// each binding changed its name's digest: a first binding to `ABSENT` is none (it read as that already).
fn bind_all(transaction: &Connection, bindings: &[(Sym, [u8; 32])]) -> StoreResult<Vec<bool>> {
    if bindings.is_empty() {
        return Ok(Vec::new());
    }
    let mut names: Vec<Sym> = bindings.iter().map(|(n, _)| *n).collect();
    names.sort_unstable();
    names.dedup();
    let mut held: HashMap<Sym, [u8; 32]> = HashMap::with_capacity(names.len());
    {
        let mut read = transaction.prepare_cached("SELECT name_atom,digest FROM name_bindings WHERE name_atom IN (SELECT value FROM json_each(?1))")?;
        let rows = read.query_map([serde_json::to_string(&names).expect("integers")], |row| Ok((row.get::<_, Sym>(0)?, row.get::<_, Vec<u8>>(1)?)))?;
        for row in rows {
            let (name, bytes) = row?;
            held.insert(name, digest(&bytes)?);
        }
    }
    let mut changed = Vec::with_capacity(bindings.len());
    let mut writes = Vec::new();
    for (name, d) in bindings {
        let was = held.insert(*name, *d);
        if was == Some(*d) {
            changed.push(false);
            continue;
        }
        writes.push([Value::Integer(i64::from(*name)), Value::Blob(d.to_vec()), Value::Integer(0)]);
        changed.push(was.is_some() || d != &ABSENT);
    }
    bounded_insert::insert(
        transaction,
        "INSERT INTO name_bindings(name_atom,digest,revision)",
        "ON CONFLICT(name_atom) DO UPDATE SET digest=excluded.digest,revision=revision+1",
        writes,
    )?;
    Ok(changed)
}

fn digest(bytes: &[u8]) -> StoreResult<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| StoreError::Corrupt("derivation digest is not 32 bytes".to_owned()))
}

#[cfg(test)]
mod tests;
