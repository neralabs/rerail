//! The rules every file the engine maps keeps (decision 0574; Unix: Windows has no such modes, and keeps none of these):
//!
//! - **Its folder** is the user's, with no group or world bit of write or search (`0700` at most). A folder the
//!   engine makes is made so; one it is given that is not is refused, never repaired.
//! - **Its file** is the user's, with no group or world write. A file mapped for writing (an open block, decision
//!   0401; a deferrable slab, 0295) is made `0600`. A frozen file is read-only: frozen, it is marked `0400`, and a
//!   frozen file mapped with its owner's write bit still on is marked so as it is opened (the bit is the user's own,
//!   on the user's own file). A group or world write bit is refused, never repaired.
//!
//! A refusal is an error of kind `PermissionDenied` that names the path, its owner and its mode.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Group and world write.
#[cfg(unix)]
const SHARED_WRITE: u32 = 0o022;
/// Group and world write and search: what a mapped file's folder may not have.
#[cfg(unix)]
const SHARED_FOLDER: u32 = 0o033;

#[cfg(unix)]
fn refused(path: &Path, what: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {what} (a mapped file and its folder are the user's alone: decision 0574)", path.display()))
}

#[cfg(unix)]
fn euid() -> u32 {
    // SAFETY: `geteuid` reads the process's effective user id; it cannot fail and touches no memory.
    unsafe { libc::geteuid() }
}

/// Refuses a folder a mapped file may not be in: another user's, or with group or world write or search.
pub fn check_folder(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
        let m = std::fs::metadata(dir)?;
        if !m.is_dir() {
            return Err(refused(dir, "not a folder".into()));
        }
        if m.uid() != euid() {
            return Err(refused(dir, format!("a folder of user {}, not this user's ({})", m.uid(), euid())));
        }
        if m.mode() & SHARED_FOLDER != 0 {
            return Err(refused(dir, format!("a folder of mode {:o}: group or world may write or search it (chmod 700)", m.mode() & 0o7777)));
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// The folder `path` is in, checked (`check_folder`).
pub fn check_folder_of(path: &Path) -> io::Result<()> {
    check_folder(path.parent().unwrap_or(Path::new(".")))
}

/// Makes `dir` and the folders above it that are missing, each `0700`; then checks `dir` (`check_folder`).
pub fn make_folder(dir: &Path) -> io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)?;
    check_folder(dir)
}

/// Checks the open file `f` at `path` and its folder: the user's, no group or world write; `frozen`: read-only, its
/// owner's write bit taken away where it is on.
pub fn check_file(f: &File, path: &Path, frozen: bool) -> io::Result<()> {
    check_folder_of(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let m = f.metadata()?;
        if !m.is_file() {
            return Err(refused(path, "not a regular file".into()));
        }
        if m.uid() != euid() {
            return Err(refused(path, format!("a file of user {}, not this user's ({})", m.uid(), euid())));
        }
        let mode = m.mode() & 0o7777;
        if mode & SHARED_WRITE != 0 {
            return Err(refused(path, format!("a file of mode {mode:o}: group or world may write it")));
        }
        if frozen && mode & 0o200 != 0 {
            f.set_permissions(std::fs::Permissions::from_mode(mode & !0o222))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (f, path, frozen);
    Ok(())
}

/// Marks the file `f` frozen: read-only (`0400`).
pub fn mark_frozen(f: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o400))?;
    }
    #[cfg(not(unix))]
    let _ = f;
    Ok(())
}

/// Marks the file at `path` frozen (`mark_frozen`).
pub fn mark_frozen_at(path: &Path) -> io::Result<()> {
    mark_frozen(&File::open(path)?)
}

/// Opens `path` for reading only, checked (`check_file`); `frozen`: it is marked read-only.
pub fn open_read(path: &Path, frozen: bool) -> io::Result<File> {
    check_folder_of(path)?;
    let f = File::open(path)?;
    check_file(&f, path, frozen)?;
    Ok(f)
}

/// Opens `path` for reading and writing, made `0600` where it is new (`create`), its length cut to 0 with
/// `truncate`, checked (`check_file`). A file of the user's marked frozen (`0400`) is the writer's to write again: its
/// owner's write bit is put back first.
pub fn open_write(path: &Path, create: bool, truncate: bool) -> io::Result<File> {
    check_folder_of(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if let Ok(m) = std::fs::symlink_metadata(path)
            && m.is_file()
            && m.uid() == euid()
            && m.mode() & 0o200 == 0
            && m.mode() & SHARED_WRITE == 0
        {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let mut o = OpenOptions::new();
    o.read(true).write(true).create(create).truncate(truncate);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let f = o.open(path)?;
    check_file(&f, path, false)?;
    Ok(f)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn folder(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rerail-mapped-{name}-{}", std::process::id()));
        make_folder(&d).unwrap();
        d
    }
    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    /// A mapped file's folder is the user's alone: one with group or world write or search is refused, one made is
    /// `0700`.
    #[test]
    fn a_shared_folder_is_refused() {
        let d = folder("folder");
        assert_eq!(mode(&d), 0o700);
        for bad in [0o755, 0o750, 0o711, 0o770, 0o701, 0o702] {
            std::fs::set_permissions(&d, std::fs::Permissions::from_mode(bad)).unwrap();
            let e = check_folder(&d).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "mode {bad:o}");
            assert!(open_write(&d.join("f"), true, true).is_err(), "no file is made in a folder of mode {bad:o}");
        }
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700)).unwrap();
        check_folder(&d).unwrap();
    }

    /// A file mapped for writing is made `0600`; frozen, `0400`; a frozen file left owner-writable is marked
    /// read-only as it is opened; group or world write is refused, for reading and for writing.
    #[test]
    fn mapped_files_are_the_users_and_frozen_ones_read_only() {
        let d = folder("files");
        let p = d.join("block");
        let f = open_write(&p, true, true).unwrap();
        assert_eq!(mode(&p), 0o600);
        mark_frozen(&f).unwrap();
        drop(f);
        assert_eq!(mode(&p), 0o400);
        open_read(&p, true).unwrap();
        // The writer writes it again: its own write bit put back.
        drop(open_write(&p, false, false).unwrap());
        assert_eq!(mode(&p), 0o600);
        // Opened frozen with the owner's write bit on: marked read-only.
        drop(open_read(&p, true).unwrap());
        assert_eq!(mode(&p), 0o400);
        for bad in [0o620, 0o602, 0o622, 0o664] {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(bad)).unwrap();
            assert_eq!(open_read(&p, true).unwrap_err().kind(), io::ErrorKind::PermissionDenied, "mode {bad:o}");
            assert_eq!(open_read(&p, false).unwrap_err().kind(), io::ErrorKind::PermissionDenied, "mode {bad:o}");
            assert_eq!(open_write(&p, false, false).unwrap_err().kind(), io::ErrorKind::PermissionDenied, "mode {bad:o}");
        }
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::remove_file(&p).unwrap();
        std::fs::remove_dir(&d).unwrap();
    }
}
