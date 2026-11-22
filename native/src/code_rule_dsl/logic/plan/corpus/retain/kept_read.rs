//! A space's kept facts bound where they lie (decisions 0401 F3, 0456 K6, 0438). A process maps a space's chain of
//! packed files once for each commit of it (`Bound::of`), and a kept entry it binds (a clean derived relation, a kept
//! input's granules, a delta's old rows) is an engine relation over the mapping: a view of the entry's `Packed`
//! (`KeptSource`), its values read where they lie. No row is decoded or copied (0353), but for a relation whose levels
//! the entry keeps (decision 0090): those are packed beside its rows, and the relation is read in with them.

use rerail_blocks::vector::Vector;
use super::kept_source::KeptSource;
use crate::code_rule_dsl::logic::native::component::InRel;
use crate::content_store::atoms::Atoms;
use crate::content_store::kept_facts::{ChainRows, KeptReader, KeptSpace, READERS, record_idents};
use crate::content_store::{ContentStore, StoreResult};
use rerail_blocks::kept::{FactKey, Kept, KeptChain, KeptRead};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A space's chain as a commit left it, mapped: every view of one of its entries holds it.
pub(in crate::code_rule_dsl) struct Bound {
    space: KeptSpace,
    /// The space's identity: what a run asks of it is its program's (`KeptReader`).
    id: [u8; 32],
}

/// Each space's chain the runs sharing a shared object mapped, by the store's file and the space, with the rows that
/// named its files: mapped again only when a commit changed them. Shared by design (decision 0442): the shared object's.
#[derive(Default)]
struct BoundChains(Mutex<HashMap<(String, [u8; 32]), (ChainRows, Arc<Bound>)>>);

impl Bound {
    /// `space`'s chain as the store's last commit has it (this process's queued writes to it written first);
    /// `None` when it keeps none. Read before a run's first wave.
    pub(in crate::code_rule_dsl) fn of(store: &ContentStore, space: &[u8; 32]) -> StoreResult<Option<Arc<Bound>>> {
        let Some(rows) = store.kept_row(space)? else { return Ok(None) };
        let key = (store.database_path().to_string_lossy().into_owned(), *space);
        let chains = crate::code_rule_dsl::logic::profile::shared::<BoundChains>();
        if let Some((was, bound)) = chains.0.lock().unwrap_or_else(|e| e.into_inner()).get(&key)
            && *was == rows
        {
            return Ok(Some(Arc::clone(bound)));
        }
        let bound = Arc::new(Bound { space: store.open_kept(space, &rows)?, id: *space });
        chains.0.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (rows, Arc::clone(&bound)));
        Ok(Some(bound))
    }

    /// The chain as one: a key's newest entry.
    pub(in crate::code_rule_dsl) fn view(&self) -> KeptChain<'_> {
        self.space.view()
    }

    /// The entry of `rel` (its name and identity) at partition `part`, if the space keeps one.
    pub(in crate::code_rule_dsl) fn entry(self: &Arc<Self>, rel: &str, ident: &[u8; 32], part: i64) -> Option<Entry> {
        let rows: *const rerail_blocks::packed::Packed = self.view().get(&FactKey { rel, ident, part })?.rows;
        // Found: the run's program reads it, and its reader record names it (decision 0401 step 8). A miss keeps
        // nothing; what the run puts in its place, its puts' record names.
        crate::code_rule_dsl::logic::profile::with_bound::<KeptReader, _>(|r| {
            if let Some(log) = &r.0 {
                log.asked(&self.id, ident, |key| self.view().get(&FactKey { rel: READERS, ident: key, part: 0 }).map(|k| record_idents(&k).collect()).unwrap_or_default());
            }
        });
        Some(Entry { bound: Arc::clone(self), rel: rel.to_owned(), ident: *ident, part, rows })
    }
}

/// An entry of a bound space (`Bound::entry`), held past the borrow of its view: what the run checks of it is read
/// from the entry, its rows bound where they lie.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Entry {
    bound: Arc<Bound>,
    rel: String,
    ident: [u8; 32],
    part: i64,
    /// Its rows, in `bound`'s mapping.
    rows: *const rerail_blocks::packed::Packed,
}

// SAFETY: `rows` lies in a read-only mapping `bound` holds, shared by every thread that reads it.
unsafe impl Send for Entry {}
unsafe impl Sync for Entry {}

impl Entry {
    fn kept(&self) -> Kept<'_> {
        self.bound.view().entry(&FactKey { rel: &self.rel, ident: &self.ident, part: self.part }).expect("an entry of its space")
    }
    /// Its rows where they lie.
    pub(in crate::code_rule_dsl) fn packed(&self) -> &rerail_blocks::packed::Packed {
        // SAFETY: as above: the mapping lives as long as `bound`, which this holds.
        unsafe { &*self.rows }
    }
    pub(in crate::code_rule_dsl) fn digest(&self) -> u64 {
        self.kept().digest
    }
    pub(in crate::code_rule_dsl) fn reads(&self) -> Vec<(String, u64)> {
        self.kept().reads().map(|(r, d)| (r.to_owned(), d)).collect()
    }
    /// Its columns' value types as kept (`s` an atom).
    pub(in crate::code_rule_dsl) fn types(&self) -> Vec<u8> {
        self.kept().types.to_vec()
    }
    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.packed().len()
    }
    pub(in crate::code_rule_dsl) fn width(&self) -> usize {
        self.packed().arity as usize
    }
    /// Its rows as an engine relation, its types the engine's (an atom an integer); `None` when its atoms are not
    /// this process's numbering (`readable`).
    pub(in crate::code_rule_dsl) fn rel(&self, atoms: &Atoms) -> Option<Arc<InRel>> {
        let types = self.kept().types.iter().map(|&t| if t == b's' { b'i' } else { t }).collect();
        self.rel_as(types, atoms)
    }
    /// `rel`, held under the engine's `types` (its own, of the entry's width): a view of the entry where it lies
    /// (decision 0456 step 6b), or, when the entry keeps its rows' levels, its rows read in with them.
    pub(in crate::code_rule_dsl) fn rel_as(&self, types: Vec<u8>, atoms: &Atoms) -> Option<Arc<InRel>> {
        let rows = self.packed();
        let k = rows.arity as usize;
        if types.len() != k || !readable(&self.kept(), atoms) {
            return None;
        }
        let Some(levels) = rows.levels() else {
            return Some(InRel::over_source(Arc::new(KeptSource::new(Arc::clone(&self.bound), vec![self.rows])), types));
        };
        // copy: load: the rows and their levels into the run's relation, which maintains a recursive stratum by them.
        let n = rows.len();
        let mut flat = Vector::with_capacity(n * k);
        for i in 0..n {
            flat.extend((0..k).map(|c| rows.get(i, c)));
        }
        let mut level = Vector::with_capacity(n);
        levels.for_each(|v| level.push(v as u32));
        InRel::from_flat_levels(flat, n, types, level)
    }
    /// Its rows, `each` called with a batch of them at a time (row-major), read where they lie: for the small
    /// entries a run reads values of (an input's documents, statistics' words), and for readers that take rows.
    pub(in crate::code_rule_dsl) fn each_rows(&self, each: &mut dyn FnMut(&[i64])) {
        let rows = self.packed();
        let (k, n) = (rows.arity as usize, rows.len());
        // copy: not block data: a batch of decoded values handed to the reader, never kept.
        let mut buf: Vector<i64> = rerail_blocks::vector::Vector::with_capacity(4096.min(n) * k);
        for i in 0..n {
            buf.extend((0..k).map(|c| rows.get(i, c)));
            if buf.len() >= 4096 * k.max(1) {
                each(&buf);
                buf.clear();
            }
        }
        if !buf.is_empty() || k == 0 {
            each(&buf);
        }
    }
    /// The entry's space.
    pub(in crate::code_rule_dsl) fn bound(&self) -> &Arc<Bound> {
        &self.bound
    }
}

/// Entries `parts` of one bound space (a kept input's granules, in their order, each kept as given) as one engine
/// relation, its types the engine's: a view of the granules where they lie, one after another; `None` when one's
/// atoms are not this process's numbering.
pub(in crate::code_rule_dsl) fn gathered(parts: &[Entry], width: usize, atoms: &Atoms) -> Option<Arc<InRel>> {
    let Some(first) = parts.first() else { return Some(InRel::from_flat(rerail_blocks::vector::Vector::new(), 0, vec![b'i'; width])) };
    if parts.iter().any(|p| p.width() != width || !Arc::ptr_eq(&p.bound, &first.bound) || !readable(&p.kept(), atoms)) {
        return None;
    }
    let source = KeptSource::new(Arc::clone(&first.bound), parts.iter().map(|p| p.rows).collect());
    Some(InRel::over_source(Arc::new(source), vec![b'i'; width]))
}

/// Whether `kept`'s atoms (its `s` columns' values, as the store numbers them) read as this process numbers them:
/// always where this process's ids are the store's (`Atoms::holds_by_reading`: not forked, and leased); else held to
/// their greatest (`reads_in_place`), and past a fork (decision 0170) not read in place, the entry derived again.
fn readable(kept: &Kept<'_>, atoms: &Atoms) -> bool {
    if !atoms.holds_by_reading() {
        return true;
    }
    let rel = kept.rows;
    let mut max: Option<i64> = None;
    for (c, _) in kept.types.iter().enumerate().filter(|(_, t)| **t == b's') {
        if c >= rel.arity as usize {
            continue;
        }
        rel.col(c).for_each(|v| max = Some(max.map_or(v, |m| m.max(v))));
    }
    let Some(max) = max else { return true };
    match atoms.reads_in_place(max) {
        Ok(true) => true,
        _ => {
            crate::code_rule_dsl::logic::profile::decision("a kept entry past a fork (decision 0170)", || format!("{}: its atoms are another numbering's; derived again", kept.key.rel));
            false
        }
    }
}

#[cfg(test)]
mod tests;
