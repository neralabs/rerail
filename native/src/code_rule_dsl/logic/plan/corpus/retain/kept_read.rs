//! A space's kept facts bound where they lie (decision 0401 F3, step 7). A process maps a space's two blocks once
//! for each commit of them (`Bound::of`), and a kept entry it binds (a clean derived relation, a kept input's
//! granules, a delta's old rows) is an engine relation over the mapping: its rows the entry's relation, its dead
//! bits and levels the entry's. No row is decoded or copied (0353).

use crate::code_rule_dsl::facts::rows::{Facts, Life, Part, Rows, Shared};
use crate::code_rule_dsl::logic::native::component::InRel;
use crate::content_store::kept_facts::{KeptSpace, SpaceRow};
use crate::content_store::{ContentStore, StoreResult};
use crate::content_store::atoms::Atoms;
use rerail_blocks::kept::{FactKey, Kept, KeptFacts, KeptRead, KeptView};
use rerail_blocks::relation::Relation;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A space's blocks as a commit left them, mapped, each shared for the engine's relations (`Facts::over`).
pub(in crate::code_rule_dsl) struct Bound {
    space: KeptSpace,
    /// The base's and the delta's blocks, as the engine's parts hold them.
    facts: [Option<(Arc<KeptFacts>, Arc<Facts>)>; 2],
}

/// Each space's blocks this process mapped, by the store's file and the space, with the row that named them: mapped
/// again only when a commit changed the row.
static BOUND: Mutex<Option<HashMap<(String, [u8; 32]), (SpaceRow, Arc<Bound>)>>> = Mutex::new(None);

impl Bound {
    /// `space`'s blocks as the store's last commit has them (this process's queued writes to it written first);
    /// `None` when it keeps none. Read before a run's first wave.
    pub(in crate::code_rule_dsl) fn of(store: &ContentStore, space: &[u8; 32]) -> StoreResult<Option<Arc<Bound>>> {
        let Some(row) = store.kept_row(space)? else { return Ok(None) };
        let key = (store.database_path().to_string_lossy().into_owned(), *space);
        if let Some((was, bound)) = BOUND.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key))
            && *was == row
        {
            return Ok(Some(Arc::clone(bound)));
        }
        let space_blocks = store.open_kept(space, &row)?;
        let share = |f: &Option<Arc<KeptFacts>>| {
            f.as_ref().map(|f| {
                let keep: Arc<dyn Send + Sync> = Arc::clone(f) as Arc<dyn Send + Sync>;
                (Arc::clone(f), Facts::over(f.block().share(keep)))
            })
        };
        let bound = Arc::new(Bound { facts: [share(&space_blocks.base), share(&space_blocks.delta)], space: space_blocks });
        BOUND.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, (row, Arc::clone(&bound)));
        Ok(Some(bound))
    }

    /// The space as one: the delta's entry for a key in place of the base's.
    pub(in crate::code_rule_dsl) fn view(&self) -> KeptView<'_> {
        self.space.view()
    }

    /// The block holding `rows`, and its engine handle.
    fn holder(&self, rows: &Relation) -> Option<&(Arc<KeptFacts>, Arc<Facts>)> {
        self.facts.iter().flatten().find(|(k, _)| k.holds(rows))
    }

    /// `kept`'s rows as a part, where they lie.
    pub(in crate::code_rule_dsl) fn part(&self, kept: &Kept<'_>) -> Option<Part> {
        let (_, facts) = self.holder(kept.rows)?;
        Some(Part::over_mapped(Arc::clone(facts), kept.rows))
    }

    /// `kept` as an engine relation of types `types` (the engine's codes): its rows, dead bits and levels where they
    /// lie.
    pub(in crate::code_rule_dsl) fn rel(&self, kept: &Kept<'_>, types: Vec<u8>) -> Option<Arc<InRel>> {
        let (block, facts) = self.holder(kept.rows)?;
        let k = kept.rows.arity as usize;
        if types.len() != k {
            return None;
        }
        let n = kept.rows.rows.len() as usize;
        if !kept.levels.is_empty() && kept.levels.len() != n {
            return None;
        }
        let mut rows = Rows::living(k, Life::Kept);
        if n > 0 {
            rows.append_part(Part::over_mapped(Arc::clone(facts), kept.rows));
        }
        let keep: Arc<dyn Send + Sync> = Arc::clone(block) as Arc<dyn Send + Sync>;
        Some(InRel::held_shared(rows.sealed(), n, types, Shared::mapped(Arc::clone(&keep), kept.dead), Shared::mapped(keep, kept.levels)))
    }

    /// Entries `parts` (a kept input's granules, in their order) as one engine relation of types `types`: each
    /// granule's rows a segment, where they lie (as a chunk's exports are gathered, `Rows::append_part`).
    pub(in crate::code_rule_dsl) fn gathered(&self, parts: &[Kept<'_>], types: Vec<u8>) -> Option<Arc<InRel>> {
        let k = types.len();
        let mut rows = Rows::living(k, Life::Kept);
        let mut n = 0usize;
        for p in parts {
            if p.rows.arity as usize != k {
                return None;
            }
            let len = p.rows.rows.len() as usize;
            if len > 0 {
                rows.append_part(self.part(p)?);
            }
            n += len;
        }
        Some(InRel::held(rows.sealed(), n, types, Vec::new()))
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
}

impl Bound {
    /// The entry of `rel` (its name and identity) at partition `part`, if the space keeps one.
    pub(in crate::code_rule_dsl) fn entry(self: &Arc<Self>, rel: &str, ident: &[u8; 32], part: i64) -> Option<Entry> {
        self.view().find(&FactKey { rel, ident, part })?;
        Some(Entry { bound: Arc::clone(self), rel: rel.to_owned(), ident: *ident, part })
    }
}

impl Entry {
    fn kept(&self) -> Kept<'_> {
        self.bound.view().find(&FactKey { rel: &self.rel, ident: &self.ident, part: self.part }).expect("an entry of its space")
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
        self.kept().rows.rows.len() as usize
    }
    pub(in crate::code_rule_dsl) fn width(&self) -> usize {
        self.kept().rows.arity as usize
    }
    /// Its rows as an engine relation (`Bound::rel`), its types the engine's (an atom an integer); `None` when its
    /// atoms are not this process's numbering (`readable`).
    pub(in crate::code_rule_dsl) fn rel(&self, atoms: &Atoms) -> Option<Arc<InRel>> {
        let kept = self.kept();
        if !readable(&kept, atoms) {
            return None;
        }
        self.bound.rel(&kept, kept.types.iter().map(|&t| if t == b's' { b'i' } else { t }).collect())
    }
    /// `rel`, held under the engine's `types` (its own, of the entry's width).
    pub(in crate::code_rule_dsl) fn rel_as(&self, types: Vec<u8>, atoms: &Atoms) -> Option<Arc<InRel>> {
        let kept = self.kept();
        if !readable(&kept, atoms) {
            return None;
        }
        self.bound.rel(&kept, types)
    }
    /// Its rows, `each` called with a batch of them at a time (row-major), read where they lie: for the small
    /// entries a run reads values of (an input's documents, statistics' words), and for readers that take rows.
    pub(in crate::code_rule_dsl) fn each_rows(&self, each: &mut dyn FnMut(&[i64])) {
        let kept = self.kept();
        let (k, n) = (kept.rows.arity as usize, kept.rows.rows.len());
        let mut buf: Vec<i64> = Vec::with_capacity(4096.min(n as usize) * k);
        for i in 0..n {
            for c in 0..k {
                buf.push(kept.rows.rows.get(i, c));
            }
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

/// Entries `parts` of one bound space (a kept input's granules, in their order) as one engine relation
/// (`Bound::gathered`), its types the engine's; `None` when one's atoms are not this process's numbering.
pub(in crate::code_rule_dsl) fn gathered(parts: &[Entry], width: usize, atoms: &Atoms) -> Option<Arc<InRel>> {
    let bound = parts.first().map(|p| Arc::clone(&p.bound));
    let Some(bound) = bound else { return Some(InRel::held(Rows::living(width, Life::Kept), 0, vec![b'i'; width], Vec::new())) };
    let view = bound.view();
    let kept: Vec<Kept<'_>> = parts.iter().map(|p| view.find(&FactKey { rel: &p.rel, ident: &p.ident, part: p.part })).collect::<Option<_>>()?;
    if !kept.iter().all(|k| readable(k, atoms)) {
        return None;
    }
    bound.gathered(&kept, vec![b'i'; width])
}

/// Whether `kept`'s atoms (its `s` columns' values, as the store numbers them) read as this process numbers them:
/// always, but past a fork (`Atoms::reads_in_place`), where they are not read in place and the entry is derived again.
fn readable(kept: &Kept<'_>, atoms: &Atoms) -> bool {
    let rel = kept.rows;
    let n = rel.rows.len();
    let mut max: Option<i64> = None;
    for (c, _) in kept.types.iter().enumerate().filter(|(_, t)| **t == b's') {
        if c >= rel.arity as usize || n == 0 {
            continue;
        }
        let m = match rel.stats.as_slice().get(c) {
            Some(s) => s.max,
            None => (0..n).map(|i| rel.rows.get(i, c)).max().unwrap_or(0),
        };
        max = Some(max.map_or(m, |x| x.max(m)));
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
