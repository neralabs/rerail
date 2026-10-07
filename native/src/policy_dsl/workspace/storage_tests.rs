//! Storage attachment: when a workspace may attach its content store, which store the attach
//! opens, and what a close that lands mid-attach leaves behind.

use super::api::{ON_STORAGE_ATTACHED, PolicyDslWorkspace, StorageAttachment, WorkspaceApplyTask};
use super::storage::storage_context_in;
use super::tests::{WORKSPACE_TEST_SLOT, git};
use crate::content_store::{
    AFTER_GIT_TREE_TRANSACTION, CLOCK_OFFSET, ContentStore, MAX_COLLECTION_BATCH, STORE_FILE_NAME,
};
use crate::policy_dsl::{CompiledPolicyDsl, PolicyDslRuntime};
use napi::Task;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn needle_policy() -> CompiledPolicyDsl {
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
    });
    let policy = json!({
        "format": "rerail.policy/v1", "id": "storage-needle", "revision": 1,
        "scope": { "include": ["**/*.py"], "exclude": [] },
        "checks": [{
            "id": "needle", "finding": { "anchor": "witness.span", "code": "NEEDLE",
                "message": "found needle", "severity": "error" },
            "forbid": { "regex": { "pattern": "NEEDLE", "source": "file.content" } }
        }]
    });
    PolicyDslRuntime::compile(policy.to_string(), "json".into(), capabilities.to_string()).unwrap()
}

fn apply_source(workspace: &PolicyDslWorkspace, revision: u32, source: &str) -> Vec<String> {
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({
            "revision": revision,
            "files": [{ "path": "edited.py", "language": "python", "source": source }],
            "removed": [],
        })
        .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let result: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    result["evaluation"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["code"].as_str().unwrap().to_owned())
        .collect()
}

/// One failing storage test must not poison the slot for the others.
fn test_slot() -> MutexGuard<'static, ()> {
    WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn store_argument(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn storage_attaches_after_policy_registration_until_the_first_apply() {
    let _slot = test_slot();
    let repository = Repository::new("after-registration");
    let store = repository.store("content.sqlite");

    let mut registered = PolicyDslWorkspace::new();
    registered.add_policy(&needle_policy()).unwrap();
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

    let mut applied = PolicyDslWorkspace::new();
    applied.add_policy(&needle_policy()).unwrap();
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
    let workspace = PolicyDslWorkspace::new();
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
    let workspace = Arc::new(PolicyDslWorkspace::new());
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

/// The previous release stamped its machine-wide store at floor 16 and refuses any store above
/// its floor, so the store the managed build names in that directory must never be that file.
#[test]
fn default_storage_leaves_an_older_binary_store_on_its_floor() {
    let _slot = test_slot();
    let repository = Repository::new("older-binary-store");
    let home = repository.store("home");
    let older = home.join(".cache/nera/rerail/content-v2.sqlite");
    std::fs::create_dir_all(older.parent().unwrap()).unwrap();
    drop(ContentStore::open(&older).unwrap());
    {
        let connection = rusqlite::Connection::open(&older).unwrap();
        // Floor 16's git_trees held only the id and the hash.
        let added = connection
            .prepare("SELECT name FROM pragma_table_info('git_trees') WHERE cid > 1")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for column in added {
            connection
                .execute_batch(&format!("ALTER TABLE git_trees DROP COLUMN {column}"))
                .unwrap();
        }
        connection
            .execute_batch(
                "UPDATE store_schema_revision SET revision=16 WHERE singleton=1;
                 PRAGMA user_version=16;",
            )
            .unwrap();
    }

    let managed = home.join(".cache/nera/rerail").join(STORE_FILE_NAME);
    let attached = storage_context_in(&repository.root(), managed);
    assert!(attached.is_ok(), "{:?}", attached.err());
    let floor: i64 = rusqlite::Connection::open(&older)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(
        floor, 16,
        "attaching the default store raised the older binary's store above the floor it accepts"
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
    let workspace = PolicyDslWorkspace::new();
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
    let workspace = PolicyDslWorkspace::new();
    let outcome = format!(
        "{:?}",
        workspace
            .storage_task(
                outside.to_string_lossy().into_owned(),
                store_argument(&store),
            )
            .unwrap()
            .compute()
    );
    workspace.close();
    assert_eq!(
        outcome, "Ok(StorageAttachment { attached: false, reason: None })",
        "a root outside any repository is expected to attach no store"
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

    let missing = PolicyDslWorkspace::new();
    let refused = missing.storage_task(repository.root(), String::new());
    missing.close();
    assert!(
        refused.is_err(),
        "an attach without a store path must be refused, not defaulted"
    );

    let workspace = PolicyDslWorkspace::new();
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
    let bare = PolicyDslWorkspace::new();
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
