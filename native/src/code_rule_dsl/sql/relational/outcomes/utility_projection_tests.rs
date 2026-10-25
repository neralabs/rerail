//! Executable scalar contract; utility execution/publication stays outside it.
use super::super::ir::Set;
use super::utility_projection_emit::{Summary, render};
use super::utility_projection_lower::{Expr, Function, Program};
use rusqlite::Connection;

fn program(expressions: Vec<Expr>, entry: usize) -> Program {
    Program {
        expressions,
        functions: vec![Function {
            utility: 71,
            body: 0,
        }],
        entry,
        original: None,
        source_operators: 4,
    }
}
fn answer(truth: i64, cost: i64, peak: i64) -> Summary {
    Summary {
        truth: truth.to_string(),
        cost: cost.to_string(),
        peak: peak.to_string(),
    }
}
#[test]
fn utility_projection_preserves_ordered_unknown_and_fault_prefixes() {
    let db = Connection::open_in_memory().unwrap();
    for all in [false, true] {
        let p = program(
            vec![
                Expr::Call(0),
                Expr::Call(0),
                Expr::Call(0),
                if all {
                    Expr::All(vec![0, 1, 2])
                } else {
                    Expr::Any(vec![0, 1, 2])
                },
            ],
            3,
        );
        for a in [-2, 0, 1, 2] {
            for b in [-3, 0, 1, 2] {
                for c in [-4, 0, 1, 2] {
                    let values = [a, b, c];
                    let mut expected = if all { 1 } else { 0 };
                    let mut unknown = false;
                    let mut cost = 0;
                    let mut peak = 0;
                    let mut reached = [false; 3];
                    for (i, t) in values.into_iter().enumerate() {
                        reached[i] = true;
                        cost += i as i64 + 1;
                        peak = peak.max(i as i64 + 2);
                        if t < 0 || t == if all { 0 } else { 1 } {
                            expected = t;
                            break;
                        }
                        unknown |= t == 2;
                        if i == 2 && unknown {
                            expected = 2;
                        }
                    }
                    let q = render(
                        &p,
                        p.entry,
                        |_| unreachable!(),
                        |id, _| Ok(answer(values[id], id as i64 + 1, id as i64 + 2)),
                    )
                    .unwrap();
                    let sql = format!(
                        "SELECT {},{},{}",
                        q.summary.truth, q.summary.cost, q.summary.peak
                    );
                    let got = db
                        .query_row(&sql, [], |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(2)?,
                            ))
                        })
                        .unwrap();
                    assert_eq!(got, (expected, cost, peak), "{all} {values:?}");
                    for r in q.reached {
                        let yes = db
                            .query_row(&format!("SELECT {}", r.guard), [], |r| r.get::<_, bool>(0))
                            .unwrap();
                        assert_eq!(yes, reached[r.expression]);
                    }
                }
            }
        }
    }
}
#[test]
fn utility_projection_keeps_memo_events_even_when_truth_is_constant() {
    let p = program(
        vec![
            Expr::Atom(Set::AllNodes),
            Expr::Call(0),
            Expr::Not(1),
            Expr::Any(vec![1, 2]),
        ],
        3,
    );
    assert!(p.requires_completion_recipe());
    let q = render(&p, p.entry, |_| Ok("1".into()), |_, _| Ok(answer(0, 1, 1))).unwrap();
    assert_eq!(
        q.reached.len(),
        2,
        "a repeated logical call is not erased by scalar CSE"
    );
    assert!(q.scalar_bytes > 0);
}

#[test]
fn utility_projection_producer_reduces_ordered_children_and_preserves_completions() {
    use super::Axis;
    use super::utility_projection_producer::compile;
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE node(file,id,parent,kind,start,\"end\",prev,next,field); INSERT INTO node VALUES(0,0,NULL,10,0,10,NULL,NULL,0),(0,1,0,20,1,2,NULL,2,0),(0,2,0,30,3,4,1,NULL,0);").unwrap();
    let p = Program {
        expressions: vec![
            Expr::Atom(Set::Kinds(vec![30])),
            Expr::Call(0),
            Expr::Search {
                axis: Axis::Child,
                field: None,
                body: 1,
                filter: None,
            },
        ],
        functions: vec![Function {
            utility: 812,
            body: 0,
        }],
        entry: 2,
        original: None,
        source_operators: 3,
    };
    for incoming in [0, 254, 255, 256] {
        let q = compile(
            &p,
            &format!("SELECT 0 file,0 node,{incoming} incoming"),
            "renamed_projection",
        )
        .unwrap();
        let prefix = format!("WITH RECURSIVE {} ", q.definitions.join(","));
        let got = db
            .query_row(&(prefix.clone() + &q.summary_select), [], |r| {
                Ok((
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .unwrap();
        assert_eq!(
            got,
            if incoming < 256 {
                (1, 2, 1)
            } else {
                (-1, 0, 0)
            }
        );
        let mut stmt = db.prepare(&(prefix + q.completion_select())).unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(7)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            if incoming < 256 {
                vec![(812, 1, 0), (812, 2, 1)]
            } else {
                vec![]
            }
        );
    }
}

#[test]
fn utility_projection_search_obeys_empty_filter_and_first_field_member() {
    use super::utility_projection_producer::compile;
    use super::{Axis, WalkFilter};
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE node(file,id,parent,kind,start,\"end\",prev,next,field,ffirst); INSERT INTO node VALUES(0,0,NULL,10,0,10,NULL,NULL,0,0),(0,1,0,20,1,2,NULL,2,7,1),(0,2,0,30,3,4,1,NULL,7,0);").unwrap();
    for (field, filter, expected) in [
        (
            None,
            Some(WalkFilter {
                kinds: vec![],
                text: vec![],
            }),
            0,
        ),
        (Some(7), None, 0),
        (None, None, 1),
    ] {
        let p = Program {
            expressions: vec![
                Expr::Atom(Set::Kinds(vec![30])),
                Expr::Search {
                    axis: Axis::Child,
                    field,
                    body: 0,
                    filter,
                },
            ],
            functions: vec![],
            entry: 1,
            original: None,
            source_operators: 2,
        };
        let q = compile(&p, "SELECT 0 file,0 node,0 incoming", "field_control").unwrap();
        let sql = format!(
            "WITH RECURSIVE {} {}",
            q.definitions.join(","),
            q.summary_select
        );
        assert_eq!(
            db.query_row(&sql, [], |r| r.get::<_, i64>(3)).unwrap(),
            expected
        );
    }
}

#[test]
fn utility_projection_original_native_oracle_for_renamed_helpers_and_depth() {
    use super::publication_tests::policy_utils;
    use super::utility_projection_lower::lower_ast;
    use super::utility_projection_producer::compile;
    use crate::code_rule_dsl::{
        matcher::{self, node_at},
        sql::{exec, tests::differential::tape},
        test_support::grammar_matchers,
    };
    use serde_json::json;
    for (helper, kind, regex) in [
        ("renamed_one", "identifier", "^a$"),
        ("different_helper", "number", "^1$"),
    ] {
        let rules = [json!({"matches":helper})];
        let policy = policy_utils(
            &rules,
            json!({(helper):{"any":[{"kind":kind,"regex":regex},{"has":{"stopBy":"neighbor","matches":"child_relabelled"}}]},"child_relabelled":{"kind":kind,"regex":regex}}),
        );
        let grammars = grammar_matchers(&policy);
        let g = &grammars[0];
        let mut lower = super::super::lower::Lowerer::new(g.language);
        let program = lower_ast(&g.matcher, &mut lower, false, |u| u.index)
            .unwrap()
            .unwrap();
        exec::with_terminals(
            vec![(tape(g.language, "f(a,1);"), Default::default())],
            lower.program.terminals,
            |db, batch, _| {
                crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                let file = &batch.files[0];
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                matcher::with_file_memo(&file.root, || {
                    for incoming in [0, 253, 254, 255, 256] {
                        let q = compile(
                            &program,
                            &format!("SELECT file,id node,{incoming} incoming FROM node"),
                            "native_renamed",
                        )
                        .unwrap();
                        let sql = format!(
                            "WITH RECURSIVE {} {}",
                            q.definitions.join(","),
                            q.summary_select
                        );
                        let mut stmt = db.prepare(&sql).unwrap();
                        let rows = stmt
                            .query_map([], |r| {
                                Ok((
                                    r.get::<_, u32>(1)? as usize,
                                    r.get::<_, i64>(3)?,
                                    r.get::<_, u32>(4)? as usize,
                                    r.get::<_, u32>(5)?,
                                ))
                            })
                            .unwrap();
                        for row in rows {
                            let (node, truth, cost, peak) = row.unwrap();
                            let native = matcher::match_at_depth(
                                g,
                                node_at(&file.root, node.try_into().unwrap()),
                                incoming,
                                10000,
                            );
                            assert_eq!(
                                (truth, 10000 - cost, peak),
                                (
                                    native.truth,
                                    native.work,
                                    native.peak.saturating_sub(incoming)
                                )
                            );
                        }
                    }
                });
            },
        );
    }
}

#[test]
fn utility_projection_live_finite_adapter_preserves_native_work_and_field_order() {
    use super::publication_tests::{check_work_compiler, policy_utils};
    use serde_json::json;
    for (rule, utils, source) in [
        (
            json!({"matches":"outer_renamed"}),
            json!({"outer_renamed":{"any":[{"matches":"leaf_renamed"},{"has":{"stopBy":"neighbor","matches":"leaf_renamed"}}]},"leaf_renamed":{"kind":"identifier","regex":"^a$"}}),
            "f(a,1);b;",
        ),
        (
            json!({"matches":"field_reader"}),
            json!({"field_reader":{"kind":"function_declaration","has":{"field":"name","regex":"^a$"}}}),
            "function a(){} function b(){}",
        ),
    ] {
        let rules = [rule];
        let policy = policy_utils(&rules, utils);
        for work in [0, 1, 2, 3, 4, 5, 8, 16, 1000, i64::MAX] {
            let r = check_work_compiler(
                &policy,
                &rules,
                &[source],
                None,
                work,
                super::compile_job_finite,
            );
            assert_eq!(r["metrics"]["execution"]["matchingStatements"], 1);
            if work == i64::MAX {
                assert!(
                    r["sql"].as_str().unwrap().contains("projected_truth"),
                    "must execute common finite projection adapter"
                );
            }
        }
    }
}

#[test]
fn utility_projection_child_summary_join_scales_with_edges_not_demand_pairs() {
    use super::{Axis, utility_projection_producer::compile};
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        sql::{exec, tests::differential::tape},
    };
    let language = GrammarDialect::Python;
    let identifier = language.kinds().kind_id("identifier");
    let p = Program {
        expressions: vec![
            Expr::Atom(Set::Kinds(vec![identifier])),
            Expr::Call(0),
            Expr::Search {
                axis: Axis::Child,
                field: None,
                body: 1,
                filter: None,
            },
        ],
        functions: vec![Function {
            utility: 0,
            body: 0,
        }],
        entry: 2,
        original: None,
        source_operators: 3,
    };
    let mut measured = vec![];
    for size in [128, 512] {
        let source = "f(a,b,c)\n".repeat(size);
        let vm = exec::with_terminals(
            vec![(tape(language, &source), Default::default())],
            vec![],
            |db, batch, _| {
                let q = compile(
                    &p,
                    "SELECT file,id node,0 incoming FROM node",
                    "linear_edges",
                )
                .unwrap();
                let sql = format!(
                    "WITH RECURSIVE {} {}",
                    q.definitions.join(","),
                    q.summary_select
                );
                let facts = &batch.files[0].facts;
                let mut children = vec![vec![]; facts.len() as usize];
                for id in 0..facts.len() {
                    let parent = facts.parent(id);
                    if parent != crate::code_rule_dsl::sql::nodes::NONE {
                        children[parent as usize].push(id);
                    }
                }
                let mut stmt = db.prepare(&sql).unwrap();
                let mut count = 0;
                {
                    let mut rows = stmt.query([]).unwrap();
                    while let Some(row) = rows.next().unwrap() {
                        let node = row.get::<_, u32>(1).unwrap() as usize;
                        let first = children[node]
                            .iter()
                            .position(|&child| facts.kind(child) == identifier);
                        let cost = first.map_or(children[node].len(), |i| i + 1);
                        assert_eq!(
                            (
                                row.get::<_, i64>(3).unwrap(),
                                row.get::<_, u32>(4).unwrap() as usize,
                                row.get::<_, u32>(5).unwrap() as usize
                            ),
                            (i64::from(first.is_some()), cost, usize::from(cost > 0))
                        );
                        count += 1;
                    }
                }
                assert_eq!(count, facts.len());
                stmt.get_status(rusqlite::StatementStatus::VmStep)
            },
        );
        measured.push(vm);
    }
    eprintln!("projection child-edge scaling: {measured:?}");
    assert!(
        measured[1] < 5 * measured[0],
        "four times the AST must not create quadratic demand pairs: {measured:?}"
    );
}

#[test]
fn utility_projection_completion_dag_keeps_logical_cost_without_expanding_paths() {
    use super::utility_projection_producer::compile;
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE node(file,id,kind); INSERT INTO node VALUES(0,0,1)")
        .unwrap();
    let mut p = Program {
        expressions: vec![Expr::Atom(Set::AllNodes)],
        functions: vec![Function {
            utility: 100,
            body: 0,
        }],
        entry: 0,
        original: None,
        source_operators: 1,
    };
    for f in 1..=4 {
        let call = p.expressions.len();
        p.expressions.push(Expr::Call(f - 1));
        let body = p.expressions.len();
        p.expressions.push(Expr::All(vec![call, call]));
        p.functions.push(Function {
            utility: 100 + f,
            body,
        });
    }
    p.entry = p.expressions.len();
    p.expressions.push(Expr::Call(4));
    let q = compile(&p, "SELECT 0 file,0 node,0 incoming", "dag").unwrap();
    let prefix = format!("WITH RECURSIVE {} ", q.definitions.join(","));
    let actual = db
        .query_row(&(prefix.clone() + &q.summary_select), [], |r| {
            Ok((
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .unwrap();
    assert_eq!(
        actual,
        (1, 31, 5),
        "every logical invocation retains its charge and depth"
    );
    let mut stmt = db.prepare(&(prefix + q.completion_select())).unwrap();
    let completed = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let logical = super::utility_projection_producer::compile_complete(
        &p,
        "SELECT 0 file,0 node",
        "logical_entries",
    )
    .unwrap();
    let mut statement = db
        .prepare(&format!(
            "WITH RECURSIVE {} SELECT incoming FROM ({}) ORDER BY substr(path,1,length(path)-1)",
            logical.definitions.join(","),
            logical.entry_select()
        ))
        .unwrap();
    let logical_depths = statement
        .query_map([], |r| r.get::<_, u32>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    fn visit(depth: u32, out: &mut Vec<u32>) {
        out.push(depth);
        if depth < 4 {
            visit(depth + 1, out);
            visit(depth + 1, out);
        }
    }
    let mut expected_depths = vec![];
    visit(0, &mut expected_depths);
    assert_eq!(
        logical_depths, expected_depths,
        "logical work prefixes retain all 31 ordered invocations"
    );
    assert_eq!(
        completed,
        vec![
            (100, 1, 1),
            (101, 3, 2),
            (102, 7, 3),
            (103, 15, 4),
            (104, 31, 5)
        ],
        "one completion per reached pure answer, children before parents"
    );
}
