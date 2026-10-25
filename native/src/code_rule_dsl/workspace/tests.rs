use super::super::CodeRuleDslRuntime;
use super::super::compile::compile_scope;
use super::api::{CodeRuleDslWorkspace, StateLease, WorkspaceApplyTask};
use super::ingest::{
    BINARY_REQUEST_MAGIC, Request, WireFile, apply_request, decode_buffer_request, make_file,
};
use super::persisted::{decode_local_result, encode_local_result, grammar_version};
use super::results::{
    RESULT_PAGE_EVICTED_RESTORES, RESULT_PAGE_SORTS, RESULT_PAGE_WITNESSES_VISITED,
};
use super::storage::{CONTENT_STORES, StorageContext, index_stored_tapes, shared_content_store};
use super::*;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::content_store::{ContentStore, GitTree, HashKey};
use napi::Task;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::Arc as StdArc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Serializes the tests that share process-wide state. A parking_lot mutex never poisons, so a
/// failing test cannot fail every test that takes the slot after it.
pub(super) static WORKSPACE_TEST_SLOT: Mutex<()> = Mutex::new(());

#[cfg(test)]
/// Build a Git fixture repository. Tests only: production discovery reads Git through gix.
pub(super) fn git(root: &std::path::Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", root)
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?} failed");
}

#[test]
fn content_store_registry_shares_one_process_store() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rerail-shared-content-store-{}-{nonce}.sqlite",
        std::process::id()
    ));
    let first = shared_content_store(path.clone()).unwrap();
    let second = shared_content_store(path.clone()).unwrap();
    assert!(StdArc::ptr_eq(&first.0, &second.0));
    assert!(StdArc::ptr_eq(&first.1, &second.1));
    drop(first);
    drop(second);
    CONTENT_STORES.lock().unwrap().remove(&path);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn local_result_round_trips_negative_results_and_failures() {
    let negative = encode_local_result(&[], None, None).unwrap();
    let (witnesses, failure, syntax) = decode_local_result(&negative, "src/a.py").unwrap();
    assert!(witnesses.is_empty());
    assert!(failure.is_none());
    assert!(syntax.is_none());

    for failure in [
        ("budget-exceeded", "file line index could not be built"),
        ("adapter-error", "file tape encoding failed"),
        ("source-incomplete", "file has syntax errors"),
    ] {
        let encoded = encode_local_result(&[], Some(failure), None).unwrap();
        let (witnesses, restored_failure, syntax) =
            decode_local_result(&encoded, "src/a.py").unwrap();
        assert!(witnesses.is_empty());
        assert_eq!(restored_failure, Some(failure));
        assert!(syntax.is_none());
    }

    // A file the parser did not read keeps its static failure and the detail it reports.
    let detail = "file did not parse: error regions cover 95% of it, first at line 2, column 1";
    let encoded = encode_local_result(
        &[],
        Some(("source-incomplete", "file did not parse")),
        Some(detail),
    )
    .unwrap();
    let (_, restored_failure, syntax) = decode_local_result(&encoded, "src/a.js").unwrap();
    assert_eq!(
        restored_failure,
        Some(("source-incomplete", "file did not parse"))
    );
    assert_eq!(syntax.as_deref(), Some(detail));
}

#[test]
fn a_stored_local_result_decodes_to_typed_witnesses_and_re_encodes_to_its_bytes() {
    // A version 3 frame as engines before the typed witness span and captures wrote it: spans and
    // captures were JSON objects, serialized in key order.
    let span = |start: usize, end: usize, line: usize, column: usize| {
        format!(
            r#"{{"endByte":{end},"endColumn":{},"endLine":{line},"startByte":{start},"startColumn":{column},"startLine":{line}}}"#,
            column + end - start
        )
    };
    let stored = format!(
        concat!(
            r#"{{"failure":null,"syntax":null,"version":3,"witnesses":["#,
            r#"{{"captures":{{"$project":{{"package":"pkg"}},"A":[{a}],"ARGS":[{first},{last}]}},"#,
            r#""fix":{{"span":{whole},"text":"log(1, 2)"}},"message":"print 1","span":{whole}}},"#,
            r#"{{"captures":{{}},"fix":null,"message":null,"span":{other}}}]}}"#,
        ),
        a = span(14, 19, 2, 4),
        first = span(20, 21, 2, 10),
        last = span(23, 24, 2, 13),
        whole = span(14, 25, 2, 4),
        other = span(0, 6, 0, 0),
    );

    let (witnesses, failure, syntax) = decode_local_result(stored.as_bytes(), "pkg/a.py").unwrap();
    assert!(failure.is_none() && syntax.is_none());
    let [printed, plain] = witnesses.as_slice() else {
        panic!("two witnesses");
    };
    let expected =
        |start_byte, end_byte, start_column, end_column| crate::code_rule_dsl::violations::Span {
            start_byte,
            end_byte,
            start_line: 2,
            end_line: 2,
            start_column,
            end_column,
        };
    assert_eq!(printed.path, "pkg/a.py");
    assert_eq!(printed.span, expected(14, 25, 4, 15));
    assert_eq!(
        printed.captures.get("A"),
        Some(&[expected(14, 19, 4, 9)][..])
    );
    assert_eq!(
        printed.captures.get("ARGS"),
        Some(&[expected(20, 21, 10, 11), expected(23, 24, 13, 14)][..])
    );
    assert_eq!(printed.captures.get("B"), None);
    assert_eq!(
        printed
            .captures
            .projection()
            .map(crate::code_rule_dsl::violations::projection::Row::json),
        Some(json!({ "package": "pkg" }))
    );
    assert_eq!(printed.message.as_deref(), Some("print 1"));
    assert_eq!(printed.fix.as_ref().unwrap()["text"], "log(1, 2)");
    assert_eq!(plain.captures, Default::default());
    assert_eq!(plain.span.end_byte, 6);

    let rewritten = encode_local_result(&witnesses, None, None).unwrap();
    assert_eq!(String::from_utf8(rewritten).unwrap(), stored);
}

#[test]
fn close_releases_idle_workspace_files() {
    let workspace = CodeRuleDslWorkspace::new();
    {
        let mut state = workspace.state.lock().unwrap();
        let input = FileInput {
            path: "example.py".into(),
            language: "python".into(),
            source: Arc::new("print('secret')\n".into()),
            module: None,
            package: None,
        };
        let file = make_file(input, 1, &[], &mut state.candidate_sets);
        state.files.insert(file.input.path.clone(), file);
    }
    let memory: Value = serde_json::from_str(&workspace.memory_profile().unwrap()).unwrap();
    assert_eq!(memory["files"], 1);
    assert!(memory["fileStateBytesEstimate"].as_u64().unwrap() > 0);
    workspace.scopes.lock().unwrap().push(
        compile_scope(Some(&json!({
            "include": ["**/*"], "exclude": []
        })))
        .unwrap(),
    );
    assert!(workspace.accepts_path("example.py".into()));
    workspace.close();
    assert!(workspace.state.lock().unwrap().files.is_empty());
    assert!(workspace.scopes.lock().unwrap().is_empty());
    assert!(!workspace.accepts_path("example.py".into()));
}

#[test]
fn close_clears_poisoned_scope_metadata() {
    let workspace = Arc::new(CodeRuleDslWorkspace::new());
    let scope = compile_scope(Some(&json!({
        "include": ["**/*"], "exclude": []
    })))
    .unwrap();
    workspace.scopes.lock().unwrap().push(scope);
    let owner = Arc::clone(&workspace);
    assert!(
        std::thread::spawn(move || {
            let _scopes = owner.scopes.lock().unwrap();
            panic!("poison scope metadata");
        })
        .join()
        .is_err()
    );

    workspace.close();
    assert!(
        workspace
            .scopes
            .lock()
            .err()
            .expect("poisoned scopes")
            .into_inner()
            .is_empty()
    );
    assert!(!workspace.accepts_path("example.py".into()));
}

#[test]
fn poisoned_state_rejections_and_close_release_resident_sources() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let compiled = CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "poisoned-workspace", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{
                "id": "probe", "violation": { "anchor": "witness.span", "code": "PROBE",
                    "message": "probe" },
                "forbid": { "regex": { "pattern": "probe", "source": "file.content" } }
            }]
        })
        .to_string(),
        "json".into(),
        json!({
            "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
                "languages": { "python": "tree-sitter-python@0.23" } },
            "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
                "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
                "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
        })
        .to_string(),
    )
    .unwrap();
    let request = json!({ "revision": 1, "files": [], "removed": [] }).to_string();
    for _ in 0..128 {
        let mut workspace = CodeRuleDslWorkspace::new();
        let source = Arc::new(String::from("poisoned resident source"));
        let retained = Arc::downgrade(&source);
        {
            let mut state = workspace.state.lock().unwrap();
            let file = make_file(
                FileInput {
                    path: "resident.py".into(),
                    language: "python".into(),
                    source: Arc::clone(&source),
                    module: None,
                    package: None,
                },
                1,
                &[],
                &mut state.candidate_sets,
            );
            state.files.insert(file.input.path.clone(), file);
        }
        drop(source);
        let state = Arc::clone(&workspace.state);
        assert!(
            std::thread::spawn(move || {
                let _guard = state.lock().unwrap();
                panic!("poison workspace state");
            })
            .join()
            .is_err()
        );

        assert_eq!(
            workspace.add_code_rule(&compiled, None).unwrap_err().reason,
            "policy workspace is evaluating"
        );
        assert!(workspace.try_apply(request.clone()).unwrap().is_none());
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: request.clone(),
            request_buffer: None,
            request_direct: None,
        };
        assert_eq!(
            task.compute().unwrap_err().reason,
            "policy workspace is unavailable"
        );
        let workspace = Arc::new(workspace);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let closer = Arc::clone(&workspace);
        let start = Arc::clone(&barrier);
        let closing = std::thread::spawn(move || {
            start.wait();
            closer.close();
        });
        let caller = Arc::clone(&workspace);
        let start = Arc::clone(&barrier);
        let input = request.clone();
        let applying = std::thread::spawn(move || {
            start.wait();
            let _ = caller.try_apply(input);
        });
        barrier.wait();
        closing.join().unwrap();
        applying.join().unwrap();
        assert!(
            retained.upgrade().is_none(),
            "closed poisoned workspace retained a source"
        );
    }
}

#[test]
fn tape_workspace_matches_native_after_edit_noop_and_invalid_revision() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
    });
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "workspace-parity", "revision": 1,
        "mode": "steer",
        "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [{
            "id": "print", "violation": { "anchor": "witness.span", "code": "PRINT",
                "message": "found print" },
            "forbid": { "ast": { "language": "python",
                "rule": { "pattern": "print($ARG)" } } }
        }]
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    let file =
        |source: &str| json!({ "path": "sample.py", "language": "python", "source": source });
    let apply = |revision, files: Vec<Value>| {
        json!({
            "revision": revision, "files": files, "removed": []
        })
        .to_string()
    };
    let native = |source: &str| -> Value {
        let input = json!({ "format": "rerail.repository-snapshot/v1", "revision": 0,
            "facts": [], "files": [file(source)] });
        serde_json::from_str(
            &compiled
                .evaluate(input.to_string(), "report".into())
                .unwrap(),
        )
        .unwrap()
    };
    for (revision, source, submit) in [
        (1, "print('é')\n", true),
        (2, "print('é')\n", false),
        (3, "print('é')\n", true),
        (4, "print('é')\n", false),
        (5, "# λ\nprint('new')\n", true),
        (6, "safe()\n", true),
        (7, "print('back')\n", true),
        (8, "def broken(:\n", true),
        (9, "print('back')\n", true),
    ] {
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: apply(revision, if submit { vec![file(source)] } else { vec![] }),
            request_buffer: None,
            request_direct: None,
        };
        let actual: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
        let expected = native(source);
        assert_eq!(
            actual["evaluation"]["status"], expected["status"],
            "revision {revision}"
        );
        assert_eq!(
            actual["evaluation"]["violations"], expected["violations"],
            "revision {revision}"
        );
        if revision == 8 {
            // Error-tolerant evaluation: a sound rule runs around the syntax error.
            assert_eq!(expected["failures"][0]["code"], "source-partial");
            assert_eq!(
                actual["evaluation"]["failures"][0]["code"],
                "source-partial"
            );
            assert_eq!(actual["evaluation"]["failures"][0]["path"], "sample.py");
        } else {
            assert_eq!(
                actual["evaluation"]["failures"], expected["failures"],
                "revision {revision}"
            );
        }
        if revision == 4 {
            assert_eq!(actual["evaluation"]["metrics"]["files"], 1);
        }
        if revision == 3 {
            // Code rules are immutable after the first evaluation; scheduling an
            // unchanged file explicitly exercises the retained tape restore.
            workspace
                .state
                .lock()
                .unwrap()
                .dirty_files
                .insert("sample.py".into());
        }
        if revision == 2 || revision == 3 {
            assert_eq!(actual["evaluation"]["metrics"]["files"], 0);
        }
    }
    let mut bad = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: apply(9, vec![file("print('wrong')\n")]),
        request_buffer: None,
        request_direct: None,
    };
    assert!(bad.compute().is_err());
    let mut verify = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: apply(10, vec![]),
        request_buffer: None,
        request_direct: None,
    };
    let actual: Value = serde_json::from_str(&verify.compute().unwrap()).unwrap();
    assert_eq!(
        actual["evaluation"]["violations"],
        native("print('back')\n")["violations"]
    );
    workspace.close();
}

#[test]
fn close_cancels_busy_owner_without_waiting_for_its_lock() {
    let workspace = Arc::new(CodeRuleDslWorkspace::new());
    {
        let mut state = workspace.state.lock().unwrap();
        let input = FileInput {
            path: "busy.py".into(),
            language: "python".into(),
            source: Arc::new("print('x')\n".into()),
            module: None,
            package: None,
        };
        let file = make_file(input, 1, &[], &mut state.candidate_sets);
        state.files.insert(file.input.path.clone(), file);
    }
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let owner = Arc::clone(&workspace);
    let worker = std::thread::spawn(move || {
        let guard = owner.state.lock().unwrap();
        let lease = StateLease {
            guard: Some(guard),
            mutex: &owner.state,
            closed: &owner.closed,
        };
        acquired_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        drop(lease);
    });
    acquired_rx.recv().unwrap();
    workspace.close();
    assert!(workspace.closed.load(Ordering::Acquire));
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(workspace.state.lock().unwrap().files.is_empty());
}

#[test]
fn close_and_lease_release_handoff_drops_resident_sources() {
    for _ in 0..128 {
        let workspace = Arc::new(CodeRuleDslWorkspace::new());
        let source = Arc::new(String::from("resident source"));
        let retained = Arc::downgrade(&source);
        {
            let mut state = workspace.state.lock().unwrap();
            let file = make_file(
                FileInput {
                    path: "resident.py".into(),
                    language: "python".into(),
                    source: Arc::clone(&source),
                    module: None,
                    package: None,
                },
                1,
                &[],
                &mut state.candidate_sets,
            );
            state.files.insert(file.input.path.clone(), file);
        }
        drop(source);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let owner = Arc::clone(&workspace);
        let release = Arc::clone(&barrier);
        let worker = std::thread::spawn(move || {
            let guard = owner.state.lock().unwrap();
            let lease = StateLease {
                guard: Some(guard),
                mutex: &owner.state,
                closed: &owner.closed,
            };
            release.wait();
            drop(lease);
        });
        let closer = Arc::clone(&workspace);
        let start = Arc::clone(&barrier);
        let closing = std::thread::spawn(move || {
            start.wait();
            closer.close();
        });
        barrier.wait();
        worker.join().unwrap();
        closing.join().unwrap();
        assert!(
            retained.upgrade().is_none(),
            "closed workspace retained a source"
        );
    }
}

#[test]
fn deferred_global_flush_uses_complete_taped_evidence() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
    });
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "global-tape", "revision": 1,
        "mode": "steer",
        "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [{
            "id": "cross-file", "violation": { "anchor": "witness.span", "code": "CROSS",
                "message": "cross file" },
            "forbid": { "select_when": {
                "select": { "regex": { "pattern": "report.txt$", "source": "file.path" } },
                "exists": { "ast": { "language": "python",
                "rule": { "pattern": "print($ARG)" } } }
            } }
        }]
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    let report = json!({ "path": "report.txt", "language": "typescript", "source": "{}" });
    let evidence =
        |source: &str| json!({ "path": "evidence.py", "language": "python", "source": source });
    let run = |revision: u32, files: Vec<Value>, deferred: bool| -> Value {
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": revision, "files": files,
                "removed": [], "deferGlobal": deferred })
            .to_string(),
            request_buffer: None,
            request_direct: None,
        };
        serde_json::from_str(&task.compute().unwrap()).unwrap()
    };
    let first = run(1, vec![report.clone(), evidence("print('yes')\n")], true);
    assert_eq!(first["evaluation"]["violations"], json!([]));
    let flushed = run(2, vec![], false);
    let expected: Value = serde_json::from_str(
        &compiled
            .evaluate(
                json!({
                    "format": "rerail.repository-snapshot/v1", "revision": 0, "facts": [],
                    "files": [report.clone(), evidence("print('yes')\n")]
                })
                .to_string(),
                "report".into(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(flushed["evaluation"]["status"], expected["status"]);
    assert_eq!(flushed["evaluation"]["violations"], expected["violations"]);
    assert_eq!(flushed["evaluation"]["failures"], expected["failures"]);
    let cleared = run(3, vec![evidence("safe()\n")], false);
    assert_eq!(cleared["evaluation"]["violations"], json!([]));
    workspace.close();
}

fn tape_test_code_rule() -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "tape-lifecycle", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [
                { "id": "python", "violation": { "anchor": "witness.span", "code": "PY",
                    "message": "python call" },
                  "forbid": { "ast": { "language": "python",
                "rule": { "pattern": "print($ARG)" } } } },
                { "id": "typescript", "violation": { "anchor": "witness.span", "code": "TS",
                    "message": "typescript call" },
                  "forbid": { "ast": { "language": "typescript",
                "rule": { "pattern": "print($ARG)" } } } },
                { "id": "blade", "violation": { "anchor": "witness.span", "code": "BLADE",
                    "message": "blade echo" },
                  "forbid": { "ast": { "language": "blade",
                "rule": { "kind": "echo" } } } }
            ]
        })
        .to_string(),
        "json".into(),
        json!({
            "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
                "languages": { "python": "tree-sitter-python@0.23",
                    "typescript": "tree-sitter-typescript@0.23", "blade": "forte@1.1.0" } },
            "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
                "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
                "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
        })
        .to_string(),
    )
    .unwrap()
}

fn tape_test_apply(workspace: &CodeRuleDslWorkspace, revision: u32, files: Vec<Value>) -> Value {
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": revision, "files": files, "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    serde_json::from_str(&task.compute().unwrap()).unwrap()
}

#[test]
fn binary_workspace_request_matches_json_and_rejects_malformed_lengths() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let source = "print('binary')\n";
    let metadata = json!({
        "revision": 1,
        "files": [{"path": "binary.py", "language": "python", "sourceBytes": source.len()}],
        "removed": []
    })
    .to_string();
    let mut buffer = Vec::new();
    buffer.extend_from_slice(BINARY_REQUEST_MAGIC);
    buffer.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    buffer.extend_from_slice(metadata.as_bytes());
    buffer.extend_from_slice(source.as_bytes());
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&tape_test_code_rule(), None)
        .unwrap();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: String::new(),
        request_buffer: Some(buffer.clone()),
        request_direct: None,
    };
    let result: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    assert_eq!(result["evaluation"]["violations"][0]["path"], "binary.py");

    buffer.pop();
    let Err(error) = decode_buffer_request(&buffer) else {
        panic!("truncated workspace buffer was accepted");
    };
    assert_eq!(error.reason, "workspace buffer source length is invalid");
    workspace.close();
}

#[test]
fn stored_workspace_releases_source_and_returns_changed_file_deltas() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let database = std::env::temp_dir().join(format!(
        "rerail-policy-storage-{}.sqlite",
        std::process::id()
    ));
    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
    }
    let store = StdArc::new(ContentStore::open(&database).unwrap());
    let root = HashKey::git_sha1([9; 20]);
    store
        .put_git_trees_bulk(&[GitTree {
            id: root,
            prefix: String::new(),
            entries: Vec::new(),
        }])
        .unwrap();
    let snapshot = store
        .put_snapshot(
            root,
            1,
            HashKey::sha256(b"rerail-grammars-v1"),
            i64::from(tape::TAPE_FORMAT_VERSION),
        )
        .unwrap();
    let owner = "stored-workspace-test";
    let workspace_id = store
        .open_workspace("/stored-test", "repository", owner, snapshot, 1, i64::MAX)
        .unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&tape_test_code_rule(), None)
        .unwrap();
    workspace.state.lock().unwrap().storage = Some(StorageContext {
        store: StdArc::clone(&store),
        repository: None,
        repository_prefix: String::new(),
        workspace_id,
        snapshot_id: snapshot,
        owner: owner.to_owned(),
        _maintenance: None,
    });

    let first = tape_test_apply(
        &workspace,
        1,
        vec![
            json!({
                "path": "first.py", "language": "python", "source": "print('first')\n"
            }),
            json!({
                "path": "irrelevant.py", "language": "python", "source": "value = 1\n"
            }),
        ],
    );
    assert_eq!(first["replacedPaths"], json!(["first.py", "irrelevant.py"]));
    assert_eq!(
        first["evaluation"]["violations"].as_array().unwrap().len(),
        1
    );
    let state = workspace.state.lock().unwrap();
    assert!(state.files["first.py"].input.source.is_empty());
    assert!(state.files["first.py"].tape.is_none());
    assert!(state.files["first.py"].stored.is_some());
    drop(state);
    let stored = store
        .overlay_file(workspace_id, "first.py")
        .unwrap()
        .unwrap();
    assert_eq!(
        store.get_source(stored.source).unwrap(),
        b"print('first')\n"
    );
    assert!(stored.tape.is_some());
    // Publication leaves selector indexes to the maintenance thread; once it has run, the
    // stored tapes answer selector lookups.
    let irrelevant_tape = store
        .overlay_file(workspace_id, "irrelevant.py")
        .unwrap()
        .unwrap()
        .tape
        .unwrap();
    let call = ast_grep_language::LanguageExt::get_ts_language(
        &crate::code_rule_dsl::language::GrammarDialect::Python,
    )
    .id_for_node_kind("call", true);
    let lookups = [stored.tape.unwrap(), irrelevant_tape].map(|tape_id| {
        crate::content_store::SearchKindLookup {
            tape_id,
            index_version: crate::code_rule_dsl::transducer::POSITIONAL_INDEX_VERSION,
            kind_tokens: vec![crate::code_rule_dsl::transducer::kind_index_token(call)],
        }
    });
    assert_eq!(
        store.get_search_kind_presence_bulk(&lookups).unwrap(),
        vec![
            crate::content_store::StoredKindPresence::Unknown,
            crate::content_store::StoredKindPresence::Unknown
        ],
        "no index before the maintenance pass"
    );
    let mut cursor = None;
    index_stored_tapes(&store, &AtomicBool::new(false), &mut cursor);
    assert!(
        store
            .tapes_without_search_index_after(
                grammar_version(),
                i64::from(tape::TAPE_FORMAT_VERSION),
                None,
                8
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.get_search_kind_presence_bulk(&lookups).unwrap(),
        vec![
            crate::content_store::StoredKindPresence::Present,
            crate::content_store::StoredKindPresence::Absent
        ],
        "the indexed tape containing `print(...)` has a call; the other does not"
    );

    let second = tape_test_apply(
        &workspace,
        2,
        vec![json!({
            "path": "second.py", "language": "python", "source": "print('second')\n"
        })],
    );
    assert_eq!(second["replacedPaths"], json!(["second.py"]));
    assert_eq!(
        second["evaluation"]["violations"].as_array().unwrap().len(),
        1
    );
    assert_eq!(second["evaluation"]["violations"][0]["path"], "second.py");
    let complete = tape_test_apply(&workspace, 3, Vec::new());
    workspace.completion_task().compute().unwrap();
    assert_eq!(
        store.completed_workspace_revision(workspace_id).unwrap(),
        Some(3)
    );
    assert!(complete["replacedPaths"].is_null());
    assert_eq!(
        complete["evaluation"]["violations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    RESULT_PAGE_WITNESSES_VISITED.set(0);
    RESULT_PAGE_SORTS.set(0);
    let (paged, pages) = all_result_pages(&workspace, 1);
    assert_eq!(pages, 2);
    assert_eq!(
        paged,
        *complete["evaluation"]["violations"].as_array().unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&paged).unwrap(),
        serde_json::to_vec(complete["evaluation"]["violations"].as_array().unwrap()).unwrap(),
        "concatenated paged findings preserve complete-result bytes"
    );
    assert_eq!(
        RESULT_PAGE_WITNESSES_VISITED.get(),
        2,
        "all result pages share one ordered witness index"
    );
    assert_eq!(
        RESULT_PAGE_SORTS.get(),
        1,
        "all result pages share one sorted index"
    );
    assert!(workspace.evict_inactive().unwrap());
    let evicted = {
        let state = workspace.state.lock().unwrap();
        assert!(state.files.is_empty());
        state.evicted_inputs.len()
    };
    assert_eq!(evicted, 3, "every file is evicted with its stored results");
    RESULT_PAGE_EVICTED_RESTORES.set(0);
    let first_page: Value = serde_json::from_str(&result_page(&workspace, 0, 1).unwrap()).unwrap();
    assert!(!first_page["done"].as_bool().unwrap());
    assert_eq!(
        RESULT_PAGE_EVICTED_RESTORES.get(),
        evicted,
        "the index at cursor zero restores each evicted file's stored results once"
    );
    let second_page: Value = serde_json::from_str(
        &result_page(&workspace, first_page["cursor"].as_u64().unwrap() as u32, 1).unwrap(),
    )
    .unwrap();
    assert!(second_page["done"].as_bool().unwrap());
    assert_eq!(
        RESULT_PAGE_EVICTED_RESTORES.get(),
        evicted,
        "pages after cursor zero do not decode evicted result state"
    );
    let mut evicted_paged = first_page["violations"].as_array().unwrap().clone();
    evicted_paged.extend(second_page["violations"].as_array().unwrap().clone());
    assert_eq!(
        evicted_paged, paged,
        "evicted files page their stored results"
    );
    let restored = tape_test_apply(
        &workspace,
        4,
        vec![json!({
            "path": "first.py", "language": "python", "source": "print('first')\n"
        })],
    );
    assert_eq!(
        restored["evaluation"]["violations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let restored_state = workspace.state.lock().unwrap();
    assert_eq!(
        restored_state.files["first.py"].restored_local_results,
        usize::from(REUSE_LOCAL_RESULTS)
    );
    assert!(restored_state.files["first.py"].tape.is_none());
    drop(restored_state);
    let irrelevant = tape_test_apply(
        &workspace,
        5,
        vec![json!({
            "path": "irrelevant.py", "language": "python", "source": "value = 1\n"
        })],
    );
    assert_eq!(irrelevant["evaluation"]["violations"], json!([]));
    let irrelevant_state = workspace.state.lock().unwrap();
    assert!(irrelevant_state.files["irrelevant.py"].tape.is_none());
    drop(irrelevant_state);
    let after_eviction = tape_test_apply(
        &workspace,
        6,
        vec![json!({
            "path": "first.py", "language": "python", "source": "print('after')\n"
        })],
    );
    assert_eq!(after_eviction["replacedPaths"], json!(["first.py"]));
    assert_eq!(
        after_eviction["evaluation"]["violations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    workspace.close();
    drop(store);
    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", database.display()));
    }
}

/// Issues a result page request the way the JS thread does (`resultPageAsync` returns its task);
/// the returned closure is the work the request leaves for a libuv pool thread.
fn request_result_page(
    workspace: &CodeRuleDslWorkspace,
    cursor: u32,
    maximum_bytes: u32,
) -> impl FnOnce() -> napi::Result<String> {
    let mut task = workspace.result_page_task(cursor, maximum_bytes);
    move || task.compute()
}

pub(super) fn result_page(
    workspace: &CodeRuleDslWorkspace,
    cursor: u32,
    maximum_bytes: u32,
) -> napi::Result<String> {
    request_result_page(workspace, cursor, maximum_bytes)()
}

/// Every violation of the workspace's current revision, paged; and the number of pages.
fn all_result_pages(workspace: &CodeRuleDslWorkspace, maximum_bytes: u32) -> (Vec<Value>, usize) {
    let mut violations = Vec::new();
    let mut cursor = 0;
    for pages in 1.. {
        let page: Value =
            serde_json::from_str(&result_page(workspace, cursor, maximum_bytes).unwrap()).unwrap();
        violations.extend(page["violations"].as_array().unwrap().iter().cloned());
        if page["done"].as_bool().unwrap() {
            return (violations, pages);
        }
        cursor = page["cursor"].as_u64().unwrap() as u32;
    }
    unreachable!("result pages always finish")
}

/// A workspace with two violations, one per one-byte result page.
fn paging_workspace() -> CodeRuleDslWorkspace {
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&tape_test_code_rule(), None)
        .unwrap();
    let applied = tape_test_apply(
        &workspace,
        1,
        vec![
            json!({ "path": "first.py", "language": "python", "source": "print('first')\n" }),
            json!({ "path": "second.py", "language": "python", "source": "print('second')\n" }),
        ],
    );
    assert_eq!(
        applied["evaluation"]["violations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    workspace
}

#[test]
fn result_page_request_leaves_the_index_build_to_its_pool_task() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let workspace = paging_workspace();
    RESULT_PAGE_SORTS.set(0);
    RESULT_PAGE_WITNESSES_VISITED.set(0);
    let pending = request_result_page(&workspace, 0, 1);
    assert_eq!(
        (RESULT_PAGE_SORTS.get(), RESULT_PAGE_WITNESSES_VISITED.get()),
        (0, 0),
        "issuing a result page request on the JS thread builds no result index"
    );
    let page: Value = serde_json::from_str(&pending().unwrap()).unwrap();
    assert!(!page["done"].as_bool().unwrap());
    assert_eq!(page["violations"].as_array().unwrap().len(), 1);
    assert_eq!(
        (RESULT_PAGE_SORTS.get(), RESULT_PAGE_WITNESSES_VISITED.get()),
        (1, 2),
        "the pool task builds the index for cursor zero"
    );
    workspace.close();
}

#[test]
fn memory_profile_counts_an_open_result_page_index() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let workspace = paging_workspace();
    let profile =
        || -> Value { serde_json::from_str(&workspace.memory_profile().unwrap()).unwrap() };
    let closed = profile();
    let first: Value = serde_json::from_str(&result_page(&workspace, 0, 1).unwrap()).unwrap();
    assert!(!first["done"].as_bool().unwrap());
    let open = profile();
    let index_bytes = open["resultPageIndexBytesEstimate"].as_u64();
    assert!(
        index_bytes.is_some_and(|bytes| bytes > 0),
        "an open result page index is resident memory: {open}"
    );
    assert_eq!(
        open["estimatedBytes"].as_u64(),
        closed["estimatedBytes"]
            .as_u64()
            .map(|bytes| bytes + index_bytes.unwrap()),
        "the estimate includes the open result page index"
    );
    workspace.close();
}

#[test]
fn last_result_page_releases_the_result_index() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let workspace = paging_workspace();
    let first: Value = serde_json::from_str(&result_page(&workspace, 0, 1).unwrap()).unwrap();
    assert!(!first["done"].as_bool().unwrap());
    assert!(workspace.state.lock().unwrap().result_page_index.is_some());
    let last: Value = serde_json::from_str(
        &result_page(&workspace, first["cursor"].as_u64().unwrap() as u32, 1).unwrap(),
    )
    .unwrap();
    assert!(last["done"].as_bool().unwrap());
    assert!(
        workspace.state.lock().unwrap().result_page_index.is_none(),
        "the last page releases the result index"
    );
    workspace.close();
}

#[test]
fn result_page_cursor_is_stale_after_an_apply() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let workspace = paging_workspace();
    let first: Value = serde_json::from_str(&result_page(&workspace, 0, 1).unwrap()).unwrap();
    assert!(!first["done"].as_bool().unwrap());
    tape_test_apply(&workspace, 2, Vec::new());
    let Err(error) = result_page(&workspace, first["cursor"].as_u64().unwrap() as u32, 1) else {
        panic!("a cursor from before an apply was accepted");
    };
    assert_eq!(
        error.reason,
        "result page cursor is stale; restart at cursor 0"
    );
    assert_eq!(all_result_pages(&workspace, 1).0.len(), 2);
    workspace.close();
}

#[test]
fn close_during_live_native_tree_or_restored_doc_cancels_and_releases_source() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let compiled = tape_test_code_rule();
    // A tree-sitter tree, Forte's tree, and a restored document.
    let python = json!({
        "path": "active.py", "language": "python", "source": "print('active')\n"
    });
    let blade = json!({
        "path": "active.blade.php", "language": "blade", "source": "{{ $active }}\n"
    });
    for (phase, file) in [
        ("native-tree", python.clone()),
        ("native-tree", blade),
        ("restored-doc", python),
    ] {
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.add_code_rule(&compiled, None).unwrap();
        if phase == "restored-doc" {
            let initial = tape_test_apply(&workspace, 1, vec![file.clone()]);
            assert_eq!(initial["evaluation"]["status"], "completed");
            assert_eq!(initial["evaluation"]["violations"][0]["code"], "PY");
            // An unchanged dirty file re-enters through the retained image.
            workspace
                .state
                .lock()
                .unwrap()
                .dirty_files
                .insert("active.py".into());
        }
        let revision = if phase == "restored-doc" { 2 } else { 1 };
        let files = if phase == "restored-doc" {
            vec![]
        } else {
            vec![file]
        };
        let workspace = Arc::new(workspace);
        let (reached_tx, reached_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let owner = Arc::clone(&workspace);
        let worker = std::thread::spawn(move || {
            TEST_PHASE_GATE.with(|gate| {
                *gate.borrow_mut() = Some(TestPhaseGate {
                    phase,
                    reached: reached_tx,
                    release: release_rx,
                });
            });
            let mut task = WorkspaceApplyTask {
                state: Arc::clone(&owner.state),
                closed: Arc::clone(&owner.closed),
                request_json: json!({
                    "revision": revision, "files": files, "removed": []
                })
                .to_string(),
                request_buffer: None,
                request_direct: None,
            };
            task.compute().map_err(|error| error.reason)
        });
        let source = reached_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("worker must own the requested parsed tree or restored doc");
        assert!(source.upgrade().is_some(), "{phase} source was not live");
        let (close_tx, close_rx) = std::sync::mpsc::channel();
        let closing = Arc::clone(&workspace);
        let closer = std::thread::spawn(move || {
            closing.close();
            close_tx.send(()).expect("close observer");
        });
        // Always release the owner before asserting: a blocking-close regression
        // must fail the test, not strand both threads at the phase gate.
        let close_before_release = close_rx.recv_timeout(Duration::from_secs(10));
        release_tx.send(()).expect("release phase gate");
        closer.join().unwrap();
        assert!(
            close_before_release.is_ok(),
            "{phase} close blocked behind the active owner"
        );
        assert_eq!(
            worker.join().unwrap().unwrap_err(),
            "policy workspace is closed",
            "{phase} must not return a partial successful evaluation"
        );
        assert!(
            source.upgrade().is_none(),
            "{phase} retained the closed source"
        );
        assert!(workspace.state.lock().unwrap().files.is_empty());
        assert_eq!(
            workspace
                .try_apply(
                    json!({ "revision": revision + 1, "files": [],
                "removed": [] })
                    .to_string()
                )
                .unwrap(),
            None
        );
        let mut later = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": revision + 1, "files": [],
                "removed": [] })
            .to_string(),
            request_buffer: None,
            request_direct: None,
        };
        assert_eq!(
            later.compute().unwrap_err().reason,
            "policy workspace is closed"
        );
    }
}

#[test]
fn cached_tape_rejects_revision_source_and_grammar_mismatch_without_stale_violations() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let compiled = tape_test_code_rule();
    let original = json!({ "path": "active.py", "language": "python",
        "source": "print('stale')\n" });
    let stable = json!({ "path": "stable.py", "language": "python",
        "source": "print('stable')\n" });
    let donor = json!({ "path": "donor.ts", "language": "typescript",
        "source": "print('stale')\n" });
    let repaired = json!({ "path": "active.py", "language": "python",
        "source": "print('fixed')\n" });
    for mismatch in ["revision", "source", "grammar"] {
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.add_code_rule(&compiled, None).unwrap();
        let baseline = tape_test_apply(
            &workspace,
            1,
            vec![original.clone(), stable.clone(), donor.clone()],
        );
        assert_eq!(baseline["evaluation"]["status"], "completed");
        assert_eq!(baseline["evaluation"]["failures"], json!([]));
        let unaffected = baseline["evaluation"]["violations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|violation| violation["path"] != "active.py")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(unaffected.len(), 2);
        {
            let mut state = workspace.state.lock().unwrap();
            let image = state.files[if mismatch == "grammar" {
                "donor.ts"
            } else {
                "active.py"
            }]
            .tape
            .as_ref()
            .unwrap()
            .1
            .clone();
            let source = if mismatch == "source" {
                "print('fresh')\n"
            } else {
                "print('stale')\n"
            };
            let WorkspaceState {
                code_rules,
                candidate_sets,
                ..
            } = &mut *state;
            let mut changed = make_file(
                FileInput {
                    path: "active.py".into(),
                    language: "python".into(),
                    source: Arc::new(source.into()),
                    module: None,
                    package: None,
                },
                1,
                code_rules,
                candidate_sets,
            );
            changed.tape = Some((if mismatch == "revision" { 0 } else { 1 }, image));
            state.files.insert("active.py".into(), changed);
            state.dirty_files.insert("active.py".into());
        }
        let rejected = tape_test_apply(&workspace, 2, vec![]);
        assert_eq!(rejected["evaluation"]["status"], "failed", "{mismatch}");
        assert_eq!(
            rejected["evaluation"]["violations"],
            json!(unaffected),
            "{mismatch}"
        );
        assert_eq!(
            rejected["evaluation"]["failures"]
                .as_array()
                .unwrap()
                .iter()
                .map(|failure| (
                    failure["ruleId"].as_str().unwrap(),
                    failure["clauseId"].as_str().unwrap(),
                    failure["path"].as_str().unwrap(),
                    failure["code"].as_str().unwrap(),
                ))
                .collect::<Vec<_>>(),
            vec![("tape-lifecycle", "python", "active.py", "adapter-error")],
            "{mismatch}"
        );
        let restored = tape_test_apply(&workspace, 3, vec![repaired.clone()]);
        let oracle: Value = serde_json::from_str(
            &compiled
                .evaluate(
                    json!({ "format": "rerail.repository-snapshot/v1", "revision": 0,
                        "facts": [], "files": [repaired.clone(), stable.clone(), donor.clone()] })
                    .to_string(),
                    "report".into(),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            restored["evaluation"]["status"], oracle["status"],
            "{mismatch}"
        );
        assert_eq!(
            restored["evaluation"]["violations"], oracle["violations"],
            "{mismatch}"
        );
        assert_eq!(
            restored["evaluation"]["failures"], oracle["failures"],
            "{mismatch}"
        );
        workspace.close();
    }
}

fn workspace(scopes: &[&str], ast: bool) -> CodeRuleDslWorkspace {
    let mut workspace = CodeRuleDslWorkspace::new();
    for (index, scope) in scopes.iter().enumerate() {
        let query = if ast {
            json!({"ast": {"language": "typescript", "rule": {"kind": "call_expression"}}})
        } else {
            json!({"regex": {"source": "file.content", "pattern": "forbidden"}})
        };
        let code_rule = CodeRuleDslRuntime::compile(
            json!({
                "format": "rerail.policy/v1",
                "id": format!("lifecycle-{index}"),
                "revision": 1,
                "mode": "steer",
                "scope": {"include": [scope], "exclude": []},
                "clauses": [{
                    "id": "check",
                    "forbid": query,
                    "violation": {
                        "anchor": "witness.span", "code": "FORBIDDEN",
                        "message": "Forbidden call"
                    }
                }]
            })
            .to_string(),
            "json".to_owned(),
            json!({
                "format": "rerail.policy-capabilities/v1",
                "ast": {
                    "engine": "ast-grep-core", "engineVersion": "0.45",
                    "languages": {"typescript": "tree-sitter-typescript@0.23"}
                },
                "factRelations": {},
                "limits": {
                    "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
                    "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
                    "maxVisitedNodes": 100000, "maxWitnesses": 10000
                }
            })
            .to_string(),
        )
        .unwrap();
        workspace.add_code_rule(&code_rule, None).unwrap();
    }
    workspace
}

fn apply(state: &mut WorkspaceState, files: &[(&str, &str, &str)], removed: &[&str]) {
    apply_request(
        state,
        Request {
            revision: state.revision + 1,
            files: files
                .iter()
                .map(|&(path, language, source)| WireFile {
                    path: path.to_owned(),
                    language: language.to_owned(),
                    source: source.to_owned(),
                    module: None,
                    package: None,
                })
                .collect(),
            removed: removed.iter().map(|path| (*path).to_owned()).collect(),
            defer_global: false,
            quiet: false,
        },
    )
    .unwrap();
}

#[test]
fn live_peers_and_same_language_edits_share_until_final_removal() {
    let workspace = workspace(&["**/*"], true);
    let mut state = workspace.state.lock().unwrap();
    apply(
        &mut state,
        &[
            ("a.ts", "typescript", "first()"),
            ("b.ts", "typescript", "second()"),
        ],
        &[],
    );
    let candidates = Arc::downgrade(&state.files["a.ts"].candidates);
    assert!(Arc::ptr_eq(
        &state.files["a.ts"].candidates,
        &state.files["b.ts"].candidates
    ));
    apply(&mut state, &[("a.ts", "typescript", "edited()")], &[]);
    assert!(candidates.ptr_eq(&Arc::downgrade(&state.files["a.ts"].candidates)));
    apply(&mut state, &[], &["a.ts"]);
    assert!(candidates.upgrade().is_some());
    assert_eq!(state.candidate_sets.len(), 1);
    apply(&mut state, &[], &["b.ts"]);
    assert!(candidates.upgrade().is_none());
    assert!(state.candidate_sets.is_empty());
    assert_eq!(state.candidate_sets.capacity(), 0);
}

#[test]
fn language_replacement_preserves_live_peers_but_reclaims_obsolete_grammar() {
    let workspace = workspace(&["**/*"], true);
    let mut state = workspace.state.lock().unwrap();
    apply(
        &mut state,
        &[
            ("a.ts", "typescript", "first()"),
            ("b.ts", "typescript", "second()"),
        ],
        &[],
    );
    let typescript = Arc::downgrade(&state.files["a.ts"].candidates);
    apply(&mut state, &[("a.ts", "tsx", "first()")], &[]);
    let tsx = Arc::downgrade(&state.files["a.ts"].candidates);
    assert!(!typescript.ptr_eq(&tsx));
    assert!(typescript.upgrade().is_some());
    assert_eq!(state.candidate_sets.len(), 2);
    apply(&mut state, &[("b.ts", "tsx", "second()")], &[]);
    assert!(typescript.upgrade().is_none());
    assert!(tsx.ptr_eq(&Arc::downgrade(&state.files["b.ts"].candidates)));
    assert_eq!(state.candidate_sets.len(), 1);
    apply(&mut state, &[("a.ts", "python", "first()")], &["b.ts"]);
    assert!(tsx.upgrade().is_none());
    assert_eq!(state.candidate_sets.len(), 1);
}

#[test]
fn subset_churn_reclaims_keys_payloads_and_contracted_bucket_storage() {
    let workspace = workspace(&["**/a/**", "**/b/**", "**/c/**", "**/d/**"], true);
    let mut state = workspace.state.lock().unwrap();
    let paths: Vec<String> = (1..16)
        .map(|mask| {
            let mut path = String::from("src/");
            for (bit, segment) in ["a/", "b/", "c/", "d/"].iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    path.push_str(segment);
                }
            }
            path.push_str("file.ts");
            path
        })
        .collect();
    let files: Vec<_> = paths
        .iter()
        .map(|path| (path.as_str(), "typescript", "call()"))
        .collect();
    apply(&mut state, &files, &[]);
    assert_eq!(state.candidate_sets.len(), paths.len());
    let retired: Vec<_> = paths[1..]
        .iter()
        .map(|path| Arc::downgrade(&state.files[path].candidates))
        .collect();
    let removed: Vec<_> = paths[1..].iter().map(String::as_str).collect();
    apply(&mut state, &[], &removed);
    assert!(
        retired
            .iter()
            .all(|candidates| candidates.upgrade().is_none())
    );
    assert_eq!(state.candidate_sets.len(), 1);
    assert!(state.candidate_sets.capacity() <= 16);

    let mut previous = paths[0].as_str();
    for path in paths.iter().skip(1).cycle().take(42) {
        let retired = Arc::downgrade(&state.files[previous].candidates);
        apply(&mut state, &[(path, "typescript", "call()")], &[previous]);
        assert!(retired.upgrade().is_none());
        assert_eq!(state.candidate_sets.len(), 1);
        assert!(state.candidate_sets.capacity() <= 16);
        previous = path;
    }
    apply(&mut state, &[], &[previous]);
    assert!(state.candidate_sets.is_empty());
    assert_eq!(state.candidate_sets.capacity(), 0);
}

#[test]
fn rejected_inline_staging_does_not_retain_new_scope_or_grammar_candidates() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let workspace = workspace(&["**/a/**", "**/b/**"], false);
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({
            "revision": 1,
            "files": [{"path": "src/a/live.ts", "language": "typescript", "source": "safe"}],
            "removed": []
        })
        .to_string(),
        request_buffer: None,
        request_direct: None,
    };
    task.compute().unwrap();
    let live = Arc::downgrade(&workspace.state.lock().unwrap().files["src/a/live.ts"].candidates);
    for (path, language) in [
        ("src/a/live.ts", "tsx"),
        ("src/b/rejected.ts", "typescript"),
        ("src/a/b/rejected.py", "python"),
    ] {
        let result = workspace.try_apply(
            json!({
                "revision": 1,
                "files": [{"path": path, "language": language, "source": "safe"}],
                "removed": []
            })
            .to_string(),
        );
        // Either the stale revision or an earlier inline deadline rejects staging.
        assert!(!matches!(result, Ok(Some(_))));
        let state = workspace.state.lock().unwrap();
        assert_eq!(state.revision, 1);
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files["src/a/live.ts"].input.language, "typescript");
        assert!(live.ptr_eq(&Arc::downgrade(&state.files["src/a/live.ts"].candidates)));
        assert_eq!(state.candidate_sets.len(), 1);
    }
}

const CAPABILITIES: &str = include_str!("../capabilities.json");

fn clause(id: &str, rule: Value) -> Value {
    json!({
        "id": id,
        "forbid": { "ast": { "language": "typescript", "rule": rule } },
        "violation": { "anchor": "witness.span", "code": "AST_MATCH", "message": "match" },
    })
}

fn compile_clauses(clauses: Vec<Value>) -> CompiledCodeRuleDsl {
    let document = json!({ "format": "rerail.policy/v1", "id": "tolerant", "revision": 1, "mode": "steer", "clauses": clauses });
    CodeRuleDslRuntime::compile(
        document.to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap_or_else(|error| panic!("policy compiles: {}", error.reason))
}

fn apply_clauses(clauses: Vec<Value>, files: Value) -> Value {
    apply_revisions(clauses, vec![files])
        .pop()
        .expect("one revision")
}

/// Applies each file set as the next workspace revision, returning every evaluation.
fn apply_revisions(clauses: Vec<Value>, revisions: Vec<Value>) -> Vec<Value> {
    let compiled = compile_clauses(clauses);
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&compiled, None)
        .expect("policy is added");
    // `compute` refuses to wait for the process-wide CPU slot; serialize the tests instead.
    let _slot = WORKSPACE_TEST_SLOT.lock();
    revisions
        .into_iter()
        .enumerate()
        .map(|(index, files)| {
            let mut task = WorkspaceApplyTask {
                state: Arc::clone(&workspace.state),
                closed: Arc::clone(&workspace.closed),
                request_json: json!({ "revision": index + 1, "files": files, "removed": [] })
                    .to_string(),
                request_buffer: None,
                request_direct: None,
            };
            let output = task.compute().expect("workspace applies");
            serde_json::from_str::<Value>(&output).expect("result is JSON")["evaluation"].take()
        })
        .collect()
}

fn stateless(clauses: Vec<Value>, files: Value) -> Value {
    let snapshot = json!({ "format": "rerail.repository-snapshot/v1", "revision": 0, "facts": [], "files": files });
    let output = compile_clauses(clauses)
        .evaluate(snapshot.to_string(), "report".to_owned())
        .expect("evaluates");
    serde_json::from_str(&output).expect("result is JSON")
}

fn failure_codes(evaluation: &Value) -> Vec<(String, String)> {
    let mut codes = evaluation["failures"]
        .as_array()
        .expect("failures")
        .iter()
        .map(|failure| {
            (
                failure["clauseId"].as_str().unwrap().to_owned(),
                failure["code"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    codes.sort();
    codes
}

#[test]
fn global_regex_leaves_never_mark_a_broken_file_partial() {
    // Review finding 6 on #2561: a global clause whose leaves are all regexes reported
    // source-partial on any broken file an AST clause had parsed; the stateless path never did.
    let gate = |select: Value| {
        json!({
            "id": if select.get("regex").is_some() { "regex-gate" } else { "ast-gate" },
            "forbid": { "select_when": {
                "exists": { "regex": { "source": "file.content", "pattern": "danger\\(" } },
                "select": select,
            } },
            "violation": { "anchor": "witness.span", "code": "GATE", "message": "gate" },
        })
    };
    let clauses = || {
        vec![
            clause("debugger", json!({ "kind": "debugger_statement" })),
            gate(json!({ "regex": { "source": "file.content", "pattern": "debugger" } })),
            gate(
                json!({ "ast": { "language": "typescript", "rule": { "kind": "debugger_statement" } } }),
            ),
        ]
    };
    let files = json!([
        { "language": "typescript", "path": "broken.ts", "source": "debugger; // clean text keeps the error region under half the file\nfunction f( {\n" },
        { "language": "typescript", "path": "danger.ts", "source": "danger();\n" },
    ]);
    let workspace = apply_clauses(clauses(), files.clone());
    let expected = [
        ("ast-gate".to_owned(), "source-partial".to_owned()),
        ("debugger".to_owned(), "source-partial".to_owned()),
    ];
    assert_eq!(failure_codes(&workspace), expected, "{workspace}");
    assert_eq!(failure_codes(&stateless(clauses(), files)), expected);
}

#[test]
fn an_unread_file_reports_its_detail_and_path_on_every_evaluation_path() {
    // Regression: the stateless path reported `query source is
    // incomplete` for a file the parser did not read, and a global `select_when` reported a
    // pathless `A cross-file query source is incomplete`, so `rules test` could not name the file.
    let clauses = || {
        vec![
            clause("debugger", json!({ "kind": "debugger_statement" })),
            json!({
                "id": "gated",
                "forbid": { "select_when": {
                    "exists": { "regex": { "source": "file.content", "pattern": "debugger" } },
                    "select": { "ast": { "language": "typescript", "rule": { "kind": "debugger_statement" } } },
                } },
                "violation": { "anchor": "witness.span", "code": "GATE", "message": "gate" },
            }),
        ]
    };
    // The grammar recovers `debugger;`, then reads the prose as one ERROR to the end.
    let source = format!(
        "debugger;\n{}",
        "Install it: run the script, then open the page.\n".repeat(5)
    );
    let files = json!([{ "language": "typescript", "path": "notes.ts", "source": source }]);
    let failures = |evaluation: &Value| {
        let mut failures = evaluation["failures"]
            .as_array()
            .expect("failures")
            .iter()
            .map(|failure| {
                [
                    failure["clauseId"].clone(),
                    failure["code"].clone(),
                    failure["path"].clone(),
                    failure["message"].clone(),
                ]
            })
            .collect::<Vec<_>>();
        failures.sort_by_key(|failure| failure[0].to_string());
        failures
    };
    let unread = "file did not parse: error regions cover 95% of it, first at line 2, column 1";
    let expected = vec![
        [
            json!("debugger"),
            json!("source-incomplete"),
            json!("notes.ts"),
            json!(unread),
        ],
        [
            json!("gated"),
            json!("source-incomplete"),
            json!("notes.ts"),
            json!(unread),
        ],
    ];
    let workspace = apply_clauses(clauses(), files.clone());
    assert_eq!(failures(&workspace), expected, "{workspace}");
    let full = stateless(clauses(), files);
    assert_eq!(failures(&full), expected, "{full}");
}

#[test]
fn a_global_query_names_the_file_it_could_not_read() {
    // A file a clause is unsound in (not unread) keeps the generic message, with its path.
    let clauses = || {
        vec![json!({
            "id": "gated",
            "forbid": { "select_when": {
                "exists": { "regex": { "source": "file.content", "pattern": "debugger" } },
                "select": { "ast": { "language": "typescript", "rule": {
                    "kind": "debugger_statement",
                    "not": { "follows": { "kind": "import_statement", "stopBy": "end" } },
                } } },
            } },
            "violation": { "anchor": "witness.span", "code": "GATE", "message": "gate" },
        })]
    };
    let files = json!([
        { "language": "typescript", "path": "broken.ts", "source": "debugger; // clean text keeps the error region under half the file\nfunction f( {\n" },
        { "language": "typescript", "path": "clean.ts", "source": "debugger;\n" },
    ]);
    let workspace = apply_clauses(clauses(), files);
    let failures = workspace["failures"].as_array().expect("failures");
    assert_eq!(failures.len(), 1, "{workspace}");
    assert_eq!(failures[0]["code"], "source-incomplete");
    assert_eq!(failures[0]["path"], "broken.ts");
    assert_eq!(
        failures[0]["message"],
        "A cross-file query source is incomplete"
    );
}

#[test]
fn broken_files_report_sound_witnesses_and_are_marked_partial() {
    let source = "debugger; // clean text keeps the error region under half the file\nfunction f( {\n  debugger;\n";
    let evaluation = apply_clauses(
        vec![
            clause("debugger", json!({ "kind": "debugger_statement" })),
            clause(
                "no-class",
                json!({ "kind": "program", "not": { "has": { "kind": "class_declaration", "stopBy": "end" } } }),
            ),
            clause(
                "unguarded",
                json!({ "kind": "debugger_statement", "not": { "follows": { "kind": "import_statement", "stopBy": "end" } } }),
            ),
        ],
        json!([{ "language": "typescript", "path": "broken.ts", "source": source }]),
    );
    let violations = evaluation["violations"].as_array().expect("findings");
    assert_eq!(violations.len(), 1, "{evaluation}");
    assert_eq!(violations[0]["clauseId"], "debugger");
    assert_eq!(
        violations[0]["span"]["startLine"], 0,
        "the debugger inside the damaged function is suppressed"
    );
    assert_eq!(
        failure_codes(&evaluation),
        [
            ("debugger".to_owned(), "source-partial".to_owned()),
            ("no-class".to_owned(), "source-partial".to_owned()),
            ("unguarded".to_owned(), "source-incomplete".to_owned()),
        ]
    );
    let partial = evaluation["failures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|failure| failure["code"] == "source-partial")
        .expect("a partial failure");
    assert_eq!(partial["path"], "broken.ts");
    assert!(
        partial["message"]
            .as_str()
            .unwrap()
            .contains("first at line 2"),
        "{partial}"
    );
}

/// E2E F1 on #47: a Vue template's syntax error lies outside the program the scripts and
/// template expressions make, so a clause whose witnesses lie in a clean program runs, even one
/// not sound under errors, and is partial; a clause reading the template is incomplete, and so
/// is every clause where the program has an error of its own.
#[test]
fn a_vue_template_error_leaves_program_clauses_complete() {
    let vue = |id: &str, ast: Value| {
        json!({
            "id": id,
            "forbid": { "ast": ast },
            "violation": { "anchor": "witness.span", "code": "AST_MATCH", "message": "match" },
        })
    };
    // Each is unsound under errors: a negated relation leaves the witness. The script clause
    // reads the component's program (`embed`), the template clause its own tree: each document
    // has its own syntax errors.
    let clauses = || {
        vec![
            vue(
                "script",
                json!({ "language": "vue", "embedded": "script-block", "rule": { "kind": "binary_expression", "not": { "inside": { "kind": "function_declaration", "stopBy": "end" } } } }),
            ),
            vue(
                "template",
                json!({ "language": "vue", "rule": { "kind": "element", "not": { "follows": { "kind": "element" } } } }),
            ),
        ]
    };
    let template_error = "<script setup>\nimport { ref } from 'vue'\nconst a = ref(1)\nconst b = a + 1\n</script>\n\n<template>\n  <div v-for=\"x in xs\">{{ x </div>\n  <p v-html=\"b\"></p>\n</template>\n";
    let files = json!([{ "language": "vue", "path": "BrokenTpl.vue", "source": template_error }]);
    for evaluation in [
        apply_clauses(clauses(), files.clone()),
        stateless(clauses(), files),
    ] {
        let violations = evaluation["violations"].as_array().expect("findings");
        assert_eq!(violations.len(), 1, "{evaluation}");
        assert_eq!(violations[0]["clauseId"], "script", "{evaluation}");
        assert_eq!(violations[0]["span"]["startLine"], 3, "{evaluation}");
        // A file read around syntax errors in any of its documents reports them on its clauses.
        assert_eq!(
            failure_codes(&evaluation),
            [
                ("script".to_owned(), "source-partial".to_owned()),
                ("template".to_owned(), "source-incomplete".to_owned()),
            ],
            "{evaluation}"
        );
    }
    let script_error = template_error
        .replace("a + 1\n", "a + 1\nfunction (\n")
        .replace("{{ x </div>", "{{ x }}</div>");
    let files = json!([{ "language": "vue", "path": "Broken.vue", "source": script_error }]);
    for evaluation in [
        apply_clauses(clauses(), files.clone()),
        stateless(clauses(), files),
    ] {
        let violations = evaluation["violations"].as_array().expect("findings");
        assert_eq!(violations.len(), 1, "{evaluation}");
        assert_eq!(violations[0]["clauseId"], "template", "{evaluation}");
        assert_eq!(
            failure_codes(&evaluation),
            [
                ("script".to_owned(), "source-incomplete".to_owned()),
                ("template".to_owned(), "source-partial".to_owned()),
            ],
            "{evaluation}"
        );
    }
}

/// A file's syntax errors are its documents' together: a document no clause reads still leaves the
/// file read around its errors, and a clause crossing between the documents (`embedded`) reads
/// around both's.
#[test]
fn every_document_of_a_vue_file_has_its_errors_read() {
    let vue = |id: &str, ast: Value| {
        json!({
            "id": id,
            "forbid": { "ast": ast },
            "violation": { "anchor": "witness.span", "code": "AST_MATCH", "message": "match" },
        })
    };
    // Only the program is read; the template's error still makes the file partial.
    let script = || {
        vec![vue(
            "script",
            json!({ "language": "vue", "embedded": "script-block", "rule": { "kind": "lexical_declaration" } }),
        )]
    };
    let template_error =
        "<script setup>\nconst a = 1\n</script>\n<template>\n  <div>{{ a </div>\n</template>\n";
    let files = json!([{ "language": "vue", "path": "T.vue", "source": template_error }]);
    for evaluation in [
        apply_clauses(script(), files.clone()),
        stateless(script(), files),
    ] {
        assert_eq!(
            evaluation["violations"].as_array().map(Vec::len),
            Some(1),
            "{evaluation}"
        );
        assert_eq!(
            failure_codes(&evaluation),
            [("script".to_owned(), "source-partial".to_owned())],
            "{evaluation}"
        );
    }
    // The template is clean and the program broken: a template clause reaching the program
    // (`embedded`, unsound under errors) is incomplete.
    let crossing = || {
        vec![vue(
            "crossing",
            json!({ "language": "vue", "rule": { "kind": "interpolation", "embedded": { "kind": "identifier" } } }),
        )]
    };
    let script_error = "<script setup>\nconst a = 1\nfunction (\n</script>\n<template>\n  <div>{{ a }}</div>\n</template>\n";
    let files = json!([{ "language": "vue", "path": "S.vue", "source": script_error }]);
    for evaluation in [
        apply_clauses(crossing(), files.clone()),
        stateless(crossing(), files),
    ] {
        assert_eq!(
            failure_codes(&evaluation),
            [("crossing".to_owned(), "source-incomplete".to_owned())],
            "{evaluation}"
        );
    }
}

#[test]
fn anchor_selection_keeps_witnesses_beside_error_subtrees() {
    // A pattern with an exact terminal is anchored; its postings come from the whole tree,
    // so an ERROR sibling must not hide the valid call after it. The missing comma is invalid
    // TypeScript, so the ERROR stays whatever the grammar fixes.
    let source = "const m = importOriginal(1 load(\"./api\"));\nconsole.log(m);\n";
    let evaluation = apply_clauses(
        vec![clause("log", json!({ "pattern": "console.log($A)" }))],
        json!([{ "language": "typescript", "path": "api.test.ts", "source": source }]),
    );
    let violations = evaluation["violations"].as_array().expect("findings");
    assert_eq!(violations.len(), 1, "{evaluation}");
    assert_eq!(violations[0]["span"]["startLine"], 1);
    assert_eq!(
        failure_codes(&evaluation),
        [("log".to_owned(), "source-partial".to_owned())]
    );
}

#[test]
fn sibling_selection_stays_linear_when_a_file_has_a_missing_token() {
    // Review finding 2 on #2561: one MISSING token anywhere sends every node through the
    // walked sibling clause, which was cubic in the number of siblings: 1,500 statements (3,000
    // in a release build) spent the file's time budget, and every AST clause ended
    // budget-exceeded instead of source-partial.
    let statements = (0..1500).map(|index| format!("f{index}();\n"));
    let source = statements.collect::<String>() + "import x from 'x';\nfunction g() {\n";
    let evaluation = apply_clauses(
        vec![clause(
            "after-import",
            json!({ "kind": "expression_statement", "follows": { "stopBy": "end", "kind": "import_statement" } }),
        )],
        json!([{ "language": "typescript", "path": "long.ts", "source": source }]),
    );
    assert_eq!(
        failure_codes(&evaluation),
        [("after-import".to_owned(), "source-partial".to_owned())],
        "{}",
        evaluation["failures"]
    );
}

#[test]
fn sibling_selection_matches_the_matcher_around_zero_width_nodes() {
    // MISSING tokens are zero-width; ast-grep's sibling walks position by start byte there, so
    // the anchor plan asks the same walk. Indexed violations must equal the unindexed matcher's.
    let sources = [
        "a();\nb();\nif (c) { d(); e(\n",
        "function f() { a(); b()\n",
        "x = [1, 2\ny();\nz();\n",
        "a();\nb();\nimport x from 'x';\nfunction f() {\n",
    ];
    let rules = [
        json!({ "kind": "expression_statement", "precedes": { "kind": "expression_statement" } }),
        json!({ "kind": "expression_statement", "follows": { "kind": "expression_statement" } }),
        json!({ "kind": "expression_statement", "precedes": { "stopBy": "end", "kind": "expression_statement" } }),
        json!({ "kind": "expression_statement", "follows": { "stopBy": "end", "kind": "expression_statement" } }),
        json!({ "kind": "number", "precedes": { "regex": "^,$" } }),
        json!({ "kind": "program", "precedes": { "stopBy": "end", "kind": "expression_statement" } }),
        json!({ "kind": "expression_statement", "follows": { "stopBy": "end", "kind": "import_statement" } }),
        json!({ "kind": "expression_statement", "precedes": { "stopBy": "end", "kind": "import_statement" } }),
    ];
    for source in sources {
        for rule in &rules {
            let files = json!([{ "language": "typescript", "path": "zero.ts", "source": source }]);
            let indexed = apply_clauses(vec![clause("sibling", rule.clone())], files.clone());
            let document = json!({ "format": "rerail.policy/v1", "id": "tolerant", "revision": 1, "mode": "steer", "clauses": [clause("sibling", rule.clone())] });
            let compiled = CodeRuleDslRuntime::compile(
                document.to_string(),
                "json".to_owned(),
                CAPABILITIES.to_owned(),
            )
            .unwrap_or_else(|error| panic!("policy compiles: {}", error.reason));
            let snapshot = json!({ "format": "rerail.repository-snapshot/v1", "revision": 0, "facts": [], "files": files });
            let stateless: Value = serde_json::from_str(
                &compiled
                    .evaluate(snapshot.to_string(), "report".to_owned())
                    .expect("evaluates"),
            )
            .expect("result is JSON");
            assert_eq!(
                indexed["violations"], stateless["violations"],
                "{source:?} {rule}"
            );
        }
    }
}

fn reported_clause(id: &str, forbid: Value, violation: Value) -> Value {
    json!({ "id": id, "forbid": forbid, "violation": violation })
}

fn reporting_clauses() -> Vec<Value> {
    vec![
        reported_clause(
            "log",
            json!({ "ast": { "language": "typescript", "rule": { "pattern": "console.log($$$ARGS)" } } }),
            json!({ "anchor": "capture.ARGS", "code": "LOG", "message": "log of $$$ARGS", "fix": "logger.info($$$ARGS)" }),
        ),
        reported_clause(
            "end",
            json!({ "ast": { "language": "typescript", "rule": { "kind": "return_statement", "has": { "pattern": "$V" } } } }),
            json!({ "anchor": "witness.end", "code": "RET", "message": "returns $V" }),
        ),
        reported_clause(
            "todo",
            json!({ "regex": { "source": "file.content", "pattern": "TODO\\((?<WHO>[a-z]+)\\)" } }),
            json!({ "anchor": "capture.WHO", "code": "TODO", "message": "todo for $WHO", "fix": "TODO($WHO, #123)" }),
        ),
    ]
}

#[test]
fn php_suppressions_remove_the_same_violations_as_stateless() {
    let clause = |id: &str, suppress: Value| {
        json!({
            "id": id,
            "forbid": { "ast": { "language": "php", "rule": { "kind": "variable_name" } } },
            "violation": { "anchor": "witness.span", "code": id, "message": "m",
                "suppress": suppress },
        })
    };
    let clauses = || {
        vec![
            clause(
                "psalm",
                json!({ "psalm": { "issues": ["UndefinedVariable"] } }),
            ),
            clause(
                "phpcs",
                json!({ "phpcs": { "sniffs": ["Squiz.PHP.DiscouragedFunctions"] } }),
            ),
        ]
    };
    // Each revision: its source, and the variables each clause keeps.
    let revisions = [
        (
            "<?php\nfunction f() {\n    /** @psalm-suppress UndefinedVariable */\n    echo $a;\n    echo $b; // phpcs:ignore\n    echo $c;\n}\n",
            [("psalm", vec!["$b", "$c"]), ("phpcs", vec!["$a", "$c"])],
        ),
        (
            "<?php\nfunction f() {\n    echo $a;\n    // phpcs:disable Squiz\n    echo $b;\n    /** @psalm-suppress all */\n    echo $c;\n}\n",
            [("psalm", vec!["$a", "$b", "$c"]), ("phpcs", vec!["$a"])],
        ),
    ];
    let files = |source: &str| json!([{ "language": "php", "path": "a.php", "source": source }]);
    let incremental = apply_revisions(
        clauses(),
        revisions.iter().map(|(source, _)| files(source)).collect(),
    );
    for ((source, kept), evaluation) in revisions.iter().zip(&incremental) {
        assert_eq!(
            evaluation["violations"],
            stateless(clauses(), files(source))["violations"],
            "{source:?}"
        );
        for (id, expected) in kept {
            let texts = evaluation["violations"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|violation| violation["clauseId"] == *id)
                .map(|violation| {
                    let span = &violation["span"];
                    &source[span["startByte"].as_u64().unwrap() as usize
                        ..span["endByte"].as_u64().unwrap() as usize]
                })
                .collect::<Vec<_>>();
            assert_eq!(&texts, expected, "{id} {source:?}");
        }
    }
}

#[test]
fn workspace_violations_carry_anchors_messages_and_fixes_like_stateless() {
    // Every revision's violations must equal a fresh stateless evaluation, including after a
    // changed indent before an unchanged witness re-indents its multi-line fix.
    let sources = [
        "function f() {\n  console.log(a, {\n    b,\n  });\n  // TODO(ann)\n  return a;\n}\nconsole.log(z);\n",
        "function f() {\n  console.log(a, {\n    b,\n  });\n  // TODO(ann)\n  return a;\n}\nconsole.log(y, 2);\n",
        "// head\nfunction f() {\n  x; console.log(a, {\n    b,\n  });\n  // TODO(bob)\n  return a;\n}\nconsole.log(y, 2);\n",
        "// head\nfunction f() {\n      x; console.log(a, {\n    b,\n  });\n  // TODO(bob)\n  return a;\n}\nconsole.log(y, 2);\n",
    ];
    let files =
        |source: &str| json!([{ "language": "typescript", "path": "a.ts", "source": source }]);
    let incremental = apply_revisions(
        reporting_clauses(),
        sources.iter().map(|source| files(source)).collect(),
    );
    for (source, evaluation) in sources.iter().zip(&incremental) {
        let fresh = stateless(reporting_clauses(), files(source));
        assert_eq!(evaluation["violations"], fresh["violations"], "{source:?}");
        assert_eq!(
            evaluation["violations"].as_array().map(Vec::len),
            Some(4),
            "{evaluation}"
        );
    }
    let last = &incremental[3]["violations"];
    let log = last
        .as_array()
        .unwrap()
        .iter()
        .find(|violation| violation["clauseId"] == "log")
        .unwrap();
    assert_eq!(log["message"], "log of a, {…");
    assert_eq!(
        log["fix"]["text"],
        "logger.info(a, {\n          b,\n        })"
    );
}

/// A violation's anchored text, message and fix (its text over its range, and its text).
type Found = (String, String, Option<(String, String)>);

/// Violations of `clauses` over one file, the workspace's and a stateless evaluation's, which
/// must agree.
fn agreed_violations(clauses: impl Fn() -> Vec<Value>, language: &str, source: &str) -> Vec<Found> {
    let path = if language == "python" { "a.py" } else { "a.ts" };
    let files = json!([{ "language": language, "path": path, "source": source }]);
    let incremental = apply_clauses(clauses(), files.clone());
    let fresh = stateless(clauses(), files);
    assert_eq!(incremental["violations"], fresh["violations"]);
    assert_eq!(incremental["failures"], json!([]), "{incremental}");
    let text = |span: &Value| {
        source[span["startByte"].as_u64().unwrap() as usize
            ..span["endByte"].as_u64().unwrap() as usize]
            .to_owned()
    };
    let violations = incremental["violations"].as_array().unwrap();
    let ids = violations
        .iter()
        .map(|violation| violation["id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), violations.len(), "one id per finding");
    violations
        .iter()
        .map(|violation| {
            (
                text(&violation["span"]),
                violation["message"].as_str().unwrap().to_owned(),
                violation["fix"]
                    .as_object()
                    .map(|fix| (text(&fix["span"]), fix["text"].as_str().unwrap().to_owned())),
            )
        })
        .collect()
}

fn texts(violations: &[Found]) -> Vec<&str> {
    violations
        .iter()
        .map(|(text, _, _)| text.as_str())
        .collect()
}

/// A `line` transform with the fix round's `default`, read through `${NAME}` in a message.
#[test]
fn a_line_transform_supplies_its_default_where_the_capture_is_unbound() {
    let clauses = || {
        vec![reported_clause(
            "lines",
            json!({ "ast": { "language": "typescript",
                "rule": { "kind": "call_expression", "any": [{ "pattern": "f($A)" }, { "pattern": "g()" }] },
                "transform": {
                    "L": { "line": { "source": "$A", "default": "none" } },
                    "E": { "line": { "source": "$A", "edge": "end" } },
                    "S": { "lines": { "source": "$A", "default": "0" } },
                } } }),
            json!({ "anchor": "witness.span", "code": "L", "message": "argument on line ${L}th, ends on ${E}, ${S} lines" }),
        )]
    };
    let source = "f(\n  [1,\n  2]);\ng();\n";
    let messages = agreed_violations(clauses, "typescript", source)
        .into_iter()
        .map(|(_, message, _)| message)
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        [
            "argument on line 2th, ends on 3, 2 lines",
            "argument on line noneth, ends on , 0 lines"
        ]
    );
}

/// `each` over a regex group: a witness where no match of the regex binds the group reports
/// nothing, on the workspace path as on the stateless one (the fix round after the suites
/// scored M12).
#[test]
fn each_over_a_regex_group_that_no_match_binds_reports_nothing() {
    let clauses = || {
        vec![reported_clause(
            "runs",
            json!({ "ast": { "language": "typescript", "rule": { "kind": "program", "regex": "q",
                "all": [{ "regex": "'[^']*'|(?P<WS>q+)" }] } } }),
            json!({ "anchor": "capture.WS", "code": "WS", "each": "WS",
                    "message": "run $WS" }),
        )]
    };
    assert_eq!(
        agreed_violations(clauses, "typescript", "f('q', 'qq');\n"),
        []
    );
    assert_eq!(
        texts(&agreed_violations(
            clauses,
            "typescript",
            "f('q'); qq; qqq;\n"
        )),
        ["qq", "qqq"]
    );
}

/// A transform of a regex group of the witness, with `each` over the group: each match's own
/// group, lowercased into its message and fix, on both paths.
#[test]
fn a_transform_reads_each_matchs_regex_group() {
    let clauses = || {
        vec![reported_clause(
            "letters",
            json!({ "ast": { "language": "typescript", "rule": { "kind": "comment", "regex": "(?<U>\\p{Lu})" },
                "transform": { "L": { "convert": { "source": "$U", "toCase": "lowerCase" } } } } }),
            json!({ "anchor": "capture.U", "code": "U", "each": "U",
                    "message": "$U to $L", "fix": { "template": "$L", "range": "capture.U" } }),
        )]
    };
    let found = agreed_violations(clauses, "typescript", "// aBcD\n");
    assert_eq!(
        found,
        [
            (
                "B".to_owned(),
                "B to b".to_owned(),
                Some(("B".to_owned(), "b".to_owned()))
            ),
            (
                "D".to_owned(),
                "D to d".to_owned(),
                Some(("D".to_owned(), "d".to_owned()))
            ),
        ]
    );
}
#[test]
fn regex_group_transform_limit_is_local_to_its_clause_in_native_and_workspace_evaluation() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 2,
            "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
    });
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "group-transform-limit", "revision": 1,
        "mode": "steer",
        "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [
            {
                "id": "expanded", "violation": { "anchor": "witness.span", "code": "EXPANDED",
                    "message": "$U" },
                "forbid": { "ast": { "language": "python",
                    "rule": { "kind": "identifier", "regex": "(?<G>x)" },
                    "transform": {
                        "A": { "replace": { "source": "$G", "replace": "x", "by": "xx" } },
                        "U": { "replace": { "source": "$A", "replace": "x", "by": "xx" } }
                    }
                } }
            },
            {
                "id": "ordinary", "violation": { "anchor": "witness.span", "code": "ORDINARY",
                    "message": "ordinary" },
                "forbid": { "regex": { "pattern": "x", "source": "file.content" } }
            }
        ]
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap();
    let file = json!({ "path": "a.py", "language": "python", "source": "x\n" });
    let snapshot = json!({
        "format": "rerail.repository-snapshot/v1", "revision": 1, "facts": [], "files": [file]
    });
    let native: Value = serde_json::from_str(
        &compiled
            .evaluate(snapshot.to_string(), "report".into())
            .unwrap(),
    )
    .unwrap();

    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 1, "files": [file], "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let workspace: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    for evaluation in [&native, &workspace["evaluation"]] {
        assert_eq!(
            evaluation["violations"][0]["clauseId"], "ordinary",
            "{evaluation}"
        );
        assert_eq!(
            evaluation["failures"][0]["clauseId"], "expanded",
            "{evaluation}"
        );
        assert_eq!(
            evaluation["failures"][0]["code"], "budget-exceeded",
            "{evaluation}"
        );
    }
}

#[test]
fn matching_stops_after_decision_or_witness_budget_in_native_and_workspace() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 100000, "maxWitnesses": 2 }
    });
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "query-cursor", "revision": 1,
        "mode": "steer",
        "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [{
            "id": "print", "violation": { "anchor": "witness.span", "code": "PRINT",
                "message": "found print" },
            "forbid": { "ast": { "language": "python",
                "rule": { "pattern": "print($ARG)" } } }
        }]
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap();
    let file = json!({
        "path": "sample.py", "language": "python",
        "source": "print(0)\n".repeat(5_000)
    });
    let snapshot = json!({
        "format": "rerail.repository-snapshot/v1", "revision": 1, "facts": [],
        "files": [file.clone()]
    });

    crate::code_rule_dsl::matcher::reset_roots_matched();
    compiled
        .evaluate(snapshot.to_string(), "decision".into())
        .unwrap();
    assert!(
        crate::code_rule_dsl::matcher::roots_matched() <= 1,
        "a decision must stop at its first matching root"
    );

    crate::code_rule_dsl::matcher::reset_roots_matched();
    compiled
        .evaluate(snapshot.to_string(), "report".into())
        .unwrap();
    assert!(
        crate::code_rule_dsl::matcher::roots_matched() <= 3,
        "the native evaluator must stop once its witness budget is exceeded"
    );

    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    crate::code_rule_dsl::matcher::reset_roots_matched();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 1, "files": [file], "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    task.compute().unwrap();
    assert!(
        crate::code_rule_dsl::matcher::roots_matched() <= 3,
        "the workspace evaluator must stop once its witness budget is exceeded"
    );
}

/// A violation with `suppress` spends no allowance on suppressed witnesses, yet both evaluators
/// stop building a regex capture's groups once the reported ones overflow the allowance: none of
/// these 50,000 is suppressed, so the second ends the enumeration.
#[test]
fn suppressible_each_stops_at_the_witness_overflow_in_native_and_workspace() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    capabilities["limits"]["maxWitnesses"] = json!(1);
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "bounded-each", "revision": 1,
        "mode": "steer",
        "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [{
            "id": "comment-characters",
            "violation": { "anchor": "capture.X", "code": "E", "message": "$X", "each": "X",
                "suppress": { "eslint": { "rules": ["no-warning-comments"] } } },
            "forbid": { "ast": { "language": "typescript",
                "rule": { "kind": "comment", "regex": "(?<X>a)" } } }
        }]
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap();
    let file = json!({
        "path": "a.ts", "language": "typescript",
        "source": format!("//{}\n", "a".repeat(50_000))
    });
    let snapshot = json!({
        "format": "rerail.repository-snapshot/v1", "revision": 1, "facts": [],
        "files": [file.clone()]
    });

    crate::code_rule_dsl::violations::reset_node_group_enumerations();
    let native: Value = serde_json::from_str(
        &compiled
            .evaluate(snapshot.to_string(), "report".into())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(native["failures"][0]["code"], "witness-limit", "{native}");
    assert_eq!(
        crate::code_rule_dsl::violations::node_group_enumerations(),
        2,
        "the native evaluator must stop at the overflow"
    );

    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    crate::code_rule_dsl::violations::reset_node_group_enumerations();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 1, "files": [file], "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let result: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    assert_eq!(
        result["evaluation"]["failures"][0]["code"], "witness-limit",
        "{result}"
    );
    assert_eq!(
        crate::code_rule_dsl::violations::node_group_enumerations(),
        2,
        "the workspace evaluator must stop at the overflow"
    );
    workspace.close();
}

#[test]
fn roots_past_max_visited_nodes_are_not_matched_in_native_and_workspace() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 1000, "maxWitnesses": 10000 }
    });
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "node-bound", "revision": 1, "mode": "steer",
        "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [{
            "id": "print", "violation": { "anchor": "witness.span", "code": "PRINT",
                "message": "found print" },
            "forbid": { "ast": { "language": "python",
                "rule": { "pattern": "print($ARG)" } } }
        }]
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap();
    // 2,000 calls of seven nodes each: only the first ~140 lie within the node bound, and none
    // past it may be matched, including the `print` at the end.
    let file = json!({
        "path": "sample.py", "language": "python",
        "source": format!("print(0)\n{}print(1)\n", "f(0)\n".repeat(2_000))
    });
    let snapshot = json!({
        "format": "rerail.repository-snapshot/v1", "revision": 1, "facts": [],
        "files": [file.clone()]
    });

    crate::code_rule_dsl::matcher::reset_roots_matched();
    let native: Value = serde_json::from_str(
        &compiled
            .evaluate(snapshot.to_string(), "report".into())
            .unwrap(),
    )
    .unwrap();
    let native_roots = crate::code_rule_dsl::matcher::roots_matched();

    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    crate::code_rule_dsl::matcher::reset_roots_matched();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 1, "files": [file], "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let workspace: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    let workspace_roots = crate::code_rule_dsl::matcher::roots_matched();
    assert!(native_roots <= 200, "native matched {native_roots} roots");
    assert!(
        workspace_roots <= 200,
        "workspace matched {workspace_roots} roots"
    );
    // Reaching the bound fails the clause for the file, with no violation, as before.
    for (evaluation, message) in [
        (
            &native,
            crate::code_rule_dsl::matcher::UtilLimit::Work.message(),
        ),
        (&workspace["evaluation"], "file AST work budget exceeded"),
    ] {
        assert_eq!(evaluation["violations"], json!([]), "{evaluation}");
        assert_eq!(
            evaluation["failures"].as_array().unwrap().len(),
            1,
            "{evaluation}"
        );
        assert_eq!(
            evaluation["failures"][0]["code"], "budget-exceeded",
            "{evaluation}"
        );
        assert_eq!(
            evaluation["failures"][0]["message"], message,
            "{evaluation}"
        );
    }
}

#[test]
fn workspace_visits_grow_with_the_file_by_max_visits_per_node() {
    // The file of `roots_past_max_visited_nodes_are_not_matched_in_native_and_workspace`, whose
    // ~14,000 nodes are past `maxVisitedNodes`: with `maxVisitsPerNode` the workspace allows a
    // file that many visits per node, so the clause reads it to the end instead of failing.
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 1000, "maxVisitsPerNode": 16, "maxWitnesses": 10000 }
    });
    let code_rule = json!({
        "format": "rerail.policy/v1", "id": "node-scaled", "revision": 1, "mode": "steer",
        "scope": { "include": ["**/*"], "exclude": [] },
        "clauses": [{
            "id": "print", "violation": { "anchor": "witness.span", "code": "PRINT",
                "message": "found print" },
            "forbid": { "ast": { "language": "python",
                "rule": { "pattern": "print($ARG)" } } }
        }]
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".into(),
        capabilities.to_string(),
    )
    .unwrap();
    let file = json!({
        "path": "sample.py", "language": "python",
        "source": format!("print(0)\n{}print(1)\n", "f(0)\n".repeat(2_000))
    });
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 1, "files": [file], "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let result: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    let evaluation = &result["evaluation"];
    assert_eq!(evaluation["failures"], json!([]), "{evaluation}");
    // Both prints, the one past the old bound included.
    assert_eq!(
        evaluation["violations"].as_array().unwrap().len(),
        2,
        "{evaluation}"
    );
}

#[test]
fn select_when_exists_leaf_decides_at_first_witness_in_native_and_workspace() {
    // An `exists` side only asks whether a witness exists: more matches than the witness budget
    // in a file, or a later root over a limit, must not make the condition unknown.
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let capabilities = json!({
        "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
            "languages": { "python": "tree-sitter-python@0.23" } },
        "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
            "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
            "maxVisitedNodes": 100000, "maxWitnesses": 2 }
    });
    let gate = |exists: Value| {
        json!({
            "format": "rerail.policy/v1", "id": "exists-gate", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{
                "id": "gate", "violation": { "anchor": "witness.span", "code": "G",
                    "message": "gated" },
                "forbid": { "select_when": {
                    "select": { "regex": { "source": "file.path", "pattern": "sample\\.py$" } },
                    "exists": exists,
                } }
            }]
        })
    };
    let file = json!({
        "path": "sample.py", "language": "python", "source": "print(0)\n".repeat(5)
    });
    for exists in [
        json!({ "ast": { "language": "python", "rule": { "pattern": "print($ARG)" } } }),
        json!({ "regex": { "source": "file.content", "pattern": "print" } }),
    ] {
        let compiled = CodeRuleDslRuntime::compile(
            gate(exists.clone()).to_string(),
            "json".into(),
            capabilities.to_string(),
        )
        .unwrap();
        let snapshot = json!({
            "format": "rerail.repository-snapshot/v1", "revision": 1, "facts": [],
            "files": [file.clone()]
        });
        let native: Value = serde_json::from_str(
            &compiled
                .evaluate(snapshot.to_string(), "report".into())
                .unwrap(),
        )
        .unwrap();
        let located = |evaluation: &Value| {
            evaluation["violations"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|violation| {
                    (
                        violation["code"].clone(),
                        violation["path"].clone(),
                        violation["span"].clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(native["failures"], json!([]), "{exists}: {native}");
        assert_eq!(located(&native).len(), 1, "{exists}: {native}");

        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.add_code_rule(&compiled, None).unwrap();
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": 1, "files": [file.clone()], "removed": [] })
                .to_string(),
            request_buffer: None,
            request_direct: None,
        };
        let applied: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
        workspace.close();
        let evaluation = &applied["evaluation"];
        assert_eq!(evaluation["failures"], json!([]), "{exists}: {evaluation}");
        assert_eq!(located(evaluation), located(&native), "{exists}");
    }
}

#[test]
fn line_transforms_build_one_line_table_per_file_in_both_evaluators() {
    const MATCHES: usize = 1_000;
    let source = (0..MATCHES)
        .map(|index| format!("f({index});\n"))
        .collect::<String>();
    let clauses = || {
        vec![reported_clause(
            "lines",
            json!({ "ast": {
                "language": "typescript",
                "rule": { "pattern": "f($A)" },
                "transform": { "L": { "line": { "source": "$A" } } }
            } }),
            json!({ "anchor": "witness.span", "code": "LINE", "message": "line $L" }),
        )]
    };
    let files = json!([{ "language": "typescript", "path": "a.ts", "source": source }]);

    crate::code_rule_dsl::matcher::reset_line_table_constructions();
    let native = stateless(clauses(), files.clone());
    assert_eq!(native["violations"].as_array().map(Vec::len), Some(MATCHES));
    assert_eq!(
        crate::code_rule_dsl::matcher::line_table_constructions(),
        1,
        "stateless evaluation must retain one table for all transforms in the file"
    );

    crate::code_rule_dsl::matcher::reset_line_table_constructions();
    let workspace = apply_clauses(clauses(), files);
    assert_eq!(
        workspace["violations"].as_array().map(Vec::len),
        Some(MATCHES)
    );
    assert_eq!(
        crate::code_rule_dsl::matcher::line_table_constructions(),
        1,
        "workspace evaluation must retain one table for all transforms in the file"
    );
}

#[test]
fn nth_child_sibling_tables_are_built_once_per_file_across_clauses() {
    const ARGUMENTS: usize = 500;
    const CLAUSES: usize = 4;
    let source = format!(
        "f({});\n",
        (0..ARGUMENTS)
            .map(|index| format!("a{index}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let clauses = (0..CLAUSES)
        .map(|index| {
            clause(
                &format!("nth-{index}"),
                json!({ "kind": "identifier", "nthChild": format!("{}n+1", index + 1) }),
            )
        })
        .collect::<Vec<_>>();
    crate::code_rule_dsl::matcher::reset_named_sibling_visits();
    let evaluation = apply_clauses(
        clauses,
        json!([{ "language": "typescript", "path": "a.ts", "source": source }]),
    );
    assert_eq!(evaluation["failures"], json!([]), "{evaluation}");
    // The callee `f` is the first of its parent's two named children: every clause reports it.
    let expected = (1..=CLAUSES)
        .map(|step| ARGUMENTS.div_ceil(step) + 1)
        .sum::<usize>();
    assert_eq!(
        evaluation["violations"].as_array().map(Vec::len),
        Some(expected)
    );
    assert!(
        crate::code_rule_dsl::matcher::named_sibling_visits() <= ARGUMENTS + 8,
        "each parent's named children are listed once per file, not once per check: {} visits",
        crate::code_rule_dsl::matcher::named_sibling_visits()
    );
}

#[test]
fn resolves_builds_one_table_per_file_revision_for_every_clause() {
    let clauses = || {
        vec![
            clause(
                "alert",
                json!({ "kind": "identifier", "resolves": { "global": ["alert"] } }),
            ),
            clause(
                "confirm",
                json!({ "kind": "identifier", "resolves": { "global": ["confirm"] } }),
            ),
        ]
    };
    let files =
        |source: &str| json!([{ "language": "typescript", "path": "a.ts", "source": source }]);
    let first = "alert(1);\nfunction f(confirm) { confirm(2); }\nconfirm(3);\n";
    let second = "alert(1);\nfunction f(alert) { alert(2); }\nconfirm(3);\n";
    crate::code_rule_dsl::matcher::TABLE_BUILDS.set(0);
    let evaluations = apply_revisions(clauses(), vec![files(first), files(second)]);
    assert_eq!(
        crate::code_rule_dsl::matcher::TABLE_BUILDS.get(),
        2,
        "one table per source revision, shared by both checks"
    );
    for (source, evaluation) in [first, second].into_iter().zip(&evaluations) {
        let fresh = stateless(clauses(), files(source));
        assert_eq!(evaluation["violations"], fresh["violations"], "{source:?}");
        assert_eq!(
            evaluation["violations"].as_array().map(Vec::len),
            Some(2),
            "{evaluation}"
        );
    }
}

#[test]
fn layout_reads_a_taped_file_s_lines_like_stateless() {
    let clauses = || {
        vec![
            clause(
                "one-line",
                json!({ "kind": "if_statement", "layout": { "span": { "eq": 0 } } }),
            ),
            clause(
                "first",
                json!({ "kind": "comment", "layout": { "start": { "eq": 0 } } }),
            ),
            clause(
                "grouped",
                json!({ "kind": "if_statement", "layout": {
                    "before": { "to": "sibling", "blank": { "min": 1 } },
                    "after": { "to": "token", "edge": true, "breaks": { "min": 1 } } } }),
            ),
        ]
    };
    let source = "// head\nif (a) b();\nif (c)\n  d();\r\n\r\nif (e) f(`\u{2028}`);\n";
    let files = json!([{ "language": "typescript", "path": "a.ts", "source": source }]);
    let incremental = apply_clauses(clauses(), files.clone());
    let fresh = stateless(clauses(), files);
    assert_eq!(incremental["violations"], fresh["violations"]);
    assert_eq!(
        incremental["violations"].as_array().map(Vec::len),
        Some(3),
        "{incremental}"
    );
}

#[test]
fn suppression_removes_the_same_violations_as_stateless() {
    let clauses = || {
        let mut named = clause("names", json!({ "kind": "identifier" }));
        named["violation"]["suppress"] = json!({ "eslint": { "rules": ["names"] } });
        vec![named, clause("all-names", json!({ "kind": "identifier" }))]
    };
    let revisions = [
        "a; // eslint-disable-line\n/* eslint-disable names */\nb;\n/* eslint-enable */\nc;\n",
        "a;\n/* eslint-disable names */\nb;\n/* eslint-enable */\nc; // eslint-disable-line names\n",
    ];
    let files =
        |source: &str| json!([{ "language": "typescript", "path": "a.ts", "source": source }]);
    let incremental = apply_revisions(
        clauses(),
        revisions.iter().map(|source| files(source)).collect(),
    );
    for (source, evaluation) in revisions.iter().zip(&incremental) {
        let fresh = stateless(clauses(), files(source));
        assert_eq!(evaluation["violations"], fresh["violations"], "{source:?}");
        let named = evaluation["violations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|violation| violation["clauseId"] == "names")
            .count();
        assert_eq!(named, 1, "{evaluation}");
    }
}

#[test]
fn noqa_suppression_reads_a_taped_file_like_stateless() {
    let clauses = || {
        vec![json!({
            "id": "imports",
            "forbid": { "ast": { "language": "python", "rule": { "kind": "dotted_name", "inside": { "kind": "import_from_statement" } } } },
            "violation": { "anchor": "witness.span", "code": "F401", "message": "unused", "suppress": { "noqa": { "codes": ["F401"], "parent": "statement" } } },
        })]
    };
    let source = "#!/usr/bin/env python\n# noqa: F401\nfrom a import (  # noqa: F401\n    b,\n)\nfrom c import d\ns = '''x\ny'''  # noqa\nfrom e import f  # NOQA: F401\n";
    let files = json!([{ "language": "python", "path": "a.py", "source": source }]);
    let incremental = apply_clauses(clauses(), files.clone());
    let fresh = stateless(clauses(), files);
    assert_eq!(incremental["violations"], fresh["violations"]);
    let texts = incremental["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|violation| {
            let span = &violation["span"];
            &source[span["startByte"].as_u64().unwrap() as usize
                ..span["endByte"].as_u64().unwrap() as usize]
        })
        .collect::<Vec<_>>();
    assert_eq!(texts, ["c", "d"], "{incremental}");
}

/// M12a's `layout` gap to a capture, beside the fix round's capture ordering: the capture is
/// bound by a sibling relation, a utility or a `parse` export (directly or through a
/// utility), and the gap is read under `not`, which compiles before those siblings and so
/// runs after them only by what it reads.

#[test]
fn a_negated_gap_to_a_capture_runs_after_the_term_that_binds_it() {
    let callee = json!({ "stopBy": "end", "kind": "call_expression",
        "has": { "field": "function", "pattern": "$C" } });
    let gap = |to: &str| json!({ "layout": { "before": { "to": to, "breaks": { "eq": 0 } } } });
    // `b` a line after `f`, and on `g`'s line.
    let source = "f(a,\n  b);\ng(a, b);\n";
    let messages = |violations: &[Found]| {
        violations
            .iter()
            .map(|(_, message, _)| message.clone())
            .collect::<Vec<_>>()
    };
    let violations = agreed_violations(
        || {
            let utils = json!({ "callee": { "inside": callee } });
            [
                (
                    "relation",
                    json!({ "not": gap("capture.C"), "inside": callee }),
                    None,
                ),
                (
                    "utility",
                    json!({ "not": gap("capture.C"), "matches": "callee" }),
                    Some(&utils),
                ),
                (
                    "positive",
                    json!({ "layout": gap("capture.C")["layout"], "matches": "callee" }),
                    Some(&utils),
                ),
            ]
            .into_iter()
            .map(|(id, mut rule, utils)| {
                rule["kind"] = json!("identifier");
                rule["pattern"] = json!("$B");
                rule["regex"] = json!("^b$");
                let mut clause = clause(id, rule);
                clause["forbid"]["ast"]["transform"] =
                    json!({ "L": { "line": { "source": "$B" } } });
                if let Some(utils) = utils {
                    clause["forbid"]["ast"]["utils"] = utils.clone();
                }
                clause["violation"]["message"] = json!(format!("{id} on ${{L}}"));
                clause
            })
            .collect()
        },
        "typescript",
        source,
    );
    let mut found = messages(&violations);
    found.sort();
    assert_eq!(found, ["positive on 3", "relation on 2", "utility on 2"]);
    // A `/* global */` item a `parse` exports, directly or through a utility: the statement
    // on the comment's line is refused, the one on the next reported.
    let directive = json!({ "kind": "comment", "parse": { "grammar": "eslintDirective",
        "value": { "regex": "^globals?$" }, "has": { "kind": "item", "capture": "N" } } });
    let source = "/* global a */\nx;\n/* global b */ y;\n";
    let violations = agreed_violations(
        || {
            let direct = clause(
                "direct",
                json!({ "kind": "expression_statement",
                "not": gap("capture.N"), "follows": directive.clone() }),
            );
            let mut through = clause(
                "through",
                json!({ "kind": "expression_statement",
                "not": gap("capture.N"), "follows": { "matches": "directive" } }),
            );
            through["forbid"]["ast"]["utils"] = json!({ "directive": directive.clone() });
            through["violation"]["anchor"] = json!("capture.N");
            vec![direct, through]
        },
        "typescript",
        source,
    );
    let mut found = texts(&violations);
    found.sort();
    assert_eq!(found, ["a", "x;"]);
}

/// An `eslintDirective` capture a utility's `parse` exports (the fix round's nested export):
/// `each` reports every item of the comment, anchored, messaged and fixed at its own range.

#[test]
fn each_and_a_fix_range_read_a_directive_item_exported_through_a_utility() {
    let clauses = || {
        vec![reported_clause(
            "globals",
            json!({ "ast": { "language": "typescript", "rule": { "matches": "items" },
                "utils": { "items": { "kind": "comment", "parse": { "grammar": "eslintDirective",
                    "value": { "regex": "^globals?$" }, "has": { "kind": "item", "capture": "N" } } } } } }),
            json!({ "anchor": "capture.N", "code": "G", "each": "N",
                    "message": "global ${N}!", "fix": { "template": "${N}2", "range": "capture.N" } }),
        )]
    };
    let source = "/* global a, b:off */\n/* eslint-disable c */\nx;\n";
    let fixed = |name: &str| {
        (
            name.to_owned(),
            format!("global {name}!"),
            Some((name.to_owned(), format!("{name}2"))),
        )
    };
    assert_eq!(
        agreed_violations(clauses, "typescript", source),
        [fixed("a"), fixed("b")]
    );
}

/// `suppress` with `each`: each enumerated violation is placed at its own anchor, so a directive
/// on one element's line suppresses that element's violation only.

#[test]
fn suppression_applies_to_each_enumerated_violation_by_its_own_line() {
    let clauses = |suppress: bool| {
        move || {
            let mut violation =
                json!({ "anchor": "capture.X", "code": "N", "each": "X", "message": "$X" });
            if suppress {
                violation["suppress"] = json!({ "eslint": { "rules": ["names"] } });
            }
            vec![reported_clause(
                "names",
                json!({ "ast": { "language": "typescript",
                    "rule": { "kind": "array", "has": { "kind": "identifier", "pattern": "$X" } } } }),
                violation,
            )]
        }
    };
    let source = "const v = [\n  a,\n  // eslint-disable-next-line names\n  b,\n  c, // eslint-disable-line names\n  d,\n];\n";
    assert_eq!(
        texts(&agreed_violations(clauses(false), "typescript", source)),
        ["a", "b", "c", "d"]
    );
    assert_eq!(
        texts(&agreed_violations(clauses(true), "typescript", source)),
        ["a", "d"]
    );
}

/// `suppress.noqa` with `parent: "statement"` on a violation anchored at a capture a `parse`
/// exports: the anchor is the capture's host range, and the directive where the statement
/// starts suppresses it.

#[test]
fn noqa_reads_the_statement_around_an_exported_capture() {
    let clauses = |parent: bool| {
        move || {
            let mut noqa = json!({ "codes": ["F821"] });
            if parent {
                noqa["parent"] = json!("statement");
            }
            vec![reported_clause(
                "names",
                json!({ "ast": { "language": "python", "rule": { "kind": "string",
                    "parse": { "grammar": "annotation", "has": { "stopBy": "end", "kind": "identifier",
                        "regex": "^(foo|bar|baz)$", "capture": "N" } } } } }),
                json!({ "anchor": "capture.N", "code": "F821", "message": "$N",
                        "suppress": { "noqa": noqa } }),
            )]
        }
    };
    let source = "x: (  # noqa: F821\n    \"list[foo]\"\n) = 1\ny: \"bar\"  # noqa: F401\nz: \"baz\"  # noqa\n";
    // Without `parent` the directive where the statement starts is not the violation's line.
    assert_eq!(
        texts(&agreed_violations(clauses(false), "python", source)),
        ["foo", "bar"]
    );
    assert_eq!(
        texts(&agreed_violations(clauses(true), "python", source)),
        ["bar"]
    );
}

/// Compiles `clauses`, returning the refusal's reason.
fn refused(clauses: Vec<Value>) -> String {
    let document = json!({ "format": "rerail.policy/v1", "id": "strict", "revision": 1, "mode": "steer", "clauses": clauses });
    CodeRuleDslRuntime::compile(
        document.to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .err()
    .expect("compile is refused")
    .reason
}

/// M12c's `all` order check reads M12a's `layout`: an item holding a gap to `capture.C`, bare
/// or negated, reads `$C`, so a later item that alone binds it is refused, naming the reorder.
/// Written in order it compiles and matches; in a parameterized utility a gap to a parameter
/// the caller binds is admitted.

#[test]
fn an_all_item_holding_a_gap_reads_its_capture() {
    let callee = json!({ "stopBy": "end", "kind": "call_expression",
        "has": { "field": "function", "pattern": "$C" } });
    let gap = json!({ "layout": { "before": { "to": "capture.C", "breaks": { "eq": 0 } } } });
    let rule =
        |all: Value| json!({ "kind": "identifier", "pattern": "$B", "regex": "^b$", "all": all });
    for first in [gap.clone(), json!({ "not": gap.clone() })] {
        assert_eq!(
            refused(vec![clause(
                "order",
                rule(json!([first, { "inside": callee.clone() }]))
            )]),
            "/clauses/0/forbid/ast/rule/all/0 reads `$C`, which only /clauses/0/forbid/ast/rule/all/1 binds, \
             after it: an `all` runs its items in the order written; move all/1 before all/0"
        );
    }
    let source = "f(a,\n  b);\ng(a, b);\n";
    let lines = |clauses: Vec<Value>| {
        let mut found = agreed_violations(
            || {
                clauses
                    .iter()
                    .cloned()
                    .map(|mut clause| {
                        clause["forbid"]["ast"]["transform"] =
                            json!({ "L": { "line": { "source": "$B" } } });
                        let id = clause["id"].as_str().unwrap().to_owned();
                        clause["violation"]["message"] = json!(format!("{id} on ${{L}}"));
                        clause
                    })
                    .collect()
            },
            "typescript",
            source,
        )
        .into_iter()
        .map(|(_, message, _)| message)
        .collect::<Vec<_>>();
        found.sort();
        found
    };
    // In a frame, a parameter excuses the order (a caller may pass it bound): the call reads
    // the caller's capture before its `all` binds it, so it runs after the relation that does.
    let mut framed = clause(
        "framed",
        json!({ "kind": "identifier", "pattern": "$B", "regex": "^b$",
            "inside": callee.clone(), "matches": { "near": { "P": "$C" } } }),
    );
    framed["forbid"]["ast"]["utils"] = json!({ "near": { "params": ["P"], "rule": { "all": [
        { "layout": { "before": { "to": "capture.P", "breaks": { "eq": 0 } } } },
        { "inside": { "stopBy": "end", "kind": "call_expression",
            "has": { "field": "function", "pattern": "$P" } } },
    ] } } });
    assert_eq!(
        lines(vec![
            clause(
                "ordered",
                rule(json!([{ "inside": callee.clone() }, gap.clone()]))
            ),
            clause(
                "negated",
                rule(json!([{ "inside": callee.clone() }, { "not": gap.clone() }]))
            ),
            framed,
        ]),
        ["framed on 3", "negated on 2", "ordered on 3"]
    );
}

/// A parameterized utility's `layout` gap to a parameter reads the node the caller's capture
/// holds, bare or negated; the call runs after the sibling relation that binds the capture.

#[test]
fn a_gap_to_a_parameter_reads_the_callers_capture() {
    let source = "f(a,\n  b);\ng(a, b);\n";
    let violations = agreed_violations(
        || {
            [
                ("positive", json!({ "matches": { "near": { "P": "$C" } } })),
                ("negated", json!({ "not": { "matches": { "near": { "P": "$C" } } } })),
            ]
            .into_iter()
            .map(|(id, mut rule)| {
                rule["kind"] = json!("identifier");
                rule["pattern"] = json!("$B");
                rule["regex"] = json!("^b$");
                rule["inside"] = json!({ "stopBy": "end", "kind": "call_expression",
                    "has": { "field": "function", "pattern": "$C" } });
                let mut clause = clause(id, rule);
                clause["forbid"]["ast"]["utils"] = json!({ "near": { "params": ["P"],
                    "rule": { "layout": { "before": { "to": "capture.P", "breaks": { "eq": 0 } } } } } });
                clause["forbid"]["ast"]["transform"] = json!({ "L": { "line": { "source": "$B" } } });
                clause["violation"]["message"] = json!(format!("{id} on ${{L}} after $C"));
                clause
            })
            .collect()
        },
        "typescript",
        source,
    );
    let mut messages = violations
        .into_iter()
        .map(|(_, message, _)| message)
        .collect::<Vec<_>>();
    messages.sort();
    assert_eq!(messages, ["negated on 2 after f", "positive on 3 after g"]);
}

/// A parameterized utility whose `parse` captures a parameter returns the parsed item under
/// the caller's name: `each`, the anchor, `${NAME}` and `fix.range` read it at its own range.

#[test]
fn a_parameter_a_parse_captures_comes_back_for_each_and_a_fix_range() {
    let clauses = || {
        vec![reported_clause(
            "globals",
            json!({ "ast": { "language": "typescript", "rule": { "matches": { "items": { "N": "$G" } } },
                "utils": { "items": { "params": ["N"], "rule": { "kind": "comment", "parse": {
                    "grammar": "eslintDirective", "value": { "regex": "^globals?$" },
                    "has": { "kind": "item", "capture": "N" } } } } } } }),
            json!({ "anchor": "capture.G", "code": "G", "each": "G",
                    "message": "global ${G}!", "fix": { "template": "${G}2", "range": "capture.G" } }),
        )]
    };
    let source = "/* global a, b:off */\n/* eslint-disable c */\nx;\n";
    let fixed = |name: &str| {
        (
            name.to_owned(),
            format!("global {name}!"),
            Some((name.to_owned(), format!("{name}2"))),
        )
    };
    assert_eq!(
        agreed_violations(clauses, "typescript", source),
        [fixed("a"), fixed("b")]
    );
}

/// Transforms read a capture a call returns: its text, its `value`, its line and count, and
/// their `default` where the call's branch did not run.

#[test]
fn transforms_read_what_a_call_returns_with_value_and_default() {
    let clauses = || {
        vec![reported_clause(
            "args",
            json!({ "ast": { "language": "typescript",
                "rule": { "kind": "call_expression", "any": [
                    { "pattern": "f()" },
                    { "pattern": "f($$$)", "matches": { "arg": { "X": "$A" } } },
                ] },
                "utils": { "arg": { "params": ["X"], "rule": {
                    "has": { "field": "arguments", "has": { "kind": "number", "pattern": "$X" } } } } },
                "transform": {
                    "V": { "substring": { "source": "$A", "value": true, "default": "none" } },
                    "T": { "replace": { "source": "$A", "replace": "x", "by": "X", "default": "-" } },
                    "L": { "line": { "source": "$A", "default": "?" } },
                } } }),
            json!({ "anchor": "witness.span", "code": "A", "message": "${V} ${T} on ${L}" }),
        )]
    };
    let source = "f(\n  0x10);\nf();\n";
    let messages = agreed_violations(clauses, "typescript", source)
        .into_iter()
        .map(|(_, message, _)| message)
        .collect::<Vec<_>>();
    assert_eq!(messages, ["16 0X10 on 2", "none - on ?"]);
}

/// `suppress` with `each` over a capture a call returns: each enumerated violation is placed at
/// its own anchor, so a directive on one element's line suppresses that element only.

#[test]
fn suppression_applies_to_each_capture_a_call_returns() {
    let clauses = |suppress: bool| {
        move || {
            let mut violation =
                json!({ "anchor": "capture.X", "code": "N", "each": "X", "message": "$X" });
            if suppress {
                violation["suppress"] = json!({ "eslint": { "rules": ["names"] } });
            }
            vec![reported_clause(
                "names",
                json!({ "ast": { "language": "typescript",
                    "rule": { "kind": "array", "matches": { "element": { "E": "$X" } } },
                    "utils": { "element": { "params": ["E"],
                        "rule": { "has": { "kind": "identifier", "pattern": "$E" } } } } } }),
                violation,
            )]
        }
    };
    let source = "const v = [\n  a,\n  // eslint-disable-next-line names\n  b,\n  c, // eslint-disable-line names\n  d,\n];\n";
    assert_eq!(
        texts(&agreed_violations(clauses(false), "typescript", source)),
        ["a", "b", "c", "d"]
    );
    assert_eq!(
        texts(&agreed_violations(clauses(true), "typescript", source)),
        ["a", "d"]
    );
}

/// `$option` inside M12a's keys and `languageOptions`: a `suppress` list, a `layout` count, a
/// transform's `default` and `by`, and a query's globals, each resolved per configuration.

#[test]
fn options_resolve_inside_suppress_layout_transform_and_language_options() {
    let clauses = |config: Value| {
        move || {
            let options = json!({ "id": "options", "define": {
                "options": {
                    "disable": { "type": "array", "default": ["names"] },
                    "maxBreaks": { "type": "integer", "default": 0 },
                    "missing": { "type": "string", "default": "none" },
                    "browser": { "type": "boolean", "default": false },
                },
                "config": config.clone() } });
            let names = reported_clause(
                "names",
                json!({ "ast": { "language": "typescript",
                    "rule": { "kind": "identifier", "pattern": "$X", "regex": "^[a-d]$",
                        "inside": { "kind": "array" },
                        "any": [{ "pattern": "$Y", "regex": "^never$" }, { "kind": "identifier" }],
                        "layout": { "before": { "to": "token", "breaks": { "max": { "$option": "maxBreaks" } } } } },
                    "transform": { "T": { "replace": { "source": "$X", "replace": "^", "by": { "$option": "missing" } } },
                        "D": { "substring": { "source": "$Y", "default": { "$option": "missing" } } } } } }),
                json!({ "anchor": "capture.X", "code": "N", "message": "$X ${T} ${D}",
                        "suppress": { "eslint": { "rules": { "$option": "disable" } } } }),
            );
            let window = reported_clause(
                "window",
                json!({ "ast": { "language": "typescript",
                    "rule": { "kind": "identifier", "regex": "^window$", "resolves": { "global": ["window"] } },
                    "languageOptions": { "globals": { "$option": "browser",
                        "cases": { "true": { "window": "readonly" } }, "default": {} } } } }),
                json!({ "anchor": "witness.span", "code": "W", "message": "global" }),
            );
            vec![options, names, window]
        }
    };
    let source =
        "const v = [a,\n  // eslint-disable-next-line names\n  b,\n\n  c, d];\nwindow.x;\n";
    let messages = |config: Value| {
        agreed_violations(clauses(config), "typescript", source)
            .into_iter()
            .map(|(_, message, _)| message)
            .collect::<Vec<_>>()
    };
    // Defaults: `b` suppressed; `c` two breaks after its token; `window` undeclared.
    assert_eq!(messages(json!({})), ["a nonea none", "d noned none"]);
    assert_eq!(
        messages(json!({ "disable": ["other"], "maxBreaks": 2, "missing": "-", "browser": true })),
        ["a -a -", "b -b -", "c -c -", "d -d -", "global"]
    );
}

/// `file.start`: a file-level violation at byte 0 where the module starts at a later token, and
/// `suppress.noqa` placing it there (a directive on the module's first line is not on line 1).

#[test]
fn a_file_start_anchor_is_byte_zero_on_both_paths() {
    let clauses = || {
        vec![reported_clause(
            "inp",
            json!({ "ast": { "language": "python", "rule": { "kind": "module" } } }),
            json!({ "anchor": "file.start", "code": "INP001", "message": "m",
                    "suppress": { "noqa": { "codes": ["INP001"] } } }),
        )]
    };
    for (source, kept) in [
        ("\n\nimport os\n", 1),
        ("\x0cimport os\n", 1),
        ("\nimport os  # noqa: INP001\n", 1),
        ("import os  # noqa: INP001\n", 0),
    ] {
        let files = json!([{ "language": "python", "path": "a.py", "source": source }]);
        let incremental = apply_clauses(clauses(), files.clone());
        assert_eq!(
            incremental["violations"],
            stateless(clauses(), files)["violations"]
        );
        let violations = incremental["violations"].as_array().expect("findings");
        assert_eq!(violations.len(), kept, "{source:?}");
        for violation in violations {
            assert_eq!(
                violation["span"],
                json!({ "startByte": 0, "endByte": 0, "startLine": 0, "startColumn": 0, "endLine": 0, "endColumn": 0 })
            );
        }
    }
}

/// `host` inside a directive's item, with `each`: every item a declaration of the file names,
/// an item's `resolves` as a configured global, and `compare` `name` against an item, on both
/// paths; a revision that declares another name changes what the host rule sees.

#[test]
fn host_rules_and_item_globals_agree_across_paths_and_revisions() {
    let clauses = || {
        let declared = json!({ "inside": { "kind": "program", "has": { "stopBy": "end",
            "kind": "variable_declarator", "has": { "field": "name", "pattern": "$V" },
            "compare": { "left": "V", "op": "eq", "right": "N", "text": "name" } } } });
        vec![reported_clause(
            "items",
            json!({ "ast": { "language": "typescript",
                "languageOptions": { "sourceType": "script", "globals": { "conf": "readonly" } },
                "rule": { "kind": "comment", "parse": { "grammar": "eslintDirective",
                    "value": { "regex": "^globals?$" }, "has": { "kind": "item", "capture": "N",
                        "any": [{ "resolves": { "global": true } }, { "host": declared }] } } } } }),
            json!({ "anchor": "capture.N", "code": "I", "each": "N",
                    "message": "'$N' is already defined." }),
        )]
    };
    let source = "/*global b, conf, a, c*/ var a, \\u0063;\n";
    assert_eq!(
        texts(&agreed_violations(clauses, "typescript", source)),
        ["conf", "a", "c"]
    );
    let revisions = apply_revisions(
        clauses(),
        vec![
            json!([{ "language": "typescript", "path": "a.ts", "source": source }]),
            json!([{ "language": "typescript", "path": "a.ts", "source": "/*global b, conf, a, c*/ var b;\n" }]),
        ],
    );
    let messages = |evaluation: &Value| {
        evaluation["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| violation["message"].as_str().expect("message").to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        messages(&revisions[1]),
        ["'b' is already defined.", "'conf' is already defined."]
    );
}

#[test]
fn index_labels_a_basename_by_its_longest_listed_suffix_shorter_than_the_basename() {
    let workspace = CodeRuleDslWorkspace::new();
    let mut task = super::index::WorkspaceIndexTask::for_test(
        StdArc::clone(&workspace.state),
        StdArc::clone(&workspace.closed),
        u32::MAX,
    );
    task.options.source_languages = [(".php", "php"), (".blade.php", "blade")]
        .into_iter()
        .map(|(extension, label)| (extension.to_owned(), label.to_owned()))
        .collect();
    let label = |path: &str| task.source_language(path);
    assert_eq!(label("views/x.blade.php").as_deref(), Some("blade"));
    assert_eq!(label("views/X.Blade.PHP").as_deref(), Some("blade"));
    assert_eq!(label("app/Kernel.php").as_deref(), Some("php"));
    // Suffix-only basenames have no extension, as the host's `languageOfPath` reads them.
    assert_eq!(label("views/.blade.php").as_deref(), Some("php"));
    assert_eq!(label("app/.php"), None);
}

#[test]
fn reads_package_manifests_only_with_an_ast_clause_in_a_manifest_family() {
    let ast = |language: &str, grammar: &str, kind: &str| {
        let capabilities = json!({
            "ast": { "engine": "ast-grep-core", "engineVersion": "0.45",
                "languages": { language: grammar } },
            "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
                "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
                "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
        });
        let code_rule = json!({
            "format": "rerail.policy/v1", "id": format!("{language}-call"), "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{
                "id": "call", "violation": { "anchor": "witness.span", "code": "CALL",
                    "message": "found call" },
                "forbid": { "ast": { "language": language, "rule": { "kind": kind } } }
            }]
        });
        CodeRuleDslRuntime::compile(
            code_rule.to_string(),
            "json".into(),
            capabilities.to_string(),
        )
        .unwrap()
    };
    assert!(!CodeRuleDslWorkspace::new().reads_package_manifests());
    // A regex clause reads no file's facts, and Python files read no `package.json`.
    assert!(!workspace(&["**/*"], false).reads_package_manifests());
    let mut python = CodeRuleDslWorkspace::new();
    python
        .add_code_rule(&ast("python", "tree-sitter-python@0.23", "call"), None)
        .unwrap();
    assert!(!python.reads_package_manifests());
    // An ECMA AST clause does, beside the Python one.
    assert!(workspace(&["**/*"], true).reads_package_manifests());
    python
        .add_code_rule(
            &ast(
                "javascript",
                "tree-sitter-javascript@0.23",
                "call_expression",
            ),
            None,
        )
        .unwrap();
    assert!(python.reads_package_manifests());
}

#[test]
fn positional_index_lives_with_the_workspace_across_applies_and_deletions() {
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&tape_test_code_rule(), None)
        .unwrap();
    let file = |path: &str, source: &str| json!({"path":path,"language":"python","source":source});
    tape_test_apply(
        &workspace,
        1,
        vec![file("a.py", "print('a')\n"), file("b.py", "print('b')\n")],
    );
    let census = || workspace.state.lock().unwrap().anchor_corpus.census();
    let first = census();
    assert_eq!(first.1, 2);
    // Neither an empty apply nor resending an identical source rebuilds postings.
    tape_test_apply(&workspace, 2, vec![]);
    assert_eq!(census(), first);
    tape_test_apply(&workspace, 3, vec![file("a.py", "print('a')\n")]);
    assert_eq!(census(), first);
    // A retry of an interrupted rule is still the same indexed source revision.
    workspace
        .state
        .lock()
        .unwrap()
        .files
        .get_mut("a.py")
        .unwrap()
        .transient_failure = true;
    tape_test_apply(&workspace, 4, vec![file("a.py", "print('a')\n")]);
    assert_eq!(census(), first);
    tape_test_apply(&workspace, 5, vec![file("a.py", "print('edited')\n")]);
    assert_eq!(census().1, 2);
    assert!(census().0 > first.0);
    let mut remove = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({"revision":6,"files":[],"removed":["a.py"]}).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    remove.compute().unwrap();
    assert_eq!(census().1, 1);
    // A new workspace has no rows from the previous workspace's SQL corpus.
    let independent = CodeRuleDslWorkspace::new();
    assert_eq!(
        independent.state.lock().unwrap().anchor_corpus.census(),
        (0, 0)
    );
    independent.close();
    workspace.close();
}
