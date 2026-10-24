//! Ordered child recursion shares a preorder forest, never an origin/descendant product.
use super::{Demand, Output, Sql, ordered::nested::bridge};
use crate::code_rule_dsl::sql::relational::{emit::condition, recursive_downward::Downward};
use serde_json::json;

#[path = "downward_shape.rs"]
pub(super) mod shape;

impl Sql {
    pub(super) fn downward(
        &mut self,
        id: usize,
        component: &Downward,
        input: &Demand,
    ) -> Result<Output, String> {
        let static_levels = 256u32.saturating_sub(self.generation.utility_depth);
        let dynamic = self.generation.dynamic_depth.is_some();
        let levels = if dynamic {
            format!("MAX(0,256-({}))", self.generation.depth_expression())
        } else {
            static_levels.to_string()
        };
        let prefix = {
            let mut components = self.generation.components.borrow_mut();
            let prefix = format!("downward{}_{}", components.clause, id);
            if !components.downward.contains_key(&id) {
                let definitions = shape::definitions(component, &prefix, &components.roots)?;
                components
                    .downward
                    .insert(id, (definitions, Default::default()));
            }
            let (_, depths) = components.downward.get_mut(&id).unwrap();
            if !dynamic && !depths.contains_key(&static_levels) {
                if depths.len() >= 16 {
                    return Err(
                        "downward recursion: more than 16 distinct incoming depth domains".into(),
                    );
                }
                depths.insert(
                    static_levels,
                    shape::depth_definitions(&prefix, static_levels),
                );
            }
            prefix
        };
        let base = condition(&component.base, "n")?;
        let guard = condition(&component.guard, "n")?;
        // Entry failure precedes all predicates and shared-domain preparation.
        let local = self.relation("downward invocation predicates", format!(
            "SELECT d.k,d.file,d.node,d.work,CASE WHEN d.work>0 AND {levels}>0 THEN ({base}) ELSE 0 END AS base,CASE WHEN d.work>0 AND {levels}>0 THEN ({guard}) ELSE 0 END AS active FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node",input.name
        ), true, "k,file,node,work,base,active");
        let lookup_query = if self.generation.peak_tracking {
            let (join, breach) = if dynamic {
                (
                    String::new(),
                    format!(
                        "(SELECT MIN(n.rank)-s.rank FROM {prefix}_ranked n WHERE n.file=s.file AND n.tree=s.tree AND n.rank BETWEEN s.rank AND s.finish AND n.depth>=s.depth+({levels}))"
                    ),
                )
            } else {
                (
                    format!(
                        " LEFT JOIN {prefix}_depth{levels} b ON b.file=s.file AND b.node=s.node"
                    ),
                    "b.charged".into(),
                )
            };
            format!(
                "WITH reached AS MATERIALIZED (SELECT s.*,({breach}) AS breach FROM {prefix}_summary s{join} WHERE d.work>0 AND {levels}>0 AND NOT d.base AND d.active AND s.file=d.file AND s.node=d.node) SELECT s.truth,s.cost,s.breach,(SELECT MAX(n.depth)-s.depth+1 FROM {prefix}_ranked n WHERE n.file=s.file AND n.tree=s.tree AND n.rank>=s.rank AND n.rank<s.rank+MIN(s.cost,d.work+1,COALESCE(s.breach,s.cost))) FROM reached s"
            )
        } else if dynamic {
            // Runtime depth comes from a reached semantic wrapper. Search only its
            // shared interval; no per-origin depth-state table is materialized.
            format!(
                "SELECT s.truth,s.cost,(SELECT MIN(n.rank)-s.rank FROM {prefix}_ranked n WHERE n.file=s.file AND n.tree=s.tree AND n.rank BETWEEN s.rank AND s.finish AND n.depth>=s.depth+({levels})) FROM {prefix}_summary s WHERE d.work>0 AND {levels}>0 AND NOT d.base AND d.active AND s.file=d.file AND s.node=d.node"
            )
        } else {
            format!(
                "SELECT s.truth,s.cost,b.charged FROM {prefix}_summary s LEFT JOIN {prefix}_depth{levels} b ON b.file=s.file AND b.node=s.node WHERE d.work>0 AND {levels}>0 AND NOT d.base AND d.active AND s.file=d.file AND s.node=d.node"
            )
        };
        let lookup = bridge(
            &lookup_query,
            if self.generation.peak_tracking { 4 } else { 3 },
        );
        let peak_column = if self.generation.peak_tracking {
            ",COALESCE(r.c3,1) AS peak"
        } else {
            ""
        };
        let measured = self.relation("downward invocation shared range",format!(
            "SELECT d.k,d.work,CASE WHEN d.base THEN 1 ELSE COALESCE(r.c0,0) END AS truth,COALESCE(r.c1,1) AS cost,CASE WHEN {levels}=0 THEN 0 ELSE r.c2 END AS breach{peak_column} FROM {local} d {lookup}"
        ),true,"k,work,truth,cost,charges before depth breach");
        let peak_note = if self.generation.peak_tracking {
            format!(
                " WHERE CASE WHEN {levels}>0 THEN relation_peak_note(({})+peak) ELSE 1 END=1",
                self.generation.depth_expression()
            )
        } else {
            String::new()
        };
        let name = self.relation("downward ordered charge and depth",format!(
            "SELECT k,MAX(0,work-COALESCE(breach,cost)) AS work,CASE WHEN breach IS NOT NULL AND work>=breach THEN -1 WHEN work<COALESCE(breach,cost) THEN -2 ELSE truth END AS truth FROM {measured}{peak_note}"
        ),true,"k,work,truth");
        self.profile.last_mut().unwrap()["incomingUtilityDepth"] =
            json!(self.generation.utility_depth);
        Ok(Output { name })
    }
}
