//! Standalone experiment, deliberately not an outcome-compiler admission path.
//! Answers and utility charges are exact for the recognized chain shape. Logical
//! depth is modeled separately; native stack-byte checks and incoming outcome
//! utility levels still block production integration.
use super::{
    emit,
    ir::{Edge, Set},
    lower::Lowerer,
};
use crate::code_rule_dsl::{
    matcher::{AstMatcher, GrammarMatcher, StopBy},
    sql::terminals::Terminal,
};

pub(super) struct Kernel {
    /// WITH RECURSIVE definitions. Caller supplies chain_input(file,node), whose
    /// keys must be valid nodes in the immutable batch. Duplicate keys are allowed.
    pub definitions: String,
    pub terminals: Vec<Terminal>,
}
impl Kernel {
    pub fn sql(&self) -> String {
        format!(
            "WITH RECURSIVE {} SELECT s.file,s.node,s.truth,s.cost,s.peak FROM chain_summary s JOIN (SELECT DISTINCT file,node FROM chain_input) i USING(file,node) ORDER BY s.file,s.node",
            self.definitions
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Limited {
    Answer(bool),
    Work,
    LogicalDepth,
}
/// Complete invocation summaries include entry. Depth is checked before charge.
/// This does not claim to reproduce native's actual 1 MiB stack-byte guard.
pub(super) fn limit(truth: bool, cost: u64, incoming: u32, work: u64) -> Limited {
    let levels = 256u64.saturating_sub(u64::from(incoming));
    if cost > levels {
        if work < levels {
            Limited::Work
        } else {
            Limited::LogicalDepth
        }
    } else if work < cost {
        Limited::Work
    } else {
        Limited::Answer(truth)
    }
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Kernel, String> {
    let AstMatcher::Util(util) = &grammar.matcher else {
        return Err("chain: root must be a utility invocation".into());
    };
    if !grammar.applies {
        return Err("chain: inactive grammar".into());
    }
    let identity = (util.table.as_ptr() as usize, util.index);
    let mut lower = Lowerer::new(grammar.language);
    let (base, guard, edge) = util
        .with_target(|body| {
            let AstMatcher::Any(items) = body else {
                return Err("chain: ordered two-alternative body required".into());
            };
            if items.len() != 2 {
                return Err("chain: exactly two alternatives required".into());
            }
            pure(&items[0])?;
            let (related, edge) = match &items[1] {
                AstMatcher::Inside(r, StopBy::Neighbor) => (r, Edge::Parent),
                AstMatcher::Follows(r, StopBy::Neighbor) => (r, Edge::Previous),
                AstMatcher::Precedes(r, StopBy::Neighbor) => (r, Edge::Next),
                _ => return Err("chain: single parent/previous/next continuation required".into()),
            };
            let candidate = related.rule();
            let guards: &[AstMatcher] = match candidate {
                AstMatcher::All(items) if !items.is_empty() => {
                    self_call(items.last().unwrap(), identity)?;
                    &items[..items.len() - 1]
                }
                _ => {
                    self_call(candidate, identity)?;
                    &[]
                }
            };
            for guard in guards {
                pure(guard)?;
            }
            let base = lower.set(&items[0])?;
            let guard = Set::Intersect(
                guards
                    .iter()
                    .map(|g| lower.set(g))
                    .collect::<Result<_, _>>()?,
            );
            Ok::<_, String>((base, guard, edge))
        })
        .ok_or("chain: missing utility body")??;
    let base = emit::condition(&base, "n")?;
    let guard = emit::condition(&guard, "p")?;
    let column = match edge {
        Edge::Parent => "parent",
        Edge::Previous => "prev",
        Edge::Next => "next",
        _ => unreachable!(),
    };
    // Each demanded key has one successor. UNION deduplicates shared suffixes;
    // reverse UNION ALL emits each key once, without carrying an origin column.
    let definitions = format!(
        r#"
chain_demand(file,node) AS (
 SELECT file,node FROM chain_input
 UNION
 SELECT p.file,p.id FROM chain_demand d
 JOIN node n ON n.file=d.file AND n.id=d.node
 JOIN node p ON p.file=n.file AND p.id=n.{column}
 WHERE NOT ({base}) AND ({guard})
),
chain_local(file,node,base,next) AS MATERIALIZED (
 SELECT n.file,n.id,({base}),
 CASE WHEN NOT ({base}) AND p.id IS NOT NULL AND ({guard}) THEN p.id END
 FROM chain_demand d JOIN node n ON n.file=d.file AND n.id=d.node
 LEFT JOIN node p ON p.file=n.file AND p.id=n.{column}
),
chain_summary(file,node,truth,cost,peak) AS (
 SELECT file,node,base,1,1 FROM chain_local WHERE next IS NULL
 UNION ALL
 SELECT l.file,l.node,s.truth,s.cost+1,s.peak+1
 FROM chain_summary s JOIN chain_local l ON l.file=s.file AND l.next=s.node
)"#
    );
    Ok(Kernel {
        definitions,
        terminals: lower.program.terminals,
    })
}
fn self_call(matcher: &AstMatcher, identity: (usize, usize)) -> Result<(), String> {
    match matcher {
        AstMatcher::Util(u) if (u.table.as_ptr() as usize, u.index) == identity => Ok(()),
        _ => Err("chain: final term must invoke the same utility identity".into()),
    }
}
fn pure(matcher: &AstMatcher) -> Result<(), String> {
    match matcher {
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never | AstMatcher::Pattern(_) => {
            Ok(())
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for i in items {
                pure(i)?;
            }
            Ok(())
        }
        AstMatcher::Not(item) => pure(item),
        AstMatcher::Inside(r, StopBy::Neighbor)
        | AstMatcher::Has(r, StopBy::Neighbor)
        | AstMatcher::Follows(r, StopBy::Neighbor)
        | AstMatcher::Precedes(r, StopBy::Neighbor) => pure(r.rule()),
        // Lowerer additionally rejects every pattern with captures. In particular,
        // no nonrecursive utility may hide charges/depth inside a guard or base.
        _ => {
            Err("chain: predicate needs total capture-free utility-free neighbor-set proof".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{ExecutionTrace, policy};
    use super::*;
    use crate::code_rule_dsl::{
        matcher::node_at,
        sql::{exec, relational::leaves, tests::differential::tape},
        test_support::grammar_matchers,
    };
    use rusqlite::StatementStatus;
    use serde_json::{Value, json};

    fn body() -> Value {
        json!({"any":[{"inside":{"stopBy":"neighbor","kind":"call"},"precedes":{"stopBy":"neighbor","any":[{"kind":"argument_list"},{"kind":"generator_expression"}]}},{"inside":{"stopBy":"neighbor","kind":"parenthesized_expression","matches":"u"}}]})
    }
    #[test]
    fn recognizes_identity_and_refuses_hidden_effects_or_branching() {
        let valid = policy(json!({"matches":"u"}), json!({"u":body()}));
        assert!(compile(&grammar_matchers(&valid)[0]).is_ok());
        for bad in [
            json!({"any":[{"matches":"other"},{"inside":{"stopBy":"neighbor","matches":"u"}}]}),
            json!({"any":[{"pattern":"$X"},{"inside":{"stopBy":"neighbor","matches":"u"}}]}),
            json!({"any":[{"value":{"eq":1}},{"inside":{"stopBy":"neighbor","matches":"u"}}]}),
            json!({"any":[{"kind":"none"},{"has":{"stopBy":"neighbor","matches":"u"}}]}),
            json!({"any":[{"kind":"none"},{"inside":{"stopBy":"end","matches":"u"}}]}),
            json!({"any":[{"kind":"none"},{"inside":{"stopBy":"neighbor","matches":"other"}}]}),
            json!({"any":[{"kind":"none"},{"inside":{"stopBy":"neighbor","all":[{"matches":"u"},{"kind":"identifier"}]}}]}),
        ] {
            let p = policy(
                json!({"matches":"u"}),
                json!({"u":bad,"other":{"kind":"none"}}),
            );
            assert!(compile(&grammar_matchers(&p)[0]).is_err());
        }
    }
    #[test]
    fn logical_depth_model_preserves_work_failure_precedence() {
        assert_eq!(limit(true, 1, 256, 0), Limited::LogicalDepth);
        assert_eq!(limit(true, 257, 0, 255), Limited::Work);
        assert_eq!(limit(true, 257, 0, 256), Limited::LogicalDepth);
        assert_eq!(limit(false, 256, 0, 256), Limited::Answer(false));
        assert_eq!(limit(true, 256, 0, 255), Limited::Work);
        assert_eq!(limit(true, 2, 255, 1), Limited::LogicalDepth);
    }
    #[test]
    fn demanded_chains_share_suffixes_with_native_reached_work_and_sql_scaling() {
        let p = policy(json!({"matches":"u"}), json!({"u":body()}));
        let grammar = &grammar_matchers(&p)[0];
        let mut sql_bytes = None;
        let mut scaling = Vec::new();
        for depth in [32usize, 128, 512] {
            let mut vms = Vec::new();
            for unrelated in [0, 512] {
                let source = format!(
                    "{}f{}()\n(f)\n{}",
                    "(".repeat(depth),
                    ")".repeat(depth),
                    "unrelated = 1\n".repeat(unrelated)
                );
                let kernel = compile(grammar).unwrap();
                let sql = kernel.sql();
                assert_eq!(*sql_bytes.get_or_insert(sql.len()), sql.len());
                exec::with_terminals(
                    vec![(tape(grammar.language, &source), Default::default())],
                    kernel.terminals,
                    |db, batch, _| {
                        leaves::register(db, batch).unwrap();
                        let setup_trace = ExecutionTrace::start(db);
                        db.execute_batch(
                            "CREATE TEMP TABLE chain_input(file INTEGER,node INTEGER)",
                        )
                        .unwrap();
                        let file = &batch.files[0];
                        // All parentheses and the enclosed identifier share one suffix.
                        let roots: Vec<u32> = (0..file.facts.len())
                            .filter(|&id| {
                                let n = node_at(&file.root, id);
                                n.kind() == "parenthesized_expression"
                                    || (n.kind() == "identifier" && n.text() == "f")
                            })
                            .collect();
                        let root_json = serde_json::to_string(&roots).unwrap();
                        db.execute(
                            "INSERT INTO chain_input SELECT 0,value FROM json_each(?1)",
                            [root_json],
                        )
                        .unwrap();
                        db.execute("INSERT INTO chain_input SELECT * FROM chain_input", [])
                            .unwrap();
                        assert_eq!(
                            setup_trace.count(),
                            3,
                            "setup executions must remain independent of depth"
                        );
                        drop(setup_trace);
                        let matching_trace = ExecutionTrace::start(db);
                        let mut stmt = db.prepare(&sql).unwrap();
                        let rows: Vec<(u32, bool, i64, i64)> = stmt
                            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
                            .unwrap()
                            .collect::<rusqlite::Result<_>>()
                            .unwrap();
                        assert_eq!(
                            matching_trace.count(),
                            1,
                            "actual matching SQL executions, including callbacks"
                        );
                        drop(matching_trace);
                        vms.push(stmt.get_status(StatementStatus::VmStep));
                        assert_eq!(rows.len(), roots.len());
                        for (id, truth, cost, peak) in rows {
                            let cost = u64::try_from(cost).expect("nonnegative utility cost");
                            let peak = u64::try_from(peak).expect("nonnegative utility peak");
                            assert_eq!(cost, peak);
                            // Native's actual stack-byte guard may precede logical depth
                            // on long chains. Compare complete native runs through 128;
                            // long-chain work failures are compared before either depth guard.
                            if cost <= 129 {
                                let mut work =
                                    usize::try_from(cost).expect("utility cost fits native work");
                                let native = grammar
                                    .match_root(node_at(&file.root, id), &mut work)
                                    .unwrap();
                                assert_eq!(native.is_some(), truth);
                                assert_eq!(work, 0);
                            }
                            for allowance in
                                [0usize, 1, 8].into_iter().filter(|w| (*w as u64) < cost)
                            {
                                let mut work = allowance;
                                assert!(
                                    grammar
                                        .match_root(node_at(&file.root, id), &mut work)
                                        .is_err()
                                );
                                assert_eq!(work, 0);
                                assert_eq!(limit(truth, cost, 0, allowance as u64), Limited::Work);
                            }
                        }
                        let count_sql = format!(
                            "WITH RECURSIVE {} SELECT (SELECT count(*) FROM chain_demand),(SELECT count(*) FROM chain_local),(SELECT count(*) FROM chain_summary)",
                            kernel.definitions
                        );
                        let counts: (i64, i64, i64) = db
                            .query_row(&count_sql, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                            .unwrap();
                        let counts = (
                            usize::try_from(counts.0).expect("demand count fits usize"),
                            usize::try_from(counts.1).expect("local count fits usize"),
                            usize::try_from(counts.2).expect("summary count fits usize"),
                        );
                        assert_eq!(counts, (roots.len(), roots.len(), roots.len()));
                        let explain: Vec<String> = db
                            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                            .unwrap()
                            .query_map([], |r| r.get(3))
                            .unwrap()
                            .collect::<rusqlite::Result<_>>()
                            .unwrap();
                        assert!(explain.iter().any(|s| s.contains("RECURSIVE STEP")));
                        eprintln!(
                            "chain depth={depth} unrelated={unrelated} sql={} vm={} states={} plan={explain:?}",
                            sql.len(),
                            vms.last().unwrap(),
                            counts.0
                        );
                    },
                );
            }
            assert_eq!(
                vms[0], vms[1],
                "unrelated nodes must not add matching VM work"
            );
            scaling.push(vms[0]);
        }
        assert!(
            scaling[2] <= scaling[0] * 24,
            "16-fold chain growth must stay below quadratic VM growth: {scaling:?}"
        );
    }
}
