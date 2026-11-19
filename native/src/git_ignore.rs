//! Whether Git ignores a worktree path, as `git check-ignore` answers it: every `.gitignore` from
//! the worktree root down to the path, `.git/info/exclude` and `core.excludesFile` (or the XDG
//! default), with negation and directory patterns; a path the index tracks is never ignored, nor
//! is a folder that holds a tracked path. Hosts that follow `.gitignore` skip what this ignores,
//! and can name the pattern that ignored a path, with its file and line (`git check-ignore -v`).
//!
//! The index is re-read whenever its file changes on disk, so `git add` of an ignored file brings it
//! back; a `.gitignore` is read when the walk first enters its folder.
//!
//! The repository opens through the engine's one opener (`git_access::opener`), so a repository
//! Git refuses has no matcher. Opening runs on a thread of its own under the Git read time limit; questions
//! are answered on the caller's thread, since a walk asks one or two per file. A question's own
//! disk reads are a stat of the index and the `.gitignore` of a folder the walk has just listed,
//! so a disk that stops answering holds the walk before it holds the matcher.

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
        let repository = match opener::discover(&root) {
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
        let Ok(below) = root.strip_prefix(&worktree) else {
            return Ok(None);
        };
        let prefix = below
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect();
        let (index, stack) = index_and_exclude_stack(&repository)?;
        Ok(Some(Self {
            repository,
            stack,
            prefix,
            index,
        }))
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
        self.index.refresh(&self.repository)?;
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
    let stack = repository
        .excludes(
            &index.file,
            None,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        )
        .map_err(|error| format!("cannot read Git excludes: {error}"))?
        .detach();
    Ok((index, stack))
}

/// The index as a matcher last read it, with its file's stamp then. gix's shared index is read
/// again only once the file's modification time is later, but Git writes the index to a new file
/// and renames it over the old one, and a second write within one step of the file system's clock
/// keeps the first one's time: the stamp tells that new file from the one read.
struct IndexFile {
    path: PathBuf,
    stamp: Option<FileStamp>,
    file: gix::index::File,
}

impl IndexFile {
    /// `repository`'s index as it is now; an empty one where it has none.
    fn read(repository: &gix::Repository) -> std::result::Result<Self, String> {
        let path = repository.index_path();
        // Stamped before it is read, so a write that lands meanwhile reads as a change at the
        // next question.
        let stamp = FileStamp::of(&path);
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

    /// Reads the index again once its file's stamp differs from the one it was read with.
    fn refresh(&mut self, repository: &gix::Repository) -> std::result::Result<(), String> {
        if FileStamp::of(&self.path) != self.stamp {
            *self = Self::read(repository)?;
        }
        Ok(())
    }
}

/// What tells one index file from another and from itself rewritten: the file's identity (device
/// and inode on Unix; volume and file id on Windows, as the tree watch reads a folder's), which
/// every write of Git's changes, and its modification time and size, which a write in place
/// changes. The change time is left out: only a write in place that keeps the size and sets the
/// time back would need it, and Git never writes the index in place.
#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    identity: (u64, u128),
    modified: Option<std::time::SystemTime>,
    len: u64,
}

impl FileStamp {
    /// `path`'s stamp; `None` when it is absent or cannot be read.
    #[cfg(unix)]
    fn of(path: &Path) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            identity: (metadata.dev(), u128::from(metadata.ino())),
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }

    /// `path`'s stamp; `None` when it is absent or cannot be read.
    #[cfg(windows)]
    fn of(path: &Path) -> Option<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
        };
        // No access, as `std::fs::metadata` opens it, and every share mode: Git may rename meanwhile.
        let file = std::fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .ok()?;
        let metadata = file.metadata().ok()?;
        let mut info = FILE_ID_INFO::default();
        // SAFETY: an open handle and a buffer of the size passed.
        let identified = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&raw mut info).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        } != 0;
        identified.then(|| Self {
            identity: (
                info.VolumeSerialNumber,
                u128::from_le_bytes(info.FileId.Identifier),
            ),
            modified: metadata.modified().ok(),
            len: metadata.len(),
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
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        assert!(ignore.ignored("e.py", false).unwrap());

        // Bytes no index holds, in the same file at the same size and time: only a read sees them.
        let time = modified(&index);
        let len = std::fs::metadata(&index).unwrap().len();
        std::fs::write(&index, vec![0; usize::try_from(len).unwrap()]).unwrap();
        set_modified(&index, time);
        assert!(
            ignore.ignored("e.py", false).unwrap(),
            "answered from the index read before, with one stat of its file"
        );
        set_modified(&index, time + Duration::from_secs(1));
        assert!(
            ignore.ignored("e.py", false).is_err(),
            "read again once its time moves, and said to be unreadable"
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

    #[test]
    fn a_folder_outside_git_has_no_matcher() {
        let root = fixture("plain", &[(".gitignore", "*.py\n"), ("a.py", "")]);
        // A temporary directory may sit inside a checkout on some hosts; only assert where it does not.
        if gix::discover(&root).is_err() {
            assert!(GitIgnore::open(&root).unwrap().is_none());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
