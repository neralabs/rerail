//! Stored-workspace contracts from Phase A of the integrated query plan: every ingestion path
//! carries the same file context, persisted reuse never crosses file context, transient failures
//! are retried, and paged results agree with complete results.

use super::super::CodeRuleDslRuntime;
use super::super::matcher::UtilLimit;
use super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask};
use super::index::{WorkspaceIndexTask, repository_module, repository_package};
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
    let module = |path: &str| match super::super::module_path(
        "python".to_owned(),
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

#[test]
fn hosts_repository_ingestion_and_file_sets_share_one_package_derivation() {
    let directory = TempDir::new("package-facts");
    let root = directory.0.join("repo");
    let oversized = format!(
        "{{\"type\":\"module\",\"description\":\"{}\"}}",
        "x".repeat(1024 * 1024)
    );
    let root_manifest = r#"{ "name": "root", "type": "esm", "bin": { "a": "./bin/../cli.js", "b": "/abs.js", "c": "../out.js" },
        "engines": { "node": "not a range" }, "devEngines": { "runtime": [{ "name": "bun" }, { "name": "node", "version": "^20.9" }] },
        "dependencies": { "react": "19" }, "devDependencies": { "vitest": "4" } }"#;
    let files = [
        ("package.json", root_manifest),
        ("a.js", ""),
        (
            "esm/package.json",
            r#"{ "type": "module", "bin": "x.js", "engines": { "node": ">=22" } }"#,
        ),
        ("esm/deep/a.js", ""),
        (
            "esm/cjs/package.json",
            r#"{ "type": "commonjs", "peerDependencies": { "vue": "3" } }"#,
        ),
        ("esm/cjs/a.js", ""),
        ("broken/package.json", "{ \"type\": \"module\""),
        ("broken/a.js", ""),
        ("large/package.json", oversized.as_str()),
        ("large/a.js", ""),
        ("linked/a.js", ""),
        ("sub/a.js", ""),
    ];
    for (path, source) in files {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.join("esm/package.json"),
        root.join("linked/package.json"),
    )
    .unwrap();
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
    // What a host sends: `packageFacts`, in the shape of the wire's `package`.
    let host_as = |root: &std::path::Path, path: &str, label: &str| {
        super::super::package_facts(
            label.to_owned(),
            root.join(path).to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        )
        .map(|facts| {
            let mut wire = json!({ "folder": facts.folder, "type": facts.package_type });
            if !facts.bin.is_empty() {
                wire["bin"] = json!(facts.bin);
            }
            if let Some(version) = facts.node_version {
                wire["nodeVersion"] = json!(version);
            }
            if !facts.dependencies.is_empty() {
                wire["dependencies"] = json!(facts.dependencies);
            }
            wire
        })
    };
    let host = |root: &std::path::Path, path: &str| host_as(root, path, "javascript");
    let repository = git_workspace::discover(&root).unwrap();
    let mut manifests = crate::code_rule_dsl::package_manifest::Manifests::new(root.clone());
    let mut indexed = |path: &str, language: &str| {
        repository_package(&mut manifests, &repository, path, language)
            .map(|manifest| manifest.to_wire())
    };
    // A workspace or snapshot holding the same texts (the symlink is not one of its entries).
    let entries = files
        .iter()
        .filter_map(|(path, source)| {
            let directory = crate::code_rule_dsl::package_manifest::manifest_directory(path)?;
            Some((
                directory,
                StdArc::new(crate::code_rule_dsl::package_manifest::Manifest::parse(
                    directory.to_owned(),
                    source.as_bytes(),
                )),
            ))
        })
        .collect::<HashMap<_, _>>();
    let in_file_set = |path: &str| {
        crate::code_rule_dsl::package_manifest::manifest_in(path, |directory| {
            entries.get(directory)
        })
        .map(|manifest| manifest.to_wire())
    };
    // The nearest manifest decides. `bin` targets resolve against its folder (an absolute one or
    // one outside it never names a file below it); an unparsable `engines.node` falls through to
    // `devEngines.runtime`'s `node` entry. One without a valid `type`, malformed, oversized or a
    // symlink has nothing else either.
    let unreadable = |folder: &str| json!({ "folder": folder, "type": "unset" });
    for (path, expected) in [
        (
            "a.js",
            json!({ "folder": "", "type": "unset", "bin": ["cli.js"], "nodeVersion": "^20.9",
                "dependencies": ["react", "vitest"] }),
        ),
        (
            "esm/deep/a.js",
            json!({ "folder": "esm", "type": "module", "bin": ["x.js"], "nodeVersion": ">=22" }),
        ),
        (
            "esm/cjs/a.js",
            json!({ "folder": "esm/cjs", "type": "commonjs", "dependencies": ["vue"] }),
        ),
        ("broken/a.js", unreadable("broken")),
        ("large/a.js", unreadable("large")),
        ("sub/a.js", indexed("a.js", "javascript").unwrap()),
    ] {
        assert_eq!(host(&root, path), Some(expected.clone()), "{path}");
        assert_eq!(
            indexed(path, "javascript"),
            Some(expected.clone()),
            "{path}"
        );
        assert_eq!(in_file_set(path), Some(expected), "{path}");
    }
    #[cfg(unix)]
    for read in [
        host(&root, "linked/a.js"),
        indexed("linked/a.js", "javascript"),
    ] {
        assert_eq!(read, Some(unreadable("linked")));
    }
    // Only ECMA files carry one, and no manifest above the root counts.
    assert_eq!(indexed("a.js", "python"), None);
    assert_eq!(host_as(&root, "a.py", "python"), None);
    assert!(
        super::super::reads_package_manifests("javascript".to_owned())
            && !super::super::reads_package_manifests("python".to_owned())
    );
    assert_eq!(host(&root.join("sub"), "a.js"), None);
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
fn package_json_entries_give_unhosted_ecma_files_their_nearest_manifest() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let code_rule = compile(
        "package-depends-on",
        json!([{
            "id": "next-file",
            "violation": { "anchor": "witness.span", "code": "NEXT", "message": "next file" },
            "forbid": { "ast": { "language": "typescript",
                "file": { "package": { "dependsOn": ["next"] } }, "rule": { "kind": "program" } } }
        }]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    let source =
        |path: &str| json!({ "path": path, "language": "typescript", "source": "export {};\n" });
    let manifest = |path: &str, dependency: &str| {
        json!({ "path": path, "language": "json",
            "source": format!("{{ \"dependencies\": {{ \"{dependency}\": \"1\" }} }}") })
    };
    let reported = || {
        located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES))
            .into_iter()
            .map(|(path, _, _)| path)
            .collect::<Vec<_>>()
    };
    // Sources before their manifest, in separate requests, as a batching host may send them.
    apply_host(
        &workspace,
        1,
        json!([source("app/a.ts"), source("apps/web/b.ts")]),
        &[],
    );
    assert!(reported().is_empty());
    apply_host(
        &workspace,
        2,
        json!([manifest("package.json", "next")]),
        &[],
    );
    assert_eq!(reported(), ["app/a.ts", "apps/web/b.ts"]);
    // A nearer manifest wins below it.
    apply_host(
        &workspace,
        3,
        json!([manifest("apps/web/package.json", "vite")]),
        &[],
    );
    assert_eq!(reported(), ["app/a.ts"]);
    // Removing it, the root's applies again; a host's own `package` stays authoritative.
    let hosted = json!({ "path": "apps/web/c.ts", "language": "typescript", "source": "export {};\n",
        "package": { "folder": "apps/web", "type": "unset" } });
    apply_host(&workspace, 4, json!([hosted]), &["apps/web/package.json"]);
    assert_eq!(reported(), ["app/a.ts", "apps/web/b.ts"]);
    // Editing the root manifest's dependencies changes the result.
    apply_host(
        &workspace,
        5,
        json!([manifest("package.json", "react")]),
        &[],
    );
    assert!(reported().is_empty());
    workspace.close();
}

/// A workspace deriving a Next server graph for files the host sent no `package` for refreshes it
/// whenever a request changes what the graph reads, even a request with no such file: removing
/// the page that reached a module, editing the alias configuration (or a base it `extends`), or
/// editing the manifest alone.
#[test]
fn a_derived_server_graph_follows_removals_configuration_and_manifest_edits() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let code_rule = compile(
        "server-graph",
        json!([{
            "id": "server",
            "violation": { "anchor": "witness.span", "code": "SERVER", "message": "server module" },
            "forbid": { "ast": { "language": "typescript",
                "file": { "package": { "serverGraph": true } }, "rule": { "kind": "program" } } }
        }]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    let file = |path: &str, language: &str, source: &str| json!({ "path": path, "language": language, "source": source });
    let next = r#"{ "dependencies": { "next": "16" } }"#;
    let reported = || {
        located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES))
            .into_iter()
            .map(|(path, _, _)| path)
            .collect::<Vec<_>>()
    };
    apply_host(
        &workspace,
        1,
        json!([
            file("package.json", "json", next),
            file(
                "tsconfig.json",
                "json",
                r#"{ "extends": "./tsconfig.base.json" }"#
            ),
            file(
                "tsconfig.base.json",
                "json",
                r#"{ "compilerOptions": { "paths": { "@/*": ["./lib/*"] } } }"#
            ),
            file(
                "app/page.tsx",
                "tsx",
                "import '@/hook';\nexport default function Page() {}\n"
            ),
            file("lib/hook.ts", "typescript", "export const hook = 1;\n"),
        ]),
        &[],
    );
    assert_eq!(reported(), ["app/page.tsx", "lib/hook.ts"]);
    // A base configuration's edit alone moves the alias, and with it the module.
    apply_host(
        &workspace,
        2,
        json!([file(
            "tsconfig.base.json",
            "json",
            r#"{ "compilerOptions": { "paths": { "@/*": ["./other/*"] } } }"#
        )]),
        &[],
    );
    assert_eq!(reported(), ["app/page.tsx"]);
    apply_host(
        &workspace,
        3,
        json!([file(
            "tsconfig.base.json",
            "json",
            r#"{ "compilerOptions": { "paths": { "@/*": ["./lib/*"] } } }"#
        )]),
        &[],
    );
    assert_eq!(reported(), ["app/page.tsx", "lib/hook.ts"]);
    // Removing the page leaves the module unreached.
    apply_host(&workspace, 4, json!([]), &["app/page.tsx"]);
    assert!(reported().is_empty());
    apply_host(
        &workspace,
        5,
        json!([file(
            "app/page.tsx",
            "tsx",
            "import '@/hook';\nexport default function Page() {}\n"
        )]),
        &[],
    );
    assert_eq!(reported(), ["app/page.tsx", "lib/hook.ts"]);
    // A manifest edit alone derives the graph again rather than dropping it.
    apply_host(
        &workspace,
        6,
        json!([file(
            "package.json",
            "json",
            r#"{ "name": "app", "dependencies": { "next": "16" } }"#
        )]),
        &[],
    );
    assert_eq!(reported(), ["app/page.tsx", "lib/hook.ts"]);
    workspace.close();
}

/// A manifest that gains (or regains) `next` after its sources arrived derives the graph for
/// them, and a nested package created or removed between a page and what it imports moves the
/// boundary of the enclosing graph.
#[test]
fn a_derived_server_graph_follows_a_late_next_dependency_and_nested_packages() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let code_rule = compile(
        "server-graph-boundaries",
        json!([
            { "id": "server",
              "violation": { "anchor": "witness.span", "code": "SERVER", "message": "server" },
              "forbid": { "ast": { "language": "typescript",
                  "file": { "package": { "serverGraph": true } }, "rule": { "kind": "program" } } } },
            { "id": "client",
              "violation": { "anchor": "witness.span", "code": "CLIENT", "message": "client" },
              "forbid": { "ast": { "language": "typescript",
                  "file": { "package": { "serverGraph": false } }, "rule": { "kind": "program" } } } }
        ]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    let file = |path: &str, language: &str, source: &str| json!({ "path": path, "language": language, "source": source });
    let reported = || {
        let mut reported = located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES))
            .into_iter()
            .map(|(path, code, _)| format!("{path} {code}"))
            .collect::<Vec<_>>();
        reported.sort();
        reported
    };
    // Sources first, then a manifest without `next`: no graph, so neither value holds.
    apply_host(
        &workspace,
        1,
        json!([
            file(
                "app/page.tsx",
                "tsx",
                "import './sub/bridge';\nexport default function Page() {}\n"
            ),
            file(
                "app/sub/bridge.ts",
                "typescript",
                "export * from '../shared/hook';\n"
            ),
            file(
                "app/shared/hook.ts",
                "typescript",
                "export const hook = 1;\n"
            ),
        ]),
        &[],
    );
    apply_host(
        &workspace,
        2,
        json!([file("package.json", "json", "{}")]),
        &[],
    );
    assert!(reported().is_empty());
    let server = [
        "app/page.tsx server",
        "app/shared/hook.ts server",
        "app/sub/bridge.ts server",
    ];
    let next = r#"{ "dependencies": { "next": "16" } }"#;
    apply_host(
        &workspace,
        3,
        json!([file("package.json", "json", next)]),
        &[],
    );
    assert_eq!(reported(), server);
    // Losing and regaining `next`.
    apply_host(
        &workspace,
        4,
        json!([file("package.json", "json", "{}")]),
        &[],
    );
    assert!(reported().is_empty());
    apply_host(
        &workspace,
        5,
        json!([file("package.json", "json", next)]),
        &[],
    );
    assert_eq!(reported(), server);
    // A nested package around the bridge is no edge: the hook beyond it leaves the graph.
    apply_host(
        &workspace,
        6,
        json!([file("app/sub/package.json", "json", "{}")]),
        &[],
    );
    assert_eq!(
        reported(),
        ["app/page.tsx server", "app/shared/hook.ts client"]
    );
    apply_host(&workspace, 7, json!([]), &["app/sub/package.json"]);
    assert_eq!(reported(), server);
    workspace.close();
}

/// A derived graph keeps what it read of files outside the code rule's scope across requests: the
/// page that reaches an in-scope hook, and a root `app` folder's file of no source.
#[test]
fn a_derived_server_graph_keeps_inputs_outside_the_code_rules_scope() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let scoped = CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "server-graph-scoped", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["app/shared/**", "src/**"], "exclude": [] },
            "clauses": [{ "id": "server",
                "violation": { "anchor": "witness.span", "code": "SERVER", "message": "server" },
                "forbid": { "ast": { "language": "typescript",
                    "file": { "package": { "serverGraph": true } }, "rule": { "kind": "program" } } } }],
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&scoped, None).unwrap();
    let file = |path: &str, language: &str, source: &str| json!({ "path": path, "language": language, "source": source });
    let reported = || {
        located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES))
            .into_iter()
            .map(|(path, _, _)| path)
            .collect::<Vec<_>>()
    };
    apply_host(
        &workspace,
        1,
        json!([
            file(
                "package.json",
                "json",
                r#"{ "dependencies": { "next": "16" } }"#
            ),
            file(
                "app/page.tsx",
                "tsx",
                "import './shared/hook';\nexport default function Page() {}\n"
            ),
            file(
                "app/shared/hook.ts",
                "typescript",
                "export const hook = 1;\n"
            ),
            file("app/styles.json", "json", "{}"),
            file(
                "src/app/page.tsx",
                "tsx",
                "export default function Page() {}\n"
            ),
        ]),
        &[],
    );
    // The page outside the scope still reaches the hook; the root app folder hides src/app.
    assert_eq!(reported(), ["app/shared/hook.ts"]);
    // Editing the hook alone reads the page from the earlier request.
    apply_host(
        &workspace,
        2,
        json!([file(
            "app/shared/hook.ts",
            "typescript",
            "export const hook = 2;\n"
        )]),
        &[],
    );
    assert_eq!(reported(), ["app/shared/hook.ts"]);
    // Editing the src/app page alone keeps the root app folder present.
    apply_host(
        &workspace,
        3,
        json!([file(
            "src/app/page.tsx",
            "tsx",
            "export default function Page() { return 2; }\n"
        )]),
        &[],
    );
    assert_eq!(reported(), ["app/shared/hook.ts"]);
    // Removing every root app file uncovers src/app; a file of no source there hides it again.
    apply_host(
        &workspace,
        4,
        json!([]),
        &["app/page.tsx", "app/shared/hook.ts", "app/styles.json"],
    );
    assert_eq!(reported(), ["src/app/page.tsx"]);
    apply_host(
        &workspace,
        5,
        json!([file("app/styles.json", "json", "{}")]),
        &[],
    );
    assert!(reported().is_empty());
    workspace.close();
}

#[test]
fn a_server_graph_change_evaluates_again_only_the_files_whose_membership_it_changes() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let code_rule = compile(
        "server-graph-membership",
        json!([{ "id": "server",
            "violation": { "anchor": "witness.span", "code": "SERVER", "message": "server" },
            "forbid": { "ast": { "language": "typescript",
                "file": { "package": { "serverGraph": true } }, "rule": { "kind": "program" } } } }]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    let file = |path: &str, language: &str, source: &str| json!({ "path": path, "language": language, "source": source });
    let evaluated = |response: &Value| response["evaluation"]["metrics"]["files"].as_u64();
    let first = apply_json(
        &workspace,
        1,
        json!([
            file(
                "package.json",
                "json",
                r#"{ "dependencies": { "next": "16" } }"#
            ),
            file(
                "app/page.tsx",
                "tsx",
                "import './a';\nexport default function Page() {}\n"
            ),
            file("app/a.ts", "typescript", "export const a = 1;\n"),
            file("lib/b.ts", "typescript", "export const b = 1;\n"),
            file("lib/c.ts", "typescript", "export const c = 1;\n"),
        ]),
    );
    // The four sources and the manifest.
    assert_eq!(evaluated(&first), Some(5));
    // The page now imports a new module: the graph gains it, and no file already held changes
    // membership but the page, which the request carries.
    let second = apply_json(
        &workspace,
        2,
        json!([
            file(
                "app/page.tsx",
                "tsx",
                "import './a';\nimport './new';\nexport default function Page() {}\n"
            ),
            file("app/new.ts", "typescript", "export const n = 1;\n"),
        ]),
    );
    assert_eq!(evaluated(&second), Some(2));
    let mut reported = located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES))
        .into_iter()
        .map(|(path, _, _)| path)
        .collect::<Vec<_>>();
    reported.sort();
    assert_eq!(reported, ["app/a.ts", "app/new.ts", "app/page.tsx"]);
    workspace.close();
}

#[test]
fn a_syntax_error_is_no_failure_where_the_file_gates_exclude_the_file() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    // A negated outward relation: unsound under syntax errors, so an admitted errored file fails.
    let gated = |file: Option<Value>| {
        let mut ast = json!({ "language": "typescript",
            "when": { "sourceType": "commonjs" },
            "rule": { "kind": "identifier",
                "not": { "inside": { "kind": "function_declaration", "stopBy": "end" } } } });
        if let Some(file) = file {
            ast["file"] = file;
        }
        CodeRuleDslRuntime::compile(
            json!({
                "format": "rerail.policy/v1", "id": "gated", "revision": 1, "mode": "steer",
                "scope": { "include": ["**/*"], "exclude": [] },
                "clauses": [{ "id": "gated",
                    "violation": { "anchor": "witness.span", "code": "GATED", "message": "gated" },
                    "forbid": { "ast": ast } }],
            })
            .to_string(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap()
    };
    let failures = |compiled: &CompiledCodeRuleDsl, files: Value| {
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.add_code_rule(compiled, None).unwrap();
        let response = apply_json(&workspace, 1, files);
        workspace.close();
        response["evaluation"]["failures"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|failure| failure["code"] == "source-incomplete")
            .map(|failure| failure["path"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let file = |path: &str, language: &str, source: &str| json!({ "path": path, "language": language, "source": source });
    let typeless = || {
        json!([
            file("package.json", "json", r#"{ "name": "a" }"#),
            // An ES module by its syntax, with a syntax error.
            file(
                "esm.js",
                "javascript",
                "import x from 'x';\nfunction f() { x(; }\n"
            ),
            file(
                "cjs.js",
                "javascript",
                "require('x');\nfunction f() { x(; }\n"
            ),
        ])
    };
    // Node's syntax detection makes esm.js a module, which the commonjs-gated query skips.
    let any_runtime = gated(None);
    assert_eq!(failures(&any_runtime, typeless()), ["cjs.js"]);
    // A file whose JSX runtime the query does not list is skipped too.
    let preact = gated(Some(json!({ "jsxRuntime": ["react"] })));
    assert_eq!(
        failures(
            &preact,
            json!([file(
                "a.cjs",
                "javascript",
                "/** @jsxImportSource preact */\nx(;\n"
            )])
        ),
        Vec::<String>::new()
    );
}

/// An errored file reads the module system and JSX runtime its candidates' file gates ask for
/// once, however many gated candidates the syntax errors leave incomplete.
#[test]
fn an_errored_file_derives_its_file_gate_facts_once_for_all_gated_candidates() {
    use crate::code_rule_dsl::matcher::{JSX_RUNTIME_DERIVATIONS, SOURCE_TYPE_DERIVATIONS};
    let _slot = WORKSPACE_TEST_SLOT.lock();
    const CANDIDATES: usize = 4;
    // Negated outward relations: unsound under syntax errors, so each admitted clause fails.
    let clauses = (0..CANDIDATES)
        .map(|index| {
            json!({ "id": format!("gated-{index}"),
                "violation": { "anchor": "witness.span", "code": "GATED", "message": "gated" },
                "forbid": { "ast": { "language": "typescript",
                    "when": { "sourceType": "commonjs" }, "file": { "jsxRuntime": ["unknown"] },
                    "rule": { "kind": "identifier", "regex": format!("^x{index}$"),
                        "not": { "inside": { "kind": "function_declaration", "stopBy": "end" } } } } } })
        })
        .collect::<Vec<_>>();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&compile("gated", json!(clauses)), None)
        .unwrap();
    SOURCE_TYPE_DERIVATIONS.set(0);
    JSX_RUNTIME_DERIVATIONS.set(0);
    // A typeless package leaves the module system to the file's syntax.
    let response = apply_json(
        &workspace,
        1,
        json!([
            { "path": "package.json", "language": "json", "source": r#"{ "name": "a" }"# },
            { "path": "cjs.js", "language": "javascript",
                "source": "require('x');\nfunction f() { x(; }\n" },
        ]),
    );
    workspace.close();
    let incomplete = response["evaluation"]["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|failure| failure["code"] == "source-incomplete" && failure["path"] == "cjs.js")
        .count();
    assert_eq!(incomplete, CANDIDATES, "{response}");
    assert_eq!(
        (SOURCE_TYPE_DERIVATIONS.get(), JSX_RUNTIME_DERIVATIONS.get()),
        (1, 1)
    );
}

#[test]
fn directory_reuse_restores_results_and_evidence_on_a_second_workspace() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
    assert_hidden_edit_is_read("assume-unchanged", "api/a.py", |root| {
        git(root, &["update-index", "--assume-unchanged", "api/a.py"]);
        std::fs::write(root.join("api/a.py"), "print(1)\n").unwrap();
    });
}

#[test]
fn a_warm_index_reads_a_skip_worktree_edit() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    assert_hidden_edit_is_read("skip-worktree", "api/a.py", |root| {
        git(root, &["update-index", "--skip-worktree", "api/a.py"]);
        std::fs::write(root.join("api/a.py"), "print(1)\n").unwrap();
    });
}

#[test]
fn a_warm_index_reads_an_edit_under_a_case_variant_builtin_directory_when_git_ignores_case() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    assert_hidden_edit_is_read("ignorecase-builtin", "api/Tmp/a.py", |root| {
        git(root, &["config", "core.ignorecase", "true"]);
        std::fs::write(root.join("api/Tmp/a.py"), "print(1)\n").unwrap();
    });
}

#[test]
fn an_interactive_apply_runs_while_a_repository_index_plans_its_restore() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
        "source": "debugger; // clean text keeps the error region under half the file\nfunction broken( {\n" }]);
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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

/// A budget row a `rerail-ignore` covers is stored marked: restored from the content store, a
/// row after the first still counts (the group stays over its maximum, reported at its first row)
/// and a marked first row still silences the group's violation.
#[test]
fn restored_budget_rows_keep_their_rerail_ignore_marks() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let code_rule = compile(
        "budget-marks",
        json!([{
            "id": "budget", "violation": { "anchor": "witness.span", "code": "OVER",
                "message": "too many debugger statements" },
            "budget": {
                "query": { "evidence": {
                    "as": "usage",
                    "query": { "ast": { "language": "typescript", "rule": {
                        "kind": "debugger_statement" } } },
                    "project": { "directory": "file.directory" }
                } },
                "group_by": ["usage.directory"],
                "count": { "lte": 1 }
            }
        }]),
    );
    let ignored = "// rerail-ignore budget-marks\ndebugger;\n";
    let reported = vec![("src/a.ts".to_owned(), "budget".to_owned(), 0)];
    for (tag, first, second, expected) in [
        ("later-row", "debugger;\n", ignored, reported),
        ("reported-row", ignored, "debugger;\n", vec![]),
    ] {
        let directory = TempDir::new(&format!("budget-marks-{tag}"));
        let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
        let files = json!([
            { "path": "src/a.ts", "language": "typescript", "source": first },
            { "path": "src/b.ts", "language": "typescript", "source": second },
        ]);
        let cold = stored_workspace(&store, "/budget-marks", &code_rule);
        let result = apply_json(&cold, 1, files.clone());
        assert_eq!(
            result["evaluation"]["failures"],
            json!([]),
            "{tag}: {result}"
        );
        assert_eq!(
            located(&result["evaluation"]["violations"]),
            expected,
            "{tag}: cold"
        );
        cold.close();

        let restarted = stored_workspace(&store, "/budget-marks", &code_rule);
        let warm = apply_json(&restarted, 1, files);
        assert_eq!(
            restored_results(&restarted),
            2,
            "{tag}: both files' rows come from the store"
        );
        assert_eq!(warm["evaluation"]["failures"], json!([]), "{tag}: {warm}");
        assert_eq!(
            located(&warm["evaluation"]["violations"]),
            expected,
            "{tag}: restored"
        );
        restarted.close();
    }
}

/// A file's clauses share a work ceiling: once the clauses before it spent it, a clause fails as
/// a coverage gap (`FILE_CLAUSE_WORK_EXCEEDED`) instead of running, so a file's total work is
/// bounded however many clauses run on it. That failure is the other clauses' doing, not the
/// clause's verdict on the content: the workspace keeps it, the store never does, and a later
/// workspace evaluates the clause again.
#[test]
fn a_file_s_clauses_share_a_work_ceiling_whose_failures_are_never_stored() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("clause-ceiling");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let print = |id: &str| {
        json!({
            "id": id, "violation": { "anchor": "witness.span", "code": "PY", "message": "print" },
            "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
        })
    };
    let code_rule = compile(
        "clause-ceiling",
        json!([print("a"), print("b"), print("c")]),
    );
    let file = json!([{ "path": "ceiling.py", "language": "python", "source": "print(1)\n" }]);
    let workspace = stored_workspace(&store, "/clause-ceiling", &code_rule);
    // Every clause polls its allowance at least once: one unit is spent by the first.
    TEST_CLAUSE_WORK_CEILING.set(Some(1));
    let capped = apply_json(&workspace, 1, file.clone());
    TEST_CLAUSE_WORK_CEILING.set(None);
    let failures = capped["evaluation"]["failures"].as_array().unwrap().clone();
    assert!(
        failures
            .iter()
            .all(|failure| failure["message"] == FILE_CLAUSE_WORK_EXCEEDED),
        "{capped}"
    );
    assert!(
        failures.iter().any(|failure| failure["clauseId"] == "c"),
        "the last clause finds the ceiling spent: {capped}"
    );
    assert_eq!(
        failures.len() + located(&capped["evaluation"]["violations"]).len(),
        3,
        "{capped}"
    );
    {
        let state = workspace.state.lock().unwrap();
        let file = &state.files["ceiling.py"];
        assert!(!file.transient_failure);
        assert_eq!(file.unstored.len(), failures.len());
    }
    // The same content, in the same workspace, is not evaluated again.
    let again = apply_json(&workspace, 2, file.clone());
    assert_eq!(
        again["evaluation"]["failures"],
        capped["evaluation"]["failures"]
    );
    workspace.close();

    let restarted = stored_workspace(&store, "/clause-ceiling", &code_rule);
    let warm = apply_json(&restarted, 1, file);
    assert_eq!(
        restored_results(&restarted),
        3 - failures.len(),
        "only the clauses that ran are restored: {warm}"
    );
    assert_eq!(warm["evaluation"]["failures"], json!([]), "{warm}");
    assert_eq!(
        located(&warm["evaluation"]["violations"]).len(),
        3,
        "{warm}"
    );
    restarted.close();
}

/// A clause whose required text the file lacks matches nothing and costs nothing: it answers even
/// after the clauses before it spent the file's clause-work ceiling, instead of failing.
#[test]
fn a_clause_with_nothing_to_match_answers_past_the_ceiling() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let clause = |id: &str, call: &str| {
        json!({
            "id": id, "violation": { "anchor": "witness.span", "code": "PY", "message": "call" },
            "forbid": { "ast": { "language": "python", "rule": { "pattern": format!("{call}($A)") } } }
        })
    };
    let code_rule = compile(
        "nothing-to-match",
        json!([
            clause("print", "print"),
            clause("absent", "never_called_anywhere")
        ]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    TEST_CLAUSE_WORK_CEILING.set(Some(1));
    let applied = apply_json(
        &workspace,
        1,
        json!([{ "path": "needle.py", "language": "python", "source": "print(1)\n" }]),
    );
    TEST_CLAUSE_WORK_CEILING.set(None);
    let failed = applied["evaluation"]["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|failure| failure["clauseId"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(!failed.contains(&"absent".to_owned()), "{applied}");
    workspace.close();
}

/// Running out of work in a pass over some of a file's clauses is no verdict on the content: run
/// with the others, the clauses before it would have left tables and verdicts built. Clause `b` is
/// evaluated alone (`a`'s result is restored) and runs out: that failure is never stored, and a
/// later workspace evaluates `b` again.
#[test]
fn running_out_of_work_in_a_pass_over_some_clauses_is_never_stored() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("partial-pass");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    // `a` polls before scanning and once for its match, before `b` runs.
    let code_rule = compile(
        "partial-pass",
        json!([
            {
                "id": "a", "violation": { "anchor": "witness.span", "code": "PY", "message": "print" },
                "forbid": { "regex": { "source": "file.content", "pattern": "print" } }
            },
            {
                "id": "b", "violation": { "anchor": "witness.span", "code": "PY", "message": "print" },
                "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
            }
        ]),
    );
    let file = json!([{ "path": "partial.py", "language": "python", "source": "print(1)\n" }]);
    // `a` runs and is stored; the ceiling stops `b`, which is not.
    let first = stored_workspace(&store, "/partial-pass", &code_rule);
    TEST_CLAUSE_WORK_CEILING.set(Some(2));
    let capped = apply_json(&first, 1, file.clone());
    TEST_CLAUSE_WORK_CEILING.set(None);
    assert_eq!(
        capped["evaluation"]["failures"][0]["message"], FILE_CLAUSE_WORK_EXCEEDED,
        "{capped}"
    );
    first.close();
    // `a` is restored; `b`, evaluated alone, runs out of its allowance.
    let partial = stored_workspace(&store, "/partial-pass", &code_rule);
    TEST_CLAUSE_WORK_BUDGET.set(Some(0));
    let ran_out = apply_json(&partial, 1, file.clone());
    TEST_CLAUSE_WORK_BUDGET.set(None);
    assert_eq!(restored_results(&partial), 1, "{ran_out}");
    assert_eq!(
        ran_out["evaluation"]["failures"][0]["clauseId"], "b",
        "{ran_out}"
    );
    partial.close();
    let later = stored_workspace(&store, "/partial-pass", &code_rule);
    let warm = apply_json(&later, 1, file);
    assert_eq!(warm["evaluation"]["failures"], json!([]), "{warm}");
    assert_eq!(
        located(&warm["evaluation"]["violations"]).len(),
        2,
        "{warm}"
    );
    later.close();
}

/// A clause that fails a file for its syntax (unsound under errors) fails it whatever ran before
/// it: that failure is the content's verdict, stored and restored like any other.
#[test]
fn a_syntax_failure_is_stored_and_restored() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("syntax-stored");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let code_rule = compile(
        "syntax-stored",
        json!([{
            "id": "unsound", "violation": { "anchor": "witness.span", "code": "U", "message": "u" },
            "forbid": { "ast": { "language": "python", "rule": {
                "kind": "call", "not": { "follows": { "kind": "import_statement", "stopBy": "end" } } } } }
        }]),
    );
    let file =
        json!([{ "path": "broken.py", "language": "python", "source": "print(3)\ndef (:\n" }]);
    let workspace = stored_workspace(&store, "/syntax-stored", &code_rule);
    let cold = apply_json(&workspace, 1, file.clone());
    assert_eq!(
        cold["evaluation"]["failures"][0]["code"], "source-incomplete",
        "{cold}"
    );
    assert!(
        workspace.state.lock().unwrap().files["broken.py"]
            .unstored
            .is_empty()
    );
    workspace.close();
    let restarted = stored_workspace(&store, "/syntax-stored", &code_rule);
    let warm = apply_json(&restarted, 1, file);
    assert_eq!(restored_results(&restarted), 1, "{warm}");
    assert_eq!(
        warm["evaluation"]["failures"], cold["evaluation"]["failures"],
        "{warm}"
    );
    restarted.close();
}

/// A file whose shared work (its parse, its tape's restore) runs out keeps no result: how much of
/// that work an evaluation does depends on what an earlier one stored (a stored tape skips the
/// parse), so the failure is not the content's verdict. It is transient: evaluated again by the
/// next apply, never stored.
#[test]
fn a_file_whose_shared_work_runs_out_is_evaluated_again() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("deadline");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let print = |id: &str| {
        compile(
            id,
            json!([{
                "id": id, "violation": { "anchor": "witness.span", "code": "PY",
                    "message": "print" },
                "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
            }]),
        )
    };
    let file = json!([{ "path": "slow.py", "language": "python", "source": "print(1)\n" }]);
    let printed = vec![("slow.py".to_owned(), "print".to_owned(), 0)];

    // The parse runs out.
    let workspace = stored_workspace(&store, "/deadline", &print("print"));
    TEST_FILE_WORK_BUDGET.set(Some(0));
    let ran_out = apply_json(&workspace, 1, file.clone());
    TEST_FILE_WORK_BUDGET.set(None);
    assert_eq!(ran_out["evaluation"]["violations"], json!([]));
    assert_eq!(
        ran_out["evaluation"]["failures"][0]["code"], "file-budget-exceeded",
        "{ran_out}"
    );
    assert!(workspace.state.lock().unwrap().files["slow.py"].transient_failure);
    let again = apply_json(&workspace, 2, file.clone());
    assert_eq!(
        located(&again["evaluation"]["violations"]),
        printed,
        "{again}"
    );
    workspace.close();

    // Another rule set reads the file's stored tape: its restore runs out instead.
    let other = print("other");
    let restoring = stored_workspace(&store, "/deadline-restore", &other);
    TEST_FILE_WORK_BUDGET.set(Some(0));
    let ran_out = apply_json(&restoring, 1, file.clone());
    TEST_FILE_WORK_BUDGET.set(None);
    assert_eq!(
        ran_out["evaluation"]["failures"][0]["message"], "file tape restoration failed",
        "{ran_out}"
    );
    assert!(restoring.state.lock().unwrap().files["slow.py"].transient_failure);
    restoring.close();
    let restarted = stored_workspace(&store, "/deadline-restore", &other);
    let warm = apply_json(&restarted, 1, file);
    assert_eq!(warm["evaluation"]["failures"], json!([]), "{warm}");
    assert_eq!(
        located(&warm["evaluation"]["violations"]),
        vec![("slow.py".to_owned(), "other".to_owned(), 0)],
        "{warm}"
    );
    restarted.close();
}

/// A clause that runs out of work fails alone: the clauses before it keep their violations, and
/// the clauses after it still run under allowances of their own. Running out of its own allowance
/// depends on the content alone, so the result is the content's verdict: persisted, and restored
/// by a later workspace instead of paid for again. Before, one shared file budget failed every
/// clause of the file, dropped the violations of those that had finished, and was re-run on every
/// apply.
#[test]
fn a_clause_that_runs_out_of_work_fails_alone() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("clause-allowance");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let print = |id: &str| {
        json!({
            "id": id, "violation": { "anchor": "witness.span", "code": "PY", "message": "print" },
            "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
        })
    };
    // Enough roots to exhaust the clause allowance even when candidate selection and sibling
    // checks share their work. No required literal rules the file out before matching.
    let costly = json!({
        "id": "costly", "violation": { "anchor": "witness.span", "code": "SLOW", "message": "slow" },
        "forbid": { "ast": { "language": "python", "rule": {
            "kind": "integer", "follows": { "stopBy": "end", "regex": "^[a-z]{40}$" } } } }
    });
    let code_rule = compile(
        "clause-allowance",
        json!([print("before"), costly, print("after")]),
    );
    let numbers = (0..30_000)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let file = json!([{ "path": "big.py", "language": "python",
        "source": format!("print(1)\nx = [{numbers}]\n") }]);
    let workspace = stored_workspace(&store, "/clause-allowance", &code_rule);
    TEST_CLAUSE_WORK_BUDGET.set(Some(20));
    let evaluated = apply_json(&workspace, 1, file.clone());
    TEST_CLAUSE_WORK_BUDGET.set(None);
    let clauses = |values: &Value| {
        let mut clauses = values
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["clauseId"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        clauses.sort();
        clauses
    };
    assert_eq!(
        clauses(&evaluated["evaluation"]["violations"]),
        ["after", "before"],
        "{evaluated}"
    );
    let failures = evaluated["evaluation"]["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{evaluated}");
    assert_eq!(
        [
            &failures[0]["clauseId"],
            &failures[0]["code"],
            &failures[0]["message"],
            &failures[0]["path"]
        ],
        [
            &json!("costly"),
            &json!("budget-exceeded"),
            &json!(CLAUSE_WORK_BUDGET_EXCEEDED),
            &json!("big.py")
        ],
    );
    assert!(
        !workspace.state.lock().unwrap().files["big.py"].transient_failure,
        "running out of work is the content's verdict, not a transient failure"
    );
    workspace.close();

    // A later workspace, under the full allowance, reads every clause's stored result back,
    // the costly clause's failure included, instead of evaluating the file again.
    let restarted = stored_workspace(&store, "/clause-allowance", &code_rule);
    let warm = apply_json(&restarted, 1, file);
    assert_eq!(restored_results(&restarted), 3, "{warm}");
    assert_eq!(
        warm["evaluation"]["failures"], evaluated["evaluation"]["failures"],
        "{warm}"
    );
    restarted.close();
}

/// A utility verdict a clause computed while its allowance ran out is not remembered for the
/// clauses after it: the second clause reads the same utility at the same node and either finds
/// its violation or fails, never reports nothing. Both share a `define`'s utility, so a verdict
/// one remembers the other reads.
#[test]
fn a_verdict_cut_short_by_a_spent_allowance_is_not_remembered_for_later_clauses() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("clause-allowance-memo");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    // A double negation of the utility: the anchor plan selects by `kind` alone, so a clause's
    // allowance is spent matching the utility, not selecting its roots.
    let reads = |id: &str| {
        json!({
            "id": id, "violation": { "anchor": "witness.span", "code": "LATE", "message": "late" },
            "forbid": { "ast": { "language": "python", "rule": {
                "kind": "list", "not": { "not": { "matches": "late" } } } } }
        })
    };
    // A list holding a five-digit number, which only its 10,000th element is: no literal to
    // search for, so the walk reads the elements one by one.
    let shared = json!({ "id": "shared", "define": { "language": "python", "utils": {
        "late": { "kind": "list", "has": { "stopBy": "end", "kind": "integer", "regex": "^[0-9]{5}$" } }
    } } });
    let code_rule = compile(
        "clause-allowance-memo",
        json!([shared, reads("first"), reads("second")]),
    );
    let numbers = (0..10_001)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let file = json!([{ "path": "late.py", "language": "python", "source": format!("x = [{numbers}]\n") }]);
    let workspace = stored_workspace(&store, "/clause-allowance-memo", &code_rule);
    TEST_CLAUSE_WORK_BUDGET.set(Some(5));
    let evaluated = apply_json(&workspace, 1, file);
    TEST_CLAUSE_WORK_BUDGET.set(None);
    let named = |list: &Value| {
        list.as_array()
            .unwrap()
            .iter()
            .filter(|item| item["clauseId"] == "second")
            .count()
    };
    assert_eq!(
        evaluated["evaluation"]["failures"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["clauseId"] == "first")
            .count(),
        1,
        "the first clause runs out: {evaluated}"
    );
    assert_eq!(
        named(&evaluated["evaluation"]["violations"]) + named(&evaluated["evaluation"]["failures"]),
        1,
        "the second clause reports its violation or fails, never nothing: {evaluated}"
    );
    workspace.close();
}

#[test]
fn an_evaluation_that_finishes_past_an_unobserved_deadline_is_kept() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
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

/// A wall-clock deadline (the inline fast path's budget) passes after the first clause's last
/// poll: the second clause is skipped, and the evaluation must be known cut short (`observed`,
/// which makes the inline path decline its result) and transient, so that the background apply
/// evaluates the content again. Before, the check between clauses read the clock without
/// recording what it found, so the cut-short result read as complete.
#[test]
fn an_evaluation_whose_deadline_passes_between_clauses_is_known_cut_short() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let print = |id: &str| {
        json!({
            "id": id, "violation": { "anchor": "witness.span", "code": "PY", "message": "print" },
            "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }
        })
    };
    let code_rule = compile("inline-deadline", json!([print("first"), print("second")]));
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    let closed = StdArc::clone(&workspace.closed);
    let mut state = workspace.state.lock().unwrap();
    let WorkspaceState {
        code_rules,
        candidate_sets,
        grammars,
        ..
    } = &mut *state;
    let mut file = super::ingest::make_file(
        FileInput {
            path: "late.py".into(),
            language: "python".into(),
            source: StdArc::new("print(1)\n".to_owned()),
            module: None,
            package: None,
        },
        1,
        code_rules,
        candidate_sets,
    );
    let budget =
        crate::code_rule_dsl::budget::MatchBudget::until(&closed, Duration::from_secs(600));
    TEST_DEADLINE_PASSES_AT.set(Some("clause-done"));
    super::evaluate::evaluate_file(
        &mut file, code_rules, grammars, &closed, false, &budget, &mut None,
    )
    .unwrap();
    TEST_DEADLINE_PASSES_AT.set(None);
    assert_eq!(
        file.failures.len(),
        1,
        "the second clause was skipped: {:?}",
        file.failures
    );
    assert!(
        budget.observed(),
        "the cut-short evaluation reads as complete"
    );
    assert!(file.transient_failure, "the cut-short evaluation is kept");
    drop(state);
    workspace.close();
}

/// The file's budget is spent before a clause whose required text the file lacks: that clause
/// matches nothing and costs nothing, so it answers instead of failing for the file's work.
#[test]
fn a_clause_with_nothing_to_match_answers_past_the_file_budget() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let clause = |id: &str, call: &str| {
        json!({
            "id": id, "violation": { "anchor": "witness.span", "code": "PY", "message": "call" },
            "forbid": { "ast": { "language": "python", "rule": { "pattern": format!("{call}($A)") } } }
        })
    };
    let code_rule = compile(
        "nothing-to-match-late",
        json!([
            clause("print", "print"),
            clause("absent", "never_called_anywhere")
        ]),
    );
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    let closed = StdArc::clone(&workspace.closed);
    let mut state = workspace.state.lock().unwrap();
    let WorkspaceState {
        code_rules,
        candidate_sets,
        grammars,
        ..
    } = &mut *state;
    let mut file = super::ingest::make_file(
        FileInput {
            path: "late.py".into(),
            language: "python".into(),
            source: StdArc::new("print(1)\n".to_owned()),
            module: None,
            package: None,
        },
        1,
        code_rules,
        candidate_sets,
    );
    let budget =
        crate::code_rule_dsl::budget::MatchBudget::until(&closed, Duration::from_secs(600));
    TEST_DEADLINE_PASSES_AT.set(Some("clause-done"));
    super::evaluate::evaluate_file(
        &mut file, code_rules, grammars, &closed, false, &budget, &mut None,
    )
    .unwrap();
    TEST_DEADLINE_PASSES_AT.set(None);
    assert!(file.failures.is_empty(), "{:?}", file.failures);
    drop(state);
    workspace.close();
}

/// Two clauses read the same lazily built per-file table (`file_table` / `file_php_names`). The
/// first runs out of its own allowance while building it: that is no verdict on the table, so
/// the second, under an allowance of its own, builds it again and reports its violation.
fn a_table_build_cut_short_by_one_allowance_is_built_again(
    language: &str,
    path: &str,
    source: String,
    rule: Value,
) {
    let clause = |id: &str| {
        json!({
            "id": id, "violation": { "anchor": "witness.span", "code": "R", "message": "r" },
            "forbid": { "ast": { "language": language, "rule": rule } }
        })
    };
    let code_rule = compile("table-rebuild", json!([clause("first"), clause("second")]));
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&code_rule, None).unwrap();
    // Enough for the first clause to reach the build, not to finish it.
    TEST_NEXT_CLAUSE_WORK_BUDGET.set(Some(8));
    let applied = apply_json(
        &workspace,
        1,
        json!([{ "path": path, "language": language, "source": source }]),
    );
    TEST_NEXT_CLAUSE_WORK_BUDGET.set(None);
    let of = |list: &Value, clause: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .filter(|item| item["clauseId"] == clause)
            .count()
    };
    let first = applied["evaluation"]["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["clauseId"] == "first")
        .map(|item| item["message"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        first,
        [json!(UtilLimit::Scope.message())],
        "the first clause runs out building the table: {applied}"
    );
    assert_eq!(
        (
            of(&applied["evaluation"]["violations"], "second"),
            of(&applied["evaluation"]["failures"], "second")
        ),
        (1, 0),
        "the second clause builds the table again: {applied}"
    );
    workspace.close();
}

#[test]
fn a_scope_table_cut_short_by_one_allowance_is_built_again() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    // Thousands of statements for the build to walk, and one identifier for the clauses to read.
    a_table_build_cut_short_by_one_allowance_is_built_again(
        "python",
        "table.py",
        format!("{}print(1)\n", "1\n".repeat(5_000)),
        json!({ "kind": "identifier", "regex": "^print$", "resolves": { "global": ["print"] } }),
    );
}

#[test]
fn php_names_cut_short_by_one_allowance_are_built_again() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    a_table_build_cut_short_by_one_allowance_is_built_again(
        "php",
        "names.php",
        format!("<?php\n{}random_int(1, 2);\n", "1;\n".repeat(5_000)),
        json!({ "kind": "function_call_expression",
            "has": { "field": "function", "resolves": { "import": ["random_int"] } } }),
    );
}

#[test]
fn an_ast_evaluation_that_finishes_past_an_unobserved_deadline_is_kept() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
        [
            &applied["evaluation"]["failures"][0]["code"],
            &applied["evaluation"]["failures"][0]["message"]
        ],
        [&json!("input-limit"), &json!("file input budget exceeded")],
        "{applied}"
    );
    assert_eq!(
        applied["evaluation"]["failures"],
        fresh["evaluation"]["failures"]
    );
}

#[test]
fn node_references_resolve_through_the_store_and_refuse_stale_versions() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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
    let _slot = WORKSPACE_TEST_SLOT.lock();
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

/// A Next package with no server graph (one of its folders cannot be read on disk, or a file set
/// holds more sources than `module_graph::MAX_GRAPH_FILES`) gives its files no membership:
/// neither `serverGraph: true` nor `serverGraph: false` holds, rather than every file outside.
#[test]
fn a_package_without_a_server_graph_gives_its_files_no_membership() {
    let next = r#"{ "dependencies": { "next": "16" } }"#;
    let page = "export default function Page() {}\n";
    // The file-set path: one source past the bound.
    let mut files = vec![
        json!({ "path": "package.json", "language": "json", "source": next }),
        json!({ "path": "app/page.tsx", "language": "tsx", "source": page }),
    ];
    files.extend((0..20_000).map(|index| {
        json!({ "path": format!("lib/m{index}.ts"), "language": "typescript",
            "source": "export {};\n" })
    }));
    let snapshot = json!({ "facts": [], "files": files,
        "format": "rerail.repository-snapshot/v1", "revision": 0 });
    let (snapshot, _) =
        crate::code_rule_dsl::eval::parse_snapshot(&snapshot.to_string(), true).unwrap();
    let package = snapshot.files[1]
        .package
        .as_ref()
        .expect("the page's manifest");
    assert_eq!(package.in_server_graph("app/page.tsx"), None);
    // The disk path: a folder of the package cannot be read.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = TempDir::new("unread-server-graph");
        let root = directory.0.join("repo");
        for (path, source) in [
            ("package.json", next),
            ("app/page.tsx", page),
            ("app/locked/x.ts", "export {};\n"),
        ] {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, source).unwrap();
        }
        let membership = || {
            crate::code_rule_dsl::package_manifest::Manifests::new(root.clone())
                .reading_server_graphs(true)
                .of_file(&root.join("app/page.tsx"))
                .expect("the page's manifest")
                .in_server_graph("app/page.tsx")
        };
        let locked = root.join("app/locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unread = membership();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(unread, None);
        assert_eq!(membership(), Some(true));
    }
}

/// A folder below the root whose path resolves outside it (a committed `vendor -> /home/user`)
/// holds no manifest: the lookup reads nothing there and goes on to the folders above it.
#[cfg(unix)]
#[test]
fn a_symlinked_folder_pointing_outside_the_root_holds_no_manifest() {
    let directory = TempDir::new("manifest-outside-root");
    let root = directory.0.join("repo");
    let outside = directory.0.join("outside");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(outside.join("app")).unwrap();
    std::fs::write(
        outside.join("package.json"),
        r#"{ "type": "module", "dependencies": { "next": "16" } }"#,
    )
    .unwrap();
    std::fs::write(outside.join("x.js"), "").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("vendor")).unwrap();
    // A symlink to a folder inside the root still reads its manifest.
    std::fs::create_dir_all(root.join("inner")).unwrap();
    std::fs::write(root.join("inner/package.json"), r#"{ "type": "module" }"#).unwrap();
    std::os::unix::fs::symlink(root.join("inner"), root.join("alias")).unwrap();
    let package = |path: &str| {
        crate::code_rule_dsl::package_manifest::Manifests::new(root.clone())
            .reading_server_graphs(true)
            .of_file(&root.join(path))
            .map(|manifest| manifest.to_wire())
    };
    assert_eq!(package("vendor/x.js"), None);
    assert_eq!(
        package("alias/x.js"),
        Some(json!({ "folder": "alias", "type": "module" }))
    );
    std::fs::write(root.join("package.json"), "{}").unwrap();
    assert_eq!(
        package("vendor/x.js"),
        Some(json!({ "folder": "", "type": "unset" }))
    );
}

/// A clean tree's reuse follows the configuration a nested package's `extends` reaches outside
/// it: editing the base's `jsxImportSource` leaves the tree's key as it was, but each restored
/// file's results are keyed by its context (`file_context`), whose package digest carries the
/// `jsxImportSource` read again, so the file is evaluated again rather than restored.
#[test]
fn a_reused_directory_follows_a_configuration_base_outside_it() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let code_rule = compile(
        "jsx-other",
        json!([{
            "id": "other",
            "violation": { "anchor": "witness.span", "code": "OTHER", "message": "other JSX" },
            "forbid": { "ast": { "language": "typescript",
                "file": { "jsxRuntime": ["other"] }, "rule": { "kind": "program" } } }
        }]),
    );
    let base = |source: &str| {
        format!("{{ \"compilerOptions\": {{ \"jsxImportSource\": \"{source}\" }} }}\n")
    };
    let preact = base("preact");
    let directory = backdated_repository(
        "reuse-outside-base",
        &[
            ("tsconfig.base.json", preact.as_bytes()),
            (
                "packages/web/package.json",
                br#"{ "dependencies": { "react": "19" } }"#,
            ),
            (
                "packages/web/tsconfig.json",
                br#"{ "extends": "../../tsconfig.base.json" }"#,
            ),
            ("packages/web/src/a.ts", b"export const a = 1;\n"),
        ],
    );
    let root = directory.0.join("repo");
    let index = || {
        let mut workspace = storage_workspace(&root, directory.store());
        workspace.add_code_rule(&code_rule, None).unwrap();
        let indexed = index_workspace(&workspace);
        let violations = located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES));
        workspace.completion_task().compute().unwrap();
        workspace.close();
        (indexed, violations)
    };
    let other = vec![("packages/web/src/a.ts".to_owned(), "other".to_owned(), 0)];
    let (indexed, violations) = index();
    assert_eq!(violations, other, "{indexed}");
    let (indexed, violations) = index();
    assert_eq!(violations, other, "{indexed}");
    assert_eq!(indexed["restoredFiles"], json!(1), "{indexed}");
    // The edit dirties only the root tree; `packages/` stays clean.
    std::fs::write(root.join("tsconfig.base.json"), base("react")).unwrap();
    let (indexed, violations) = index();
    assert_eq!(violations, vec![], "{indexed}");
    assert_eq!(indexed["restoredFiles"], json!(0), "{indexed}");
}

/// A request that fails after the workspace derived its package context (here, the store refuses
/// a removal that is no repository path) leaves that context as it was: later server graphs read
/// only what the workspace accepted.
#[test]
fn a_request_that_fails_partway_leaves_the_package_context_unchanged() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let code_rule = compile(
        "server-graph-atomic",
        json!([{ "id": "server",
            "violation": { "anchor": "witness.span", "code": "SERVER", "message": "server" },
            "forbid": { "ast": { "language": "typescript",
                "file": { "package": { "serverGraph": true } }, "rule": { "kind": "program" } } } }]),
    );
    let directory = TempDir::new("context-atomic");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let workspace = stored_workspace(&store, "/context-atomic", &code_rule);
    let file = |path: &str, language: &str, source: &str| json!({ "path": path, "language": language, "source": source });
    let next = r#"{ "dependencies": { "next": "16" } }"#;
    let page = |import: &str| format!("import '{import}';\nexport default function Page() {{}}\n");
    let reported = || {
        let mut reported = located(&all_pages(&workspace, MAXIMUM_PAGE_BYTES))
            .into_iter()
            .map(|(path, _, _)| path)
            .collect::<Vec<_>>();
        reported.sort();
        reported
    };
    apply_host(
        &workspace,
        1,
        json!([
            file("package.json", "json", next),
            file("app/page.tsx", "tsx", &page("./a")),
            file("app/a.ts", "typescript", "export const a = 1;\n"),
            file("lib/b.ts", "typescript", "export const b = 1;\n"),
        ]),
        &[],
    );
    assert_eq!(reported(), ["app/a.ts", "app/page.tsx"]);
    let context = || {
        let state = workspace.state.lock().unwrap();
        (
            state.graph_entries.keys().cloned().collect::<Vec<_>>(),
            state.server_graph_directories.clone(),
            state.module_configs.clone(),
            state
                .manifests
                .iter()
                .map(|(directory, manifest)| (directory.clone(), *manifest.digest()))
                .collect::<Vec<_>>(),
        )
    };
    let before = context();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 2, "removed": ["../escape.ts"], "files": [
            file("app/page.tsx", "tsx", &page("../lib/b")),
            file("tsconfig.json", "json", "{}"),
            file("web/package.json", "json", next),
            file("web/app/page.tsx", "tsx", &page("./c")),
        ] })
        .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    task.compute().unwrap_err();
    assert!(
        context() == before,
        "the failed request changed the context"
    );
    // A later request deriving the graph again reads the page the workspace holds.
    apply_host(
        &workspace,
        3,
        json!([file("lib/c.ts", "typescript", "export const c = 1;\n")]),
        &[],
    );
    assert_eq!(reported(), ["app/a.ts", "app/page.tsx"]);
    workspace.close();
}

/// A code rule `id` whose clauses read the documents Vue and Blade files derive
/// (`ast.embedded`), and a Python file's own tree.
fn derived_documents_code_rule(id: &str) -> CompiledCodeRuleDsl {
    let clause = |id: &str, ast: Value| {
        json!({
            "id": id,
            "violation": { "anchor": "witness.span", "code": "DERIVED", "message": id },
            "forbid": { "ast": ast }
        })
    };
    CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": id, "revision": 1, "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [
                clause("vue-script", json!({ "language": "vue", "embedded": "script-block",
                    "rule": { "kind": "lexical_declaration" } })),
                clause("blade-view", json!({ "language": "blade", "embedded": "compiled-view",
                    "rule": { "kind": "binary_expression" } })),
                clause("python", json!({ "language": "python", "rule": { "kind": "call" } })),
            ],
        })
        .to_string(),
        "json".into(),
        include_str!("../capabilities.json").into(),
    )
    .unwrap()
}

/// Vue and Blade files whose derived documents hold witnesses, others whose do not, and a
/// single-language file.
fn derived_document_files() -> Value {
    let file = |path: &str, language: &str, source: &str| json!({ "path": path, "language": language, "source": source });
    json!([
        file(
            "src/Counter.vue",
            "vue",
            "<script setup>\nconst count = ref(0)\nconst double = count.value * 2\n</script>\n\
             <template>\n  <p>{{ double }}</p>\n</template>\n",
        ),
        file(
            "src/Plain.vue",
            "vue",
            "<script setup>\nload()\n</script>\n<template>\n  <p>{{ title }}</p>\n</template>\n",
        ),
        file(
            "views/sum.blade.php",
            "blade",
            "<h1>{{ $title }}</h1>\n@if ($count > 0)\n  <p>{{ $a + $b }}</p>\n@endif\n",
        ),
        file("views/plain.blade.php", "blade", "<p>{{ $name }}</p>\n"),
        file("probe.py", "python", "print(1)\n"),
    ])
}

/// The stored tape of `path` in `workspace`'s overlay.
fn overlay_tape(
    store: &ContentStore,
    workspace: &CodeRuleDslWorkspace,
    path: &str,
) -> crate::content_store::TapeId {
    let workspace_id = workspace
        .state
        .lock()
        .unwrap()
        .storage
        .as_ref()
        .unwrap()
        .workspace_id;
    store
        .overlay_file(workspace_id, path)
        .unwrap()
        .unwrap()
        .tape
        .unwrap()
}

/// Vue and Blade files published to the store and evaluated again from it: each file's tape and
/// the documents it derives (`file_documents`) restore, and an `ast.embedded` clause finds the
/// witnesses a fresh parse finds. A single-language file has no `file_documents` row, and a
/// clause over a derived document is narrowed by that document's own selector index.
#[test]
fn derived_documents_restored_from_the_store_evaluate_as_a_fresh_parse() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("derived-documents");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let files = derived_document_files();
    let first = stored_workspace(
        &store,
        "/derived-first",
        &derived_documents_code_rule("derived-first"),
    );
    apply_json(&first, 1, files.clone());
    for path in ["src/Counter.vue", "views/sum.blade.php"] {
        let tape = overlay_tape(&store, &first, path);
        let rows = store.file_documents_bulk(&[tape]).unwrap();
        let [row] = &rows[&tape][..] else {
            panic!("{path} derives one document: {rows:?}");
        };
        assert_eq!((row.ordinal, row.parent_ordinal), (1, 0), "{path}");
        assert_eq!(row.role_id, tape::DocumentRole::Generated.code(), "{path}");
        assert!(
            row.source.is_some(),
            "{path}: the derived tape is a row of its own"
        );
    }
    let python = overlay_tape(&store, &first, "probe.py");
    assert!(
        store.file_documents_bulk(&[python]).unwrap().is_empty(),
        "a single-language file has no document rows"
    );
    first.close();
    super::storage::index_stored_tapes(
        &store,
        &std::sync::atomic::AtomicBool::new(false),
        &mut None,
    );

    // A new catalog reuses no result: its clauses run over the stored documents.
    let code_rule = derived_documents_code_rule("derived-second");
    let restored = stored_workspace(&store, "/derived-second", &code_rule);
    let applied = apply_json(&restored, 1, files.clone());
    let mut unstored = CodeRuleDslWorkspace::new();
    unstored.add_code_rule(&code_rule, None).unwrap();
    let fresh = apply_json(&unstored, 1, files);
    unstored.close();
    for key in ["violations", "failures"] {
        assert_eq!(
            applied["evaluation"][key], fresh["evaluation"][key],
            "{key}"
        );
    }
    let witnesses = located(&applied["evaluation"]["violations"]);
    for (path, clause) in [
        ("src/Counter.vue", "vue-script"),
        ("views/sum.blade.php", "blade-view"),
    ] {
        assert!(
            witnesses
                .iter()
                .any(|(at, id, _)| at == path && id == clause),
            "{path}: {witnesses:?}"
        );
    }
    let state = restored.state.lock().unwrap();
    for path in ["src/Counter.vue", "views/sum.blade.php"] {
        let file = &state.files[path];
        assert!(
            !file.selector_skipped && (file.tape_cache_hit || file.tape_decompressed),
            "{path} read its stored documents"
        );
    }
    // A derived document with no witness root, by its own index, is not decoded at all.
    for path in ["src/Plain.vue", "views/plain.blade.php"] {
        assert!(state.files[path].selector_skipped, "{path}");
    }
    drop(state);
    restored.close();
}

/// A stored primary tape whose derived document's tape is gone is not stored: the file is parsed
/// again, never restored without the document it derives.
#[test]
fn a_stored_tape_missing_a_derived_tape_is_parsed_again() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let directory = TempDir::new("derived-missing");
    let store = StdArc::new(ContentStore::open(&directory.store()).unwrap());
    let files = json!([{ "path": "src/Counter.vue", "language": "vue", "source":
        "<script setup>\nconst count = ref(0)\n</script>\n<template>\n  <p>{{ count }}</p>\n</template>\n" }]);
    let first = stored_workspace(
        &store,
        "/derived-missing-first",
        &derived_documents_code_rule("derived-missing-first"),
    );
    let expected = located(&apply_json(&first, 1, files.clone())["evaluation"]["violations"]);
    assert_eq!(expected.len(), 1, "{expected:?}");
    let tape = overlay_tape(&store, &first, "src/Counter.vue");
    first.close();
    {
        // The store's own foreign keys refuse this: a raw connection stands for lost rows.
        let connection = rusqlite::Connection::open(directory.store()).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        let deleted = connection
            .execute(
                "DELETE FROM tape_objects WHERE tape_id IN (SELECT document_tape_id FROM file_documents)",
                [],
            )
            .unwrap();
        assert_eq!(deleted, 1);
    }
    let [document] = &store.file_documents_bulk(&[tape]).unwrap()[&tape][..] else {
        panic!("the row outlives its tape");
    };
    assert_eq!(document.source, None);

    let restored = stored_workspace(
        &store,
        "/derived-missing-second",
        &derived_documents_code_rule("derived-missing-second"),
    );
    let applied = apply_json(&restored, 1, files);
    assert_eq!(located(&applied["evaluation"]["violations"]), expected);
    {
        let state = restored.state.lock().unwrap();
        let file = &state.files["src/Counter.vue"];
        assert!(
            !file.tape_cache_hit && !file.tape_decompressed,
            "the file was parsed, not restored in part"
        );
    }
    restored.close();
}
