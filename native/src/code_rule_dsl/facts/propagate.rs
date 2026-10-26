//! Maintenance of the derivation graph (docs/final-shape/03-architecture.md §2, §14), adapted
//! from Y1's propagate (Moti-Velz/facts-engine `spikes/facts-b/src/ygraph/engine.rs`).
//!
//! Y1 re-ran one memoised query at a time, in level order, from in-memory reverse indexes. Here
//! the read sets are `derivation_inputs` rows in the content store and the unit is a wave: every
//! derivation that read a changed name is recomputed together, as one batch handed to the
//! producer, and the outputs whose digest changed are the next wave's changed names. A re-run
//! whose output digest is unchanged rebinds nothing, so its readers are never enqueued (early
//! cutoff, Y1's rule). There is no level order: a derivation reached again by a later wave runs
//! again, which `Work` counts.

use crate::content_store::{ContentStore, Derivation, DerivationRecord, StoreError, StoreResult};

/// Counts of one propagation. `rerun - changed` re-runs produced what they produced before: the
/// work spurious invalidation costs (C19).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Work {
    pub waves: u32,
    pub rerun: u64,
    pub changed: u64,
}

/// Recomputes, wave by wave, every derivation that read a changed name, starting from `changed`
/// (names an observer rebound), until a wave changes nothing. `recompute` receives one wave's
/// frontier and returns its derivations as re-run; a frontier is never empty. More than
/// `max_waves` waves is an error: a cycle whose digests do not settle.
pub(crate) fn propagate(
    store: &ContentStore,
    mut changed: Vec<String>,
    max_waves: u32,
    mut recompute: impl FnMut(&[Derivation]) -> StoreResult<Vec<DerivationRecord>>,
) -> StoreResult<Work> {
    let mut work = Work::default();
    while !changed.is_empty() {
        let frontier = store.derivations_reading(&changed)?;
        if frontier.is_empty() {
            break;
        }
        if work.waves == max_waves {
            return Err(StoreError::Invalid(format!(
                "derivations did not settle in {max_waves} waves"
            )));
        }
        work.waves += 1;
        let records = recompute(&frontier)?;
        work.rerun += records.len() as u64;
        changed = store.record_derivations(&records)?;
        work.changed += changed.len() as u64;
    }
    Ok(work)
}

#[cfg(test)]
mod tests;
