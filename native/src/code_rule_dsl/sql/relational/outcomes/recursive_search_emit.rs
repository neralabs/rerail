//! One shared structural forest; only reached nodes execute semantic base SQL.
use super::super::lower::recursive_search_lower::RecursiveSearch;
use super::{
    Demand, Output, Sql, fields,
    ordered::nested::{bridge, program},
};
use crate::code_rule_dsl::sql::relational::{ir::Set, recursive_downward::Downward};
use serde_json::json;

#[path = "memo_recursive.rs"]
mod memo_recursive;

impl Sql {
    pub(super) fn recursive_search(
        &mut self,
        id: usize,
        component: &RecursiveSearch,
        input: &Demand,
    ) -> Result<Output, String> {
        if component.memo {
            return self.memo_recursive(id, component, input);
        }
        let prefix = {
            let mut shared = self.generation.components.borrow_mut();
            let prefix = format!("semantic{}_{}", shared.clause, id);
            if !shared.semantic.contains_key(&id) {
                let shape = Downward {
                    base: Set::Empty,
                    guard: component.guard.clone(),
                    child_kinds: component.child_kinds.clone(),
                };
                let definitions =
                    super::downward::shape::definitions(&shape, &prefix, &shared.roots)?;
                shared.semantic.insert(id, definitions);
            }
            prefix
        };
        let depth = self.generation.depth_expression();
        let exports = &component.base.writes;
        let columns = fields(exports, |_| "NULL".into());
        let guard =
            crate::code_rule_dsl::sql::relational::emit::condition(&component.guard, "probe")?;
        let lookup = bridge(
            &format!(
                "SELECT tree,rank,finish,depth FROM {prefix}_summary WHERE file=d.file AND node=d.node AND d.work>0 AND ({depth})<256 AND ({guard})"
            ),
            4,
        );
        let seed=self.relation("recursive semantic interval",format!("SELECT d.k,d.file,d.node,d.work,r.c0 AS tree,COALESCE(r.c1,1) AS rank,COALESCE(r.c2,1) AS finish,COALESCE(r.c3,0) AS origin,({depth}) AS incoming FROM {} d CROSS JOIN node probe ON probe.file=d.file AND probe.id=d.node {lookup}",input.name),true,"invocation key,shared preorder bounds,incoming depth");
        let walk = format!("outcome{}", self.definitions.len());
        // A unique alias keeps the incoming level visible through nested walk SQL.
        let alias = format!(
            "semantic_entry{}_{}",
            self.generation.components.borrow().clause,
            id
        );
        let mut generation = self.generation.clone();
        generation.dynamic_depth = Some(format!("{alias}.level+1"));
        generation.utility_depth = 0;
        let (query, profile) = program(
            &component.base,
            input,
            false,
            &format!("{alias}.work>0 AND {alias}.level<256"),
            exports,
            (
                &format!("{alias}.file"),
                &format!("{alias}.node"),
                &format!("{alias}.work-1"),
            ),
            generation,
        )?;
        let returned = exports
            .iter()
            .enumerate()
            .map(|(i, _)| format!(",r.c{}", i + 2))
            .collect::<String>();
        let empty = exports.iter().map(|_| ",NULL").collect::<String>();
        // Native may_bind includes both node captures and count exports.
        let strict = !component.base.writes.is_empty();
        let continues = if strict { "w.own=0" } else { "w.own IN (0,2)" };
        let peak_note = if self.generation.peak_tracking {
            format!(
                " AND CASE WHEN {alias}.phase=0 AND {alias}.level<256 THEN relation_peak_note({alias}.level+1) ELSE 1 END=1"
            )
        } else {
            String::new()
        };
        let sql = format!(
            r#"SELECT s.k,s.file,s.node,s.tree,s.rank,s.finish,s.origin,s.incoming,s.incoming AS level,0 AS phase,0 AS own,0 AS uncertain,s.work{columns} FROM {seed} s
UNION ALL
SELECT {alias}.k,{alias}.file,{alias}.node,{alias}.tree,{alias}.rank,{alias}.finish,{alias}.origin,{alias}.incoming,{alias}.level,1,CASE WHEN {alias}.level>=256 THEN -1 WHEN {alias}.work=0 THEN -2 ELSE r.c0 END,{alias}.uncertain,COALESCE(r.c1,{alias}.work){returned}
FROM {walk} {alias} JOIN {input} d ON d.k={alias}.k {bridge} WHERE {alias}.phase=0{peak_note}
UNION ALL
SELECT w.k,w.file,n.node,w.tree,n.rank,w.finish,w.origin,w.incoming,w.incoming+n.depth-w.origin,0,0,w.uncertain OR w.own=2,w.work{empty}
FROM {walk} w JOIN {prefix}_ranked n ON n.file=w.file AND n.tree=w.tree AND n.rank=w.rank+1
WHERE w.phase=1 AND ({continues}) AND w.rank<w.finish"#,
            input = input.name,
            bridge = bridge(&query, 2 + exports.len())
        );
        self.relation(
            "ordered recursive semantic entries",
            sql,
            true,
            "invocation,preorder cursor,depth,truth,work,exports",
        );
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"base":profile});
        let payload = fields(exports, |slot| format!("w.c{slot}"));
        let name=self.relation("recursive semantic outcome",format!("SELECT w.k,w.work,CASE WHEN w.own=0 AND w.uncertain THEN 2 ELSE w.own END AS truth{payload} FROM {walk} w WHERE w.phase=1 AND (NOT ({continues}) OR w.rank=w.finish)"),true,"k,truth,work,exports");
        Ok(Output { name })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Generation, Slots};
    use super::*;
    use crate::code_rule_dsl::tape::TapeDoc;
    use crate::code_rule_dsl::{
        matcher::node_at,
        sql::{exec, relational::leaves, tests::differential::tape},
        test_support::grammar_matchers,
    };
    use ast_grep_core::Node;
    fn oracle(node: Node<'_, TapeDoc>, depth: u32, work: &mut i64, peak: &mut u32) -> i64 {
        // Both the recursive utility and its acyclic capture helper enter here.
        for level in [depth, depth + 1] {
            if level >= 256 {
                return -1;
            }
            *peak = (*peak).max(level + 1);
            if *work == 0 {
                return -2;
            }
            *work -= 1;
        }
        if node.is_named() && node.text() == "yes" {
            return 1;
        }
        if node.kind() == "array" {
            for child in node.children() {
                let answer = oracle(child, depth + 1, work, peak);
                if answer != 0 {
                    return answer;
                }
            }
        }
        0
    }
    #[test]
    fn semantic_recursion_checks_runtime_depth_before_acyclic_helper_charge() {
        let policy =
            crate::code_rule_dsl::sql::relational::outcomes::publication_tests::policy_utils(
                &[json!({"matches":"u"})],
                json!({"u":{"any":[{"matches":"base"},{"kind":"array","has":{"stopBy":"neighbor","matches":"u"}}]},"base":{"all":[{"pattern":"$X"},{"regex":"^yes$"}]}}),
            );
        let grammar = &grammar_matchers(&policy)[0];
        for source in ["[no,[yes],no];", "[[no],no];", "[yes,[[[[no]]]]];"] {
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
                    for tracked in [false, true] {
                        for incoming in [0, 1, 253, 254, 255, 256] {
                            for allowance in 0..=20 {
                                let mut sql = Sql {
                                    definitions: vec![],
                                    profile: vec![],
                                    correlated: false,
                                    generation: Generation {
                                        utility_depth: incoming,
                                        peak_tracking: tracked,
                                        memo_clause: tracked.then_some(0),
                                        ..Generation::default()
                                    },
                                };
                                let demand = Demand {
                                name: sql.relation(
                                    "test semantic invocation",
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
                                    output.name
                                );
                                db.query_row::<i64, _, _>(
                                    "SELECT relation_memo_reset(0)",
                                    [],
                                    |r| r.get(0),
                                )
                                .unwrap();
                                let token = db
                                    .query_row::<i64, _, _>(
                                        "SELECT relation_peak_open(0)",
                                        [],
                                        |r| r.get(0),
                                    )
                                    .unwrap();
                                let actual: (i64, i64) = db
                                    .query_row(&query, [], |r| Ok((r.get(0)?, r.get(1)?)))
                                    .unwrap();
                                let actual_peak = db
                                    .query_row::<u32, _, _>(
                                        "SELECT relation_peak_close(?1)",
                                        [token],
                                        |r| r.get(0),
                                    )
                                    .unwrap();
                                let mut remaining = allowance;
                                let mut peak = 0;
                                let expected = oracle(
                                    node_at(&file.root, root),
                                    incoming,
                                    &mut remaining,
                                    &mut peak,
                                );
                                if tracked {
                                    assert_eq!(
                                        actual_peak, peak,
                                        "{source} depth={incoming} work={allowance}"
                                    );
                                }
                                assert_eq!(
                                    actual,
                                    (expected, remaining),
                                    "{source} depth={incoming} work={allowance}"
                                );
                            }
                        }
                    }
                },
            );
        }
    }
}
