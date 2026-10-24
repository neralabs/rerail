//! Shared preorder summaries for pure, ordered neighbor-child recursion.
use crate::code_rule_dsl::sql::relational::{emit::condition, recursive_downward::Downward};

/// Definitions are published once at the statement boundary. The finite guard
/// domain and its immediate children include anonymous children and failed calls.
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn definitions(
    component: &Downward,
    prefix: &str,
    roots: &str,
) -> Result<String, String> {
    fn identifier(name: &str) -> bool {
        let mut chars = name.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
    if !identifier(prefix) || !identifier(roots) {
        return Err("downward: invalid summary identifier".into());
    }
    let kinds = component
        .guard_kinds()
        .ok_or("downward: missing finite guard kinds")?
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let base = condition(&component.base, "n")?;
    let guard = condition(&component.guard, "n")?;
    let eligible = component.child_kinds.as_ref().map_or("1".into(), |kinds| {
        format!(
            "n.kind IN ({})",
            kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    });
    let child_eligible = component.child_kinds.as_ref().map_or("1".into(), |kinds| {
        format!(
            "c.kind IN ({})",
            kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    });
    Ok(format!(
        r#"{prefix}_seed(file,node,active) AS MATERIALIZED (
 SELECT n.file,n.id,CASE WHEN NOT ({base}) AND ({guard}) THEN 1 ELSE 0 END
 FROM node n
 WHERE n.kind IN ({kinds}) AND n.file IN (SELECT DISTINCT file FROM {roots})
),
{prefix}_domain(file,node) AS MATERIALIZED (
 SELECT file,node FROM {prefix}_seed
 UNION
 SELECT c.file,c.id FROM {prefix}_seed s
 CROSS JOIN node c ON c.file=s.file AND c.parent=s.node
 WHERE s.active AND ({child_eligible})
),
{prefix}_local(file,node,parent,last,base,active,eligible) AS MATERIALIZED (
 SELECT n.file,n.id,n.parent,n."last",({base}),
 CASE WHEN NOT ({base}) AND ({guard}) THEN 1 ELSE 0 END,({eligible})
 FROM {prefix}_domain d CROSS JOIN node n ON n.file=d.file AND n.id=d.node
),
{prefix}_forest(file,node,tree,depth) AS MATERIALIZED (
 SELECT l.file,l.node,l.node,0 FROM {prefix}_local l
 LEFT JOIN {prefix}_local p ON p.file=l.file AND p.node=l.parent
 WHERE NOT l.eligible OR COALESCE(p.active,0)=0
 UNION ALL
 SELECT c.file,c.node,f.tree,f.depth+1
 FROM {prefix}_forest f
 JOIN {prefix}_local p ON p.file=f.file AND p.node=f.node AND p.active
 JOIN {prefix}_local c ON c.file=p.file AND c.parent=p.node AND c.eligible
),
{prefix}_ranked(file,node,tree,depth,last,base,rank) AS MATERIALIZED (
 SELECT f.file,f.node,f.tree,f.depth,l.last,l.base,
 ROW_NUMBER() OVER (PARTITION BY f.file,f.tree ORDER BY f.node)
 FROM {prefix}_forest f JOIN {prefix}_local l ON l.file=f.file AND l.node=f.node
),
{prefix}_success(file,node,tree,depth,last,rank,success) AS MATERIALIZED (
 SELECT file,node,tree,depth,last,rank,
 MIN(CASE WHEN base THEN rank END) OVER (
 PARTITION BY file,tree ORDER BY rank
 ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING)
 FROM {prefix}_ranked
),
{prefix}_events(file,node,tree,position,event,rank) AS MATERIALIZED (
 SELECT file,node,tree,node,0,rank FROM {prefix}_ranked
 UNION ALL
 SELECT file,node,tree,last,1,NULL FROM {prefix}_ranked
),
{prefix}_exits(file,node,tree,event,endrank) AS MATERIALIZED (
 SELECT file,node,tree,event,
 MAX(rank) OVER (PARTITION BY file,tree ORDER BY position,event
 ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
 FROM {prefix}_events
),
{prefix}_finished(file,node,tree,depth,rank,finish,truth) AS MATERIALIZED (
 SELECT s.file,s.node,s.tree,s.depth,s.rank,
 CASE WHEN s.success<=e.endrank THEN s.success ELSE e.endrank END,
 CASE WHEN s.success<=e.endrank THEN 1 ELSE 0 END
 FROM {prefix}_success s JOIN {prefix}_exits e
 ON e.file=s.file AND e.node=s.node AND e.tree=s.tree AND e.event=1
),
{prefix}_summary(file,node,tree,depth,rank,finish,truth,cost) AS MATERIALIZED (
 SELECT file,node,tree,depth,rank,finish,truth,finish-rank+1
 FROM {prefix}_finished
)"#
    ))
}

/// A forward marker sweep finds the first prohibited entry without a correlated
/// rank-range scan. Queries sort before markers at equal ranks, including level 0.
pub(super) fn depth_definitions(prefix: &str, levels: u32) -> String {
    format!(
        r#"{prefix}_depth{levels}_possible AS MATERIALIZED (
 SELECT 1 FROM {prefix}_ranked WHERE depth>={levels} LIMIT 1
),
{prefix}_depth{levels}_events(file,node,tree,target,position,event,marker) AS MATERIALIZED (
 SELECT file,node,tree,depth+{levels},rank,0,NULL FROM {prefix}_depth{levels}_possible CROSS JOIN {prefix}_summary
 UNION ALL
 SELECT file,node,tree,depth,rank,1,rank FROM {prefix}_depth{levels}_possible CROSS JOIN {prefix}_summary
),
{prefix}_depth{levels}_sweep(file,node,event,breach) AS MATERIALIZED (
 SELECT file,node,event,
 MIN(marker) OVER (PARTITION BY file,tree,target ORDER BY position,event
 ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING)
 FROM {prefix}_depth{levels}_events
),
{prefix}_depth{levels}(file,node,charged) AS MATERIALIZED (
 SELECT s.file,s.node,d.breach-s.rank
 FROM {prefix}_summary s JOIN {prefix}_depth{levels}_sweep d
 ON d.file=s.file AND d.node=s.node AND d.event=0
 WHERE d.breach<=s.finish
)"#
    )
}

#[cfg(test)]
mod tests {
    use super::super::super::{Demand, Generation, Slots, Sql};
    use crate::code_rule_dsl::{
        matcher::node_at,
        sql::{
            exec,
            relational::{leaves, outcomes::publication_tests::policy_utils},
            tests::differential::tape,
        },
        tape::TapeDoc,
        test_support::grammar_matchers,
    };
    use ast_grep_core::Node;
    use serde_json::json;

    // Independent entry interpreter: predicates use only AST kind/text, and the
    // traversal explicitly visits all children in native order.
    fn oracle(node: Node<'_, TapeDoc>, depth: u32, work: &mut i64, wrapper_base: bool) -> i64 {
        if depth >= 256 {
            return -1;
        }
        if *work == 0 {
            return -2;
        }
        *work -= 1;
        if node.text() == "1" || (wrapper_base && node.text() == "[1]") {
            return 1;
        }
        if node.kind() != "array" {
            return 0;
        }
        for child in node.children() {
            let result = oracle(child, depth + 1, work, wrapper_base);
            if result != 0 {
                return result;
            }
        }
        0
    }

    #[test]
    fn sql_downward_logical_depth_before_work_with_failed_children() {
        for (source, wrapper_base) in [
            ("[2,[3,[1]],4];", false),
            ("[2,[3,[5]],4];", false),
            ("[1,[[[[2]]]]];", false),
            ("[[1],[2]];", true),
            ("[1];", true),
        ] {
            // An unbounded regex base disables the native kind candidate filter,
            // so this fixture independently checks every anonymous child charge.
            let number = json!({"regex":"^1$"});
            let base = if wrapper_base {
                json!({"any":[number,{"regex":"^\\[1\\]$"}]})
            } else {
                number
            };
            let policy = policy_utils(
                &[json!({"matches":"u"})],
                json!({"u":{"any":[base,{"kind":"array",
                    "has":{"stopBy":"neighbor","matches":"u"}}]}}),
            );
            let grammar = &grammar_matchers(&policy)[0];
            let mut lower = crate::code_rule_dsl::sql::relational::outcomes::lower::Lower::new(
                grammar.language,
                grammar.three_valued,
            );
            let plan = lower.plan(&grammar.matcher).unwrap();
            exec::with_terminals(
                vec![(tape(grammar.language, source), Default::default())],
                lower.pure.program.terminals,
                |db, batch, _| {
                    leaves::register(db, batch).unwrap();
                    let file = &batch.files[0];
                    let root = (0..file.facts.len())
                        .find(|&id| node_at(&file.root, id).kind() == "array")
                        .unwrap();
                    for incoming in [0, 1, 253, 254, 255, 256] {
                        for allowance in 0..=20 {
                            let mut sql = Sql {
                                definitions: vec![],
                                profile: vec![],
                                correlated: false,
                                generation: Generation {
                                    utility_depth: incoming,
                                    ..Generation::default()
                                },
                            };
                            let demand = Demand {
                                name: sql.relation(
                                    "test downward invocation",
                                    format!(
                                        "SELECT 1 AS k,0 AS file,{root} AS node,{allowance} AS work"
                                    ),
                                    true,
                                    "k,file,node,work",
                                ),
                                slots: Slots::new(),
                            };
                            sql.generation.components.borrow_mut().roots = demand.name.clone();
                            let output = sql.emit(&plan, &demand).unwrap();
                            sql.publish_components();
                            let query = format!(
                                "WITH RECURSIVE {} SELECT truth,work FROM {}",
                                sql.definitions.join(",\n"),
                                output.name,
                            );
                            let actual: (i64, i64) = db
                                .query_row(&query, [], |r| Ok((r.get(0)?, r.get(1)?)))
                                .unwrap();
                            let mut remaining = allowance;
                            let truth = oracle(
                                node_at(&file.root, root),
                                incoming,
                                &mut remaining,
                                wrapper_base,
                            );
                            assert_eq!(
                                actual,
                                (truth, remaining),
                                "source={source}, incoming={incoming}, allowance={allowance}",
                            );
                        }
                    }
                },
            );
        }
    }
}
