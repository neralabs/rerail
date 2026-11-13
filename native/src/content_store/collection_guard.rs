//! A shared read lease closes the gap between loading cached sources and publishing their roots.
//! Collectors try an exclusive lease. Handles release their locks on normal return, errors, or process exit.
use super::{ContentStore, StoreResult};
#[cfg(not(target_family = "wasm"))]
use std::fs::{File, OpenOptions};

/// A handle's shared lease: the lock file, opened once, and how many leases hold it. A lock belongs to the open
/// file, so leases on one handle share it; the last one to go unlocks it. Was: the file canonicalized, opened, locked
/// and closed for every lease, once per guest the Python pack parses (40,000, about 1.4 s).
#[derive(Default)]
pub(super) struct SharedLease {
    #[cfg(not(target_family = "wasm"))]
    state: std::sync::Mutex<(Option<File>, usize)>,
}

/// Closing the handle releases the lease, including when unwinding or exiting the process.
pub struct CollectionLease {
    #[cfg(not(target_family = "wasm"))]
    held: Held,
}

#[cfg(not(target_family = "wasm"))]
enum Held {
    /// A collector's exclusive lease: its own file.
    Own(File),
    /// A reader's: the handle's shared lease, counted.
    Shared(std::sync::Arc<SharedLease>),
}

#[cfg(all(test, not(target_family = "wasm")))]
impl CollectionLease {
    /// A duplicate of the lease's file descriptor, as a child inherits it at fork.
    fn duplicate(&self) -> File {
        match &self.held {
            Held::Own(file) => file.try_clone().unwrap(),
            Held::Shared(shared) => shared.state.lock().unwrap().0.as_ref().expect("held").try_clone().unwrap(),
        }
    }
}

impl Drop for CollectionLease {
    fn drop(&mut self) {
        // A subprocess can inherit this open file description between fork and exec.
        // Closing only our fd would leave its lock held until that subprocess execs or exits.
        #[cfg(not(target_family = "wasm"))]
        match &self.held {
            Held::Own(file) => {
                let _ = file.unlock();
            }
            Held::Shared(shared) => {
                let mut state = shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                state.1 -= 1;
                if state.1 == 0
                    && let Some(file) = &state.0
                {
                    let _ = file.unlock();
                }
            }
        }
    }
}

impl ContentStore {
    /// Keep cache objects alive while a reader builds a result or prepares a durable root.
    /// Acquire before looking up source ids and release only after publishing their references.
    pub fn hold_collection(&self) -> StoreResult<CollectionLease> {
        #[cfg(not(target_family = "wasm"))]
        {
            let shared = std::sync::Arc::clone(&self.lease);
            {
                let mut state = shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.1 == 0 {
                    if state.0.is_none() {
                        state.0 = Some(self.collection_lock_file()?);
                    }
                    state.0.as_ref().expect("opened").lock_shared()?;
                }
                state.1 += 1;
            }
            Ok(CollectionLease { held: Held::Shared(shared) })
        }
        #[cfg(target_family = "wasm")]
        {
            Ok(CollectionLease {})
        }
    }

    /// Collection yields to readers rather than waiting behind an active analysis.
    pub(super) fn try_collect(&self) -> StoreResult<Option<CollectionLease>> {
        #[cfg(not(target_family = "wasm"))]
        {
            let file = self.collection_lock_file()?;
            match file.try_lock() {
                Ok(()) => Ok(Some(CollectionLease { held: Held::Own(file) })),
                Err(std::fs::TryLockError::WouldBlock) => Ok(None),
                Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
            }
        }
        #[cfg(target_family = "wasm")]
        {
            Ok(Some(CollectionLease {}))
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn collection_lock_file(&self) -> StoreResult<File> {
        // Resolve aliases so separate handles and processes protect the same database.
        let mut path = std::fs::canonicalize(&self.database_path)?.into_os_string();
        path.push(".collection.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(options.open(path)?)
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests;
