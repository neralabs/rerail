//! A shared read lease closes the gap between loading cached sources and publishing their roots.
//! Collectors try an exclusive lease. Handles release their locks on normal return, errors, or process exit.
use super::{ContentStore, StoreResult};
#[cfg(not(target_family = "wasm"))]
use std::fs::{File, OpenOptions};

/// Closing the handle releases the lease, including when unwinding or exiting the process.
pub(crate) struct CollectionLease {
    #[cfg(not(target_family = "wasm"))]
    _file: File,
}

impl Drop for CollectionLease {
    fn drop(&mut self) {
        // A subprocess can inherit this open file description between fork and exec.
        // Closing only our fd would leave its lock held until that subprocess execs or exits.
        #[cfg(not(target_family = "wasm"))]
        let _ = self._file.unlock();
    }
}

impl ContentStore {
    /// Keep cache objects alive while a reader builds a result or prepares a durable root.
    /// Acquire before looking up source ids and release only after publishing their references.
    pub(crate) fn hold_collection(&self) -> StoreResult<CollectionLease> {
        #[cfg(not(target_family = "wasm"))]
        {
            let file = self.collection_lock_file()?;
            file.lock_shared()?;
            Ok(CollectionLease { _file: file })
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
                Ok(()) => Ok(Some(CollectionLease { _file: file })),
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
