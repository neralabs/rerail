//! Admission is structural; symbols and capture names are deliberately varied.
use super::super::super::{parse_tests, publication_tests};
use crate::code_rule_dsl::{
    CodeRuleDslRuntime, CompiledCodeRuleDsl,
    test_support::{CAPABILITIES, grammar_matchers},
};
use serde_json::{Value, json};

fn compile(
    g: &[&crate::code_rule_dsl::matcher::GrammarMatcher],
    work: i64,
) -> Result<super::super::super::Compiled, String> {
    super::try_compile(g, work, 0, 0)?
        .ok_or_else(|| "test shape unexpectedly outside parsed frontier contract".into())
}
fn end_work(p: &CompiledCodeRuleDsl, sources: &[(String, String)], work: i64) -> Vec<(u32, i64)> {
    use crate::code_rule_dsl::sql::{exec, tests::differential::tape};
    let g = grammar_matchers(p);
    let c = compile(&[&g[0]], work).unwrap();
    let query = c.relations["workDiagnosticsSql"]
        .as_str()
        .unwrap()
        .to_string();
    let docs = sources
        .iter()
        .map(|(path, source)| {
            (
                tape(g[0].language, source),
                exec::FileContext {
                    path: (!path.is_empty()).then(|| path.clone()),
                    ..Default::default()
                },
            )
        })
        .collect();
    exec::with_terminals(docs, c.terminals, |db, batch, _| {
        crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
        let mut stmt = db.prepare(&query).unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    })
}
fn fixture(symbol: &str, capture: &str) -> (CompiledCodeRuleDsl, Value, Value) {
    let rule = json!({"any":[{"regex":"^NEVER$"},{"matches":symbol}]});
    let utils = json!({(symbol):{"any":[{"kind":"string"},{"kind":"concatenated_string"}],"parse":{"grammar":"annotation","has":{"stopBy":"end","any":[{"kind":"identifier","regex":"^Witness$","capture":capture},{"matches":symbol}]}}}});
    (
        parse_tests::policy("python", rule.clone(), utils.clone()),
        rule,
        utils,
    )
}
#[test]
fn parsed_frontier_common_names_captures_and_finite_work_are_generic() {
    for (symbol, capture) in [
        ("descend_types", "FOUND"),
        ("scan_text", "RESULT"),
        ("walk_nested", "NODE"),
    ] {
        let (p, rule, _) = fixture(symbol, capture);
        for work in [0, 1, 2, 3, 5, 8, 16, 32, 1000] {
            let r = publication_tests::check_work_compiler(
                &p,
                std::slice::from_ref(&rule),
                &["a: 'list[\"Witness\"]'\nb: 'int'\n", "x = 'Witness'\n", ""],
                None,
                work,
                compile,
            );
            assert_eq!(
                r["relations"]["physicalShape"],
                json!("parsed document frontier")
            );
            assert!(r["relations"]["lowering"][0]["loweredOperators"].is_number());
            assert!(r["relations"]["nestedReturnCells"][0].as_u64().unwrap() <= 128);
            let sources = ["a: 'list[\"Witness\"]'\nb: 'int'\n", "x = 'Witness'\n", ""]
                .iter()
                .map(|source| (String::new(), source.to_string()))
                .collect::<Vec<_>>();
            for (file, left) in end_work(&p, &sources, work) {
                assert_eq!(json!(left), r["nativeTraces"][file as usize]["remaining"]);
            }
        }
    }
}
#[test]
fn parsed_frontier_namespace_allocates_before_lowering() {
    let (p, rule, _) = fixture("recurse_annotation", "TEXT");
    publication_tests::check_work_compiler(
        &p,
        std::slice::from_ref(&rule),
        &["a: 'list[\"Witness\"]'\n"],
        None,
        1000,
        |g, work| {
            let mut c = super::try_compile(g, work, 7, 11)?.expect("namespace shape");
            assert!(c.terminals.len() > 7);
            assert!(
                c.relations["terminalOwners"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|v| v == 11)
            );
            // The fixture oracle represents one clause; this typed boundary changes
            // only its external clause label, while native term owner stays12.
            c.sql = format!(
                "WITH answer(clause,file,node,payload,failure,work) AS ({}) SELECT 0,file,node,payload,failure,work FROM answer",
                c.sql
            );
            c.relations["terminalOwners"] = json!(vec![0; c.terminals.len()]);
            Ok(c)
        },
    );
}
#[test]
fn parsed_frontier_false_applies_does_not_open_atomic_parse_or_utility_work() {
    let (p, _, _) = fixture("find_types", "HIT");
    let mut g = grammar_matchers(&p)[0].clone();
    g.applies = false;
    let c = super::try_compile(&[&g], 0, 0, 0)
        .unwrap()
        .expect("lowered false applicability");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let fixtures = [
        crate::code_rule_dsl::sql::relational::cutover_tests::execution::Fixture {
            path: root.join("false.py").display().to_string(),
            source: "a: 'Witness'\n".into(),
        },
    ];
    let r =
        crate::code_rule_dsl::sql::relational::cutover_tests::execution::check(&g, c, &fixtures)
            .unwrap();
    assert_eq!(r["passed"], true, "{r}");
    assert_eq!(r["nativeRows"], 0);
}
#[test]
fn parsed_frontier_unsupported_first_unknown_or_repeated_self_stays_unadmitted() {
    let (p, _, _) = fixture("walk", "C");
    let mut g = grammar_matchers(&p)[0].clone();
    if let crate::code_rule_dsl::matcher::AstMatcher::Any(arms) = &mut g.matcher {
        arms[0] = arms[1].clone();
    }
    assert!(super::try_compile(&[&g], 1000, 0, 0).unwrap().is_none());
    assert!(super::try_compile(&[], 1000, 0, 0).unwrap().is_none());
    assert!(super::try_compile(&[&g], -1, 0, 0).is_err());
}
#[test]
fn parsed_frontier_common_full_reference_native_gate_and_true_size_budget() {
    use crate::code_rule_dsl::sql::relational::cutover_tests::execution;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let source: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("packs/python/rules/ruff-f632-is-literal.json"))
            .unwrap(),
    )
    .unwrap();
    let ast = &source["clauses"][0]["forbid"]["ast"];
    let p = parse_tests::policy("python", ast["rule"].clone(), ast["utils"].clone());
    let gs = grammar_matchers(&p);
    let original = super::super::parsed_relation_spike::compile(&[&gs[0]], i64::MAX).unwrap();
    let c = compile(&[&gs[0]], i64::MAX).unwrap();
    // Historical7831 bytes came from an overwritten selective-control dump.
    // Compare directly against the actual full reference statement instead.
    let reference_bytes = original.sql.len();
    assert!(
        c.sql.len() * 4 <= reference_bytes * 5,
        "common={} reference={reference_bytes}",
        c.sql.len()
    );
    let fixtures = execution::fixtures(root);
    let common_bytes = c.sql.len();
    let r = execution::check(&gs[0], c, &fixtures).unwrap();
    assert_eq!(r["passed"], true, "{r}");
    assert!(r["nativeMatches"].as_u64().unwrap() > 0);
    assert!(r["vm"].as_u64().unwrap() <= 24_034_427, "{r}");
    let sources = fixtures
        .iter()
        .map(|f| (f.path.clone(), f.source.clone()))
        .collect::<Vec<_>>();
    for (file, left) in end_work(&p, &sources, i64::MAX) {
        assert_eq!(
            json!(left),
            r["nativeFileTraces"][file as usize]["remainingWork"],
            "full F632 final allowance file={file}"
        );
    }
    println!(
        "PARSED_FRONTIER_COMMON {}",
        json!({"referenceSqlBytes":reference_bytes,"commonSqlBytes":common_bytes,"vm":r["vm"],"fullFiles":fixtures.len(),"nativeMatches":r["nativeMatches"],"historical7831ArtifactWasSelectiveControl":true})
    );
}
#[test]
fn parsed_frontier_common_job_uses_independent_clause_and_term_namespaces() {
    let (_, rule, utils) = fixture("inner_types", "MATCHED");
    let clauses=(0..2).map(|i|json!({"id":format!("c{i}"),"forbid":{"ast":{"language":"python","rule":rule,"utils":utils}},"violation":{"code":"C","message":"match","anchor":"witness.span"}})).collect::<Vec<_>>();
    let p=CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"frontier-job","revision":1,"mode":"steer","clauses":clauses}).to_string(),"json".into(),CAPABILITIES.into()).unwrap();
    let rules = [rule.clone(), rule];
    publication_tests::check_work_compiler(
        &p,
        &rules,
        &["a: 'list[\"Witness\"]'\n"],
        None,
        i64::MAX,
        |grammars, _| {
            let routed = super::super::super::compile_namespaced(grammars[0], 0, 0)?;
            assert_eq!(
                routed.relations["physicalShape"],
                json!("parsed document frontier"),
                "common namespaced route must select the proved representation"
            );
            let mut packs =
                crate::code_rule_dsl::sql::relational::job::compile(grammars, &Default::default())?;
            assert_eq!(packs.len(), 1);
            let pack = packs.pop().unwrap();
            assert_eq!(pack.arms.len(), 2);
            Ok(super::super::super::Compiled {
                sql: pack.sql,
                relations: json!({"terminalOwners":vec![0;pack.terminals.len()]}),
                terminals: pack.terminals,
            })
        },
    );
}
