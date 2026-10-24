//! Shared, capture-free recursive suffixes. Logical charges remain per invocation.
use super::{Demand, Output, Sql};
use crate::code_rule_dsl::sql::relational::{emit::condition, recursive_chain::Chain};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Components {
    pub(super) clause: usize,
    pub(super) roots: String,
    definitions: BTreeMap<usize, String>,
    pub(super) semantic: BTreeMap<usize, String>,
    pub(super) downward: BTreeMap<usize, (String, BTreeMap<u32, String>)>,
    /// Immutable compiler tables, materialized once for all reached roots.
    constants: Vec<String>,
}

impl Components {
    pub(super) fn len(&self) -> usize {
        self.definitions.len() + self.downward.len() + self.semantic.len()
    }
}

impl Sql {
    pub(super) fn constant_relation(&mut self, operator: &str, body: String) -> String {
        let mut components = self.generation.components.borrow_mut();
        let name = format!(
            "constant{}_{}",
            components.clause,
            components.constants.len()
        );
        components
            .constants
            .push(format!("{name} AS MATERIALIZED ({body})"));
        self.profile
            .push(json!({"operator":operator,"relation":name,"scope":"statement","constant":true}));
        name
    }

    /// This is called once at the statement boundary, never in a correlated child.
    pub(super) fn publish_components(&mut self) {
        let components = self.generation.components.borrow();
        self.definitions
            .extend(components.constants.iter().cloned());
        for (id, definitions) in &components.semantic {
            self.definitions.push(definitions.clone());
            self.profile.push(json!({"operator":"shared semantic recursion forest","component":id,"scope":"statement","nativeComposition":false,"materializedCaptures":false}));
        }
        for (id, (definitions, depths)) in &components.downward {
            self.definitions.push(definitions.clone());
            self.definitions.extend(depths.values().cloned());
            self.profile.push(json!({"operator":"shared ordered downward forest","component":id,"clause":components.clause,"key":["file","node"],"recursiveColumns":4,"staticDepthDomains":depths.keys().collect::<Vec<_>>(),"scope":"statement","nativeComposition":false,"demand":"guard kinds plus native-eligible immediate children in candidate-root files","cost":"ordered preorder interval through first base success","limits":"logical depth 256; native stack bytes unqualified"}));
        }
        for (id, definitions) in &components.definitions {
            self.definitions.push(definitions.clone());
            self.profile.push(json!({
                "operator":"shared recursive suffix summary",
                "component":id,"clause":components.clause,
                "key":["file","node"],"recursiveColumns":5,
                "scope":"statement","nativeComposition":false,
                "demand":"continuation guard kinds within files with candidate roots",
                "cost":"full utility entry cost per reached invocation",
                "limits":"logical depth 256; native stack-byte guard remains unqualified"
            }));
        }
    }

    pub(super) fn chain(
        &mut self,
        id: usize,
        chain: &Chain,
        input: &Demand,
    ) -> Result<Output, String> {
        let base = condition(&chain.base, "n")?;
        let edge = chain.edge_column()?;
        let prefix = {
            let mut components = self.generation.components.borrow_mut();
            let prefix = format!("component{}_{}", components.clause, id);
            if !components.definitions.contains_key(&id) {
                let kinds = chain
                    .guard_kinds()
                    .ok_or("chain: missing finite guard kinds")?
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let guard = condition(&chain.guard, "n")?;
                let successor_guard = condition(&chain.guard, "p")?;
                let roots = &components.roots;
                // Every recursive successor satisfies guard. Thus this indexed domain is
                // closed under continuation; no origin x ancestor expansion is needed.
                // Materialization belongs to the outer statement, so all roots share it.
                let definitions = format!(
                    "{prefix}_local(file,node,base,next) AS MATERIALIZED (\n\
                     SELECT n.file,n.id,({base}),CASE WHEN NOT ({base}) AND p.id IS NOT NULL AND ({successor_guard}) THEN p.id END\n\
                     FROM node n LEFT JOIN node p ON p.file=n.file AND p.id=n.{edge}\n\
                     WHERE n.kind IN ({kinds}) AND ({guard}) AND n.file IN (SELECT DISTINCT file FROM {roots})\n\
                     ),\n\
                     {prefix}_summary(file,node,truth,cost,peak) AS MATERIALIZED (\n\
                     SELECT file,node,base,1,1 FROM {prefix}_local WHERE next IS NULL\n\
                     UNION ALL SELECT l.file,l.node,s.truth,s.cost+1,s.peak+1\n\
                     FROM {prefix}_summary s JOIN {prefix}_local l ON l.file=s.file AND l.next=s.node\n\
                     )"
                );
                components.definitions.insert(id, definitions);
            }
            prefix
        };
        let levels = self.generation.dynamic_depth.as_ref().map_or_else(
            || {
                256u32
                    .saturating_sub(self.generation.utility_depth)
                    .to_string()
            },
            |_| format!("MAX(0,256-({}))", self.generation.depth_expression()),
        );
        // Base is tested before continuation, including for roots outside the guard domain.
        let local = self.relation("recursive invocation base", format!(
            "SELECT d.k,d.file,d.work,n.{edge} AS next,CASE WHEN d.work>0 AND {levels}>0 THEN ({base}) ELSE 0 END AS base FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node", input.name
        ), true, "k,file,work,next,base");
        // An outer-join ON condition does not guard SQLite's materialization.
        // The correlated lookup checks entry failure before it opens the summary.
        let lookup = super::ordered::nested::bridge(
            &format!(
                "SELECT s.truth,s.cost FROM {prefix}_summary s WHERE d.work>0 AND {levels}>0 AND NOT d.base AND s.file=d.file AND s.node=d.next"
            ),
            2,
        );
        let measured = self.relation("recursive invocation summary", format!(
            "SELECT d.k,d.work,CASE WHEN d.base THEN 1 ELSE COALESCE(r.c0,0) END AS truth,1+COALESCE(r.c1,0) AS cost FROM {local} d {lookup}"
        ), true, "k,work,truth,cost");
        let peak_note = if self.generation.peak_tracking {
            format!(
                " WHERE CASE WHEN {levels}>0 THEN relation_peak_note(({})+MIN(cost,{levels},work+1)) ELSE 1 END=1",
                self.generation.depth_expression()
            )
        } else {
            String::new()
        };
        let name = self.relation("recursive invocation charge and logical depth", format!(
            "SELECT k,MAX(0,work-MIN(cost,{levels})) AS work,CASE WHEN cost>{levels} AND work>={levels} THEN -1 WHEN work<cost THEN -2 ELSE truth END AS truth FROM {measured}{peak_note}"
        ), true, "k,work,truth");
        self.profile.last_mut().unwrap()["incomingUtilityDepth"] =
            json!(self.generation.utility_depth);
        Ok(Output { name })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Generation, Slots};
    use super::*;
    use crate::code_rule_dsl::{
        matcher::node_at,
        sql::{exec, relational::leaves, tests::differential::tape},
        test_support::grammar_matchers,
    };

    #[test]
    fn sql_chain_checks_logical_depth_before_entry_charge() {
        let policy = super::super::super::publication_tests::policy_utils(
            &[json!({"matches":"u"})],
            json!({"u":{"any":[
                {"inside":{"stopBy":"neighbor","kind":"call_expression"}},
                {"inside":{"stopBy":"neighbor","kind":"parenthesized_expression","matches":"u"}}
            ]}}),
        );
        let grammar = &grammar_matchers(&policy)[0];
        for (source, cost, answer) in [("1();", 1, 1), ("((1))();", 3, 1), ("((1));", 3, 0)] {
            let mut lower =
                super::super::super::lower::Lower::new(grammar.language, grammar.three_valued);
            let plan = lower.plan(&grammar.matcher).unwrap();
            exec::with_terminals(
                vec![(tape(grammar.language, source), Default::default())],
                lower.pure.program.terminals,
                |db, batch, _| {
                    leaves::register(db, batch).unwrap();
                    let file = &batch.files[0];
                    let root = (0..file.facts.len())
                        .find(|&id| node_at(&file.root, id).kind() == "number")
                        .unwrap();
                    for incoming in [0, 1, 253, 254, 255, 256] {
                        for work in [0, 1, 2, 3, 4, 256] {
                            let mut sql = Sql {
                                definitions: vec![],
                                profile: vec![],
                                correlated: false,
                                generation: Generation {
                                    utility_depth: incoming,
                                    ..Generation::default()
                                },
                            };
                            let roots = Demand {
                                name: sql.relation(
                                    "test invocation",
                                    format!(
                                        "SELECT 1 AS k,0 AS file,{root} AS node,{work} AS work"
                                    ),
                                    true,
                                    "k,file,node,work",
                                ),
                                slots: Slots::new(),
                            };
                            sql.generation.components.borrow_mut().roots = roots.name.clone();
                            let output = sql.emit(&plan, &roots).unwrap();
                            sql.publish_components();
                            let query = format!(
                                "WITH RECURSIVE {} SELECT truth,work FROM {}",
                                sql.definitions.join(",\n"),
                                output.name
                            );
                            let actual: (i64, i64) = db
                                .query_row(&query, [], |r| Ok((r.get(0)?, r.get(1)?)))
                                .unwrap();
                            // Independent entry-by-entry oracle, without native stack addresses.
                            let mut expected = (answer, work);
                            for entry in 0..cost {
                                if incoming + entry >= 256 {
                                    expected.0 = -1;
                                    break;
                                }
                                if expected.1 == 0 {
                                    expected.0 = -2;
                                    break;
                                }
                                expected.1 -= 1;
                            }
                            assert_eq!(
                                actual, expected,
                                "source={source}, incoming={incoming}, work={work}"
                            );
                        }
                    }
                },
            );
        }
    }
}
