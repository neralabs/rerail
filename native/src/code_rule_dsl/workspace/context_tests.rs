//! Stored-workspace contracts from Phase A of the integrated query plan: every ingestion path
//! carries the same file context, persisted reuse never crosses file context, transient failures
//! are retried, and paged results agree with complete results.

use super::super::CodeRuleDslRuntime;
use super::super::matcher::UtilLimit;
use super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask};
use super::index::{WorkspaceIndexTask, repository_module};
use super::ingest::{BINARY_REQUEST_MAGIC, PACKAGE_CONTEXT_PATH_VISITS};
use super::persisted::{decode_local_result, encode_local_result};
use super::storage::{StorageContext, storage_context_in};
use super::tests::{WORKSPACE_TEST_SLOT, git, result_page};
use super::*;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::content_store::{ContentStore, GitTree, HashKey};
use crate::git_workspace;
use napi::Task;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc as StdArc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

const CAPABILITIES: &str = r#"{
    "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
        "languages": { "python": "tree-sitter-python@0.23",
            "typescript": "tree-sitter-typescript@0.23",
            "javascript": "tree-sitter-javascript@0.23",
            "php": "tree-sitter-php@0.22" } },
    "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
        "maxFiles": 1000, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
        "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
}"#;

/// The largest result page `resultPageAsync` accepts.
const MAXIMUM_PAGE_BYTES: u32 = 64 * 1024 * 1024;

/// `from . import helper` resolves to `alpha.helper` only in a module of the `alpha` package.
const PACKAGE_SOURCE: &str = "from . import helper\nhelper()\n";

fn compile(id: &str, clauses: Value) -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": id, "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": clauses,
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}

fn module_code_rule() -> CompiledCodeRuleDsl {
    compile(
        "module-context",
        json!([{
            "id": "alpha-helper",
            "violation": { "anchor": "witness.span", "code": "CTX", "message": "alpha helper" },
            "forbid": { "ast": { "language": "python", "rule": {
                "kind": "identifier", "regex": "^helper$",
                "resolves": { "import": ["alpha.helper"] } } } }
        }]),
    )
}

/// A structural probe with no file condition: directory reuse restores its results unread.
fn helper_call_code_rule() -> CompiledCodeRuleDsl {
    compile(
        "helper-call",
        json!([{
            "id": "helper-call",
            "violation": { "anchor": "witness.span", "code": "CTX", "message": "helper call" },
            "forbid": { "ast": { "language": "python", "rule": {
                "kind": "identifier", "regex": "^helper$", "inside": { "kind": "call" } } } }
        }]),
    )
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rerail-context-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn store(&self) -> PathBuf {
        self.0.join("store").join("content.sqlite")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A committed repository with two packages holding identical module source and one loose file.
fn package_repository(tag: &str) -> TempDir {
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

fn apply_json(workspace: &CodeRuleDslWorkspace, revision: u32, files: Value) -> Value {
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": revision, "files": files, "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    serde_json::from_str(&task.compute().unwrap()).unwrap()
}

/// The binary request the host sends: JSON metadata followed by each file's UTF-8 source.
fn buffer_request(revision: u32, files: &Value) -> Vec<u8> {
    let mut sources = Vec::new();
    let metadata = files
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            let source = file["source"].as_str().unwrap();
            sources.extend_from_slice(source.as_bytes());
            let mut entry = json!({
                "path": file["path"], "language": file["language"], "sourceBytes": source.len(),
            });
            if let Some(module) = file.get("module") {
                entry["module"] = module.clone();
            }
            entry
        })
        .collect::<Vec<_>>();
    let metadata = json!({ "revision": revision, "files": metadata, "removed": [] }).to_string();
    let mut buffer = Vec::new();
    buffer.extend_from_slice(BINARY_REQUEST_MAGIC);
    buffer.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    buffer.extend_from_slice(metadata.as_bytes());
    buffer.extend_from_slice(&sources);
    buffer
}

fn apply_buffer(workspace: &CodeRuleDslWorkspace, revision: u32, files: &Value) -> Result<Value> {
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: String::new(),
        request_buffer: Some(buffer_request(revision, files)),
        request_direct: None,
    };
    Ok(serde_json::from_str(&task.compute()?).unwrap())
}

/// `(path, clause, start byte)` for every violation, in the order reported.
fn located(violations: &Value) -> Vec<(String, String, u64)> {
    violations
        .as_array()
        .unwrap()
        .iter()
        .map(|violation| {
            (
                violation["path"].as_str().unwrap().to_owned(),
                violation["clauseId"].as_str().unwrap().to_owned(),
                violation["span"]["startByte"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn all_pages(workspace: &CodeRuleDslWorkspace, maximum_bytes: u32) -> Value {
    let mut violations = Vec::new();
    let mut cursor = 0;
    loop {
        let page: Value =
            serde_json::from_str(&result_page(workspace, cursor, maximum_bytes).unwrap()).unwrap();
        violations.extend(page["violations"].as_array().unwrap().iter().cloned());
        if page["done"].as_bool().unwrap() {
            return Value::Array(violations);
        }
        cursor = page["cursor"].as_u64().unwrap() as u32;
    }
}

/// A workspace over an explicit store, registered as `root` without Git discovery.
fn stored_workspace(
    store: &StdArc<ContentStore>,
    root: &str,
    code_rule: &CompiledCodeRuleDsl,
) -> CodeRuleDslWorkspace {
    let root_tree = HashKey::git_sha1([7; 20]);
    store
        .put_git_trees_bulk(&[GitTree {
            id: root_tree,
            prefix: String::new(),
            entries: Vec::new(),
        }])
        .unwrap();
    let snapshot = store
        .put_snapshot(
            root_tree,
            1,
            HashKey::sha256(b"rerail-grammars-v1"),
            i64::from(tape::TAPE_FORMAT_VERSION),
        )
        .unwrap();
    let owner = format!("context-test:{root}:{}", std::process::id());
    let workspace_id = store
        .open_workspace(root, "repository", &owner, snapshot, 1, i64::MAX)
        .unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.state.lock().unwrap().storage = Some(StorageContext {
        store: StdArc::clone(store),
        repository: None,
        repository_prefix: String::new(),
        workspace_id,
        snapshot_id: snapshot,
        owner,
        _maintenance: None,
    });
    workspace.add_code_rule(code_rule, None).unwrap();
    workspace
}

fn package_files(first: &str) -> Value {
    let alpha = json!({ "path": "alpha/x.py", "language": "python", "source": PACKAGE_SOURCE,
        "module": "alpha.x" });
    let beta = json!({ "path": "beta/x.py", "language": "python", "source": PACKAGE_SOURCE,
        "module": "beta.x" });
    if first == "alpha" {
        json!([alpha, beta])
    } else {
        json!([beta, alpha])
    }
}

fn expected_package_violations() -> Vec<(String, String, u64)> {
    vec![("alpha/x.py".to_owned(), "alpha-helper".to_owned(), 21)]
}

#[test]
fn hosts_and_repository_ingestion_share_one_python_module_derivation() {
    let repository = package_repository("module-path");
    let root = repository.0.join("repo");
    let module = |path: &str| match super::super::python_module_path(
        root.join(path).to_string_lossy().into_owned(),
    ) {
        Some(napi::Either::A(dotted)) => Some(dotted),
        Some(napi::Either::B(names)) => Some(names.join("|")),
        None => None,
    };
    assert_eq!(module("alpha/x.py").as_deref(), Some("alpha.x"));
    assert_eq!(
        module("alpha/__init__.py").as_deref(),
        Some("alpha.__init__")
    );
    assert_eq!(module("loose/x.py"), None);
    let repository_module =
        |path: &str| repository_module(&git_workspace::discover(&root).unwrap(), path, "python");
    assert_eq!(repository_module("beta/x.py"), Some(json!("beta.x")));
    assert_eq!(repository_module("loose/x.py"), None);
}

fn package_marker_code_rule() -> CompiledCodeRuleDsl {
    compile(
        "package-marker-context",
        json!([{
            "id": "pkg-helper",
            "violation": { "anchor": "witness.span", "code": "CTX", "message": "pkg helper" },
            "forbid": { "ast": { "language": "python", "rule": {
                "kind": "identifier", "regex": "^helper$",
                "resolves": { "import": ["pkg.helper"] } } } }
        }]),
    )
}

/// A repository index as the host runs it: every chunk, then the closing apply that evaluates the
/// deferred global clauses. Returns the last chunk, whose counts cover the whole index.
fn index_workspace(workspace: &CodeRuleDslWorkspace) -> Value {
    let last = index_until_done(workspace, u32::MAX).pop().unwrap();
    let revision = u32::try_from(last["revision"].as_u64().unwrap()).unwrap() + 1;
    apply_json(workspace, revision, json!([]));
    last
}

/// A host request carrying `files` and removing `removed`, as the bridge sends an edit.
fn apply_host(workspace: &CodeRuleDslWorkspace, revision: u32, files: Value, removed: &[&str]) {
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": revision, "files": files, "removed": removed })
            .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    task.compute().unwrap();
}

fn commit(repository: &std::path::Path, message: &str) {
    git(repository, &["add", "-A"]);
    git(
        repository,
        &[
            "-c",
            "user.name=rerail",
            "-c",
            "user.email=rerail@example.invalid",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

#[test]
fn package_marker_changes_refresh_unchanged_siblings_and_match_a_fresh_index() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("package-marker-refresh");
    let root = directory.0.join("repo");
    let sibling = root.join("pkg/mod.py");
    std::fs::create_dir_all(sibling.parent().unwrap()).unwrap();
    std::fs::write(root.join("pkg/helper.py"), "def helper():\n    pass\n").unwrap();
    std::fs::write(&sibling, PACKAGE_SOURCE).unwrap();
    git(&root, &["init", "-q"]);
    commit(&root, "initial");
    let code_rule = package_marker_code_rule();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), directory.store()).unwrap());
    workspace.add_code_rule(&code_rule, None).unwrap();
    index_workspace(&workspace);
    assert_eq!(all_pages(&workspace, MAXIMUM_PAGE_BYTES), json!([]));
    workspace.completion_task().compute().unwrap();
    assert!(workspace.evict_inactive().unwrap());

    let fresh_results = |tag: &str| {
        let mut fresh = CodeRuleDslWorkspace::new();
        fresh.state.lock().unwrap().storage = Some(
            storage_context_in(
                root.to_str().unwrap(),
                directory.0.join(format!("{tag}.sqlite")),
            )
            .unwrap(),
        );
        fresh.add_code_rule(&code_rule, None).unwrap();
        index_workspace(&fresh);
        let results = all_pages(&fresh, MAXIMUM_PAGE_BYTES);
        fresh.close();
        results
    };
    let assert_fresh = |tag: &str| {
        assert_eq!(
            all_pages(&workspace, MAXIMUM_PAGE_BYTES),
            fresh_results(tag),
            "incremental results after {tag} equal a fresh workspace index"
        );
    };

    std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
    commit(&root, "create package marker");
    let marker = |path: &str| json!([{ "path": path, "language": "python", "source": "" }]);
    apply_host(&workspace, 10, marker("pkg/__init__.py"), &[]);
    assert_eq!(
        located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES)),
        vec![("pkg/mod.py".to_owned(), "pkg-helper".to_owned(), 21)],
        "creating the marker refreshes the unchanged sibling"
    );
    assert_fresh("after-create");

    std::fs::create_dir_all(root.join("other")).unwrap();
    std::fs::rename(root.join("pkg/__init__.py"), root.join("other/__init__.py")).unwrap();
    commit(&root, "rename package marker");
    apply_host(
        &workspace,
        11,
        marker("other/__init__.py"),
        &["pkg/__init__.py"],
    );
    assert_eq!(all_pages(&workspace, MAXIMUM_PAGE_BYTES), json!([]));
    assert_fresh("after-rename");

    std::fs::remove_file(root.join("other/__init__.py")).unwrap();
    commit(&root, "delete package marker");
    apply_host(&workspace, 12, json!([]), &["other/__init__.py"]);
    assert_eq!(all_pages(&workspace, MAXIMUM_PAGE_BYTES), json!([]));
    assert_fresh("after-delete");
    workspace.close();
}

#[test]
fn directory_reuse_restores_results_and_evidence_on_a_second_workspace() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = package_repository("directory-reuse");
    let root = directory.0.join("repo");
    let code_rule = helper_call_code_rule();
    let mut first = CodeRuleDslWorkspace::new();
    first.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), directory.store()).unwrap());
    first.add_code_rule(&code_rule, None).unwrap();
    let cold = index_workspace(&first);
    let expected = all_pages(&first, MAXIMUM_PAGE_BYTES);
    first.completion_task().compute().unwrap();
    first.close();

    let mut second = CodeRuleDslWorkspace::new();
    second.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), directory.store()).unwrap());
    second.add_code_rule(&code_rule, None).unwrap();
    let reused = index_workspace(&second);
    assert_eq!(all_pages(&second, MAXIMUM_PAGE_BYTES), expected);
    assert!(
        cold["readFiles"].as_u64().unwrap() > 0,
        "the first workspace reads the cold repository"
    );
    assert!(
        reused["restoredFiles"].as_u64().unwrap() > 0,
        "the second workspace restores a clean completed directory without source reads"
    );
    assert_eq!(
        reused["readFiles"].as_u64().unwrap(),
        0,
        "every scoped file came from the reusable directory"
    );
    second.close();
}

/// A completed first index of a repository holding `edited` and the clean `lib/b.py`, then
/// `hide` edits `edited` to print in a way Git status may not report, then a second index on
/// the same store: its index response and violations.
fn warm_index_after_hidden_edit(
    tag: &str,
    edited: &str,
    hide: impl FnOnce(&std::path::Path),
) -> (Value, Vec<(String, String, u64)>) {
    let directory = committed_repository(tag, &[(edited, b"x = 1\n"), ("lib/b.py", b"x = 2\n")]);
    let root = directory.0.join("repo");
    let code_rule = print_code_rule();
    let mut first = storage_workspace(&root, directory.store());
    first.add_code_rule(&code_rule, None).unwrap();
    index_workspace(&first);
    assert_eq!(all_pages(&first, MAXIMUM_PAGE_BYTES), json!([]));
    first.completion_task().compute().unwrap();
    first.close();

    hide(&root);
    let mut second = storage_workspace(&root, directory.store());
    second.add_code_rule(&code_rule, None).unwrap();
    let indexed = index_workspace(&second);
    let violations = located(&all_pages(&second, MAXIMUM_PAGE_BYTES));
    second.close();
    (indexed, violations)
}

fn assert_hidden_edit_is_read(tag: &str, edited: &str, hide: impl FnOnce(&std::path::Path)) {
    let (indexed, violations) = warm_index_after_hidden_edit(tag, edited, hide);
    assert_eq!(
        violations,
        vec![(edited.to_owned(), "print".to_owned(), 0)],
        "a warm index evaluates the edited bytes, not the committed blob's stored results: {indexed}"
    );
    assert_eq!(
        indexed["restoredFiles"],
        json!(1),
        "only the clean sibling directory is restored: {indexed}"
    );
}

#[test]
fn a_warm_index_reads_an_assume_unchanged_edit() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    assert_hidden_edit_is_read("assume-unchanged", "api/a.py", |root| {
        git(root, &["update-index", "--assume-unchanged", "api/a.py"]);
        std::fs::write(root.join("api/a.py"), "print(1)\n").unwrap();
    });
}

#[test]
fn a_warm_index_reads_a_skip_worktree_edit() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    assert_hidden_edit_is_read("skip-worktree", "api/a.py", |root| {
        git(root, &["update-index", "--skip-worktree", "api/a.py"]);
        std::fs::write(root.join("api/a.py"), "print(1)\n").unwrap();
    });
}

#[test]
fn a_warm_index_reads_an_edit_under_a_case_variant_builtin_directory_when_git_ignores_case() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    assert_hidden_edit_is_read("ignorecase-builtin", "api/Tmp/a.py", |root| {
        git(root, &["config", "core.ignorecase", "true"]);
        std::fs::write(root.join("api/Tmp/a.py"), "print(1)\n").unwrap();
    });
}

#[test]
fn an_interactive_apply_runs_while_a_repository_index_plans_its_restore() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = package_repository("plan-interleave");
    let root = directory.0.join("repo");
    let code_rule = helper_call_code_rule();
    let mut warm = storage_workspace(&root, directory.store());
    warm.add_code_rule(&code_rule, None).unwrap();
    index_workspace(&warm);
    warm.completion_task().compute().unwrap();
    warm.close();

    let mut scan = storage_workspace(&root, directory.store());
    scan.add_code_rule(&code_rule, None).unwrap();
    let mut session = CodeRuleDslWorkspace::new();
    session.add_code_rule(&code_rule, None).unwrap();
    let interactive = std::rc::Rc::new(std::cell::RefCell::new(None));
    let mut edit = WorkspaceApplyTask {
        state: Arc::clone(&session.state),
        closed: Arc::clone(&session.closed),
        request_json: json!({ "revision": 1, "removed": [], "files": [
            { "path": "edit.py", "language": "python", "source": "helper()\n" }] })
        .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let landed = std::rc::Rc::clone(&interactive);
    super::index::gate_next_plan(move || *landed.borrow_mut() = Some(edit.compute()));
    let first_chunk = index_chunk(&scan, u32::MAX);
    let applied = interactive
        .borrow_mut()
        .take()
        .expect("the first chunk plans the index");
    let applied: Value = serde_json::from_str(&applied.unwrap_or_else(|error| {
        panic!("an interactive apply requested before the restore must run at once: {error}")
    }))
    .unwrap();
    assert_eq!(
        located(&applied["evaluation"]["violations"]),
        vec![("edit.py".to_owned(), "helper-call".to_owned(), 0)]
    );
    assert!(
        first_chunk["restoredFiles"].as_u64().unwrap() > 0,
        "the plan restores the warm directory after the interactive apply"
    );
    scan.close();
    session.close();
}

/// A committed repository holding exactly `files`.
fn committed_repository(tag: &str, files: &[(&str, &[u8])]) -> TempDir {
    let directory = TempDir::new(tag);
    let root = directory.0.join("repo");
    for (path, bytes) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    git(&root, &["init", "-q"]);
    commit(&root, "fixture");
    directory
}

/// One repository index call, as the host makes it; sources above `max_file_bytes` are refused.
fn index_chunk(workspace: &CodeRuleDslWorkspace, max_file_bytes: u32) -> Value {
    let mut task = WorkspaceIndexTask::for_test(
        Arc::clone(&workspace.state),
        Arc::clone(&workspace.closed),
        max_file_bytes,
    );
    serde_json::from_str(&task.compute().unwrap()).unwrap()
}

/// Every index call until the index reports it is done.
fn index_until_done(workspace: &CodeRuleDslWorkspace, max_file_bytes: u32) -> Vec<Value> {
    let mut responses = Vec::new();
    loop {
        let response = index_chunk(workspace, max_file_bytes);
        let done = response["done"] != json!(false);
        responses.push(response);
        if done {
            return responses;
        }
    }
}

fn storage_workspace(root: &std::path::Path, store: PathBuf) -> CodeRuleDslWorkspace {
    let workspace = CodeRuleDslWorkspace::new();
    workspace.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), store).unwrap());
    workspace
}

#[test]
fn repository_index_skips_only_the_nested_checkouts_the_host_names() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = committed_repository("nested-checkouts", &[("main.py", b"print(1)\n")]);
    let root = directory.0.join("repo");
    for path in [
        "vendor/cloned/lib.py",
        "vendor/linked/lib.py",
        "node_modules/x/lib.py",
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"print(2)\n").unwrap();
    }
    git(&root.join("vendor/cloned"), &["init", "-q"]);
    std::fs::write(
        root.join("vendor/linked/.git"),
        b"gitdir: ../../elsewhere\n",
    )
    .unwrap();
    std::fs::create_dir(root.join("node_modules/x/.git")).unwrap();
    let mut workspace = storage_workspace(&root, directory.store());
    workspace.add_code_rule(&print_code_rule(), None).unwrap();

    let mut responses = Vec::new();
    loop {
        let mut task = WorkspaceIndexTask::for_test(
            Arc::clone(&workspace.state),
            Arc::clone(&workspace.closed),
            u32::MAX,
        );
        task.options.ignored_directories = vec!["node_modules".to_owned()];
        // `vendor/linked` holds a `.git` entry too, but only the host decides what is a checkout of its own.
        task.options.nested_checkouts = vec!["vendor/cloned".to_owned()];
        let response: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
        let done = response["done"] != json!(false);
        responses.push(response);
        if done {
            break;
        }
    }

    for response in &responses {
        assert_eq!(response["nestedCheckouts"], json!(["vendor/cloned"]));
        assert_eq!(response["discoveredFiles"], json!(2));
    }
    let paths = workspace
        .state
        .lock()
        .unwrap()
        .files
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        paths.iter().all(|path| !path.contains("vendor/cloned")),
        "nested checkout files were indexed: {paths:?}"
    );
    assert!(
        paths.iter().any(|path| path == "vendor/linked/lib.py"),
        "a folder holding a stray `.git` was not indexed: {paths:?}"
    );
    workspace.close();
}

#[test]
fn repository_index_reads_at_most_one_batch_per_call_and_keeps_no_source_resident() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    const FILES: usize = 80;
    let source = "value = 1\n".repeat(4 * 1024);
    let paths = (0..FILES)
        .map(|index| format!("pkg/f{index:03}.py"))
        .collect::<Vec<_>>();
    let files = paths
        .iter()
        .map(|path| (path.as_str(), source.as_bytes()))
        .collect::<Vec<_>>();
    let directory = committed_repository("bounded-index", &files);
    let mut workspace = storage_workspace(&directory.0.join("repo"), directory.store());
    workspace.add_code_rule(&print_code_rule(), None).unwrap();
    let mut read = 0;
    for call in 1..=FILES {
        let response = index_chunk(&workspace, u32::MAX);
        let total = response["readFiles"].as_u64().unwrap() as usize;
        let batch = total - read;
        read = total;
        assert!(
            batch <= 32,
            "call {call} read {batch} files, more than one batch"
        );
        assert!(
            batch * source.len() <= 1024 * 1024 + source.len(),
            "call {call} read {} bytes, more than one 1 MiB batch and its last file",
            batch * source.len()
        );
        let resident = workspace
            .state
            .lock()
            .unwrap()
            .files
            .values()
            .map(|file| file.input.source.len())
            .sum::<usize>();
        assert_eq!(resident, 0, "call {call} left published sources resident");
        if response["done"] == json!(true) {
            break;
        }
    }
    assert_eq!(read, FILES, "the index reads every scoped file");
    workspace.close();
}

#[test]
fn repository_index_reports_paths_relative_to_the_storage_root() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = committed_repository(
        "rooted-index",
        &[
            ("outside/a.py", b"print(1)\n"),
            ("sub/pkg/a.py", b"print(2)\n"),
        ],
    );
    let mut workspace = storage_workspace(&directory.0.join("repo/sub"), directory.store());
    workspace.add_code_rule(&print_code_rule(), None).unwrap();
    index_until_done(&workspace, u32::MAX);
    assert_eq!(
        located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES)),
        vec![("pkg/a.py".to_owned(), "print".to_owned(), 0)],
        "the index reports the host's workspace-relative paths and nothing outside the root"
    );
    let revision = workspace.state.lock().unwrap().revision + 1;
    apply_buffer(
        &workspace,
        revision,
        &json!([{ "path": "pkg/a.py", "language": "python", "source": "value = 2\n" }]),
    )
    .unwrap();
    assert_eq!(
        all_pages(&workspace, MAXIMUM_PAGE_BYTES),
        json!([]),
        "a host edit replaces the indexed file it names"
    );
    workspace.close();
}

#[test]
fn repository_index_reports_unreadable_sources_as_failures() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let large = "print(2)\n".repeat(100);
    let directory = committed_repository(
        "index-failures",
        &[
            ("binary.py", b"print(1)\0\n"),
            ("large.py", large.as_bytes()),
            ("ok.py", b"print(3)\n"),
        ],
    );
    let mut workspace = storage_workspace(&directory.0.join("repo"), directory.store());
    workspace.add_code_rule(&print_code_rule(), None).unwrap();
    let mut failures = index_until_done(&workspace, 64)
        .iter()
        .flat_map(|response| response["failures"].as_array().cloned().unwrap_or_default())
        .map(|failure| {
            (
                failure["path"].as_str().unwrap().to_owned(),
                failure["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    failures.sort();
    assert_eq!(
        failures,
        vec![
            ("binary.py".to_owned(), "source-incomplete".to_owned()),
            ("large.py".to_owned(), "budget-exceeded".to_owned()),
        ],
        "binary and oversized sources are reported, not skipped or evaluated"
    );
    assert_eq!(
        located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES)),
        vec![("ok.py".to_owned(), "print".to_owned(), 0)]
    );
    workspace.close();
}

/// Whether a snapshot in the store at `store` holds `path`.
fn snapshot_holds(store: &std::path::Path, path: &str) -> bool {
    rusqlite::Connection::open(store)
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshot_files f JOIN paths p ON p.path_id=f.path_id \
             JOIN strings s ON s.string_id=p.string_id WHERE s.value=?1)",
            [path],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn host_files_with_clean_blob_bytes_are_aliased_and_promoted() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = committed_repository(
        "host-aliases",
        &[("pkg/a.py", b"print(1)\n"), ("pkg/b.py", b"print(2)\n")],
    );
    let root = directory.0.join("repo");
    let repository = git_workspace::discover(&root).unwrap();
    let clean = repository.clean_source_alias("pkg/a.py").unwrap();
    let edited = repository.clean_source_alias("pkg/b.py").unwrap();
    std::fs::write(root.join("pkg/c.py"), "print(4)\n").unwrap();
    let mut workspace = storage_workspace(&root, directory.store());
    workspace.add_code_rule(&print_code_rule(), None).unwrap();
    crate::git_workspace::reset_source_alias_hashes();
    apply_buffer(
        &workspace,
        1,
        &json!([
            { "path": "pkg/a.py", "language": "python", "source": "print(1)\n" },
            { "path": "pkg/b.py", "language": "python", "source": "print(3)\n" },
            { "path": "pkg/c.py", "language": "python", "source": "print(4)\n" },
        ]),
    )
    .unwrap();
    assert_eq!(
        crate::git_workspace::source_alias_hashes(),
        2,
        "publication hashes exactly the host files whose paths have a clean committed blob"
    );
    let storage = workspace.state.lock().unwrap().storage.clone().unwrap();
    assert!(
        storage.store.source_by_alias(clean).unwrap().is_some(),
        "a host file with the committed bytes is stored under its Git alias"
    );
    assert!(
        storage.store.source_by_alias(edited).unwrap().is_none(),
        "an unsaved edit never claims the committed blob"
    );
    assert!(snapshot_holds(&directory.store(), "pkg/a.py"));
    assert!(!snapshot_holds(&directory.store(), "pkg/b.py"));
    assert!(!snapshot_holds(&directory.store(), "pkg/c.py"));
    workspace.close();
}

#[test]
fn restored_results_keep_partial_source_diagnostics() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("restored-partial-source");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "restored-partial-source",
        json!([{
            "id": "debugger",
            "violation": { "anchor": "witness.span", "code": "DEBUG", "message": "debugger" },
            "forbid": { "ast": { "language": "typescript",
                "rule": { "kind": "debugger_statement" } } }
        }]),
    );
    let files = json!([{ "path": "broken.ts", "language": "typescript",
        "source": "debugger;\nfunction broken( {\n" }]);
    let cold = stored_workspace(&store, "/restored-partial-source", &code_rule);
    let expected = apply_json(&cold, 1, files.clone());
    assert_eq!(
        expected["evaluation"]["failures"][0]["code"],
        "source-partial"
    );
    cold.close();

    let warm = stored_workspace(&store, "/restored-partial-source", &code_rule);
    let restored = apply_json(&warm, 1, files);
    assert_eq!(
        restored["evaluation"]["failures"],
        expected["evaluation"]["failures"]
    );
    let page: Value =
        serde_json::from_str(&result_page(&warm, 0, MAXIMUM_PAGE_BYTES).unwrap()).unwrap();
    assert_eq!(page["failures"], expected["evaluation"]["failures"]);
    warm.close();
}

#[test]
fn a_storage_prefetch_failure_keeps_dirty_files_for_the_next_revision() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("storage-prefetch-retry");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let workspace = stored_workspace(&store, "/storage-prefetch-retry", &print_code_rule());
    TEST_STORAGE_PREFETCH_FAILURE.set(true);
    let failed = apply_buffer(
        &workspace,
        1,
        &json!([{ "path": "a.py", "language": "python", "source": "print(1)\n" }]),
    )
    .unwrap_err();
    assert!(
        failed
            .reason
            .contains("injected local-result storage failure")
    );
    {
        let state = workspace.state.lock().unwrap();
        assert!(state.files.contains_key("a.py"));
        assert!(state.dirty_files.contains("a.py"));
    }
    let retry = apply_json(&workspace, 2, json!([]));
    assert_eq!(
        located(&retry["evaluation"]["violations"]),
        vec![("a.py".to_owned(), "print".to_owned(), 0)]
    );
    workspace.close();
}

#[test]
fn a_recovered_file_is_replaced_with_later_revision_changes() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("storage-prefetch-replacement");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let workspace = stored_workspace(&store, "/storage-prefetch-replacement", &print_code_rule());
    TEST_STORAGE_PREFETCH_FAILURE.set(true);
    let failed = apply_buffer(
        &workspace,
        1,
        &json!([{ "path": "a.py", "language": "python", "source": "print(1)\n" }]),
    )
    .unwrap_err();
    assert!(
        failed
            .reason
            .contains("injected local-result storage failure")
    );
    let retry = apply_json(
        &workspace,
        2,
        json!([{ "path": "b.py", "language": "python", "source": "print(2)\n" }]),
    );
    assert_eq!(
        located(&retry["evaluation"]["violations"]),
        vec![
            ("a.py".to_owned(), "print".to_owned(), 0),
            ("b.py".to_owned(), "print".to_owned(), 0),
        ]
    );
    workspace.close();
}
#[test]
fn package_marker_overlays_refresh_unchanged_siblings_for_json_and_buffer_requests() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("package-marker-overlays");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = package_marker_code_rule();
    let workspace = stored_workspace(&store, "/package-marker-overlays", &code_rule);
    let siblings = json!([
        { "path": "pkg/helper.py", "language": "python", "source": "def helper():\n    pass\n" },
        { "path": "pkg/mod.py", "language": "python", "source": PACKAGE_SOURCE },
    ]);
    assert_eq!(
        apply_json(&workspace, 1, siblings)["evaluation"]["violations"],
        json!([])
    );
    let created = apply_buffer(
        &workspace,
        2,
        &json!([{ "path": "pkg/__init__.py", "language": "python", "source": "" }]),
    )
    .unwrap();
    assert_eq!(
        located(&created["evaluation"]["violations"]),
        vec![("pkg/mod.py".to_owned(), "pkg-helper".to_owned(), 21)],
        "a binary unsaved marker overlay refreshes the unchanged JSON sibling"
    );
    let mut remove = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 3, "files": [], "removed": ["pkg/__init__.py"] })
            .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let deleted: Value = serde_json::from_str(&remove.compute().unwrap()).unwrap();
    assert_eq!(
        deleted["evaluation"]["violations"],
        json!([]),
        "a JSON unsaved marker deletion refreshes the unchanged binary sibling"
    );
    workspace.close();
}

#[test]
fn removing_a_package_marker_never_reintroduces_the_removed_file() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("removed-package-marker");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "removed-package-marker",
        json!([{
            "id": "marker",
            "violation": { "anchor": "witness.span", "code": "MARKER", "message": "marker" },
            "forbid": { "ast": { "language": "python",
                "rule": { "pattern": "marker()" } } }
        }]),
    );
    let workspace = stored_workspace(&store, "/removed-package-marker", &code_rule);
    let files = json!([
        { "path": "pkg/__init__.py", "language": "python", "source": "marker()\n" },
        { "path": "pkg/mod.py", "language": "python", "source": "value = 1\n" }
    ]);
    assert_eq!(
        located(&apply_json(&workspace, 1, files)["evaluation"]["violations"]).len(),
        1
    );
    let mut remove = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 2, "files": [], "removed": ["pkg/__init__.py"] })
            .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let deleted: Value = serde_json::from_str(&remove.compute().unwrap()).unwrap();
    let mut fresh = CodeRuleDslWorkspace::new();
    fresh.add_code_rule(&code_rule, None).unwrap();
    let expected = apply_json(
        &fresh,
        1,
        json!([{ "path": "pkg/mod.py", "language": "python", "source": "value = 1\n" }]),
    );
    assert_eq!(deleted["evaluation"], expected["evaluation"]);
    assert_eq!(all_pages(&workspace, MAXIMUM_PAGE_BYTES), json!([]));
    fresh.close();
    workspace.close();
}

#[test]
fn an_edit_without_a_module_keeps_the_module_the_workspace_markers_give() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&package_marker_code_rule(), None)
        .unwrap();
    let files = json!([
        { "path": "pkg/__init__.py", "language": "python", "source": "" },
        { "path": "pkg/helper.py", "language": "python", "source": "def helper():\n    pass\n" },
        { "path": "pkg/mod.py", "language": "python", "source": PACKAGE_SOURCE },
    ]);
    let expected = vec![("pkg/mod.py".to_owned(), "pkg-helper".to_owned(), 21)];
    assert_eq!(
        located(&apply_json(&workspace, 1, files)["evaluation"]["violations"]),
        expected
    );
    let edited = apply_json(
        &workspace,
        2,
        json!([{ "path": "pkg/mod.py", "language": "python",
            "source": format!("{PACKAGE_SOURCE}# edited\n") }]),
    );
    assert_eq!(
        located(&edited["evaluation"]["violations"]),
        expected,
        "the edited file is still `pkg.mod`, not a loose `mod`"
    );
    workspace.close();
}

#[test]
fn buffer_inline_and_repository_ingestion_agree_on_python_module_context() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let repository = package_repository("ingestion");
    let root = repository.0.join("repo");
    let code_rule = module_code_rule();
    let files = json!([
        { "path": "alpha/x.py", "language": "python", "source": PACKAGE_SOURCE, "module": "alpha.x" },
        { "path": "beta/x.py", "language": "python", "source": PACKAGE_SOURCE, "module": "beta.x" },
        { "path": "loose/x.py", "language": "python", "source": PACKAGE_SOURCE },
    ]);

    let mut inline = CodeRuleDslWorkspace::new();
    inline.add_code_rule(&code_rule, None).unwrap();
    let inline_result = apply_json(&inline, 1, files.clone());
    assert_eq!(
        located(&inline_result["evaluation"]["violations"]),
        expected_package_violations()
    );
    inline.close();

    let mut buffered = CodeRuleDslWorkspace::new();
    buffered.add_code_rule(&code_rule, None).unwrap();
    let buffered_result = apply_buffer(&buffered, 1, &files).expect("buffer request with modules");
    assert_eq!(
        buffered_result["evaluation"]["violations"],
        inline_result["evaluation"]["violations"]
    );
    buffered.close();

    let mut indexed = CodeRuleDslWorkspace::new();
    indexed.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), repository.store()).unwrap());
    indexed.add_code_rule(&code_rule, None).unwrap();
    index_workspace(&indexed);
    assert_eq!(
        all_pages(&indexed, MAXIMUM_PAGE_BYTES),
        inline_result["evaluation"]["violations"]
    );
    indexed.close();
}

#[test]
fn identical_source_in_two_packages_never_reuses_a_wrong_result() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    for first in ["alpha", "beta"] {
        let directory = TempDir::new(&format!("packages-{first}"));
        let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
        let code_rule = module_code_rule();

        let workspace = stored_workspace(&store, "/packages", &code_rule);
        let cold = apply_json(&workspace, 1, package_files(first));
        assert_eq!(
            located(&cold["evaluation"]["violations"]),
            expected_package_violations(),
            "cold evaluation, {first} first"
        );
        workspace.completion_task().compute().unwrap();
        assert!(workspace.evict_inactive().unwrap());
        let evicted = apply_json(&workspace, 2, package_files(first));
        assert_eq!(
            located(&evicted["evaluation"]["violations"]),
            expected_package_violations(),
            "after eviction, {first} first"
        );
        workspace.close();

        let restarted = stored_workspace(&store, "/packages", &code_rule);
        let warm = apply_json(&restarted, 1, package_files(first));
        assert_eq!(
            located(&warm["evaluation"]["violations"]),
            expected_package_violations(),
            "after restart, {first} first"
        );
        assert!(
            restored_results(&restarted) > 0,
            "a warm restart reuses the results stored under the same file context"
        );
        restarted.close();
    }
}

fn restored_results(workspace: &CodeRuleDslWorkspace) -> usize {
    workspace
        .state
        .lock()
        .unwrap()
        .files
        .values()
        .map(|file| file.restored_local_results)
        .sum()
}

#[test]
fn another_engine_build_never_reuses_stored_results() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("engine-build");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = module_code_rule();
    let cold = stored_workspace(&store, "/engine", &code_rule);
    let expected = apply_json(&cold, 1, package_files("alpha"));
    cold.close();

    let same = stored_workspace(&store, "/engine", &code_rule);
    let warm = apply_json(&same, 1, package_files("alpha"));
    assert!(
        restored_results(&same) > 0,
        "the same build reuses its results"
    );
    same.close();

    TEST_ENGINE_BUILD_ID.set(Some("another-engine-build"));
    let other = stored_workspace(&store, "/engine", &code_rule);
    let rebuilt = apply_json(&other, 1, package_files("alpha"));
    TEST_ENGINE_BUILD_ID.set(None);
    assert_eq!(
        restored_results(&other),
        0,
        "another engine build evaluates again"
    );
    other.close();
    for result in [&warm, &rebuilt] {
        assert_eq!(
            result["evaluation"]["violations"],
            expected["evaluation"]["violations"]
        );
    }
}

#[test]
fn select_when_leaves_never_restore_each_others_results() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("leaves");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "leaves",
        json!([{
            "id": "gated", "violation": { "anchor": "witness.span", "code": "GATE",
                "message": "print while os is imported" },
            "forbid": { "select_when": {
                "select": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } },
                "exists": { "ast": { "language": "python", "rule": { "pattern": "import os" } } }
            } }
        }]),
    );
    let files = json!([{ "path": "a.py", "language": "python", "source": "print(1)\n" }]);
    let cold = stored_workspace(&store, "/leaves", &code_rule);
    let expected = apply_json(&cold, 1, files.clone());
    assert_eq!(expected["evaluation"]["violations"], json!([]));
    cold.close();
    let restarted = stored_workspace(&store, "/leaves", &code_rule);
    let warm = apply_json(&restarted, 1, files);
    assert_eq!(
        warm["evaluation"]["violations"], expected["evaluation"]["violations"],
        "the exists leaf must not restore the select leaf's witnesses"
    );
    restarted.close();
}

#[test]
fn transient_timeouts_are_retried_and_never_persisted() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("deadline");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "deadline",
        json!([{
            "id": "print", "violation": { "anchor": "witness.span", "code": "PY",
                "message": "print" },
            "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
        }]),
    );
    let file = json!([{ "path": "slow.py", "language": "python", "source": "print(1)\n" }]);

    let workspace = stored_workspace(&store, "/deadline", &code_rule);
    TEST_FILE_WORK_BUDGET.set(Some(0));
    let timed_out = apply_json(&workspace, 1, file.clone());
    TEST_FILE_WORK_BUDGET.set(None);
    assert_eq!(timed_out["evaluation"]["violations"], json!([]));
    assert_eq!(
        timed_out["evaluation"]["failures"][0]["code"],
        "budget-exceeded"
    );

    let retried = apply_json(&workspace, 2, file.clone());
    assert_eq!(
        located(&retried["evaluation"]["violations"]),
        vec![("slow.py".to_owned(), "print".to_owned(), 0)],
        "an unchanged file whose evaluation timed out is evaluated again"
    );
    assert_eq!(retried["evaluation"]["failures"], json!([]));
    workspace.close();

    let timed_out_again = stored_workspace(&store, "/deadline-restart", &code_rule);
    TEST_FILE_WORK_BUDGET.set(Some(0));
    apply_json(&timed_out_again, 1, file.clone());
    TEST_FILE_WORK_BUDGET.set(None);
    timed_out_again.close();
    let restarted = stored_workspace(&store, "/deadline-restart", &code_rule);
    let warm = apply_json(&restarted, 1, file);
    assert_eq!(
        located(&warm["evaluation"]["violations"]),
        vec![("slow.py".to_owned(), "print".to_owned(), 0)],
        "a timed-out result is never restored as the content's verdict"
    );
    restarted.close();
}

#[test]
fn an_evaluation_that_finishes_past_an_unobserved_deadline_is_kept() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("unobserved-deadline");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "unobserved-deadline",
        json!([{
            "id": "needle", "violation": { "anchor": "witness.span", "code": "NEEDLE",
                "message": "needle" },
            "forbid": { "regex": { "source": "file.content", "pattern": "needle" } }
        }]),
    );
    let file = json!([{ "path": "clean.py", "language": "python", "source": "print(1)\n" }]);

    // The deadline passes before the evaluation ends, yet no step of it polls the deadline: a
    // regex without matches never does. The finished evaluation is the content's verdict.
    let workspace = stored_workspace(&store, "/unobserved-deadline", &code_rule);
    TEST_FILE_WORK_BUDGET.set(Some(0));
    let finished = apply_json(&workspace, 1, file.clone());
    TEST_FILE_WORK_BUDGET.set(None);
    assert_eq!(finished["evaluation"]["failures"], json!([]));
    assert_eq!(finished["evaluation"]["violations"], json!([]));
    assert!(
        !workspace.state.lock().unwrap().files["clean.py"].transient_failure,
        "an evaluation no deadline check interrupted is not transient"
    );
    workspace.close();

    let restarted = stored_workspace(&store, "/unobserved-deadline", &code_rule);
    apply_json(&restarted, 1, file);
    assert_eq!(
        restored_results(&restarted),
        1,
        "the finished evaluation was persisted and is restored"
    );
    restarted.close();
}

#[test]
fn an_ast_evaluation_that_finishes_past_an_unobserved_deadline_is_kept() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("unobserved-ast-deadline");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "unobserved-ast-deadline",
        json!([{
            "id": "print", "violation": { "anchor": "witness.span", "code": "PY",
                "message": "print" },
            "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
        }]),
    );
    let file = json!([{ "path": "late.py", "language": "python", "source": "print(1)\n" }]);

    // Every clause ran to its end; the deadline passes after the last one, and no step polls
    // it again. The finished evaluation is the content's verdict.
    let workspace = stored_workspace(&store, "/unobserved-ast-deadline", &code_rule);
    TEST_DEADLINE_PASSES_AT.set(Some("checks-done"));
    let finished = apply_json(&workspace, 1, file.clone());
    TEST_DEADLINE_PASSES_AT.set(None);
    assert_eq!(finished["evaluation"]["failures"], json!([]));
    assert_eq!(
        located(&finished["evaluation"]["violations"]),
        vec![("late.py".to_owned(), "print".to_owned(), 0)]
    );
    assert!(
        !workspace.state.lock().unwrap().files["late.py"].transient_failure,
        "an AST evaluation no deadline check interrupted is not transient"
    );
    workspace.close();

    let restarted = stored_workspace(&store, "/unobserved-ast-deadline", &code_rule);
    apply_json(&restarted, 1, file);
    assert_eq!(
        restored_results(&restarted),
        1,
        "the finished AST evaluation was persisted and is restored"
    );
    restarted.close();
}

/// A registered code rule's AST clauses report their violations and its `llm` clauses still report what
/// they lack.
#[test]
fn a_registered_code_rule_reports_ast_violations_and_missing_llm_capability() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let code_rule = compile(
        "ast-and-llm",
        json!([
            {
                "id": "print", "violation": { "anchor": "witness.span", "code": "PY",
                    "message": "print" },
                "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
            },
            {
                "id": "intent", "violation": { "anchor": "witness.span", "code": "INTENT",
                    "message": "intent" },
                "llm": { "input": { "source": "changed-symbol" },
                    "rubric": "Fail on direct persistence access", "verdicts": ["pass", "fail"] }
            }
        ]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    let applied = apply_json(
        &workspace,
        1,
        json!([{ "path": "a.py", "language": "python", "source": "print(1)\n" }]),
    );
    workspace.close();
    assert_eq!(
        located(&applied["evaluation"]["violations"]),
        vec![("a.py".to_owned(), "print".to_owned(), 0)]
    );
    let failures = applied["evaluation"]["failures"].as_array().unwrap();
    assert_eq!(
        failures
            .iter()
            .map(|failure| (failure["clauseId"].as_str(), failure["code"].as_str()))
            .collect::<Vec<_>>(),
        [(Some("intent"), Some("missing-capability"))]
    );
}

#[test]

fn every_engine_limit_failure_round_trips_through_the_local_result_codec() {
    for limit in [
        UtilLimit::Depth,
        UtilLimit::Work,
        UtilLimit::Scope,
        UtilLimit::Value,
    ] {
        let bytes =
            encode_local_result(&[], Some(("budget-exceeded", limit.message())), None).unwrap();
        let decoded = decode_local_result(&bytes, "a.py").expect("decodable limit failure");
        assert_eq!(decoded.1, Some(("budget-exceeded", limit.message())));
    }
}

#[test]
fn paged_results_equal_complete_results_for_each_violations_and_capture_anchors() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("paging");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let rule = json!({ "kind": "comparison_operator",
        "has": { "any": [{ "kind": "true" }, { "kind": "false" }], "pattern": "$OP" } });
    let violation = |each: bool| {
        let mut violation =
            json!({ "anchor": "witness.span", "code": "CMP", "message": "compare" });
        if each {
            violation["each"] = json!("OP");
            violation["anchor"] = json!("capture.OP");
        }
        violation
    };
    let code_rule = compile(
        "paging",
        json!([
            { "id": "once", "violation": violation(false),
              "forbid": { "ast": { "language": "python", "rule": rule } } },
            { "id": "each", "violation": violation(true),
              "forbid": { "ast": { "language": "python", "rule": rule } } }
        ]),
    );
    let workspace = stored_workspace(&store, "/paging", &code_rule);
    let complete = apply_json(
        &workspace,
        1,
        json!([{ "path": "a.py", "language": "python",
            "source": "if res == True != False:\n    pass\n" }]),
    );
    assert_eq!(
        complete["evaluation"]["violations"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for maximum_bytes in [1, MAXIMUM_PAGE_BYTES] {
        assert_eq!(
            all_pages(&workspace, maximum_bytes),
            complete["evaluation"]["violations"],
            "pages of at most {maximum_bytes} bytes"
        );
    }
    workspace.close();
}

/// Every stored local result as `(path, clause, codec, raw length, frame)`, in that order.
fn stored_local_results(store: &std::path::Path) -> Vec<(String, String, i64, i64, Vec<u8>)> {
    let connection = rusqlite::Connection::open(store).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT ps.value,cs.value,r.codec_id,r.raw_length,r.frame FROM context_check_results r \
             JOIN workspace_overlay o ON o.source_id=r.source_id \
             JOIN paths p ON p.path_id=o.path_id JOIN strings ps ON ps.string_id=p.string_id \
             JOIN candidate_checks c ON c.check_id=r.check_id \
             JOIN strings cs ON cs.string_id=c.check_string_id ORDER BY 1,2",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn stored_local_results_and_their_read_back_are_unchanged_by_eviction_round_trips() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("round-trips");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "round-trips",
        json!([
            { "id": "print", "violation": { "anchor": "witness.span", "code": "PRINT",
                "message": "print of $A", "fix": "log($A)" },
              "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } } },
            { "id": "call", "violation": { "anchor": "capture.ARGS", "code": "CALL",
                "message": "call" },
              "forbid": { "ast": { "language": "python",
                "rule": { "pattern": "foo($$$ARGS)" } } } },
            { "id": "never", "violation": { "anchor": "witness.span", "code": "NEVER",
                "message": "never" },
              "forbid": { "ast": { "language": "python", "rule": { "pattern": "exec($A)" } } } }
        ]),
    );
    let files = json!([
        { "path": "a.py", "language": "python",
          "source": "print(1)\nfoo(1, 2)\nprint('ü')\n" },
        { "path": "b.py", "language": "python", "source": "x = 1\n" },
        { "path": "c.py", "language": "python", "source": "foo()\nprint(x\n" },
    ]);
    let workspace = stored_workspace(&store, "/round-trips", &code_rule);
    let cold = apply_json(&workspace, 1, files.clone())["evaluation"]["violations"].clone();
    assert!(cold.as_array().unwrap().len() >= 3, "{cold}");
    let stored = stored_local_results(&directory.store());
    for clause in ["call", "print"] {
        assert!(
            stored.iter().any(|row| row.0 == "a.py" && row.1 == clause),
            "{clause}: {stored:?}"
        );
    }
    for (path, clause, _, raw_length, frame) in &stored {
        let bytes = zstd::stream::decode_all(std::io::Cursor::new(frame)).unwrap();
        assert_eq!(bytes.len() as i64, *raw_length, "{path} {clause}");
        // A result is the canonical (key-ordered) serialization of its JSON value.
        let value = serde_json::from_slice::<Value>(&bytes).unwrap();
        assert_eq!(
            serde_json::to_vec(&value).unwrap(),
            bytes,
            "{path} {clause}"
        );
        assert_eq!(value["version"], 3, "{path} {clause}");
    }

    for revision in 2..=3 {
        workspace.completion_task().compute().unwrap();
        assert!(workspace.evict_inactive().unwrap());
        let restored = apply_json(&workspace, revision, files.clone());
        assert_eq!(
            restored["evaluation"]["violations"], cold,
            "revision {revision}"
        );
        assert_eq!(all_pages(&workspace, 1), cold, "revision {revision} pages");
        assert!(
            restored_results(&workspace) > 0,
            "revision {revision} reads the store"
        );
        assert_eq!(
            stored_local_results(&directory.store()),
            stored,
            "revision {revision}"
        );
    }
    workspace.close();

    let restarted = stored_workspace(&store, "/round-trips", &code_rule);
    let warm = apply_json(&restarted, 1, files);
    assert_eq!(warm["evaluation"]["violations"], cold, "after restart");
    assert!(
        restored_results(&restarted) > 0,
        "a restart reads the store"
    );
    assert_eq!(
        stored_local_results(&directory.store()),
        stored,
        "after restart"
    );
    restarted.close();
}

fn print_code_rule() -> CompiledCodeRuleDsl {
    compile(
        "documents",
        json!([
            { "id": "print", "violation": { "anchor": "witness.span", "code": "PY",
                "message": "print" },
              "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } } },
            { "id": "alpha-helper", "violation": { "anchor": "witness.span", "code": "CTX",
                "message": "alpha helper" },
              "forbid": { "ast": { "language": "python", "rule": {
                  "kind": "identifier", "regex": "^helper$",
                  "resolves": { "import": ["alpha.helper"] } } } } }
        ]),
    )
}

fn resolve(workspace: &CodeRuleDslWorkspace, reference: &str) -> Result<Value> {
    Ok(serde_json::from_str(&workspace.resolve_node_reference(reference.to_owned())?).unwrap())
}

#[test]
fn selector_index_misses_keep_input_budget_failures() {
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDir::new("selector-limits");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let files = json!([{ "path": "plain.py", "language": "python", "source": "value = 1\n" }]);
    let first = stored_workspace(&store, "/selector-limits-first", &print_code_rule());
    apply_json(&first, 1, files.clone());
    first.close();
    super::storage::index_stored_tapes(
        &store,
        &std::sync::atomic::AtomicBool::new(false),
        &mut None,
    );
    // `plain.py` has no lambda, but it is over this code rule's source-byte cap.
    let capped = CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "capped-lambdas", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{
                "id": "lambda",
                "violation": { "anchor": "witness.span", "code": "L", "message": "lambda" },
                "forbid": { "ast": { "language": "python", "rule": { "kind": "lambda" } } }
            }],
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.replace("\"maxSourceBytes\": 1000000", "\"maxSourceBytes\": 4"),
    )
    .unwrap();
    let narrowed = stored_workspace(&store, "/selector-limits-second", &capped);
    let applied = apply_json(&narrowed, 1, files.clone());
    assert!(narrowed.state.lock().unwrap().files["plain.py"].selector_skipped);
    narrowed.close();
    let mut unstored = CodeRuleDslWorkspace::new();
    unstored.add_code_rule(&capped, None).unwrap();
    let fresh = apply_json(&unstored, 1, files);
    unstored.close();
    assert_eq!(
        applied["evaluation"]["failures"][0]["message"], "file input budget exceeded",
        "{applied}"
    );
    assert_eq!(
        applied["evaluation"]["failures"],
        fresh["evaluation"]["failures"]
    );
}

#[test]
fn node_references_resolve_through_the_store_and_refuse_stale_versions() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("references");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let workspace = stored_workspace(&store, "/references", &print_code_rule());
    let applied = apply_json(
        &workspace,
        1,
        json!([
            { "path": "a.py", "language": "python", "source": "x = 1\nprint(x)\n" },
            { "path": "b.py", "language": "python", "source": "print(2)\n" }
        ]),
    );
    let span = &applied["evaluation"]["violations"][0]["span"];
    assert_eq!(applied["evaluation"]["violations"][0]["path"], "a.py");
    let (start, end) = (
        span["startByte"].as_u64().unwrap() as u32,
        span["endByte"].as_u64().unwrap() as u32,
    );
    let reference = workspace
        .node_reference("a.py".to_owned(), start, end)
        .unwrap()
        .expect("the finding's node");
    let resident = resolve(&workspace, &reference).unwrap();
    assert_eq!(resident["kind"], "call");
    assert_eq!(resident["text"], "print(x)");

    assert!(workspace.evict_inactive().unwrap());
    assert_eq!(
        resolve(&workspace, &reference).unwrap(),
        resident,
        "an evicted file resolves from the store to the same node"
    );
    workspace.set_document_residency_limit(1).unwrap();
    for _ in 0..2 {
        resolve(&workspace, "b.py-is-not-a-reference").unwrap_err();
        assert_eq!(resolve(&workspace, &reference).unwrap(), resident);
    }
    {
        let state = workspace.state.lock().unwrap();
        assert!(
            state.documents.loads.stored > 0,
            "evicted loads come from the store"
        );
        assert!(
            state.documents.loads.evicted > 0,
            "the residency bound evicted documents"
        );
        assert!(state.documents.resident_bytes() <= 1);
    }

    let other = stored_workspace(&store, "/references-other", &print_code_rule());
    apply_json(
        &other,
        1,
        json!([{ "path": "a.py", "language": "python", "source": "x = 1\nprint(x)\n" }]),
    );
    let foreign = resolve(&other, &reference).unwrap_err();
    assert!(
        foreign.reason.contains("another workspace"),
        "{}",
        foreign.reason
    );
    other.close();

    apply_json(
        &workspace,
        2,
        json!([{ "path": "a.py", "language": "python", "source": "x = 2\nprint(x)\n" }]),
    );
    let edited = resolve(&workspace, &reference).unwrap_err();
    assert!(
        edited.reason.contains("stale node reference"),
        "{}",
        edited.reason
    );
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 3, "files": [], "removed": ["a.py"] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    task.compute().unwrap();
    let removed = resolve(&workspace, &reference).unwrap_err();
    assert!(
        removed.reason.contains("no longer in the workspace"),
        "{}",
        removed.reason
    );
    workspace.close();
}

#[test]
fn javascript_and_php_documents_have_distinct_persisted_grammars() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("javascript-php-grammars");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "javascript-php-grammars",
        json!([
            { "id": "js", "violation": { "anchor": "witness.span", "code": "JS",
                "message": "js" },
              "forbid": { "ast": { "language": "typescript",
                  "rule": { "pattern": "marker()" } } } },
            { "id": "php", "violation": { "anchor": "witness.span", "code": "PHP",
                "message": "php" },
              "forbid": { "ast": { "language": "php",
                  "rule": { "pattern": "marker()" } } } }
        ]),
    );
    let workspace = stored_workspace(&store, "/javascript-php-grammars", &code_rule);
    let applied = apply_json(
        &workspace,
        1,
        json!([
            { "path": "a.js", "language": "javascript", "source": "marker();\n" },
            { "path": "b.php", "language": "php", "source": "marker();\n" }
        ]),
    );
    let js = &applied["evaluation"]["violations"][0]["span"];
    let reference = workspace
        .node_reference(
            "a.js".to_owned(),
            js["startByte"].as_u64().unwrap() as u32,
            js["endByte"].as_u64().unwrap() as u32,
        )
        .unwrap()
        .expect("JavaScript finding has a reference");
    assert!(workspace.evict_inactive().unwrap());
    assert_eq!(resolve(&workspace, &reference).unwrap()["text"], "marker()");
    workspace.close();
}

#[test]
fn an_empty_resident_document_resolves_without_a_store() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&print_code_rule(), None).unwrap();
    apply_json(
        &workspace,
        1,
        json!([{ "path": "empty.py", "language": "python", "source": "" }]),
    );
    let reference = workspace
        .node_reference("empty.py".to_owned(), 0, 0)
        .unwrap()
        .expect("empty source has a root node");
    assert_eq!(resolve(&workspace, &reference).unwrap()["text"], "");
    workspace.close();
}

#[test]
fn evicted_resident_and_unstored_workspaces_publish_the_same_results() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("eviction");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = print_code_rule();
    let file = |path: &str, source: &str, module: Option<&str>| {
        let mut file = json!({ "path": path, "language": "python", "source": source });
        if let Some(module) = module {
            file["module"] = json!(module);
        }
        file
    };
    let revisions = [
        (
            json!([
                file("alpha/x.py", PACKAGE_SOURCE, Some("alpha.x")),
                file("beta/x.py", PACKAGE_SOURCE, Some("beta.x")),
                file("c.py", "print(1)\nprint(2)\n", None),
            ]),
            json!([]),
        ),
        (json!([file("c.py", "print(3)\n", None)]), json!([])),
        (
            json!([file("beta/x.py", PACKAGE_SOURCE, Some("beta.x"))]),
            json!([]),
        ),
        (json!([]), json!(["alpha/x.py"])),
        (
            json!([file("alpha/x.py", PACKAGE_SOURCE, Some("alpha.x"))]),
            json!([]),
        ),
        (
            json!([file("c.py", "print(1)\nprint(2)\n", None)]),
            json!([]),
        ),
    ];
    let resident = stored_workspace(&store, "/eviction-resident", &code_rule);
    let evicted = stored_workspace(&store, "/eviction-evicted", &code_rule);
    let mut unstored = CodeRuleDslWorkspace::new();
    unstored.add_code_rule(&code_rule, None).unwrap();
    for (index, (files, removed)) in revisions.iter().enumerate() {
        let revision = index as u32 + 1;
        let apply = |workspace: &CodeRuleDslWorkspace| -> Value {
            let mut task = WorkspaceApplyTask {
                state: Arc::clone(&workspace.state),
                closed: Arc::clone(&workspace.closed),
                request_json: json!({ "revision": revision, "files": files, "removed": removed })
                    .to_string(),
                request_buffer: None,
                request_direct: None,
            };
            serde_json::from_str::<Value>(&task.compute().unwrap()).unwrap()["evaluation"].clone()
        };
        let changed = files
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["path"].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>();
        let resident_delta = apply(&resident);
        let evicted_delta = apply(&evicted);
        assert!(evicted.evict_inactive().unwrap());
        let complete = apply(&unstored);
        let unstored_delta = complete["violations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|violation| changed.contains(violation["path"].as_str().unwrap()))
            .cloned()
            .collect::<Vec<_>>();
        // Work metrics differ by design (a restored or reloaded file does less); results must not.
        for key in ["status", "violations", "failures"] {
            assert_eq!(
                evicted_delta[key], resident_delta[key],
                "revision {revision}: {key}"
            );
        }
        assert_eq!(
            resident_delta["violations"],
            Value::Array(unstored_delta),
            "revision {revision}"
        );
    }
    resident.close();
    evicted.close();
    unstored.close();
}

#[test]
fn an_evicted_workspace_pages_its_complete_results() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("complete-pages");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let workspace = stored_workspace(&store, "/complete-pages", &print_code_rule());
    apply_json(
        &workspace,
        1,
        json!([
            { "path": "alpha/x.py", "language": "python", "source": PACKAGE_SOURCE,
              "module": "alpha.x" },
            { "path": "beta/x.py", "language": "python", "source": PACKAGE_SOURCE,
              "module": "beta.x" },
            { "path": "c.py", "language": "python", "source": "print(1)\nprint(2)\n" },
            { "path": "broken.py", "language": "python", "source": "print(3)\ndef (:\n" }
        ]),
    );
    workspace.completion_task().compute().unwrap();
    let complete = |workspace: &CodeRuleDslWorkspace| {
        let first: Value =
            serde_json::from_str(&result_page(workspace, 0, MAXIMUM_PAGE_BYTES).unwrap()).unwrap();
        (all_pages(workspace, 1), first["failures"].clone())
    };
    let (resident_violations, resident_failures) = complete(&workspace);
    assert_eq!(resident_violations.as_array().unwrap().len(), 4);
    assert!(!resident_failures.as_array().unwrap().is_empty());

    assert!(workspace.evict_inactive().unwrap());
    assert_eq!(
        complete(&workspace),
        (resident_violations.clone(), resident_failures.clone()),
        "eviction must not change the complete result"
    );

    apply_json(
        &workspace,
        2,
        json!([{ "path": "c.py", "language": "python", "source": "print(1)\n" }]),
    );
    assert!(workspace.evict_inactive().unwrap());
    let (edited, _) = complete(&workspace);
    assert_eq!(
        located(&edited)
            .into_iter()
            .filter(|(path, _, _)| path == "c.py")
            .count(),
        1,
        "the edited file's current results replace its evicted ones"
    );
    assert_eq!(edited.as_array().unwrap().len(), 3);
    workspace.close();
}

#[test]
fn package_context_refresh_visits_only_affected_paths_across_index_batches() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    const FILES: usize = 2_000;
    const BATCH: usize = 32;
    let code_rule = compile(
        "package-refresh-cost",
        json!([{
            "id": "typescript-only",
            "violation": { "anchor": "witness.span", "code": "COST", "message": "cost" },
            "forbid": { "ast": { "language": "typescript", "rule": { "kind": "identifier" } } }
        }]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();

    PACKAGE_CONTEXT_PATH_VISITS.store(0, Ordering::Relaxed);
    let batches = FILES.div_ceil(BATCH);
    for batch in 0..batches {
        let start = batch * BATCH;
        let files = (start..(start + BATCH).min(FILES))
            .map(|index| {
                if index % BATCH == 0 {
                    json!({ "path": format!("pkg-{batch}/__init__.py"), "language": "python",
                        "source": "" })
                } else {
                    json!({ "path": format!("loose/{index}.py"), "language": "python",
                        "source": "value = 1\n" })
                }
            })
            .collect::<Vec<_>>();
        apply_json(&workspace, (batch + 1) as u32, Value::Array(files));
    }
    assert!(
        PACKAGE_CONTEXT_PATH_VISITS.load(Ordering::Relaxed) < FILES * 8,
        "package refresh must not rescan resident paths for every index batch"
    );

    PACKAGE_CONTEXT_PATH_VISITS.store(0, Ordering::Relaxed);
    apply_json(
        &workspace,
        (batches + 1) as u32,
        json!([{ "path": "loose/1.py", "language": "python", "source": "value = 2\n" }]),
    );
    assert!(
        PACKAGE_CONTEXT_PATH_VISITS.load(Ordering::Relaxed) <= 1,
        "an unhosted edit without a marker change visits only the edited path"
    );
    workspace.close();
}

#[test]

fn repository_index_carries_each_verified_blob_hash_to_publication() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = package_repository("blob-hash-once");
    let root = directory.0.join("repo");
    let code_rule = helper_call_code_rule();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), directory.store()).unwrap());
    workspace.add_code_rule(&code_rule, None).unwrap();

    crate::git_workspace::reset_source_alias_hashes();
    let indexed = index_workspace(&workspace);
    assert_eq!(
        crate::git_workspace::source_alias_hashes(),
        indexed["readFiles"].as_u64().unwrap() as usize,
        "each tracked source is blob-hashed once while indexing and publishing"
    );
    workspace.close();
}

fn resident_module(workspace: &CodeRuleDslWorkspace, path: &str) -> Option<String> {
    workspace.state.lock().unwrap().files[path]
        .input
        .module
        .as_ref()
        .map(|module| module.text())
}

#[test]
fn a_bulk_restored_package_marker_names_unhosted_files_and_its_removal_rederives_siblings() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = package_repository("restored-marker");
    let root = directory.0.join("repo");
    let code_rule = module_code_rule();
    let mut first = CodeRuleDslWorkspace::new();
    first.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), directory.store()).unwrap());
    first.add_code_rule(&code_rule, None).unwrap();
    index_workspace(&first);
    first.completion_task().compute().unwrap();
    first.close();

    let mut restored = CodeRuleDslWorkspace::new();
    restored.state.lock().unwrap().storage =
        Some(storage_context_in(root.to_str().unwrap(), directory.store()).unwrap());
    restored.add_code_rule(&code_rule, None).unwrap();
    let indexed = index_workspace(&restored);
    assert!(
        restored.state.lock().unwrap().files["alpha/__init__.py"].restored_local_results > 0,
        "the marker enters through the bulk restore, not a request: {indexed}"
    );
    let next = indexed["discoveredFiles"].as_u64().unwrap() as u32 + 10;
    let added = apply_json(
        &restored,
        next,
        json!([{ "path": "alpha/new.py", "language": "python", "source": PACKAGE_SOURCE }]),
    );
    assert_eq!(
        resident_module(&restored, "alpha/new.py").as_deref(),
        Some("alpha.new")
    );
    assert!(
        located(&added["evaluation"]["violations"]).contains(&(
            "alpha/new.py".to_owned(),
            "alpha-helper".to_owned(),
            21
        )),
        "the restored marker names the unhosted file's package"
    );
    let mut remove = WorkspaceApplyTask {
        state: Arc::clone(&restored.state),
        closed: Arc::clone(&restored.closed),
        request_json:
            json!({ "revision": next + 1, "files": [], "removed": ["alpha/__init__.py"] })
                .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    remove.compute().unwrap();
    assert_eq!(
        resident_module(&restored, "alpha/new.py").as_deref(),
        Some("new")
    );
    assert_eq!(
        all_pages(&restored, MAXIMUM_PAGE_BYTES),
        json!([]),
        "removing the restored marker rederives every sibling's module"
    );
    restored.close();
}

#[test]
fn a_package_marker_offered_inline_names_unhosted_siblings() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let code_rule = compile(
        "inline-marker",
        json!([{
            "id": "needle",
            "violation": { "anchor": "witness.span", "code": "NEEDLE", "message": "needle" },
            "forbid": { "regex": { "source": "file.content", "pattern": "needle" } }
        }]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    apply_json(
        &workspace,
        1,
        json!([{ "path": "pkg/a.py", "language": "python", "source": "value = 1\n" }]),
    );
    assert_eq!(
        resident_module(&workspace, "pkg/a.py").as_deref(),
        Some("a")
    );
    let marker = json!([{ "path": "pkg/__init__.py", "language": "python", "source": "" }]);
    let inline = workspace
        .try_apply(json!({ "revision": 2, "files": marker, "removed": [] }).to_string())
        .unwrap();
    if inline.is_none() {
        // The host applies what the inline path declines.
        apply_json(&workspace, 2, marker);
    }
    assert_eq!(
        resident_module(&workspace, "pkg/a.py").as_deref(),
        Some("pkg.a"),
        "a new marker renames its unhosted siblings"
    );
    apply_json(
        &workspace,
        3,
        json!([{ "path": "pkg/b.py", "language": "python", "source": "value = 2\n" }]),
    );
    assert_eq!(
        resident_module(&workspace, "pkg/b.py").as_deref(),
        Some("pkg.b")
    );
    let inline = workspace
        .try_apply(
            json!({ "revision": 4, "removed": [],
                "files": [{ "path": "pkg/c.py", "language": "python", "source": "value = 3\n" }] })
            .to_string(),
        )
        .unwrap();
    assert!(
        inline.is_some(),
        "an unhosted file stays on the inline path"
    );
    assert_eq!(
        resident_module(&workspace, "pkg/c.py").as_deref(),
        Some("pkg.c"),
        "the inline path names an unhosted file by the workspace's markers"
    );
    workspace.close();
}

#[test]
fn stateless_regex_clauses_report_snapshot_matches() {
    let code_rule = compile(
        "stateless-regex",
        json!([{
            "id": "needle",
            "violation": { "anchor": "witness.span", "code": "NEEDLE", "message": "needle" },
            "forbid": { "regex": { "source": "file.content", "pattern": "needle" } }
        }]),
    );
    let snapshot = json!({ "format": "rerail.repository-snapshot/v1", "revision": 0, "facts": [],
        "files": [{ "path": "probe.py", "language": "python", "source": "needle\n" }] });
    let mut session = code_rule.open_editor(snapshot.to_string()).unwrap();
    let result: Value =
        serde_json::from_str(&session.evaluate("report".to_owned()).unwrap()).unwrap();
    assert_eq!(located(&result["violations"]).len(), 1, "{result}");
}

/// Tapes the maintenance indexer can never decode (a previous tape format left behind by a
/// `TAPE_FORMAT_VERSION` bump with unchanged grammars, or a current-format image this build
/// rejects) fill whole 64-tape backlog batches ahead of a valid tape. They must neither pin the
/// keyset cursor nor stay in the backlog: the valid tape stored after them is indexed, and an
/// undecodable tape is never narrowed.
#[test]
fn undecodable_tapes_leave_the_search_index_backlog_without_pinning_its_cursor() {
    use super::persisted::grammar_version;
    use super::storage::index_stored_tapes;
    use crate::code_rule_dsl::transducer::{POSITIONAL_INDEX_VERSION, kind_index_token};
    use crate::content_store::{SearchKindLookup, StoredKindPresence};
    use std::sync::atomic::AtomicBool;
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = TempDir::new("undecodable-backlog");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let mut undecodable = Vec::new();
    for (format, prefix) in [
        (tape::TAPE_FORMAT_VERSION - 1, "previous"),
        (tape::TAPE_FORMAT_VERSION, "corrupt"),
    ] {
        for index in 0..64 {
            let source = store
                .put_source(format!("{prefix}_{index} = {index}\n").as_bytes(), &[])
                .unwrap();
            let mut image = format.to_le_bytes().to_vec();
            image.extend_from_slice(b"not a tape image");
            undecodable.push(
                store
                    .put_tape(source, 1, grammar_version(), i64::from(format), &image)
                    .unwrap(),
            );
        }
    }
    let code_rule = print_code_rule();
    let workspace = stored_workspace(&store, "/undecodable-backlog", &code_rule);
    apply_json(
        &workspace,
        1,
        json!([{ "path": "ok.py", "language": "python", "source": "print(1)\n" }]),
    );
    let workspace_id = workspace
        .state
        .lock()
        .unwrap()
        .storage
        .as_ref()
        .unwrap()
        .workspace_id;
    let valid = store
        .overlay_file(workspace_id, "ok.py")
        .unwrap()
        .unwrap()
        .tape
        .unwrap();
    let backlog = || {
        store
            .tapes_without_search_index_after(
                grammar_version(),
                i64::from(tape::TAPE_FORMAT_VERSION),
                None,
                10_000,
            )
            .unwrap()
    };
    let mut cursor = None;
    for _ in 0..10 {
        index_stored_tapes(&store, &AtomicBool::new(false), &mut cursor);
        if backlog().is_empty() {
            break;
        }
    }
    let call = ast_grep_language::LanguageExt::get_ts_language(
        &crate::code_rule_dsl::language::GrammarDialect::Python,
    )
    .id_for_node_kind("call", true);
    let lookup = |tape_id| SearchKindLookup {
        tape_id,
        index_version: POSITIONAL_INDEX_VERSION,
        kind_tokens: vec![kind_index_token(call)],
    };
    assert_eq!(
        store
            .get_search_kind_presence_bulk(&[lookup(valid)])
            .unwrap(),
        vec![StoredKindPresence::Present],
        "the valid tape stored after 128 undecodable tapes must be indexed"
    );
    assert_eq!(
        backlog().len(),
        0,
        "tapes this build can never decode must leave the backlog"
    );
    let undecodable = store
        .get_search_kind_presence_bulk(&undecodable.into_iter().map(lookup).collect::<Vec<_>>())
        .unwrap();
    assert!(
        undecodable
            .iter()
            .all(|presence| *presence == StoredKindPresence::Unknown),
        "an undecodable tape must never be narrowed: {undecodable:?}"
    );
    workspace.close();
}

/// A modification time long before any index write, so no index entry recorded for the file is
/// racy and a warm restore verifies it by its stat alone.
fn backdate(path: &std::path::Path) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800))
        .unwrap();
}

/// A committed repository holding `files`, each backdated before `git add` so its index entry is
/// not racy.
fn backdated_repository(tag: &str, files: &[(&str, &[u8])]) -> TempDir {
    let directory = TempDir::new(tag);
    let root = directory.0.join("repo");
    for (path, bytes) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, bytes).unwrap();
        backdate(&file);
    }
    git(&root, &["init", "-q"]);
    commit(&root, "fixture");
    directory
}

/// A clean tracked file that changes after discovery and before the restore, keeping its size and
/// getting its modification time back (`touch -r`), is read: its stat no longer equals the stat
/// Git recorded in its index entry (the change time and inode change with any write), so the
/// restore does not trust discovery's clean status for it.
#[test]
fn a_warm_index_reads_a_same_size_edit_that_keeps_its_mtime_after_discovery() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let directory = backdated_repository(
        "restore-stat",
        &[("api/a.py", b"pront(1)\n"), ("lib/b.py", b"x = 2\n")],
    );
    let root = directory.0.join("repo");
    let code_rule = print_code_rule();
    let mut first = storage_workspace(&root, directory.store());
    first.add_code_rule(&code_rule, None).unwrap();
    index_workspace(&first);
    assert_eq!(all_pages(&first, MAXIMUM_PAGE_BYTES), json!([]));
    first.completion_task().compute().unwrap();
    first.close();

    // Discovery runs here, while `api/a.py` is clean.
    let mut second = storage_workspace(&root, directory.store());
    second.add_code_rule(&code_rule, None).unwrap();
    let edited = root.join("api/a.py");
    super::index::gate_next_plan(move || {
        let modified = std::fs::metadata(&edited).unwrap().modified().unwrap();
        std::fs::write(&edited, b"print(1)\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&edited)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    });
    let indexed = index_workspace(&second);
    assert_eq!(
        located(&all_pages(&second, MAXIMUM_PAGE_BYTES)),
        vec![("api/a.py".to_owned(), "print".to_owned(), 0)],
        "the restore must not reuse the committed blob's results for edited bytes: {indexed}"
    );
    assert_eq!(
        indexed["restoredFiles"],
        json!(1),
        "the unchanged sibling is still restored: {indexed}"
    );
    second.close();
}

/// Three checkouts of one commit share a content store. `lf` checks out the committed bytes;
/// `crlf` converts line endings (`core.autocrlf`), which changes the size; `swap` runs a
/// size-preserving filter driver (`p`↔`q`). A blob alias the `lf` checkout stored names bytes
/// neither converted checkout holds, so each warm index restores only results of its own bytes.
#[test]
fn checkouts_sharing_a_store_restore_only_results_of_their_own_checkout_bytes() {
    let _slot = WORKSPACE_TEST_SLOT.lock().unwrap();
    let code_rule = compile(
        "checkout-bytes",
        json!([
            { "id": "cr", "violation": { "anchor": "witness.span", "code": "CR",
                "message": "carriage return" },
              "forbid": { "regex": { "source": "file.content", "pattern": "\\r" } } },
            { "id": "print", "violation": { "anchor": "witness.span", "code": "PY",
                "message": "print" },
              "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } } }
        ]),
    );
    let directory = backdated_repository(
        "checkout-bytes",
        &[("api/a.py", b"qrint(1)\n"), ("lib/b.py", b"x = 2\n")],
    );
    let lf = directory.0.join("repo");
    git(
        &directory.0,
        &["clone", "-q", "-c", "core.autocrlf=true", "repo", "crlf"],
    );
    git(&directory.0, &["clone", "-q", "-n", "repo", "swap"]);
    let swap = directory.0.join("swap");
    std::fs::write(swap.join(".git/info/attributes"), "*.py filter=swap\n").unwrap();
    git(&swap, &["config", "filter.swap.clean", "tr pq qp"]);
    git(&swap, &["config", "filter.swap.smudge", "tr pq qp"]);
    git(&swap, &["reset", "-q", "--hard"]);
    let crlf = directory.0.join("crlf");
    assert_eq!(
        std::fs::read(crlf.join("api/a.py")).unwrap(),
        b"qrint(1)\r\n"
    );
    assert_eq!(std::fs::read(swap.join("api/a.py")).unwrap(), b"print(1)\n");
    for checkout in [&crlf, &swap] {
        for path in ["api/a.py", "lib/b.py"] {
            backdate(&checkout.join(path));
        }
        // Record the backdated stat in the index, so its entries are not racy.
        git(checkout, &["update-index", "-q", "--refresh"]);
    }
    let index = |root: &std::path::Path| {
        let mut workspace = storage_workspace(root, directory.store());
        workspace.add_code_rule(&code_rule, None).unwrap();
        let indexed = index_workspace(&workspace);
        let violations = located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES));
        workspace.completion_task().compute().unwrap();
        workspace.close();
        (indexed, violations)
    };
    let crlf_violations = vec![
        ("api/a.py".to_owned(), "cr".to_owned(), 8),
        ("lib/b.py".to_owned(), "cr".to_owned(), 5),
    ];
    let swap_violations = vec![("api/a.py".to_owned(), "print".to_owned(), 0)];
    let (indexed, violations) = index(&lf);
    assert_eq!(violations, vec![], "{indexed}");
    for round in ["cold", "warm"] {
        let (indexed, violations) = index(&crlf);
        assert_eq!(
            violations, crlf_violations,
            "{round}: the CRLF checkout's own bytes, never the LF checkout's: {indexed}"
        );
        let (indexed, violations) = index(&swap);
        assert_eq!(
            violations, swap_violations,
            "{round}: the filtered checkout's own bytes, never the LF checkout's: {indexed}"
        );
        assert_eq!(
            indexed["restoredFiles"],
            json!(1),
            "{round}: only the unfiltered lib/b.py is restored: {indexed}"
        );
        let (indexed, violations) = index(&lf);
        assert_eq!(violations, vec![], "{round}: {indexed}");
        assert_eq!(
            indexed["restoredFiles"],
            json!(2),
            "{round}: the LF checkout restores its own stored results: {indexed}"
        );
    }
}

#[test]
fn selector_indexes_skip_only_error_free_tapes_that_cannot_match() {
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDir::new("selectors");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let file =
        |path: &str, source: &str| json!({ "path": path, "language": "python", "source": source });
    let files = json!([
        file("lambda.py", "f = lambda: 1\n"),
        file("plain.py", "value = 1\n"),
        file("broken.py", "value = (\n"),
    ]);
    // Store the tapes under one catalog and build their selector indexes.
    let first = stored_workspace(&store, "/selectors-first", &print_code_rule());
    apply_json(&first, 1, files.clone());
    first.close();
    super::storage::index_stored_tapes(
        &store,
        &std::sync::atomic::AtomicBool::new(false),
        &mut None,
    );
    // A new catalog reuses no result: its clause runs over the stored tapes.
    let lambdas = compile(
        "lambdas",
        json!([{
            "id": "lambda",
            "violation": { "anchor": "witness.span", "code": "L", "message": "lambda" },
            "forbid": { "ast": { "language": "python", "rule": { "kind": "lambda" } } }
        }]),
    );
    let narrowed = stored_workspace(&store, "/selectors-second", &lambdas);
    let applied = apply_json(&narrowed, 1, files.clone());
    let mut unstored = CodeRuleDslWorkspace::new();
    unstored.add_code_rule(&lambdas, None).unwrap();
    let fresh = apply_json(&unstored, 1, files);
    unstored.close();
    for key in ["violations", "failures"] {
        assert_eq!(
            applied["evaluation"][key], fresh["evaluation"][key],
            "{key}"
        );
    }
    assert_eq!(located(&applied["evaluation"]["violations"]).len(), 1);
    // `plain.py` has no lambda and no syntax error: skipped undecoded. `broken.py` has no lambda
    // either, but its syntax errors must still be reported, so it is decoded.
    let skipped = |workspace: &CodeRuleDslWorkspace, path: &str| {
        workspace.state.lock().unwrap().files[path].selector_skipped
    };
    assert!(skipped(&narrowed, "plain.py"));
    assert!(!skipped(&narrowed, "broken.py"));
    assert!(!skipped(&narrowed, "lambda.py"));
    narrowed.close();
}
