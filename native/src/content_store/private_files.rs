//! Unix permissions for content stores. SQLite uses the database mode for its journals.
//! The caller owns an existing parent directory. Rerail owns its database and the directories it allocates for cache data.

use std::fs;
use std::io;
use std::path::Path;

/// The files SQLite keeps beside a database, by the suffix it appends to the database's name.
#[cfg(unix)]
const JOURNAL_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

fn create_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// Creates a directory reserved for the cache and repairs its Unix mode on reopening.
pub fn prepare_cache_directory(path: &Path) -> io::Result<()> {
    create_directory(path)?;
    #[cfg(unix)]
    keep_private(path, true)?;
    Ok(())
}

/// Prepares a database at a caller-supplied path. Only new parent directories get a mode here.
pub(super) fn prepare(path: &Path) -> io::Result<()> {
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        create_directory(folder)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
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
        keep_private(path, false)?;
        for suffix in JOURNAL_SUFFIXES {
            let mut journal = path.as_os_str().to_owned();
            journal.push(suffix);
            keep_private(Path::new(&journal), false)?;
        }
    }
    Ok(())
}

/// Takes group and other access away from a folder (to `0700`) or regular file (to `0600`) this user owns. The path
/// is judged with `lstat`, so a symlink is left alone, and changed by path, never through a descriptor of its own:
/// closing any descriptor of a database file drops every POSIX lock this process holds on it, SQLite's included. A
/// missing path, a symlink, another user's, or anything else is left alone.
#[cfg(unix)]
fn keep_private(path: &Path, directory: bool) -> io::Result<()> {
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
    let mode = if directory && metadata.is_dir() {
        0o700
    } else if !directory && metadata.is_file() {
        0o600
    } else {
        return Ok(());
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}
