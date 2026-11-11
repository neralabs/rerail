//! Whether Git ignores a worktree path, as `git check-ignore` answers it: every `.gitignore` from
//! the worktree root down to the path, `.git/info/exclude` and `core.excludesFile` (or the XDG
//! default), with negation and directory patterns; a path the index tracks is never ignored, nor
//! is a folder that holds a tracked path. Hosts that follow `.gitignore` skip what this ignores,
//! and can name the pattern that ignored a path, with its file and line (`git check-ignore -v`).
//!
//! The index is re-read whenever it changes on disk, so `git add` of an ignored file brings it back;
//! a `.gitignore` is read when the walk first enters its folder, and the top one, with
//! `.git/info/exclude` and `core.excludesFile`, again at each `reread`. A walk asks once per folder
//! (`ignored_entries`), so the index is looked at once per folder, not once per file.

use gix::bstr::{BStr, ByteSlice};
use gix::index::entry::Mode;
use napi::{Error, Result, Status};
use napi_derive::napi;
use std::path::Path;
use std::sync::Mutex;

pub struct GitIgnore {
    repository: gix::Repository,
    stack: gix::worktree::Stack,
    /// Where the opened root sits below the worktree root: empty, or `/`-joined components.
    prefix: Vec<String>,
}

impl GitIgnore {
    /// The exclude stack of the Git worktree around `root`; `None` when `root` is in no Git
    /// worktree (a plain folder or a bare repository), where nothing is ignored.
    pub fn open(root: &Path) -> std::result::Result<Option<Self>, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("cannot resolve {}: {error}", root.display()))?;
        let repository = match gix::discover(&root) {
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
        let Ok(below) = root.strip_prefix(&worktree) else {
            return Ok(None);
        };
        let prefix = below
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect();
        let index = repository
            .index_or_empty()
            .map_err(|error| format!("cannot read Git index: {error}"))?;
        let stack = exclude_stack(&repository, &index)?;
        Ok(Some(Self {
            repository,
            stack,
            prefix,
        }))
    }

    /// Reads Git's ignore files again: `.git/info/exclude` and `core.excludesFile` now, and the
    /// top `.gitignore` at the next question. The stack reads those once and keeps them, where a
    /// `.gitignore` below the top is read each time a question goes into its folder again; a host
    /// rereads as it starts a walk, so each walk follows the files as they are.
    pub fn reread(&mut self) -> std::result::Result<(), String> {
        let index = self.index()?;
        self.stack = exclude_stack(&self.repository, &index)?;
        Ok(())
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
        let index = self.index()?;
        self.matching(&index, relative, is_directory)
    }

    /// Which entries of the folder `folder` Git ignores: `ignored` of `folder/names[i]`, a folder
    /// where `directories[i]` says so. `folder` is relative to the opened root as for `ignored`
    /// (empty for the root), and each name is one path component. The index is looked at once for
    /// all of them, where `ignored` looks at it, a file system call, for each path; and the exclude
    /// stack, positioned at the folder for the first entry, stays there for the rest.
    pub fn ignored_entries(
        &mut self,
        folder: &str,
        names: &[String],
        directories: &[bool],
    ) -> std::result::Result<Vec<bool>, String> {
        if names.len() != directories.len() {
            return Err(format!(
                "{} entry names but {} folder marks",
                names.len(),
                directories.len()
            ));
        }
        let index = self.index()?;
        names
            .iter()
            .zip(directories)
            .map(|(name, &is_directory)| {
                let relative = if folder.is_empty() {
                    name.clone()
                } else {
                    format!("{folder}/{name}")
                };
                Ok(self.matching(&index, &relative, is_directory)?.is_some())
            })
            .collect()
    }

    /// The index as it is on disk now: re-read once it changed, so `git add` of an ignored file
    /// brings it back.
    fn index(&self) -> std::result::Result<gix::worktree::Index, String> {
        self.repository
            .index_or_empty()
            .map_err(|error| format!("cannot read Git index: {error}"))
    }

    /// `ignored_by` against `index`.
    fn matching(
        &mut self,
        index: &gix::index::State,
        relative: &str,
        is_directory: bool,
    ) -> std::result::Result<Option<GitIgnoreMatch>, String> {
        let Some(path) = self.worktree_path(relative) else {
            return Ok(None);
        };
        if path.is_empty() {
            return Ok(None);
        }
        if tracked(index, path.as_bytes().as_bstr(), is_directory) {
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

/// The exclude stack of `repository`'s worktree: `.git/info/exclude` and `core.excludesFile` read
/// now, each `.gitignore` as a question first goes into its folder (from the worktree, else from
/// `index`).
fn exclude_stack(
    repository: &gix::Repository,
    index: &gix::index::State,
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

    /// Reads Git's ignore files again (`.git/info/exclude`, `core.excludesFile`, the top
    /// `.gitignore`), which the matcher otherwise keeps as it first read them. A walk rereads as
    /// it starts.
    #[napi(catch_unwind)]
    pub fn reread(&self) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "Git ignore matcher is poisoned"))?;
        inner
            .reread()
            .map_err(|message| Error::new(Status::GenericFailure, message))
    }

    /// Which entries of the folder `folder` (relative to the matcher's root, `/`-separated; empty
    /// for the root) Git ignores: `ignored` of each of `names`, a folder where `directories` says
    /// so, asked with one look at the index for all of them. A walk asks this once per folder.
    #[napi(catch_unwind)]
    pub fn ignored_entries(
        &self,
        folder: String,
        names: Vec<String>,
        directories: Vec<bool>,
    ) -> Result<Vec<bool>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| Error::new(Status::GenericFailure, "Git ignore matcher is poisoned"))?;
        inner
            .ignored_entries(&folder, &names, &directories)
            .map_err(|message| Error::new(Status::GenericFailure, message))
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
    fn answers_a_folders_entries_as_it_answers_each_path() {
        let root = fixture(
            "entries",
            &[
                ("tracked/gen.py", "tracked\n"),
                (".gitignore", "gen/\n*.log\n!keep.log\n"),
                ("api/.gitignore", "e.py\n"),
                ("api/d.py", ""),
                ("api/e.py", ""),
                ("api/gen/c.py", ""),
                ("api/build/x.py", ""),
                ("keep.log", ""),
                ("drop.log", ""),
                ("local.py", ""),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", "tracked/gen.py"]);
        std::fs::write(
            root.join(".gitignore"),
            "gen/\n*.log\n!keep.log\ntracked/\nbuild/\n",
        )
        .unwrap();
        std::fs::write(root.join(".git/info/exclude"), "local.py\n").unwrap();
        std::fs::write(root.join("tracked/fresh.py"), "").unwrap();

        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        for folder in ["", "api", "api/gen", "api/build", "tracked"] {
            let mut names = Vec::new();
            let mut directories = Vec::new();
            for entry in std::fs::read_dir(root.join(folder)).unwrap() {
                let entry = entry.unwrap();
                names.push(entry.file_name().to_string_lossy().into_owned());
                directories.push(entry.file_type().unwrap().is_dir());
            }
            let one_by_one: Vec<bool> = names
                .iter()
                .zip(&directories)
                .map(|(name, &directory)| {
                    let path = if folder.is_empty() {
                        name.clone()
                    } else {
                        format!("{folder}/{name}")
                    };
                    ignore.ignored(&path, directory).unwrap()
                })
                .collect();
            assert_eq!(
                ignore
                    .ignored_entries(folder, &names, &directories)
                    .unwrap(),
                one_by_one,
                "entries of {folder:?}: {names:?}"
            );
        }
        let names = ["drop.log", "keep.log", "gen", "api", "local.py"].map(String::from);
        assert_eq!(
            ignore
                .ignored_entries("", &names, &[false, false, true, true, false])
                .unwrap(),
            [true, false, true, false, true]
        );
        assert_eq!(
            ignore
                .ignored_entries(
                    "api",
                    &["gen".to_owned(), "build".to_owned()],
                    &[false, true]
                )
                .unwrap(),
            [false, true],
            "a directory-only pattern matches an entry marked a folder only"
        );
        assert!(
            ignore
                .ignored_entries("", &names, &[false])
                .unwrap_err()
                .contains("5 entry names but 1 folder marks")
        );

        git(&root, &["add", "-f", "api/e.py"]);
        assert_eq!(
            ignore
                .ignored_entries("api", &["e.py".to_owned()], &[false])
                .unwrap(),
            [false],
            "the index is re-read once it changes"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rereads_the_top_gitignore_and_the_exclude_files_when_asked() {
        let root = fixture(
            "reread",
            &[
                (".gitignore", "*.log\n"),
                ("api/.gitignore", "\n"),
                ("top.py", ""),
                ("local.py", ""),
                ("global.py", ""),
                ("api/nested.py", ""),
            ],
        );
        git(&root, &["init", "--initial-branch=main"]);
        let excludes = root.with_extension("excludes");
        std::fs::write(&excludes, "\n").unwrap();
        git(
            &root,
            &["config", "core.excludesFile", &excludes.to_string_lossy()],
        );
        let mut ignore = GitIgnore::open(&root).unwrap().expect("a Git worktree");
        let names = ["top.py", "local.py", "global.py"].map(String::from);
        let ask = |ignore: &mut GitIgnore| {
            let mut answers = ignore
                .ignored_entries("", &names, &[false, false, false])
                .unwrap();
            answers.push(ignore.ignored("api/nested.py", false).unwrap());
            answers
        };
        assert_eq!(ask(&mut ignore), [false, false, false, false]);

        std::fs::write(root.join(".gitignore"), "*.log\ntop.py\n").unwrap();
        std::fs::write(root.join(".git/info/exclude"), "local.py\n").unwrap();
        std::fs::write(&excludes, "global.py\n").unwrap();
        std::fs::write(root.join("api/.gitignore"), "nested.py\n").unwrap();
        assert_eq!(
            ask(&mut ignore),
            [false, false, false, true],
            "a nested .gitignore is read as its folder is entered again; the rest is kept"
        );
        ignore.reread().unwrap();
        assert_eq!(ask(&mut ignore), [true, true, true, true]);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_file(excludes).unwrap();
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
