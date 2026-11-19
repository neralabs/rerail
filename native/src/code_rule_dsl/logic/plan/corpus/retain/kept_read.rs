//! A space's kept facts bound where they lie (decision 0401 F3, step 7). A process maps a space's two blocks once
//! for each commit of them (`Bound::of`), and a kept entry it binds (a clean derived relation, a kept input's
//! granules, a delta's old rows) is an engine relation over the mapping: its rows the entry's relation, its dead
//! bits and levels the entry's. No row is decoded or copied (0353).

use crate::code_rule_dsl::facts::rows::{Facts, Life, Part, Rows, Shared};
use crate::code_rule_dsl::logic::native::component::InRel;
use crate::content_store::kept_facts::{KeptSpace, SpaceRow};
use crate::content_store::{ContentStore, StoreResult};
use rerail_blocks::kept::{Kept, KeptFacts, KeptRead, KeptView};
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

#[cfg(test)]
mod tests;
