use super::{graph::Graph, recursive_machine, run};
use crate::code_rule_dsl::{
    language::GrammarDialect,
    sql::{
        self,
        exec::{Captured, FileContext, SqlMatch},
        lower::ProgramBuilder,
        plan::SlotKind,
        tests::{differential::tape, hard_cases::compile, same, snapshot},
    },
    test_support::grammar_matchers,
};
use serde_json::{Value, json};

fn check(language: &str, policies: Vec<Value>, sources: &[&str], cycle: bool) {
    let compiled: Vec<_> = policies
        .into_iter()
        .map(|ast| compile(language, ast))
        .collect();
    let dialect = if language == "python" {
        GrammarDialect::Python
    } else {
        GrammarDialect::TypeScript
    };
    if !cycle {
        let paths: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| (language, format!("f{i}.src"), *s))
            .collect();
        let refs: Vec<_> = paths.iter().map(|(l, p, s)| (*l, p.as_str(), *s)).collect();
        for policy in &compiled {
            same(policy, &snapshot(&refs));
        }
    }
    let grammars: Vec<_> = compiled.iter().map(|p| &grammar_matchers(p)[0]).collect();
    let docs: Vec<_> = sources
        .iter()
        .map(|s| (tape(dialect, s), FileContext::default()))
        .collect();
    let runs: Vec<_> = (0..grammars.len())
        .map(|statement| sql::ClauseRun {
            statement,
            files: (0..sources.len()).map(|file| (file, None)).collect(),
        })
        .collect();
    let legacy = sql::exec::compile(&grammars)
        .unwrap()
        .session()
        .unwrap()
        .run(docs.clone(), &runs);
    let mut builder = ProgramBuilder::default();
    let statements: Vec<_> = grammars
        .iter()
        .map(|g| builder.clause_each(g, None).unwrap())
        .collect();
    let has_total_facts=builder.terminals.iter().any(|t|matches!(t,crate::code_rule_dsl::sql::terminals::Terminal::Fact(f) if f.verdicts.spike_eager_safe()));
    let sql = if has_total_facts {
        recursive_machine::compile_batch_with_contract(
            &statements.iter().collect::<Vec<_>>(),
            &builder.terminals,
        )
        .unwrap()
    } else if statements.len() == 1 {
        recursive_machine::compile(&statements[0]).unwrap()
    } else {
        recursive_machine::compile_batch(&statements.iter().collect::<Vec<_>>()).unwrap()
    };
    sql::exec::with_terminals(docs, builder.terminals, |connection, batch, frames| {
        recursive_machine::register_text(connection, batch).unwrap();
        let got = run::execute_program(
            connection,
            frames,
            &Graph::default(),
            std::slice::from_ref(&sql),
            true,
        )
        .unwrap_or_else(|e| panic!("{e}\n{sql}"));
        let faults: Vec<_> = got.hits.iter().filter(|row| row.3["fault"] != 0).collect();
        if cycle {
            assert!(legacy.is_err());
            assert!(faults.iter().any(|r| r.3["fault"] == 1));
            return;
        }
        assert!(
            faults.is_empty(),
            "unexpected recursive scheduler fault {faults:?}"
        );
        let mut actual = vec![vec![Vec::new(); sources.len()]; statements.len()];
        for (rule, file, root, answer) in &got.hits {
            if answer["result"] != 1 {
                continue;
            }
            let mut captures = Vec::new();
            for (id, slot) in statements[*rule]
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.scope == 0)
            {
                if let Some(value) = answer["env"][id.to_string()].as_i64() {
                    captures.push((
                        slot.name.clone(),
                        match slot.kind {
                            SlotKind::Node => Captured::Node(value as u32),
                            SlotKind::Count => Captured::Count(value),
                            _ => unreachable!(),
                        },
                    ));
                }
            }
            actual[*rule][*file].push(SqlMatch {
                root: *root,
                captures,
            });
        }
        assert_eq!(
            actual,
            legacy.unwrap(),
            "recursive CTE complete roots and captures"
        );
        eprintln!(
            "SQL recursive scheduler {} rules, {} files, {} VM, {} steps, {} SQL bytes",
            statements.len(),
            sources.len(),
            got.metrics["vmSteps"],
            got.hits
                .iter()
                .map(|h| h.3["steps"].as_u64().unwrap())
                .sum::<u64>(),
            sql.len()
        );
    });
}
#[test]
fn recursive_cte_binds_captures_and_keeps_order_across_all_files_and_rules() {
    check(
        "typescript",
        vec![
            json!({"rule":{"kind":"call_expression","has":{"field":"function","matches":"base"}},"utils":{"base":{"any":[{"kind":"identifier","pattern":"$A"},{"kind":"member_expression","has":{"field":"object","matches":"base"}}]}}}),
            json!({"rule":{"pattern":"$A($$$)","has":{"field":"arguments","matches":"same"}},"utils":{"same":{"any":[{"pattern":"$A"},{"has":{"matches":"same"}}]}}}),
            json!({"rule":{"matches":"find"},"utils":{"find":{"any":[{"kind":"identifier","regex":"^target$","pattern":"$X"},{"has":{"matches":"find"}}]}}}),
        ],
        &[
            "x.a.b(); y(); f(g(f)); h(1); k([k]); target; target;",
            "f(f); x.c(); { target; } obj.f(obj.f); obj.f(obj.g); obj.f([obj.f]); (a+b)(a+b); (a+b)(a-b); a.b(a/*note*/.b);",
        ],
        false,
    );
}
#[test]
fn recursive_cte_evaluates_negation_and_counts_after_dependencies() {
    check(
        "typescript",
        vec![
            json!({"rule":{"kind":"statement_block","matches":"returns"},"utils":{"returns":{"any":[{"kind":"return_statement"},{"kind":"statement_block","has":{"kind":"return_statement"},"not":{"has":{"kind":"if_statement","not":{"has":{"field":"consequence","matches":"returns"}}}}}]}}}),
            json!({"rule":{"kind":"statement_block","count":{"has":{"matches":"deep","stopBy":"end"},"min":2,"as":"N"}},"utils":{"deep":{"any":[{"kind":"return_statement"},{"kind":"if_statement","has":{"field":"consequence","matches":"deep"}},{"kind":"statement_block","has":{"matches":"deep"}}]}}}),
        ],
        &[
            "function f(){if(a){return 1;}return 2;}function g(){if(a){b();}return 2;}",
            "function z(){return 3;}",
        ],
        false,
    );
}
#[test]
fn recursive_cte_refuses_actual_read_cycles_and_ignores_unread_ones() {
    let looped = json!({"rule":{"kind":"identifier","matches":"loop"},"utils":{"loop":{"any":[{"kind":"number"},{"inside":{"has":{"matches":"loop"}}}]}}});
    check("typescript", vec![looped], &["f(a,1);"], true);
    let first = json!({"rule":{"kind":"identifier","matches":"u"},"utils":{"u":{"any":[{"regex":"^a$"},{"inside":{"has":{"matches":"u"}}}]}}});
    check("typescript", vec![first.clone()], &["a; a;"], false);
    check("typescript", vec![first], &["b;"], true);
}

#[test]
fn recursive_cte_preserves_unknown_search_counts_and_withheld_captures() {
    let utils = json!({"tail":{"any":[{"kind":"identifier","regex":"^[xu]$","pattern":"$X","testUnknown":"^u"},{"kind":"member_expression","has":{"field":"object","matches":"tail"}},{"kind":"call_expression","has":{"field":"function","matches":"tail"}}]}});
    let mut policies = vec![
        json!({"rule":{"kind":"call_expression","matches":"tail"},"utils":utils.clone()}),
        json!({"rule":{"kind":"call_expression","not":{"matches":"tail"}},"utils":utils}),
        json!({"rule":{"kind":"identifier","not":{"all":[{"all":[{"testUnknown":"^u"},{"pattern":"$X"}]},{"pattern":"$X","regex":"^never$"}]}}}),
    ];
    for bounds in [
        json!({"min":2}),
        json!({"min":3}),
        json!({"max":1}),
        json!({"min":4,"max":4}),
        json!({"min":1,"as":"N"}),
    ] {
        let mut count = json!({"has":{"kind":"identifier","testUnknown":"^u"}});
        for (key, value) in bounds.as_object().unwrap() {
            count[key] = value.clone();
        }
        policies.push(json!({"rule":{"kind":"arguments","not":{"count":count}}}));
    }
    check(
        "typescript",
        policies,
        &[
            "x.a();u.a();y.a();x.b.c();u.b.c();f(a,b,u1);g(a,u1);h(u1,u2);",
            "x(); u(); f(a,b);",
        ],
        false,
    );
}

#[test]
fn recursive_program_seeds_exceed_the_compound_select_limit_in_one_execution() {
    use crate::code_rule_dsl::sql::plan::{Op, Statement};
    let kind = GrammarDialect::TypeScript.kind_id("identifier");
    let statement = Statement {
        slots: Vec::new(),
        kinds: Some(vec![kind, kind]),
        op: Op::Kinds(vec![kind]),
        units: Vec::new(),
        sources: Vec::new(),
    };
    let query = recursive_machine::compile_batch(&[&statement; 818]).unwrap();
    sql::exec::with_terminals(
        vec![(
            tape(GrammarDialect::TypeScript, "a;"),
            FileContext::default(),
        )],
        Vec::new(),
        |connection, batch, frames| {
            recursive_machine::register_text(connection, batch).unwrap();
            let result = run::execute(connection, frames, &Graph::default(), &query).unwrap();
            assert_eq!(result.metrics["sqlExecutions"], 1);
            assert_eq!(result.hits.len(), 818);
            assert!(
                result
                    .hits
                    .iter()
                    .all(|h| h.3["result"] == 1 && h.3["fault"] == 0)
            );
            let rules: std::collections::BTreeSet<_> = result.hits.iter().map(|h| h.0).collect();
            assert_eq!(rules.len(), 818);
        },
    );
}
