//! The OS advisory lock behind Rerail's single main instance (`src/main-instance.ts`).
//!
//! `std::fs::File::try_lock` is `flock(LOCK_EX | LOCK_NB)` on Unix and `LockFileEx` on Windows.
//! Both belong to the open handle, so the OS drops the lock when the holder closes it or exits for
//! any reason, `SIGKILL` included; no stale state survives a crash and nothing is reclaimed.

use napi::{Error, Result, Status};
use napi_derive::napi;
use parking_lot::Mutex;
use std::fs::{File, OpenOptions, TryLockError};

/// An exclusive lock on one file, held until `release` or until the owning process exits.
#[napi]
pub struct ExclusiveFileLock {
    file: Mutex<Option<File>>,
}

#[napi]
impl ExclusiveFileLock {
    /// Unlocks and closes the file now rather than at process exit; later calls do nothing.
    #[napi]
    pub fn release(&self) {
        if let Some(file) = self.file.lock().take() {
            // Unlock before the close so the lock is free the moment this returns, even where a
            // duplicated handle (a forked child) would keep the open file description alive. An
            // unlock failure leaves the close, which releases the lock anyway.
            let _ = file.unlock();
        }
    }
}

/// Takes an exclusive lock on `path`, creating the file when missing. `None` when another handle
/// holds it, whether in another process or in this one.
#[napi]
pub fn try_lock_exclusive(path: String) -> Result<Option<ExclusiveFileLock>> {
    let failure = |action: &str, error: std::io::Error| {
        Error::new(
            Status::GenericFailure,
            format!("cannot {action} lock file {path}: {error}"),
        )
    };
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| failure("open", error))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(ExclusiveFileLock {
            file: Mutex::new(Some(file)),
        })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(failure("lock", error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_refuses_every_other_handle_until_released() {
        let directory = std::env::temp_dir().join(format!(
            "rerail-main-instance-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory
            .join("main-instance.lock")
            .to_string_lossy()
            .into_owned();

        let held = try_lock_exclusive(path.clone())
            .unwrap()
            .expect("the first handle takes the lock");
        assert!(
            try_lock_exclusive(path.clone()).unwrap().is_none(),
            "a second handle must be refused while the first holds the lock"
        );
        held.release();
        held.release();
        let again = try_lock_exclusive(path.clone()).unwrap();
        assert!(again.is_some(), "a released lock is free again");
        drop(again);

        std::fs::remove_dir_all(directory).unwrap();
    }
}
