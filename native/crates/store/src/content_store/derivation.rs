//! The derivation graph (docs/final-shape/03-architecture.md §14): every fact family is the output
//! of a derivation, and the dependency engine and the fact engine are one mechanism.
//!
//! - A name (a workspace path, a Git ref, a buffer, a derivation's own output) is an atom
//!   (atoms.rs), bound to a 32-byte digest. Only observers rebind names; rebinding a name to the digest
//!   it already has changes nothing.
//! - A derivation records the names it read with the digest each had then, absent names included (bound to
//!   `ABSENT`), so it is valid while every one still binds to that digest. An input without a name is an immutable
//!   digest (a content hash).
//! - A derivation's output is itself a name, bound to the output's digest when it is recorded. A
//!   recomputed derivation whose digest is unchanged therefore rebinds nothing, and nothing that
//!   read it is enqueued (early cutoff, `facts::propagate`).
//! - The readers of a name are kept with it (`derivation_reader`): the next wave's frontier.
//!
//! Kept facts of the store (t33 step 1): one space, each relation's partition a name (the store's atom of it):
//! `name_binding(d0, d1, d2, d3, revision)` the name's digest, `derivation(p0, p1, p2, p3, status, cost)` and
//! `derivation_input(ordinal, named, name, d0, d1, d2, d3)` the derivation whose output it is, and
//! `derivation_reader(output)` the derivations that read it. A digest is four words, little-endian. A write is one
//! `write_space` in the store's write transaction, read as that transaction finds the space, so a binding's change
//! is decided against the rows it replaces. SQL reads the three tables of before through virtual tables of their
//! names and columns (`prefilter::vtab`).

#[allow(unused_imports)]
use crate::content_store::Cached as _;
use super::kept_facts::{ChainRows, KeptPut, KeptSpace, chain_rows, sorted_put, write_space};
use super::{ContentStore, StoreError, StoreResult, Sym};
use rerail_blocks::kept::{FactKey, Kept, KeptRead};
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeSet, HashMap};

pub use rerail_store_base::content_store::{ABSENT, Binding, Derivation, DerivationInput, DerivationRecord, StoredDerivation};

const BINDING: &str = "name_binding";
const DERIVATION: &str = "derivation";
const INPUT: &str = "derivation_input";
const READER: &str = "derivation_reader";

/// The derivation graph's space.
pub fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.derivation/v1").into()
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.derivation/v1\0{rel}").as_bytes()).into()
}

fn entry<'a, R: KeptRead + ?Sized>(view: &'a R, rel: &str, part: i64) -> Option<Kept<'a>> {
    let ident = ident(rel);
    view.get(&FactKey { rel, ident: &ident, part }).filter(|e| *e.key.ident == ident)
}

fn words(d: &[u8; 32]) -> [i64; 4] {
    std::array::from_fn(|i| i64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().expect("eight bytes")))
}

fn of_words(w: impl Fn(usize) -> i64) -> [u8; 32] {
    let mut d = [0; 32];
    for i in 0..4 {
        d[i * 8..i * 8 + 8].copy_from_slice(&w(i).to_le_bytes());
    }
    d
}

/// The digest name `name` (the store's atom) is bound to, and its revision.
pub(in crate::content_store) fn bound<R: KeptRead + ?Sized>(view: &R, name: i64) -> Option<([u8; 32], i64)> {
    let e = entry(view, BINDING, name).filter(|e| e.rows.len() == 1)?;
    Some((of_words(|i| e.rows.get(0, i)), e.rows.get(0, 4)))
}

/// Every bound name, ascending.
pub(in crate::content_store) fn names<R: KeptRead + ?Sized>(view: &R) -> impl Iterator<Item = i64> + '_ {
    let id = ident(BINDING);
    view.of(BINDING).filter(move |e| *e.key.ident == id).map(|e| e.key.part)
}

/// Every derivation's output, ascending.
pub(in crate::content_store) fn outputs<R: KeptRead + ?Sized>(view: &R) -> impl Iterator<Item = i64> + '_ {
    let id = ident(DERIVATION);
    view.of(DERIVATION).filter(move |e| *e.key.ident == id).map(|e| e.key.part)
}

/// The derivation whose output is `output`, by the store's atoms.
pub(in crate::content_store) fn stored<R: KeptRead + ?Sized>(view: &R, output: i64) -> Option<StoredDerivation> {
    let e = entry(view, DERIVATION, output).filter(|e| e.rows.len() == 1)?;
    let producer = of_words(|i| e.rows.get(0, i));
    let (status, cost) = (e.rows.get(0, 4), e.rows.get(0, 5));
    // Rows in canonical order: by ordinal.
    let inputs = entry(view, INPUT, output)
        .map(|e| (0..e.rows.len()).map(|r| ((e.rows.get(r, 1) != 0).then(|| e.rows.get(r, 2) as Sym), of_words(|i| e.rows.get(r, 3 + i)))).collect())
        .unwrap_or_default();
    Some(StoredDerivation { producer, status, cost, inputs })
}

/// The outputs of the derivations that read `name`, ascending (the store's atoms).
pub(in crate::content_store) fn readers<R: KeptRead + ?Sized>(view: &R, name: i64) -> Vec<Sym> {
    entry(view, READER, name).map(|e| (0..e.rows.len()).map(|r| e.rows.get(r, 0) as Sym).collect()).unwrap_or_default()
}

/// Whether `derivation` holds: every name it read still binds to the digest it read (an unbound name to `ABSENT`).
fn holds<R: KeptRead + ?Sized>(view: &R, derivation: &StoredDerivation) -> bool {
    derivation.inputs.iter().all(|(name, d)| name.is_none_or(|n| bound(view, i64::from(n)).map_or(ABSENT, |b| b.0) == *d))
}

fn open(store: &std::path::Path, t: &Connection) -> StoreResult<Option<KeptSpace>> {
    let rows = chain_rows(t, &space())?;
    if rows.is_empty() { Ok(None) } else { KeptSpace::open(store, &space(), &rows).map(Some) }
}

/// The puts of one write against the space as `old` holds it: each binding, in order (a rebinding bumps its
/// revision), and each derivation with its inputs replaced (a derivation listed twice is its last), its names'
/// readers kept. Whether each binding changed its name's digest: a first binding to `ABSENT` is none (it read as
/// that already).
fn graph_puts(old: Option<&KeptSpace>, bindings: &[(Sym, [u8; 32])], derivations: &[(Sym, StoredDerivation)]) -> StoreResult<(Vec<KeptPut>, Vec<bool>)> {
    let view = old.map(KeptSpace::view);
    let view = view.as_ref();
    let mut puts = Vec::new();
    let mut last: HashMap<Sym, &StoredDerivation> = HashMap::with_capacity(derivations.len());
    for (output, derivation) in derivations {
        last.insert(*output, derivation);
    }
    let mut outputs: Vec<Sym> = last.keys().copied().collect();
    outputs.sort_unstable();
    let mut readers: HashMap<Sym, BTreeSet<Sym>> = HashMap::new();
    fn reading<'r>(readers: &'r mut HashMap<Sym, BTreeSet<Sym>>, view: Option<&rerail_blocks::kept::KeptChain<'_>>, name: Sym) -> &'r mut BTreeSet<Sym> {
        readers.entry(name).or_insert_with(|| view.map(|v| self::readers(v, i64::from(name))).unwrap_or_default().into_iter().collect())
    }
    for &output in &outputs {
        let d = last[&output];
        if let Some(was) = view.and_then(|v| stored(v, i64::from(output))) {
            for name in was.inputs.iter().filter_map(|(n, _)| *n) {
                reading(&mut readers, view, name).remove(&output);
            }
        }
        for name in d.inputs.iter().filter_map(|(n, _)| *n) {
            reading(&mut readers, view, name).insert(output);
        }
        let part = i64::from(output);
        puts.push(sorted_put(DERIVATION, ident(DERIVATION), part, b"iiiiii", 1, |push| {
            words(&d.producer).into_iter().for_each(&mut *push);
            push(d.status);
            push(d.cost);
            Ok::<(), StoreError>(())
        })?);
        puts.push(sorted_put(INPUT, ident(INPUT), part, b"iisiiii", d.inputs.len(), |push| {
            for (ordinal, (name, digest)) in d.inputs.iter().enumerate() {
                push(ordinal as i64);
                push(i64::from(name.is_some()));
                push(name.map_or(0, i64::from));
                words(digest).into_iter().for_each(&mut *push);
            }
            Ok::<(), StoreError>(())
        })?);
    }
    let mut touched: Vec<(Sym, BTreeSet<Sym>)> = readers.into_iter().collect();
    touched.sort_unstable_by_key(|(n, _)| *n);
    for (name, outputs) in touched {
        puts.push(sorted_put(READER, ident(READER), i64::from(name), b"s", outputs.len(), |push| {
            outputs.iter().for_each(|&o| push(i64::from(o)));
            Ok::<(), StoreError>(())
        })?);
    }
    let mut held: HashMap<Sym, Option<([u8; 32], i64)>> = HashMap::with_capacity(bindings.len());
    let mut changed = Vec::with_capacity(bindings.len());
    let mut moved = Vec::new();
    for &(name, d) in bindings {
        let now = held.entry(name).or_insert_with(|| view.and_then(|v| bound(v, i64::from(name))));
        if now.is_some_and(|(was, _)| was == d) {
            changed.push(false);
            continue;
        }
        changed.push(now.is_some() || d != ABSENT);
        *now = Some((d, now.map_or(0, |(_, revision)| revision + 1)));
        moved.push(name);
    }
    moved.sort_unstable();
    moved.dedup();
    for name in moved {
        let (d, revision) = held[&name].expect("bound");
        puts.push(sorted_put(BINDING, ident(BINDING), i64::from(name), b"iiiii", 1, |push| {
            words(&d).into_iter().for_each(&mut *push);
            push(revision);
            Ok::<(), StoreError>(())
        })?);
    }
    Ok((puts, changed))
}


impl ContentStore {
    /// Binds each name to its digest, in one transaction; returns the names whose digest changed
    /// (newly bound, or bound to another digest), in input order.
    pub fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>> {
        let names = self.intern_atoms(&bindings.iter().map(|b| b.name.as_str()).collect::<Vec<_>>())?;
        let named: Vec<(Sym, [u8; 32])> = names.iter().copied().zip(bindings.iter().map(|b| b.digest)).collect();
        let changed = self.write_graph(&named, &[])?;
        Ok(bindings.iter().zip(changed).filter(|(_, c)| *c).map(|(b, _)| b.name.clone()).collect())
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
        let changed = self.write_graph(&bindings, &derivations)?;
        Ok(records.iter().zip(changed).filter(|(_, c)| *c).map(|(r, _)| r.output.clone()).collect())
    }

    /// Writes `derivations` and then `bindings` (this process's atoms) in one transaction; whether each binding
    /// changed its name's digest. Its atoms are the store's (`derivations_to_store`), so it is written after a fork
    /// too (decision 0170).
    fn write_graph(&self, bindings: &[(Sym, [u8; 32])], derivations: &[(Sym, StoredDerivation)]) -> StoreResult<Vec<bool>> {
        let path = self.database_path.clone();
        let (changed, written) = super::atoms::mapped(|| {
            self.immediate_transaction(|t| {
                let (derivations, bindings) = self.derivations_to_store(t, derivations, bindings)?;
                let old = open(&path, t)?;
                let (puts, changed) = graph_puts(old.as_ref(), &bindings, &derivations)?;
                let written: Option<ChainRows> = if puts.is_empty() { None } else { write_space(&path, t, &space(), puts, None)? };
                Ok((changed, written))
            })
        })?;
        self.note_kept(&space(), written);
        Ok(changed)
    }

    /// The digest `name` is bound to, if any.
    pub fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        let Some(id) = self.atom(name)?.map(|id| self.stored_sym(id)).transpose()?.flatten() else {
            return Ok(None);
        };
        Ok(self.kept_space(&space())?.and_then(|k| bound(&k.view(), i64::from(id))).map(|b| b.0))
    }

    /// Every derivation that read one of `names`: the frontier those names' changes dirty, each
    /// once, ordered by output name.
    pub fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>> {
        // The names' atoms in one lookup, as the store numbers them.
        let ids: Vec<Sym> = self.atoms.lookup_all(&names.iter().map(String::as_str).collect::<Vec<_>>())?.into_iter().flatten().collect();
        let ids: Vec<Sym> = self.atoms.stored(&ids)?.into_iter().flatten().collect();
        let mut rows = Vec::new();
        if !ids.is_empty()
            && let Some(kept) = self.kept_space(&space())?
        {
            let view = kept.view();
            let outputs: BTreeSet<Sym> = ids.iter().flat_map(|&n| readers(&view, i64::from(n))).collect();
            for output in outputs {
                if let Some(d) = stored(&view, i64::from(output)) {
                    rows.push((output, d.producer, d.status, d.cost));
                }
            }
        }
        let frontier: BTreeSet<Derivation> =
            self.named_outputs(rows)?.into_iter().map(|(output, producer, status, cost)| Derivation { output, producer, status, cost }).collect();
        Ok(frontier.into_iter().collect())
    }

    /// Whether `output`'s derivation is stored and every name it read still binds to the digest
    /// it read (an unbound name to `ABSENT`).
    pub fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        let Some(id) = self.atom(output)?.map(|id| self.stored_sym(id)).transpose()?.flatten() else {
            return Ok(false);
        };
        let Some(kept) = self.kept_space(&space())? else { return Ok(false) };
        let view = kept.view();
        Ok(stored(&view, i64::from(id)).is_some_and(|d| holds(&view, &d)))
    }

    /// Marks a push begun, committed before the push writes anything (open-items O1).
    pub fn begin_push(&self) -> StoreResult<()> {
        self.immediate_transaction(|transaction| {
            transaction.execute_cached("INSERT OR IGNORE INTO push_pending(pending) VALUES (1)", [])?;
            Ok(())
        })
    }

    /// Marks the push ended: called only once every write of the push is in SQLite.
    pub fn end_push(&self) -> StoreResult<()> {
        self.immediate_transaction(|transaction| {
            transaction.execute_cached("DELETE FROM push_pending", [])?;
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

    /// Every stored derivation that is not valid (the validity test over all derivations), ordered
    /// by output: what an unclean push left behind.
    pub fn invalid_derivations(&self) -> StoreResult<Vec<Derivation>> {
        let mut rows = Vec::new();
        if let Some(kept) = self.kept_space(&space())? {
            let view = kept.view();
            for output in outputs(&view) {
                if let Some(d) = stored(&view, output).filter(|d| !holds(&view, d)) {
                    rows.push((output as Sym, d.producer, d.status, d.cost));
                }
            }
        }
        let mut rows = self.named_outputs(rows)?;
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(rows.into_iter().map(|(output, producer, status, cost)| Derivation { output, producer, status, cost }).collect())
    }

    /// The digest the atom `name` is bound to, if any.
    pub fn binding_of(&self, name: Sym) -> StoreResult<Option<[u8; 32]>> {
        let Some(name) = self.stored_sym(name)? else { return Ok(None) };
        Ok(self.kept_space(&space())?.and_then(|k| bound(&k.view(), i64::from(name))).map(|b| b.0))
    }

    /// The stored derivation whose output is the atom `output`, with its inputs in order.
    pub fn derivation_of(&self, output: Sym) -> StoreResult<Option<StoredDerivation>> {
        let Some(output) = self.stored_sym(output)? else { return Ok(None) };
        let Some(kept) = self.kept_space(&space())? else { return Ok(None) };
        let Some(mut derivation) = stored(&kept.view(), i64::from(output)) else { return Ok(None) };
        drop(kept);
        if self.atoms.forked() {
            let mut names: Vec<i64> = derivation.inputs.iter().filter_map(|(n, _)| n.map(i64::from)).collect();
            self.atoms.to_local(super::StoreIds::as_stored(&mut names))?;
            let mut names = names.into_iter();
            for (name, _) in &mut derivation.inputs {
                if name.is_some() {
                    *name = Some(names.next().expect("one per named input") as Sym);
                }
            }
        }
        Ok(Some(derivation))
    }

    /// The outputs of the stored derivations that read the atom `name`, ascending.
    pub fn readers_of(&self, name: Sym) -> StoreResult<Vec<Sym>> {
        let Some(name) = self.stored_sym(name)? else { return Ok(Vec::new()) };
        let Some(kept) = self.kept_space(&space())? else { return Ok(Vec::new()) };
        let mut readers = readers(&kept.view(), i64::from(name));
        drop(kept);
        if self.atoms.forked() {
            let mut local: Vec<i64> = readers.iter().map(|&r| i64::from(r)).collect();
            self.atoms.to_local(super::StoreIds::as_stored(&mut local))?;
            readers = local.into_iter().map(|r| r as Sym).collect();
            readers.sort_unstable();
        }
        readers.dedup();
        Ok(readers)
    }

    /// Writes one overlay layer in one transaction: each derivation with its inputs replaced, and each binding (a
    /// rebinding bumps its revision). The layer's own rows are already final, so nothing here compares or reports
    /// changes.
    pub fn write_derivation_layer(
        &self,
        bindings: &[(Sym, [u8; 32])],
        derivations: &[(Sym, StoredDerivation)],
    ) -> StoreResult<()> {
        self.write_graph(bindings, derivations).map(drop)
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
        self.atoms.to_store(transaction, super::ProcessIds::of_process(&mut values))?;
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
        self.atoms.to_local(super::StoreIds::as_stored(&mut ids))?;
        rows.into_iter()
            .zip(ids)
            .map(|((_, a, b, c), id)| {
                let name = self.atoms.value(id as Sym)?.ok_or_else(|| StoreError::Corrupt(format!("derivation output atom {id} is no atom")))?;
                Ok((name, a, b, c))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
