//! Compiled statements against the matcher they replace: for every check of a catalog and every
//! file, the roots each matches and the captures each binds must be identical.

use super::{CatalogBuilder, CheckPlan, Env, QueryConnection, compile_check};
use crate::policy_dsl::PolicyDslRuntime;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::matcher::GrammarMatcher;
use crate::policy_dsl::model::{CheckKind, Query};
use crate::policy_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::policy_dsl::test_support::CAPABILITIES;
use crate::policy_dsl::transducer::sql_backend::BatchFile;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{AstGrep, Doc};
use std::collections::BTreeMap;
use std::sync::Arc;

fn tape(source: &str, language: GrammarDialect) -> Option<TapeDoc> {
    let grammar = Grammar::new(language).ok()?;
    let native = StrDoc::new(source, language);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).ok()?;
    let bytes = image.to_bytes().ok()?;
    let image = TapeImage::from_bytes(&bytes, &grammar).ok()?;
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).ok()
}

pub(super) fn language_of(path: &str) -> Option<GrammarDialect> {
    crate::policy_dsl::language::label_of_path(path)
        .and_then(crate::policy_dsl::language::dialect_by_label)
}

/// Every AST check matcher of the policy documents in `directories`, labelled.
pub(super) fn catalog_matchers(directories: &[String]) -> Vec<(String, GrammarMatcher)> {
    let mut matchers = Vec::new();
    for directory in directories {
        let mut paths = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            let source = std::fs::read_to_string(&path).unwrap();
            let compiled =
                PolicyDslRuntime::compile(source, "json".into(), CAPABILITIES.into()).unwrap();
            for check in &compiled.inner.checks {
                if let CheckKind::Forbid(Query::Ast {
                    matchers: grammar, ..
                }) = &check.kind
                {
                    for matcher in grammar {
                        matchers.push((
                            format!("{}/{}:{:?}", compiled.inner.id, check.id, matcher.language),
                            matcher.clone(),
                        ));
                    }
                }
            }
        }
    }
    matchers
}

/// Roots and captures, per check label and file path, where the two paths differ; and counts.
#[derive(Default, Debug)]
pub(super) struct Report {
    pub(super) compiled: usize,
    pub(super) unsupported: BTreeMap<String, &'static str>,
    pub(super) files: usize,
    pub(super) matches: usize,
    pub(super) differences: Vec<String>,
    /// Statement time per check label, and the slowest single file for it.
    pub(super) timings: BTreeMap<String, (std::time::Duration, std::time::Duration, String)>,
    /// Matcher time per check label over the same files.
    pub(super) matcher_timings: BTreeMap<String, std::time::Duration>,
}

pub(super) fn compare(matchers: &[(String, GrammarMatcher)], files: &[(String, String)]) -> Report {
    let mut report = Report::default();
    let mut builder = CatalogBuilder::default();
    let mut plans: Vec<(usize, CheckPlan)> = Vec::new();
    for (index, (label, matcher)) in matchers.iter().enumerate() {
        match compile_check(matcher, &mut builder, false) {
            Ok(plan) => plans.push((index, plan)),
            Err(reason) => {
                report.unsupported.insert(label.clone(), reason.0);
            }
        }
    }
    report.compiled = plans.len();
    let (terminals, catalog) = builder.finish();
    let terminals = Arc::new(terminals);
    let catalog = Arc::new(catalog);
    let batch_files = files
        .iter()
        .filter_map(|(path, source)| {
            let language = language_of(path)?;
            let doc = tape(source, language)?;
            Some(BatchFile::new(
                doc,
                Arc::from(path.as_str()),
                Arc::from(format!("{language:?}")),
                None,
            ))
        })
        .collect::<Vec<_>>();
    report.files = batch_files.len();
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(Arc::clone(&terminals), batch_files, Arc::clone(&catalog));
    let view = connection.registry.files(batch.token()).unwrap();
    for (file_index, file) in view.files().iter().enumerate() {
        let language = *file.tape().get_lang();
        let root = AstGrep::doc(file.tape().clone());
        for (index, plan) in &plans {
            let (label, matcher) = &matchers[*index];
            if matcher.language != language {
                continue;
            }
            let started = std::time::Instant::now();
            let mut work = usize::MAX;
            let compiled = batch
                .run(plan, file_index, None, matcher, &mut work)
                .unwrap_or_else(|error| panic!("{label}: {error}\n{}", plan.sql));
            let elapsed = started.elapsed();
            let timing = report.timings.entry(label.clone()).or_default();
            timing.0 += elapsed;
            if elapsed > timing.1 {
                timing.1 = elapsed;
                timing.2 = file.path.to_string();
            }
            let mut expected = Vec::new();
            let mut work = usize::MAX;
            let started = std::time::Instant::now();
            for node in root.root().dfs() {
                if let Some(found) = matcher.match_root(node.clone(), &mut work).unwrap() {
                    expected.push((
                        found.get_node().node_id() as u32,
                        Env::from_meta(found.get_env()),
                    ));
                }
            }
            *report.matcher_timings.entry(label.clone()).or_default() += started.elapsed();
            let actual = compiled
                .into_iter()
                .map(|(node, env)| (node, (*env).clone()))
                .collect::<Vec<_>>();
            report.matches += expected.len();
            if actual != expected {
                report.differences.push(format!(
                    "{label} {}: matcher {:?} statement {:?}",
                    file.path,
                    expected.iter().map(|(node, _)| node).collect::<Vec<_>>(),
                    actual.iter().map(|(node, _)| node).collect::<Vec<_>>(),
                ));
            }
        }
    }
    report
}

fn rules(relative: &str) -> String {
    format!(
        "{}/../policies/executable-policy-dsl/{relative}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Every AST check of the default catalog compiles to a statement, and each statement finds what
/// its matcher finds on fixtures written to exercise the catalog's Python, TypeScript and TSX
/// rules (a check runs on every fixture of its language, whatever its scope).
#[test]
fn compiled_default_catalog_matches_the_matcher_on_fixtures() {
    let files = [
        (
            "nera-data-layer/a.py",
            "import importlib\nfrom x import y as z\n\ndef __getattr__(name):\n    return importlib.import_module(name)\n\nclass C:\n    def f(self):\n        print('x')\n        raise PermissionError('no')\n        try:\n            pass\n        except PermissionError:\n            pass\n",
        ),
        (
            "api/nera_api/routes/items.py",
            "from typing import Optional\nfrom uuid import UUID\nimport uuid\n\nasync def get_item(session, item_id: str, user_id: str) -> Optional[dict]:\n    rows = session.query(Item).filter(Item.id == item_id).first()\n    if hasattr(rows, 'value'):\n        rows = rows.value\n    items = list(session.select(Item))[0:10]\n    count = len(list(session.select(Item)))\n    node_id = str(uuid.uuid4())\n    keys = str(rows.keys())\n    session.execute(f\"SELECT * FROM items WHERE id = '{item_id}'\")\n    session.execute(\"DELETE FROM items WHERE id = %s\" % item_id)\n    if user_id == 'admin:write':\n        raise HTTPException(status_code=403, detail='forbidden')\n    try:\n        result = await session.get(item_id)\n    except:\n        result = None\n    config = {'model_config': {'extra': 'forbid'}}\n    cache = {}\n    cache[item_id] = result\n    return result\n\nclass Item:\n    __annotations__ = {}\n    version: int = 0\n    def update(self, txn):\n        txn.update(self)\n        return self.__annotations__\n",
        ),
        (
            "nera-data-layer/tests/test_items.py",
            "import inspect\nimport time\nimport pytest\n\n@pytest.mark.xfail(reason='flaky')\ndef test_items(snapshot):\n    start = time.time()\n    source = inspect.getsource(test_items)\n    assert time.time() - start < 1.0\n    assert snapshot == source\n    if not source:\n        return\n    assert 'x' in source\n",
        ),
        (
            "web/src/a.tsx",
            "import React from 'react';\nexport const View = () => { const x: any = 1; console.log(x); return <div onClick={() => fetch('/api')}>{x}</div>; };\n",
        ),
        (
            "web/src/components/Panel.tsx",
            "import { FaBeer } from 'react-icons/fa';\nimport { Button } from '@/components/ui/button';\nexport function Panel({ items }: { items: any[] }) {\n  const Lazy = React.lazy(() => import('./Heavy'));\n  const color = '#ff00aa';\n  return (\n    <div className=\"gap-[13px] rounded-[7px] text-[15px] bg-[#101010] text-gray-500 bg-slate-100 shadow-[var(--nera-shadow)] p-2\" style={{ color: '#abc' }}>\n      <Button className=\"bg-red-500 text-white\">Apply decisions</Button>\n      <FaBeer />\n      {items.map((item) => <span key={item.id} className=\"text-[var(--radix-gray-11)]\">{item.name}</span>)}\n      <Lazy />\n    </div>\n  );\n}\n",
        ),
        (
            "web/src/index.ts",
            "export * from './components/Panel';\nexport { default as View } from './a';\nconst query = 'SELECT * FROM users WHERE id = 1';\nexport async function load(id: any): Promise<unknown> {\n  const response = await fetch(`/api/items/${id}`);\n  return (await response.json()) as any;\n}\n",
        ),
        (
            "web/src/b.ts",
            "export function f(a: any) { if (a == null) { throw new Error('x'); } return a as unknown as string; }\nvar legacy = 1;\nlet unused;\nexport class K { constructor(private readonly v: number) {} m(): void { debugger; eval('1'); } }\n",
        ),
        (
            "web/src/Panel.stories.tsx",
            "import type { Meta } from '@storybook/react';\nimport { useState } from 'react';\nimport { Panel } from './components/Panel';\nconst meta: Meta<typeof Panel> = { title: 'Screens/Panel/Nested/Deep', component: Panel };\nexport default meta;\nexport const Default = () => { const [items] = useState([]); return <Panel items={items} />; };\n",
        ),
    ]
    .map(|(path, source)| (path.to_owned(), source.to_owned()));
    let matchers = catalog_matchers(&[rules("rules"), rules("rules/opt-in/tslint")]);
    let report = compare(&matchers, &files);
    assert!(
        report.unsupported.is_empty(),
        "catalog checks without a statement: {:#?}",
        report.unsupported
    );
    assert_eq!(report.files, files.len(), "every fixture is taped");
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
    assert_eq!(report.compiled, matchers.len());
    assert!(report.matches > 0, "the fixtures exercise no check");
}

#[test]
fn deeply_nested_field_rules_preserve_captures() {
    let mut rule =
        serde_json::json!({ "kind": "identifier", "pattern": "$BASE", "regex": "^root$" });
    for _ in 0..18 {
        rule["field"] = serde_json::json!("object");
        rule = serde_json::json!({ "kind": "member_expression", "any": [
            { "not": { "testUnknown": "^never$" } }, { "has": rule }
        ] });
    }
    let document = serde_json::json!({
        "format": "rerail.policy/v1", "id": "nested-fields", "revision": 1,
        "checks": [{
            "id": "nested",
            "forbid": { "ast": { "language": "typescript", "rule": rule } },
            "finding": { "anchor": "capture.BASE", "code": "BASE", "message": "$BASE", "severity": "warning" }
        }]
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let suffix = ".a".repeat(18);
    let source = format!("root{suffix}; other{suffix};");
    let report = compare(
        &[("nested-fields".to_owned(), matchers[0].clone())],
        &[("a.ts".to_owned(), source)],
    );
    assert_eq!(report.matches, 1);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
}

#[test]
fn relation_first_capture_controls_later_comparison() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1", "id": "relation-order", "revision": 1,
        "checks": [{
            "id": "first",
            "forbid": { "ast": { "language": "typescript", "rule": {
                "kind": "call_expression",
                "all": [
                    { "has": { "field": "function", "pattern": "$FN" } },
                    { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$ARG" } } },
                    { "compare": { "left": "FN", "op": "eq", "right": "ARG", "text": "source" } }
                ]
            } } },
            "finding": { "anchor": "capture.ARG", "code": "FIRST", "message": "$ARG", "severity": "warning" }
        }]
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let report = compare(
        &[("relation-order".to_owned(), matchers[0].clone())],
        &[("a.ts".to_owned(), "f(f, g); f(g, f);".to_owned())],
    );
    assert_eq!(report.matches, 1);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
}

/// Structural sub-rules the kernel answers, sharing operations across checks and files of one
/// batch: every relation axis, both walk lengths, and negations over remembered answers.
#[test]
fn structural_kernel_matches_the_matcher_across_shared_sub_rules() {
    let returns = serde_json::json!({ "has": { "stopBy": "end", "kind": "return_statement" } });
    let rules = [
        serde_json::json!({ "kind": "if_statement", "all": [returns] }),
        serde_json::json!({ "kind": "if_statement", "not": returns }),
        serde_json::json!({ "kind": "return_statement", "inside": { "stopBy": "end",
            "kind": "if_statement", "inside": { "stopBy": "end", "kind": "function_declaration" } } }),
        serde_json::json!({ "kind": "expression_statement",
            "precedes": { "stopBy": "end", "kind": "expression_statement", "regex": "^b;$" } }),
        serde_json::json!({ "kind": "expression_statement", "follows": { "stopBy": "end", "regex": "^a;$" } }),
        serde_json::json!({ "kind": "expression_statement", "any": [
            { "precedes": { "kind": "expression_statement" } },
            { "follows": { "kind": "if_statement" } }
        ] }),
        serde_json::json!({ "kind": "statement_block", "has": { "kind": "return_statement" },
            "inside": { "kind": "if_statement" } }),
        serde_json::json!({ "kind": "function_declaration", "has": { "stopBy": "end", "all": [
            { "kind": "if_statement" }, { "not": returns }
        ] } }),
    ];
    let checks = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            serde_json::json!({
                "id": format!("structure-{index}"),
                "forbid": { "ast": { "language": "typescript", "rule": rule } },
                "finding": { "anchor": "witness.span", "code": "STRUCTURE", "message": "structure", "severity": "warning" }
            })
        })
        .collect::<Vec<_>>();
    let document = serde_json::json!({
        "format": "rerail.policy/v1", "id": "structure", "revision": 1, "checks": checks
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let matchers = compiled
        .inner
        .checks
        .iter()
        .map(|check| {
            let CheckKind::Forbid(Query::Ast { matchers, .. }) = &check.kind else {
                panic!("expected AST forbid check");
            };
            (check.id.clone(), matchers[0].clone())
        })
        .collect::<Vec<_>>();
    let files = [
        "function f(x) {\n  if (x) {\n    if (x > 1) { return 1; }\n    a; b; c;\n  } else {\n    a; b;\n  }\n  if (x) { g(); }\n  return 0;\n}\n",
        "function h() { if (y) { k(); } a; b; }\nif (z) { return; }\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, source)| (format!("{index}.ts"), (*source).to_owned()))
    .collect::<Vec<_>>();
    let report = compare(&matchers, &files);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
    assert!(report.matches >= rules.len());
}

#[test]
fn long_utility_chains_match_without_exceeding_sqlite_expression_depth() {
    let mut utilities = serde_json::Map::new();
    for index in 0..100 {
        let rule = if index == 99 {
            serde_json::json!({ "kind": "identifier" })
        } else {
            serde_json::json!({ "matches": format!("u{}", index + 1) })
        };
        utilities.insert(format!("u{index}"), rule);
    }
    let document = serde_json::json!({
        "format": "rerail.policy/v1",
        "id": "query-depth",
        "revision": 1,
        "checks": [{
            "id": "chain",
            "forbid": {
                "ast": {
                    "language": "typescript",
                    "rule": { "matches": "u0" },
                    "utils": utilities,
                },
            },
            "finding": {
                "anchor": "witness.span",
                "code": "CHAIN",
                "message": "chain",
                "severity": "warning",
            },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let report = compare(
        &[("query-depth/chain".to_owned(), matchers[0].clone())],
        &[("a.ts".to_owned(), "const value = 1;".to_owned())],
    );
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
}

#[test]
fn wide_any_patterns_prepare_and_match() {
    let alternatives = (0..1001)
        .map(|index| serde_json::json!({ "pattern": format!("value{index}") }))
        .collect::<Vec<_>>();
    let document = serde_json::json!({
        "format": "rerail.policy/v1",
        "id": "wide-any",
        "revision": 1,
        "checks": [{
            "id": "wide",
            "forbid": { "ast": { "language": "typescript", "rule": { "any": alternatives } } },
            "finding": {
                "anchor": "witness.span",
                "code": "WIDE",
                "message": "wide",
                "severity": "warning",
            },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let report = compare(
        &[("wide-any".to_owned(), matchers[0].clone())],
        &[("a.ts".to_owned(), "const value1000 = 1;".to_owned())],
    );
    assert_eq!(report.matches, 1);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
}

fn query_budget_matches_matcher(
    matcher: &GrammarMatcher,
    source: String,
    budget: usize,
) -> rusqlite::Result<(usize, usize)> {
    let doc = tape(&source, GrammarDialect::TypeScript).expect("typescript tape");
    let mut matcher_work = budget;
    for node in AstGrep::doc(doc.clone()).root().dfs() {
        matcher
            .match_root(node, &mut matcher_work)
            .expect("matcher fits budget");
    }

    let mut builder = CatalogBuilder::default();
    let plan = compile_check(matcher, &mut builder, false).expect("compiles");
    let (terminals, catalog) = builder.finish();
    let connection = QueryConnection::open().expect("query connection");
    let batch = connection.register(
        Arc::new(terminals),
        vec![BatchFile::new(
            doc,
            Arc::from("a.ts"),
            Arc::from("TypeScript"),
            None,
        )],
        Arc::new(catalog),
    );
    let mut query_work = budget;
    batch.run(&plan, 0, None, matcher, &mut query_work)?;
    Ok((matcher_work, query_work))
}

#[test]
fn count_stops_after_the_minimum_is_met() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1",
        "id": "query-count-short-circuit",
        "revision": 1,
        "checks": [{
            "id": "count",
            "forbid": { "ast": {
                "language": "typescript",
                "rule": {
                    "kind": "program",
                    "count": { "has": { "kind": "expression_statement", "stopBy": "end" }, "min": 1 },
                },
            }},
            "finding": {
                "anchor": "witness.span", "code": "COUNT", "message": "count", "severity": "warning",
            },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let (matcher_work, query_work) =
        query_budget_matches_matcher(&matchers[0], "x;\n".repeat(20), 5)
            .expect("query fits budget");
    assert_eq!(query_work, matcher_work);
}

#[test]
fn three_valued_relation_stops_after_the_first_match() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1",
        "id": "query-relation-short-circuit",
        "revision": 1,
        "checks": [{
            "id": "relation",
            "forbid": { "ast": {
                "language": "typescript",
                "rule": { "kind": "program", "has": { "stopBy": "end", "matches": "statement" } },
                "utils": {
                    "statement": {
                        "any": [
                            { "kind": "expression_statement" },
                            { "testUnknown": "^never$" },
                        ],
                    },
                },
            }},
            "finding": {
                "anchor": "witness.span", "code": "RELATION", "message": "relation", "severity": "warning",
            },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    assert!(matchers[0].three_valued);
    let (matcher_work, query_work) =
        query_budget_matches_matcher(&matchers[0], "x;\n".repeat(20), 5)
            .expect("query fits budget");
    assert_eq!(query_work, matcher_work);
}

#[test]
fn count_with_max_zero_matches_a_leaf() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1", "id": "query-count-empty", "revision": 1,
        "checks": [{
            "id": "count",
            "forbid": { "ast": { "language": "typescript", "rule": {
                "kind": "identifier",
                "count": { "has": { "kind": "identifier", "stopBy": "end" }, "max": 0 },
            }}},
            "finding": { "anchor": "witness.span", "code": "COUNT", "message": "count", "severity": "warning" },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let report = compare(
        &[("count-empty".to_owned(), matchers[0].clone())],
        &[("a.ts".to_owned(), "x;".to_owned())],
    );
    assert_eq!(report.matches, 1);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
}

/// An unknown stop verdict ends a count's walk without saying how many nodes lie past it: the
/// count is unbounded above, so `max` cannot hold (the verdict is unknown, no match), while a
/// definite stop closes the walk and `max: 0` holds.
#[test]
fn an_unknown_stop_leaves_a_count_unbounded_above() {
    let unknown_stop = serde_json::json!({ "kind": "statement_block", "testUnknown": "." });
    let definite_stop = serde_json::json!({ "kind": "statement_block" });
    let has = |stop: &serde_json::Value| {
        serde_json::json!({ "kind": "function_declaration", "count": {
            "has": { "kind": "identifier", "regex": "^z$", "stopBy": stop }, "max": 0,
        } })
    };
    let inside = |stop: &serde_json::Value| {
        serde_json::json!({ "kind": "identifier", "regex": "^z$", "count": {
            "inside": { "kind": "function_declaration", "stopBy": stop }, "max": 0,
        } })
    };
    for (label, rule, matches) in [
        ("has-unknown-stop", has(&unknown_stop), 0),
        ("has-definite-stop", has(&definite_stop), 1),
        ("inside-unknown-stop", inside(&unknown_stop), 0),
        ("inside-definite-stop", inside(&definite_stop), 1),
    ] {
        let document = serde_json::json!({
            "format": "rerail.policy/v1", "id": "query-count-open", "revision": 1,
            "checks": [{
                "id": "count",
                "forbid": { "ast": { "language": "typescript", "rule": rule } },
                "finding": { "anchor": "witness.span", "code": "COUNT", "message": "count", "severity": "warning" },
            }],
        });
        let compiled =
            PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
                .unwrap();
        let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
            panic!("expected AST forbid check");
        };
        let report = compare(
            &[(label.to_owned(), matchers[0].clone())],
            &[("a.ts".to_owned(), "function f() { z; }\n".to_owned())],
        );
        assert_eq!(report.matches, matches, "{label}");
        assert!(report.differences.is_empty(), "{:#?}", report.differences);
    }
}

#[test]
fn three_valued_relation_stops_at_a_definite_stop() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1", "id": "query-relation-stop", "revision": 1,
        "checks": [{
            "id": "relation",
            "forbid": { "ast": { "language": "typescript", "rule": {
                "all": [
                    { "pattern": "x();" },
                    { "precedes": { "pattern": "later();", "stopBy": { "pattern": "stop();" } } },
                    { "testUnknown": "^never$" },
                ],
            }}},
            "finding": { "anchor": "witness.span", "code": "STOP", "message": "stop", "severity": "warning" },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let report = compare(
        &[("relation-stop".to_owned(), matchers[0].clone())],
        &[("a.ts".to_owned(), "x(); stop(); later();".to_owned())],
    );
    assert_eq!(report.matches, 0);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
}

#[test]
fn three_valued_field_relation_keeps_the_first_capture() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1", "id": "query-field-first", "revision": 1,
        "checks": [{
            "id": "field",
            "forbid": { "ast": { "language": "typescript", "rule": {
                "kind": "call_expression",
                "has": { "field": "arguments", "stopBy": "end", "all": [
                    { "kind": "identifier", "pattern": "$ARG" },
                    { "testUnknown": "^never$" },
                ]},
            }}},
            "finding": { "anchor": "capture.ARG", "code": "FIELD", "message": "field", "severity": "warning" },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let report = compare(
        &[("field-first".to_owned(), matchers[0].clone())],
        &[("a.ts".to_owned(), "f(a, b);".to_owned())],
    );
    assert_eq!(report.matches, 1);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
}

#[test]
fn utility_work_budget_matches_the_matcher() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1",
        "id": "query-work-budget",
        "revision": 1,
        "checks": [{
            "id": "slow",
            "forbid": {
                "ast": {
                    "language": "typescript",
                    "rule": { "kind": "program", "matches": "slow" },
                    "utils": {
                        "slow": {
                            "has": {
                                "stopBy": "end",
                                "all": [{ "matches": "slow" }, { "kind": "regex" }],
                            },
                        },
                    },
                },
            },
            "finding": {
                "anchor": "witness.span",
                "code": "SLOW",
                "message": "slow",
                "severity": "warning",
            },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let matcher = &matchers[0];
    let source = format!("{}x;{}\n", "{".repeat(30), "}".repeat(30));
    let doc = tape(&source, GrammarDialect::TypeScript).unwrap();

    let mut matcher_work = 10_000;
    let expected = AstGrep::doc(doc.clone())
        .root()
        .dfs()
        .find_map(|node| matcher.match_root(node, &mut matcher_work).err())
        .expect("matcher should exceed its utility work budget");
    assert_eq!(format!("{expected:?}"), "Work");

    let mut builder = CatalogBuilder::default();
    let plan = compile_check(matcher, &mut builder, false).unwrap();
    let (terminals, catalog) = builder.finish();
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(
        Arc::new(terminals),
        vec![BatchFile::new(
            doc,
            Arc::from("a.ts"),
            Arc::from("TypeScript"),
            None,
        )],
        Arc::new(catalog),
    );
    let mut query_work = 10_000;
    let actual = batch
        .run(&plan, 0, None, matcher, &mut query_work)
        .expect_err("statement should exceed its utility work budget");
    assert!(actual.to_string().contains("work budget"), "{actual}");
    assert_eq!(query_work, matcher_work);
}

/// A `matches` of a non-recursive structural utility is answered by the kernel: it spends no step
/// of the work budget, where the matcher spends one per call and runs out.
#[test]
fn structural_utilities_spend_no_work_budget() {
    let document = serde_json::json!({
        "format": "rerail.policy/v1",
        "id": "query-structural-budget",
        "revision": 1,
        "checks": [{
            "id": "blocked",
            "forbid": {
                "ast": {
                    "language": "typescript",
                    "rule": { "kind": "identifier", "matches": "in-block" },
                    "utils": {
                        "in-block": { "inside": { "stopBy": "end", "kind": "statement_block" } },
                    },
                },
            },
            "finding": {
                "anchor": "witness.span",
                "code": "BLOCKED",
                "message": "blocked",
                "severity": "warning",
            },
        }],
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let CheckKind::Forbid(Query::Ast { matchers, .. }) = &compiled.inner.checks[0].kind else {
        panic!("expected AST forbid check");
    };
    let matcher = &matchers[0];
    let identifiers = 50;
    let source = format!("{{ {} }}\n", "x;".repeat(identifiers));
    let doc = tape(&source, GrammarDialect::TypeScript).unwrap();

    let mut matcher_work = 10;
    let exceeded = AstGrep::doc(doc.clone())
        .root()
        .dfs()
        .find_map(|node| matcher.match_root(node, &mut matcher_work).err())
        .expect("the matcher spends a step per utility call");
    assert_eq!(format!("{exceeded:?}"), "Work");

    let mut builder = CatalogBuilder::default();
    let plan = compile_check(matcher, &mut builder, false).unwrap();
    let (terminals, catalog) = builder.finish();
    let connection = QueryConnection::open().unwrap();
    let batch = connection.register(
        Arc::new(terminals),
        vec![BatchFile::new(
            doc,
            Arc::from("a.ts"),
            Arc::from("TypeScript"),
            None,
        )],
        Arc::new(catalog),
    );
    let mut query_work = 10;
    let rows = batch
        .run(&plan, 0, None, matcher, &mut query_work)
        .expect("the kernel answers the utility without spending the budget");
    assert_eq!(rows.len(), identifiers);
    assert_eq!(query_work, 10);
}

/// `RERAIL_QUERY_PARITY_RULES`: `:`-separated policy directories; `RERAIL_QUERY_PARITY_ROOT` and
/// `RERAIL_QUERY_PARITY_MANIFEST`: a checkout and a JSON array of paths under it.
#[test]
#[ignore = "corpus parity; set RERAIL_QUERY_PARITY_{RULES,ROOT,MANIFEST}"]
fn compiled_catalog_matches_the_matcher_on_a_corpus() {
    let directories = std::env::var("RERAIL_QUERY_PARITY_RULES")
        .unwrap()
        .split(':')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let root = std::path::PathBuf::from(std::env::var("RERAIL_QUERY_PARITY_ROOT").unwrap());
    let manifest: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("RERAIL_QUERY_PARITY_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let files = manifest
        .iter()
        .filter_map(|path| Some((path.clone(), std::fs::read_to_string(root.join(path)).ok()?)))
        .collect::<Vec<_>>();
    let matchers = catalog_matchers(&directories);
    let started = std::time::Instant::now();
    let report = compare(&matchers, &files);
    eprintln!(
        "matchers={} compiled={} unsupported={} files={} matches={} differences={} elapsed={:?}",
        matchers.len(),
        report.compiled,
        report.unsupported.len(),
        report.files,
        report.matches,
        report.differences.len(),
        started.elapsed()
    );
    let statements = report
        .timings
        .values()
        .map(|(total, _, _)| *total)
        .sum::<std::time::Duration>();
    let slowest_files = report
        .timings
        .values()
        .map(|(_, worst, _)| *worst)
        .sum::<std::time::Duration>();
    let matcher = report.matcher_timings.values().sum::<std::time::Duration>();
    eprintln!(
        "TOTAL statements {statements:?} (slowest file per check {slowest_files:?}, which also \
         runs each terminal over the whole batch) matcher {matcher:?}"
    );
    for (label, reason) in &report.unsupported {
        eprintln!("unsupported {label}: {reason}");
    }
    for difference in report.differences.iter().take(40) {
        eprintln!("DIFF {difference}");
    }
    let mut slowest = report.timings.iter().collect::<Vec<_>>();
    slowest.sort_by_key(|(_, (total, _, _))| std::cmp::Reverse(*total));
    for (label, (total, worst, path)) in slowest.iter().take(20) {
        let matcher = report
            .matcher_timings
            .get(*label)
            .copied()
            .unwrap_or_default();
        eprintln!("TIME {total:?} matcher {matcher:?} worst {worst:?} {path} {label}");
    }
    assert!(report.differences.is_empty());
}

/// PHP's binding predicates (`redefines` over the bindings a flow reaches a binding with, in both
/// modes and under `sameScope`; `occurrence`; flow-decided `latest` and `outer`; `only` and
/// destructured values) give the compiled statements' answers, unknowns included.
#[test]
fn php_binding_predicates_match_the_matcher() {
    let rules = [
        serde_json::json!({ "kind": "variable_name", "binds": { "redefines": { "used": false } } }),
        serde_json::json!({ "kind": "variable_name", "binds": { "redefines": { "every": true, "sameScope": true, "form": "assignment", "site": { "pattern": "$S" } } } }),
        serde_json::json!({ "kind": "variable_name", "binds": { "redefines": { "sameScope": false } } }),
        serde_json::json!({ "kind": "variable_name", "binds": { "occurrence": { "inside": { "kind": "echo_statement", "stopBy": "end" }, "pattern": "$O" } } }),
        serde_json::json!({ "kind": "variable_name", "binds": { "latest": false } }),
        serde_json::json!({ "kind": "variable_name", "not": { "binds": { "outer": true } } }),
        serde_json::json!({ "kind": "variable_name", "resolves": { "binding": { "only": true, "value": { "pattern": "$V" } } } }),
        serde_json::json!({ "kind": "variable_name", "resolves": { "binding": { "destructuredValue": false, "value": { "kind": "integer" } } } }),
        serde_json::json!({ "kind": "variable_name", "resolves": { "binding": { "outer": false, "occurrence": { "inside": { "kind": "arrow_function", "stopBy": "end" } } } } }),
    ];
    let checks = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            serde_json::json!({
                "id": format!("php-{index}"),
                "forbid": { "ast": { "language": "php", "rule": rule } },
                "finding": { "anchor": "witness.span", "code": "PHP", "message": "php", "severity": "warning" }
            })
        })
        .collect::<Vec<_>>();
    let document = serde_json::json!({
        "format": "rerail.policy/v1", "id": "php-bindings", "revision": 1, "checks": checks
    });
    let compiled =
        PolicyDslRuntime::compile(document.to_string(), "json".into(), CAPABILITIES.into())
            .unwrap();
    let matchers = compiled
        .inner
        .checks
        .iter()
        .map(|check| {
            let CheckKind::Forbid(Query::Ast { matchers, .. }) = &check.kind else {
                panic!("expected AST forbid check");
            };
            (check.id.clone(), matchers[0].clone())
        })
        .collect::<Vec<_>>();
    let files = [
        "<?php\nfunction f(bool $c, array $items) {\n    global $g;\n    $g = 1;\n    $a = 1;\n    $a = 2;\n    [$x, $y] = [3, 4];\n    echo $a, $x, $y;\n    if ($c) { global $h; }\n    $h = 1;\n    foreach ($items as $item) { $last = $item; $last = 0; }\n    echo $last ?? 0;\n    $fn = fn() => $a + ($item = 2) + $item;\n    extract($items);\n    $z = 1;\n    $z = 2;\n}\n",
        "<?php\n$top = 1;\n$top = 2;\n$byReference = function () use (&$top) { $top = 3; };\n",
    ]
    .iter()
    .enumerate()
    .map(|(index, source)| (format!("{index}.php"), (*source).to_owned()))
    .collect::<Vec<_>>();
    let report = compare(&matchers, &files);
    assert!(report.differences.is_empty(), "{:#?}", report.differences);
    assert!(report.matches >= rules.len(), "{} matches", report.matches);
}
