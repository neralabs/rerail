    use super::super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask};
    use super::super::storage::StorageContext;
    use super::super::tests::{WORKSPACE_TEST_SLOT, result_page};
    use crate::code_rule_dsl::CodeRuleDslRuntime;
    use crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION;
    use crate::code_rule_dsl::test_support::{CAPABILITIES, violation};
    use crate::content_store::{ContentStore, GitTree, HashKey};
    use napi::Task;
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// A workspace over a fresh registration of `root` in `store`, holding `code_rule`.
    fn stored_workspace(
        store: &Arc<ContentStore>,
        root: &str,
        code_rule: &str,
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
                i64::from(TAPE_FORMAT_VERSION),
            )
            .unwrap();
        let owner = format!("coverage-test:{root}:{}", std::process::id());
        let workspace_id = store
            .open_workspace(root, "repository", &owner, snapshot, 1, i64::MAX)
            .unwrap();
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.state.lock().unwrap().storage = Some(StorageContext {
            store: Arc::clone(store),
            repository: None,
            repository_prefix: String::new(),
            workspace_id,
            snapshot_id: snapshot,
            owner,
            secret_names: Default::default(),
            _maintenance: None,
        });
        let compiled =
            CodeRuleDslRuntime::compile(code_rule.to_owned(), "json".into(), CAPABILITIES.into())
                .unwrap();
        workspace.add_code_rule(&compiled, None).unwrap();
        workspace
    }

    fn apply(workspace: &CodeRuleDslWorkspace, files: &Value) -> Value {
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": 1, "files": files, "removed": [] }).to_string(),
            request_buffer: None,
            request_direct: None,
        };
        serde_json::from_str(&task.compute().unwrap()).unwrap()
    }

    #[test]
    fn coverage_reports_a_broken_files_partial_clause_cold_warm_and_paged() {
        let _slot = WORKSPACE_TEST_SLOT.lock();
        let directory = std::env::temp_dir().join(format!(
            "rerail-coverage-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let store = Arc::new(ContentStore::open(&directory.join("content.sqlite")).unwrap());
        let ast = |id: &str, language: &str, rule: Value| json!({ "id": id, "forbid": { "ast": { "language": language, "rule": rule } }, "violation": violation() });
        let code_rule = json!({
            "format": "rerail.policy/v1", "id": "covered", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [
                ast("print", "python", json!({ "pattern": "print($A)" })),
                ast("clean", "typescript", json!({ "kind": "debugger_statement" })),
            ],
        })
        .to_string();
        let files =
            json!([{ "path": "broken.py", "language": "python", "source": "print(3)\ndef (:\n" }]);
        let expected = json!([
            { "ruleId": "covered", "clauseId": "print", "status": "partial", "reasons": [
                { "code": "source-partial", "count": 1, "paths": ["broken.py"] },
            ] },
            { "ruleId": "covered", "clauseId": "clean", "status": "complete", "reasons": [] },
        ]);

        let cold = stored_workspace(&store, "/coverage", &code_rule);
        let result = apply(&cold, &files);
        assert_eq!(result["evaluation"]["coverage"], expected, "{result}");
        cold.close();

        let warm = stored_workspace(&store, "/coverage", &code_rule);
        let result = apply(&warm, &files);
        let restored = warm
            .state
            .lock()
            .unwrap()
            .files
            .values()
            .map(|file| file.restored_local_results)
            .sum::<usize>();
        assert!(restored > 0, "the file's results are restored");
        assert_eq!(result["evaluation"]["coverage"], expected, "{result}");
        let page: Value =
            serde_json::from_str(&result_page(&warm, 0, 64 * 1024 * 1024).unwrap()).unwrap();
        assert_eq!(page["coverage"], expected, "{page}");
        warm.close();
        drop(store);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A file no clause read is reported once, by path and reason, cold, restored and paged: one
    /// whose syntax errors failed every clause (`syntax`), one whose every clause ran out of work
    /// (`budget`). A file any clause read is not.
    #[test]
    fn blind_files_are_reported_by_reason_cold_warm_and_paged() {
        let _slot = WORKSPACE_TEST_SLOT.lock();
        let directory = std::env::temp_dir().join(format!(
            "rerail-blind-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let store = Arc::new(ContentStore::open(&directory.join("content.sqlite")).unwrap());
        // Unsound under syntax errors: a broken file fails it `source-incomplete`.
        let code_rule = json!({
            "format": "rerail.policy/v1", "id": "blind", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{ "id": "unguarded", "violation": violation(), "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "debugger_statement",
                    "not": { "follows": { "kind": "import_statement", "stopBy": "end" } } } } } }],
        })
        .to_string();
        let files = json!([
            { "path": "broken.ts", "language": "typescript", "source": "function f( {\n  debugger;\n" },
            { "path": "ok.ts", "language": "typescript", "source": "debugger;\n" },
        ]);
        // Its error regions cover most of it: the parser did not read it (`UNREAD`).
        let syntax = json!({ "clauses": 1, "message": "file did not parse",
            "path": "broken.ts", "reason": "syntax", "folded": [["blind", "unguarded"]] });

        let cold = stored_workspace(&store, "/blind", &code_rule);
        let result = apply(&cold, &files);
        assert_eq!(result["blindFiles"], json!([syntax]), "{result}");
        assert_eq!(
            result["evaluation"]["violations"].as_array().unwrap().len(),
            1
        );
        cold.close();

        let warm = stored_workspace(&store, "/blind", &code_rule);
        let result = apply(&warm, &files);
        assert_eq!(result["blindFiles"], json!([syntax]), "{result}");
        let page: Value =
            serde_json::from_str(&result_page(&warm, 0, 64 * 1024 * 1024).unwrap()).unwrap();
        assert_eq!(page["blindFiles"], json!([syntax]), "{page}");
        warm.close();

        // New content, so no stored result answers it.
        let starved = stored_workspace(&store, "/blind-budget", &code_rule);
        let files = json!([files[0], { "path": "ok.ts", "language": "typescript",
            "source": "debugger;\ndebugger;\n" }]);
        super::super::TEST_CLAUSE_WORK_BUDGET.set(Some(0));
        let result = apply(&starved, &files);
        super::super::TEST_CLAUSE_WORK_BUDGET.set(None);
        assert_eq!(
            result["blindFiles"],
            json!([syntax, { "clauses": 1, "message": "clause work budget exceeded",
                "path": "ok.ts", "reason": "budget", "folded": [["blind", "unguarded"]] }]),
            "{result}"
        );
        starved.close();
        drop(store);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A complete result after an edit renders the edited file's violations alone and sorts none:
    /// unchanged files' renderings are kept, in path order, and the result is a fresh
    /// workspace's.
    #[test]
    fn an_edit_renders_only_its_file_and_keeps_result_order() {
        use super::{RESULT_RENDERS, RESULT_SORTS};
        let _slot = WORKSPACE_TEST_SLOT.lock();
        let code_rule = json!({
            "format": "rerail.policy/v1", "id": "rendered", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [
                { "id": "print", "forbid": { "ast": { "language": "python", "rule": { "pattern": "print($A)" } } }, "violation": violation() },
                { "id": "call", "forbid": { "ast": { "language": "python", "rule": { "kind": "call" } } }, "violation": violation() },
            ],
        })
        .to_string();
        let compiled =
            CodeRuleDslRuntime::compile(code_rule, "json".into(), CAPABILITIES.into()).unwrap();
        let file = |path: &str, source: &str| json!({ "path": path, "language": "python", "source": source });
        let files = [
            file("b.py", "print(1)\nf()\n"),
            file("a.py", "print(2)\n"),
            file("c/d.py", "g(print(3))\n"),
        ];
        let run = |workspace: &CodeRuleDslWorkspace, revision: u32, files: &[Value]| -> Value {
            let mut task = WorkspaceApplyTask {
                state: Arc::clone(&workspace.state),
                closed: Arc::clone(&workspace.closed),
                request_json: json!({ "revision": revision, "files": files, "removed": [] })
                    .to_string(),
                request_buffer: None,
                request_direct: None,
            };
            serde_json::from_str(&task.compute().unwrap()).unwrap()
        };
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace.add_code_rule(&compiled, None).unwrap();
        run(&workspace, 1, &files);
        let edited = file("b.py", "x = 1\nprint(1)\nf()\n");
        RESULT_RENDERS.set(0);
        RESULT_SORTS.set(0);
        let result = run(&workspace, 2, std::slice::from_ref(&edited));
        assert_eq!((RESULT_RENDERS.get(), RESULT_SORTS.get()), (1, 0));
        let mut fresh = CodeRuleDslWorkspace::new();
        fresh.add_code_rule(&compiled, None).unwrap();
        let expected = run(&fresh, 1, &[files[1].clone(), edited, files[2].clone()]);
        assert_eq!(
            result["evaluation"]["violations"],
            expected["evaluation"]["violations"]
        );
        assert_eq!(
            result["evaluation"]["violations"].as_array().unwrap().len(),
            8
        );
        workspace.close();
        fresh.close();
    }
