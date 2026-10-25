//! The native oracle compares captures, callback work, final reached failures,
//! and atomic probes. Timings/VM are evidence, not asserted speedups.
use super::super::super::{parse_tests, publication_tests};
use crate::code_rule_dsl::{CompiledCodeRuleDsl, test_support::grammar_matchers};
use serde_json::{Value, json};
use std::io::Write;

fn record(label: &str, r: &Value) {
    if let Ok(path) = std::env::var("RERAIL_PARSED_RELATION_SPIKE_EVIDENCE") {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f,"{}",json!({"case":label,"allowance":r["allowance"],"output":r["output"],"nativeTraces":r["nativeTraces"],"budgetCalls":r["budgetCalls"],"metrics":r["metrics"],"sqlBytes":r["sql"].as_str().unwrap().len(),"contract":r["relations"]["evidence"]})).unwrap();
    }
}
fn check(p: &CompiledCodeRuleDsl, rule: &Value, sources: &[&str], allowance: i64) -> Value {
    let r = publication_tests::check_work_compiler(
        p,
        std::slice::from_ref(rule),
        sources,
        None,
        allowance,
        |grammars, work| {
            let compiled = super::compile(grammars, work)?;
            if let Ok(path) = std::env::var("RERAIL_PARSED_RELATION_SPIKE_EVIDENCE") {
                std::fs::write(
                    std::path::Path::new(&path).with_extension("sql"),
                    &compiled.sql,
                )
                .unwrap();
            }
            Ok(compiled)
        },
    );
    record("local-document-frontier", &r);
    let observed = diagnostics(p, sources, allowance);
    for row in observed.as_array().unwrap() {
        let file = row[0].as_u64().unwrap() as usize;
        assert_eq!(
            row[4], r["nativeTraces"][file]["remaining"],
            "final allowance includes failed suffix roots"
        );
    }
    if let Ok(path) = std::env::var("RERAIL_PARSED_RELATION_SPIKE_EVIDENCE") {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f,"{}",json!({"case":"fresh-context-frontier-counters","allowance":allowance,"columns":["file","rootProducers","parseRequests","localBaseProducers","remaining"],"rows":observed,"timingIncludedInMatchingProfile":false})).unwrap();
    }
    r
}
fn diagnostics(p: &CompiledCodeRuleDsl, sources: &[&str], allowance: i64) -> Value {
    use crate::code_rule_dsl::sql::relational::leaves;
    use crate::code_rule_dsl::sql::{exec, tests::differential::tape};
    let gs = grammar_matchers(p);
    let compiled = super::compile(&[&gs[0]], allowance).unwrap();
    let query = compiled.relations["frontierDiagnosticsSql"]
        .as_str()
        .unwrap()
        .to_string();
    let docs = sources
        .iter()
        .map(|s| (tape(gs[0].language, s), Default::default()))
        .collect();
    exec::with_terminals(docs, compiled.terminals, |db, batch, _| {
        leaves::register(db, batch).unwrap();
        let mut stmt = db.prepare(&query).unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok(json!([
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?
                ]))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        json!(rows)
    })
}
fn fixture(first: bool) -> (CompiledCodeRuleDsl, Value) {
    let utility = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"comparison_operator","capture":"C","has":{"stopBy":"neighbor","regex":"^is$","capture":"O"}},{"matches":"walk"}]}}}});
    let root = if first {
        json!({"any":[{"kind":"comparison_operator","pattern":"$C","has":{"stopBy":"neighbor","regex":"^is$"}},{"matches":"walk"}]})
    } else {
        json!({"matches":"walk"})
    };
    (parse_tests::policy("python", root.clone(), utility), root)
}
fn nested(payload: &str, depth: usize) -> String {
    let mut inner = payload.to_string();
    for _ in 0..depth {
        inner = format!("list[{}]", serde_json::to_string(&inner).unwrap());
    }
    format!("a: {}\n", serde_json::to_string(&inner).unwrap())
}
#[test]
fn structural_spike_local_frontier_preserves_nested_exports_and_host_identity() {
    let (p, rule) = fixture(true);
    let deep = nested("x is 1", 5);
    let sources = [
        "x is 1\na: 'x is 1'\nb: 'x == 1'\n",
        "a: 'tuple[\"x is 1\",\"y is 2\"]'\nb: 'list[\"x is 1\"]'\n",
        "a: 'x is 1'\nx='x is 1'\nb: 'x is 1'\nc: 'not valid !'\n",
        &deep,
    ];
    for allowance in [0, 1, 2, 3, 5, 8, 13, 20, 32, 64, 128, 1000, 10000] {
        check(&p, &rule, &sources, allowance);
    }
}
#[test]
fn structural_spike_reached_exhaustion_includes_last_failed_roots() {
    let (p, rule) = fixture(false);
    let sources = [
        "a: 'x is 1'\nb: 'list[\"x == 1\"]'\nx = ordinary + ordinary\n",
        "",
        "x = ordinary + ordinary\n",
    ];
    let r = check(&p, &rule, &sources, 10000);
    let spent = 10000 - r["nativeTraces"][0]["remaining"].as_i64().unwrap();
    for allowance in [spent - 1, spent, spent + 1] {
        check(&p, &rule, &sources, allowance);
    }
}
#[test]
fn structural_spike_width_profiles_broad_and_selective_populations() {
    let utility = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"comparison_operator","capture":"C","has":{"stopBy":"neighbor","regex":"^is$","capture":"O"}},{"matches":"walk"}]}}}});
    let rule = json!({"any":[{"regex":"^NEVER$"},{"matches":"walk"}]});
    let p = parse_tests::policy("python", rule.clone(), utility);
    for width in [1, 8, 32] {
        let irrelevant = (0..width)
            .map(|i| format!("v{i} = ordinary + ordinary\n"))
            .collect::<String>();
        let source = format!("{irrelevant}a: 'list[\"x is 1\"]'\n");
        let r = check(&p, &rule, &[&source], 100000);
        let baseline = publication_tests::check_work_compiler(
            &p,
            std::slice::from_ref(&rule),
            &[&source],
            None,
            100000,
            super::super::super::compile_job,
        );
        record(&format!("baseline-broad-{width}"), &baseline);
        record(&format!("frontier-broad-{width}"), &r);
        let grammars = grammar_matchers(&p);
        assert!(
            grammars[0].anchors.root_kinds().is_none(),
            "this is the broad population control"
        );
        // A selective domain comes from a different native grammar, not a SQL
        // filter that silently skips original broad-root entry charges.
        let selective = json!({"kind":"string","matches":"walk"});
        let utility = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"comparison_operator","capture":"C"},{"matches":"walk"}]}}}});
        let q = parse_tests::policy("python", selective.clone(), utility);
        let selective_result = check(&q, &selective, &[&source], 100000);
        record(&format!("frontier-selective-{width}"), &selective_result);
    }
}
#[test]
fn structural_spike_actual_f632_root_and_annotation_helpers() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packs/python/rules/ruff-f632-is-literal.json");
    let policy: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let ast = &policy["clauses"][0]["forbid"]["ast"];
    let rule = ast["rule"].clone();
    let p = parse_tests::policy("python", rule.clone(), ast["utils"].clone());
    let deep = nested("x is (1,2)", 4);
    let sources = [
        "x is 1\nx is None\nx is []\nx is (1, 2)\n",
        "a: 'x is 1'\nb: 'list[\"x is []\"]'\nc: 'x is None'\n",
        "x='x is 1'\na: 'x is 1'\nb: 'not valid !'\nc: 'list[\"x == 1\"]'\n",
        &deep,
    ];
    for allowance in [0, 1, 2, 3, 5, 8, 13, 32, 64, 128, 1000, 10000] {
        check(&p, &rule, &sources, allowance);
    }
    let control = publication_tests::check_work_compiler(
        &p,
        std::slice::from_ref(&rule),
        &sources,
        None,
        10000,
        super::super::super::compile_job,
    );
    record("baseline-actual-f632", &control);
}
#[test]
fn structural_spike_repeated_original_parse_utility_is_explicitly_refused() {
    let (p, _) = fixture(false);
    let grammars = grammar_matchers(&p);
    assert!(super::compile(&[&grammars[0]], 10000).is_ok());
    let u = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","capture":"C"},{"matches":"walk"}]}}}});
    let r = json!({"any":[{"matches":"walk"},{"matches":"walk"}]});
    let q = parse_tests::policy("python", r, u);
    let gs = grammar_matchers(&q);
    assert!(
        super::compile(&[&gs[0]], 10000)
            .err()
            .unwrap()
            .contains("repeat")
    );
}

#[test]
fn structural_spike_full_f632_fixture_batch() {
    use crate::code_rule_dsl::sql::relational::cutover_tests::execution;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let policy: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("packs/python/rules/ruff-f632-is-literal.json"))
            .unwrap(),
    )
    .unwrap();
    let ast = &policy["clauses"][0]["forbid"]["ast"];
    let p = parse_tests::policy("python", ast["rule"].clone(), ast["utils"].clone());
    let grammars = grammar_matchers(&p);
    let compiled = super::compile(&[&grammars[0]], i64::MAX).unwrap();
    use sha2::{Digest, Sha256};
    let sql_bytes = compiled.sql.len();
    let sql_sha256 = format!("{:x}", Sha256::digest(compiled.sql.as_bytes()));
    let full_sql_path = std::env::var("RERAIL_PARSED_RELATION_SPIKE_EVIDENCE")
        .ok()
        .map(|path| std::path::Path::new(&path).with_extension("full-f632.sql"));
    if let Some(path) = &full_sql_path {
        std::fs::write(path, &compiled.sql).unwrap();
        let persisted = std::fs::read(path).unwrap();
        assert_eq!(persisted.len(), sql_bytes);
        assert_eq!(format!("{:x}", Sha256::digest(&persisted)), sql_sha256);
    }
    let fixtures = execution::fixtures(root);
    assert_eq!(compiled.relations["evidence"]["rejectedRootCost"], json!(2));
    assert_eq!(
        compiled.relations["evidence"]["bulkOriginalRoots"],
        json!(true)
    );
    let r = execution::check(&grammars[0], compiled, &fixtures).unwrap();
    assert_eq!(r["sqlSha256"], json!(sql_sha256));
    assert_eq!(
        r["passed"],
        json!(true),
        "full F632 fixture transfer failed: {r}"
    );
    assert!(
        r["nativeMatches"].as_u64().unwrap() > 0,
        "full corpus requires positive witnesses"
    );
    if let Ok(path) = std::env::var("RERAIL_PARSED_RELATION_SPIKE_EVIDENCE") {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(
            f,
            "{}",
            json!({"case":"full-f632-fixture-batch","files":fixtures.len(),"sqlBytes":sql_bytes,"sqlSha256":sql_sha256,"sqlPath":full_sql_path,"result":r})
        )
        .unwrap();
    }
}

#[test]
fn structural_spike_unknown_and_utility_depth_are_reached_failures() {
    let root = json!({"matches":"walk"});
    let unknown = json!({"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","value":{"eq":1},"capture":"C"},{"matches":"walk"}]}}}});
    let p = parse_tests::policy("python", root.clone(), unknown);
    for allowance in [0, 1, 2, 8, 32, 1000] {
        check(
            &p,
            &root,
            &["a: 'tuple[x,y]'\nb: 'list[\"x\"]'\n"],
            allowance,
        );
    }
    let utilities = json!({"climb":{"any":[{"kind":"module"},{"inside":{"stopBy":"neighbor","matches":"climb"}}]},"walk":{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","matches":"climb","capture":"C"},{"matches":"walk"}]}}}});
    let p = parse_tests::policy("python", root.clone(), utilities);
    for depth in [3, 260] {
        let source = format!("a: '{}x{}'\n", "(".repeat(depth), ")".repeat(depth));
        for allowance in [1, 8, 100000] {
            check(&p, &root, &[&source], allowance);
        }
    }
}

#[test]
fn structural_spike_00_small_positive_native_contract() {
    let (p, rule) = fixture(false);
    let sources = ["a: 'x is 1'\n"];
    let override_path = std::env::var("RERAIL_PARSED_RELATION_SPIKE_SQL_OVERRIDE").ok();
    let r = publication_tests::check_work_compiler(
        &p,
        std::slice::from_ref(&rule),
        &sources,
        None,
        10000,
        |grammars, work| {
            let mut compiled = super::compile(grammars, work)?;
            if let Some(path) = &override_path {
                compiled.sql = std::fs::read_to_string(path).unwrap();
            }
            if let Ok(path) = std::env::var("RERAIL_PARSED_RELATION_SPIKE_EVIDENCE") {
                std::fs::write(
                    std::path::Path::new(&path).with_extension("small.sql"),
                    &compiled.sql,
                )
                .unwrap();
            }
            Ok(compiled)
        },
    );
    assert_eq!(r["output"].as_array().unwrap().len(), 1);
    assert_eq!(r["output"][0][4], json!(0));
    if override_path.is_some() {
        // SQL replay is a diagnostic only, never a qualification of the emitter.
        println!("PARSED_SQL_OVERRIDE_DIAGNOSTIC {}", r["output"]);
    } else {
        assert_eq!(
            diagnostics(&p, &sources, 10000)[0][4],
            r["nativeTraces"][0]["remaining"]
        );
        record("small-positive-native-gate", &r);
    }
}

#[test]
fn structural_spike_01_continuation_root_and_guarded_return_primitive() {
    use crate::code_rule_dsl::sql::relational::{continuation_store, row};
    let db = rusqlite::Connection::open_in_memory().unwrap();
    let _store = continuation_store::register(&db).unwrap();
    row::register(&db).unwrap();
    let query = "WITH initial AS MATERIALIZED (SELECT f.handle FROM relation_continuation f WHERE f.mode IS CASE WHEN ?1 THEN 1 END AND f.v0 IS 9), reached AS MATERIALIZED (SELECT handle FROM initial) SELECT p.parent,p.c0 FROM reached s CROSS JOIN relation_continuation p ON p.mode IS CASE WHEN ?1 THEN 2 END AND p.h IS s.handle";
    for active in [0, 1, 0, 1] {
        let rows = db
            .prepare(query)
            .unwrap()
            .query_map([active], |r| {
                Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        if active == 1 {
            assert_eq!(rows, vec![(None, 9)]);
        } else {
            assert!(rows.is_empty());
        }
        assert_eq!(
            db.query_row("SELECT relation_continuation_rows()", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
