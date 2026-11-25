//! A code rule placed per folder of a checkout: where it runs, at which mode, whether linters'
//! suppression comments silence it, and the result catalog its placement keys.

use super::super::model::Capabilities;
use super::super::test_support::CAPABILITIES;
use super::super::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
use super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask};
use super::persisted::candidate_catalog;
use super::tests::WORKSPACE_TEST_SLOT;
use crate::content_store::CandidateCatalogIdentity;
use globset::GlobSet;
use napi::Task;
use serde_json::{Value, json};
use std::sync::Arc;

const NESTED: &str = r#"[
    {"folder": "web", "mode": "steer"},
    {"folder": "web/legacy", "mode": null},
    {"folder": "web/legacy/keep", "mode": "log"}
]"#;

fn needle_code_rule() -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "placed-needle", "revision": 1,
            "mode": "block",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{
                "id": "needle", "violation": { "anchor": "witness.span", "code": "NEEDLE",
                    "message": "found needle" },
                "forbid": { "regex": { "source": "file.content", "pattern": "NEEDLE" } }
            }]
        })
        .to_string(),
        "json".into(),
        json!({
            "ast": { "engine": "ast-grep-core", "engineVersion": "0.45", "languages": {} },
            "limits": { "maxAstPatternBytes": 16384, "maxRegexPatternBytes": 16384,
                "maxFiles": 100, "maxGraphDepth": 16, "maxSourceBytes": 1000000,
                "maxVisitedNodes": 100000, "maxWitnesses": 10000 }
        })
        .to_string(),
    )
    .unwrap()
}

fn placed(placement: Option<&str>) -> CodeRuleDslWorkspace {
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(&needle_code_rule(), placement.map(str::to_owned))
        .unwrap();
    workspace
}

/// Reports each TypeScript identifier as `names`, a rule ESLint comments can disable.
fn names_code_rule() -> CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "placed-names", "revision": 1,
            "mode": "block",
            "clauses": [{
                "id": "names",
                "forbid": { "ast": { "language": "typescript", "rule": { "kind": "identifier" } } },
                "violation": { "anchor": "witness.span", "code": "NAMES", "message": "name",
                    "suppress": { "eslint": { "rules": ["names"] } } }
            }]
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.to_owned(),
    )
    .unwrap()
}

/// One apply of `files` (`{path, language, source}` objects); its evaluation.
fn apply(workspace: &CodeRuleDslWorkspace, files: Vec<Value>) -> Value {
    // `compute` refuses to wait for the process-wide CPU slot; serialize with the other tests.
    let _slot = WORKSPACE_TEST_SLOT.lock();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 1, "files": files, "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let result: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    result["evaluation"].clone()
}

/// Each violation's path and mode after one apply of `paths`, every one holding the needle.
fn violations(workspace: &CodeRuleDslWorkspace, paths: &[&str]) -> Vec<(String, String)> {
    let files = paths
        .iter()
        .map(|path| json!({ "path": path, "language": "typescript", "source": "NEEDLE\n" }))
        .collect::<Vec<_>>();
    let evaluation = apply(workspace, files);
    let mut violations = evaluation["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|violation| {
            (
                violation["path"].as_str().unwrap().to_owned(),
                violation["mode"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    violations.sort();
    violations
}

fn catalog(workspace: &CodeRuleDslWorkspace) -> CandidateCatalogIdentity {
    candidate_catalog(&workspace.state.lock().unwrap().code_rules).unwrap()
}

#[test]
fn the_deepest_placed_folder_decides_acceptance_and_mode() {
    let workspace = placed(Some(NESTED));
    let paths = [
        "web/a.ts",
        "web/legacy/a.ts",
        "web/legacy/keep/a.ts",
        "web2/x.ts",
        "api/a.ts",
    ];
    assert_eq!(
        paths.map(|path| workspace.accepts_path(path.to_owned())),
        [true, false, true, false, false]
    );
    assert_eq!(
        violations(&workspace, &paths),
        [
            ("web/a.ts".to_owned(), "steer".to_owned()),
            ("web/legacy/keep/a.ts".to_owned(), "log".to_owned()),
        ]
    );
    workspace.close();
}

#[test]
fn an_unplaced_code_rule_runs_everywhere_at_its_rule_mode() {
    let workspace = placed(None);
    assert!(workspace.accepts_path("web/legacy/a.ts".to_owned()));
    assert_eq!(
        violations(&workspace, &["api/a.ts", "web/legacy/a.ts"]),
        [
            ("api/a.ts".to_owned(), "block".to_owned()),
            ("web/legacy/a.ts".to_owned(), "block".to_owned()),
        ]
    );
    workspace.close();
}

#[test]
fn placing_a_code_rule_leaves_the_compiled_code_rule_unplaced() {
    let compiled = needle_code_rule();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace
        .add_code_rule(
            &compiled,
            Some(r#"[{"folder": "web", "mode": "log"}]"#.to_owned()),
        )
        .unwrap();
    assert!(!workspace.accepts_path("api/a.ts".to_owned()));
    assert!(compiled.in_scope("api/a.ts".to_owned()));
    workspace.close();
}

#[test]
fn an_invalid_placement_registers_nothing() {
    let mut workspace = CodeRuleDslWorkspace::new();
    for placement in [
        "{",
        r#"[{"folder": "web", "mode": "warn"}]"#,
        r#"[{"folder": "../web", "mode": "log"}]"#,
        r#"[{"folder": "web", "mode": "log"}, {"folder": "web/", "mode": null}]"#,
    ] {
        assert!(
            workspace
                .add_code_rule(&needle_code_rule(), Some(placement.to_owned()))
                .is_err(),
            "{placement}"
        );
    }
    assert!(workspace.state.lock().unwrap().code_rules.is_empty());
    assert!(!workspace.accepts_path("web/a.ts".to_owned()));
    workspace.close();
}

#[test]
fn the_result_catalog_differs_when_only_the_placement_differs() {
    let steer = placed(Some(r#"[{"folder": "", "mode": "steer"}]"#));
    let log = placed(Some(r#"[{"folder": "", "mode": "log"}]"#));
    let narrowed = placed(Some(r#"[{"folder": "web", "mode": "steer"}]"#));
    let unplaced = placed(None);
    let again = placed(None);
    let catalogs = [&steer, &log, &narrowed, &unplaced].map(catalog);
    for (index, catalog) in catalogs.iter().enumerate() {
        assert!(
            catalogs[index + 1..].iter().all(|other| other != catalog),
            "catalog {index} is shared"
        );
    }
    assert_eq!(catalog(&again), catalogs[3]);
    for workspace in [steer, log, narrowed, unplaced, again] {
        workspace.close();
    }
}

#[test]
fn the_deciding_folder_s_suppressions_choice_decides_whether_a_disable_comment_silences() {
    const SOURCE: &str = "a; // eslint-disable-line names\nb;\n";
    // Each placement, and the names it reports in `web/a.ts` and in `api/a.ts`.
    let cases = [
        (None, [vec!["b"], vec!["b"]]),
        (
            Some(r#"[{"folder": "", "mode": "log"}]"#),
            [vec!["b"], vec!["b"]],
        ),
        (
            Some(r#"[{"folder": "", "mode": "log", "suppressions": "honour"}]"#),
            [vec!["b"], vec!["b"]],
        ),
        (
            Some(r#"[{"folder": "", "mode": "log", "suppressions": "ignore"}]"#),
            [vec!["a", "b"], vec!["a", "b"]],
        ),
        (
            Some(
                r#"[{"folder": "", "mode": "log", "suppressions": "honour"},
                    {"folder": "web", "mode": "log", "suppressions": "ignore"}]"#,
            ),
            [vec!["a", "b"], vec!["b"]],
        ),
    ];
    for (placement, expected) in cases {
        let mut workspace = CodeRuleDslWorkspace::new();
        workspace
            .add_code_rule(&names_code_rule(), placement.map(str::to_owned))
            .unwrap();
        let files = ["web/a.ts", "api/a.ts"]
            .map(|path| json!({ "path": path, "language": "typescript", "source": SOURCE }));
        let evaluation = apply(&workspace, files.to_vec());
        let reported = ["web/a.ts", "api/a.ts"].map(|path| {
            evaluation["violations"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|violation| violation["path"] == path)
                .map(|violation| {
                    let span = &violation["span"];
                    &SOURCE[span["startByte"].as_u64().unwrap() as usize
                        ..span["endByte"].as_u64().unwrap() as usize]
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(reported, expected, "{placement:?}: {evaluation}");
        workspace.close();
    }
}

#[test]
fn the_result_catalog_differs_when_only_the_suppressions_choice_differs() {
    let omitted = placed(Some(r#"[{"folder": "", "mode": "log"}]"#));
    let honour = placed(Some(
        r#"[{"folder": "", "mode": "log", "suppressions": "honour"}]"#,
    ));
    let ignore = placed(Some(
        r#"[{"folder": "", "mode": "log", "suppressions": "ignore"}]"#,
    ));
    assert_eq!(catalog(&omitted), catalog(&honour));
    assert_ne!(catalog(&honour), catalog(&ignore));
    for workspace in [omitted, honour, ignore] {
        workspace.close();
    }
}

/// A workspace's `file.content` regex reads an ECMAScript file from past a leading byte order
/// mark, as ESLint's rules read its text, and reports over the file's own bytes.
#[test]
fn content_regexes_skip_a_leading_byte_order_mark_in_ecmascript_files() {
    let compiled = CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "bom", "revision": 1, "mode": "block",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{
                "id": "start", "violation": { "anchor": "witness.span", "code": "START",
                    "message": "start" },
                "forbid": { "regex": { "source": "file.content", "pattern": "\\A[\\x{FEFF}x]" } }
            }]
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.to_owned(),
    )
    .unwrap();
    let mut workspace = CodeRuleDslWorkspace::new();
    workspace.add_code_rule(&compiled, None).unwrap();
    let source = "\u{feff}x = 1;\n";
    let evaluation = apply(
        &workspace,
        vec![
            json!({ "path": "a.ts", "language": "typescript", "source": source }),
            json!({ "path": "a.py", "language": "python", "source": source }),
        ],
    );
    let mut found = evaluation["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|violation| {
            (
                violation["path"].as_str().unwrap().to_owned(),
                violation["span"]["startByte"].as_u64().unwrap(),
                violation["span"]["endByte"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    found.sort();
    assert_eq!(
        found,
        [("a.py".to_owned(), 0, 3), ("a.ts".to_owned(), 3, 4)],
        "{evaluation}"
    );
}

/// Where a workspace's first code rule keeps its canonical text and its clauses, and its placement.
fn first_code_rule(workspace: &CodeRuleDslWorkspace) -> (*const u8, *const u8, usize, bool) {
    let state = workspace.state.lock().unwrap();
    let code_rule = &state.code_rules[0];
    (
        code_rule.canonical.as_ptr(),
        code_rule.clauses.as_ptr().cast(),
        code_rule.clauses.len(),
        code_rule.scope.is_placed(),
    )
}

/// A placement is all a workspace changes of a file-local code rule, so the workspaces that place one
/// compiled code rule, each its own way, hold its canonical text and its clauses once between them:
/// a copy in each cost a session's lane about 7 MB at the 382 rules of four packs, and a session
/// opens one lane in each other worktree whose changes it audits.
#[test]
fn workspaces_placing_one_code_rule_share_its_text_and_clauses() {
    let compiled = needle_code_rule();
    let mut nested = CodeRuleDslWorkspace::new();
    nested
        .add_code_rule(&compiled, Some(NESTED.to_owned()))
        .unwrap();
    let mut whole = CodeRuleDslWorkspace::new();
    whole
        .add_code_rule(
            &compiled,
            Some(r#"[{"folder": "", "mode": "log"}]"#.to_owned()),
        )
        .unwrap();
    let held: (*const u8, *const u8, usize, bool) = (
        compiled.inner.canonical.as_ptr(),
        compiled.inner.clauses.as_ptr().cast(),
        compiled.inner.clauses.len(),
        true,
    );
    assert_eq!(first_code_rule(&nested), held);
    assert_eq!(first_code_rule(&whole), held);
    // Each keeps its own placement: the same file is steered in one and logged in the other.
    assert_eq!(
        violations(&nested, &["web/a.ts"]),
        [("web/a.ts".to_owned(), "steer".to_owned())]
    );
    assert_eq!(
        violations(&whole, &["web/a.ts"]),
        [("web/a.ts".to_owned(), "log".to_owned())]
    );
}

/// A code rule with a global clause compiles it into clauses of the workspace's own (`global.rs`):
/// those go into the workspace's copy, never into the compiled code rule another workspace shares.
#[test]
fn a_global_clause_compiles_into_the_workspace_copy_alone() {
    let compiled = CodeRuleDslRuntime::compile(
        json!({
            "format": "rerail.policy/v1", "id": "placed-budget", "revision": 1,
            "mode": "steer",
            "scope": { "include": ["**/*"], "exclude": [] },
            "clauses": [{
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
            }]
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.to_owned(),
    )
    .unwrap();
    let clauses = compiled.inner.clauses.len();
    let mut first = CodeRuleDslWorkspace::new();
    first
        .add_code_rule(&compiled, Some(NESTED.to_owned()))
        .unwrap();
    let mut second = CodeRuleDslWorkspace::new();
    second.add_code_rule(&compiled, None).unwrap();
    assert_eq!(compiled.inner.clauses.len(), clauses);
    assert!(first_code_rule(&first).2 > clauses);
    assert_eq!(first_code_rule(&second).2, first_code_rule(&first).2);
    assert_ne!(first_code_rule(&first).1, first_code_rule(&second).1);
}

/// Where a workspace's first code rule keeps its scope's globs and its capabilities.
fn first_code_rule_globs(workspace: &CodeRuleDslWorkspace) -> [usize; 3] {
    let state = workspace.state.lock().unwrap();
    let code_rule = &state.code_rules[0];
    [
        std::ptr::from_ref::<GlobSet>(code_rule.scope.include_globs()).addr(),
        std::ptr::from_ref::<GlobSet>(code_rule.scope.exclude_globs()).addr(),
        std::ptr::from_ref::<Capabilities>(&code_rule.capabilities).addr(),
    ]
}

/// The scope's globs and the capabilities are the rest of what a placed copy took from the compiled
/// code rule unchanged: the workspaces that place one compiled code rule hold them once too (a copy
/// of each was about 7 KB a code rule, 2.6 MB a lane at the 382 rules of four packs).
#[test]
fn workspaces_placing_one_code_rule_share_its_globs_and_capabilities() {
    let compiled = needle_code_rule();
    let mut nested = CodeRuleDslWorkspace::new();
    nested
        .add_code_rule(&compiled, Some(NESTED.to_owned()))
        .unwrap();
    let mut unplaced = CodeRuleDslWorkspace::new();
    unplaced.add_code_rule(&compiled, None).unwrap();
    assert_eq!(
        first_code_rule_globs(&nested),
        first_code_rule_globs(&unplaced)
    );
    assert_eq!(
        violations(&nested, &["web/legacy/a.ts", "web/a.ts", "a.ts"]),
        [("web/a.ts".to_owned(), "steer".to_owned())]
    );
}
