//! Storage attachment: when a workspace may attach its content store, which store the attach
//! opens, and what a close that lands mid-attach leaves behind.

use super::api::{
    CodeRuleDslWorkspace, LAST_APPLY_STORAGE_RETRIES, LAST_APPLY_UNPUBLISHED_FILES,
    ON_STORAGE_ATTACHED, StorageAttachment, WorkspaceApplyTask,
};
use super::persisted::{AFTER_FILES_PUBLISHED, LEASE_RENEWAL_ATTEMPTS};
use super::storage::storage_context_in;
use super::tests::{WORKSPACE_TEST_SLOT, git, result_page};
use super::{STORE_BUSY_COOLDOWN, TEST_STORE_BUSY_CLOCK_ADVANCE, TEST_UNPUBLISHED_RESIDENT_BYTES};
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
use crate::content_store::{
    AFTER_GIT_TREE_TRANSACTION, BUSY_ATTEMPT_HOOK, BUSY_DEADLINE_OVERRIDE, BUSY_TIMEOUT_OVERRIDE,
    BUSY_WAIT_HOOK, CLOCK_OFFSET, ContentStore, MAX_COLLECTION_BATCH,
};
use napi::Task;
use parking_lot::MutexGuard;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STORE_VARIABLE: &str = "RERAIL_CONTENT_STORE_PATH";

/// A committed Git repository in a private temporary directory.
struct Repository(PathBuf);

impl Repository {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "rerail-storage-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let root = directory.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("seed.py"), "value = 1\n").unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["add", "-A"]);
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
                "seed",
            ],
        );
        Self(directory.canonicalize().unwrap())
    }

    fn root(&self) -> String {
        self.0.join("repo").to_string_lossy().into_owned()
    }

    fn store(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Points `RERAIL_CONTENT_STORE_PATH` elsewhere for the guard's lifetime, then restores the previous
/// value, so a test can show the attach ignores it. Holders also hold `WORKSPACE_TEST_SLOT`: only
/// these tests write the variable, and Rust readers take std's environment lock.
struct StoreVariable(Option<OsString>);

impl StoreVariable {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os(STORE_VARIABLE);
        // SAFETY: see the type's documentation.
        unsafe { std::env::set_var(STORE_VARIABLE, path) };
        Self(previous)
    }

    fn replace(&self, path: &Path) {
        // SAFETY: see the type's documentation.
        unsafe { std::env::set_var(STORE_VARIABLE, path) };
    }
}

impl Drop for StoreVariable {
    fn drop(&mut self) {
        // SAFETY: see the type's documentation.
        unsafe {
            match &self.0 {
                Some(previous) => std::env::set_var(STORE_VARIABLE, previous),
                None => std::env::remove_var(STORE_VARIABLE),
            }
        }
    }
}

fn needle_code_rule() -> CompiledCodeRuleDsl {
    needle_code_rule_reporting("NEEDLE")
}

/// The needle code rule reporting `code`: another code is another rule, so another catalog.
fn needle_code_rule_reporting(code: &str) -> CompiledCodeRuleDsl {
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
    });
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "storage-needle", "revision": 1,
        "mode": "steer",
        "scope": { "include": ["**/*.py"], "exclude": [] },
        "clauses": [{
            "id": "needle", "violation": { "anchor": "witness.span", "code": code,
                "message": "found needle" },
            "forbid": { "regex": { "pattern": "NEEDLE", "source": "file.content" } }
        }]
    });
    CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap()
}

fn apply_source(workspace: &CodeRuleDslWorkspace, revision: u32, source: &str) -> Vec<String> {
    apply_files(workspace, revision, &[("edited.py", source)])
}

/// Applies `files` (path and source) as `revision`; the codes of the violations it returns.
fn apply_files(
    workspace: &CodeRuleDslWorkspace,
    revision: u32,
    files: &[(&str, &str)],
) -> Vec<String> {
    apply_changes(workspace, revision, files, &[])
}

/// Applies `files` (path and source) and the removal of `removed` as `revision`; the codes of the
/// violations it returns.
fn apply_changes(
    workspace: &CodeRuleDslWorkspace,
    revision: u32,
    files: &[(&str, &str)],
    removed: &[&str],
) -> Vec<String> {
    codes(&apply_value(workspace, revision, files, removed)["evaluation"]["violations"])
}

/// Applies as `apply_changes` does; the whole response.
fn apply_value(
    workspace: &CodeRuleDslWorkspace,
    revision: u32,
    files: &[(&str, &str)],
    removed: &[&str],
) -> Value {
    let files = files
        .iter()
        .map(|(path, source)| json!({ "path": path, "language": "python", "source": source }))
        .collect::<Vec<_>>();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": revision, "files": files, "removed": removed })
            .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    serde_json::from_str(&task.compute().unwrap()).unwrap()
}

/// The code of each of `violations`.
fn codes(violations: &Value) -> Vec<String> {
    violations
        .as_array()
        .unwrap()
        .iter()
        .map(|violation| violation["code"].as_str().unwrap().to_owned())
        .collect()
}

/// The slot every storage test takes; it never poisons, so one failing test fails alone.
fn test_slot() -> MutexGuard<'static, ()> {
    WORKSPACE_TEST_SLOT.lock()
}

fn store_argument(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn storage_attaches_after_code_rule_registration_until_the_first_apply() {
    let _slot = test_slot();
    let repository = Repository::new("after-registration");
    let store = repository.store("content.sqlite");

    let mut registered = CodeRuleDslWorkspace::new();
    registered.add_code_rule(&needle_code_rule(), None).unwrap();
    let attached = registered
        .storage_task(repository.root(), store_argument(&store))
        .unwrap()
        .compute();
    assert!(
        matches!(attached, Ok(StorageAttachment { attached: true, .. })),
        "a workspace whose scopes are already registered attaches storage before its first apply: {attached:?}"
    );
    assert_eq!(apply_source(&registered, 1, "NEEDLE\n"), ["NEEDLE"]);
    registered.close();

    let mut applied = CodeRuleDslWorkspace::new();
    applied.add_code_rule(&needle_code_rule(), None).unwrap();
    assert_eq!(apply_source(&applied, 1, "NEEDLE\n"), ["NEEDLE"]);
    assert!(
        applied
            .storage_task(repository.root(), store_argument(&store))
            .unwrap()
            .compute()
            .is_err(),
        "storage cannot attach under results an earlier apply produced without it"
    );
    applied.close();
}

#[test]
fn storage_attach_opens_the_store_named_when_it_was_requested() {
    let _slot = test_slot();
    let repository = Repository::new("requested-store");
    let requested = repository.store("requested.sqlite");
    let changed = repository.store("changed.sqlite");
    let environment = StoreVariable::set(&changed);
    let workspace = CodeRuleDslWorkspace::new();
    let mut task = workspace
        .storage_task(repository.root(), store_argument(&requested))
        .unwrap();
    // The host changes its environment after asking for the attach, before a pool thread runs it.
    environment.replace(&repository.store("later.sqlite"));
    let attached = task.compute();
    assert!(
        matches!(attached, Ok(StorageAttachment { attached: true, .. })),
        "{attached:?}"
    );
    assert!(
        requested.exists(),
        "the attach opens the store named when it was requested"
    );
    assert!(
        !changed.exists() && !repository.store("later.sqlite").exists(),
        "the environment never redirects a requested attach"
    );
    workspace.close();
}

#[test]
fn closing_while_storage_attaches_releases_the_attached_storage() {
    let _slot = test_slot();
    let repository = Repository::new("close-during-attach");
    let store = repository.store("content.sqlite");
    let workspace = Arc::new(CodeRuleDslWorkspace::new());
    let mut task = workspace
        .storage_task(repository.root(), store_argument(&store))
        .unwrap();
    let closer = Arc::clone(&workspace);
    ON_STORAGE_ATTACHED.set(Some(Box::new(move || closer.close())));
    let _ = task.compute();
    assert!(
        ON_STORAGE_ATTACHED.take().is_none(),
        "the attach reached the workspace lock and ran the close there"
    );
    assert!(
        workspace.state.lock().unwrap().storage.is_none(),
        "a close that lands while the attach holds the workspace leaves no storage registration behind"
    );
}

/// An attach Git cannot describe still succeeds without a store, and says why unless the root is
/// simply outside any repository: persistent causes must not disable reuse silently.
#[test]
fn storage_attach_reports_why_it_attached_no_store() {
    let _slot = test_slot();
    let repository = Repository::new("unreadable-rerailignore");
    let store = repository.store("content.sqlite");
    std::fs::create_dir(Path::new(&repository.root()).join(".rerailignore")).unwrap();
    let workspace = CodeRuleDslWorkspace::new();
    let outcome = format!(
        "{:?}",
        workspace
            .storage_task(repository.root(), store_argument(&store))
            .unwrap()
            .compute()
    );
    workspace.close();
    assert!(
        outcome.starts_with(
            "Ok(StorageAttachment { attached: false, reason: Some(\"cannot read .rerailignore"
        ),
        "an attach without a store reports its discovery error: {outcome}"
    );

    let outside = repository.store("outside");
    std::fs::create_dir(&outside).unwrap();
    let workspace = CodeRuleDslWorkspace::new();
    let outcome = workspace
        .storage_task(
            outside.to_string_lossy().into_owned(),
            store_argument(&store),
        )
        .unwrap()
        .compute();
    workspace.close();
    assert!(
        matches!(
            outcome,
            Ok(StorageAttachment {
                attached: false,
                reason: None,
                ..
            })
        ),
        "a root outside any repository is expected to attach no store: {outcome:?}"
    );
}

/// An attach reports the wall time it spent discovering the Git checkout and publishing its
/// snapshot, so a slow attach shows which phase it waited in; a phase that did not run is 0.
#[test]
fn storage_attach_reports_the_time_of_each_phase() {
    let _slot = test_slot();
    let repository = Repository::new("attach-phases");
    let workspace = CodeRuleDslWorkspace::new();
    let attached = workspace
        .storage_task(
            repository.root(),
            store_argument(&repository.store("content.sqlite")),
        )
        .unwrap()
        .compute()
        .unwrap();
    workspace.close();
    assert!(attached.attached, "{attached:?}");
    assert!(
        attached.discovery_milliseconds > 0.0 && attached.publication_milliseconds > 0.0,
        "an attach to a Git checkout ran both phases: {attached:?}"
    );

    let outside = repository.store("outside");
    std::fs::create_dir(&outside).unwrap();
    let workspace = CodeRuleDslWorkspace::new();
    let detached = workspace
        .storage_task(
            outside.to_string_lossy().into_owned(),
            store_argument(&repository.store("unused.sqlite")),
        )
        .unwrap()
        .compute()
        .unwrap();
    workspace.close();
    assert!(!detached.attached, "{detached:?}");
    assert!(
        detached.discovery_milliseconds > 0.0 && detached.publication_milliseconds == 0.0,
        "an attach outside Git discovered but published nothing: {detached:?}"
    );
}

/// The attach opens the store file its caller names and nothing else: no default below `HOME`
/// and no environment variable, whichever build (managed or checkout) the host is.
#[test]
fn enable_storage_opens_exactly_the_store_the_caller_names() {
    let _slot = test_slot();
    let repository = Repository::new("explicit-store");
    let environment = repository.store("environment.sqlite");
    let _environment = StoreVariable::set(&environment);
    let chosen = repository.store("chosen").join("content.sqlite");

    let missing = CodeRuleDslWorkspace::new();
    let refused = missing.storage_task(repository.root(), String::new());
    missing.close();
    assert!(
        refused.is_err(),
        "an attach without a store path must be refused, not defaulted"
    );

    let workspace = CodeRuleDslWorkspace::new();
    let attached = workspace
        .storage_task(repository.root(), store_argument(&chosen))
        .unwrap()
        .compute();
    workspace.close();
    assert!(
        matches!(attached, Ok(StorageAttachment { attached: true, .. })),
        "{attached:?}"
    );
    assert!(chosen.is_file(), "the attach opens the store it was given");
    assert!(
        !environment.exists(),
        "the environment never chooses the store"
    );

    let outside = repository.store("outside");
    std::fs::create_dir(&outside).unwrap();
    let unused = repository.store("unused").join("content.sqlite");
    let bare = CodeRuleDslWorkspace::new();
    let outcome = bare
        .storage_task(
            outside.to_string_lossy().into_owned(),
            store_argument(&unused),
        )
        .unwrap()
        .compute();
    bare.close();
    assert!(
        matches!(
            outcome,
            Ok(StorageAttachment {
                attached: false,
                ..
            })
        ),
        "{outcome:?}"
    );
    assert!(
        !unused.exists(),
        "outside Git nothing attaches, so no store is created"
    );
}

/// Advances this thread's store clock for the guard's lifetime.
struct ClockOffset;

impl ClockOffset {
    fn advance(seconds: i64) {
        CLOCK_OFFSET.set(CLOCK_OFFSET.get() + seconds);
    }
}

impl Drop for ClockOffset {
    fn drop(&mut self) {
        CLOCK_OFFSET.set(0);
        AFTER_GIT_TREE_TRANSACTION.set(None);
    }
}

/// A snapshot publication that outlasts the workspace lease still registers a live workspace: the
/// lease runs from the registration, so a collection right after the attach keeps it.
#[test]
fn a_publication_slower_than_the_lease_registers_a_live_workspace() {
    let _slot = test_slot();
    let repository = Repository::new("slow-publication");
    let _clock = ClockOffset;
    let mut slow = true;
    // Writing the snapshot's trees takes longer than a whole lease.
    AFTER_GIT_TREE_TRANSACTION.set(Some(Box::new(move || {
        if std::mem::take(&mut slow) {
            ClockOffset::advance(1_000);
        }
    })));
    let context = storage_context_in(&repository.root(), repository.store("content.sqlite"));
    AFTER_GIT_TREE_TRANSACTION.set(None);
    let context = context.unwrap();
    assert_eq!(CLOCK_OFFSET.get(), 1_000, "the tree publication ran");

    let now = crate::content_store::unix_seconds().unwrap();
    context.store.collect(now, MAX_COLLECTION_BATCH).unwrap();
    let renewed =
        context
            .store
            .renew_workspace_lease(context.workspace_id, &context.owner, now + 120);
    assert!(
        renewed.is_ok(),
        "a collection right after a slow attach retired the just-attached workspace: {renewed:?}"
    );
}

/// Shortens this thread's store write deadline for the guard's lifetime, and the wait of each
/// attempt to 10 ms: the deadline then spans many attempts however the scheduler treats them,
/// instead of one or two 250 ms ones. The guard clears the busy-wait hooks too.
pub(super) struct BusyDeadline;

impl BusyDeadline {
    pub(super) fn set(deadline: Duration) -> Self {
        BUSY_DEADLINE_OVERRIDE.set(Some(deadline));
        BUSY_TIMEOUT_OVERRIDE.set(Some(Duration::from_millis(10)));
        Self
    }
}

impl Drop for BusyDeadline {
    fn drop(&mut self) {
        BUSY_DEADLINE_OVERRIDE.set(None);
        BUSY_TIMEOUT_OVERRIDE.set(None);
        BUSY_ATTEMPT_HOOK.set(None);
        BUSY_WAIT_HOOK.set(None);
    }
}

/// Makes each of this thread's store writes that finds the store busy spend `spent` of its
/// deadline without sleeping it, so a test reaches a deadline by counting attempts instead of
/// waiting out the clock; `on_attempt` runs on each such attempt (to release a writer). Returns
/// how much of the operation's deadline each attempt found already spent.
pub(super) fn spend_deadline_on_busy_attempts(
    spent: Duration,
    mut on_attempt: impl FnMut() + 'static,
) -> Rc<RefCell<Vec<Duration>>> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&seen);
    BUSY_ATTEMPT_HOOK.set(Some(Box::new(move |waited| {
        recorded.borrow_mut().push(waited);
        on_attempt();
        spent
    })));
    seen
}

/// Lets a test pass the window a busy write opened (`note_store_busy`) without waiting for it;
/// the thread's busy clock is reset when the guard drops.
pub(super) struct BusyWindowClock;

impl BusyWindowClock {
    pub(super) fn pass_window(&self) {
        TEST_STORE_BUSY_CLOCK_ADVANCE
            .set(TEST_STORE_BUSY_CLOCK_ADVANCE.get() + STORE_BUSY_COOLDOWN);
    }
}

impl Drop for BusyWindowClock {
    fn drop(&mut self) {
        TEST_STORE_BUSY_CLOCK_ADVANCE.set(Duration::ZERO);
    }
}

/// The storage retries and retry wait (nanoseconds) the last apply on this thread reported.
fn last_apply_storage_retries() -> (usize, u64) {
    LAST_APPLY_STORAGE_RETRIES.get()
}

/// Another connection that holds the store's write lock until dropped.
pub(super) struct HeldWriter(rusqlite::Connection);

impl HeldWriter {
    pub(super) fn hold(store: &Path) -> Self {
        let connection = rusqlite::Connection::open(store).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        Self(connection)
    }
}

impl Drop for HeldWriter {
    fn drop(&mut self) {
        let _ = self.0.execute_batch("COMMIT");
    }
}

/// A store another writer holds past the write deadline is a cache miss: the attach succeeds
/// without a store and says why, whether the store already exists (snapshot publication waits)
/// or not (creating its schema waits).
#[test]
fn storage_attaches_without_a_store_another_writer_holds_past_the_deadline() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let repository = Repository::new("busy-store");
    let current = repository.store("current.sqlite");
    drop(ContentStore::open(&current).unwrap());
    for (store, during) in [
        (current, "while publishing the Git snapshot"),
        (repository.store("new.sqlite"), "while opening the store"),
    ] {
        let _writer = HeldWriter::hold(&store);
        let workspace = CodeRuleDslWorkspace::new();
        let outcome = workspace
            .storage_task(repository.root(), store_argument(&store))
            .unwrap()
            .compute();
        workspace.close();
        assert!(
            matches!(
                &outcome,
                Ok(StorageAttachment { attached: false, reason: Some(reason), .. })
                    if reason.starts_with("content store busy: ") && reason.ends_with(during)
            ),
            "{outcome:?}"
        );
    }
}

/// An apply whose publication finds the store held past the write deadline still returns its
/// evaluation: the files stay resident and unpublished, and only their reuse is lost.
#[test]
fn an_apply_returns_its_results_while_another_writer_holds_the_store() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-apply");
    let store = repository.store("content.sqlite");
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&needle_code_rule(), None).unwrap();
    let attached = workspace
        .storage_task(repository.root(), store_argument(&store))
        .unwrap()
        .compute();
    assert!(
        matches!(attached, Ok(StorageAttachment { attached: true, .. })),
        "{attached:?}"
    );
    let writer = HeldWriter::hold(&store);
    assert_eq!(apply_source(&workspace, 1, "NEEDLE\n"), ["NEEDLE"]);
    {
        let state = workspace.state.lock().unwrap();
        let edited = &state.files["edited.py"];
        assert!(
            edited.stored.is_none() && edited.input.source.as_str() == "NEEDLE\n",
            "an unpublished file stays resident"
        );
    }
    drop(writer);
    // The store is free again, and the window the busy apply opened has passed.
    window.pass_window();
    assert_eq!(apply_source(&workspace, 2, "NEEDLE = 2\n"), ["NEEDLE"]);
    assert!(
        workspace.state.lock().unwrap().files["edited.py"]
            .stored
            .is_some(),
        "once the writer is gone the next apply publishes again"
    );
    workspace.close();
}

/// A workspace running `code_rule` with `store` attached.
fn attached_workspace(
    repository: &Repository,
    store: &Path,
    code_rule: &CompiledCodeRuleDsl,
) -> CodeRuleDslWorkspace {
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(code_rule, None).unwrap();
    let attached = workspace
        .storage_task(repository.root(), store_argument(store))
        .unwrap()
        .compute();
    assert!(
        matches!(attached, Ok(StorageAttachment { attached: true, .. })),
        "{attached:?}"
    );
    workspace
}

/// The codes of the current revision's violations, read through one result page.
fn paged_codes(workspace: &CodeRuleDslWorkspace) -> Vec<String> {
    let page: Value =
        serde_json::from_str(&result_page(workspace, 0, 64 * 1024 * 1024).unwrap()).unwrap();
    assert_eq!(page["done"], true, "{page}");
    codes(&page["violations"])
}

/// A source the store already holds, evaluated under another catalog while another writer holds
/// the store past the deadline, keeps its new results through eviction: the store holds its
/// source but not those results, so the file stays resident. The next apply that publishes
/// stores them too; eviction then releases the file, and paging reads them back from the store.
#[test]
fn an_unpublished_file_keeps_its_results_through_eviction_though_its_source_is_stored() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let repository = Repository::new("busy-stored-source");
    let window = BusyWindowClock;
    let store = repository.store("content.sqlite");
    let earlier = attached_workspace(&repository, &store, &needle_code_rule());
    assert_eq!(apply_source(&earlier, 1, "NEEDLE\n"), ["NEEDLE"]);
    earlier.close();

    let code_rule = needle_code_rule_reporting("SHARP_NEEDLE");
    let workspace = attached_workspace(&repository, &store, &code_rule);
    let writer = HeldWriter::hold(&store);
    assert_eq!(apply_source(&workspace, 1, "NEEDLE\n"), ["SHARP_NEEDLE"]);
    assert!(
        workspace.state.lock().unwrap().files["edited.py"]
            .stored
            .is_some(),
        "the store already holds the source"
    );
    assert!(workspace.evict_inactive().unwrap());
    assert!(
        workspace
            .state
            .lock()
            .unwrap()
            .files
            .contains_key("edited.py"),
        "eviction keeps a file whose results are unpublished"
    );
    assert_eq!(paged_codes(&workspace), ["SHARP_NEEDLE"]);

    drop(writer);
    window.pass_window();
    assert!(apply_files(&workspace, 2, &[("other.py", "value = 2\n")]).is_empty());
    assert!(workspace.evict_inactive().unwrap());
    assert!(
        !workspace
            .state
            .lock()
            .unwrap()
            .files
            .contains_key("edited.py"),
        "once the next apply publishes its results, eviction releases the file"
    );
    assert_eq!(paged_codes(&workspace), ["SHARP_NEEDLE"]);
    workspace.close();
}

/// A busy publication holds back only its own files: eviction still releases every file the
/// store holds, and the next apply that publishes stores the held-back file too, so eviction
/// releases it as well instead of keeping every later file resident.
#[test]
fn a_busy_publication_keeps_only_its_own_files_resident() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-eviction");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let resident = || {
        workspace
            .state
            .lock()
            .unwrap()
            .files
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(apply_files(&workspace, 1, &[("persisted.py", "value = 1\n")]).is_empty());
    let writer = HeldWriter::hold(&store);
    assert_eq!(apply_source(&workspace, 2, "NEEDLE\n"), ["NEEDLE"]);
    assert!(workspace.evict_inactive().unwrap());
    assert_eq!(
        resident(),
        ["edited.py"],
        "eviction releases the stored file and keeps the unpublished one"
    );
    assert_eq!(paged_codes(&workspace), ["NEEDLE"]);

    drop(writer);
    window.pass_window();
    assert!(apply_files(&workspace, 3, &[("other.py", "value = 3\n")]).is_empty());
    assert!(workspace.evict_inactive().unwrap());
    assert!(
        resident().is_empty(),
        "the apply after the writer left published the held-back file: {:?}",
        resident()
    );
    assert_eq!(paged_codes(&workspace), ["NEEDLE"]);
    workspace.close();
}

/// An apply that removes a file while another writer holds the store past the deadline still
/// returns its evaluation: the removal applies in memory, the store keeps the removed file's
/// overlay row, and the next apply that publishes deletes it.
#[test]
fn an_apply_that_removes_a_file_returns_its_results_while_another_writer_holds_the_store() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-removal");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let overlay = |path: &str| {
        let state = workspace.state.lock().unwrap();
        let storage = state.storage.as_ref().unwrap();
        storage
            .store
            .overlay_file(storage.workspace_id, path)
            .unwrap()
    };
    assert!(apply_files(&workspace, 1, &[("removed.py", "value = 1\n")]).is_empty());
    assert!(overlay("removed.py").is_some(), "the file is published");

    let writer = HeldWriter::hold(&store);
    assert_eq!(
        apply_changes(&workspace, 2, &[("edited.py", "NEEDLE\n")], &["removed.py"]),
        ["NEEDLE"]
    );
    assert!(
        !workspace
            .state
            .lock()
            .unwrap()
            .files
            .contains_key("removed.py"),
        "the removal applies in memory"
    );
    assert!(
        overlay("removed.py").is_some(),
        "the busy store keeps the removed file's row"
    );

    drop(writer);
    window.pass_window();
    assert!(apply_files(&workspace, 3, &[("other.py", "value = 3\n")]).is_empty());
    assert!(
        overlay("removed.py").is_none(),
        "the apply after the writer left deleted the removed file's row"
    );
    assert_eq!(paged_codes(&workspace), ["NEEDLE"]);
    workspace.close();
}

/// Another writer that takes the store once an apply published its files, holding it past the
/// deadline while the apply promotes them to the snapshot and renews its lease, costs only reuse:
/// the apply still returns its evaluation.
#[test]
fn an_apply_returns_its_results_when_another_writer_takes_the_store_after_its_publication() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let repository = Repository::new("busy-promotion");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let writer = Rc::new(RefCell::new(None));
    let taken = Rc::clone(&writer);
    let held = store.clone();
    AFTER_FILES_PUBLISHED.set(Some(Box::new(move || {
        *taken.borrow_mut() = Some(HeldWriter::hold(&held));
    })));
    // The committed `seed.py` is clean, so the apply promotes it to the snapshot.
    assert_eq!(
        apply_files(
            &workspace,
            1,
            &[("seed.py", "value = 1\n"), ("edited.py", "NEEDLE\n")]
        ),
        ["NEEDLE"]
    );
    assert!(
        writer.borrow().is_some(),
        "the writer took the store after the publication"
    );
    assert!(
        workspace.state.lock().unwrap().files["edited.py"]
            .stored
            .is_some(),
        "the files are published"
    );
    drop(writer);
    workspace.close();
}

/// One apply waits out other writers for one write deadline in all, not one per write: its
/// publication spends most of the deadline on a writer that lets go, then another writer takes
/// the store, and the promotion gives up once what is left of the deadline is spent instead of
/// waiting a whole deadline of its own. Each busy attempt spends 2.4 s of the 3 s deadline
/// without sleeping it: the promotion finds the publication's share spent and makes one busy
/// attempt, where a deadline of its own would allow two. The files stay published; only the
/// sharing is lost.
#[test]
fn an_apply_waits_out_other_writers_for_one_deadline_in_all() {
    let _slot = test_slot();
    let spent = Duration::from_millis(2_400);
    let _deadline = BusyDeadline::set(Duration::from_secs(3));
    let _window = BusyWindowClock;
    let repository = Repository::new("busy-one-deadline");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let first = Rc::new(RefCell::new(Some(HeldWriter::hold(&store))));
    let released = Rc::clone(&first);
    let seen = spend_deadline_on_busy_attempts(spent, move || drop(released.borrow_mut().take()));
    let writer = Rc::new(RefCell::new(None));
    let taken = Rc::clone(&writer);
    let held = store.clone();
    AFTER_FILES_PUBLISHED.set(Some(Box::new(move || {
        *taken.borrow_mut() = Some(HeldWriter::hold(&held));
    })));
    // The committed `seed.py` is clean, so the apply promotes it to the snapshot.
    assert_eq!(
        apply_files(
            &workspace,
            1,
            &[("seed.py", "value = 1\n"), ("edited.py", "NEEDLE\n")]
        ),
        ["NEEDLE"]
    );
    assert!(first.borrow().is_none(), "the first writer let go");
    assert!(writer.borrow().is_some(), "the writer took the store");
    assert!(
        workspace.state.lock().unwrap().files["edited.py"]
            .stored
            .is_some(),
        "the files are published"
    );
    assert!(
        workspace.state.lock().unwrap().store_busy_until.is_some(),
        "the promotion found the store busy"
    );
    let seen = seen.borrow();
    assert_eq!(
        seen.len(),
        2,
        "one busy attempt in the publication, one in the promotion: {seen:?}"
    );
    assert!(
        seen[1] >= spent,
        "the promotion starts from what the publication spent: {seen:?}"
    );
    drop(writer);
    workspace.close();
}

/// An apply that leaves files unpublished because the store is busy counts them, and one that
/// publishes them all counts none.
#[test]
fn a_busy_apply_counts_the_files_it_left_unpublished() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-unpublished-count");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let writer = HeldWriter::hold(&store);
    assert!(
        apply_files(
            &workspace,
            1,
            &[("one.py", "value = 1\n"), ("two.py", "value = 2\n")]
        )
        .is_empty()
    );
    assert_eq!(LAST_APPLY_UNPUBLISHED_FILES.get(), 2);

    // Inside the busy window the apply writes nothing: its own file and the two still waiting.
    assert!(apply_files(&workspace, 2, &[("three.py", "value = 3\n")]).is_empty());
    assert_eq!(LAST_APPLY_UNPUBLISHED_FILES.get(), 3);

    drop(writer);
    window.pass_window();
    assert!(apply_files(&workspace, 3, &[("four.py", "value = 4\n")]).is_empty());
    assert_eq!(LAST_APPLY_UNPUBLISHED_FILES.get(), 0);
    workspace.close();
}

/// Whether `path` is resident and flagged as left unpublished by a busy store.
fn is_unpublished(workspace: &CodeRuleDslWorkspace, path: &str) -> bool {
    workspace.state.lock().unwrap().files[path].unpublished
}

/// A store another writer keeps busy costs one write deadline, not one per apply: the apply that
/// waits it out opens a window, and an apply inside it makes no store write (no publication, and
/// a removal waits in `pending_removals`, even once the writer left) yet returns its evaluation.
#[test]
fn a_store_that_stays_busy_costs_one_deadline_not_one_per_apply() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let _window = BusyWindowClock;
    let repository = Repository::new("busy-window");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let overlay_present = |path: &str| {
        let state = workspace.state.lock().unwrap();
        let storage = state.storage.as_ref().unwrap();
        storage
            .store
            .overlay_file(storage.workspace_id, path)
            .unwrap()
            .is_some()
    };
    assert!(apply_files(&workspace, 1, &[("removed.py", "value = 1\n")]).is_empty());
    assert!(overlay_present("removed.py"), "the file is published");

    let writer = HeldWriter::hold(&store);
    assert_eq!(apply_source(&workspace, 2, "NEEDLE\n"), ["NEEDLE"]);
    let (retries, _) = last_apply_storage_retries();
    assert!(retries > 0, "the first apply waited the store out");
    assert!(is_unpublished(&workspace, "edited.py"));
    assert!(workspace.state.lock().unwrap().store_busy_until.is_some());
    assert!(workspace.state.lock().unwrap().store_busy_reported);

    assert_eq!(
        apply_source(&workspace, 3, "NEEDLE = 2\n"),
        ["NEEDLE"],
        "an apply inside the window returns its evaluation"
    );
    assert_eq!(
        last_apply_storage_retries(),
        (0, 0),
        "an apply inside the window never waits on the store"
    );
    assert!(is_unpublished(&workspace, "edited.py"));

    drop(writer);
    assert!(apply_changes(&workspace, 4, &[], &["removed.py"]).is_empty());
    assert_eq!(
        workspace.state.lock().unwrap().pending_removals,
        ["removed.py".to_owned()].into(),
        "the removal waits for a store write"
    );
    assert!(
        overlay_present("removed.py"),
        "a free store is left alone inside the window"
    );
    workspace.close();
}

/// Once the window has passed and the writer left, the next apply tries the store again: it
/// deletes the removals the busy store kept, then publishes the files it left unpublished. Until
/// then even a free store is left alone.
#[test]
fn the_apply_after_the_busy_window_publishes_and_deletes_pending_removals() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-window-expiry");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let overlay_present = |path: &str| {
        let state = workspace.state.lock().unwrap();
        let storage = state.storage.as_ref().unwrap();
        storage
            .store
            .overlay_file(storage.workspace_id, path)
            .unwrap()
            .is_some()
    };
    assert!(apply_files(&workspace, 1, &[("removed.py", "value = 1\n")]).is_empty());

    let writer = HeldWriter::hold(&store);
    // The removal waits out the deadline and opens the window.
    assert_eq!(
        apply_changes(&workspace, 2, &[("edited.py", "NEEDLE\n")], &["removed.py"]),
        ["NEEDLE"]
    );
    assert!(is_unpublished(&workspace, "edited.py"));
    drop(writer);

    assert_eq!(apply_source(&workspace, 3, "NEEDLE = 2\n"), ["NEEDLE"]);
    assert!(
        is_unpublished(&workspace, "edited.py") && overlay_present("removed.py"),
        "inside the window a free store is left alone"
    );

    window.pass_window();
    assert_eq!(apply_source(&workspace, 4, "NEEDLE = 3\n"), ["NEEDLE"]);
    assert!(!is_unpublished(&workspace, "edited.py"));
    assert!(
        workspace.state.lock().unwrap().files["edited.py"]
            .stored
            .is_some(),
        "the file is published"
    );
    assert!(!overlay_present("removed.py"), "the removal is deleted");
    let state = workspace.state.lock().unwrap();
    assert!(state.pending_removals.is_empty());
    assert!(
        state.store_busy_until.is_none(),
        "a store that answers closes the window"
    );
    drop(state);
    workspace.close();
}

/// A writer that takes the store once an apply published its files (the promotion waits out the
/// deadline) opens the window too, and the lease renewal is not attempted behind the promotion
/// (it would wait the same deadline again): the apply still returns its evaluation, and the next
/// apply makes no store write.
#[test]
fn a_busy_promotion_opens_the_window() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let _window = BusyWindowClock;
    let repository = Repository::new("busy-promotion-window");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    LEASE_RENEWAL_ATTEMPTS.set(0);
    let writer = Rc::new(RefCell::new(None));
    let taken = Rc::clone(&writer);
    let held = store.clone();
    AFTER_FILES_PUBLISHED.set(Some(Box::new(move || {
        *taken.borrow_mut() = Some(HeldWriter::hold(&held));
    })));
    // The committed `seed.py` is clean, so the apply promotes it to the snapshot.
    assert_eq!(
        apply_files(
            &workspace,
            1,
            &[("seed.py", "value = 1\n"), ("edited.py", "NEEDLE\n")]
        ),
        ["NEEDLE"]
    );
    assert!(writer.borrow().is_some(), "the writer took the store");
    assert_eq!(
        LEASE_RENEWAL_ATTEMPTS.get(),
        0,
        "the renewal is not attempted once the promotion was busy"
    );
    assert!(
        workspace.state.lock().unwrap().store_busy_until.is_some(),
        "a busy promotion opens the window"
    );

    assert_eq!(apply_source(&workspace, 2, "NEEDLE = 2\n"), ["NEEDLE"]);
    assert_eq!(
        last_apply_storage_retries(),
        (0, 0),
        "the apply after a busy promotion never waits on the store"
    );
    assert!(is_unpublished(&workspace, "edited.py"));
    drop(writer);
    workspace.close();
}

/// An apply whose publication waits out the deadline reports the retries it spent, though it
/// published nothing.
#[test]
fn a_busy_apply_reports_its_publication_retries() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let _window = BusyWindowClock;
    let repository = Repository::new("busy-retries");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let _writer = HeldWriter::hold(&store);
    assert_eq!(apply_source(&workspace, 1, "NEEDLE\n"), ["NEEDLE"]);
    let (retries, wait_nanoseconds) = last_apply_storage_retries();
    assert!(retries > 0, "{retries} retries");
    assert!(wait_nanoseconds > 0, "{wait_nanoseconds} ns of waiting");
    assert!(is_unpublished(&workspace, "edited.py"));
    workspace.close();
}

/// A completion inside the window makes no store write: it fails as a busy store does, so the
/// host still reports that the reuse was not recorded, although the store is free again. Once
/// the window has passed, it records the revision.
#[test]
fn a_completion_inside_the_busy_window_makes_no_store_write() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-completion-window");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let completed = || {
        let state = workspace.state.lock().unwrap();
        let storage = state.storage.as_ref().unwrap();
        storage
            .store
            .completed_workspace_revision(storage.workspace_id)
            .unwrap()
    };
    let writer = HeldWriter::hold(&store);
    assert_eq!(apply_source(&workspace, 1, "NEEDLE\n"), ["NEEDLE"]);
    drop(writer);
    assert!(
        workspace.state.lock().unwrap().store_busy_until.is_some(),
        "the busy apply opened the window"
    );

    let error = workspace.completion_task().compute().unwrap_err();
    assert_eq!(error.reason, "content store writer is busy");
    assert_eq!(completed(), None, "the free store was left alone");

    window.pass_window();
    workspace.completion_task().compute().unwrap();
    assert_eq!(
        completed(),
        Some(1),
        "once the window passed it is recorded"
    );
    workspace.close();
}

/// A completion that waits out the write deadline opens the window: it fails as a busy store
/// does, and the next apply makes no store write.
#[test]
fn a_busy_completion_opens_the_window() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let _window = BusyWindowClock;
    let repository = Repository::new("busy-completion");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    assert_eq!(apply_source(&workspace, 1, "NEEDLE\n"), ["NEEDLE"]);
    assert!(workspace.state.lock().unwrap().store_busy_until.is_none());

    let writer = HeldWriter::hold(&store);
    let error = workspace.completion_task().compute().unwrap_err();
    assert_eq!(error.reason, "content store writer is busy");
    assert!(
        workspace.state.lock().unwrap().store_busy_until.is_some(),
        "a busy completion opens the window"
    );
    drop(writer);

    assert_eq!(apply_source(&workspace, 2, "NEEDLE = 2\n"), ["NEEDLE"]);
    assert!(
        is_unpublished(&workspace, "edited.py"),
        "the apply after a busy completion makes no store write"
    );
    workspace.close();
}

/// An apply that publishes nothing of its own (here a removal-only one) still retries the files a
/// busy scan left unpublished once the window has passed, so they are stored and evictable.
#[test]
fn a_removal_only_apply_publishes_the_files_a_busy_scan_left_unpublished() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-removal-only-retry");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let id = workspace
        .state
        .lock()
        .unwrap()
        .storage
        .as_ref()
        .unwrap()
        .workspace_id;
    let writer = HeldWriter::hold(&store);
    assert!(apply_files(&workspace, 1, &[("one.py", "value = 1\n")]).is_empty());
    assert!(is_unpublished(&workspace, "one.py"));
    drop(writer);
    window.pass_window();

    assert!(apply_changes(&workspace, 2, &[], &["absent.py"]).is_empty());
    assert!(!is_unpublished(&workspace, "one.py"));
    assert!(overlay_row_exists(&store, id, "one.py"));
    assert!(workspace.evict_inactive().unwrap());
    assert!(workspace.state.lock().unwrap().files.is_empty());
    workspace.close();
}

/// The scan-completion task publishes the files a busy scan left unpublished once the window has
/// passed, before recording the completion.
#[test]
fn a_completion_publishes_the_files_a_busy_scan_left_unpublished() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-completion-retry");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let id = workspace
        .state
        .lock()
        .unwrap()
        .storage
        .as_ref()
        .unwrap()
        .workspace_id;
    let writer = HeldWriter::hold(&store);
    assert!(apply_files(&workspace, 1, &[("one.py", "value = 1\n")]).is_empty());
    assert!(is_unpublished(&workspace, "one.py"));
    drop(writer);
    window.pass_window();

    workspace.completion_task().compute().unwrap();
    assert!(!is_unpublished(&workspace, "one.py"));
    assert!(overlay_row_exists(&store, id, "one.py"));
    assert!(workspace.evict_inactive().unwrap());
    workspace.close();
}

/// Whether the overlay of the workspace `id` in the store at `path` holds a row for `file`, read
/// through a store handle of its own.
fn overlay_row_exists(path: &Path, id: crate::content_store::WorkspaceId, file: &str) -> bool {
    ContentStore::open(path)
        .unwrap()
        .overlay_file(id, file)
        .unwrap()
        .is_some()
}

/// A workspace with `removed.py` published, then removed in an apply whose store is held past
/// the write deadline (each busy attempt spends the whole 1 s at once): the removal waits in
/// `pending_removals` and the overlay row stays. Returns the held writer, which the caller drops
/// to free the store, and the workspace's id.
fn workspace_with_a_removal_a_busy_store_kept(
    repository: &Repository,
    store: &Path,
) -> (
    CodeRuleDslWorkspace,
    HeldWriter,
    crate::content_store::WorkspaceId,
) {
    let workspace = attached_workspace(repository, store, &needle_code_rule());
    let id = workspace
        .state
        .lock()
        .unwrap()
        .storage
        .as_ref()
        .unwrap()
        .workspace_id;
    assert!(apply_files(&workspace, 1, &[("removed.py", "value = 1\n")]).is_empty());
    assert!(overlay_row_exists(store, id, "removed.py"));
    let writer = HeldWriter::hold(store);
    let _spent = spend_deadline_on_busy_attempts(Duration::from_secs(1), || {});
    assert!(apply_changes(&workspace, 2, &[], &["removed.py"]).is_empty());
    assert_eq!(
        workspace.state.lock().unwrap().pending_removals,
        ["removed.py".to_owned()].into(),
        "the busy store kept the removal"
    );
    assert!(overlay_row_exists(store, id, "removed.py"));
    BUSY_ATTEMPT_HOOK.set(None);
    (workspace, writer, id)
}

/// The removals a busy store kept are deleted by the next apply that finds the store free, though
/// that apply publishes nothing: no later apply need publish for the rows to be collected.
#[test]
fn an_apply_that_publishes_nothing_deletes_the_removals_a_busy_store_kept() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-removal-flush");
    let store = repository.store("content.sqlite");
    let (workspace, writer, id) = workspace_with_a_removal_a_busy_store_kept(&repository, &store);
    drop(writer);
    window.pass_window();

    assert!(apply_changes(&workspace, 3, &[], &[]).is_empty());
    assert_eq!(LAST_APPLY_UNPUBLISHED_FILES.get(), 0);
    assert!(
        !overlay_row_exists(&store, id, "removed.py"),
        "the apply that found the store free deleted the removed file's row"
    );
    assert!(workspace.state.lock().unwrap().pending_removals.is_empty());
    workspace.close();
}

/// Closing a workspace deletes the removals a busy store kept once, if the store is free: the
/// rows would otherwise survive a re-registration at the same root and never be collected.
#[test]
fn closing_a_workspace_deletes_the_removals_a_busy_store_kept() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let repository = Repository::new("busy-removal-close");
    let store = repository.store("content.sqlite");
    let (workspace, writer, id) = workspace_with_a_removal_a_busy_store_kept(&repository, &store);
    drop(writer);

    workspace.close();
    assert!(
        !overlay_row_exists(&store, id, "removed.py"),
        "close with a free store deleted the removed file's row"
    );
}

/// Closing never waits for another writer: with the store held it makes its one attempt, which
/// gives up at once, and the workspace still closes (the rows stay for the store's collection).
#[test]
fn closing_a_workspace_does_not_wait_for_a_writer_holding_the_store() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let repository = Repository::new("busy-removal-close-held");
    let store = repository.store("content.sqlite");
    let (workspace, writer, id) = workspace_with_a_removal_a_busy_store_kept(&repository, &store);
    let waits = Rc::new(std::cell::Cell::new(0_usize));
    let counted = Rc::clone(&waits);
    BUSY_WAIT_HOOK.set(Some(Box::new(move |_| counted.set(counted.get() + 1))));

    workspace.close();
    assert_eq!(waits.get(), 0, "close waited for the writer");
    assert!(workspace.state.lock().unwrap().storage.is_none());
    drop(writer);
    assert!(overlay_row_exists(&store, id, "removed.py"));
}

/// An attach spends one write deadline in all across its writes (its Git trees, then the
/// snapshot), not one per write. Each busy attempt spends 600 ms of the 1 s deadline without
/// sleeping it: a writer that lets go in the tree write's wait and is replaced before the
/// snapshot write leaves that write one busy attempt, where a deadline of its own would allow two.
#[test]
fn an_attach_waits_out_a_writer_for_one_deadline_across_its_writes() {
    let _slot = test_slot();
    let spent = Duration::from_millis(600);
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let repository = Repository::new("busy-attach-one-deadline");
    let store = repository.store("content.sqlite");
    drop(ContentStore::open(&store).unwrap());
    let first = Rc::new(RefCell::new(Some(HeldWriter::hold(&store))));
    let released = Rc::clone(&first);
    let seen = spend_deadline_on_busy_attempts(spent, move || drop(released.borrow_mut().take()));
    let second = Rc::new(RefCell::new(None));
    let taken = Rc::clone(&second);
    let held = store.clone();
    AFTER_GIT_TREE_TRANSACTION.set(Some(Box::new(move || {
        *taken.borrow_mut() = Some(HeldWriter::hold(&held));
    })));
    let workspace = CodeRuleDslWorkspace::new();
    let outcome = workspace
        .storage_task(repository.root(), store_argument(&store))
        .unwrap()
        .compute();
    AFTER_GIT_TREE_TRANSACTION.set(None);
    workspace.close();
    assert!(
        matches!(
            &outcome,
            Ok(StorageAttachment { attached: false, reason: Some(reason), .. })
                if reason.ends_with("while publishing the Git snapshot")
        ),
        "{outcome:?}"
    );
    assert!(
        second.borrow().is_some(),
        "the snapshot write found the store held"
    );
    let seen = seen.borrow();
    assert_eq!(
        seen.len(),
        2,
        "one busy attempt for the trees, one for the snapshot: {seen:?}"
    );
    assert!(
        seen[1] >= spent,
        "the snapshot write starts from what the tree write spent: {seen:?}"
    );
}

/// The deadline an attach spends covers the store open too: a fresh store's schema setup finds
/// a writer that lets go after spending 600 ms of the 1 s deadline, and the snapshot write that
/// finds another writer afterwards starts from that spent share, not from nothing.
#[test]
fn an_attach_shares_one_deadline_between_opening_the_store_and_publishing() {
    let _slot = test_slot();
    let spent = Duration::from_millis(600);
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let repository = Repository::new("busy-attach-open-one-deadline");
    let store = repository.store("fresh.sqlite");
    let first = Rc::new(RefCell::new(Some(HeldWriter::hold(&store))));
    let released = Rc::clone(&first);
    let seen = spend_deadline_on_busy_attempts(spent, move || drop(released.borrow_mut().take()));
    let second = Rc::new(RefCell::new(None));
    let taken = Rc::clone(&second);
    let held = store.clone();
    AFTER_GIT_TREE_TRANSACTION.set(Some(Box::new(move || {
        *taken.borrow_mut() = Some(HeldWriter::hold(&held));
    })));
    let workspace = CodeRuleDslWorkspace::new();
    let outcome = workspace
        .storage_task(repository.root(), store_argument(&store))
        .unwrap()
        .compute();
    AFTER_GIT_TREE_TRANSACTION.set(None);
    workspace.close();
    assert!(
        matches!(
            &outcome,
            Ok(StorageAttachment { attached: false, reason: Some(reason), .. })
                if reason.ends_with("while publishing the Git snapshot")
        ),
        "{outcome:?}"
    );
    let seen = seen.borrow();
    assert_eq!(
        seen.len(),
        2,
        "one busy attempt for the open, one for the snapshot: {seen:?}"
    );
    assert!(
        seen[1] >= spent,
        "the snapshot write starts from what the open spent: {seen:?}"
    );
}

/// Shrinks this thread's resident budget for unpublished files while it lives.
pub(super) struct UnpublishedBudget;

impl UnpublishedBudget {
    pub(super) fn set(bytes: usize) -> Self {
        TEST_UNPUBLISHED_RESIDENT_BYTES.set(Some(bytes));
        Self
    }
}

impl Drop for UnpublishedBudget {
    fn drop(&mut self) {
        TEST_UNPUBLISHED_RESIDENT_BYTES.set(None);
    }
}

/// The source and tape bytes the unpublished files of `workspace` hold resident.
fn unpublished_resident_bytes(workspace: &CodeRuleDslWorkspace) -> usize {
    workspace
        .state
        .lock()
        .unwrap()
        .files
        .values()
        .filter(|file| file.unpublished)
        .map(|file| {
            file.input.source.len()
                + file
                    .tape
                    .as_ref()
                    .map_or(0, |(_, image)| image.estimated_bytes())
        })
        .sum()
}

/// The paths of the files `workspace` freed down to their results.
fn results_only_paths(workspace: &CodeRuleDslWorkspace) -> Vec<String> {
    workspace
        .state
        .lock()
        .unwrap()
        .files
        .iter()
        .filter(|(_, file)| file.results_only)
        .map(|(path, _)| path.clone())
        .collect()
}

/// A store that stays busy through many applies does not make the workspace hold every file's
/// source: past the budget a file is freed down to its results, which still page. The files
/// under the budget publish once the window has passed; the freed ones stay resident with their
/// results (the store never got their source).
#[test]
fn a_store_that_stays_busy_does_not_keep_every_unpublished_source_resident() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    // "NEEDLE\n" is 7 bytes: two files fit.
    let budget = 20;
    let _budget = UnpublishedBudget::set(budget);
    let repository = Repository::new("busy-unpublished-budget");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let writer = HeldWriter::hold(&store);
    let _spent = spend_deadline_on_busy_attempts(Duration::from_secs(1), || {});
    let needle = "NEEDLE\n";
    assert_eq!(
        apply_files(
            &workspace,
            1,
            &[("a.py", needle), ("b.py", needle), ("c.py", needle)]
        ),
        ["NEEDLE"; 3]
    );
    assert!(unpublished_resident_bytes(&workspace) <= budget);
    assert_eq!(
        apply_files(
            &workspace,
            2,
            &[("d.py", needle), ("e.py", needle), ("f.py", needle)]
        ),
        ["NEEDLE"; 3]
    );
    assert!(unpublished_resident_bytes(&workspace) <= budget);
    assert_eq!(
        results_only_paths(&workspace),
        ["c.py", "d.py", "e.py", "f.py"],
        "the files past the budget are freed down to their results"
    );
    assert_eq!(paged_codes(&workspace), ["NEEDLE"; 6]);

    drop(writer);
    BUSY_ATTEMPT_HOOK.set(None);
    window.pass_window();
    assert!(apply_files(&workspace, 3, &[("g.py", "value = 1\n")]).is_empty());
    {
        let state = workspace.state.lock().unwrap();
        for path in ["a.py", "b.py", "g.py"] {
            let file = &state.files[path];
            assert!(
                !file.unpublished && file.stored.is_some(),
                "{path} within the budget publishes after the window"
            );
        }
        for path in ["c.py", "d.py", "e.py", "f.py"] {
            let file = &state.files[path];
            assert!(
                file.unpublished
                    && file.stored.is_none()
                    && file.input.source.is_empty()
                    && file.tape.is_none(),
                "{path} stays results-only"
            );
        }
    }
    assert_eq!(paged_codes(&workspace), ["NEEDLE"; 6]);
    assert!(workspace.evict_inactive().unwrap());
    assert_eq!(
        workspace
            .state
            .lock()
            .unwrap()
            .files
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["c.py", "d.py", "e.py", "f.py"],
        "eviction keeps the results the store does not hold"
    );
    assert_eq!(paged_codes(&workspace), ["NEEDLE"; 6]);
    workspace.close();
}

/// A package marker that changes a freed file's module needs its source, which nothing can supply
/// (the worktree has no such file): the apply still succeeds, and the audit lists the file's
/// clauses as incomplete instead of the busy store failing the apply. That is no syntax gap: the
/// file is no blind file, so its failures stay listed and fail the audit.
#[test]
fn a_freed_file_nothing_can_reread_is_reported_incomplete_not_failed_by_a_package_marker() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let _budget = UnpublishedBudget::set(0);
    let repository = Repository::new("busy-unpublished-marker");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let _writer = HeldWriter::hold(&store);
    let _spent = spend_deadline_on_busy_attempts(Duration::from_secs(1), || {});
    assert_eq!(
        apply_files(&workspace, 1, &[("pkg/mod.py", "NEEDLE\n")]),
        ["NEEDLE"]
    );
    assert_eq!(results_only_paths(&workspace), ["pkg/mod.py"]);

    let response = apply_value(&workspace, 2, &[("pkg/__init__.py", "")], &[]);
    assert!(
        response["replacedPaths"]
            .as_array()
            .unwrap()
            .contains(&json!("pkg/mod.py")),
        "{response}"
    );
    let failures = response["evaluation"]["failures"].as_array().unwrap();
    assert!(
        failures
            .iter()
            .any(|failure| failure["path"] == "pkg/mod.py"
                && failure["code"] == "source-incomplete"
                && failure["message"]
                    .as_str()
                    .is_some_and(|message| message.starts_with("source released while"))),
        "{response}"
    );
    assert!(
        response["evaluation"]["violations"]
            .as_array()
            .unwrap()
            .is_empty(),
        "the dropped answer is not reported: {response}"
    );
    assert_eq!(response["blindFiles"], json!([]), "{response}");
    workspace.close();
}

/// A file freed down to its results has its source read back from the worktree while that still
/// hashes to the version evaluated; a worktree that changed is an error, never another version.
#[test]
fn a_freed_files_source_is_read_back_only_while_the_worktree_still_has_it() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let _budget = UnpublishedBudget::set(0);
    let repository = Repository::new("busy-unpublished-reread");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let root = PathBuf::from(repository.root());
    std::fs::write(root.join("same.py"), "NEEDLE\n").unwrap();
    std::fs::write(root.join("changed.py"), "NEEDLE = 2\n").unwrap();
    let _writer = HeldWriter::hold(&store);
    let _spent = spend_deadline_on_busy_attempts(Duration::from_secs(1), || {});
    assert_eq!(
        apply_files(
            &workspace,
            1,
            &[("same.py", "NEEDLE\n"), ("changed.py", "NEEDLE\n")]
        ),
        ["NEEDLE"; 2]
    );
    assert_eq!(results_only_paths(&workspace), ["changed.py", "same.py"]);

    let mut state = workspace.state.lock().unwrap();
    let (_, document) = super::documents::load(&mut state, "same.py").unwrap();
    assert_eq!(document.source.as_str(), "NEEDLE\n");
    assert!(super::documents::load(&mut state, "changed.py").is_err());
    drop(state);
    workspace.close();
}

/// A completion publishes the files a busy store left unpublished under the same write deadline
/// as its other writes: the publication spends 600 ms of the 1 s deadline on a writer that lets
/// go, another writer takes the store, and the next write finds the share spent instead of
/// starting a deadline of its own. The workspace lock is held meanwhile, so an apply waiting on it
/// waits one deadline, not one per write.
#[test]
fn a_completion_waits_out_other_writers_for_one_deadline_in_all() {
    let _slot = test_slot();
    let spent = Duration::from_millis(600);
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-completion-one-deadline");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let writer = HeldWriter::hold(&store);
    let _spent = spend_deadline_on_busy_attempts(Duration::from_secs(1), || {});
    assert!(apply_files(&workspace, 1, &[("one.py", "value = 1\n")]).is_empty());
    assert!(is_unpublished(&workspace, "one.py"));
    drop(writer);
    window.pass_window();

    let first = Rc::new(RefCell::new(Some(HeldWriter::hold(&store))));
    let released = Rc::clone(&first);
    let seen = spend_deadline_on_busy_attempts(spent, move || drop(released.borrow_mut().take()));
    let second = Rc::new(RefCell::new(None));
    let taken = Rc::clone(&second);
    let held = store.clone();
    AFTER_FILES_PUBLISHED.set(Some(Box::new(move || {
        *taken.borrow_mut() = Some(HeldWriter::hold(&held));
    })));
    let error = workspace.completion_task().compute().unwrap_err();
    assert_eq!(error.reason, "content store writer is busy");
    assert!(first.borrow().is_none(), "the first writer let go");
    assert!(
        second.borrow().is_some(),
        "the next write found the store held"
    );
    let seen = seen.borrow();
    assert_eq!(
        seen.len(),
        2,
        "one busy attempt for the publication, one for the write after it: {seen:?}"
    );
    assert!(
        seen[1] >= spent,
        "the next write starts from what the publication spent: {seen:?}"
    );
    drop(second);
    workspace.close();
}

/// The workspace says on stderr that the store is busy once per busy period: a publication that
/// succeeds after the window passed ends the period, so a later one is reported again.
#[test]
fn a_later_busy_period_is_reported_again() {
    let _slot = test_slot();
    let _deadline = BusyDeadline::set(Duration::from_secs(1));
    let window = BusyWindowClock;
    let repository = Repository::new("busy-report-again");
    let store = repository.store("content.sqlite");
    let workspace = attached_workspace(&repository, &store, &needle_code_rule());
    let reported = || workspace.state.lock().unwrap().store_busy_reported;
    assert!(!reported());

    let writer = HeldWriter::hold(&store);
    let _spent = spend_deadline_on_busy_attempts(Duration::from_secs(1), || {});
    assert_eq!(apply_source(&workspace, 1, "NEEDLE\n"), ["NEEDLE"]);
    assert!(reported(), "the busy period is reported");
    drop(writer);

    assert_eq!(apply_source(&workspace, 2, "NEEDLE = 2\n"), ["NEEDLE"]);
    assert!(
        reported(),
        "inside the window the period is still the same one"
    );
    window.pass_window();
    assert_eq!(apply_source(&workspace, 3, "NEEDLE = 3\n"), ["NEEDLE"]);
    assert!(
        !reported(),
        "a publication after the window ends the busy period"
    );

    let writer = HeldWriter::hold(&store);
    assert_eq!(apply_source(&workspace, 4, "NEEDLE = 4\n"), ["NEEDLE"]);
    assert!(reported(), "the next busy period is reported again");
    drop(writer);
    workspace.close();
}
