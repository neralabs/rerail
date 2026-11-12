//! The opener answers as Git does or not at all, starts no Git, and survives damaged repositories.
//!
//! Tests that need an environment of their own (no Git on `PATH`, a global config file, another
//! owner) run again in a child process of this test binary ([`in_child`]), since the environment
//! and the installed setup belong to the whole process.

use super::opener::{self, OpenError};
use super::pool;
use super::setup::{self, GitSetup};
use crate::git_ignore::{GitIgnore, GitIgnoreMatch, GitIgnoreMatcher};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

const CHILD: &str = "RERAIL_GIT_ACCESS_CHILD";
const FIXTURE: &str = "RERAIL_GIT_ACCESS_FIXTURE";

fn temporary(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "rerail-git-access-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

fn git_output(directory: &Path, arguments: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new("git");
    command.arg("-C").arg(directory).args(arguments);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = git_output(directory, arguments, &[]);
    assert!(
        output.status.success(),
        "git {} failed:\n{}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(root: &Path, files: &[(&str, &[u8])]) {
    for (path, contents) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

/// A repository at `root` with one commit of `files`.
fn repository_at(root: &Path, files: &[(&str, &[u8])]) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(root, &["config", "user.email", "rerail-tests@example.com"]);
    git(root, &["config", "user.name", "Rerail tests"]);
    git(root, &["config", "core.autocrlf", "false"]);
    write(root, files);
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "initial"]);
    root.to_path_buf()
}

/// Whether this process is the child `name` was run in.
fn in_child(name: &str) -> bool {
    std::env::var(CHILD).is_ok_and(|child| child == name)
}

/// Runs the test `name` of this module again in a process of its own with `env` (a `None` value
/// removes the variable), and fails unless it passes there.
fn run_child(name: &str, env: &[(&str, Option<OsString>)]) {
    let test = format!(
        "{}::{name}",
        module_path!().trim_start_matches("rerail_engine::")
    );
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([test.as_str(), "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, name);
    for (key, value) in env {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "child {name} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture() -> PathBuf {
    PathBuf::from(std::env::var_os(FIXTURE).expect("the parent names the fixture"))
}

/// The variables that move which Git runs or which config it reads, removed so a child sees only
/// what its test sets.
fn clean_env() -> Vec<(&'static str, Option<OsString>)> {
    [
        "EXEPATH",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_ATTR_NOSYSTEM",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_TRACE2_EVENT",
        "XDG_CONFIG_HOME",
    ]
    .into_iter()
    .map(|name| (name, None))
    .collect()
}

#[test]
fn the_report_names_the_system_and_installation_files() {
    let listing = "unknown\0file:/apple/gitconfig\0credential.helper\0system\0file:/etc/gitconfig\0core.autocrlf\0system\0file:/etc/gitconfig\0include.path\0";
    let setup = GitSetup::from_report(
        "git version 2.53.0\n",
        Some("/etc/gitconfig\n"),
        Some(listing),
    );
    assert_eq!(setup.version, "git version 2.53.0");
    assert_eq!(setup.system_config.as_deref(), Some(Path::new("/etc/gitconfig")));
    assert_eq!(
        setup.installation_config.as_deref(),
        Some(Path::new("/apple/gitconfig"))
    );
    assert_eq!(
        setup.system_attributes().as_deref(),
        Some(Path::new("/etc/gitattributes"))
    );

    // A Git before 2.42 has no `git var GIT_CONFIG_SYSTEM`: the listing names the file.
    let old = GitSetup::from_report("git version 2.39.5", None, Some(listing));
    assert_eq!(old.system_config.as_deref(), Some(Path::new("/etc/gitconfig")));

    // Git for Windows lists its system file first, with the system scope: no installation file.
    let windows = GitSetup::from_report(
        "git version 2.56.0.windows.1",
        Some("C:/Program Files/Git/etc/gitconfig"),
        Some("system\0file:C:/Program Files/Git/etc/gitconfig\0core.autocrlf\0"),
    );
    assert_eq!(windows.installation_config, None);
    assert_eq!(
        windows.system_attributes().as_deref(),
        Some(Path::new("C:/Program Files/Git/etc/gitattributes"))
    );

    let nothing = GitSetup::from_report("", None, None);
    assert_eq!(nothing.system_config, None);
    assert_eq!(nothing.installation_config, None);
}

/// On a machine whose `PATH` holds no Git, the opener reads the system file Rerail's Git named,
/// where gix alone finds none on Windows without starting a Git.
#[test]
fn the_named_system_file_is_read_with_no_git_on_path() {
    const NAME: &str = "the_named_system_file_is_read_with_no_git_on_path";
    if in_child(NAME) {
        let root = fixture();
        setup::install(Arc::new(GitSetup::from_report(
            "git version 2.53.0",
            Some(&root.join("system-gitconfig").to_string_lossy()),
            None,
        )));
        let repository = opener::discover(&root.join("work")).expect("the opener opens it");
        let config = repository.config_snapshot();
        let value = |key: &str| config.string(key).map(|value| value.to_string());
        assert_eq!(value("core.autocrlf").as_deref(), Some("true"));
        assert_eq!(value("rerail.source").as_deref(), Some("named-system-file"));
        assert!(crate::git_workspace::discover(&root.join("work")).is_ok());
        assert!(GitIgnore::open(&root.join("work")).unwrap().is_some());
        return;
    }
    let root = temporary("no-git-on-path");
    let work = repository_at(&root.join("work"), &[("a.py", b"a\n")]);
    // The repository's own value would win over the system file's.
    git(&work, &["config", "--unset", "core.autocrlf"]);
    std::fs::write(
        root.join("system-gitconfig"),
        "[core]\n\tautocrlf = true\n[rerail]\n\tsource = named-system-file\n",
    )
    .unwrap();
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(root.join("global-gitconfig"), "").unwrap();
    let mut env = clean_env();
    // No Git where gix-path looks for one: `PATH` and the program folders it searches on Windows.
    let mut path = OsString::from(&empty);
    if let Some(system) = std::env::var_os("SystemRoot") {
        path.push(";");
        path.push(PathBuf::from(system).join("System32"));
    }
    env.extend([
        ("PATH", Some(path)),
        ("ProgramFiles", Some(empty.clone().into())),
        ("ProgramW6432", Some(empty.clone().into())),
        ("ProgramFiles(x86)", Some(empty.clone().into())),
        ("LocalAppData", Some(empty.clone().into())),
        ("GIT_CONFIG_GLOBAL", Some(root.join("global-gitconfig").into())),
        (FIXTURE, Some(root.clone().into())),
    ]);
    run_child(NAME, &env);
    std::fs::remove_dir_all(root).unwrap();
}

/// Opens the fixture's checkout as the ignore matcher and the workspace discovery do, through
/// the Git read pool, and changes a file under a filter driver so status converts it.
fn read_fixture_as_the_engine_does(work: &Path) {
    let matcher = GitIgnoreMatcher::open(work.to_string_lossy().into_owned())
        .expect("the matcher opens")
        .expect("a Git worktree");
    assert!(matcher.ignored("drop.log".to_owned(), false).unwrap());
    let root = work.to_path_buf();
    pool::run("discovery", pool::GIT_READ_TIME_LIMIT, move || {
        crate::git_workspace::discover(&root)
    })
    .unwrap()
    .expect("discovery answers");
}

/// The Git processes the engine starts while it opens a checkout and reads it: 0 through the
/// opener. gix alone starts one on Windows to find the system config, one for the system
/// attributes, and on every system runs the filter driver a changed file names.
#[test]
fn reading_a_checkout_starts_no_git() {
    const NAME: &str = "reading_a_checkout_starts_no_git";
    if in_child(NAME) {
        let root = fixture();
        if std::env::var_os("RERAIL_GIT_ACCESS_TODAY").is_some() {
            let repository = gix::discover(root.join("work")).unwrap();
            let index = repository.index_or_empty().unwrap();
            let _ = repository
                .attributes_only(&index, gix::worktree::stack::state::attributes::Source::IdMapping)
                .unwrap();
            for item in repository
                .status(gix::progress::Discard)
                .unwrap()
                .into_iter(Vec::<gix::bstr::BString>::new())
                .unwrap()
            {
                let _ = item;
            }
            return;
        }
        setup::install(Arc::new(GitSetup::from_report(
            "git version 2.53.0",
            Some(&root.join("system-gitconfig").to_string_lossy()),
            None,
        )));
        read_fixture_as_the_engine_does(&root.join("work"));
        return;
    }
    let root = temporary("starts");
    let work = repository_at(
        &root.join("work"),
        &[
            (".gitignore", b"*.log\n"),
            (".gitattributes", b"*.txt filter=probe\n"),
            ("notes.txt", b"one\n"),
        ],
    );
    // A clean filter that is a Git: if anything runs it, the trace counts it.
    git(&work, &["config", "filter.probe.clean", "git hash-object --stdin"]);
    std::fs::write(work.join("notes.txt"), b"two\n").unwrap();
    std::fs::write(work.join("drop.log"), b"").unwrap();
    std::fs::write(root.join("system-gitconfig"), "[core]\n\tautocrlf = false\n").unwrap();
    let count = |today: bool| {
        let trace = root.join(if today { "trace-today" } else { "trace" });
        std::fs::create_dir_all(&trace).unwrap();
        let mut env = clean_env();
        env.extend([
            ("GIT_TRACE2_EVENT", Some(trace.clone().into())),
            (FIXTURE, Some(root.clone().into())),
        ]);
        if today {
            env.push(("RERAIL_GIT_ACCESS_TODAY", Some("1".into())));
        }
        run_child(NAME, &env);
        std::fs::read_dir(&trace).unwrap().count()
    };
    assert_eq!(count(false), 0, "Git processes started through the opener");
    let today = count(true);
    assert!(
        today >= if cfg!(windows) { 3 } else { 1 },
        "Git processes gix starts on its own: {today}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A repository another user owns is refused by Git unless `safe.directory` names it, and the
/// opener gives no answer there either; once named, both answer.
#[test]
fn another_owners_repository_gets_no_answer_outside_safe_directory() {
    const NAME: &str = "another_owners_repository_gets_no_answer_outside_safe_directory";
    if in_child(NAME) {
        opener::ASSUME_ANOTHER_OWNER.store(true, std::sync::atomic::Ordering::Relaxed);
        let root = fixture();
        let work = root.join("work");
        let safe = std::env::var_os("RERAIL_GIT_ACCESS_SAFE").is_some();
        let opened = opener::discover(&work);
        let matcher = GitIgnoreMatcher::open(work.to_string_lossy().into_owned());
        let discovered = crate::git_workspace::discover(&work);
        if safe {
            assert!(opened.is_ok(), "{:?}", opened.err());
            assert!(matcher.unwrap().is_some());
            assert!(discovered.is_ok());
        } else {
            assert!(matches!(opened, Err(OpenError::Failed(_))));
            assert!(matcher.is_err());
            assert!(matches!(
                discovered,
                Err(crate::git_workspace::DiscoveryError::Failed(_))
            ));
        }
        return;
    }
    let root = temporary("owner");
    let work = repository_at(&root.join("work"), &[(".gitignore", b"*.log\n")]);
    let global = root.join("global-gitconfig");
    let check = |safe: bool| {
        std::fs::write(&global, if safe { "[safe]\n\tdirectory = *\n" } else { "" }).unwrap();
        let global_value = global.to_string_lossy().into_owned();
        let git = git_output(
            &work,
            &["check-ignore", "drop.log"],
            &[
                ("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1"),
                ("GIT_CONFIG_GLOBAL", &global_value),
            ],
        );
        let mut env = clean_env();
        env.extend([
            ("GIT_CONFIG_GLOBAL", Some(global.clone().into())),
            (FIXTURE, Some(root.clone().into())),
        ]);
        if safe {
            env.push(("RERAIL_GIT_ACCESS_SAFE", Some("1".into())));
        }
        run_child(NAME, &env);
        git.status.code()
    };
    assert_eq!(check(false), Some(128), "Git refuses another user's repository");
    assert_eq!(check(true), Some(0), "Git answers once safe.directory names it");
    std::fs::remove_dir_all(root).unwrap();
}

/// What the matcher answers for `path` against what `git check-ignore -v -n` answers: `None` for
/// a path neither ignores, else the source file (absolute), the line and the pattern.
type IgnoreAnswer = Option<(PathBuf, u32, String)>;

fn git_check_ignore(work: &Path, paths: &[&str]) -> Vec<IgnoreAnswer> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(work)
        .args(["check-ignore", "-v", "-n", "--stdin", "-z"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = paths.iter().map(|path| format!("{path}\0")).collect::<String>();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success() || output.status.code() == Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    let fields = text.split('\0').collect::<Vec<_>>();
    fields
        .chunks_exact(4)
        .map(|record| {
            let (source, line, pattern) = (record[0], record[1], record[2]);
            if source.is_empty() || pattern.starts_with('!') {
                return None;
            }
            let source = Path::new(source);
            let source = if source.is_absolute() {
                source.to_path_buf()
            } else {
                work.join(source)
            };
            Some((comparable(&source), line.parse().unwrap(), pattern.to_owned()))
        })
        .collect()
}

fn comparable(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn matcher_answers(work: &Path, paths: &[&str]) -> Vec<IgnoreAnswer> {
    let mut matcher = GitIgnore::open(work).unwrap().expect("a Git worktree");
    paths
        .iter()
        .map(|path| {
            let is_directory = work.join(path).is_dir();
            matcher
                .ignored_by(path, is_directory)
                .unwrap()
                .map(|GitIgnoreMatch { source, line, pattern }| {
                    (comparable(Path::new(&source.unwrap())), line, pattern)
                })
        })
        .collect()
}

/// The ignore cases the matcher is compared on, each an existing file or folder.
const IGNORE_CASES: &[&str] = &[
    "a.log",
    "keep.log",
    "sub/keep.log",
    "build",
    "build/out.js",
    "src/build",
    "src/build/x.js",
    "root-only.txt",
    "src/root-only.txt",
    "a/deep/b/x.tmp",
    "deep/x.tmp",
    "#hash.txt",
    "!bang.txt",
    "spaced.txt",
    "doc/a.md",
    "doc/sub/b.md",
    "a.c",
    "c.c",
    "tracked-dir",
    "tracked-dir/t.txt",
    "tracked-dir/u.txt",
    "api/e.py",
    "api/d.py",
    "api/local-only.py",
    "api/sub/local-only.py",
    "api/api-keep.log",
    "local.py",
    "x.swp",
    "api/x.swp",
    "global.bak",
    "api/sub",
];

fn ignore_fixture(root: &Path) -> PathBuf {
    let work = repository_at(&root.join("work"), &[("tracked-dir/t.txt", b"t\n")]);
    write(
        &work,
        &[
            (
                ".gitignore",
                b"# comment\n*.log\n!keep.log\nbuild/\n/root-only.txt\n**/deep/**/x.tmp\n\\#hash.txt\n\\!bang.txt\nspaced.txt   \ndoc/*.md\n[ab].c\ntracked-dir/\n",
            ),
            ("api/.gitignore", b"e.py\n/local-only.py\n!api-keep.log\n"),
            (".git/info/exclude", b"# local\n\nlocal.py\n"),
        ],
    );
    std::fs::write(root.join("excludes"), b"*.swp\n").unwrap();
    git(&work, &["config", "core.excludesFile", &root.join("excludes").to_string_lossy()]);
    for path in IGNORE_CASES {
        let path = work.join(path);
        if path.extension().is_some() || path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(['#', '!'])) {
            if !path.exists() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, b"").unwrap();
            }
        } else {
            std::fs::create_dir_all(&path).unwrap();
        }
    }
    work
}

/// The matcher names the pattern `git check-ignore -v` names, from the same file and line, for
/// every case, under the repository's own exclude files.
#[test]
fn the_matcher_answers_as_git_check_ignore_does() {
    let root = temporary("check-ignore");
    let work = ignore_fixture(&root);
    let git = git_check_ignore(&work, IGNORE_CASES);
    let engine = matcher_answers(&work, IGNORE_CASES);
    for ((path, git), engine) in IGNORE_CASES.iter().zip(&git).zip(&engine) {
        assert_eq!(engine, git, "{path}");
    }
    assert!(git.iter().filter(|answer| answer.is_some()).count() >= 15);
    std::fs::remove_dir_all(root).unwrap();
}

/// Under the global files `HOME`, `XDG_CONFIG_HOME` and `GIT_CONFIG_GLOBAL` name, the matcher
/// still answers as Git does: the opener leaves them to gix, which reads them as Git does.
#[test]
fn the_matcher_follows_the_global_files_git_reads() {
    const NAME: &str = "the_matcher_follows_the_global_files_git_reads";
    if in_child(NAME) {
        let work = fixture();
        let git = git_check_ignore(&work, IGNORE_CASES);
        let engine = matcher_answers(&work, IGNORE_CASES);
        for ((path, git), engine) in IGNORE_CASES.iter().zip(&git).zip(&engine) {
            assert_eq!(engine, git, "{path}");
        }
        assert!(
            git.iter().flatten().any(|(_, _, pattern)| pattern == "*.bak"),
            "the global ignore file applies"
        );
        return;
    }
    let root = temporary("global-files");
    let work = ignore_fixture(&root);
    git(&work, &["config", "--unset", "core.excludesFile"]);
    let home = root.join("home");
    let xdg = root.join("xdg");
    write(&xdg, &[("git/ignore", b"*.bak\n")]);
    write(&home, &[(".config/git/ignore", b"*.swp\n")]);
    let global = root.join("global-gitconfig");
    let excludes = root.join("global-excludes");
    std::fs::write(&excludes, b"*.bak\n*.swp\n").unwrap();
    std::fs::write(
        &global,
        format!("[core]\n\texcludesFile = {}\n", excludes.to_string_lossy().replace('\\', "/")),
    )
    .unwrap();
    let base = || {
        let mut env = clean_env();
        env.extend([
            ("HOME", Some(home.clone().into())),
            (FIXTURE, Some(work.clone().into())),
        ]);
        env
    };
    // `git/ignore` in XDG_CONFIG_HOME.
    let mut xdg_env = base();
    xdg_env.push(("XDG_CONFIG_HOME", Some(xdg.clone().into())));
    run_child(NAME, &xdg_env);
    // `core.excludesFile` in the file GIT_CONFIG_GLOBAL names.
    let mut global_env = base();
    global_env.push(("GIT_CONFIG_GLOBAL", Some(global.clone().into())));
    run_child(NAME, &global_env);
    std::fs::remove_dir_all(root).unwrap();
}

/// What a damaged copy of the fixture gave: whether the matcher and discovery answered.
#[derive(Debug, Default)]
struct Outcomes {
    answered: usize,
    refused: usize,
    /// Copies where the matcher answered and `git check-ignore` failed.
    answered_where_git_failed: Vec<String>,
    /// Answers of the matcher that differ from Git's where Git answered too.
    disagreed: Vec<String>,
}

const DAMAGE_PATHS: &[&str] = &["drop.log", "kept.py", "build", "build/out.js", "src/a.py"];

fn damage_fixture(root: &Path) -> PathBuf {
    let work = repository_at(
        &root.join("pristine"),
        &[
            (".gitignore", b"*.log\nbuild/\n"),
            ("kept.py", b"print('kept')\n"),
            ("src/a.py", b"print('a')\n"),
        ],
    );
    for index in 0..3 {
        write(&work, &[(&format!("src/more{index}.py"), format!("print({index})\n").as_bytes())]);
        git(&work, &["add", "."]);
        git(&work, &["commit", "--quiet", "-m", &format!("more {index}")]);
    }
    git(&work, &["branch", "side"]);
    git(&work, &["gc", "--quiet"]);
    git(&work, &["pack-refs", "--all"]);
    write(&work, &[("drop.log", b""), ("build/out.js", b"")]);
    work
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Reads a damaged copy as the engine does, counting what answered and comparing the matcher's
/// answers with Git's where both answer. A panic or an abort would end the test process.
fn read_damaged(work: &Path, outcomes: &mut Outcomes, label: &str) {
    let matcher = GitIgnoreMatcher::open(work.to_string_lossy().into_owned());
    let engine = match matcher {
        Ok(Some(matcher)) => DAMAGE_PATHS
            .iter()
            .map(|path| matcher.ignored((*path).to_owned(), work.join(path).is_dir()).ok())
            .collect::<Option<Vec<_>>>(),
        _ => None,
    };
    let root = work.to_path_buf();
    let discovered = pool::run("discovery", pool::GIT_READ_TIME_LIMIT, move || {
        crate::git_workspace::discover(&root).is_ok()
    });
    for answered in [engine.is_some(), discovered == Ok(true)] {
        if answered {
            outcomes.answered += 1;
        } else {
            outcomes.refused += 1;
        }
    }
    let Some(engine) = engine else { return };
    let git = Command::new("git")
        .arg("-C")
        .arg(work)
        .args(["check-ignore", "--stdin", "-z", "-n", "-v"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            let input = DAMAGE_PATHS.iter().map(|path| format!("{path}\0")).collect::<String>();
            child.stdin.take().unwrap().write_all(input.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    if !(git.status.success() || git.status.code() == Some(1)) {
        outcomes.answered_where_git_failed.push(label.to_owned());
        return;
    }
    let text = String::from_utf8_lossy(&git.stdout).into_owned();
    let ignored = text
        .split('\0')
        .collect::<Vec<_>>()
        .chunks_exact(4)
        .map(|record| !record[0].is_empty() && !record[2].starts_with('!'))
        .collect::<Vec<_>>();
    if ignored != engine {
        outcomes
            .disagreed
            .push(format!("{label}: Git {ignored:?}, engine {engine:?}"));
    }
}

/// A fixed set of damaged repository files: every read answers or refuses, and the process stays up.
#[test]
fn damaged_files_never_take_the_process_down() {
    let root = temporary("damaged");
    let pristine = damage_fixture(&root);
    let pack = |extension: &str| {
        std::fs::read_dir(pristine.join(".git/objects/pack"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|found| found == extension))
            .unwrap()
            .strip_prefix(&pristine)
            .unwrap()
            .to_path_buf()
    };
    let cases: Vec<(&str, Box<dyn Fn(&Path)>)> = vec![
        ("truncated index", Box::new(|work: &Path| truncate(&work.join(".git/index"), 2))),
        ("index with a bad checksum", Box::new(|work: &Path| {
            let index = work.join(".git/index");
            let length = std::fs::metadata(&index).unwrap().len();
            flip(&index, length - 1);
        })),
        ("truncated pack", Box::new(move |work: &Path| truncate(&work.join(pack("pack")), 2))),
        ("truncated pack index", Box::new(move |work: &Path| truncate(&work.join(pack("idx")), 2))),
        ("garbage in packed-refs", Box::new(|work: &Path| {
            std::fs::write(work.join(".git/packed-refs"), b"\xff\xfe not a ref\n0000 refs/heads/x y z\n").unwrap();
        })),
        ("a config with one 1 GB line", Box::new(|work: &Path| {
            let config = work.join(".git/config");
            let mut text = std::fs::read(&config).unwrap();
            text.extend_from_slice(b"[rerail]\n\tlong = ");
            std::fs::write(&config, &text).unwrap();
            std::fs::OpenOptions::new().write(true).open(&config).unwrap().set_len(1 << 30).unwrap();
        })),
    ];
    let mut outcomes = Outcomes::default();
    for (index, (label, damage)) in cases.iter().enumerate() {
        let work = root.join(format!("case-{index}"));
        copy_tree(&pristine, &work);
        damage(&work);
        read_damaged(&work, &mut outcomes, label);
        std::fs::remove_dir_all(&work).unwrap();
    }
    eprintln!("damaged files: {outcomes:?}");
    assert_eq!(outcomes.answered + outcomes.refused, 2 * cases.len());
    assert!(outcomes.refused >= 2, "the 1 GB config is refused by both reads: {outcomes:?}");
    assert_eq!(outcomes.disagreed, Vec::<String>::new());
    std::fs::remove_dir_all(root).unwrap();
}

/// Git writes packs and loose objects read-only; a test damages its own copy.
fn make_writable(path: &Path) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false, reason = "the file is a test's own copy")]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn truncate(path: &Path, divisor: u64) {
    make_writable(path);
    let length = std::fs::metadata(path).unwrap().len();
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(length / divisor)
        .unwrap();
}

fn flip(path: &Path, offset: u64) {
    make_writable(path);
    let mut bytes = std::fs::read(path).unwrap();
    let offset = usize::try_from(offset).unwrap();
    bytes[offset] ^= 0xff;
    std::fs::write(path, bytes).unwrap();
}

/// A seeded run of byte flips over the files a read opens: every read answers or refuses, the
/// process stays up, and where Git answers the matcher's answers are Git's.
#[test]
fn seeded_byte_flips_never_take_the_process_down() {
    const FLIPS: usize = 200;
    let root = temporary("flips");
    let pristine = damage_fixture(&root);
    let mut files = vec![
        PathBuf::from(".git/index"),
        PathBuf::from(".git/HEAD"),
        PathBuf::from(".git/config"),
        PathBuf::from(".git/packed-refs"),
        PathBuf::from(".gitignore"),
    ];
    for entry in std::fs::read_dir(pristine.join(".git/objects/pack")).unwrap() {
        files.push(entry.unwrap().path().strip_prefix(&pristine).unwrap().to_path_buf());
    }
    let mut state: u64 = 0x5eed_0001;
    let mut next = || {
        // xorshift64: the same sequence on every run.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut outcomes = Outcomes::default();
    for flip_index in 0..FLIPS {
        let file = &files[usize::try_from(next() % files.len() as u64).unwrap()];
        let length = std::fs::metadata(pristine.join(file)).unwrap().len();
        let offset = next() % length.max(1);
        let work = root.join(format!("flip-{flip_index}"));
        copy_tree(&pristine, &work);
        flip(&work.join(file), offset);
        read_damaged(&work, &mut outcomes, &format!("{} at {offset}", file.display()));
        std::fs::remove_dir_all(&work).unwrap();
    }
    eprintln!("byte flips: {outcomes:?}");
    assert_eq!(outcomes.answered + outcomes.refused, 2 * FLIPS);
    assert_eq!(outcomes.disagreed, Vec::<String>::new());
    std::fs::remove_dir_all(root).unwrap();
}

/// A status run converts a changed file under a filter driver without starting the driver.
#[cfg(unix)]
#[test]
fn discovery_runs_no_filter_driver() {
    let root = temporary("filter");
    let work = repository_at(
        &root.join("work"),
        &[(".gitattributes", b"*.txt filter=probe\n"), ("notes.txt", b"one\n")],
    );
    let marker = root.join("driver-ran");
    git(
        &work,
        &["config", "filter.probe.clean", &format!("touch '{}'; cat", marker.display())],
    );
    std::fs::write(work.join("notes.txt"), b"two\n").unwrap();
    let workspace = crate::git_workspace::discover(&work).unwrap();
    assert!(!marker.exists(), "the filter driver ran");
    assert!(
        workspace
            .overlays
            .iter()
            .any(|overlay| overlay.path == "notes.txt"),
        "the changed file under the driver reads as changed"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_read_past_its_time_limit_is_an_error_and_the_pool_keeps_answering() {
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let stuck = pool::run("a stuck read", Duration::from_millis(50), move || {
        let _ = wait.recv();
    });
    assert!(stuck.unwrap_err().contains("a stuck read gave no answer within"));
    assert_eq!(pool::run("a read", Duration::from_secs(10), || 7), Ok(7));
    release.send(()).unwrap();
}

#[test]
fn a_read_that_panics_is_an_error_and_the_pool_keeps_answering() {
    let panicked = pool::run("a read", Duration::from_secs(10), || -> u8 {
        panic!("index out of range")
    });
    assert_eq!(panicked, Err("a read panicked: index out of range".to_owned()));
    let answers = (0..pool::GIT_READ_THREADS * 2)
        .map(|value| pool::run("a read", Duration::from_secs(10), move || value))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(answers, (0..pool::GIT_READ_THREADS * 2).collect::<Vec<_>>());
}

#[test]
fn a_file_past_the_read_limit_is_refused_before_it_is_read() {
    let root = temporary("size");
    let big = root.join("config");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(opener::MAX_GIT_FILE_BYTES + 1)
        .unwrap();
    assert!(matches!(opener::check_size(&big), Err(OpenError::Failed(_))));
    assert!(opener::check_size(&root.join("absent")).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}
