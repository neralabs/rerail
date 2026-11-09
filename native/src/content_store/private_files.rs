//! Keeps the store's folder and files private to the user who runs Rerail.
//!
//! The store holds copies of every source file the engine read. A folder made `0700` and a database made `0600`
//! before SQLite opens it stay private: SQLite gives the `-wal`, `-shm` and `-journal` files it creates the
//! database file's mode. `mkdir` and `open` apply a mode only when they create, so on Unix an existing folder or
//! file that allows group or other access (an older release's, a loose umask's) loses that access when the store
//! opens, as long as it belongs to this user. A symlink is never followed. Windows has no such modes; there the
//! folder is created as it always was.

use std::fs;
use std::io;
use std::path::Path;

/// The files SQLite keeps beside a database, by the suffix it appends to the database's name.
#[cfg(unix)]
const SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Creates the store's folder and database file where absent, private on Unix, and tightens existing ones.
pub(super) fn prepare(path: &Path) -> io::Result<()> {
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if let Some(folder) = folder {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(folder)?;
            keep_private(folder)?;
        }
        // The database is created here only when absent; an existing one is never opened by this code, because
        // closing any descriptor of it would drop the POSIX locks this process's other connections hold on it.
        if fs::symlink_metadata(path).is_err() {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        keep_private(path)?;
        for suffix in SIDECAR_SUFFIXES {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(suffix);
            keep_private(Path::new(&sidecar))?;
        }
    }
    #[cfg(not(unix))]
    {
        if let Some(folder) = folder {
            fs::create_dir_all(folder)?;
        }
    }
    Ok(())
}

/// Takes group and other access away from a folder (to `0700`) or regular file (to `0600`) this user owns. The path
/// is judged with `lstat`, so a symlink is left alone, and changed by path, never through a descriptor of its own:
/// closing any descriptor of a database file drops every POSIX lock this process holds on it, SQLite's included. A
/// missing path, a symlink, another user's, or anything else is left alone.
#[cfg(unix)]
fn keep_private(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    // SAFETY: `geteuid` reads the process's effective user id; it cannot fail and touches no memory.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 == 0 {
        return Ok(());
    }
    let mode = if metadata.is_dir() {
        0o700
    } else if metadata.is_file() {
        0o600
    } else {
        return Ok(());
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}
