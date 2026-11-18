//! A directory's entries with their stats, read as a listing (decision 0399): on macOS by `getattrlistbulk`, which
//! returns many entries' attributes a call; elsewhere on Unix by `readdir` and `fstatat` relative to the open
//! directory. The observer checks a small root's kept answers against these, with no `stat` of a path.

use super::{Kind, Stat};
use std::ffi::OsString;
use std::path::Path;

/// What a listing gives of one entry: its kind and stat, or `None` where the entry is to be `stat`ed on its own (a
/// symbolic link, whose answer is of what it names; an entry whose attributes the file system did not give).
pub(super) type Listed = Option<(Kind, Stat)>;

/// The entries of `dir`, by name (`.` and `..` left out). An error where `dir` cannot be opened as a directory.
#[cfg(target_os = "macos")]
pub(super) fn entries(dir: &Path) -> std::io::Result<Vec<(OsString, Listed)>> {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())?;
    // SAFETY: a NUL-terminated path; the descriptor is closed by `Fd`.
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    struct Fd(i32);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: a descriptor this function opened, closed once.
            unsafe { libc::close(self.0) };
        }
    }
    let fd = Fd(fd);
    // SAFETY: a plain C struct; every field set below or zero.
    let mut asked: libc::attrlist = unsafe { std::mem::zeroed() };
    asked.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    asked.commonattr = libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_NAME | libc::ATTR_CMN_DEVID | libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_MODTIME | libc::ATTR_CMN_CHGTIME | libc::ATTR_CMN_FILEID;
    asked.fileattr = libc::ATTR_FILE_DATALENGTH;
    // The attributes each entry carries, all of them for a stat: one missing and the entry is `stat`ed alone.
    const STAT: u32 = libc::ATTR_CMN_DEVID | libc::ATTR_CMN_OBJTYPE | libc::ATTR_CMN_MODTIME | libc::ATTR_CMN_CHGTIME | libc::ATTR_CMN_FILEID;
    let mut buf = vec![0u8; 64 * 1024];
    let mut out = Vec::new();
    loop {
        // SAFETY: `asked` and `buf` live across the call, `buf.len()` bytes writable.
        let n = unsafe { libc::getattrlistbulk(fd.0, (&raw mut asked).cast(), buf.as_mut_ptr().cast(), buf.len(), u64::from(libc::FSOPT_PACK_INVAL_ATTRS)) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n == 0 {
            break;
        }
        let mut at = 0usize;
        for _ in 0..n {
            let Some(len) = u32_at(&buf, at).map(|l| l as usize).filter(|&l| l >= 4 && at + l <= buf.len()) else {
                return Err(std::io::Error::other("a bulk listing's entry runs past its buffer"));
            };
            let e = &buf[at..at + len];
            at += len;
            // The layout, with every attribute packed (`FSOPT_PACK_INVAL_ATTRS`), each on a 4-byte boundary and in
            // the order of its bit, the returned set first: length, returned set (5 × u32), name (offset, length),
            // dev (i32), type (u32), mtime and ctime (timespecs), file id (u64), then the file's data length (i64).
            let (Some(common), Some(file)) = (u32_at(e, 4), u32_at(e, 16)) else { continue };
            let (Some(name_at), Some(name_len)) = (i32_at(e, 24), u32_at(e, 28)) else { continue };
            let start = 24usize.checked_add_signed(name_at as isize).unwrap_or(usize::MAX);
            let Some(name) = e.get(start..start.saturating_add(name_len as usize)) else { continue };
            let name = name.strip_suffix(&[0]).unwrap_or(name);
            if name == b"." || name == b".." {
                continue;
            }
            let name = OsString::from_vec(name.to_vec());
            let field = (|| {
                let dev = i32_at(e, 32)?;
                let kind = u32_at(e, 36)?;
                let (ms, mn, cs, cn) = (i64_at(e, 40)?, i64_at(e, 48)?, i64_at(e, 56)?, i64_at(e, 64)?);
                let ino = i64_at(e, 72)?;
                let size = i64_at(e, 80)?;
                Some((dev, kind, ms, mn, cs, cn, ino, size))
            })();
            let listed = field.and_then(|(dev, kind, ms, mn, cs, cn, ino, size)| {
                if common & STAT != STAT {
                    return None;
                }
                // `vtype`: VREG 1, VDIR 2, VLNK 5.
                let kind = match kind {
                    1 if file & libc::ATTR_FILE_DATALENGTH != 0 => Kind::File,
                    1 | 5 => return None,
                    2 => Kind::Dir,
                    _ => Kind::Other,
                };
                let ns = |s: i64, n: i64| s.saturating_mul(1_000_000_000).saturating_add(n);
                Some((kind, Stat { dev: i64::from(dev), ino, size: if kind == Kind::File { size } else { 0 }, mtime_ns: ns(ms, mn), ctime_ns: ns(cs, cn) }))
            });
            out.push((name, listed));
        }
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(target_os = "macos")]
fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(target_os = "macos")]
fn i64_at(b: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_ne_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The entries of `dir`, by name: `readdir`, then each entry's `fstatat` relative to the open directory (no path
/// walked again), a symbolic link left to be `stat`ed on its own.
#[cfg(all(unix, not(target_os = "macos")))]
pub(super) fn entries(dir: &Path) -> std::io::Result<Vec<(OsString, Listed)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let kind = e.file_type()?;
        if kind.is_symlink() {
            out.push((e.file_name(), None));
            continue;
        }
        let listed = e.metadata().ok().map(|m| {
            let kind = if m.is_file() {
                Kind::File
            } else if m.is_dir() {
                Kind::Dir
            } else {
                Kind::Other
            };
            let stat = Stat::of(&m);
            (kind, if kind == Kind::File { stat } else { Stat { size: 0, ..stat } })
        });
        out.push((e.file_name(), listed));
    }
    Ok(out)
}

/// No listing with stats here: a root is not checked by its listings (`supported`).
#[cfg(not(unix))]
pub(super) fn entries(_dir: &Path) -> std::io::Result<Vec<(OsString, Listed)>> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// Whether a root may be checked by its listings on this system.
pub(super) const SUPPORTED: bool = cfg!(unix);
