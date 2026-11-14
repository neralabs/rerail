//! Content store databases for tests, each in a folder of its own below the system's temporary
//! folder.

use super::STORE_FILE_NAME;
use super::connection::release_all_at;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `<name>-<process id>-<nanoseconds>` below the system's temporary folder.
fn unique_temporary_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()))
}

/// A new, empty folder `<name>-<process id>-<nanoseconds>` below the system's temporary folder,
/// which no other test or run uses. Dropping it removes the folder and everything in it, also when
/// the test fails an assert, since `Drop` runs as the test unwinds.
pub(crate) struct TemporaryFolder(PathBuf);

impl TemporaryFolder {
    pub(crate) fn new(name: &str) -> Self {
        loop {
            let path = unique_temporary_path(name);
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                // Another call with the same name read the same clock value (Windows counts in
                // 100 ns steps); the next read gives a later one.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("cannot create {}: {error}", path.display()),
            }
        }
    }
}

impl std::ops::Deref for TemporaryFolder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TemporaryFolder {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A database path, `STORE_FILE_NAME` in a `TemporaryFolder` of its own. Dropping it closes every
/// thread's idle connection to the database, then removes the folder with the database and every
/// file SQLite or the test made beside it: a store leaves its thread's connection open after it
/// drops, and Windows cannot remove an open file.
///
/// Bind it to a name, never `_`, so it drops after every store on it. A hook or thread that needs
/// the path takes `to_path_buf()`: a closure the guard moves into removes the database when the
/// closure drops.
pub(crate) struct TemporaryDatabase {
    path: PathBuf,
    /// Dropped after `drop` has closed the connections, which removes the folder.
    _folder: TemporaryFolder,
}

impl TemporaryDatabase {
    pub(crate) fn new(name: &str) -> Self {
        let folder = TemporaryFolder::new(name);
        Self {
            path: folder.join(STORE_FILE_NAME),
            _folder: folder,
        }
    }
}

impl std::ops::Deref for TemporaryDatabase {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TemporaryDatabase {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        release_all_at(&self.path);
    }
}
