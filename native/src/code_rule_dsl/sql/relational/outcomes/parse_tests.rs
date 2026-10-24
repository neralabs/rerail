//! Parse boundaries preserve native status and work while SQL composes child rules.
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    matcher::{self, Stopped},
    sql::{exec, profiling, tests::differential::tape},
    test_support::{CAPABILITIES, grammar_matchers},
};
use ast_grep_core::AstGrep;
use serde_json::{Value, json};

pub(super) fn policy(
    language: &str,
    rule: Value,
    utils: Value,
) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(
        json!({"format":"rerail.policy/v1","id":"catalog-atoms","revision":1,"mode":"steer",
        "clauses":[{"id":"c0","forbid":{"ast":{"language":language,"rule":rule,"utils":utils}},
        "violation":{"code":"C0","message":"match","anchor":"witness.span"}}]})
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap()
}

/// Compare the clause's roots, remaining allowance and failure publication on every file.
/// These rules bind nothing. Unlike the Value-only probe helper, this accepts other leaf kinds.
fn check_parse_budget(
    language: &str,
    rule: Value,
    utils: Value,
    sources: &[&str],
    allowance: usize,
) -> Value {
    let policy = policy(language, rule.clone(), utils);
    let grammar = &grammar_matchers(&policy)[0];
    let docs: Vec<_> = sources.iter().map(|s| tape(grammar.language, s)).collect();
    let mut expected = vec![];
    for (file, doc) in docs.iter().enumerate() {
        let tree = AstGrep::doc(doc.clone());
        let _scopes =
            matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        let mut work = allowance;
        let mut rows = vec![];
        let result = matcher::with_file_memo(&tree, || {
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
        });
        match result {
            Ok(()) => expected.extend(rows),
            Err(Stopped::Limit(limit)) => expected.push(json!([
                file,
                null,
                super::super::failure::encode(&format!("{limit:?}")).unwrap(),
                work
            ])),
            Err(Stopped::Deadline) => panic!("unexpected deadline"),
        }
    }
    let compiled = super::compile_job(&[grammar], allowance as i64).unwrap();
    let ((actual, executions), frames) = profiling::frame_counts(|| {
        exec::with_terminals(
            docs.into_iter()
                .map(|doc| (doc, Default::default()))
                .collect(),
            compiled.terminals,
            |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                let mut stmt = db.prepare(&compiled.sql).unwrap();
                let trace = super::super::tests::ExecutionTrace::start(db);
                let rows = stmt
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
                    .unwrap();
                (rows, trace.count())
            },
        )
    });
    let mut actual = actual;
    actual.sort_by_key(|row| (row[0].as_u64(), row[1].as_u64()));
    expected.sort_by_key(|row| (row[0].as_u64(), row[1].as_u64()));
    assert_eq!(
        actual, expected,
        "rule={rule}, allowance={allowance}, sources={sources:?}"
    );
    assert_eq!(executions, 1);
    assert_eq!(frames[4], 0);
    json!(actual)
}

#[test]
fn parse_child_walk_value_and_failure_budget_parity() {
    let sources = &["a='xyz'\nb='x\\x79'\nc='zz'\n", "a=f'{unknown}'\nb=123\n"];
    for child in [
        json!({"kind":"text","has":{"kind":"char","regex":"^y$"}}),
        json!({"has":{"kind":"char","value":{"eq":"z"},"inside":{"kind":"text"},"precedes":{"value":{"eq":"z"}}}}),
        json!({"count":{"has":{"kind":"char"},"min":3},"not":{"inside":{"kind":"text"}}}),
    ] {
        let mut parse = child;
        parse["grammar"] = json!("testChars");
        for allowance in [0, 1, 3, 12, 1000] {
            check_parse_budget(
                "python",
                json!({"kind":"string","parse":parse}),
                json!({}),
                sources,
                allowance,
            );
        }
    }
}

#[test]
fn format_parse_error_values_and_reached_cache_preserve_work() {
    for grammar in ["percentFormat", "braceFormat"] {
        let parse = json!({"grammar":grammar,"has":{"kind":"error","value":{"regex":"."}}});
        let atom = json!({"kind":"string","parse":parse});
        for allowance in [0, 2, 10, 1000] {
            check_parse_budget(
                "python",
                json!({"all":[{"any":[atom.clone(),atom.clone()]},{"not":{"not":atom.clone()}}]}),
                json!({}),
                &["a='%'\nb='{'\nc='%q'\nd='{}'\ne='fine'\n"],
                allowance,
            );
        }
    }
}

#[test]
fn parsed_documents_keep_language_specific_error_values() {
    for (language, grammar, kind, source) in [
        (
            "php",
            "printfFormat",
            "string",
            "<?php $a = '%'; $b = '%q'; $c = '%s';",
        ),
        (
            "php",
            "scanfFormat",
            "string",
            "<?php $a = '%'; $b = '%q'; $c = '%d';",
        ),
        (
            "rust",
            "rustFormat",
            "string_literal",
            "fn main() { let a = \"{\"; let b = \"{}\"; }",
        ),
    ] {
        for allowance in [0, 3, 1000] {
            check_parse_budget(
                language,
                json!({"kind":kind,"parse":{"grammar":grammar,"has":{"kind":"error","value":{"regex":"."}}}}),
                json!({}),
                &[source],
                allowance,
            );
        }
    }
}

#[test]
fn annotation_parse_uses_host_scope_and_preserves_nonannotation_false() {
    let atom = json!({"kind":"string","parse":{"grammar":"annotation","has":{"kind":"identifier","regex":"^int$","stopBy":"end"}}});
    for allowance in [0, 1, 20, 1000, 10000] {
        check_parse_budget(
            "python",
            atom.clone(),
            json!({}),
            &[
                "from typing import cast\na: 'int' = 1\nb = 'int'\nc = cast('int', 1)\ndef f(x: 'int') -> 'int': ...\n",
            ],
            allowance,
        );
    }
}

#[test]
fn annotation_pattern_and_nested_parse_use_parsed_document_identity() {
    for child in [
        json!({"has":{"pattern":"int","stopBy":"end"}}),
        json!({"has":{"kind":"string","parse":{"grammar":"annotation","has":{"kind":"identifier","regex":"^int$","stopBy":"end"}},"stopBy":"end"}}),
    ] {
        let mut parse = child;
        parse["grammar"] = json!("annotation");
        for allowance in [0, 40, 10000] {
            check_parse_budget(
                "python",
                json!({"kind":"string","parse":parse}),
                json!({}),
                &["from typing import List\na: 'int'\nb: 'List[\"int\"]'\nc = 'int'\n"],
                allowance,
            );
        }
    }
}

#[test]
fn parsed_value_probes_keep_host_file_and_utility_replay_contract() {
    let atom =
        json!({"parse":{"grammar":"percentFormat","has":{"kind":"error","value":{"regex":"."}}}});
    let rule =
        json!({"kind":"string","any":[{"all":[{"matches":"p"},{"matches":"p"}]},{"matches":"p"}]});
    let p = policy("python", rule.clone(), json!({"p":atom}));
    for allowance in [0, 1, 2, 3, 10, 64, 1000] {
        super::publication_tests::check_work(
            &p,
            std::slice::from_ref(&rule),
            &["a='%'\nb='%q'\nc='fine'\n", "a='%'\n"],
            None,
            allowance,
        );
    }
}

#[test]
fn parsed_value_utilities_never_recall_native_host_memo_answers() {
    let atom = json!({"value":{"regex":"."}});
    for rule in [
        json!({"kind":"string","parse":{"grammar":"annotation","has":{"kind":"identifier","all":[{"matches":"v"},{"matches":"v"}],"stopBy":"end"}}}),
        json!({"kind":"string","all":[{"matches":"v"},{"parse":{"grammar":"annotation","has":{"kind":"identifier","any":[{"matches":"v"},{"matches":"v"}],"stopBy":"end"}}}]}),
    ] {
        let p = policy("python", rule.clone(), json!({"v":atom}));
        for allowance in [0, 1, 2, 3, 10, 64, 1000] {
            super::publication_tests::check_work(
                &p,
                std::slice::from_ref(&rule),
                &["a: 'int'\nb: 'str'\nc='plain'\n"],
                None,
                allowance,
            );
        }
    }
}
