//! The fact engine above the content store (docs/final-shape/03-architecture.md §1, §2, §14).

use crate::content_store::{Binding, ContentStore, Derivation, DerivationRecord, StoreResult};

// The push path is wired into the workspace scan in a later slice; until then only tests call it.
#[allow(dead_code)]
pub(crate) mod core;
pub(crate) mod nearest;
#[allow(dead_code)]
pub(crate) mod families;
#[allow(dead_code)]
pub(crate) mod memo;
/// M's layered map as copied; `overlay` is its adaptation (open item O19).
#[allow(dead_code)]
pub(crate) mod lmap;
#[allow(dead_code)]
pub(crate) mod facts_a;
/// D's projection batches as copied; `runs` is their adaptation (open item O19).
#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod d_projection;
#[allow(dead_code)]
pub(crate) mod observe;
#[allow(dead_code)]
pub(crate) mod overlay;
#[allow(dead_code)]
pub(crate) mod pages;
#[allow(dead_code)]
pub(crate) mod producers;
#[allow(dead_code)]
pub(crate) mod propagate;
#[allow(dead_code)]
pub(crate) mod runs;
#[cfg(test)]
mod set_census;
#[allow(dead_code)]
pub(crate) mod vtab;
/// `core.rr`'s repository inputs (decision 0046).
pub(crate) mod repository;
/// M's Y1 reference graph as copied (05 Phase 1 row 2's source; open item O19).
#[allow(dead_code, unused_imports, clippy::all)]
// Unix only (not the browser build, not Windows): it forks its checks and reads macOS' and Linux' memory counters.
#[cfg(unix)]
pub(crate) mod ygraph;

/// The derivation graph as the push path reads and writes it: the store itself (every write a
/// transaction) or the overlay over it (writes in memory, written back in the background).
#[allow(dead_code)]
pub(crate) trait Graph {
    /// The store the graph's facts live in (Git trees, sources, the facts memo).
    fn store(&self) -> &ContentStore;
    /// Binds each name; returns the names whose digest changed, in input order.
    fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>>;
    /// Records each derivation and binds its output; returns the outputs whose digest changed.
    fn record_derivations(&self, records: &[DerivationRecord]) -> StoreResult<Vec<String>>;
    fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>>;
    /// Every derivation that read one of `names`, each once, ordered by output.
    fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>>;
    fn derivation_valid(&self, output: &str) -> StoreResult<bool>;
    /// Marks a push begun in SQLite before the push writes anything (open-items O1).
    fn begin_push(&self) -> StoreResult<()>;
    /// Marks the push ended once all its writes are in SQLite.
    fn end_push(&self) -> StoreResult<()>;
    /// Publishes tapes' prefilter rows: before any derivation recorded after this call reaches
    /// SQLite (a recorded prefilter derivation claims its rows), and in the store's working set
    /// at once. The store writes them now; the overlay queues them for its writer.
    fn publish_prefilters(&self, sets: Vec<crate::content_store::prefilter::TapePrefilter>) -> StoreResult<()> {
        for sets in sets.chunks(64) {
            self.store().put_prefilters_bulk(sets)?;
        }
        Ok(())
    }
}

impl Graph for ContentStore {
    fn store(&self) -> &ContentStore {
        self
    }
    fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>> {
        ContentStore::bind_names(self, bindings)
    }
    fn record_derivations(&self, records: &[DerivationRecord]) -> StoreResult<Vec<String>> {
        ContentStore::record_derivations(self, records)
    }
    fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        ContentStore::binding(self, name)
    }
    fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>> {
        ContentStore::derivations_reading(self, names)
    }
    fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        ContentStore::derivation_valid(self, output)
    }
    fn begin_push(&self) -> StoreResult<()> {
        ContentStore::begin_push(self)
    }
    fn end_push(&self) -> StoreResult<()> {
        ContentStore::end_push(self)
    }
}
