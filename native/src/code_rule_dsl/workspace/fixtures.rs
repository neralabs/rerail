//! The workspace tests' fixtures: the slot that serializes the tests sharing process-wide state, a Git fixture
//! repository, and a result page read as the JS thread reads it. This crate's tests and those above read them.

use super::api::CodeRuleDslWorkspace;
use napi::Task;
use parking_lot::Mutex;

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
