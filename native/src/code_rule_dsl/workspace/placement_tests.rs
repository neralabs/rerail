//! A code rule placed per folder of a checkout: where it runs, at which mode, and the result
//! catalog its placement keys.

use super::super::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
use super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask};
use super::persisted::candidate_catalog;
use super::tests::WORKSPACE_TEST_SLOT;
use crate::content_store::CandidateCatalogIdentity;
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

/// Each violation's path and mode after one apply of `paths`, every one holding the needle.
fn violations(workspace: &CodeRuleDslWorkspace, paths: &[&str]) -> Vec<(String, String)> {
    // `compute` refuses to wait for the process-wide CPU slot; serialize with the other tests.
    let _slot = WORKSPACE_TEST_SLOT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let files = paths
        .iter()
        .map(|path| json!({ "path": path, "language": "typescript", "source": "NEEDLE\n" }))
        .collect::<Vec<_>>();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&workspace.state),
        closed: Arc::clone(&workspace.closed),
        request_json: json!({ "revision": 1, "files": files, "removed": [] }).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let result: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    let mut violations = result["evaluation"]["violations"]
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
