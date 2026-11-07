use super::{DERIVED_MANIFESTS, parse_snapshot};
use crate::code_rule_dsl::module_graph::SUMMARIES;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use crate::code_rule_dsl::{CodeRuleDslEditor, CodeRuleDslRuntime, CompiledCodeRuleDsl};
use serde_json::{Value, json};

#[test]
#[cfg_attr(
    not(feature = "p-anchor-corpus"),
    ignore = "P's anchor corpus selects roots only on the p-anchor-corpus path (open item O20)"
)]
fn stateless_fact_extraction_uses_selective_anchor_postings() {
    let compiled = crate::code_rule_dsl::test_support::compile_typescript_ast(json!({
        "rule": { "matches": "selected" }, "utils": { "selected": {
            "kind": "call_expression", "has": { "field": "function", "kind": "identifier", "regex": "^target$" }
        } }
    })).unwrap();
    let mut source = "other(value);\n".repeat(500);
    source.push_str("target(value);\nother(target);\n");
    crate::code_rule_dsl::matcher::reset_roots_matched();
    let result =
        crate::code_rule_dsl::test_support::evaluate_file(&compiled, "typescript", "a.ts", &source);
    crate::code_rule_dsl::test_support::assert_violations(&result, 1);
    assert_eq!(crate::code_rule_dsl::matcher::roots_matched(), 1);
}

#[test]
#[cfg_attr(
    not(feature = "p-anchor-corpus"),
    ignore = "P's anchor corpus selects roots only on the p-anchor-corpus path (open item O20)"
)]
fn relational_inputs_share_one_anchor_index_and_keep_distinct_selections() {
    use crate::code_rule_dsl::test_support::{Walk, compile_document_with, reset_walks, walks};
    let input = |name: &str| {
        json!({ "evidence": { "as": "f", "query": { "ast": {
        "language": "typescript", "rule": { "kind": "call_expression", "has": {
            "field": "function", "kind": "identifier", "regex": format!("^{name}$"), "pattern": "$NAME"
        } }
    } }, "project": { "name": "capture.NAME" } } })
    };
    let document = json!({ "format": "rerail.policy/v1", "id": "shared-anchors", "revision": 1, "mode": "log", "clauses": [{
        "id": "selected", "relational": { "partitionBy": "file", "inputs": { "a": input("target"), "b": input("other") },
            "query": "SELECT a.__path AS path,a.__start AS start_byte,a.__end AS end_byte,a.name AS name,'found' AS issue FROM a WHERE EXISTS(SELECT 1 FROM b)",
            "key": ["name"], "code": "issue" },
        "violation": { "anchor": "witness.span", "code": "FOUND", "message": "found" }
    }] });
    let compiled =
        compile_document_with(document, &serde_json::from_str(CAPABILITIES).unwrap()).unwrap();
    let source = "other(value);\n".repeat(500) + "target(value);\nother(target);\n";
    reset_walks();
    let result =
        crate::code_rule_dsl::test_support::evaluate_file(&compiled, "typescript", "a.ts", &source);
    crate::code_rule_dsl::test_support::assert_violations(&result, 1);
    assert_eq!(walks(Walk::AnchorBuild), 1);
    assert_eq!(
        result["violations"][0]["variables"]["row"]["name"],
        "target"
    );
}

/// A code rule reporting each TypeScript program in (`SERVER`) and out of (`CLIENT`) its Next
/// app's server graph.
fn server_graph_rule() -> CompiledCodeRuleDsl {
    let program = |id: &str, member: bool| {
        json!({ "id": id, "violation": { "anchor": "witness.span", "code": id.to_uppercase(), "message": id },
            "forbid": { "ast": { "language": "typescript",
                "file": { "package": { "serverGraph": member } }, "rule": { "kind": "program" } } } })
    };
    CodeRuleDslRuntime::compile(
        json!({ "format": "rerail.policy/v1", "id": "server-graph", "revision": 1, "mode": "block",
            "clauses": [program("server", true), program("client", false)] })
        .to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .expect("the code rule compiles")
}

/// A snapshot of `files` (path, language, text, host `package`).
fn snapshot(files: &[(&str, &str, String, Option<Value>)]) -> String {
    let files = files
        .iter()
        .map(|(path, language, source, package)| {
            let mut file = json!({ "path": path, "language": language, "source": source });
            if let Some(package) = package {
                file["package"] = package.clone();
            }
            file
        })
        .collect::<Vec<_>>();
    json!({ "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0, "files": files })
        .to_string()
}

/// The violations of an evaluation, as `path CODE`, sorted.
fn reported(result: &str) -> Vec<String> {
    let result: Value = serde_json::from_str(result).expect("evaluation is JSON");
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut violations = result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            format!(
                "{} {}",
                violation["path"].as_str().expect("a path"),
                violation["code"].as_str().expect("a code")
            )
        })
        .collect::<Vec<_>>();
    violations.sort();
    violations
}

/// Replaces the whole text of `files[index]` through `editor`, returning the summaries computed
/// and the manifests derived for it, after checking the editor's packages and violations are a
/// fresh snapshot's.
fn edit(
    compiled: &CompiledCodeRuleDsl,
    editor: &mut CodeRuleDslEditor,
    files: &mut [(&str, &str, String, Option<Value>)],
    index: usize,
    text: &str,
) -> (usize, usize) {
    let path = files[index].0;
    let revision = editor.snapshot.revision + 1;
    let edit = json!({ "revision": revision, "path": path, "startByte": 0,
        "oldEndByte": files[index].2.len(), "newText": text });
    files[index].2 = text.to_owned();
    SUMMARIES.set(0);
    DERIVED_MANIFESTS.set(0);
    let edited = editor
        .apply_edit(edit.to_string(), "report".to_owned())
        .unwrap();
    let work = (SUMMARIES.get(), DERIVED_MANIFESTS.get());
    let snapshot = snapshot(files);
    let (fresh, _) = parse_snapshot(&snapshot, true).unwrap();
    for (held, fresh) in editor.snapshot.files.iter().zip(&fresh.files) {
        assert_eq!(held.package, fresh.package, "{} after {path}", held.path);
    }
    assert_eq!(
        reported(&edited),
        reported(&compiled.evaluate(snapshot, "report".to_owned()).unwrap()),
        "after {path}"
    );
    work
}

/// An editor's edit derives again only what it changes, as a fresh snapshot derives it: a source
/// edit summarizes that source again and derives its package's graph only where the summary
/// changes; a project configuration derives again the packages whose configuration it changes,
/// a manifest its own package; an edit outside a package naming `next` derives nothing.
#[test]
fn an_editor_edit_derives_again_only_the_package_and_summary_it_changes() {
    let compiled = server_graph_rule();
    let next = r#"{ "dependencies": { "next": "15" } }"#;
    let web_tsconfig = r#"{ "extends": "../tsconfig.base.json", "compilerOptions": { "paths": { "@/*": ["./lib/*"] } } }"#;
    let widget = "'use client';\nimport { hook } from '../lib/hook';\nexport default function Widget() { hook(); }\n";
    let file = |path, language, text: &str| (path, language, text.to_owned(), None);
    let mut files = vec![
        file("web/package.json", "json", next),
        file("web/tsconfig.json", "json", web_tsconfig),
        file(
            "tsconfig.base.json",
            "json",
            r#"{ "compilerOptions": { "jsxImportSource": "react" } }"#,
        ),
        file(
            "web/app/page.tsx",
            "tsx",
            "import Widget from './widget';\nimport { data } from '@/data';\nexport default function Page() {}\n",
        ),
        file("web/app/widget.tsx", "tsx", widget),
        file(
            "web/lib/hook.ts",
            "typescript",
            "export const hook = () => 1;\n",
        ),
        file("web/lib/data.ts", "typescript", "export const data = 1;\n"),
        file("docs/package.json", "json", next),
        file(
            "docs/app/page.tsx",
            "tsx",
            "export default function Page() {}\n",
        ),
        file("tools/package.json", "json", "{}"),
        file("tools/run.ts", "typescript", "export {};\n"),
        file("scripts/build.ts", "typescript", "export {};\n"),
        (
            "host.ts",
            "typescript",
            "export {};\n".to_owned(),
            Some(
                json!({ "folder": "", "type": "unset", "dependencies": ["next"], "serverGraph": true }),
            ),
        ),
    ];
    let mut editor = compiled.open_editor(snapshot(&files)).unwrap();
    let mut edit = |index, text: &str| edit(&compiled, &mut editor, &mut files, index, text);
    // Without its directive the widget, and the hook it imports, join the graph: one summary,
    // one package.
    assert_eq!(edit(4, &widget["'use client';\n".len()..]), (1, 1));
    // An edit that changes no import or directive changes no graph.
    assert_eq!(
        edit(5, "// the hook\nexport const hook = () => 2;\n"),
        (1, 0)
    );
    // A package naming no `next`, a file of no package, a file with a host `package`.
    assert_eq!(edit(10, "export const run = 1;\n"), (0, 0));
    assert_eq!(edit(11, "export const build = 1;\n"), (0, 0));
    assert_eq!(edit(12, "export const host = 1;\n"), (0, 0));
    // A base only `web` extends, then `web`'s own configuration, losing its alias.
    assert_eq!(
        edit(
            2,
            r#"{ "compilerOptions": { "jsxImportSource": "preact" } }"#
        ),
        (0, 1)
    );
    assert_eq!(edit(1, r#"{ "extends": "../tsconfig.base.json" }"#), (0, 1));
    // A manifest losing and gaining `next` reads the summaries already computed.
    assert_eq!(edit(7, "{}"), (0, 1));
    assert_eq!(edit(7, next), (0, 1));
    // A package gaining `next` summarizes its source the first time.
    assert_eq!(edit(9, next), (1, 1));
    assert_eq!(edit(10, "import './missing';\nexport {};\n"), (1, 1));
}

/// An editor whose every ECMA file carries a host `package` derives nothing, whatever it edits.
#[test]
fn an_editor_without_derived_packages_derives_nothing() {
    let compiled = server_graph_rule();
    let host =
        json!({ "folder": "", "type": "unset", "dependencies": ["next"], "serverGraph": true });
    let mut files = vec![
        (
            "package.json",
            "json",
            r#"{ "dependencies": { "next": "15" } }"#.to_owned(),
            None,
        ),
        ("tsconfig.json", "json", "{}".to_owned(), None),
        (
            "app/page.tsx",
            "tsx",
            "export default function Page() {}\n".to_owned(),
            Some(host),
        ),
    ];
    SUMMARIES.set(0);
    DERIVED_MANIFESTS.set(0);
    let mut editor = compiled.open_editor(snapshot(&files)).unwrap();
    assert_eq!((SUMMARIES.get(), DERIVED_MANIFESTS.get()), (0, 0));
    let mut edit = |index, text: &str| edit(&compiled, &mut editor, &mut files, index, text);
    assert_eq!(
        edit(
            1,
            r#"{ "compilerOptions": { "jsxImportSource": "preact" } }"#
        ),
        (0, 0)
    );
    assert_eq!(edit(0, "{}"), (0, 0));
    assert_eq!(edit(2, "import './a';\nexport default 1;\n"), (0, 0));
}
