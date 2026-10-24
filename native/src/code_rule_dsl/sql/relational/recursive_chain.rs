//! Typed functional recursive chains with shared suffix summaries.
//! Summaries include utility entry costs and relative logical depth. Native
//! stack-byte parity is not claimed; callers supply incoming utility levels.
use super::{
    emit,
    ir::{Edge, Set},
    lower::Lowerer,
};
use crate::code_rule_dsl::{
    matcher::{AstMatcher, GrammarMatcher, StopBy, UtilRef},
    sql::terminals::{Terminal, pattern_vars},
};

use ast_grep_core::matcher::Matcher;

/// A monotone, capture-free recursive component. Neither predicate invokes utilities.
#[derive(Clone, Debug)]
pub(super) struct Chain {
    pub base: Set,
    pub guard: Set,
    pub edge: Edge,
}

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
    let mut lower = Lowerer::new(grammar.language);
    let chain = try_lower(util, &mut lower)?.ok_or("chain: unsupported recursive utility shape")?;
    Ok(Kernel {
        definitions: chain.definitions("chain_input", "chain")?,
        terminals: lower.program.terminals,
    })
}

/// Unsupported shapes leave the supplied lowerer untouched. Complete syntactic
/// proof precedes lowering, including pattern captures and kind identities.
pub(super) fn try_lower(util: &UtilRef, lower: &mut Lowerer) -> Result<Option<Chain>, String> {
    let identity = (util.table.as_ptr() as usize, util.index);
    util.with_target(|body| {
        let AstMatcher::Any(items) = body else {
            return Ok(None);
        };
        if items.len() != 2 || pure(&items[0]).is_err() {
            return Ok(None);
        }
        let (related, edge) = match &items[1] {
            AstMatcher::Inside(r, StopBy::Neighbor) => (r, Edge::Parent),
            AstMatcher::Follows(r, StopBy::Neighbor) => (r, Edge::Previous),
            AstMatcher::Precedes(r, StopBy::Neighbor) => (r, Edge::Next),
            _ => return Ok(None),
        };
        let guards = match related.rule() {
            AstMatcher::All(terms) if !terms.is_empty() => {
                if self_call(terms.last().unwrap(), identity).is_err() {
                    return Ok(None);
                }
                &terms[..terms.len() - 1]
            }
            candidate => {
                if self_call(candidate, identity).is_err() {
                    return Ok(None);
                }
                &[]
            }
        };
        if guards.iter().any(|guard| pure(guard).is_err()) {
            return Ok(None);
        }
        Ok(Some(Chain {
            base: lower.set(&items[0])?,
            guard: Set::Intersect(
                guards
                    .iter()
                    .map(|guard| lower.set(guard))
                    .collect::<Result<_, _>>()?,
            ),
            edge,
        }))
    })
    .unwrap_or(Ok(None))
}

impl Chain {
    /// Necessary kinds of the continuation candidate, suitable for an indexed
    /// global summary domain. None means the guard has no finite kind proof.
    pub fn guard_kinds(&self) -> Option<Vec<u16>> {
        use std::collections::BTreeSet;
        fn necessary(set: &Set) -> Option<BTreeSet<u16>> {
            match set {
                Set::Kinds(kinds) => Some(kinds.iter().copied().collect()),
                Set::Empty => Some(BTreeSet::new()),
                Set::Intersect(items) => items
                    .iter()
                    .filter_map(necessary)
                    .reduce(|left, right| left.intersection(&right).copied().collect()),
                Set::Union(items) => {
                    let mut kinds = BTreeSet::new();
                    for item in items {
                        kinds.extend(necessary(item)?);
                    }
                    Some(kinds)
                }
                _ => None,
            }
        }
        necessary(&self.guard).map(|kinds| kinds.into_iter().collect())
    }

    pub fn edge_column(&self) -> Result<&'static str, String> {
        match self.edge {
            Edge::Parent => Ok("parent"),
            Edge::Previous => Ok("prev"),
            Edge::Next => Ok("next"),
            Edge::Child => Err("chain: child edges are not functional".into()),
        }
    }

    /// Caller supplies valid node keys in `input(file,node)`. Duplicates are allowed.
    /// Output `{prefix}_summary(file,node,truth,cost,peak)` includes root entry.
    pub fn definitions(&self, input: &str, prefix: &str) -> Result<String, String> {
        fn identifier(name: &str) -> bool {
            let mut chars = name.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        if !identifier(input) || !identifier(prefix) {
            return Err("chain: input and prefix must be SQL identifiers".into());
        }
        let base = emit::condition(&self.base, "n")?;
        let guard = emit::condition(&self.guard, "p")?;
        let column = self.edge_column()?;
        // Each demanded key has one successor. UNION deduplicates shared suffixes;
        // reverse UNION ALL emits each key once, without carrying an origin column.
        Ok(format!(
            r#"
{prefix}_demand(file,node) AS (
 SELECT file,node FROM {input}
 UNION
 SELECT p.file,p.id FROM {prefix}_demand d
 JOIN node n ON n.file=d.file AND n.id=d.node
 JOIN node p ON p.file=n.file AND p.id=n.{column}
 WHERE NOT ({base}) AND ({guard})
),
{prefix}_local(file,node,base,next) AS MATERIALIZED (
 SELECT n.file,n.id,({base}),
 CASE WHEN NOT ({base}) AND p.id IS NOT NULL AND ({guard}) THEN p.id END
 FROM {prefix}_demand d JOIN node n ON n.file=d.file AND n.id=d.node
 LEFT JOIN node p ON p.file=n.file AND p.id=n.{column}
),
{prefix}_summary(file,node,truth,cost,peak) AS (
 SELECT file,node,base,1,1 FROM {prefix}_local WHERE next IS NULL
 UNION ALL
 SELECT l.file,l.node,s.truth,s.cost+1,s.peak+1
 FROM {prefix}_summary s JOIN {prefix}_local l ON l.file=s.file AND l.next=s.node
)"#
        ))
    }
}

fn self_call(matcher: &AstMatcher, identity: (usize, usize)) -> Result<(), String> {
    match matcher {
        AstMatcher::Util(u) if (u.table.as_ptr() as usize, u.index) == identity => Ok(()),
        _ => Err("chain: final term must invoke the same utility identity".into()),
    }
}
pub(super) fn pure(matcher: &AstMatcher) -> Result<(), String> {
    match matcher {
        AstMatcher::Kind(kind) if kind.potential_kinds().is_some() => Ok(()),
        AstMatcher::Pattern(pattern) if pattern_vars(pattern).is_empty() => Ok(()),
        AstMatcher::Regex(_) | AstMatcher::Never => Ok(()),
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
        // No utility may hide charges/depth inside a guard or base.
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
    fn global_guard_domain_requires_a_kind_proof_for_every_union_arm() {
        let kinds = |guard| {
            Chain {
                base: Set::Empty,
                guard,
                edge: Edge::Parent,
            }
            .guard_kinds()
        };
        assert_eq!(kinds(Set::Intersect(vec![])), None);
        assert_eq!(
            kinds(Set::Intersect(vec![
                Set::Regex(0),
                Set::Kinds(vec![3, 1, 3])
            ])),
            Some(vec![1, 3])
        );
        assert_eq!(
            kinds(Set::Union(vec![Set::Kinds(vec![1]), Set::Regex(0)])),
            None
        );
        assert_eq!(
            kinds(Set::Union(vec![Set::Kinds(vec![1]), Set::Kinds(vec![3])])),
            Some(vec![1, 3])
        );
        assert_eq!(
            kinds(Set::Intersect(vec![
                Set::Kinds(vec![1]),
                Set::Kinds(vec![3])
            ])),
            Some(vec![])
        );
    }
    #[test]
    fn unsupported_chain_leaves_shared_lowerer_untouched() {
        let p = policy(
            json!({"matches":"u"}),
            json!({"u":{"any":[{"regex":"seed"},{"inside":{"stopBy":"neighbor","all":[{"regex":"guard"},{"pattern":"$X"},{"matches":"u"}]}}]}}),
        );
        let grammars = grammar_matchers(&p);
        let grammar = &grammars[0];
        let AstMatcher::Util(util) = &grammar.matcher else {
            panic!("utility root required")
        };
        let mut lower = Lowerer::new(grammar.language);
        assert!(try_lower(util, &mut lower).unwrap().is_none());
        assert!(lower.program.terminals.is_empty());
        assert!(lower.program.relations.is_empty());

        let p = policy(json!({"matches":"u"}), json!({"u":body()}));
        let grammars = grammar_matchers(&p);
        let AstMatcher::Util(util) = &grammars[0].matcher else {
            panic!("utility root required")
        };
        let chain = try_lower(util, &mut lower).unwrap().unwrap();
        let definitions = chain.definitions("reached_keys", "component_1").unwrap();
        assert!(definitions.contains("component_1_summary(file,node,truth,cost,peak)"));
        assert!(definitions.contains("SELECT file,node FROM reached_keys"));
        assert!(!definitions.contains("chain_input"));
        assert!(chain.definitions("reached keys", "component_1").is_err());
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
