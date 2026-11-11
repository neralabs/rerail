//! Content store databases for tests, below the system's temporary folder.

use super::connection::ThreadConnection;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `<name>-<process id>-<nanoseconds>` below the system's temporary folder, a path no other test
/// or run uses.
pub(super) fn unique_temporary_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()))
}

/// A database path from `unique_temporary_path` ending `.sqlite`. Dropping it closes every
/// thread's idle connection to the database, then removes the database and the `-wal` and `-shm`
/// files beside it: a store leaves its thread's connection open after it drops, and Windows cannot
/// remove an open file.
///
/// Bind it to a name, never `_`, so it drops after every store on it. A hook or thread that needs
/// the path takes `to_path_buf()`: a closure the guard moves into removes the database when the
/// closure drops.
pub(crate) struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
    pub(crate) fn new(name: &str) -> Self {
        let mut path = unique_temporary_path(name).into_os_string();
        path.push(".sqlite");
        Self(path.into())
    }
}

impl std::ops::Deref for TemporaryDatabase {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TemporaryDatabase {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        ThreadConnection {
            database_path: self.0.clone(),
        }
        .release_all();
        for suffix in ["", "-wal", "-shm"] {
            let mut file = self.0.clone().into_os_string();
            file.push(suffix);
            let _ = std::fs::remove_file(file);
        }
    }
}
