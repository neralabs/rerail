//! Cross-file checks over `evidence` relations (plan §7, E1–E3): incremental results equal a fresh
//! workspace's, eviction and restart change nothing, and incomplete evidence never proves absence.

use super::super::PolicyDslRuntime;
use super::api::{PolicyDslWorkspace, WorkspaceApplyTask};
use super::storage::StorageContext;
use super::tests::WORKSPACE_TEST_SLOT;
use super::*;
use crate::content_store::{ContentStore, GitTree, HashKey};
use crate::policy_dsl::CompiledPolicyDsl;
use napi::Task;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc as StdArc;
use std::time::{SystemTime, UNIX_EPOCH};

const CAPABILITIES: &str = r#"{
    "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
        "languages": { "python": "tree-sitter-python@0.23",
            "typescript": "tree-sitter-typescript@0.23", "php": "tree-sitter-php@0.24" } },
    "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
        "maxFiles": 1000, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
        "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
}"#;

fn compile(id: &str, checks: Value) -> CompiledPolicyDsl {
    PolicyDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": id, "revision": 1,
            "scope": { "include": ["**/*"], "exclude": [] },
            "checks": checks,
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}

fn finding(code: &str, message: &str) -> Value {
    json!({ "anchor": "witness.span", "code": code, "message": message, "severity": "error" })
}

/// E1: an exported component function without a story beside it named for it.
fn component_story_policy() -> CompiledPolicyDsl {
    compile(
        "e1-component-story",
        json!([{
            "id": "component-without-story",
            "finding": finding("E1", "component has no story"),
            "require_each": {
                "subject": { "evidence": {
                    "as": "component",
                    "paths": { "include": ["ui/**/*.tsx"], "exclude": ["**/*.stories.tsx"] },
                    "query": { "ast": { "language": "typescript", "rule": {
                        "pattern": "export function $NAME($$$PARAMS) { $$$BODY }" } } },
                    "project": { "name": "capture.NAME", "directory": "file.directory" }
                } },
                "evidence": { "evidence": {
                    "as": "story",
                    "paths": { "include": ["**/*.stories.tsx"] },
                    "query": { "ast": { "language": "typescript", "rule": {
                        "pattern": "export default $META" } } },
                    "project": { "name": "file.stem", "directory": "file.directory" },
                    "where": { "all": [
                        { "eq": ["component.name", "story.name"] },
                        { "eq": ["component.directory", "story.directory"] }
                    ] }
                } }
            }
        }]),
    )
}

/// E2: calls of `payments.charge`, however imported, only from modules `authorizations.py`
/// declares.
fn restricted_api_policy() -> CompiledPolicyDsl {
    compile(
        "e2-restricted-api",
        json!([{
            "id": "unauthorized-charge",
            "finding": finding("E2", "payments.charge called from an undeclared module"),
            "require_each": {
                "subject": { "evidence": {
                    "as": "call",
                    "query": { "ast": { "language": "python", "rule": {
                        "kind": "call",
                        "has": { "field": "function", "kind": "identifier",
                            "resolves": { "import": ["payments.charge"] } } } } },
                    "project": { "module": "file.module" }
                } },
                "evidence": { "evidence": {
                    "as": "grant",
                    "paths": { "include": ["authorizations.py"] },
                    "query": { "ast": { "language": "python", "rule": {
                        "kind": "string_content", "pattern": "$MODULE",
                        "inside": { "kind": "assignment", "stopBy": "end",
                            "has": { "field": "left", "regex": "^AUTHORIZED$" } } } } },
                    "project": { "module": "capture.MODULE" },
                    "where": { "eq": ["call.module", "grant.module"] }
                } }
            }
        }]),
    )
}

/// E3: at most one call of `legacy.fetch` per package.
fn usage_budget_policy() -> CompiledPolicyDsl {
    compile(
        "e3-usage-budget",
        json!([{
            "id": "legacy-fetch-budget",
            "finding": finding("E3", "too many legacy.fetch calls in this package"),
            "budget": {
                "query": { "evidence": {
                    "as": "usage",
                    "query": { "ast": { "language": "python", "rule": {
                        "kind": "call",
                        "has": { "field": "function", "kind": "identifier",
                            "resolves": { "import": ["legacy.fetch"] } } } } },
                    "project": { "package": "file.package", "path": "file.path" }
                } },
                "group_by": ["usage.package"],
                "count": { "lte": 1 }
            }
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
            "rerail-crossfile-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stored_workspace(
    store: &StdArc<ContentStore>,
    root: &str,
    policy: &CompiledPolicyDsl,
) -> PolicyDslWorkspace {
    let root_tree = HashKey::git_sha1([9; 20]);
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
    let owner = format!("crossfile-test:{root}:{}", std::process::id());
    let workspace_id = store
        .open_workspace(root, "repository", &owner, snapshot, 1, i64::MAX)
        .unwrap();
    let mut workspace = PolicyDslWorkspace::new();
    workspace.state.lock().unwrap().storage = Some(StorageContext {
        store: StdArc::clone(store),
        repository: None,
        repository_prefix: String::new(),
        workspace_id,
        snapshot_id: snapshot,
        owner,
        _maintenance: None,
    });
    workspace.add_policy(policy).unwrap();
    workspace
}

fn apply(
    workspace: &PolicyDslWorkspace,
    revision: u32,
    files: &[Value],
    removed: &[&str],
) -> Value {
    apply_response(workspace, revision, files, removed)["evaluation"].clone()
}

fn apply_response(
    workspace: &PolicyDslWorkspace,
    revision: u32,
    files: &[Value],
    removed: &[&str],
) -> Value {
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": revision, "files": files, "removed": removed })
            .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    serde_json::from_str::<Value>(&task.compute().unwrap()).unwrap()
}

/// The workspace's whole outcome after an apply, as a host keeps it: a delta result replaces the
/// findings and failures of its `replacedPaths` only.
fn merged(previous: &Outcome, response: &Value) -> Outcome {
    let current = outcome(&response["evaluation"]);
    let Some(replaced) = response["replacedPaths"].as_array() else {
        return current;
    };
    let replaced = replaced
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    Outcome {
        findings: previous
            .findings
            .iter()
            .filter(|finding| !replaced.contains(finding.0.as_str()))
            .cloned()
            .chain(current.findings)
            .collect(),
        failures: previous
            .failures
            .iter()
            .filter(|failure| !replaced.contains(failure.1.as_str()))
            .cloned()
            .chain(current.failures)
            .collect(),
    }
}

/// The cross-file outcome: findings as `(path, start byte, count)`, failures as `(code, path)`.
#[derive(Debug, PartialEq)]
struct Outcome {
    findings: BTreeSet<(String, u64, Option<u64>)>,
    failures: BTreeSet<(String, String)>,
}

fn outcome(evaluation: &Value) -> Outcome {
    Outcome {
        findings: evaluation["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| {
                (
                    finding["path"].as_str().unwrap_or_default().to_owned(),
                    finding["span"]["startByte"].as_u64().unwrap_or_default(),
                    finding["variables"]["count"].as_u64(),
                )
            })
            .collect(),
        failures: evaluation["failures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|failure| {
                (
                    failure["code"].as_str().unwrap().to_owned(),
                    failure["path"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect(),
    }
}

fn file(path: &str, source: &str) -> Value {
    let language = if path.ends_with(".py") {
        "python"
    } else {
        "typescript"
    };
    let mut file = json!({ "path": path, "language": language, "source": source });
    if let Some(stem) = path.strip_suffix(".py") {
        file["module"] = json!(stem.replace('/', ".").trim_end_matches(".__init__"));
    }
    file
}

/// One edit: files written, then paths removed.
struct Edit<'a> {
    files: Vec<Value>,
    removed: Vec<&'a str>,
}

/// Runs `edits` through a resident stored workspace, one evicted after every apply, and an
/// unstored one; after each edit, all three must equal a fresh workspace given the whole tree,
/// and that must equal `expected[i]`. At the end a restarted workspace on the same store agrees.
fn scenario(tag: &str, policy: &CompiledPolicyDsl, edits: Vec<Edit<'_>>, expected: &[Outcome]) {
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDir::new(tag);
    let store = StdArc::new(ContentStore::open(&directory.0.join("content.sqlite")).unwrap());
    let resident = stored_workspace(&store, &format!("/{tag}-resident"), policy);
    let evicted = stored_workspace(&store, &format!("/{tag}-evicted"), policy);
    let mut unstored = PolicyDslWorkspace::new();
    unstored.add_policy(policy).unwrap();
    let mut tree = BTreeMap::<String, Value>::new();
    let mut kept = [(); 3].map(|()| Outcome {
        findings: BTreeSet::new(),
        failures: BTreeSet::new(),
    });
    for (index, edit) in edits.iter().enumerate() {
        let revision = index as u32 + 1;
        for file in &edit.files {
            tree.insert(file["path"].as_str().unwrap().to_owned(), file.clone());
        }
        for path in &edit.removed {
            tree.remove(*path);
        }
        let mut fresh = PolicyDslWorkspace::new();
        fresh.add_policy(policy).unwrap();
        let whole = tree.values().cloned().collect::<Vec<_>>();
        let full = outcome(&apply(&fresh, 1, &whole, &[]));
        fresh.close();
        assert_eq!(
            full, expected[index],
            "{tag} revision {revision}: fresh workspace"
        );
        let snapshot = json!({ "format": "rerail.repository-snapshot/v1", "revision": 0,
            "facts": [], "files": whole });
        let stateless: Value = serde_json::from_str(
            &policy
                .evaluate(snapshot.to_string(), "diagnostic".into())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            outcome(&stateless),
            full,
            "{tag} revision {revision}: stateless"
        );
        for (slot, (name, workspace)) in [
            ("resident", &resident),
            ("evicted", &evicted),
            ("unstored", &unstored),
        ]
        .into_iter()
        .enumerate()
        {
            let response = apply_response(workspace, revision, &edit.files, &edit.removed);
            kept[slot] = merged(&kept[slot], &response);
            assert_eq!(kept[slot], full, "{tag} revision {revision}: {name}");
        }
        assert!(
            evicted.evict_inactive().unwrap(),
            "{tag}: cross-file checks allow eviction"
        );
    }
    resident.close();
    evicted.close();
    unstored.close();
    let restarted = stored_workspace(&store, &format!("/{tag}-restarted"), policy);
    let whole = tree.values().cloned().collect::<Vec<_>>();
    assert_eq!(
        outcome(&apply(&restarted, 1, &whole, &[])),
        expected[edits.len() - 1],
        "{tag}: restarted workspace"
    );
    restarted.close();
}

fn found(items: &[(&str, u64, Option<u64>)], failures: &[(&str, &str)]) -> Outcome {
    Outcome {
        findings: items
            .iter()
            .map(|(path, start, count)| ((*path).to_owned(), *start, *count))
            .collect(),
        failures: failures
            .iter()
            .map(|(code, path)| ((*code).to_owned(), (*path).to_owned()))
            .collect(),
    }
}

const COMPONENT: &str = "export function Button() { return null; }\n";
const CARD: &str = "export function Card() { return null; }\n";
const STORY: &str = "export default { title: \"story\" };\n";

#[test]
fn e1_components_without_stories_follow_story_edits_in_unchanged_files() {
    scenario(
        "e1",
        &component_story_policy(),
        vec![
            Edit {
                files: vec![
                    file("ui/Button.tsx", COMPONENT),
                    file("ui/Card.tsx", CARD),
                    file("ui/Button.stories.tsx", STORY),
                    // A story in another directory is no story of `ui/Card`.
                    file("other/Card.stories.tsx", STORY),
                ],
                removed: vec![],
            },
            // Adding a story retracts the finding in the unchanged component file.
            Edit {
                files: vec![file("ui/Card.stories.tsx", STORY)],
                removed: vec![],
            },
            // Removing one adds a finding in its unchanged component file.
            Edit {
                files: vec![],
                removed: vec!["ui/Button.stories.tsx"],
            },
            // Renaming a story away from its component.
            Edit {
                files: vec![file("ui/Kard.stories.tsx", STORY)],
                removed: vec!["ui/Card.stories.tsx"],
            },
            // An undecidable story file: components without a story are unknown, never missing.
            Edit {
                files: vec![file("ui/Broken.stories.tsx", "export default {\n")],
                removed: vec![],
            },
            Edit {
                files: vec![file("ui/Broken.stories.tsx", STORY)],
                removed: vec![],
            },
        ],
        &[
            found(&[("ui/Card.tsx", 0, None)], &[]),
            found(&[], &[]),
            found(&[("ui/Button.tsx", 0, None)], &[]),
            found(&[("ui/Button.tsx", 0, None), ("ui/Card.tsx", 0, None)], &[]),
            found(
                &[],
                &[
                    ("source-incomplete", "ui/Button.tsx"),
                    ("source-incomplete", "ui/Card.tsx"),
                ],
            ),
            found(&[("ui/Button.tsx", 0, None), ("ui/Card.tsx", 0, None)], &[]),
        ],
    );
}

/// A cross-file check whose evaluation finished is kept, however late it finished: only a
/// deadline some step observed overflows it.
#[test]
fn a_global_evaluation_that_finishes_past_an_unobserved_deadline_is_kept() {
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut workspace = PolicyDslWorkspace::new();
    workspace.add_policy(&component_story_policy()).unwrap();
    TEST_DEADLINE_PASSES_AT.set(Some("global-done"));
    let evaluation = apply(&workspace, 1, &[file("ui/Card.tsx", CARD)], &[]);
    TEST_DEADLINE_PASSES_AT.set(None);
    workspace.close();
    assert_eq!(
        outcome(&evaluation),
        found(&[("ui/Card.tsx", 0, None)], &[])
    );
}

const AUTHORIZED_BILLING: &str = "AUTHORIZED = [\"billing.api\"]\n";
const AUTHORIZED_BOTH: &str = "AUTHORIZED = [\"billing.api\", \"shop.cart\"]\n";
const ALIASED_CALL: &str = "from payments import charge as pay\npay()\n";
const CALL: &str = "from payments import charge\ncharge()\n";
const SHADOWED: &str = "def charge():\n    pass\ncharge()\n";

#[test]
fn e2_restricted_calls_follow_the_declaration_and_the_callers_module() {
    let payments = file("payments/__init__.py", "def charge():\n    pass\n");
    scenario(
        "e2",
        &restricted_api_policy(),
        vec![
            Edit {
                files: vec![
                    payments.clone(),
                    file("authorizations.py", AUTHORIZED_BILLING),
                    file("billing/api.py", ALIASED_CALL),
                    file("shop/cart.py", CALL),
                    file("shop/local.py", SHADOWED),
                ],
                removed: vec![],
            },
            // Editing only the declaration changes findings in the unchanged caller.
            Edit {
                files: vec![file("authorizations.py", AUTHORIZED_BOTH)],
                removed: vec![],
            },
            // Moving the caller to another module.
            Edit {
                files: vec![file("billing/cart.py", CALL)],
                removed: vec!["shop/cart.py"],
            },
            // An aliased call from an undeclared module.
            Edit {
                files: vec![file("shop/pay.py", ALIASED_CALL)],
                removed: vec![],
            },
        ],
        &[
            found(&[("shop/cart.py", 28, None)], &[]),
            found(&[], &[]),
            found(&[("billing/cart.py", 28, None)], &[]),
            found(
                &[("billing/cart.py", 28, None), ("shop/pay.py", 35, None)],
                &[],
            ),
        ],
    );
}

const FETCH: &str = "from legacy import fetch\nfetch()\n";

#[test]
fn e3_usage_budgets_count_per_package_and_never_call_a_partial_count_clean() {
    scenario(
        "e3",
        &usage_budget_policy(),
        vec![
            Edit {
                files: vec![
                    file("legacy.py", "def fetch():\n    pass\n"),
                    file("a/__init__.py", ""),
                    file("a/x.py", FETCH),
                    file("a/y.py", FETCH),
                    file("b/__init__.py", ""),
                    file("b/z.py", FETCH),
                ],
                removed: vec![],
            },
            // Deleting one usage brings the package within budget.
            Edit {
                files: vec![],
                removed: vec!["a/y.py"],
            },
            // A second usage in `b`.
            Edit {
                files: vec![file("b/w.py", FETCH)],
                removed: vec![],
            },
            // An unreadable file: `b` over budget is still a finding (its count is a lower
            // bound); `a` within budget is unknown, not clean.
            Edit {
                files: vec![file("c/broken.py", "def (\n")],
                removed: vec![],
            },
        ],
        &[
            found(&[("a/x.py", 25, Some(2))], &[]),
            found(&[], &[]),
            found(&[("b/w.py", 25, Some(2))], &[]),
            found(
                &[("b/w.py", 25, Some(2))],
                &[("source-incomplete", "a/x.py")],
            ),
        ],
    );
}

/// E4, E5: `calls` over the workspace call graph. The consumer rule is the ticket's: an async
/// function's un-awaited call of a sync function that reaches blocking I/O through sync functions.
fn calls_policy(id: &str, rule: Value) -> CompiledPolicyDsl {
    compile(
        id,
        json!([{
            "id": id,
            "finding": finding("E4", "an async function reaches blocking I/O"),
            "forbid": { "ast": { "language": "python", "rule": rule } }
        }]),
    )
}

/// What a sync function reaching `time.sleep` looks like to a `calls` term.
fn reaches_sleep() -> Value {
    json!({
        "through": "sync",
        "has": {
            "stopBy": { "kind": "function_definition" },
            "kind": "call",
            "has": { "field": "function", "resolves": { "import": ["time.sleep"] } }
        }
    })
}

fn async_blocking_policy() -> CompiledPolicyDsl {
    calls_policy(
        "e4-async-blocking",
        json!({
            "kind": "function_definition",
            "regex": "^async ",
            "has": {
                "stopBy": { "kind": "function_definition" },
                "kind": "call",
                "not": { "inside": { "kind": "await" } },
                "calls": reaches_sleep()
            }
        }),
    )
}

/// A Python file with the module a package walk gives it (`pkg/__init__.py` is `pkg.__init__`).
fn module_file(path: &str, source: &str) -> Value {
    let module = path.strip_suffix(".py").unwrap().replace('/', ".");
    json!({ "path": path, "language": "python", "source": source, "module": module })
}

const API: &str = "from app.services import fetch\n\nasync def handler():\n    fetch()\n";
const SERVICES: &str = "from .impl import fetch\n";
const IMPL: &str = "from app.util import pause\n\ndef fetch():\n    pause()\n";
const SLEEPS: &str = "import time\n\ndef pause():\n    time.sleep(1)\n";
const IDLE: &str = "import time\n\ndef pause():\n    pass\n";
const STARRED: &str = "from app.star import *\n\ndef pause():\n    mystery()\n";
const SECOND: &str = "from app.services.impl import fetch as get\n\nasync def poll():\n    get()\n";

#[test]
fn e4_calls_follow_a_chain_through_unchanged_files() {
    scenario(
        "e4",
        &async_blocking_policy(),
        vec![
            Edit {
                files: vec![
                    module_file("app/__init__.py", ""),
                    module_file("app/api.py", API),
                    module_file("app/services/__init__.py", SERVICES),
                    module_file("app/services/impl.py", IMPL),
                    module_file("app/util.py", SLEEPS),
                ],
                removed: vec![],
            },
            // The sink goes, three files away: the finding in unchanged `api.py` goes too.
            Edit {
                files: vec![module_file("app/util.py", IDLE)],
                removed: vec![],
            },
            // And comes back, with a second caller through another import of the chain.
            Edit {
                files: vec![
                    module_file("app/util.py", SLEEPS),
                    module_file("app/poll.py", SECOND),
                ],
                removed: vec![],
            },
            // A name only a star import may bind: whether the chain reaches is unknown, never a
            // finding.
            Edit {
                files: vec![module_file("app/util.py", STARRED)],
                removed: vec![],
            },
            // The middle file removed: its import now names nothing in the workspace.
            Edit {
                files: vec![module_file("app/util.py", SLEEPS)],
                removed: vec!["app/services/impl.py"],
            },
        ],
        &[
            found(&[("app/api.py", 32, None)], &[]),
            found(&[], &[]),
            found(&[("app/api.py", 32, None), ("app/poll.py", 44, None)], &[]),
            found(&[], &[]),
            found(&[], &[]),
        ],
    );
}

/// An edit that moves the sink's definition without changing what it calls relinks nothing; the
/// closure computed after a later, linking edit still finds the sink where it now starts.
#[test]
fn e4_a_definition_moved_by_a_comment_keeps_its_seed() {
    scenario(
        "e4-moved",
        &async_blocking_policy(),
        vec![
            Edit {
                files: vec![
                    module_file("app/__init__.py", ""),
                    module_file("app/api.py", API),
                    module_file("app/services/__init__.py", SERVICES),
                    module_file("app/services/impl.py", IMPL),
                    module_file("app/util.py", SLEEPS),
                ],
                removed: vec![],
            },
            Edit {
                files: vec![module_file(
                    "app/util.py",
                    &format!("# the sink, moved down by this comment\n{SLEEPS}"),
                )],
                removed: vec![],
            },
            Edit {
                files: vec![module_file("app/poll.py", SECOND)],
                removed: vec![],
            },
        ],
        &[
            found(&[("app/api.py", 32, None)], &[]),
            found(&[("app/api.py", 32, None)], &[]),
            found(&[("app/api.py", 32, None), ("app/poll.py", 44, None)], &[]),
        ],
    );
}

/// When a sink goes three files away, consumers of unchanged `api.py` go quiet and its results
/// move to a new stamp. Read after eviction, with no apply to rebuild the call graph first, the
/// results still answer: whether every consumer went quiet or only some, and whether `api.py` was
/// resident or evicted when they did.
#[test]
fn e4_results_of_consumers_gone_quiet_are_readable_after_eviction() {
    const BOTH: &str = "import time\n\ndef pause():\n    time.sleep(1)\n    time.monotonic()\n";
    const CLOCK: &str = "import time\n\ndef pause():\n    time.monotonic()\n";
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDir::new("e4-quiet-eviction");
    let store = StdArc::new(ContentStore::open(&directory.0.join("content.sqlite")).unwrap());
    // Two consumers: an async function's un-awaited call reaching `time.sleep`, and one reaching
    // `time.monotonic`.
    let consumer = |id: &str, import: &str| {
        json!({
            "id": id,
            "finding": finding(id, "an async function reaches a sink"),
            "forbid": { "ast": { "language": "python", "rule": {
                "kind": "function_definition",
                "regex": "^async ",
                "has": {
                    "stopBy": { "kind": "function_definition" },
                    "kind": "call",
                    "not": { "inside": { "kind": "await" } },
                    "calls": { "through": "sync", "has": {
                        "stopBy": { "kind": "function_definition" },
                        "kind": "call",
                        "has": { "field": "function", "resolves": { "import": [import] } }
                    } }
                }
            } } }
        })
    };
    let policy = compile(
        "e4-quiet-eviction",
        json!([
            consumer("sleeps", "time.sleep"),
            consumer("clocks", "time.monotonic")
        ]),
    );
    let tree = [
        module_file("app/__init__.py", ""),
        module_file("app/api.py", API),
        module_file("app/services/__init__.py", SERVICES),
        module_file("app/services/impl.py", IMPL),
        module_file("app/util.py", BOTH),
    ];
    // Both sinks go: every consumer of `api.py` is quiet. Only `time.sleep` goes: one is.
    let reported: &[(&str, u64, Option<u64>)] = &[("app/api.py", 32, None)];
    for (edit, util, expected) in [("idle", IDLE, &[][..]), ("clock", CLOCK, reported)] {
        for evicted_before in [false, true] {
            let case = format!("{edit}, api.py evicted before the edit: {evicted_before}");
            let workspace = stored_workspace(
                &store,
                &format!("/e4-quiet-eviction-{edit}-{evicted_before}"),
                &policy,
            );
            assert_eq!(
                outcome(&apply(&workspace, 1, &tree, &[])),
                found(reported, &[]),
                "{case}"
            );
            if evicted_before {
                assert!(workspace.evict_inactive().unwrap(), "{case}");
            }
            let applied = outcome(&apply(
                &workspace,
                2,
                &[module_file("app/util.py", util)],
                &[],
            ));
            assert_eq!(applied, found(expected, &[]), "{case}: apply");
            assert!(workspace.evict_inactive().unwrap(), "{case}");
            let page = super::tests::result_page(&workspace, 0, 64 * 1024 * 1024)
                .map(|page| outcome(&serde_json::from_str(&page).unwrap()));
            assert_eq!(
                page.map_err(|error| error.reason),
                Ok(found(expected, &[])),
                "{case}: read after eviction"
            );
            workspace.close();
        }
    }
}

/// E5: "must reach". An async handler none of whose calls reaches `time.sleep` is a finding; an
/// open chain is unknown, so never one.
#[test]
fn e5_absence_needs_a_closed_chain() {
    let policy = calls_policy(
        "e5-must-reach",
        json!({
            "kind": "function_definition",
            "regex": "^async def handler",
            "not": { "has": {
                "stopBy": { "kind": "function_definition" },
                "kind": "call",
                "calls": reaches_sleep()
            } }
        }),
    );
    scenario(
        "e5",
        &policy,
        vec![
            Edit {
                files: vec![
                    module_file("app/__init__.py", ""),
                    module_file("app/api.py", API),
                    module_file("app/services/__init__.py", SERVICES),
                    module_file("app/services/impl.py", IMPL),
                    module_file("app/util.py", SLEEPS),
                ],
                removed: vec![],
            },
            Edit {
                files: vec![module_file("app/util.py", IDLE)],
                removed: vec![],
            },
            Edit {
                files: vec![module_file("app/util.py", STARRED)],
                removed: vec![],
            },
        ],
        &[
            found(&[], &[]),
            found(&[("app/api.py", 32, None)], &[]),
            found(&[], &[]),
        ],
    );
}

/// E6: summaries are stored with the file's results. A second workspace over the same sources
/// (a restart, a copy) restores every summary instead of parsing for it, and its closure and
/// findings are equal.
#[test]
fn e6_stored_summaries_spare_a_restart_the_parse() {
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDir::new("e6");
    let store = StdArc::new(ContentStore::open(&directory.0.join("content.sqlite")).unwrap());
    let policy = async_blocking_policy();
    let files = vec![
        module_file("app/__init__.py", ""),
        module_file("app/api.py", API),
        module_file("app/services/__init__.py", SERVICES),
        module_file("app/services/impl.py", IMPL),
        module_file("app/util.py", SLEEPS),
    ];
    let extracted = || calls_state::SUMMARIES_EXTRACTED.load(Ordering::Relaxed);
    let first = stored_workspace(&store, "/e6-first", &policy);
    let before = extracted();
    let expected = outcome(&apply(&first, 1, &files, &[]));
    assert_eq!(expected, found(&[("app/api.py", 32, None)], &[]));
    assert_eq!(
        extracted() - before,
        files.len(),
        "the first workspace extracts every summary"
    );
    first.close();
    let second = stored_workspace(&store, "/e6-second", &policy);
    let before = extracted();
    assert_eq!(outcome(&apply(&second, 1, &files, &[])), expected);
    assert_eq!(extracted() - before, 0, "the second restores every summary");
    // An edit extracts the edited file's summary only.
    let before = extracted();
    let edited = outcome(&apply(&second, 2, &[module_file("app/util.py", IDLE)], &[]));
    assert_eq!(edited.findings, BTreeSet::new());
    assert_eq!(extracted() - before, 1);
    second.close();
}

/// Workspaces open on the same code hold one summary per file version, however they got it
/// (extracted, or restored from the store); an edit gives only the edited file its own.
#[test]
fn e7_open_workspaces_share_one_summary_per_file_version() {
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDir::new("e7");
    let store = StdArc::new(ContentStore::open(&directory.0.join("content.sqlite")).unwrap());
    let policy = async_blocking_policy();
    let files = vec![
        module_file("app/__init__.py", ""),
        module_file("app/api.py", API),
        module_file("app/services/__init__.py", SERVICES),
        module_file("app/services/impl.py", IMPL),
        module_file("app/util.py", SLEEPS),
    ];
    let first = stored_workspace(&store, "/e7-first", &policy);
    let second = stored_workspace(&store, "/e7-second", &policy);
    let expected = outcome(&apply(&first, 1, &files, &[]));
    assert_eq!(outcome(&apply(&second, 1, &files, &[])), expected);
    let shared = |path: &str| {
        let first = crossfile::CrossFileScheduler::summary(&first.state.lock().unwrap(), path);
        let second = crossfile::CrossFileScheduler::summary(&second.state.lock().unwrap(), path);
        StdArc::ptr_eq(&first.unwrap(), &second.unwrap())
    };
    for file in &files {
        assert!(shared(file["path"].as_str().unwrap()), "{}", file["path"]);
    }
    let edited = outcome(&apply(&second, 2, &[module_file("app/util.py", IDLE)], &[]));
    assert_eq!(edited.findings, BTreeSet::new());
    assert!(!shared("app/util.py"));
    assert!(shared("app/api.py"));
    first.close();
    second.close();
}

/// Two consumers in one policy, each silent where the other reaches: a file evaluates only the
/// consumer that may match there and answers the other, resident, evicted and restarted alike.
#[test]
fn e8_a_silent_consumer_is_answered_beside_one_that_reaches() {
    let reaches = |sink: &str| {
        json!({
            "has": {
                "stopBy": { "kind": "function_definition" },
                "kind": "call",
                "has": { "field": "function", "resolves": { "import": [sink] } }
            }
        })
    };
    let mut sleeps = reaches("time.sleep");
    sleeps["through"] = json!("sync");
    let policy = compile(
        "e8-two-consumers",
        json!([
            {
                "id": "async-reaches-sleep",
                "finding": finding("E8A", "reaches sleep"),
                "forbid": { "ast": { "language": "python", "rule": {
                    "kind": "call",
                    "inside": { "kind": "function_definition", "regex": "^async ",
                        "stopBy": { "kind": "function_definition" } },
                    "calls": sleeps
                } } }
            },
            {
                "id": "reaches-network",
                "finding": finding("E8B", "reaches the network"),
                "forbid": { "ast": { "language": "python", "rule": {
                    "kind": "call", "calls": reaches("requests.get")
                } } }
            }
        ]),
    );
    const NET: &str = "import requests\n\ndef fetch_remote():\n    requests.get('x')\n";
    const OFFLINE: &str = "def fetch_remote():\n    pass\n";
    const RUNNER: &str = "from app.net import fetch_remote\n\ndef run():\n    fetch_remote()\n";
    scenario(
        "e8",
        &policy,
        vec![
            Edit {
                files: vec![
                    module_file("app/__init__.py", ""),
                    module_file("app/api.py", API),
                    module_file("app/services/__init__.py", SERVICES),
                    module_file("app/services/impl.py", IMPL),
                    module_file("app/util.py", SLEEPS),
                    module_file("app/net.py", NET),
                    module_file("app/runner.py", RUNNER),
                ],
                removed: vec![],
            },
            Edit {
                files: vec![module_file("app/util.py", IDLE)],
                removed: vec![],
            },
            Edit {
                files: vec![module_file("app/net.py", OFFLINE)],
                removed: vec![],
            },
            Edit {
                files: vec![
                    module_file("app/util.py", SLEEPS),
                    module_file("app/net.py", NET),
                ],
                removed: vec![],
            },
        ],
        &[
            found(
                &[("app/api.py", 57, None), ("app/runner.py", 49, None)],
                &[],
            ),
            found(&[("app/runner.py", 49, None)], &[]),
            found(&[], &[]),
            found(
                &[("app/api.py", 57, None), ("app/runner.py", 49, None)],
                &[],
            ),
        ],
    );
}

fn php_file(path: &str, source: &str) -> Value {
    json!({ "path": path, "language": "php", "source": source })
}

const CONTROLLER: &str = "<?php\nnamespace App\\Http;\n\nuse App\\Services\\Fetcher;\n\nfinal class Controller\n{\n    public function handle(callable $next): void\n    {\n        (new Fetcher())->fetch();\n        pause_all();\n        $next();\n    }\n}\n";
const FETCHER: &str = "<?php\nnamespace App\\Services;\n\nclass Fetcher extends Base\n{\n    public function fetch(): void\n    {\n        $this->pause();\n    }\n}\n";
const BASE_SLEEPS: &str = "<?php\nnamespace App\\Services;\n\nabstract class Base\n{\n    protected function pause(): void\n    {\n        sleep(1);\n    }\n}\n";
const BASE_IDLE: &str = "<?php\nnamespace App\\Services;\n\nabstract class Base\n{\n    protected function pause(): void\n    {\n    }\n}\n";
const SLOW: &str = "<?php\nnamespace App\\Services;\n\nfinal class Slow extends Fetcher\n{\n    protected function pause(): void\n    {\n        usleep(1);\n    }\n}\n";
const HELPERS: &str = "<?php\n\nfunction pause_all(): void\n{\n    usleep(1);\n}\n";

/// E9: PHP. A call reaches a sleep through `new`, a late-bound `$this->pause()` (the parent's
/// method, then an override a new subclass declares), and an unqualified call the workspace's
/// global function answers; `$next()` of a parameter is unknown, never a finding.
#[test]
fn e9_php_calls_follow_methods_overrides_and_fallbacks_across_files() {
    let policy = compile(
        "e9-php-sleeps",
        json!([{
            "id": "reaches-sleep",
            "finding": finding("E9", "the call reaches sleep"),
            "forbid": { "ast": { "language": "php", "rule": {
                "any": [{ "kind": "member_call_expression" }, { "kind": "function_call_expression" }],
                "inside": { "kind": "method_declaration", "has": { "field": "name", "regex": "^handle$" },
                    "stopBy": "end" },
                "calls": { "has": { "stopBy": "end", "kind": "function_call_expression",
                    "has": { "field": "function", "regex": "^u?sleep$" } } }
            } } }
        }]),
    );
    let at = |needle: &str| CONTROLLER.find(needle).unwrap() as u64;
    let (fetch, pause_all) = (at("(new Fetcher())->fetch()"), at("pause_all()"));
    let controller = "app/Http/Controller.php";
    scenario(
        "e9",
        &policy,
        vec![
            Edit {
                files: vec![
                    php_file(controller, CONTROLLER),
                    php_file("app/Services/Fetcher.php", FETCHER),
                    php_file("app/Services/Base.php", BASE_SLEEPS),
                    php_file("app/helpers.php", HELPERS),
                ],
                removed: vec![],
            },
            // The parent's sleep goes, two files away: `fetch()` reaches nothing.
            Edit {
                files: vec![php_file("app/Services/Base.php", BASE_IDLE)],
                removed: vec![],
            },
            // A subclass overrides the late-bound `$this->pause()` with a sleep.
            Edit {
                files: vec![php_file("app/Services/Slow.php", SLOW)],
                removed: vec![],
            },
            // The global function goes: `pause_all()` names nothing in the workspace.
            Edit {
                files: vec![],
                removed: vec!["app/helpers.php"],
            },
        ],
        &[
            found(
                &[(controller, fetch, None), (controller, pause_all, None)],
                &[],
            ),
            found(&[(controller, pause_all, None)], &[]),
            found(
                &[(controller, fetch, None), (controller, pause_all, None)],
                &[],
            ),
            found(&[(controller, fetch, None)], &[]),
        ],
    );
}
