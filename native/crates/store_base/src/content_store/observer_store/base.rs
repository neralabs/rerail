//! The content store as the observer and git reach it (rerail-store-base): what they need of it (`Store`, `Kept`) and
//! where the store registers it at load (`STORE`). The observer and git are below the store (rerail-observe, crate
//! split 7b), yet the observer's answers, canonical forms and settings are kept facts of the store (t33 K2), and git
//! owns again the manifests a run recorded there. Here, below both, one slot is the store's for every build of them:
//! the store's provider (`observer_store.rs`, rerail-store) registers it. The statistics' learned model, a kept fact of
//! their own file (logic-core's `costs::models`, below the store too), is read and written through it as well.
use super::super::kept_facts::base::KeptPut;
use rerail_blocks::kept::KeptRead;
use std::path::Path;

/// What the observer and git need of the content store, the store's (registered at load, `STORE`): a store at a
/// path opened for its kept facts, and the manifests a run recorded there.
pub trait Store: Sync {
    /// The store at `path`, opened for its images (`ContentStore::open_images`); `None` when it cannot be.
    fn open(&self, path: &Path) -> Option<Box<dyn Kept>>;
    /// The manifests under `root` that `plans` recorded, owned again for `owner` at `now`
    /// (`ContentStore::own_recorded_manifests`) in the store at `path`.
    fn own_recorded_manifests(&self, path: &Path, root: &str, plans: &[[u8; 32]], owner: &super::super::analysis_scope::AnalysisOwner, now: i64) -> super::super::StoreResult<usize>;
    /// `f` of `space`'s chain in the kept facts beside the file at `path`, a database that is not a content store
    /// (the statistics' `costs.sqlite`: its learned model, t33 step 6), read on `db`, its connection
    /// (`kept_facts::chain_rows`, `KeptSpace::open`); `f` of `None` when the space has no file.
    fn file_read(&self, db: &rusqlite::Connection, path: &Path, space: &[u8; 32], f: &mut dyn FnMut(Option<&dyn KeptRead>)) -> super::super::StoreResult<()>;
    /// Writes `puts` as `space`'s next file beside the file at `path`, in `t`, its connection's open transaction
    /// (`kept_facts::write_space`).
    fn file_write(&self, t: &rusqlite::Connection, path: &Path, space: &[u8; 32], puts: Vec<KeptPut>) -> super::super::StoreResult<()>;
}

/// An open store, as the observer's kept facts read and write a space of it.
pub trait Kept: Send {
    /// Waits until every put queued for `space` is written (`ContentStore::settle_kept`).
    fn settle(&self, space: &[u8; 32]);
    /// An atom's text (`ContentStore::atom_value`).
    fn text(&self, atom: u32) -> Option<String>;
    /// The atoms of `texts`, interned (`ContentStore::intern_atoms`).
    fn intern(&self, texts: &[&str]) -> Option<Vec<u32>>;
    /// `f` of `space`'s chain as kept (`ContentStore::kept_space`), or of `None` when it has none.
    fn read(&self, space: &[u8; 32], f: &mut dyn FnMut(Option<&dyn KeptRead>));
    /// Queues `puts` as one put of `space`, a job called `what` (`ContentStore::queue_kept_as`).
    fn queue(&self, space: [u8; 32], puts: Vec<KeptPut>, what: &'static str);
}

/// The content store, as the observer and git reach it: rerail-store registers it at load.
pub static STORE: rerail_registry::Slot<&'static dyn Store> = rerail_registry::Slot::new("the content store, as the observer reaches it (content_store::observer_store::base::STORE)");

