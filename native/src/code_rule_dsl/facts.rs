//! The fact engine above the content store (docs/final-shape/03-architecture.md §1, §2, §14).

use crate::content_store::{Binding, ContentStore, Derivation, DerivationRecord, StoreResult};

// The push path is wired into the workspace scan in a later slice; until then only tests call it.
#[allow(dead_code)]
pub(crate) mod core;
#[allow(dead_code)]
pub(crate) mod families;
#[allow(dead_code)]
pub(crate) mod memo;
#[allow(dead_code)]
pub(crate) mod observe;
#[allow(dead_code)]
pub(crate) mod overlay;
#[allow(dead_code)]
pub(crate) mod pages;
#[allow(dead_code)]
pub(crate) mod propagate;
#[allow(dead_code)]
pub(crate) mod runs;
#[allow(dead_code)]
pub(crate) mod vtab;

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
}
