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
//! one per checkout: every holder reads the index again once its file changes. A matcher holds no
//! file open.
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
        let root = root
            .canonicalize()
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

    /// Reads the index again once its file's stamp differs from the one it was read with, and makes
    /// the stack again from it, as an open makes it: the stack names each `.gitignore` the index
    /// marks skip-worktree by the blob the index holds. The sources are read again first, as an
    /// open reads them, so a check of them compares the files the new stack reads.
    fn refresh(&mut self) -> std::result::Result<(), String> {
        if self.index.stamp == Some(FileStamp::of(&self.index.path)) {
            return Ok(());
        }
        let sources = self
            .sources
            .as_ref()
            .and_then(|read| Sources::read(&self.repository, &read.worktree));
        let index = IndexFile::read(&self.repository)?;
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
    let worktree = worktree
        .canonicalize()
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
    let index = IndexFile::read(repository)?;
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
/// keeps the first one's time: the stamp tells that new file from the one read.
struct IndexFile {
    path: PathBuf,
    /// The file's stamp when it was read (`None` inside where there was no file), or `None` where
    /// that stamp could not tell a later write from it: then the next question reads the index
    /// again, whatever it finds, an index removed meanwhile among them.
    stamp: Option<Option<FileStamp>>,
    file: gix::index::File,
}

impl IndexFile {
    /// `repository`'s index as it is now; an empty one where it has none.
    fn read(repository: &gix::Repository) -> std::result::Result<Self, String> {
        let path = repository.index_path();
        // Stamped before it is read, so a write that lands meanwhile reads as a change at the
        // next question. A stamp that cannot tell a later write is not kept, so the next question
        // reads the index again.
        let stamp = FileStamp::of(&path);
        let stamp = stamp.as_ref().is_none_or(FileStamp::tells).then_some(stamp);
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
        Ok(Self { path, stamp, file })
    }
}

#[cfg(test)]
thread_local! {
    /// The index reads made on this thread, for the cases that check a question reads none.
    static INDEX_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What tells one index file from another and from itself rewritten: the file's identity (device
/// and inode on Unix; volume serial and file index on Windows), which every write of Git's
/// changes, and its modification time and size, which a write in place changes. All three come
/// from the one query a stat makes, so a question costs what it cost before. Git writes the index
/// to a new file and renames it over the old one, and ext4 gives a freed inode number to the next
/// new file at once: two writes within one step of the file system's clock can land the second on
/// the number of the file read, at its size and time. That needs the read taken within the step,
/// so on Linux such a stamp is not kept ([`FileStamp::tells`]) and nothing is held open. A tool
/// can also set a file's time back to the one read; on Unix the stamp holds the time the file last
/// changed, which the kernel sets at every write, rename and time set and no call sets, so such a
/// file still differs once the clock has left that time's step too. On macOS the inode number tells
/// them: APFS numbers each new file from its volume's next identifier, and HFS+ gives a number again
/// only once its 32-bit count has wrapped. On Windows the file index tells them: NTFS gives every
/// new file a file index of its own.
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
    std::time::UNIX_EPOCH
        + std::time::Duration::new(
            u64::try_from(now.tv_sec).unwrap_or(0),
            u32::try_from(now.tv_nsec).unwrap_or(0),
        )
}

/// Whether `now`, from the file clock, has left the step that gave `modified`, as its digits show
/// it ([`time_step`]).
#[cfg(target_os = "linux")]
fn past_step(modified: std::time::SystemTime, now: std::time::SystemTime) -> bool {
    now > modified + time_step(modified)
}

/// The step of the file system that gave `time`, read from its digits: a file system keeps times
/// to a step of its own (a nanosecond on ext4, 100 ns on NTFS, 10 ms on exFAT, one or two seconds
/// on FAT), so a time's nanoseconds are a multiple of it. The step is taken as the largest power of
/// ten that divides them, and two seconds for a whole second; a time of a finer file system that
/// ends in zeros by chance is left a little later.
#[cfg(target_os = "linux")]
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
    /// `path`'s stamp; `None` when it is absent or cannot be read.
    #[cfg(unix)]
    fn of(path: &Path) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            identity: (metadata.dev(), metadata.ino()),
            modified: metadata.modified().ok(),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            len: metadata.len(),
        })
    }

    /// Whether this stamp tells its file from every later one: once the file clock has left the
    /// steps that gave the file's time and the time it last changed, since a later write can get
    /// either only within it. A stamp taken within a step is not kept, so the next question reads
    /// the index again; that read, taken later, is kept.
    #[cfg(target_os = "linux")]
    fn tells(&self) -> bool {
        let now = file_clock();
        let (seconds, nanos) = self.changed;
        let changed = std::time::UNIX_EPOCH
            + std::time::Duration::new(
                u64::try_from(seconds).unwrap_or(0),
                u32::try_from(nanos).unwrap_or(0),
            );
        past_step(changed, now)
            && self
                .modified
                .is_some_and(|modified| past_step(modified, now))
    }

    /// Whether this stamp tells its file from every later one: the identity alone does here.
    #[cfg(not(target_os = "linux"))]
    fn tells(&self) -> bool {
        true
    }

    /// `path`'s stamp; `None` when it is absent or cannot be read. One open and one query, the
    /// ones `std::fs::metadata` makes, so a question costs a stat: the 128-bit file id would take
    /// a second query. The 64-bit index may repeat across folders on ReFS, where the time and size
    /// still tell a rewrite.
    #[cfg(windows)]
    fn of(path: &Path) -> Option<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
        };
        // No access, as `std::fs::metadata` opens it, and every share mode: Git may rename meanwhile.
        let file = std::fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: an open handle and a buffer of the type the call fills.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) } == 0 {
            return None;
        }
        let join = |high: u32, low: u32| (u64::from(high) << 32) | u64::from(low);
        Some(Self {
            identity: (
                u64::from(info.dwVolumeSerialNumber),
                join(info.nFileIndexHigh, info.nFileIndexLow),
            ),
            modified: join(
                info.ftLastWriteTime.dwHighDateTime,
                info.ftLastWriteTime.dwLowDateTime,
            ),
            len: join(info.nFileSizeHigh, info.nFileSizeLow),
        })
    }
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
    pub(crate) fn open(root: String) -> std::result::Result<Option<Self>, pool::GitReadError> {
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
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

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
    /// is kept (Linux; elsewhere every stamp is kept). It waits for a tick of the clock at most.
    fn settle(file: &Path) {
        #[cfg(target_os = "linux")]
        {
            let stamp = FileStamp::of(file).expect("a file");
            let bound = std::time::Instant::now() + Duration::from_secs(5);
            while !stamp.tells() && std::time::Instant::now() < bound {
                std::thread::yield_now();
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = file;
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
        assert_eq!(index_reads(), reads + 1, "read again once its time moves");
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
        let root = root.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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
        let linked = linked.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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
        let root = root.canonicalize().unwrap();
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

    /// A stamp taken while the file clock still reads the index's time, or the time it last
    /// changed, cannot tell a later write within that step from the file read: it is not kept, so
    /// the next question reads the index again. A stamp taken once the clock has left both is kept,
    /// and a question costs a stat.
    #[cfg(target_os = "linux")]
    #[test]
    fn keeps_a_stamp_once_the_file_clock_has_left_its_time() {
        let root = fixture("clock-step", &[(".gitignore", "e.py\n"), ("e.py", "")]);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".gitignore"]);
        let index = root.join(".git/index");
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
            reads + 2,
            "a time the clock still reads: read again at each question"
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
            let stamp = FileStamp::of(&file).expect("a file");
            let tells = stamp.tells();
            let (seconds, nanos) = stamp.changed;
            let changed = std::time::UNIX_EPOCH
                + Duration::new(
                    u64::try_from(seconds).unwrap(),
                    u32::try_from(nanos).unwrap(),
                );
            let within = !past_step(changed, file_clock());
            assert!(!(within && tells), "a stamp read within its change time's step is kept");
            within
        });
        assert!(checked, "the file clock left every change time's step before its check");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A time is left once the clock reads later than the next step its digits show: a nanosecond
    /// time a nanosecond on, a time of 100 ns steps (NTFS) or of 10 ms steps (exFAT) a step on, and
    /// a whole second, which a file system of one or two seconds a step gives, two seconds on.
    #[cfg(target_os = "linux")]
    #[test]
    fn says_when_the_file_clock_has_left_a_files_time() {
        let at = |seconds: u64, nanos: u32| std::time::UNIX_EPOCH + Duration::new(seconds, nanos);
        assert!(!past_step(at(10, 501), at(10, 502)));
        assert!(past_step(at(10, 501), at(10, 503)));
        assert!(!past_step(at(10, 234_567_800), at(10, 234_567_900)));
        assert!(past_step(at(10, 234_567_800), at(10, 234_567_901)));
        assert!(!past_step(at(10, 10_000_000), at(10, 20_000_000)));
        assert!(past_step(at(10, 10_000_000), at(10, 20_000_001)));
        assert!(!past_step(at(10, 0), at(12, 0)));
        assert!(past_step(at(10, 0), at(12, 1)));
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
