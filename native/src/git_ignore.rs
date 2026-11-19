//! Whether Git ignores a worktree path, as `git check-ignore` answers it: every `.gitignore` from
//! the worktree root down to the path, `.git/info/exclude` and `core.excludesFile` (or the XDG
//! default), with negation and directory patterns; a path the index tracks is never ignored, nor
//! is a folder that holds a tracked path. Hosts that follow `.gitignore` skip what this ignores,
//! and can name the pattern that ignored a path, with its file and line (`git check-ignore -v`).
//!
//! The index is re-read whenever it changes on disk, so `git add` of an ignored file brings it back;
//! a `.gitignore` is read when the walk first enters its folder. A matcher says whether one opened
//! now would read the rules it read (`sources_unchanged`), so hosts can share one per checkout;
//! one handed on as a matcher opened now reads the index from disk at its next question.
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
        let stack = exclude_stack(&repository)?;
        Ok(Some(Self {
            repository,
            stack,
            prefix,
            root,
            sources,
        }))
    }

    /// Whether a matcher opened at this root now would read the rules this one read: the
    /// configuration, read again through gix as an open reads it (every file Git reads, includes
    /// and their conditions among them), chooses the same excludes file and pattern case, and the
    /// root `.gitignore`, `info/exclude` and that excludes file are as this matcher read them.
    /// When it would, the stack leaves every folder it entered, so the next path below a folder
    /// reads that folder's `.gitignore` again; with `read_index_again`, for a host that hands the
    /// matcher on as one opened now, the matcher takes the repository and the stack that open
    /// makes, which read the index from disk.
    pub fn sources_unchanged(&mut self, read_index_again: bool) -> bool {
        let now = Self::sources_now(&self.root, read_index_again);
        self.take_sources(now)
    }

    /// The repository around `root` as an open now finds it, through the engine's opener, with
    /// the sources it reads the rules from and, when `fresh`, the stack it makes; `None` where
    /// that open fails or finds no worktree.
    fn sources_now(root: &Path, fresh: bool) -> Option<Found> {
        let (repository, worktree) = discover_worktree(root).ok()??;
        let sources = Sources::read(&repository, &worktree)?;
        let stack = if fresh {
            Some(exclude_stack(&repository).ok()?)
        } else {
            None
        };
        Some(Found {
            repository,
            sources,
            stack,
        })
    }

    /// Whether `now`, from [`Self::sources_now`], holds the sources this matcher read; when it
    /// does, the matcher goes on as `sources_unchanged` says.
    fn take_sources(&mut self, now: Option<Found>) -> bool {
        let Some(found) = now else {
            return false;
        };
        if self.sources.as_ref() != Some(&found.sources) {
            return false;
        }
        match found.stack {
            // Handed on as a matcher opened now: the index read before and the stack made from it,
            // which names each `.gitignore` the index marks skip-worktree by the blob it held then,
            // go with the old repository, so an index rewritten since, even at the very time it was
            // read, is not taken for them.
            Some(stack) => {
                self.stack = stack;
                self.repository = found.repository;
            }
            None => self.leave_folders(),
        }
        true
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
        let index = self
            .repository
            .index_or_empty()
            .map_err(|error| format!("cannot read Git index: {error}"))?;
        if tracked(&index, path.as_bytes().as_bstr(), is_directory) {
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

fn exclude_stack(
    repository: &gix::Repository,
) -> std::result::Result<gix::worktree::Stack, String> {
    let index = repository
        .index_or_empty()
        .map_err(|error| format!("cannot read Git index: {error}"))?;
    Ok(repository
        .excludes(
            &index,
            None,
            gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        )
        .map_err(|error| format!("cannot read Git excludes: {error}"))?
        .detach())
}

/// What a check of a matcher's sources found as an open now finds it: the repository, the sources
/// and, for a matcher handed on as one opened now, the stack that open makes.
struct Found {
    repository: gix::Repository,
    sources: Sources,
    stack: Option<gix::worktree::Stack>,
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
    /// folder's `.gitignore` is read again as the next path enters the folder, and, unless
    /// `read_index_again` is false (for a holder that asks again), the index at the next question.
    #[napi(catch_unwind)]
    pub fn sources_unchanged(&self, read_index_again: Option<bool>) -> Result<bool> {
        let root = self.read(|inner| Ok(inner.root.clone()))?;
        let fresh = read_index_again.unwrap_or(true);
        // Opened again as an open is: a disk that stops answering costs the check, not the host's
        // thread, and while an open of this root gives no answer the check fails at once.
        let now = pool::run(
            "checking Git's ignore rules",
            &self.opened_as,
            GIT_READ_TIME_LIMIT,
            move || GitIgnore::sources_now(&root, fresh),
        )
        .map_err(|error| Error::new(Status::GenericFailure, error.message))?;
        self.read(|inner| Ok(inner.take_sources(now)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn git(directory: &Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
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
        assert!(ignore.sources_unchanged(true), "nothing changed");
        git(&root, &["config", "branch.main.remote", "origin"]);
        assert!(
            ignore.sources_unchanged(true),
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
            !ignore.sources_unchanged(true),
            "an included file that names an excludes file"
        );
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(ignore.ignored("a.py", false).unwrap());
        assert!(ignore.sources_unchanged(true));
        std::fs::write(root.join("named-a"), "b.py\nc.py\n").unwrap();
        assert!(!ignore.sources_unchanged(true), "the named file changed");
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        rewrite_keeping_size_and_time(&root.join("named-a"), "d.py\ne.py\n");
        assert!(
            !ignore.sources_unchanged(true),
            "the named file rewritten at its size and write time"
        );
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        turn_ignore_case(&root);
        assert!(!ignore.sources_unchanged(true), "the pattern case changed");

        // Each folder's .gitignore is read again once the sources hold.
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(!ignore.ignored("pkg/x.py", false).unwrap());
        std::fs::write(root.join("pkg/.gitignore"), "x.py\n").unwrap();
        assert!(ignore.sources_unchanged(true));
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
        assert!(ignore.sources_unchanged(true), "nothing changed");
        std::fs::write(root.join(".gitignore"), "gen/\na.py\n").unwrap();
        assert!(
            !ignore.sources_unchanged(true),
            "the root .gitignore changed"
        );
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(ignore.ignored("a.py", false).unwrap());
        assert!(ignore.sources_unchanged(true));
        std::fs::write(root.join(".git/info/exclude"), "b.py\n").unwrap();
        assert!(!ignore.sources_unchanged(true), "info/exclude changed");
        let mut ignore = GitIgnore::open(&root).unwrap().unwrap();
        assert!(ignore.ignored("b.py", false).unwrap());
        assert!(ignore.sources_unchanged(true));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A matcher whose sources hold is handed on as one opened then, with the index read again at
    /// its next question: an index rewritten at the very time this one read, as two writes within
    /// one tick of the file system's clock leave it, is not taken for the old one. A holder that
    /// asks again keeps the index it read, as a matcher of its own would.
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
        assert!(
            ignore.sources_unchanged(false),
            "the rules' sources are as read"
        );
        assert!(
            ignore.ignored("gen/x.py", false).unwrap(),
            "a holder that asks again keeps the index it read"
        );
        assert!(ignore.sources_unchanged(true));
        assert!(
            !ignore.ignored("gen/x.py", false).unwrap(),
            "a force-added file is tracked, as a matcher opened now reads it"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A `.gitignore` the index marks skip-worktree and the worktree lacks is read from the blob
    /// the index names. A matcher handed on as one opened now reads the blob the index names
    /// then, as that open does; a holder that asks again keeps the one it read, as a matcher of
    /// its own would.
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
        assert!(ignore.sources_unchanged(false));
        assert_eq!(answers(&mut ignore), (true, false), "a holder keeps it");
        assert!(ignore.sources_unchanged(true));
        assert_eq!(answers(&mut ignore), (false, true), "handed on");
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
        assert!(matcher.sources_unchanged(Some(true)).unwrap());
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
        let refused = matcher.sources_unchanged(Some(true)).unwrap_err();
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
        assert!(ignore.sources_unchanged(true), "nothing changed");
        std::fs::write(root.join(".git/info/exclude"), "c.py\n").unwrap();
        assert!(
            !ignore.sources_unchanged(true),
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
        assert!(ignore.sources_unchanged(true), "nothing changed");
        git(&root, &["config", "gitoxide.parsePrecious", "true"]);
        assert!(
            !ignore.sources_unchanged(true),
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
        assert!(ignore.sources_unchanged(true), "nothing changed");
        let other = config_path(&root.join("other"));
        git(&root, &["config", "core.worktree", &other]);
        assert!(
            !ignore.sources_unchanged(true),
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
}
