//! One barrier evaluates adjacent total capture-independent predicates in tracked programs.
use super::{Demand, Plan, Sql, ordered};
use serde_json::json;
impl Sql {
    pub(super) fn predicate_run(
        &mut self,
        state: &Demand,
        items: &[Plan],
        reached: &str,
        all: bool,
    ) -> Result<Option<(usize, String)>, String> {
        if !self.generation.peak_tracking || self.generation.disable_predicate_runs {
            return Ok(None);
        }
        let mut conditions = Vec::new();
        for child in items {
            if child.fallible()
                || child.reads_value()
                || child.effects.cache_fills
                || child.effects.walk_depth > 0
                || !child.mentions.is_empty()
                || !child.writes.is_empty()
            {
                break;
            }
            let Some(condition) = ordered::boolean(child, "n")? else {
                break;
            };
            conditions.push(format!("({condition})"));
        }
        if conditions.len() < 2 {
            return Ok(None);
        }
        self.generation.operator_as("fused total predicate run")?;
        let count = conditions.len();
        let expression = conditions.join(if all { " AND " } else { " OR " });
        let name=self.relation("reached total predicate run",format!("SELECT d.*,d.work AS remaining,CASE WHEN {reached} THEN CASE WHEN ({expression}) THEN 1 ELSE 0 END ELSE NULL END AS tested FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node",state.name),true,"input state,one reached total outcome,unchanged work");
        self.profile.last_mut().unwrap()["predicateRun"] = json!({"operands":count,"boolean":if all {"all"}else{"any"},"proof":"total capture-independent predicates; no utility/value/walk effects","sequencingBarriers":1});
        Ok(Some((count, name)))
    }
}
