//! Whether Git ignores a worktree path, as `git check-ignore` answers it: every `.gitignore` from
//! the worktree root down to the path, `.git/info/exclude` and `core.excludesFile` (or the XDG
//! default), with negation and directory patterns; a path the index tracks is never ignored, nor
//! is a folder that holds a tracked path. Hosts that follow `.gitignore` skip what this ignores,
//! and can name the pattern that ignored a path, with its file and line (`git check-ignore -v`).
//!
//! The index is re-read whenever it changes on disk, so `git add` of an ignored file brings it back;
//! a `.gitignore` is read when the walk first enters its folder. A matcher says whether one opened
//! now would read the rules it read (`sources_unchanged`), so hosts can share one per checkout.

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
    /// reads that folder's `.gitignore` again, as a matcher opened now would.
    pub fn sources_unchanged(&mut self) -> bool {
        let now = match discover_worktree(&self.root) {
            Ok(Some((repository, worktree))) => Sources::read(&repository, &worktree),
            _ => None,
        };
        if now.is_none() || now != self.sources {
            return false;
        }
        self.leave_folders();
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
                pattern: matched.pattern.to_string(),
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

/// The repository around `root` and its worktree, resolved; `None` outside a Git worktree.
fn discover_worktree(
    root: &Path,
) -> std::result::Result<Option<(gix::Repository, PathBuf)>, String> {
    let repository = match gix::discover(root) {
        Ok(repository) => repository,
        Err(error) if crate::git_workspace::is_no_repository(&error) => return Ok(None),
        Err(error) => return Err(format!("cannot discover Git repository: {error}")),
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

/// A Git exclude stack for a host's walk below one root; see the module documentation.
#[napi]
pub struct GitIgnoreMatcher {
    inner: Mutex<GitIgnore>,
}

#[napi]
impl GitIgnoreMatcher {
    /// Whether Git ignores `relative_path` (to the root the matcher was opened for, `/`-separated);
    /// `is_directory` lets directory-only patterns (`build/`) match.
    #[napi(catch_unwind)]
    pub fn ignored(&self, relative_path: String, is_directory: bool) -> Result<bool> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "Git ignore matcher is poisoned"))?;
        inner
            .ignored(&relative_path, is_directory)
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    /// The pattern that makes Git ignore `relative_path` (as for `ignored`), with its file and
    /// line; null when Git does not ignore it.
    #[napi(catch_unwind)]
    pub fn ignored_by(
        &self,
        relative_path: String,
        is_directory: bool,
    ) -> Result<Option<GitIgnoreMatch>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "Git ignore matcher is poisoned"))?;
        inner
            .ignored_by(&relative_path, is_directory)
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    /// Whether a matcher opened now would follow the rules this one follows (the configuration
    /// Git reads, read again, and the files read whole as they were read); when it would, each
    /// folder's `.gitignore` is read again as the next path enters the folder.
    #[napi(catch_unwind)]
    pub fn sources_unchanged(&self) -> Result<bool> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "Git ignore matcher is poisoned"))?;
        Ok(inner.sources_unchanged())
    }
}

/// The Git exclude stack for paths below `root`; null when `root` is in no Git worktree.
#[napi(catch_unwind)]
pub fn open_git_ignore_matcher(root: String) -> Result<Option<GitIgnoreMatcher>> {
    GitIgnore::open(Path::new(&root))
        .map(|opened| {
            opened.map(|inner| GitIgnoreMatcher {
                inner: Mutex::new(inner),
            })
        })
        .map_err(|message| Error::new(Status::GenericFailure, message))
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
        assert_eq!(old.len(), contents.len() as u64, "the new bytes are as long");
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
