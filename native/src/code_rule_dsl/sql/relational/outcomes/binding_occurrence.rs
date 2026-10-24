//! Ordered occurrence visits consume work before resolving a candidate node.
use super::{Demand, Plan, Sql, fields, ordered};
use serde_json::json;
impl Sql {
    pub(super) fn binding_occurrence(
        &mut self,
        state: &Demand,
        occurrence_slot: usize,
        child: &Plan,
    ) -> Result<String, String> {
        let target = "candidate.node";
        let (body, profile) = ordered::nested::program(
            child,
            state,
            false,
            &format!("w.work>0 AND ({target}) IS NOT NULL"),
            &child.writes,
            ("d.file", target, "w.work-1"),
            self.generation.clone(),
        )?;
        let seed = child
            .writes
            .iter()
            .map(|slot| format!(",NULL AS v{slot}"))
            .collect::<String>();
        let exports = child
            .writes
            .iter()
            .enumerate()
            .map(|(index, _)| format!(",r.c{}", index + 2))
            .collect::<String>();
        let keep_searching = if child.writes.is_empty() {
            "w.own IN (0,2)"
        } else {
            "w.own=0"
        };
        let payload = child
            .writes
            .iter()
            .map(|slot| format!(",w.v{slot}"))
            .collect::<String>();
        let query = format!(
            "WITH RECURSIVE binding_occurrence_candidates AS MATERIALIZED (SELECT CAST(key AS INTEGER) AS ord,value AS node FROM json_each(d.c{occurrence_slot}) WHERE d.truth=1),binding_occurrence_walk AS (SELECT -1 AS pos,0 AS own,0 AS open,d.work AS work{seed} WHERE d.truth=1 UNION ALL SELECT w.pos+1,CASE WHEN w.work=0 THEN -2 ELSE COALESCE(r.c0,0) END,(w.open OR COALESCE(r.c0=2,0)),COALESCE(r.c1,MAX(w.work-1,0)){exports} FROM binding_occurrence_walk w JOIN binding_occurrence_candidates candidate ON candidate.ord=w.pos+1 {} WHERE ({keep_searching})) SELECT CASE WHEN w.own=0 AND w.open THEN 2 ELSE w.own END,w.work{payload} FROM binding_occurrence_walk w ORDER BY w.pos DESC LIMIT 1",
            ordered::nested::bridge(&body, child.writes.len() + 2)
        );
        let exports = child
            .writes
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",r.c{} AS e{slot}", index + 2))
            .collect::<String>();
        let tested = self.relation(
            "ordered binding occurrence search",
            format!(
                "SELECT d.*,r.c0 AS tested,r.c1 AS remaining{exports} FROM {} d {}",
                state.name,
                ordered::nested::bridge(&query, child.writes.len() + 2)
            ),
            true,
            "caller context,first decisive occurrence,remaining work",
        );
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"occurrence":profile});
        let columns = fields(&state.slots, |slot| {
            if child.writes.contains(&slot) {
                format!("CASE WHEN d.tested=1 THEN d.e{slot} ELSE d.c{slot} END")
            } else {
                format!("d.c{slot}")
            }
        });
        Ok(self.relation("binding occurrence transaction barrier",format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,COALESCE(d.tested,d.truth) AS truth{columns},d.c{occurrence_slot} AS c{occurrence_slot} FROM {tested} d"),true,"caller context,search truth,provisional captures"))
    }
}
