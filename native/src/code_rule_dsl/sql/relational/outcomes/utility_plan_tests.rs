use super::publication_tests::{check_work, policy_utils};
use crate::code_rule_dsl::model::{ClauseKind, Query};
use serde_json::{Value, json};

fn fixture(boolean: &str, verdict: &str, size: usize, wide: bool) -> (Value, Value, &'static str) {
    let kind = if verdict == "unknown" {
        "identifier"
    } else {
        "number"
    };
    let search = json!({"has":{"all":[{"kind":kind},{"matches":"v"}],"stopBy":"end"}});
    let mut rule = json!({"kind":"arguments"});
    rule[boolean] = json!(vec![search; size]);
    let predicates = [".*", "^", "$", ".", "^.*$", ".+", "[0-9xy]", "[^z]"];
    let mut body = if wide {
        predicates
            .iter()
            .map(|p| json!({"regex":p}))
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    body.push(json!({"matches":"inner"}));
    let utils = json!({"v":{"all":body},"inner":{"value":if verdict=="false" {json!({"eq":99})}else{json!({"gt":0})}}});
    (
        rule,
        utils,
        if verdict == "unknown" {
            "f(x,y);"
        } else {
            "f(1,2);"
        },
    )
}

#[test]
fn utility_plan_native_and_admission_boundaries() {
    use crate::code_rule_dsl::{
        matcher::{self, value_probe},
        sql::tests::differential::tape,
    };
    use ast_grep_core::AstGrep;
    let mut evidence = vec![];
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            let (rule, utils, source) = fixture(boolean, verdict, 32, true);
            let p = policy_utils(&[rule], utils);
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &p.inner.clauses[0].kind else {
                panic!("AST")
            };
            let grammar = &matchers[0];
            let boundary = match (boolean, verdict) {
                ("all", "true") => 96,
                ("any", "true") => 3,
                ("all", "false") => 6,
                _ => 192,
            };
            for allowance in [boundary - 1, boundary] {
                let doc = tape(grammar.language, source);
                let tree = AstGrep::doc(doc.clone());
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let mut work = allowance;
                let mut matches = 0;
                let (result, calls) = value_probe::run_reached(None, || {
                    matcher::with_file_memo(&tree, || {
                        matcher::run_clause(
                            &tree,
                            grammar,
                            None,
                            doc.transducer_len(),
                            &mut work,
                            |_, _, _| {
                                matches += 1;
                                Ok::<_, ()>(true)
                            },
                        )
                        .unwrap()
                    })
                });
                assert_eq!(result.is_ok(), allowance == boundary);
                assert_eq!(work, 0);
                assert_eq!(
                    calls,
                    if verdict == "true" {
                        vec!["1"]
                    } else if verdict == "false" {
                        vec!["1", "2"]
                    } else {
                        vec!["x", "y"]
                    }
                );
                if allowance == boundary {
                    assert_eq!(matches, usize::from(verdict == "true"));
                }
                evidence.push(json!({"boolean":boolean,"verdict":verdict,"allowance":allowance,"success":result.is_ok(),"remaining":work,"calls":calls,"matches":matches}));
            }
            let compiled = super::compile_job(&[grammar], 10000).unwrap();
            assert_eq!(compiled.terminals.len(), 9);
            assert_eq!(compiled.relations["lowering"][0]["utilityBodyLowerings"], 2);
            let (rule, utils, _) = fixture(boolean, verdict, 32, true);
            let rules = [rule];
            let p = policy_utils(&rules, utils);
            for allowance in [0, boundary - 1, boundary, boundary + 1] {
                check_work(&p, &rules, &[source, source], None, allowance as i64);
            }
        }
    }
    println!("UTILITY_PLAN_NATIVE {}", json!(evidence));
}

#[test]
fn utility_plan_profile() {
    let mut samples = vec![];
    for wide in [false, true] {
        for boolean in ["all", "any"] {
            for verdict in ["true", "false", "unknown"] {
                for size in [2, 8, 32] {
                    let (rule, utils, source) = fixture(boolean, verdict, size, wide);
                    let rules = [rule];
                    let p = policy_utils(&rules, utils);
                    let mut r = check_work(&p, &rules, &[source, source], None, 10000);
                    r["dimension"] = json!(format!(
                        "{boolean}-{verdict}-{}",
                        if wide { "wide" } else { "narrow" }
                    ));
                    r["size"] = json!(size);
                    samples.push(r);
                }
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_RELATIONAL_UTILITY_PLAN_EVIDENCE") {
        std::fs::write(path, serde_json::to_string_pretty(&samples).unwrap()).unwrap();
    }
}
#[test]
fn utility_plan_captures_fallbacks_and_failure_publication() {
    for boolean in ["all", "any"] {
        for verdict in ["true", "false", "unknown"] {
            let (rule, utils, source) = fixture(boolean, verdict, 3, true);
            for variant in [
                rule.clone(),
                json!({"kind":"arguments","pattern":"$INPUT","all":[rule.clone()]}),
                json!({"kind":"arguments","any":[rule.clone(),{"pattern":"$FALLBACK"}]}),
                json!({"kind":"arguments","not":rule}),
            ] {
                let rules = [variant, json!({"kind":"number","pattern":"$OTHER"})];
                let p = policy_utils(&rules, utils.clone());
                for allowance in 0..=20 {
                    check_work(&p, &rules, &[source, "f(g(1),2);h(3);"], None, allowance);
                }
                for allowance in [0, 2, 3, 6, 18, 100] {
                    check_work(
                        &p,
                        &rules,
                        &[source, "f(g(1),2);h(3);"],
                        Some("2"),
                        allowance,
                    );
                }
            }
        }
    }
}

#[test]
fn utility_plan_capturing_bodies_and_clause_identity() {
    for boolean in ["all", "any"] {
        let mut rule = json!({"kind":"arguments"});
        rule[boolean] = json!(vec![
            json!({"has":{"all":[{"kind":"number"},{"matches":"v"}],"stopBy":"end"}});
            3
        ]);
        // Each clause owns a separate utility table with the same source names.
        let rules = [rule.clone(), rule];
        let p = policy_utils(
            &rules,
            json!({"v":{"pattern":"$CAP","matches":"inner"},"inner":{"value":{"gt":0}}}),
        );
        for allowance in 0..=20 {
            let r = check_work(&p, &rules, &["f(g(1),2);", "f(3);"], None, allowance);
            for clause in r["relations"]["lowering"].as_array().unwrap() {
                assert_eq!(clause["distinctUtilityBodies"], 2);
                assert_eq!(clause["utilityBodyLowerings"], 2);
                assert_eq!(clause["atomicTerminals"], 1);
            }
        }
    }
}

#[test]
fn utility_plan_bounds_and_cycles() {
    let mut cases = vec![
        (
            json!({"matches":"v"}),
            json!({"v":{"any":[{"testUnknown":"u"},{"inside":{"stopBy":"neighbor","has":{"stopBy":"neighbor","matches":"v"}}}]}}),
            "recursive outcomes",
        ),
        (
            json!({"matches":"v"}),
            json!({"v":{"any":[{"testUnknown":"u"},{"inside":{"stopBy":"neighbor","matches":"w"}}]},"w":{"has":{"stopBy":"neighbor","matches":"v"}}}),
            "recursive outcomes",
        ),
        (
            json!({"all":(0..257).map(|n|json!({"regex":format!("^.{{0,{n}}}$")})).collect::<Vec<_>>()}),
            json!({}),
            "distinct lowered operator count",
        ),
    ];
    let mut utils = json!({"u0":{"regex":"."}});
    for n in 1..=9 {
        utils[format!("u{n}")] =
            json!({"all":[{"matches":format!("u{}",n-1)},{"matches":format!("u{}",n-1)}]});
    }
    cases.push((
        json!({"matches":"u9"}),
        utils,
        "emitted composition operator count",
    ));
    for (rule, utils, expected) in cases {
        let p = policy_utils(&[rule], utils);
        let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &p.inner.clauses[0].kind else {
            panic!("AST")
        };
        let error = super::compile_job(&[&matchers[0]], 10000)
            .err()
            .expect("bounded rejection");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn utility_plan_single_clause_entry_and_shared_identity() {
    use std::{collections::BTreeMap, rc::Rc};
    fn visit(plan: &super::Plan, seen: &mut BTreeMap<usize, Rc<super::Plan>>, reuses: &mut usize) {
        match &plan.op {
            super::Op::Charge(id, body) => {
                if let Some(prior) = seen.get(id) {
                    assert!(Rc::ptr_eq(prior, body));
                    *reuses += 1;
                } else {
                    seen.insert(*id, Rc::clone(body));
                    visit(body, seen, reuses);
                }
            }
            super::Op::All(items) | super::Op::Any(items) => {
                for child in items {
                    visit(child, seen, reuses);
                }
            }
            super::Op::Search(_, _, child, _) => visit(child, seen, reuses),
            _ => {}
        }
    }
    let (rule, utils, _) = fixture("all", "true", 32, true);
    let p = policy_utils(&[rule], utils);
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &p.inner.clauses[0].kind else {
        panic!("AST")
    };
    let grammar = &matchers[0];
    let mut lower = super::lower::Lower::new(grammar.language, grammar.three_valued);
    let plan = lower.plan(&grammar.matcher).unwrap();
    let mut seen = BTreeMap::new();
    let mut reuses = 0;
    visit(&plan, &mut seen, &mut reuses);
    assert_eq!(seen.len(), 2);
    assert_eq!(reuses, 31);
    let single = super::compile(grammar).unwrap();
    let job = super::compile_job(&[grammar], i64::MAX).unwrap();
    assert_eq!(single.sql, job.sql);
    assert_eq!(single.relations["lowering"], job.relations["lowering"]);
    assert_eq!(single.terminals.len(), 9);
}
