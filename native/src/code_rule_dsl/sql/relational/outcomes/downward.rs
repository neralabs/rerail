//! Ordered child recursion shares a preorder forest, never an origin/descendant product.
use super::{Demand, Output, Sql, ordered::nested::bridge};
use crate::code_rule_dsl::sql::relational::{emit::condition, recursive_downward::Downward};
use serde_json::json;

#[path = "downward_shape.rs"]
mod shape;

impl Sql {
    pub(super) fn downward(
        &mut self,
        id: usize,
        component: &Downward,
        input: &Demand,
    ) -> Result<Output, String> {
        let levels = 256u32.saturating_sub(self.generation.utility_depth);
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
            if !depths.contains_key(&levels) {
                if depths.len() >= 16 {
                    return Err(
                        "downward recursion: more than 16 distinct incoming depth domains".into(),
                    );
                }
                depths.insert(levels, shape::depth_definitions(&prefix, levels));
            }
            prefix
        };
        let base = condition(&component.base, "n")?;
        let guard = condition(&component.guard, "n")?;
        // Entry failure precedes all predicates and shared-domain preparation.
        let local = self.relation("downward invocation predicates", format!(
            "SELECT d.k,d.file,d.node,d.work,CASE WHEN d.work>0 AND {levels}>0 THEN ({base}) ELSE 0 END AS base,CASE WHEN d.work>0 AND {levels}>0 THEN ({guard}) ELSE 0 END AS active FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node",input.name
        ), true, "k,file,node,work,base,active");
        let lookup = bridge(
            &format!(
                "SELECT s.truth,s.cost,b.charged FROM {prefix}_summary s LEFT JOIN {prefix}_depth{levels} b ON b.file=s.file AND b.node=s.node WHERE d.work>0 AND {levels}>0 AND NOT d.base AND d.active AND s.file=d.file AND s.node=d.node"
            ),
            3,
        );
        let measured = self.relation("downward invocation shared range",format!(
            "SELECT d.k,d.work,CASE WHEN d.base THEN 1 ELSE COALESCE(r.c0,0) END AS truth,COALESCE(r.c1,1) AS cost,CASE WHEN {levels}=0 THEN 0 ELSE r.c2 END AS breach FROM {local} d {lookup}"
        ),true,"k,work,truth,cost,charges before depth breach");
        let name = self.relation("downward ordered charge and depth",format!(
            "SELECT k,MAX(0,work-COALESCE(breach,cost)) AS work,CASE WHEN breach IS NOT NULL AND work>=breach THEN -1 WHEN work<COALESCE(breach,cost) THEN -2 ELSE truth END AS truth FROM {measured}"
        ),true,"k,work,truth");
        self.profile.last_mut().unwrap()["incomingUtilityDepth"] =
            json!(self.generation.utility_depth);
        Ok(Output { name })
    }
}
