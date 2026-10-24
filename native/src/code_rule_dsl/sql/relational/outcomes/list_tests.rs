//! Integrated list contracts: every sample first compares exact native outcomes/captures.
use super::super::tests::measure;
use super::publication_tests::{check_work, policy, policy_utils};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};

fn parity(rule: Value, sources: &[&str]) -> Value {
    let p = super::tests::policy(rule, json!({}));
    measure(
        &grammar_matchers(&p)[0],
        &sources
            .iter()
            .enumerate()
            .map(|(i, s)| (format!("{i}.ts"), (*s).into()))
            .collect::<Vec<_>>(),
    )
}
fn checked(rules: &[Value], sources: &[&str]) -> Value {
    check_work(&policy(rules), rules, sources, None, i64::MAX)
}

#[test]
fn list_patterns_preserve_empty_punctuation_multiple_and_repeated_captures() {
    for (rule, source) in [
        (
            json!({"pattern":"f($$$A)"}),
            "f(); f(a); f(a, obj.x, [b,c]); g(a);",
        ),
        (
            json!({"pattern":"f([$$$A], [$$$B])"}),
            "f([],[]); f([a,b],[c]); f([x],[y,z]);",
        ),
        (
            json!({"pattern":"f([$$$A], [$$$A])"}),
            "f([],[]); f([a,b],[a,b]); f([a,b],[a,c]);",
        ),
    ] {
        let report = parity(rule, &[source, source]);
        assert!(report["matches"].as_u64().unwrap() > 0);
        assert_eq!(report["matchingStatements"], 1);
        assert_eq!(report["framePublications"], 0);
    }
}

#[test]
fn list_inputs_unify_before_alignment_and_boolean_branches_restore_bindings() {
    let sources = ["f(a,b); f(); g(a,b); f(obj.x,obj.y);", "f(c); g();"];
    for rule in [
        json!({"kind":"call_expression","all":[{"pattern":"f($$$A)"},{"pattern":"f($$$A)"}]}),
        json!({"kind":"call_expression","any":[{"all":[{"pattern":"f($$$A)"},{"regex":"^never$"}]},{"pattern":"f($$$B)"}]}),
        json!({"kind":"call_expression","all":[{"not":{"all":[{"pattern":"f($$$A)"},{"regex":"^never$"}]}},{"pattern":"f($$$B)"}]}),
        json!({"kind":"statement_block","all":[{"has":{"pattern":"f($$$A)","stopBy":"end"}},{"has":{"pattern":"g($$$A)","stopBy":"end"}}]}),
    ] {
        checked(&[rule], &sources);
    }
    checked(
        &[
            json!({"kind":"statement_block","all":[{"has":{"pattern":"f($$$A)","stopBy":"end"}},{"has":{"pattern":"g($$$A)","stopBy":"end"}}]}),
        ],
        &[
            "{ f(a,b); g(a,b); } { f(); g(); } { f(a); g(b); }",
            "{ f(obj.x); g(obj.x); }",
        ],
    );
}

#[test]
fn policy_rejects_constraints_on_list_capture_targets() {
    let result = CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"list-constraint","revision":1,"mode":"steer","clauses":[{"id":"c0","forbid":{"ast":{"language":"typescript","rule":{"pattern":"f($$$A)"},"constraints":{"A":{"regex":"^never$"}}}},"violation":{"code":"C0","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into());
    let error = match result {
        Ok(_) => panic!("list target constraint must be rejected by policy validation"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("captures a node list; constraints apply to single-node captures only"),
        "{error}"
    );
}

#[test]
fn nested_list_walks_keep_stop_captures_private_and_count_probes_isolated() {
    let rules = [
        json!({"kind":"statement_block","has":{"kind":"call_expression","pattern":"target($$$A)","stopBy":{"pattern":"barrier($$$PRIVATE)"}}}),
        json!({"kind":"statement_block","has":{"kind":"statement_block","has":{"pattern":"target($$$A)","stopBy":"end"},"stopBy":"end"}}),
        json!({"kind":"statement_block","count":{"has":{"pattern":"target($$$PROBE)","stopBy":"end"},"min":0,"max":10,"as":"N"}}),
    ];
    let report = checked(
        &rules,
        &[
            "{ target(); target(a,b); barrier(x,y); target(c); } { { target(d); } }",
            "{ barrier(); target(e); } { empty(); }",
        ],
    );
    assert!(!report["output"].as_array().unwrap().is_empty());
}

#[test]
fn list_utility_memo_replay_preserves_environment_work_and_file_failures() {
    let rules = [
        json!({"kind":"call_expression","all":[{"pattern":"f($$$A)"},{"matches":"v"},{"not":{"not":{"matches":"v"}}},{"matches":"v"}]}),
        json!({"pattern":"g($$$B)"}),
    ];
    let utils = json!({"v":{"all":[{"pattern":"f($$$A)"},{"has":{"kind":"arguments","has":{"kind":"number","value":{"gt":0},"stopBy":"end"},"stopBy":"neighbor"}}]}});
    let p = policy_utils(&rules, utils);
    let sources = ["f(1,2); f(); g(3);", "f(4); g();"];
    for work in [0, 1, 2, 4, 8, 16, 32, i64::MAX] {
        check_work(&p, &rules, &sources, None, work);
    }
    let first = check_work(&p, &rules, &sources, Some("2"), i64::MAX);
    let second = check_work(&p, &rules, &sources, Some("2"), i64::MAX);
    assert_eq!(first["output"], second["output"]);
}

#[test]
fn real_pickle_policy_compiles_and_matches_native_import_call_fixtures() {
    let p = CodeRuleDslRuntime::compile(
        include_str!("../../../../../../packs/python/rules/bandit-b301-pickle.json").into(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let report = measure(
        &grammar_matchers(&p)[0],
        &[
            (
                "a.py".into(),
                "import pickle\npickle.loads(data)\npickle.load(stream)\nsafe(data)\n".into(),
            ),
            (
                "b.py".into(),
                "from pickle import loads\nloads(payload)\nloads()\n".into(),
            ),
        ],
    );
    assert_eq!(report["matchingStatements"], 1);
    assert_eq!(report["framePublications"], 0);
}

#[test]
fn list_pattern_demand_is_independent_of_unrelated_ast_growth() {
    let p = super::tests::policy(json!({"pattern":"f($$$A)"}), json!({}));
    let grammars = grammar_matchers(&p);
    for size in [32, 128, 512] {
        let mut source = String::from("f(a,b); g();\n");
        for i in 0..size {
            source.push_str(&format!("let unused{i}={i};\n"));
        }
        let report = measure(&grammars[0], &[("demand.ts".into(), source)]);
        assert_eq!(report["patternRequests"], 2, "{report}");
        assert_eq!(report["nodeAdvances"], 2, "{report}");
        assert_eq!(report["matchingStatements"], 1);
        assert_eq!(report["framePublications"], 0);
    }
}

#[test]
fn list_length_changes_data_without_expanding_query_or_recursive_cells() {
    let p = super::tests::policy(json!({"pattern":"f($$$A)"}), json!({}));
    let grammars = grammar_matchers(&p);
    let mut query = None;
    for size in [0, 1, 8, 32, 128] {
        let args = (0..size)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let report = measure(
            &grammars[0],
            &[("length.ts".into(), format!("f({args}); g();"))],
        );
        if let Some(prior) = &query {
            assert_eq!(prior, &report["sql"]);
        } else {
            query = Some(report["sql"].clone());
        }
        assert_eq!(report["patternRequests"], 2);
        assert_eq!(report["nodeAdvances"], 2);
        assert!(report["vmSteps"].as_u64().unwrap() < 100_000, "{report}");
        assert!(
            report["typedReturnCells"].as_u64().unwrap() < 1_000,
            "{report}"
        );
        assert_eq!(report["matchingStatements"], 1);
        assert_eq!(report["framePublications"], 0);
    }
}

#[test]
fn flat_ordered_list_alternatives_prepare_with_linear_query_growth() {
    let mut samples = Vec::new();
    for width in [8, 16, 32] {
        let alternatives = (0..width)
            .map(|i| json!({"pattern":format!("candidate{i}($$$A)")}))
            .collect::<Vec<_>>();
        let report = parity(
            json!({"any": alternatives}),
            &[
                &format!("candidate{}(a,b); candidate0(); miss(c);", width - 1),
                &format!("candidate{}(obj.x);", width - 1),
            ],
        );
        assert_eq!(report["matches"], 3);
        assert_eq!(report["matchingStatements"], 1);
        assert_eq!(report["framePublications"], 0);
        let sql_bytes = report["sql"].as_str().unwrap().len();
        let vm = report["vmSteps"].as_u64().unwrap();
        samples.push(json!({"alternatives":width,"sqlBytes":sql_bytes,"vmSteps":vm,"patternRequests":report["patternRequests"],"typedReturnCells":report["typedReturnCells"]}));
        assert!(sql_bytes <= 256_000, "{samples:?}");
        assert!(vm < 1_000_000, "{samples:?}");
    }
    eprintln!("LIST_ALTERNATIVE_SCALING {}", json!(samples));
    for pair in samples.windows(2) {
        let before = pair[0]["sqlBytes"].as_u64().unwrap();
        let after = pair[1]["sqlBytes"].as_u64().unwrap();
        assert!(after <= 2 * before + 4096, "{samples:?}");
        let before_vm = pair[0]["vmSteps"].as_u64().unwrap();
        let after_vm = pair[1]["vmSteps"].as_u64().unwrap();
        assert!(after_vm <= 3 * before_vm + 4096, "{samples:?}");
    }
}

#[test]
fn real_pycrypto_flat_alternatives_prepare_and_match_native_lists() {
    let p = CodeRuleDslRuntime::compile(
        include_str!("../../../../../../packs/python/rules/bandit-b413-import-pycrypto.json")
            .into(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let report = measure(&grammar_matchers(&p)[0], &[
        ("imports.py".into(), "import Crypto.Cipher\nimport Crypto.Hash as digest\nfrom Crypto.IO import PEM, PKCS8\nfrom Crypto import Protocol as protocol\nimport safe\n".into()),
        ("late-imports.py".into(), "from Crypto.Util import number, Counter\nimport Crypto.Util as util\nfrom Crypto import Util\nfrom safe import value\n".into()),
    ]);
    assert_eq!(report["matches"], 7, "{report}");
    assert_eq!(report["matchingStatements"], 1);
    assert_eq!(report["framePublications"], 0);
    assert!(report["sql"].as_str().unwrap().len() <= 256_000);
    eprintln!(
        "LIST_REAL_B413 {}",
        json!({"matches":report["matches"],"sqlBytes":report["sql"].as_str().unwrap().len(),"vmSteps":report["vmSteps"],"patternRequests":report["patternRequests"]})
    );
}

#[test]
fn real_pycrypto_import_demand_ignores_unrelated_ast_growth() {
    let p = CodeRuleDslRuntime::compile(
        include_str!("../../../../../../packs/python/rules/bandit-b413-import-pycrypto.json")
            .into(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let grammars = grammar_matchers(&p);
    let imports = "import Crypto.Cipher\nimport Crypto.Hash as digest\nfrom Crypto.IO import PEM, PKCS8\nfrom Crypto import Protocol as protocol\nfrom Crypto.Util import number, Counter\nimport Crypto.Util as util\nfrom Crypto import Util\nimport safe\nfrom safe import value\n";
    let mut samples = Vec::new();
    for size in [32, 128, 512] {
        let mut source = imports.to_owned();
        for i in 0..size {
            source.push_str(&format!("unused{i} = {i}\n"));
        }
        let report = measure(&grammars[0], &[("pycrypto-demand.py".into(), source)]);
        assert_eq!(report["matches"], 7, "{report}");
        assert_eq!(report["matchingStatements"], 1);
        assert_eq!(report["framePublications"], 0);
        samples.push(json!({"unrelatedDeclarations":size,"patternRequests":report["patternRequests"],"nodeAdvances":report["nodeAdvances"],"nodeFilters":report["nodeFilters"],"vmSteps":report["vmSteps"],"sqlBytes":report["sql"].as_str().unwrap().len()}));
    }
    eprintln!("LIST_B413_DEMAND_SCALING {}", json!(samples));
    for sample in &samples {
        // This counter includes root admission AND point metadata lookups in
        // scalar pattern steps. Bound both by roots times alternatives, then
        // require invariance under unrelated AST growth.
        assert!(
            sample["nodeAdvances"].as_u64().unwrap() <= 9 * 41,
            "{samples:?}"
        );
        assert!(
            sample["patternRequests"].as_u64().unwrap() <= 9 * 40,
            "{samples:?}"
        );
        assert!(sample["vmSteps"].as_u64().unwrap() <= 30_000, "{samples:?}");
        assert_eq!(
            sample["nodeAdvances"], samples[0]["nodeAdvances"],
            "{samples:?}"
        );
        assert_eq!(
            sample["patternRequests"], samples[0]["patternRequests"],
            "{samples:?}"
        );
        assert_eq!(sample["vmSteps"], samples[0]["vmSteps"], "{samples:?}");
        assert_eq!(sample["sqlBytes"], samples[0]["sqlBytes"], "{samples:?}");
    }
}
