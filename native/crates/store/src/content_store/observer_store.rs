//! The content store as the observer and git reach it: the provider of the store base's `observer_store::base`
//! (`Store`, `Kept`), which the observer (rerail-observe) and git below the store ask through `STORE`. The store
//! registers it at load (`register`).
use super::analysis_scope::AnalysisOwner;
use super::kept_facts::KeptPut;
pub use rerail_store_base::content_store::observer_store::base::*;
use super::{ContentStore, StoreResult};
use rerail_blocks::kept::KeptRead;
use std::path::Path;

/// The store's provider: opens the store at a path.
struct Provider;

impl Store for Provider {
    fn open(&self, path: &Path) -> Option<Box<dyn Kept>> {
        ContentStore::open_images(path).ok().map(|store| Box::new(store) as Box<dyn Kept>)
    }

    fn own_recorded_manifests(&self, path: &Path, root: &str, plans: &[[u8; 32]], owner: &AnalysisOwner, now: i64) -> StoreResult<usize> {
        ContentStore::open(path)?.own_recorded_manifests(root, plans, owner, now)
    }

    fn file_read(&self, db: &rusqlite::Connection, path: &Path, space: &[u8; 32], f: &mut dyn FnMut(Option<&dyn KeptRead>)) -> StoreResult<()> {
        let rows = super::kept_facts::chain_rows(db, space)?;
        if rows.is_empty() {
            f(None);
            return Ok(());
        }
        let kept = super::kept_facts::KeptSpace::open(path, space, &rows)?;
        let chain = kept.view();
        f(Some(&chain));
        Ok(())
    }

    fn file_write(&self, t: &rusqlite::Connection, path: &Path, space: &[u8; 32], puts: Vec<KeptPut>) -> StoreResult<()> {
        super::kept_facts::write_space(path, t, space, puts, None).map(drop)
    }
}

impl Kept for ContentStore {
    fn settle(&self, space: &[u8; 32]) {
        self.settle_kept(space);
    }

    fn text(&self, atom: u32) -> Option<String> {
        self.atom_value(atom).ok().flatten()
    }

    fn intern(&self, texts: &[&str]) -> Option<Vec<u32>> {
        self.intern_atoms(texts).ok()
    }

    fn read(&self, space: &[u8; 32], f: &mut dyn FnMut(Option<&dyn KeptRead>)) {
        match self.kept_space(space) {
            Ok(Some(space)) => {
                let chain = space.view();
                f(Some(&chain));
            }
            _ => f(None),
        }
    }

    fn queue(&self, space: [u8; 32], puts: Vec<KeptPut>, what: &'static str) {
        self.queue_kept_as(space, puts, what);
    }
}

/// Registers the store with the observer and git (`STORE`): boot does as the program loads
/// (`install_with`), and the store's own test binary as it loads.
pub fn register() {
    STORE.register("rerail-store", &Provider);
}
