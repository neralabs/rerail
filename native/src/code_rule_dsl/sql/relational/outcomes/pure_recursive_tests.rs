//! Pure SCC execution preserves utility charges and stores only live numeric continuations.
use super::publication_tests::{check_work, policy_utils};
use crate::code_rule_dsl::{
    sql::{exec, tests::differential::tape},
    test_support::grammar_matchers,
};
use serde_json::{Value, json};

fn universal() -> Value {
    json!({"any":[{"kind":"identifier","regex":"^a$"},{"kind":"array","not":{"has":{"stopBy":"neighbor","not":{"any":[{"regex":"^[\\[\\],]$"},{"kind":"comment"},{"matches":"u"}]}}}}]})
}
fn multi_arm() -> Value {
    json!({"any":[{"kind":"identifier","regex":"^a$"},{"kind":"array","has":{"stopBy":"neighbor","not":{"regex":"^[\\[\\],]$"},"matches":"u"}},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"u"}}]})
}
#[test]
fn pure_recursive_parent_wrappers_and_multiarm_children_keep_exact_budgets() {
    let cases = [
        (
            json!({"kind":"identifier","regex":"^x$","matches":"u"}),
            json!({"u":{"inside":{"stopBy":"neighbor","any":[{"kind":"arguments"},{"kind":"parenthesized_expression","matches":"u"}]}}}),
            vec!["f(x,((x))); x;", "f((((x))));"],
        ),
        (
            json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}}),
            json!({"u":multi_arm()}),
            vec!["f([b,[a],b]); f(([b]));", "f([[b],b]); f(a);"],
        ),
        (
            json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}}),
            json!({"u":universal()}),
            vec!["f([a,[a,a]]); f([a,[b],a]);", "f([]); f([/*x*/a]);"],
        ),
    ];
    for (rule, utils, sources) in cases {
        let rules = [rule];
        let p = policy_utils(&rules, utils);
        for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 64, 128, 1024, i64::MAX] {
            check_work(&p, &rules, &sources, None, work);
        }
    }
}
#[test]
fn pure_recursive_mutual_calls_allow_only_acyclic_zero_movement() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let utils = json!({"u":{"any":[{"kind":"identifier","regex":"^a$"},{"kind":"array","has":{"stopBy":"neighbor","matches":"v"}}]},"v":{"matches":"u"}});
    let p = policy_utils(&rules, utils);
    for work in [0, 1, 2, 3, 4, 8, 16, 32, 64, 1024, i64::MAX] {
        check_work(&p, &rules, &["f([b,[a]]); f([[b]]);"], None, work);
    }
}
#[test]
fn pure_recursive_rule_stops_prune_subtrees_after_body() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let utils = json!({"u":{"any":[{"kind":"identifier","regex":"^a$"},{"kind":"array","has":{"stopBy":{"kind":"object"},"matches":"u"}}]}});
    let p = policy_utils(&rules, utils);
    for work in [0, 1, 2, 4, 8, 16, 32, 128, i64::MAX] {
        check_work(&p, &rules, &["f([{x:a},b]); f([[[a]],b]);"], None, work);
    }
}
#[test]
fn pure_recursive_field_filters_skip_impossible_utility_entries() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let utils = json!({"u":{"any":[{"kind":"identifier","regex":"^a$"},{"kind":"call_expression","has":{"field":"function","matches":"u"}}]}});
    let p = policy_utils(&rules, utils);
    // A member-expression callee is outside u's necessary kinds. Its field test
    // must fail before entering u, while identifier/call callees may recurse.
    for work in [0, 1, 2, 3, 4, 5, 8, 16, 32, 64, 1024, i64::MAX] {
        check_work(
            &p,
            &rules,
            &["f(obj.method()); f(b()); f(a()); f((() => {})());"],
            None,
            work,
        );
    }
}

fn storage_metrics(p: &crate::code_rule_dsl::CompiledCodeRuleDsl, source: &str) -> Value {
    let grammars = grammar_matchers(p);
    let compiled = super::compile_job(&[&grammars[0]], i64::MAX).unwrap();
    let bytes = compiled.sql.len();
    let (mut metrics, frames) = crate::code_rule_dsl::sql::profiling::frame_counts(|| {
        exec::with_terminals(
            vec![(tape(grammars[0].language, source), Default::default())],
            compiled.terminals,
            |db, batch, _| {
                super::super::leaves::register(db, batch).unwrap();
                let start = std::time::Instant::now();
                let mut statement = db.prepare(&compiled.sql).unwrap();
                let prepare = start.elapsed().as_nanos();
                let rows = statement
                    .query_map([], |_| Ok(()))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                let vm = statement.get_status(rusqlite::StatementStatus::VmStep);
                let live: i64 = db
                    .query_row("SELECT relation_continuation_rows()", [], |r| r.get(0))
                    .unwrap();
                let peak: i64 = db
                    .query_row("SELECT relation_continuation_peak()", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(live, 0, "completed and failed roots release their stacks");
                json!({"sqlBytes":bytes,"prepareWallNs":prepare,"vmSteps":vm,"rows":rows.len(),"peakRecords":peak,"liveRecords":live})
            },
        )
    });
    metrics["nodeAdvances"] = json!(frames[11]);
    metrics["nodeFilters"] = json!(frames[13]);
    metrics
}
#[test]
fn pure_recursive_machine_has_constant_sql_and_linear_root_and_depth_work() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let p = policy_utils(&rules, json!({"u":universal()}));
    let mut samples = vec![];
    for depth in [1, 4, 16] {
        for roots in [1, 4, 16] {
            let source = format!("f({}a{});", "[".repeat(depth), "]".repeat(depth)).repeat(roots);
            check_work(&p, &rules, &[&source], None, i64::MAX);
            let mut sample = storage_metrics(&p, &source);
            sample["depth"] = json!(depth);
            sample["roots"] = json!(roots);
            samples.push(sample);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_PURE_RECURSIVE_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
            assert_eq!(pair[0]["peakRecords"], pair[1]["peakRecords"]);
            assert!(
                pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap()
            );
        }
    }
    for pair in samples.iter().step_by(3).collect::<Vec<_>>().windows(2) {
        assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
        assert!(pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap());
        assert!(
            pair[1]["peakRecords"].as_u64().unwrap()
                <= 5 * pair[0]["peakRecords"].as_u64().unwrap()
        );
    }
}

#[test]
fn pure_recursive_nested_sibling_seek_has_linear_cursor_work() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let p = policy_utils(&rules, json!({"u":multi_arm()}));
    let mut samples = vec![];
    for width in [8, 32, 128] {
        let source = format!("f([{},[[a]]]);", vec!["[[b]]"; width].join(","));
        check_work(&p, &rules, &[&source], None, i64::MAX);
        let mut sample = storage_metrics(&p, &source);
        sample["width"] = json!(width);
        samples.push(sample);
    }
    if let Ok(path) = std::env::var("RERAIL_PURE_SIBLING_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
    for pair in samples.windows(2) {
        assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
        assert!(pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap());
        assert!(
            pair[1]["nodeAdvances"].as_u64().unwrap()
                <= 5 * pair[0]["nodeAdvances"].as_u64().unwrap()
        );
    }
}

#[test]
fn pure_recursive_contiguous_predicates_fuse_without_crossing_utility_effects() {
    let rules = [json!({"kind":"arguments","has":{"stopBy":"neighbor","matches":"u"}})];
    let utils = json!({"u":{"any":[
        {"all":[{"kind":"identifier"},{"regex":"^[ab]$"},{"not":{"regex":"^b$"}},{"matches":"v"}]},
        {"all":[{"regex":"^z$"},{"matches":"v"},{"kind":"identifier"},{"regex":"^never$"}]},
        {"kind":"array","has":{"stopBy":"neighbor","matches":"u"}}
    ]},"v":{"kind":"identifier","regex":"^[az]$"}});
    let p = policy_utils(&rules, utils);
    let mut last = Value::Null;
    for work in [0, 1, 2, 3, 4, 5, 6, 7, 8, 16, 32, 128, i64::MAX] {
        last = check_work(&p, &rules, &["f([z,b,a]); f([z,b]);"], None, work);
    }
    fn reduced(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                object
                    .get("pureControl")
                    .is_some_and(|p| p["unfusedInstructions"].as_u64() > p["instructions"].as_u64())
                    || object.values().any(reduced)
            }
            Value::Array(values) => values.iter().any(reduced),
            _ => false,
        }
    }
    assert!(
        reduced(&last["relations"]),
        "mixed Boolean pure runs reduced actual control instructions"
    );
}
