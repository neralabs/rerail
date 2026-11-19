//! Whether Git ignores a worktree path, as `git check-ignore` answers it: every `.gitignore` from
//! the worktree root down to the path, `.git/info/exclude` and `core.excludesFile` (or the XDG
//! default), with negation and directory patterns; a path the index tracks is never ignored, nor
//! is a folder that holds a tracked path. Hosts that follow `.gitignore` skip what this ignores,
//! and can name the pattern that ignored a path, with its file and line (`git check-ignore -v`).
//!
//! The index is re-read whenever its file changes on disk, and the rules with it, so `git add` of an
//! ignored file brings it back and a `.gitignore` the index marks skip-worktree is read from the
//! blob it names now; a `.gitignore` is read when the walk first enters its folder. A matcher says
//! whether one opened now would read the rules it read (`sources_unchanged`), so hosts can share
//! one per checkout: every holder reads the index again once its file changes, which a stat of it
//! tells or, where the volume's stamps cannot, the checksum Git writes at its end. A matcher holds
//! no file open.
//!
//! The repository opens through the engine's one opener (`git_access::opener`), so a repository
//! Git refuses has no matcher. Opening runs on a thread of its own under the Git read time limit; questions
//! are answered on the caller's thread, since a walk asks one or two per file. A question's own
//! disk reads are a stat of the index and the `.gitignore` of a folder the walk has just listed,
//! so a disk that stops answering holds the walk before it holds the matcher. The check of a
//! matcher's sources opens the repository again as an open does: through the same opener, on a
//! thread of its own under the same limit, held back with the opens of its root.

use crate::git_access::opener::{self, OpenError};
use crate::git_access::pool::{self, GIT_READ_TIME_LIMIT};
use gix::bstr::{BStr, ByteSlice};
use gix::index::entry::Mode;
use napi::{Error, Result, Status};
use napi_derive::napi;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct GitIgnore {
    repository: gix::Repository,
    stack: gix::worktree::Stack,
    /// Where the opened root sits below the worktree root: empty, or `/`-joined components.
    prefix: Vec<String>,
    /// The opened root, resolved.
    root: PathBuf,
    /// What the rules were read from as the matcher opened; `None` when that could not be read.
    sources: Option<Sources>,
    /// The index the questions read, read again once its file changes.
    index: IndexFile,
}

impl GitIgnore {
    /// The exclude stack of the Git worktree around `root`; `None` when `root` is in no Git
    /// worktree (a plain folder or a bare repository), where nothing is ignored.
    pub fn open(root: &Path) -> std::result::Result<Option<Self>, String> {
        let root = dunce::canonicalize(root)
            .map_err(|error| format!("cannot resolve {}: {error}", root.display()))?;
        let Some((repository, worktree)) = discover_worktree(&root)? else {
            return Ok(None);
        };
        let Ok(below) = root.strip_prefix(&worktree) else {
            return Ok(None);
        };
        let prefix = below
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect();
        // Read before the stack reads the files, so a change made meanwhile reads as one later.
        let sources = Sources::read(&repository, &worktree);
        let (index, stack) = index_and_exclude_stack(&repository)?;
        Ok(Some(Self {
            repository,
            stack,
            prefix,
            root,
            sources,
            index,
        }))
    }

    /// Whether a matcher opened at this root now would read the rules this one read: the
    /// configuration, read again through gix as an open reads it (every file Git reads, includes
    /// and their conditions among them), chooses the same excludes file and pattern case, and the
    /// root `.gitignore`, `info/exclude` and that excludes file are as this matcher read them.
    /// When it would, the stack leaves every folder it entered, so the next path below a folder
    /// reads that folder's `.gitignore` again, as a matcher opened now would.
    pub fn sources_unchanged(&mut self) -> bool {
        let now = Self::sources_now(&self.root);
        self.take_sources(now)
    }

    /// The sources an open at `root` now reads the rules from, finding the repository through the
    /// engine's opener; `None` where that open fails or finds no worktree.
    fn sources_now(root: &Path) -> Option<Sources> {
        let (repository, worktree) = discover_worktree(root).ok()??;
        Sources::read(&repository, &worktree)
    }

    /// Whether `now`, from [`Self::sources_now`], holds the sources this matcher read; when it
    /// does, the stack leaves its folders as `sources_unchanged` says.
    fn take_sources(&mut self, now: Option<Sources>) -> bool {
        if now.is_none() || now != self.sources {
            return false;
        }
        self.leave_folders();
        true
    }

    /// Reads the index again once its file differs from the one it was read from, and makes
    /// the stack again from it, as an open makes it: the stack names each `.gitignore` the index
    /// marks skip-worktree by the blob the index holds. The sources are read again first, as an
    /// open reads them, so a check of them compares the files the new stack reads.
    fn refresh(&mut self) -> std::result::Result<(), String> {
        if self.index.unchanged() {
            return Ok(());
        }
        let sources = self
            .sources
            .as_ref()
            .and_then(|read| Sources::read(&self.repository, &read.worktree));
        let index = IndexFile::read(&self.repository, self.index.volume)?;
        // Rules that cannot be read again leave the stack and sources read before, so questions
        // are answered by them and the new index is read once.
        if let Ok(stack) = exclude_stack(&self.repository, &index.file) {
            (self.sources, self.stack) = (sources, stack);
        }
        self.index = index;
        Ok(())
    }

    /// Asked about `.git`, a path at the worktree root that Git never tracks, the stack leaves
    /// every folder below the root.
    fn leave_folders(&mut self) {
        // A file that cannot be read on the way is reported by the question that reads it.
        let _ = self
            .stack
            .at_path(Path::new(".git"), Some(Mode::DIR), &self.repository.objects);
    }

    /// Whether Git ignores `relative` (to the opened root, `/`-separated). The root itself and any
    /// path outside the worktree are never ignored.
    pub fn ignored(
        &mut self,
        relative: &str,
        is_directory: bool,
    ) -> std::result::Result<bool, String> {
        Ok(self.ignored_by(relative, is_directory)?.is_some())
    }

    /// The pattern that makes Git ignore `relative` (as for `ignored`), with the file it was read
    /// from and its line there; `None` when Git does not ignore it. A path under an ignored folder
    /// names the folder's pattern, as `git check-ignore -v` does.
    pub fn ignored_by(
        &mut self,
        relative: &str,
        is_directory: bool,
    ) -> std::result::Result<Option<GitIgnoreMatch>, String> {
        let Some(path) = self.worktree_path(relative) else {
            return Ok(None);
        };
        if path.is_empty() {
            return Ok(None);
        }
        self.refresh()?;
        if tracked(&self.index.file, path.as_bytes().as_bstr(), is_directory) {
            return Ok(None);
        }
        let platform = self
            .stack
            .at_path(
                Path::new(&path),
                Some(if is_directory { Mode::DIR } else { Mode::FILE }),
                &self.repository.objects,
            )
            .map_err(|error| format!("cannot read Git excludes for {path}: {error}"))?;
        Ok(platform
            .matching_exclude_pattern()
            .filter(|matched| !matched.pattern.is_negative())
            .map(|matched| GitIgnoreMatch {
                source: matched
                    .source
                    .map(|source| source.to_string_lossy().into_owned()),
                line: u32::try_from(matched.sequence_number).unwrap_or(u32::MAX),
                pattern: git_pattern_text(matched.pattern),
            }))
    }

    /// `relative` below the worktree root, lexically normalised; `None` when it leaves the worktree.
    fn worktree_path(&self, relative: &str) -> Option<String> {
        let mut components = self.prefix.clone();
        for component in relative.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    components.pop()?;
                }
                component => components.push(component.to_owned()),
            }
        }
        Some(components.join("/"))
    }
}

/// The repository around `root`, opened through the engine's opener, and its worktree, resolved;
/// `None` outside a Git worktree.
fn discover_worktree(
    root: &Path,
) -> std::result::Result<Option<(gix::Repository, PathBuf)>, String> {
    let repository = match opener::discover(root) {
        Ok(repository) => repository,
        Err(OpenError::NoRepository(_)) => return Ok(None),
        Err(OpenError::Failed(message)) => return Err(message),
    };
    let Some(worktree) = repository.workdir() else {
        return Ok(None);
    };
    let worktree = dunce::canonicalize(worktree)
        .map_err(|error| format!("cannot resolve Git worktree: {error}"))?;
    Ok(Some((repository, worktree)))
}

/// What a matcher reads Git's ignore rules from, besides the `.gitignore` of each folder below
/// the root, which its stack reads as it enters the folder: the configuration's choice of
/// excludes file, pattern case and precious patterns, read through gix as `Repository::excludes`
/// reads it, and the stamp of each file read whole: the root `.gitignore` and, as
/// `gix_ignore::Search::from_git_dir` reads them, the excludes file and `info/exclude` in the
/// common Git folder.
#[derive(Debug, PartialEq, Eq)]
struct Sources {
    worktree: PathBuf,
    common: PathBuf,
    excludes_file: Option<PathBuf>,
    ignore_case: bool,
    precious: bool,
    stamps: Vec<Stamp>,
}

impl Sources {
    /// `repository`'s sources as they are now; `None` when its configuration names an excludes
    /// file that cannot be resolved, which the stack's own read reports.
    fn read(repository: &gix::Repository, worktree: &Path) -> Option<Self> {
        let config = repository.config_snapshot();
        let excludes_file = config
            .trusted_path("core.excludesFile")
            .ok()?
            .or_else(default_excludes_file);
        let common = repository.common_dir().to_path_buf();
        let files = [
            worktree.join(".gitignore"),
            common.join("info").join("exclude"),
        ];
        let stamps = files
            .iter()
            .chain(&excludes_file)
            .map(|file| Stamp::of(file))
            .collect();
        Some(Self {
            worktree: worktree.to_path_buf(),
            common,
            excludes_file,
            ignore_case: config.boolean("core.ignoreCase").unwrap_or(false),
            precious: config.boolean("gitoxide.parsePrecious").unwrap_or(false),
            stamps,
        })
    }
}

/// Git's excludes file where `core.excludesFile` names none: `git/ignore` in `XDG_CONFIG_HOME`,
/// else in `.config` in the home folder, as gix's `xdg_config_path` finds it.
fn default_excludes_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".config")))?;
    Some(base.join("git").join("ignore"))
}

/// `HOME`, else the platform's home folder, as `gix_path::env::home_dir` finds it.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(std::env::home_dir)
}

/// A file as a matcher read it: absent, unreadable, or its bytes by length and digest. The bytes
/// are what the rules come from, so a rewrite at the same size within one tick of the file
/// system's clock, or with its write time put back, changes the stamp, and a touch does not.
#[derive(Debug, PartialEq, Eq)]
enum Stamp {
    Absent,
    Unreadable,
    Read { len: usize, digest: u64 },
}

impl Stamp {
    fn of(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => Self::Read {
                len: bytes.len(),
                digest: digest(&bytes),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Absent,
            Err(_) => Self::Unreadable,
        }
    }
}

/// A digest of `bytes`, compared only within one process.
fn digest(bytes: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::hash::DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// `pattern` as `git check-ignore -v` prints it: as written, but for gix dropping the `\` of a
/// leading `\#` or `\!`, which Git keeps.
fn git_pattern_text(pattern: &gix::glob::Pattern) -> String {
    let text = pattern.to_string();
    let escaped = !pattern.is_negative()
        && !pattern.mode.contains(gix::glob::pattern::Mode::ABSOLUTE)
        && matches!(pattern.text.first(), Some(b'#' | b'!'));
    if escaped { format!("\\{text}") } else { text }
}

/// Git applies ignore rules to untracked paths only: a tracked file, a submodule, and a folder
/// holding either stay visible whatever the patterns say.
fn tracked(index: &gix::index::State, path: &BStr, is_directory: bool) -> bool {
    if index.entry_by_path(path).is_some() {
        return true;
    }
    if !is_directory {
        return false;
    }
    let mut folder = path.to_owned();
    folder.push(b'/');
    index
        .prefixed_entries(folder.as_bstr())
        .is_some_and(|entries| !entries.is_empty())
}

/// `repository`'s index as it is now, and the exclude stack read with it: Git reads the
/// `.gitignore` of a folder the worktree lacks from that index.
fn index_and_exclude_stack(
    repository: &gix::Repository,
) -> std::result::Result<(IndexFile, gix::worktree::Stack), String> {
    let index = IndexFile::read(repository, Volume::of(&repository.index_path()))?;
    let stack = exclude_stack(repository, &index.file)?;
    Ok((index, stack))
}

/// `repository`'s exclude stack, reading a `.gitignore` the worktree lacks from `index`.
fn exclude_stack(
    repository: &gix::Repository,
    index: &gix::index::File,
) -> std::result::Result<gix::worktree::Stack, String> {
    Ok(repository
        .excludes(
            index,
            None,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        )
        .map_err(|error| format!("cannot read Git excludes: {error}"))?
        .detach())
}

/// The index as a matcher last read it, with its file's stamp then. gix's shared index is read
/// again only once the file's modification time is later, but Git writes the index to a new file
/// and renames it over the old one, and a second write within one step of the file system's clock
/// keeps the first one's time: the stamp tells that new file from the one read where it can, and
/// the checksum Git writes at the end of the index tells it where the stamp cannot.
struct IndexFile {
    path: PathBuf,
    /// What the index's volume lets a stamp tell, found once per reader.
    volume: Volume,
    /// The file's stamp when it was read (`None` inside where there was no file), or `None` where
    /// that stamp could not tell a later write from it: then the next question compares the
    /// checksum at the file's end with the one read, and reads the index again when it differs.
    stamp: Option<Option<FileStamp>>,
    file: gix::index::File,
}

impl IndexFile {
    /// `repository`'s index as it is now; an empty one where it has none.
    fn read(repository: &gix::Repository, volume: Volume) -> std::result::Result<Self, String> {
        let path = repository.index_path();
        // Stamped before it is read, so a write that lands meanwhile reads as a change at the
        // next question. A stamp that cannot tell a later write is not kept, by the clock read
        // with it, before the index: every write the read can miss comes after that reading.
        let stamp = volume.keeps(FileStamp::of(&path));
        #[cfg(test)]
        INDEX_READS.with(|reads| reads.set(reads.get() + 1));
        let file = match repository.open_index() {
            Ok(file) => file,
            Err(_) if !path.exists() => gix::index::File::from_state(
                gix::index::State::new(repository.object_hash()),
                path.clone(),
            ),
            Err(error) => return Err(format!("cannot read Git index: {error}")),
        };
        #[cfg(test)]
        tests::after_index_read();
        Ok(Self {
            stamp,
            path,
            volume,
            file,
        })
    }

    /// Whether the file still holds the index read: its stamp equals the one kept, or it ends with
    /// the checksum read, which Git computes over every byte before it. A stamp that tells its
    /// file is kept from then on, so a question costs a stat again.
    fn unchanged(&mut self) -> bool {
        let now = FileStamp::of(&self.path);
        if now.is_some() && self.stamp == now {
            return true;
        }
        // Whether the stamp tells a later write is decided by the clock before the checksum is
        // read, as a read decides it before the index is read.
        let kept = self.volume.keeps(now);
        if !self.same_checksum() {
            return false;
        }
        self.stamp = kept;
        true
    }

    /// Whether the file ends with the checksum of the index read; never where Git wrote none
    /// (`index.skipHash`), where the index is read again.
    fn same_checksum(&self) -> bool {
        self.file.checksum().is_some_and(|read| {
            let read = read.as_bytes();
            trailing_bytes(&self.path, read.len()).is_some_and(|now| now == read)
        })
    }
}

/// The last `len` bytes of `path`; `None` where it cannot be read or is shorter.
fn trailing_bytes(path: &Path, len: usize) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::End(-i64::try_from(len).ok()?)).ok()?;
    let mut bytes = vec![0; len];
    file.read_exact(&mut bytes).ok()?;
    #[cfg(test)]
    tests::after_index_read();
    Some(bytes)
}

#[cfg(test)]
thread_local! {
    /// The index reads made on this thread, for the cases that check a question reads none.
    static INDEX_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What a stamp of a file on one volume can tell, by the volume's file system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Volume {
    /// Every new file gets an identity no other file had (NTFS, APFS, HFS+), so a stamp tells its
    /// file from every later one at once.
    Unique,
    /// An identity can come back to a later file (ext4 gives a freed inode number to the next new
    /// file, FAT and exFAT make it from a file's place in its folder, ReFS's 64-bit index may
    /// repeat), and file times come from this machine's clock: a stamp tells its file once that
    /// clock has left the steps of its times ([`FileStamp::tells`]).
    Local,
    /// File times come from another machine's clock (NFS, SMB, 9p, FUSE and a virtual machine's
    /// shared folder among them), which this one cannot read: no stamp is kept, so each question
    /// compares the index's checksum.
    Remote,
}

impl Volume {
    /// `stamp`, from [`FileStamp::of`], where it tells its file from every later one on this
    /// volume; `None` where it cannot: a stat refused, a time the file clock has not left, or a
    /// volume whose times come from another clock.
    fn keeps(self, stamp: Option<Option<FileStamp>>) -> Option<Option<FileStamp>> {
        match self {
            Self::Unique => stamp,
            Self::Local => {
                let now = file_clock();
                stamp.filter(|stamp| stamp.as_ref().is_none_or(|file| file.tells(now)))
            }
            Self::Remote => None,
        }
    }

    /// The volume of the folder `index` is in, by its file system type (`statfs`): file systems
    /// served by another machine, or through FUSE, which may be, are remote, and every other one
    /// local. One that cannot be read is taken as remote.
    #[cfg(target_os = "linux")]
    fn of(index: &Path) -> Self {
        use std::os::unix::ffi::OsStrExt;
        #[cfg(test)]
        if let Some(volume) = tests::volume() {
            return volume;
        }
        let folder = index.parent().unwrap_or(index);
        let Ok(folder) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
            return Self::Remote;
        };
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: a NUL-terminated path and a buffer of the type the call fills.
        if unsafe { libc::statfs(folder.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return Self::Remote;
        }
        // SAFETY: the call succeeded, so it filled the buffer. A type is a 32-bit magic number.
        let kind = unsafe { stat.assume_init() }.f_type as u32;
        if REMOTE_FILE_SYSTEMS.contains(&kind) {
            Self::Remote
        } else {
            Self::Local
        }
    }

    /// The volume of the folder `index` is in: one the system does not keep locally
    /// (`MNT_LOCAL`), or that a FUSE driver serves, is remote; APFS and HFS+ give every new file
    /// an identity of its own (APFS from the volume's next object identifier, HFS+ a number again
    /// only once its 32-bit count has wrapped); every other one is local. One that cannot be read
    /// is taken as remote.
    #[cfg(target_os = "macos")]
    fn of(index: &Path) -> Self {
        use std::os::unix::ffi::OsStrExt;
        #[cfg(test)]
        if let Some(volume) = tests::volume() {
            return volume;
        }
        let folder = index.parent().unwrap_or(index);
        let Ok(folder) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
            return Self::Remote;
        };
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: a NUL-terminated path and a buffer of the type the call fills.
        if unsafe { libc::statfs(folder.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return Self::Remote;
        }
        // SAFETY: the call succeeded, so it filled the buffer.
        let stat = unsafe { stat.assume_init() };
        // SAFETY: the system ends the name with a NUL inside its 16 bytes.
        let name = unsafe { std::ffi::CStr::from_ptr(stat.f_fstypename.as_ptr()) };
        Self::of_mac(
            (stat.f_flags & libc::MNT_LOCAL as u32) != 0,
            name.to_bytes(),
        )
    }

    /// The volume of a file system named `name`, kept locally or not (`local`).
    #[cfg(target_os = "macos")]
    fn of_mac(local: bool, name: &[u8]) -> Self {
        match name {
            _ if !local || name.ends_with(b"fuse") => Self::Remote,
            b"apfs" | b"hfs" => Self::Unique,
            _ => Self::Local,
        }
    }

    /// The volume of the folder `index` is in: a network drive or share is remote, NTFS gives
    /// every new file a file index of its own (the record number with a sequence number that grows
    /// each time the record is used again), and every other file system (ReFS, FAT, exFAT) is
    /// local. One that cannot be read is taken as remote.
    #[cfg(windows)]
    fn of(index: &Path) -> Self {
        #[cfg(test)]
        if let Some(volume) = tests::volume() {
            return volume;
        }
        match volume_file_system(index.parent().unwrap_or(index)) {
            None => Self::Remote,
            Some(name) if name == "NTFS" => Self::Unique,
            Some(_) => Self::Local,
        }
    }

    /// The volume of any folder on a system that is neither Linux, macOS nor Windows: local.
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    fn of(_index: &Path) -> Self {
        Self::Local
    }
}

/// The name of the file system of the volume `folder` is on (`NTFS`, `ReFS`, `FAT32`, `exFAT`);
/// `None` on a network drive or share, or where it cannot be read.
#[cfg(windows)]
fn volume_file_system(folder: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
    };
    /// `GetDriveTypeW`'s answer for a network drive.
    const DRIVE_REMOTE: u32 = 4;
    let folder: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    let mut root = [0_u16; 1024];
    // SAFETY: a NUL-terminated path and a buffer of the length given, which the call fills.
    if unsafe { GetVolumePathNameW(folder.as_ptr(), root.as_mut_ptr(), 1024) } == 0 {
        return None;
    }
    // SAFETY: the NUL-terminated root the call above wrote.
    if unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE {
        return None;
    }
    let mut name = [0_u16; 64];
    // SAFETY: the root, and a buffer of the length given for the name; nothing else is asked for.
    let read = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            64,
        )
    };
    let end = name
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(name.len());
    (read != 0).then(|| String::from_utf16_lossy(&name[..end]))
}

/// The `statfs` types of file systems whose file times another machine sets, or may.
#[cfg(target_os = "linux")]
const REMOTE_FILE_SYSTEMS: [u32; 15] = [
    0x6969,      // NFS
    0x517B,      // SMB
    0xFF53_4D42, // CIFS
    0xFE53_4D42, // SMB2
    0x0102_1997, // 9p: WSL's drives, QEMU's shared folders
    0x6573_5546, // FUSE: sshfs, virtiofs, VMware's shared folders
    0x00C3_6400, // Ceph
    0x5346_414F, // AFS
    0x6B41_4653, // kAFS
    0x7375_7245, // Coda
    0x564C,      // NCP
    0x0116_1970, // GFS2
    0x7461_636F, // OCFS2
    0x0BD0_0BD0, // Lustre
    0x786F_4256, // VirtualBox's shared folders
];

/// What tells one index file from another and from itself rewritten: the file's identity (device
/// and inode on Unix; volume serial and file index on Windows), which every write of Git's
/// changes, and its modification time and size, which a write in place changes. All three come
/// from the one query a stat makes, so a question costs what it cost before. Git writes the index
/// to a new file and renames it over the old one; where a volume gives an identity again (ext4
/// gives a freed inode number to the next new file at once), two writes within one step of the
/// file system's clock can land the second on the identity of the file read, at its size and
/// time. That needs the read taken within the step, so there such a stamp is not kept
/// ([`Volume::keeps`]) and nothing is held open. A tool can also set a file's time back to the one
/// read; on Unix the stamp holds the time the file last changed, which the kernel sets at every
/// write, rename and time set and no call sets, so such a file still differs once the clock has
/// left that time's step too.
#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    identity: (u64, u64),
    modified: FileTime,
    /// When the file last changed in any way, as a stat gives it (seconds, nanoseconds).
    #[cfg(unix)]
    changed: (i64, i64),
    len: u64,
}

/// A file's modification time as its stat gives it.
#[cfg(unix)]
type FileTime = Option<std::time::SystemTime>;
/// A file's modification time as its stat gives it: the `FILETIME` count of 100 ns.
#[cfg(windows)]
type FileTime = u64;

/// The clock Linux gives file times from (`CLOCK_REALTIME_COARSE`): a write after a reading of it
/// gets a time no earlier than that reading, whether the file system takes this clock or a finer
/// one.
#[cfg(target_os = "linux")]
fn file_clock() -> std::time::SystemTime {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a buffer of the type the call fills; a call that fails leaves the epoch, which
    // tells nothing.
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME_COARSE, &raw mut now) };
    shifted(
        std::time::UNIX_EPOCH
            + std::time::Duration::new(
                u64::try_from(now.tv_sec).unwrap_or(0),
                u32::try_from(now.tv_nsec).unwrap_or(0),
            ),
    )
}

/// The clock this system gives file times from: the system time.
#[cfg(all(unix, not(target_os = "linux")))]
fn file_clock() -> std::time::SystemTime {
    shifted(std::time::SystemTime::now())
}

/// The clock Windows gives file times from (`GetSystemTimeAsFileTime`, which moves at each tick
/// of the system clock): a write after a reading of it gets a time no earlier than that reading.
#[cfg(windows)]
fn file_clock() -> std::time::SystemTime {
    use windows_sys::Win32::Foundation::FILETIME;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// The system time as of the clock's last tick, which NTFS stamps a write with.
        fn GetSystemTimeAsFileTime(time: *mut FILETIME);
    }
    let mut now = FILETIME::default();
    // SAFETY: a buffer of the type the call fills.
    unsafe { GetSystemTimeAsFileTime(&raw mut now) };
    let count = (u64::from(now.dwHighDateTime) << 32) | u64::from(now.dwLowDateTime);
    shifted(file_time(count).unwrap_or(std::time::UNIX_EPOCH))
}

/// `time`, read from the file clock; the cases move it ahead of the file times to stand for a
/// clock ahead of the one that set them.
fn shifted(time: std::time::SystemTime) -> std::time::SystemTime {
    #[cfg(test)]
    let time = time + tests::clock_ahead();
    time
}

/// A `FILETIME` count of 100 ns since 1601 as a time; `None` before 1970.
#[cfg(windows)]
fn file_time(count: u64) -> Option<std::time::SystemTime> {
    /// 1970 as a `FILETIME` count.
    const UNIX_EPOCH: u64 = 116_444_736_000_000_000;
    let since = count.checked_sub(UNIX_EPOCH)?;
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(since.checked_mul(100)?))
}

/// Whether `now`, from the file clock, has left the step that gave `modified`, as its digits show
/// it ([`time_step`]).
fn past_step(modified: std::time::SystemTime, now: std::time::SystemTime) -> bool {
    now > modified + time_step(modified)
}

/// The step of the file system that gave `time`, read from its digits: a file system keeps times
/// to a step of its own (a nanosecond on ext4, 100 ns on NTFS, 10 ms on exFAT, one or two seconds
/// on FAT), so a time's nanoseconds are a multiple of it. The step is taken as the largest power of
/// ten that divides them, and two seconds for a whole second; a time of a finer file system that
/// ends in zeros by chance is left a little later.
fn time_step(time: std::time::SystemTime) -> std::time::Duration {
    let nanos = time
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    if nanos == 0 {
        return std::time::Duration::from_secs(2);
    }
    let mut step = 1;
    while nanos.is_multiple_of(step * 10) {
        step *= 10;
    }
    std::time::Duration::from_nanos(u64::from(step))
}

impl FileStamp {
    /// `path`'s stamp: `Some(None)` when there is no file, `None` when the stat is refused.
    #[cfg(unix)]
    fn of(path: &Path) -> Option<Option<Self>> {
        use std::os::unix::fs::MetadataExt;
        let metadata = match stat(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(None),
            Err(_) => return None,
        };
        let stamp = Self {
            identity: (metadata.dev(), metadata.ino()),
            modified: metadata.modified().ok(),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            len: metadata.len(),
        };
        #[cfg(test)]
        let stamp = tests::as_the_volume_gives_it(stamp);
        Some(Some(stamp))
    }

    /// Whether this stamp tells its file from every later one on a local volume: once the file
    /// clock (`now`) has left the steps that gave the file's times, since a later write can get
    /// either only within it. A stamp taken within a step is not kept, so the next question
    /// compares the checksum; a stamp taken later is kept.
    fn tells(&self, now: std::time::SystemTime) -> bool {
        self.times()
            .into_iter()
            .all(|time| time.is_some_and(|time| past_step(time, now)))
    }

    /// The file's modification time and the time it last changed.
    #[cfg(unix)]
    fn times(&self) -> [Option<std::time::SystemTime>; 2] {
        let (seconds, nanos) = self.changed;
        let changed = u64::try_from(seconds).ok().zip(u32::try_from(nanos).ok());
        [
            self.modified,
            changed.map(|(seconds, nanos)| {
                std::time::UNIX_EPOCH + std::time::Duration::new(seconds, nanos)
            }),
        ]
    }

    /// The file's modification time.
    #[cfg(windows)]
    fn times(&self) -> [Option<std::time::SystemTime>; 1] {
        [file_time(self.modified)]
    }

    /// `path`'s stamp: `Some(None)` when there is no file, `None` when it cannot be read. One open
    /// and one query, the ones `std::fs::metadata` makes, so a question costs a stat: the 128-bit
    /// file id would take a second query.
    #[cfg(windows)]
    fn of(path: &Path) -> Option<Option<Self>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let file = match open_for_stat(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(None),
            Err(_) => return None,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: an open handle and a buffer of the type the call fills.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) } == 0 {
            return None;
        }
        let join = |high: u32, low: u32| (u64::from(high) << 32) | u64::from(low);
        let stamp = Self {
            identity: (
                u64::from(info.dwVolumeSerialNumber),
                join(info.nFileIndexHigh, info.nFileIndexLow),
            ),
            modified: join(
                info.ftLastWriteTime.dwHighDateTime,
                info.ftLastWriteTime.dwLowDateTime,
            ),
            len: join(info.nFileSizeHigh, info.nFileSizeLow),
        };
        #[cfg(test)]
        let stamp = tests::as_the_volume_gives_it(stamp);
        Some(Some(stamp))
    }
}

/// `path`'s metadata, as `std::fs::metadata` reads it; the cases refuse it.
#[cfg(unix)]
fn stat(path: &Path) -> std::io::Result<std::fs::Metadata> {
    #[cfg(test)]
    if tests::stat_refused() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    std::fs::metadata(path)
}

/// `path` opened as `std::fs::metadata` opens it: no access and every share mode, since Git may
/// rename over it meanwhile; the cases refuse it.
#[cfg(windows)]
fn open_for_stat(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
    #[cfg(test)]
    if tests::stat_refused() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    std::fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

/// The pattern that ignores a path: the file Git read it from (absolute; `None` when it came from
/// no file), its line there, and the pattern as written.
#[napi(object)]
#[derive(Debug, PartialEq, Eq)]
pub struct GitIgnoreMatch {
    pub source: Option<String>,
    pub line: u32,
    pub pattern: String,
}

/// A Git exclude stack for a host's walk below one root; see the module documentation. The
/// process's `GitAccess` opens it.
#[napi]
pub struct GitIgnoreMatcher {
    inner: Mutex<GitIgnore>,
    /// The root as the host named it, which the time limit of its opens is kept by (`pool::run`).
    opened_as: PathBuf,
}

impl GitIgnoreMatcher {
    /// The matcher for paths below `root`, opened on a thread of its own under the Git read time
    /// limit; `None` when `root` is in no Git worktree.
    pub fn open(root: String) -> std::result::Result<Option<Self>, pool::GitReadError> {
        let key = std::path::PathBuf::from(&root);
        let opened = pool::run(
            "opening Git's ignore rules",
            &key,
            GIT_READ_TIME_LIMIT,
            move || GitIgnore::open(Path::new(&root)),
        )?
        .map_err(|message| pool::GitReadError {
            timed_out: false,
            message,
        })?;
        Ok(opened.map(|inner| Self {
            inner: Mutex::new(inner),
            opened_as: key,
        }))
    }

    fn read<T>(
        &self,
        read: impl FnOnce(&mut GitIgnore) -> std::result::Result<T, String>,
    ) -> Result<T> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "Git ignore matcher is poisoned"))?;
        read(&mut inner).map_err(|message| Error::new(Status::GenericFailure, message))
    }
}

#[napi]
impl GitIgnoreMatcher {
    /// Whether Git ignores `relative_path` (to the root the matcher was opened for, `/`-separated);
    /// `is_directory` lets directory-only patterns (`build/`) match.
    #[napi(catch_unwind)]
    pub fn ignored(&self, relative_path: String, is_directory: bool) -> Result<bool> {
        self.read(|inner| inner.ignored(&relative_path, is_directory))
    }

    /// The pattern that makes Git ignore `relative_path` (as for `ignored`), with its file and
    /// line; null when Git does not ignore it.
    #[napi(catch_unwind)]
    pub fn ignored_by(
        &self,
        relative_path: String,
        is_directory: bool,
    ) -> Result<Option<GitIgnoreMatch>> {
        self.read(|inner| inner.ignored_by(&relative_path, is_directory))
    }

    /// Whether a matcher opened now would follow the rules this one follows (the configuration
    /// Git reads, read again, and the files read whole as they were read); when it would, each
    /// folder's `.gitignore` is read again as the next path enters the folder.
    #[napi(catch_unwind)]
    pub fn sources_unchanged(&self) -> Result<bool> {
        let root = self.read(|inner| Ok(inner.root.clone()))?;
        // Opened again as an open is: a disk that stops answering costs the check, not the host's
        // thread, and while an open of this root gives no answer the check fails at once.
        let now = pool::run(
            "checking Git's ignore rules",
            &self.opened_as,
            GIT_READ_TIME_LIMIT,
            move || GitIgnore::sources_now(&root),
        )
        .map_err(|error| Error::new(Status::GenericFailure, error.message))?;
        self.read(|inner| Ok(inner.take_sources(now)))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::time::{Duration, SystemTime};

    thread_local! {
        /// The volume this thread's readers take in place of the one found.
        static VOLUME: Cell<Option<Volume>> = const { Cell::new(None) };
        /// How far ahead of the file clock this thread's clock reads.
        static CLOCK_AHEAD: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        /// Whether this thread's stamps take one identity and change time for every file.
        static ONE_IDENTITY: Cell<bool> = const { Cell::new(false) };
        /// Whether this thread's stats of a file are refused.
        static STAT_REFUSED: Cell<bool> = const { Cell::new(false) };
        /// What runs once, right after this thread's next read of the index file: the whole
        /// index, or the checksum at its end.
        static AFTER_READ: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    }

    pub(super) fn volume() -> Option<Volume> {
        VOLUME.with(Cell::get)
    }

    pub(super) fn clock_ahead() -> Duration {
        CLOCK_AHEAD.with(Cell::get)
    }

    pub(super) fn stat_refused() -> bool {
        STAT_REFUSED.with(Cell::get)
    }

    /// `stamp` as a volume that gives an identity and a change time again gives it: with one of
    /// each for every file, so only the modification time and size tell two files apart.
    pub(super) fn as_the_volume_gives_it(mut stamp: FileStamp) -> FileStamp {
        if ONE_IDENTITY.with(Cell::get) {
            stamp.identity = (0, 0);
            #[cfg(unix)]
            {
                stamp.changed = (0, 0);
            }
        }
        stamp
    }

    pub(super) fn after_index_read() {
        if let Some(run) = AFTER_READ.with(|after| after.borrow_mut().take()) {
            run();
        }
    }

    /// Stand-ins for what a volume and its clock do, on this thread until dropped.
    struct Seams;

    impl Seams {
        fn new() -> Self {
            Self
        }

        fn volume(self, volume: Volume) -> Self {
            VOLUME.with(|kept| kept.set(Some(volume)));
            self
        }

        fn clock_ahead(self, by: Duration) -> Self {
            CLOCK_AHEAD.with(|kept| kept.set(by));
            self
        }

        fn one_identity(self) -> Self {
            ONE_IDENTITY.with(|kept| kept.set(true));
            self
        }

        fn stat_refused(self) -> Self {
            STAT_REFUSED.with(|kept| kept.set(true));
            self
        }

        fn after_index_read(self, run: impl FnOnce() + 'static) -> Self {
            AFTER_READ.with(|after| *after.borrow_mut() = Some(Box::new(run)));
            self
        }
    }

    impl Drop for Seams {
        fn drop(&mut self) {
            VOLUME.with(|kept| kept.set(None));
            CLOCK_AHEAD.with(|kept| kept.set(Duration::ZERO));
            ONE_IDENTITY.with(|kept| kept.set(false));
            STAT_REFUSED.with(|kept| kept.set(false));
            AFTER_READ.with(|after| after.borrow_mut().take());
        }
    }

    fn git(directory: &Path, arguments: &[&str]) {
        run_git(std::process::Command::new("git"), directory, arguments);
    }

    /// `git` with `index` as its index file in place of the worktree's.
    fn git_with_index(directory: &Path, index: &Path, arguments: &[&str]) {
        let mut command = std::process::Command::new("git");
        command.env("GIT_INDEX_FILE", index);
        run_git(command, directory, arguments);
    }

    fn run_git(mut command: std::process::Command, directory: &Path, arguments: &[&str]) {
        let output = command
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed:\n{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn modified(file: &Path) -> SystemTime {
        std::fs::metadata(file).unwrap().modified().unwrap()
    }

    /// Gives `file` the modification time `time`, as Git's second write of a file within one step
    /// of the file system's clock leaves it.
    fn set_modified(file: &Path, time: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    /// The index reads this thread has made.
    fn index_reads() -> usize {
        INDEX_READS.with(std::cell::Cell::get)
    }

    /// Returns once the file clock has left the steps of `file`'s times, so a stamp of it read now
    /// is kept on any volume. It waits for a tick of the clock at most.
    fn settle(file: &Path) {
        let stamp = FileStamp::of(file).flatten().expect("a file");
        let bound = std::time::Instant::now() + Duration::from_secs(5);
        while !stamp.tells(file_clock()) && std::time::Instant::now() < bound {
            std::thread::yield_now();
        }
    }

    fn fixture(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("rerail-git-ignore-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, contents) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        root
    }

    #[test]
    fn follows_nested_gitignores_negation_and_exclude_but_never_tracked_paths() {
        let root = fixture(
            "stack",
            &[
                ("tracked/gen.py", "tracked\n"),
                (".gitignore", "gen/\n*.log\n!keep.log\n"),
                ("api/.gitignore", "e.py\n"),
                ("api/d.py", ""),
                ("api/e.py", ""),
                ("api/gen/c.py", ""),
                ("keep.log", ""),
                ("drop.log", ""),
                ("local.py", ""),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", "tracked/gen.py"]);
        std::fs::write(
            root.join(".gitignore"),
            "gen/\n*.log\n!keep.log\ntracked/\n",
        )
        .unwrap();
        std::fs::write(root.join(".git/info/exclude"), "local.py\n").unwrap();
        std::fs::write(root.join("tracked/fresh.py"), "").unwrap();

        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        let mut ignored = |path: &str, directory: bool| ignore.ignored(path, directory).unwrap();
        assert!(ignored("api/gen", true));
        assert!(ignored("api/gen/c.py", false));
        assert!(ignored("api/e.py", false));
        assert!(!ignored("api/d.py", false));
        assert!(
            !ignored("e.py", false),
            "a nested .gitignore applies below its folder only"
        );
        assert!(ignored("drop.log", false));
        assert!(!ignored("keep.log", false), "negation re-includes");
        assert!(ignored("local.py", false), ".git/info/exclude applies");
        assert!(
            !ignored("tracked", true),
            "a folder holding tracked files is walked"
        );
        assert!(
            !ignored("tracked/gen.py", false),
            "a tracked file is never ignored"
        );
        assert!(ignored("tracked/fresh.py", false));
        assert!(!ignored("", true));
        assert!(!ignored("../outside.py", false));

        let mut below = GitIgnore::open(&root.join("api")).unwrap().unwrap();
        assert!(below.ignored("gen/c.py", false).unwrap());
        assert!(below.ignored("e.py", false).unwrap());
        assert!(below.ignored("../drop.log", false).unwrap());
        assert!(!below.ignored("d.py", false).unwrap());

        git(&root, &["add", "-f", "api/e.py"]);
        assert!(
            !ignore.ignored("api/e.py", false).unwrap(),
            "the index is re-read once it changes"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_the_index_again_when_git_rewrites_it_within_one_clock_step() {
        let root = fixture(
            "step",
            &[(".gitignore", "gen/\n"), ("gen/a.py", ""), ("b.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", "b.py"]);
        let index = root.join(".git/index");
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("gen/a.py", false).unwrap());

        let read = modified(&index);
        git(&root, &["add", "-f", "gen/a.py"]);
        set_modified(&index, read);
        assert!(
            !ignore.ignored("gen/a.py", false).unwrap(),
            "a file Git just made tracked is not ignored, though the index kept its time"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_the_index_again_when_its_file_identity_time_or_size_alone_changes() {
        let root = fixture(
            "stamp",
            &[(".gitignore", "e.py\n"), ("d.py", ""), ("e.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", "d.py"]);
        let index = root.join(".git/index");
        let tracking_e = root.join(".git/tracking-e");
        git_with_index(&root, &tracking_e, &["add", "-f", "e.py"]);
        let tracking_both = root.join(".git/tracking-both");
        git_with_index(&root, &tracking_both, &["add", "-f", "d.py", "e.py"]);
        let tracking_d = std::fs::read(&index).unwrap();
        assert_eq!(
            tracking_d.len(),
            std::fs::read(&tracking_e).unwrap().len(),
            "two indexes of one entry each, whose paths differ in one letter"
        );
        let time = modified(&index);
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("e.py", false).unwrap());

        // Another file of the same size and time, renamed over the index as Git writes it.
        set_modified(&tracking_e, time);
        std::fs::rename(&tracking_e, &index).unwrap();
        assert!(!ignore.ignored("e.py", false).unwrap(), "another file");

        // The same file and size at another time.
        std::fs::write(&index, &tracking_d).unwrap();
        let later = time + Duration::from_secs(1);
        set_modified(&index, later);
        assert!(ignore.ignored("e.py", false).unwrap(), "another time");

        // The same file and time at another size.
        std::fs::write(&index, std::fs::read(&tracking_both).unwrap()).unwrap();
        set_modified(&index, later);
        assert!(!ignore.ignored("e.py", false).unwrap(), "another size");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_no_index_while_its_file_is_unchanged() {
        let root = fixture("kept", &[(".gitignore", "e.py\n"), ("e.py", "")]);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore"]);
        let index = root.join(".git/index");
        settle(&index);
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("e.py", false).unwrap());
        let reads = index_reads();
        assert!(ignore.ignored("e.py", false).unwrap());
        assert_eq!(
            index_reads(),
            reads,
            "answered from the index read before, with one stat of its file"
        );
        set_modified(&index, modified(&index) + Duration::from_secs(1));
        assert!(ignore.ignored("e.py", false).unwrap());
        assert_eq!(
            index_reads(),
            reads,
            "the same bytes at another time: the checksum tells the index read, none is read"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_the_pattern_its_file_and_line() {
        let root = fixture(
            "source",
            &[
                (".gitignore", "# generated\n*.log\nscratch/\n"),
                ("api/.gitignore", "e.py\n"),
                ("api/e.py", ""),
                ("scratch/evil.ts", ""),
                ("drop.log", ""),
                ("local.py", ""),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        std::fs::write(root.join(".git/info/exclude"), "# local\n\nlocal.py\n").unwrap();
        let root = dunce::canonicalize(root).unwrap();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        let source = |path: &str| Some(root.join(path).to_string_lossy().into_owned());
        let matched = |pattern: &str, path: &str, line: u32| GitIgnoreMatch {
            source: source(path),
            line,
            pattern: pattern.to_owned(),
        };

        assert_eq!(
            ignore.ignored_by("local.py", false).unwrap(),
            Some(matched("local.py", ".git/info/exclude", 3))
        );
        assert_eq!(
            ignore.ignored_by("drop.log", false).unwrap(),
            Some(matched("*.log", ".gitignore", 2))
        );
        assert_eq!(
            ignore.ignored_by("scratch/evil.ts", false).unwrap(),
            Some(matched("scratch/", ".gitignore", 3)),
            "a file under an ignored folder names the folder's pattern"
        );
        assert_eq!(
            ignore.ignored_by("api/e.py", false).unwrap(),
            Some(matched("e.py", "api/.gitignore", 1))
        );
        assert_eq!(ignore.ignored_by("api", true).unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A path as a configuration value: forward slashes, which Git reads on every platform.
    fn config_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    /// Sets `core.ignoreCase` to the opposite of what it reads now. `git init` writes it true
    /// where file names fold case (Windows, macOS) and leaves it out elsewhere, so setting one
    /// fixed value would change nothing on one of those systems.
    fn turn_ignore_case(root: &Path) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "config",
                "--type=bool",
                "--default=false",
                "--get",
                "core.ignoreCase",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git config --get core.ignoreCase failed"
        );
        let folds = String::from_utf8_lossy(&output.stdout).trim() == "true";
        let value = if folds { "false" } else { "true" };
        git(root, &["config", "core.ignoreCase", value]);
    }

    /// Writes `contents`, as long as the file's old bytes, over `path` in place and puts its old
    /// write time back, as a copy that keeps times does, so only the bytes tell the two apart.
    fn rewrite_keeping_size_and_time(path: &Path, contents: &str) {
        let old = std::fs::metadata(path).unwrap();
        assert_eq!(
            old.len(),
            contents.len() as u64,
            "the new bytes are as long"
        );
        std::fs::write(path, contents).unwrap();
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(old.modified().unwrap()).unwrap();
    }

    #[test]
    fn says_whether_a_matcher_opened_now_reads_its_rules() {
        let root = fixture(
            "sources",
            &[
                (".gitignore", "gen/\n"),
                ("named-a", "a.py\n"),
                ("pkg/x.py", ""),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        let root = dunce::canonicalize(root).unwrap();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.sources_unchanged(), "nothing changed");
        git(&root, &["config", "branch.main.remote", "origin"]);
        assert!(
            ignore.sources_unchanged(),
            "a configuration write that leaves the rules alone"
        );

        // An included file names the excludes file, with a comment after the value.
        let named = config_path(&root.join("named-a"));
        std::fs::write(
            root.join(".git/ignore.inc"),
            format!("[core]\n\texcludesFile = {named} ; a comment\n"),
        )
        .unwrap();
        git(&root, &["config", "include.path", "ignore.inc"]);
        assert!(
            !ignore.sources_unchanged(),
            "an included file that names an excludes file"
        );
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(ignore.ignored("a.py", false).unwrap());
        assert!(ignore.sources_unchanged());
        std::fs::write(root.join("named-a"), "b.py\nc.py\n").unwrap();
        assert!(!ignore.sources_unchanged(), "the named file changed");
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        rewrite_keeping_size_and_time(&root.join("named-a"), "d.py\ne.py\n");
        assert!(
            !ignore.sources_unchanged(),
            "the named file rewritten at its size and write time"
        );
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        turn_ignore_case(&root);
        assert!(!ignore.sources_unchanged(), "the pattern case changed");

        // Each folder's .gitignore is read again once the sources hold.
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(!ignore.ignored("pkg/x.py", false).unwrap());
        std::fs::write(root.join("pkg/.gitignore"), "x.py\n").unwrap();
        assert!(ignore.sources_unchanged());
        assert!(ignore.ignored("pkg/x.py", false).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The root `.gitignore` and `info/exclude` are read whole as a matcher opens, so a change of
    /// either is a change of its rules.
    #[test]
    fn says_a_matcher_opened_now_reads_a_changed_root_gitignore_or_info_exclude() {
        let root = fixture(
            "root-files",
            &[(".gitignore", "gen/\n"), ("a.py", ""), ("b.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        let root = dunce::canonicalize(root).unwrap();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(!ignore.ignored("a.py", false).unwrap());
        assert!(ignore.sources_unchanged(), "nothing changed");
        std::fs::write(root.join(".gitignore"), "gen/\na.py\n").unwrap();
        assert!(!ignore.sources_unchanged(), "the root .gitignore changed");
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(ignore.ignored("a.py", false).unwrap());
        assert!(ignore.sources_unchanged());
        std::fs::write(root.join(".git/info/exclude"), "b.py\n").unwrap();
        assert!(!ignore.sources_unchanged(), "info/exclude changed");
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(ignore.ignored("b.py", false).unwrap());
        assert!(ignore.sources_unchanged());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The rules a matcher reads again with a new index come with their sources read again, so
    /// `info/exclude` changed, read with the new index and then put back as it was at the open is a
    /// change of the rules the matcher follows.
    #[test]
    fn says_its_rules_changed_once_a_file_read_again_with_the_index_is_put_back() {
        let root = fixture(
            "put-back",
            &[(".gitignore", "gen/\n"), ("a.py", ""), ("b.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        let root = dunce::canonicalize(root).unwrap();
        let exclude = root.join(".git").join("info").join("exclude");
        std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
        std::fs::write(&exclude, "# none\n").unwrap();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(!ignore.ignored("a.py", false).unwrap());
        std::fs::write(&exclude, "a.py\n").unwrap();
        git(&root, &["add", "b.py"]);
        assert!(
            ignore.ignored("a.py", false).unwrap(),
            "read again with the index"
        );
        std::fs::write(&exclude, "# none\n").unwrap();
        assert!(
            !ignore.sources_unchanged(),
            "info/exclude as the open read it, not as the matcher reads it now"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A matcher whose sources hold is handed on, and asked again by its holders, with the index
    /// Git wrote since it read it, even at the very time it read, as two writes within one tick of
    /// the file system's clock leave it: the rewrite is a new file, which it reads again.
    #[test]
    fn reads_the_index_again_when_handed_on_as_one_opened_now() {
        let root = fixture("index-again", &[(".gitignore", "gen/\n"), ("gen/x.py", "")]);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore"]);
        let root = dunce::canonicalize(root).unwrap();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("gen/x.py", false).unwrap());
        let index = root.join(".git").join("index");
        let read = std::fs::metadata(&index).unwrap().modified().unwrap();
        git(&root, &["add", "-f", "gen/x.py"]);
        let file = std::fs::File::options().write(true).open(&index).unwrap();
        file.set_modified(read).unwrap();
        drop(file);
        assert!(ignore.sources_unchanged(), "the rules' sources are as read");
        assert!(
            !ignore.ignored("gen/x.py", false).unwrap(),
            "a force-added file is tracked, as a matcher opened now reads it"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A `.gitignore` the index marks skip-worktree and the worktree lacks is read from the blob
    /// the index names. Once the index names another blob, the matcher reads that one, as an open
    /// then does, whether it is handed on or asked again by a holder.
    #[test]
    fn reads_a_skipped_gitignore_as_the_index_names_it_when_handed_on() {
        let root = fixture(
            "skipped",
            &[
                ("docs/.gitignore", "x.py\n"),
                ("docs/x.py", ""),
                ("docs/y.py", ""),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", "docs/.gitignore"]);
        git(
            &root,
            &["update-index", "--skip-worktree", "docs/.gitignore"],
        );
        std::fs::remove_file(root.join("docs/.gitignore")).unwrap();
        let root = dunce::canonicalize(root).unwrap();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        let answers = |ignore: &mut GitIgnore| {
            (
                ignore.ignored("docs/x.py", false).unwrap(),
                ignore.ignored("docs/y.py", false).unwrap(),
            )
        };
        assert_eq!(answers(&mut ignore), (true, false), "the index's blob");
        let other = root.join("other-gitignore");
        std::fs::write(&other, "y.py\n").unwrap();
        // Named relative to the root: Git for Windows reads no `\\?\` path as a file argument.
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["hash-object", "-w", "other-gitignore"])
            .output()
            .unwrap();
        assert!(output.status.success(), "git hash-object failed");
        let blob = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        std::fs::remove_file(&other).unwrap();
        let info = format!("100644,{blob},docs/.gitignore");
        git(&root, &["update-index", "--cacheinfo", &info]);
        git(
            &root,
            &["update-index", "--skip-worktree", "docs/.gitignore"],
        );
        let mut fresh = GitIgnore::open(&root).unwrap().unwrap();
        assert_eq!(answers(&mut fresh), (false, true), "the new blob");
        assert!(ignore.sources_unchanged());
        assert_eq!(
            answers(&mut ignore),
            (false, true),
            "handed on or asked again"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The check of a matcher's sources opens the repository again as an open does, under its
    /// root's time limit: while an open of that root has given no answer, the check fails at once
    /// rather than wait on the same disk on the host's thread.
    #[test]
    fn checks_its_sources_under_the_time_limit_of_its_root() {
        let root = fixture("check-limit", &[(".gitignore", "gen/\n")]);
        git(&root, &["init", "--initial-branch=main"]);
        let named = root.to_string_lossy().into_owned();
        let matcher = GitIgnoreMatcher::open(named.clone())
            .unwrap()
            .expect("a Git worktree");
        assert!(matcher.sources_unchanged().unwrap());
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let (started, began) = std::sync::mpsc::channel::<()>();
        let key = PathBuf::from(&named);
        let limit = std::time::Duration::from_millis(100);
        let stuck = std::thread::spawn(move || {
            pool::run("a stuck open", &key, limit, move || {
                started.send(()).unwrap();
                let _ = wait.recv();
            })
        });
        began.recv().unwrap();
        assert!(stuck.join().unwrap().unwrap_err().timed_out);
        let checked = std::time::Instant::now();
        let refused = matcher.sources_unchanged().unwrap_err();
        assert!(refused.reason.contains("has not ended"));
        assert!(checked.elapsed() < std::time::Duration::from_secs(1));
        release.send(()).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A linked worktree reads `info/exclude` in the repository's common Git folder, not in its
    /// own folder under `worktrees/`.
    #[test]
    fn says_a_matcher_in_a_linked_worktree_reads_the_common_info_exclude() {
        let root = fixture("common", &[(".gitignore", "gen/\n")]);
        let linked = std::env::temp_dir().join(format!(
            "rerail-git-ignore-common-linked-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&linked);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Rerail",
                "-c",
                "user.email=rerail@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "base",
            ],
        );
        let path = linked.to_string_lossy().into_owned();
        git(&root, &["worktree", "add", "--quiet", "--detach", &path]);
        std::fs::write(linked.join("c.py"), "").unwrap();
        let linked = dunce::canonicalize(linked).unwrap();
        let mut ignore = GitIgnore::open(&linked).unwrap().expect("a Git worktree");
        assert!(!ignore.ignored("c.py", false).unwrap());
        assert!(ignore.sources_unchanged(), "nothing changed");
        std::fs::write(root.join(".git/info/exclude"), "c.py\n").unwrap();
        assert!(
            !ignore.sources_unchanged(),
            "the common info/exclude changed"
        );
        let mut fresh = GitIgnore::open(&linked).unwrap().unwrap();
        assert!(
            fresh.ignored("c.py", false).unwrap(),
            "a matcher opened now reads the common info/exclude"
        );
        std::fs::remove_dir_all(linked).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// `gitoxide.parsePrecious` decides whether a `$` line is a precious pattern or a name that
    /// starts with `$`, so turning it on changes what the same `.gitignore` ignores.
    #[test]
    fn says_a_matcher_opened_now_parses_precious_patterns_once_they_are_turned_on() {
        let root = fixture(
            "precious",
            &[(".gitignore", "$local.py\n"), ("local.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        let root = dunce::canonicalize(root).unwrap();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(!ignore.ignored("local.py", false).unwrap());
        assert!(ignore.sources_unchanged(), "nothing changed");
        git(&root, &["config", "gitoxide.parsePrecious", "true"]);
        assert!(
            !ignore.sources_unchanged(),
            "precious patterns are parsed now"
        );
        let mut fresh = GitIgnore::open(&root).unwrap().unwrap();
        assert!(
            fresh.ignored("local.py", false).unwrap(),
            "a matcher opened now reads $local.py as a precious pattern"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The repository's `core.worktree` names another folder: the same Git folder and, in each
    /// folder, a root `.gitignore` of the same bytes, so only the worktree tells the two apart.
    #[test]
    fn says_a_root_whose_repository_now_names_another_worktree_has_changed() {
        let root = fixture(
            "worktree",
            &[
                (".gitignore", "local.py\n"),
                ("sub/local.py", ""),
                ("other/.gitignore", "local.py\n"),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        let root = dunce::canonicalize(root).unwrap();
        let sub = root.join("sub");
        let mut ignore = GitIgnore::open(&sub).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("local.py", false).unwrap());
        assert!(ignore.sources_unchanged(), "nothing changed");
        let other = config_path(&root.join("other"));
        git(&root, &["config", "core.worktree", &other]);
        assert!(
            !ignore.sources_unchanged(),
            "the repository's worktree is another folder"
        );
        assert!(
            GitIgnore::open(&sub).unwrap().is_none(),
            "a matcher opened now finds the root outside the worktree"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_folder_outside_git_has_no_matcher() {
        let root = fixture("plain", &[(".gitignore", "*.py\n"), ("a.py", "")]);
        // A temporary directory may sit inside a checkout on some hosts; only assert where it does not.
        if gix::discover(&root).is_err() {
            assert!(GitIgnore::open(&root).unwrap().is_none());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The files this process holds open under `folder`, by the paths `/proc/self/fd` names: a file
    /// deleted since keeps its path there, with ` (deleted)` after it.
    #[cfg(target_os = "linux")]
    fn open_under(folder: &Path) -> Vec<PathBuf> {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| target.starts_with(folder))
            .collect()
    }

    /// ext4 gives a freed inode number to the next new file at once. After a matcher reads the
    /// index, Git writes it twice within one tick of the file system's clock: the first write frees
    /// the number of the file read, and the second, at its size and time, can land on it. Both
    /// writes fall within the clock step of the file read only if the read did too, so the index's
    /// time is put ahead of the file clock first: the matcher keeps no stamp of a read taken then,
    /// and reads the second write. Other threads take and free inode numbers too, so the writes are
    /// made again until the second lands on the number read; a file system that gives no number
    /// again (tmpfs) cannot make the state, and says so.
    #[cfg(target_os = "linux")]
    #[test]
    fn reads_the_index_again_when_git_writes_it_twice_onto_the_inode_number_of_the_file_read() {
        use std::os::unix::fs::MetadataExt;
        let root = fixture(
            "reused",
            &[(".gitignore", "gen/\n"), ("gen/t.py", ""), ("z/m.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore", "z/m.py"]);
        let index = root.join(".git/index");
        let ahead = file_clock() + Duration::from_millis(3_600_500);
        set_modified(&index, ahead);
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        let made = (0..10).any(|_| {
            assert!(ignore.ignored("gen/t.py", false).unwrap(), "read out of the index");
            let read = std::fs::metadata(&index).unwrap();
            git(&root, &["add", "-f", "gen/t.py"]);
            git(&root, &["update-index", "--force-remove", "z/m.py"]);
            set_modified(&index, read.modified().unwrap());
            let now = std::fs::metadata(&index).unwrap();
            assert_eq!(now.len(), read.len(), "both entries take 72 bytes");
            assert!(
                !ignore.ignored("gen/t.py", false).unwrap(),
                "a file Git now tracks is not ignored (read on inode {}, the index now on inode {})",
                read.ino(),
                now.ino()
            );
            git(&root, &["update-index", "--add", "z/m.py"]);
            git(&root, &["rm", "--cached", "-q", "gen/t.py"]);
            set_modified(&index, ahead);
            now.ino() == read.ino()
        });
        if !made {
            eprintln!("no second write took the inode number of the file read in 10 tries");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A matcher holds no file under its checkout open: not the index it read, nor one Git replaced,
    /// nor any once the checkout is removed. So the sessions that read a checkout leave nothing of
    /// it open, whenever their matcher is dropped.
    #[cfg(target_os = "linux")]
    #[test]
    fn holds_no_file_under_its_checkout_open() {
        let root = fixture("open", &[(".gitignore", "gen/\n"), ("gen/t.py", "")]);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore"]);
        let root = dunce::canonicalize(root).unwrap();
        let none = Vec::<PathBuf>::new();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("gen/t.py", false).unwrap());
        assert_eq!(open_under(&root), none, "after a question");

        git(&root, &["add", "-f", "gen/t.py"]);
        assert!(
            !ignore.ignored("gen/t.py", false).unwrap(),
            "the index Git wrote"
        );
        assert_eq!(open_under(&root), none, "after Git replaced the index");

        std::fs::remove_dir_all(&root).unwrap();
        assert!(!ignore.ignored("gen/t.py", false).unwrap());
        assert_eq!(open_under(&root), none, "once the checkout is removed");
    }

    /// On a volume that gives an identity again, a stamp taken while the file clock still reads the
    /// index's time, or the time it last changed, cannot tell a later write within that step from
    /// the file read: it is not kept, so the next question compares the checksum, and reads the
    /// index only when it differs. A stamp taken once the clock has left both is kept, and a
    /// question costs a stat.
    #[test]
    fn keeps_a_stamp_once_the_file_clock_has_left_its_time() {
        let root = fixture("clock-step", &[(".gitignore", "e.py\n"), ("e.py", "")]);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore"]);
        let index = root.join(".git/index");
        let _seams = Seams::new().volume(Volume::Local);
        set_modified(&index, file_clock() - Duration::from_millis(1500));
        settle(&index);
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("e.py", false).unwrap());
        let reads = index_reads();
        assert!(ignore.ignored("e.py", false).unwrap());
        assert_eq!(
            index_reads(),
            reads,
            "times the clock has left: the stamp is kept"
        );

        set_modified(&index, file_clock() + Duration::from_millis(3_600_500));
        assert!(ignore.ignored("e.py", false).unwrap());
        assert!(ignore.ignored("e.py", false).unwrap());
        assert_eq!(
            index_reads(),
            reads,
            "a time the clock still reads: the checksum is compared at each question, none is read"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A tool can set a file's time back, but not the time it last changed. After a read whose
    /// stamp is kept, Git writes the index twice and its time is set back to the one read, as two
    /// writes within one clock step leave it; on ext4 the second write can land on the inode number
    /// read, at its size, so only the change time tells it from the file read. The writes are made
    /// again until the number is reused; a file system that gives no number again cannot make it.
    #[cfg(target_os = "linux")]
    #[test]
    fn reads_the_index_again_when_a_later_write_has_the_inode_number_size_and_time_read() {
        use std::os::unix::fs::MetadataExt;
        let root = fixture(
            "set-back",
            &[(".gitignore", "gen/\n"), ("gen/t.py", ""), ("z/m.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore", "z/m.py"]);
        let index = root.join(".git/index");
        settle(&index);
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        let made = (0..10).any(|_| {
            assert!(ignore.ignored("gen/t.py", false).unwrap(), "read out of the index");
            let read = std::fs::metadata(&index).unwrap();
            git(&root, &["add", "-f", "gen/t.py"]);
            git(&root, &["update-index", "--force-remove", "z/m.py"]);
            set_modified(&index, read.modified().unwrap());
            let now = std::fs::metadata(&index).unwrap();
            assert_eq!(now.len(), read.len(), "both entries take 72 bytes");
            assert!(
                !ignore.ignored("gen/t.py", false).unwrap(),
                "a file Git now tracks is not ignored (read on inode {}, the index now on inode {})",
                read.ino(),
                now.ino()
            );
            git(&root, &["update-index", "--add", "z/m.py"]);
            git(&root, &["rm", "--cached", "-q", "gen/t.py"]);
            settle(&index);
            now.ino() == read.ino()
        });
        if !made {
            eprintln!("no second write took the inode number of the file read in 10 tries");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// An index removed after a read whose stamp was not kept reads as no index, as Git reads it:
    /// the stamp not kept is not taken for the stamp of no file. The index's time is put an hour
    /// ahead, which no file clock has left.
    #[test]
    fn reads_no_index_once_an_index_read_within_its_clock_step_is_removed() {
        let root = fixture("removed", &[(".gitignore", "gen/\n"), ("gen/t.py", "")]);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", "-f", "gen/t.py"]);
        let index = root.join(".git/index");
        set_modified(&index, modified(&index) + Duration::from_secs(3_600));
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(!ignore.ignored("gen/t.py", false).unwrap(), "tracked");
        std::fs::remove_file(&index).unwrap();
        assert!(
            ignore.ignored("gen/t.py", false).unwrap(),
            "with no index Git tracks nothing"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A stamp read while the file clock still reads the time its file last changed is not kept,
    /// however long ago its modification time is: a write within that step can get the same change
    /// time. A time set back is a change, so each try reads the stamp right after one and checks it
    /// whenever the clock has not left that step by the end of the check.
    #[cfg(target_os = "linux")]
    #[test]
    fn keeps_no_stamp_while_the_file_clock_reads_its_change_time() {
        let root = fixture("change-step", &[("f", "")]);
        let file = root.join("f");
        let checked = (0..100).any(|_| {
            set_modified(&file, file_clock() - Duration::from_secs(5));
            let stamp = FileStamp::of(&file).flatten().expect("a file");
            let tells = stamp.tells(file_clock());
            let (seconds, nanos) = stamp.changed;
            let changed = std::time::UNIX_EPOCH
                + Duration::new(
                    u64::try_from(seconds).unwrap(),
                    u32::try_from(nanos).unwrap(),
                );
            let within = !past_step(changed, file_clock());
            assert!(
                !(within && tells),
                "a stamp read within its change time's step is kept"
            );
            within
        });
        assert!(
            checked,
            "the file clock left every change time's step before its check"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A time is left once the clock reads later than the next step its digits show: a nanosecond
    /// time a nanosecond on, a time of 100 ns steps (NTFS) or of 10 ms steps (exFAT) a step on, and
    /// a whole second, which a file system of one or two seconds a step gives, two seconds on. The
    /// later readings are a 100 ns count on, the finest time Windows holds.
    #[test]
    fn says_when_the_file_clock_has_left_a_files_time() {
        let at = |seconds: u64, nanos: u32| std::time::UNIX_EPOCH + Duration::new(seconds, nanos);
        #[cfg(unix)]
        {
            assert!(!past_step(at(10, 501), at(10, 502)));
            assert!(past_step(at(10, 501), at(10, 503)));
        }
        assert!(!past_step(at(10, 234_567_800), at(10, 234_567_900)));
        assert!(past_step(at(10, 234_567_800), at(10, 234_568_000)));
        assert!(!past_step(at(10, 10_000_000), at(10, 20_000_000)));
        assert!(past_step(at(10, 10_000_000), at(10, 20_000_100)));
        assert!(!past_step(at(10, 0), at(12, 0)));
        assert!(past_step(at(10, 0), at(12, 100)));
    }

    /// A checkout ignoring `gen/`, whose index tracks `.gitignore` and `z/m.py`; its index file.
    /// Tracking `gen/t.py` in place of `z/m.py` leaves the index as long: both entries take 72
    /// bytes.
    fn two_entry_checkout(name: &str) -> (PathBuf, PathBuf) {
        let root = fixture(
            name,
            &[(".gitignore", "gen/\n"), ("gen/t.py", ""), ("z/m.py", "")],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore", "z/m.py"]);
        let index = root.join(".git/index");
        (root, index)
    }

    /// Git's two writes of the index that track `gen/t.py` in place of `z/m.py`, the second with
    /// the time of the file read set back on it, as two writes within one step of a file system's
    /// clock leave it.
    fn track_t_in_place_of_m(root: &Path, index: &Path, read: &std::fs::Metadata) {
        git(root, &["add", "-f", "gen/t.py"]);
        git(root, &["update-index", "--force-remove", "z/m.py"]);
        set_modified(index, read.modified().unwrap());
        assert_eq!(
            std::fs::metadata(index).unwrap().len(),
            read.len(),
            "both entries take 72 bytes"
        );
    }

    /// On a volume whose file times come from another machine's clock (a share, a virtual
    /// machine's shared folder), this clock can have left a time that clock has not, so a later
    /// write can end on the identity, change time, time and size read. No stamp is kept there: a
    /// question compares the checksum at the index's end, and reads the index only when it differs.
    #[test]
    fn reads_the_index_again_on_a_volume_whose_times_come_from_another_clock() {
        let (root, index) = two_entry_checkout("remote");
        let _seams = Seams::new()
            .volume(Volume::Remote)
            .clock_ahead(Duration::from_secs(3_600))
            .one_identity();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(
            ignore.ignored("gen/t.py", false).unwrap(),
            "read out of the index"
        );
        let read = std::fs::metadata(&index).unwrap();
        track_t_in_place_of_m(&root, &index, &read);
        assert!(
            !ignore.ignored("gen/t.py", false).unwrap(),
            "a file Git now tracks is not ignored, though its stamp is the one read"
        );
        let reads = index_reads();
        assert!(!ignore.ignored("gen/t.py", false).unwrap());
        assert_eq!(index_reads(), reads, "the same checksum: no index read");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Where a volume gives a file's identity again (FAT and exFAT from its place in its folder,
    /// ReFS's 64-bit index), a stamp read within a step of the file's time is not kept, on every
    /// system: a later write can end on that identity, time and size. The index's time is put an
    /// hour ahead, which no clock has left.
    #[test]
    fn reads_the_index_again_where_a_volume_gives_a_file_identity_again() {
        let (root, index) = two_entry_checkout("identity-again");
        set_modified(&index, modified(&index) + Duration::from_secs(3_600));
        let _seams = Seams::new().volume(Volume::Local).one_identity();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(
            ignore.ignored("gen/t.py", false).unwrap(),
            "read out of the index"
        );
        let read = std::fs::metadata(&index).unwrap();
        track_t_in_place_of_m(&root, &index, &read);
        assert!(
            !ignore.ignored("gen/t.py", false).unwrap(),
            "a later file on the identity, time and size read"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The stamp is taken before the index is read, so an index Git writes as it is read is read
    /// at the next question, even where every stamp is kept at once.
    #[test]
    fn reads_an_index_written_as_it_was_read_at_the_next_question() {
        let (root, index) = two_entry_checkout("read-meanwhile");
        let next = root.join(".git/next-index");
        git_with_index(&root, &next, &["add", "-f", ".gitignore", "gen/t.py"]);
        let landing = (next.clone(), index.clone());
        let _seams = Seams::new()
            .volume(Volume::Unique)
            .after_index_read(move || std::fs::rename(&landing.0, &landing.1).unwrap());
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(!next.exists(), "the index Git wrote as the reader read");
        assert!(
            !ignore.ignored("gen/t.py", false).unwrap(),
            "the index Git wrote is read"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Git's two writes in the index's time step, the second onto the identity, time and size
    /// read, then the clock leaving that step: the run standing for those writes and that tick
    /// landing while the reader reads.
    fn writes_then_the_clock_leaves_the_step(root: &Path, index: &Path) -> impl FnOnce() + use<> {
        let read = std::fs::metadata(index).unwrap();
        let (root, index) = (root.to_path_buf(), index.to_path_buf());
        move || {
            track_t_in_place_of_m(&root, &index, &read);
            CLOCK_AHEAD.with(|kept| kept.set(Duration::from_secs(7_200)));
        }
    }

    /// On a volume that gives an identity again, whether the stamp is kept is decided by the
    /// clock as it read the stamp: where the clock leaves the index's step while the index is
    /// read, Git's writes meanwhile are read at the next question.
    #[test]
    fn reads_the_index_git_wrote_while_the_clock_left_the_step_of_the_index_read() {
        let (root, index) = two_entry_checkout("clock-left-in-read");
        set_modified(&index, modified(&index) + Duration::from_secs(3_600));
        let _seams = Seams::new()
            .volume(Volume::Local)
            .one_identity()
            .after_index_read(writes_then_the_clock_leaves_the_step(&root, &index));
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(
            !ignore.ignored("gen/t.py", false).unwrap(),
            "the index Git wrote while the reader read"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The same where the clock leaves the step while a question reads the index's checksum: the
    /// clock that decides whether the stamp is kept is read before that checksum.
    #[test]
    fn reads_the_index_git_wrote_while_the_clock_left_the_step_of_the_checksum_read() {
        let (root, index) = two_entry_checkout("clock-left-in-checksum");
        set_modified(&index, modified(&index) + Duration::from_secs(3_600));
        let seams = Seams::new().volume(Volume::Local).one_identity();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        let writes = writes_then_the_clock_leaves_the_step(&root, &index);
        let _seams = seams.after_index_read(writes);
        assert!(
            ignore.ignored("gen/t.py", false).unwrap(),
            "the index read: the file still ended with its checksum"
        );
        assert!(
            !ignore.ignored("gen/t.py", false).unwrap(),
            "the index Git wrote while the reader read its checksum"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A stamp whose stat is refused is not kept: each question compares the checksum, so an
    /// index Git writes, or removes, meanwhile is read as Git reads it.
    #[test]
    fn keeps_no_stamp_whose_stat_is_refused() {
        let (root, index) = two_entry_checkout("stat-refused");
        let _seams = Seams::new().volume(Volume::Unique).stat_refused();
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(
            ignore.ignored("gen/t.py", false).unwrap(),
            "read out of the index"
        );
        git(&root, &["add", "-f", "gen/t.py"]);
        assert!(
            !ignore.ignored("gen/t.py", false).unwrap(),
            "the index Git wrote"
        );
        std::fs::remove_file(&index).unwrap();
        assert!(
            ignore.ignored("gen/t.py", false).unwrap(),
            "with no index Git tracks nothing"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Under `index.skipHash` Git writes no checksum, so a question whose stamp is not kept reads
    /// the index again. A Git that writes one there anyway leaves the case nothing to check.
    #[test]
    fn reads_the_index_again_where_git_writes_no_checksum() {
        let (root, index) = two_entry_checkout("skip-hash");
        git(&root, &["config", "index.skipHash", "true"]);
        git(&root, &["add", "-f", "gen/t.py"]);
        git(&root, &["rm", "--cached", "-q", "gen/t.py"]);
        let bytes = std::fs::read(&index).unwrap();
        if bytes[bytes.len() - 20..] != [0; 20] {
            eprintln!("this Git writes a checksum under index.skipHash");
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        set_modified(&index, modified(&index) + Duration::from_secs(3_600));
        let _seams = Seams::new().volume(Volume::Local);
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("gen/t.py", false).unwrap());
        let reads = index_reads();
        assert!(ignore.ignored("gen/t.py", false).unwrap());
        assert_eq!(
            index_reads(),
            reads + 1,
            "no checksum to compare: read again"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// While it lives, a file this process cannot read: one with no permissions on Unix, one held
    /// open with no sharing on Windows.
    struct Unreadable {
        #[cfg(windows)]
        _file: std::fs::File,
    }

    impl Unreadable {
        /// `file` made unreadable; `None` where it stays readable, as to a process that reads any
        /// file.
        #[cfg(unix)]
        fn make(file: &Path) -> Option<Self> {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o000)).unwrap();
            std::fs::read(file).is_err().then_some(Self {})
        }

        #[cfg(windows)]
        fn make(file: &Path) -> Option<Self> {
            use std::os::windows::fs::OpenOptionsExt;
            let file = std::fs::File::options()
                .read(true)
                .share_mode(0)
                .open(file)
                .ok()?;
            Some(Self { _file: file })
        }
    }

    /// Rules that cannot be read again with a new index leave the ones the matcher read: it answers
    /// by them with the new index, reads that index once, and says an open now would not read them.
    #[test]
    fn keeps_its_rules_when_they_cannot_be_read_again_with_a_new_index() {
        let root = fixture(
            "rules-unread",
            &[
                (".gitignore", "b.py\n"),
                ("named", "a.py\n"),
                ("a.py", ""),
                ("b.py", ""),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        // The root as made, not resolved: Git for Windows takes no `\\?\` path as an index file.
        git(
            &root,
            &[
                "config",
                "core.excludesFile",
                &config_path(&root.join("named")),
            ],
        );
        let next = root.join(".git/next-index");
        git_with_index(&root, &next, &["add", "-f", "b.py"]);
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(
            ignore.ignored("a.py", false).unwrap(),
            "the excludes file's rule"
        );
        assert!(ignore.ignored("b.py", false).unwrap());

        if let Some(_unreadable) = Unreadable::make(&root.join("named")) {
            let index = root.join(".git/index");
            std::fs::rename(&next, &index).unwrap();
            settle(&index);
            assert!(
                ignore.ignored("a.py", false).unwrap(),
                "the rules read before"
            );
            assert!(!ignore.ignored("b.py", false).unwrap(), "the new index");
            let reads = index_reads();
            assert!(ignore.ignored("a.py", false).unwrap());
            assert_eq!(index_reads(), reads, "the new index read once");
            assert!(
                !ignore.sources_unchanged(),
                "an open now cannot read the rules"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
