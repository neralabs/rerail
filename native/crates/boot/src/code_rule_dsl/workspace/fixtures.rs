//! The workspace tests' fixtures: the slot that serializes the tests sharing process-wide state, a Git fixture
//! repository, and a result page read as the JS thread reads it. This crate's tests and those above read them.

use super::api::CodeRuleDslWorkspace;
use napi::Task;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Serializes the tests that share process-wide state. A parking_lot mutex never poisons, so a
/// failing test cannot fail every test that takes the slot after it.
pub static WORKSPACE_TEST_SLOT: Mutex<()> = Mutex::new(());

/// Build a Git fixture repository. Tests only: production discovery reads Git through gix.
pub fn git(root: &std::path::Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", root)
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?} failed");
}

/// Issues a result page request the way the JS thread does (`resultPageAsync` returns its task);
/// the returned closure is the work the request leaves for a libuv pool thread.
pub fn request_result_page(
    workspace: &CodeRuleDslWorkspace,
    cursor: u32,
    maximum_bytes: u32,
) -> impl FnOnce() -> napi::Result<String> {
    let mut task = workspace.result_page_task(cursor, maximum_bytes);
    move || task.compute()
}

pub fn result_page(
    workspace: &CodeRuleDslWorkspace,
    cursor: u32,
    maximum_bytes: u32,
) -> napi::Result<String> {
    request_result_page(workspace, cursor, maximum_bytes)()
}

/// The source of each package module `package_repository` holds.
pub const PACKAGE_SOURCE: &str = "from . import helper\nhelper()\n";

/// A directory of its own under the system's temporary one, removed with it.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rerail-context-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(dunce::canonicalize(path).unwrap())
    }

    pub fn store(&self) -> PathBuf {
        self.0.join("store").join("content.sqlite")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A committed repository with two packages holding identical module source and one loose file.
pub fn package_repository(tag: &str) -> TempDir {
    let directory = TempDir::new(tag);
    let root = directory.0.join("repo");
    for (path, source) in [
        ("alpha/__init__.py", ""),
        ("alpha/helper.py", "def helper():\n    pass\n"),
        ("alpha/x.py", PACKAGE_SOURCE),
        ("beta/__init__.py", ""),
        ("beta/x.py", PACKAGE_SOURCE),
        ("loose/x.py", PACKAGE_SOURCE),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    git(&root, &["init", "-q"]);
    git(&root, &["add", "."]);
    git(
        &root,
        &[
            "-c",
            "user.name=rerail",
            "-c",
            "user.email=rerail@example.invalid",
            "commit",
            "-q",
            "-m",
            "fixture",
        ],
    );
    directory
}
