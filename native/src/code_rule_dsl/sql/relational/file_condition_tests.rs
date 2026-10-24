//! File predicates select original documents before roots, charges, or atomic probes.
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    matcher::{self, Stopped, value_probe},
    module_path::ModulePath,
    sql::{exec, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};
use std::sync::Arc;

fn policy(
    language: &str,
    file: Option<Value>,
    rule: Value,
) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    let mut ast = json!({"language":language,"rule":rule});
    if let Some(file) = file {
        ast["file"] = file;
    }
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"file-selection","revision":1,"mode":"steer","clauses":[{"id":"c0","forbid":{"ast":ast},"violation":{"code":"C0","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}
fn check(
    language: &str,
    condition: Value,
    rule: Value,
    files: &[(&str, &str, Option<ModulePath>)],
    allowance: usize,
) {
    let p = policy(language, Some(condition), rule);
    let grammar = &grammar_matchers(&p)[0];
    let docs: Vec<_> = files
        .iter()
        .map(|(_, source, _)| tape(grammar.language, source))
        .collect();
    let mut expected = vec![];
    let mut native_calls = vec![];
    let mut selected = vec![];
    for (file, (doc, (path, _, module))) in docs.iter().zip(files).enumerate() {
        let applies =
            grammar.applies && grammar.file.as_ref().unwrap().holds(path, module.as_ref());
        selected.push(applies);
        let tree = AstGrep::doc(doc.clone());
        let _scopes = matcher::FileScopeSlot::install(
            Default::default(),
            module.clone().map(Arc::new),
            false,
        )
        .path(path);
        let mut work = allowance;
        let mut rows = vec![];
        let (outcome, calls) = value_probe::run_reached(None, || {
            if !applies {
                return Ok(());
            }
            matcher::with_file_memo(&tree, || {
                matcher::run_clause(
                    &tree,
                    grammar,
                    None,
                    doc.transducer_len(),
                    &mut work,
                    |node, _, left| {
                        rows.push(json!([file, node, 0, *left]));
                        Ok::<_, ()>(true)
                    },
                )
                .unwrap()
            })
        });
        match outcome {
            Ok(()) => expected.extend(rows),
            Err(Stopped::Limit(limit)) => expected.push(json!([
                file,
                null,
                super::failure::encode(&format!("{limit:?}")).unwrap(),
                work
            ])),
            Err(Stopped::Deadline) => panic!("unexpected deadline"),
        }
        if !applies {
            assert_eq!(work, allowance);
            assert!(calls.is_empty());
        }
        native_calls.push(calls);
    }
    let actual_compile = super::compile(grammar).unwrap();
    assert!(actual_compile.sql.contains("relation_file_guard("));
    let compiled = if allowance == i64::MAX as usize {
        actual_compile
    } else {
        super::outcomes::compile_job(&[grammar], allowance as i64).unwrap()
    };
    let before = super::file_guard::counts();
    let ((actual, _), calls) = super::budget_leaf::trace(|| {
        value_probe::run_reached(None, || {
            exec::with_terminals(
                docs.into_iter()
                    .zip(files)
                    .map(|(doc, (path, _, module))| {
                        (
                            doc,
                            exec::FileContext {
                                path: Some((*path).into()),
                                module: module.clone().map(Arc::new),
                                ..Default::default()
                            },
                        )
                    })
                    .collect(),
                compiled.terminals,
                |db, batch, _| {
                    super::leaves::register(db, batch).unwrap();
                    db.prepare(&compiled.sql)
                        .unwrap()
                        .query_map([], |row| {
                            let tag = row.get::<_, i64>(4)?;
                            let captures: Value =
                                serde_json::from_str(&row.get::<_, String>(3)?).unwrap();
                            assert_eq!(captures, json!([]));
                            Ok(json!([
                                row.get::<_, u32>(1)?,
                                if tag < 0 {
                                    None
                                } else {
                                    Some(row.get::<_, u32>(2)?)
                                },
                                tag,
                                row.get::<_, i64>(5)?
                            ]))
                        })
                        .unwrap()
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .unwrap()
                },
            )
        })
    });
    let after = super::file_guard::counts();
    assert_eq!(
        [after[0] - before[0], after[1] - before[1]],
        [1, files.len()]
    );
    let mut actual = actual;
    actual.sort_by_key(|row| (row[0].as_u64(), row[1].as_u64()));
    expected.sort_by_key(|row| (row[0].as_u64(), row[1].as_u64()));
    assert_eq!(actual, expected);
    for (file, expected) in native_calls.iter().enumerate() {
        let actual: Vec<_> = calls
            .iter()
            .filter(|call| call["file"] == json!(file))
            .flat_map(|call| call["valueCalls"].as_array().into_iter().flatten().cloned())
            .collect();
        assert_eq!(json!(actual), json!(expected));
        if !selected[file] {
            assert!(!calls.iter().any(|call| call["file"] == json!(file)));
        }
    }
}

#[test]
fn python_file_metadata_conditions_cover_every_key_and_missing_module() {
    let files = [
        ("pkg/mod.py", "x=1\nx\n", ModulePath::dotted("pkg.mod")),
        ("mod.py", "x=1\nx\n", ModulePath::stem("mod.py")),
        ("missing.py", "x=1\nx\n", None),
        ("pkg/Other.pyi", "x: int\n", ModulePath::dotted("pkg.Other")),
    ];
    for condition in [
        json!({"path":{"regex":"\\.py$","notRegex":"missing"}}),
        json!({"module":{"regex":"^pkg\\.","notRegex":"Other$"}}),
        json!({"module":{"notRegex":"never"}}),
        json!({"moduleName":{"regex":"^mod$","notRegex":"^Other$"}}),
        json!({"package":{"regex":"^pkg$","notRegex":"^bad$"}}),
        json!({"inPackage":true}),
        json!({"inPackage":false}),
        json!({"path":{"regex":"\\.py$"},"module":{"regex":"^pkg\\.mod$"},"moduleName":{"regex":"^mod$"},"package":{"regex":"^pkg$"},"inPackage":true}),
    ] {
        check(
            "python",
            condition,
            json!({"kind":"identifier","regex":"^x$"}),
            &files,
            0,
        );
    }
}

#[test]
fn declaration_file_selection_uses_native_case_and_suffix_rules() {
    let files = [
        ("a.d.ts", "const x=1;x;", None),
        ("a.D.CTS", "const x=1;x;", None),
        ("a.ts", "const x=1;x;", None),
        ("a.d.extra.ts", "const x=1;x;", None),
    ];
    for declaration in [true, false] {
        check(
            "typescript",
            json!({"declaration":declaration}),
            json!({"kind":"identifier","regex":"^x$"}),
            &files,
            0,
        );
    }
}

#[test]
fn excluded_python_files_do_not_consume_work_or_value_probes() {
    let files = [
        ("run.py", "x=1\nx\n", ModulePath::stem("run.py")),
        ("skip.pyi", "x=1\nx\n", ModulePath::stem("skip.pyi")),
        ("skip.py", "x=1\nx\n", None),
    ];
    for allowance in [0, 1, 2, 8, 128, i64::MAX as usize] {
        check(
            "python",
            json!({"path":{"regex":"^run\\.py$"}}),
            json!({"kind":"identifier","regex":"^x$","value":{"follow":true,"eq":1}}),
            &files,
            allowance,
        );
    }
}

#[test]
fn absent_file_condition_keeps_sql_and_terminals_unchanged() {
    let p = policy("python", None, json!({"kind":"identifier","regex":"^x$"}));
    let grammar = &grammar_matchers(&p)[0];
    let first = super::compile(grammar).unwrap();
    let mut equivalent = grammar.clone();
    equivalent.file = None;
    let second = super::compile(&equivalent).unwrap();
    assert_eq!(first.sql, second.sql);
    assert_eq!(first.terminals.len(), second.terminals.len());
    assert!(!first.sql.contains("relation_file_guard"));
}
