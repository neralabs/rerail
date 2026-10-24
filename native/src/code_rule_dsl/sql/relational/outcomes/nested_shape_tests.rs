//! Admission measures emitted return payload; parity and work scale with reached walks.
use super::super::tests::measure;
use super::publication_tests::{check_work, policy as job_policy};
use super::tests::policy;
use crate::code_rule_dsl::test_support::grammar_matchers;
use serde_json::{Value, json};

fn rule(depth: usize, wide: bool) -> Value {
    let mut body = json!({"kind":"number","pattern":"$X","value":{"eq":1},"stopBy":"neighbor"});
    for index in 0..depth {
        body = if wide {
            json!({"kind":"array","pattern":format!("$A{index}"),"has":body,"stopBy":"neighbor"})
        } else {
            json!({"kind":"array","has":body,"stopBy":"neighbor"})
        };
    }
    json!({"kind":"arguments","has":body})
}
fn source(depth: usize, roots: usize) -> String {
    format!("f({}1{});", "[".repeat(depth), "]".repeat(depth)).repeat(roots)
}

#[test]
fn deeper_walks_have_exact_outcomes_captures_failures_and_work() {
    for depth in [4, 8, 16] {
        let rules = [rule(depth, false)];
        let p = job_policy(&rules);
        let source = source(depth, 2);
        for work in [0, 1, 2, 4, 8, 16, 32, 64, 128, i64::MAX] {
            check_work(&p, &rules, &[&source], None, work);
        }
        check_work(&p, &rules, &[&source], Some("1"), i64::MAX);
    }
}

#[test]
fn nested_admission_bounds_active_typed_cells_instead_of_walk_count() {
    let narrow = policy(rule(16, false), json!({}));
    let compiled = super::super::compile(&grammar_matchers(&narrow)[0]).unwrap();
    assert_eq!(compiled.relations["nestedReturnCellBudget"], 128);
    assert!(compiled.relations["nestedReturnCells"][0].as_u64().unwrap() > 4);
    let wide = policy(rule(16, true), json!({}));
    let error = super::super::compile(&grammar_matchers(&wide)[0])
        .err()
        .expect("wide active return chain exceeds storage budget");
    assert!(error.contains("128 active cells"), "{error}");
    let deep = policy(rule(48, false), json!({}));
    let error = super::super::compile(&grammar_matchers(&deep)[0])
        .err()
        .expect("narrow return chain remains bounded");
    assert!(
        error.contains("128 active cells") || error.contains("operator count"),
        "{error}"
    );
}

#[test]
fn deeper_correlated_sql_prepares_with_bounded_text_and_linear_root_work() {
    let mut samples = vec![];
    let mut refused = vec![];
    for depth in [2, 4, 8, 16, 32] {
        let p = policy(rule(depth, false), json!({}));
        let grammars = grammar_matchers(&p);
        if let Err(error) = super::super::compile(&grammars[0]) {
            assert!(error.contains("128 active cells"), "{error}");
            refused.push(json!({"depth":depth,"reason":error}));
            continue;
        }
        for roots in [1, 4, 16] {
            let source = source(depth, roots);
            let mut sample = measure(&grammars[0], &[("a.ts".into(), source)]);
            let bytes = sample["sql"].as_str().unwrap().len();
            assert!(bytes <= 256_000);
            assert!(
                sample["prepareCpuNs"].as_u64().is_some(),
                "preparation measured"
            );
            assert_eq!(sample["matchingStatements"], 1);
            assert_eq!(sample["framePublications"], 0);
            sample["depth"] = json!(depth);
            sample["roots"] = json!(roots);
            sample["sqlBytes"] = json!(bytes);
            sample.as_object_mut().unwrap().remove("sql");
            samples.push(sample);
        }
    }
    if let Ok(path) = std::env::var("RERAIL_NESTED_RETURN_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&json!({"samples":samples,"refused":refused})).unwrap(),
        )
        .unwrap();
    }
    for series in samples.chunks(3) {
        for pair in series.windows(2) {
            assert_eq!(pair[0]["sqlBytes"], pair[1]["sqlBytes"]);
            assert!(
                pair[1]["vmSteps"].as_u64().unwrap() <= 5 * pair[0]["vmSteps"].as_u64().unwrap(),
                "depth {}",
                pair[0]["depth"]
            );
        }
    }
    let depth_samples: Vec<_> = samples.iter().step_by(3).collect();
    for pair in depth_samples.windows(2) {
        assert!(pair[1]["sqlBytes"].as_u64().unwrap() <= 3 * pair[0]["sqlBytes"].as_u64().unwrap());
    }
}
