//! Clause/file answer lifetime for utilities reached through traversal.
//! SQL owns lookup, miss composition, charge replay, and completed-answer publication.
use super::ordered::nested::{bridge, program};
use super::{Demand, Op, Output, Plan, Sql, fields, value};
use serde_json::json;

#[path = "fill_cost.rs"]
mod fill_cost;

pub(super) fn needed(plan: &Plan) -> bool {
    plan.effects.file_memo
}

impl Sql {
    pub(super) fn memo_utility(
        &mut self,
        utility: usize,
        body: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        let clause = self.generation.memo_clause.expect("clause answer scope");
        let memo_layout = self.generation.memo_layout.clone();
        let layout = memo_layout.as_ref();
        let contract = layout.and_then(|layout| layout.contracts.get(&utility));
        let memo_identity = contract.map_or(utility, |contract| contract.identity);
        let memo_column = |slot: usize| layout.map_or(slot, |layout| layout.column(slot));
        let identity = format!(
            "m.clause IS {clause} AND m.utility IS {memo_identity} AND m.file IS d.file AND m.node IS d.node"
        );
        // Parameter calls read only mapped caller captures. Other live caller
        // captures remain available for rollback but do not change the answer key.
        let mut key_slots = if matches!(&body.op, Op::CallFrame { .. }) {
            body.mentions.clone()
        } else {
            input.slots.clone()
        };
        if let Some(layout) = layout {
            let scope = contract.and_then(|contract| contract.scope.as_ref());
            key_slots.retain(|slot| layout.capture(*slot, scope));
        }
        key_slots.extend(&self.generation.following);
        let keys = key_slots
            .iter()
            .map(|&s| format!(" AND m.k{} IS {}", memo_column(s), value(input, "d", s)))
            .collect::<String>();
        let hit_exports = body
            .writes
            .iter()
            .map(|s| format!(",m.o{} AS hit{s}", memo_column(*s)))
            .collect::<String>();
        let lookup = self.relation("clause utility keyed lookup", format!("SELECT d.*,m.truth AS hit,m.cost AS cost{hit_exports} FROM {} d LEFT JOIN relation_memo m ON m.mode IS 0 AND {identity}{keys}", input.name), true, "demand,prior truth,cost,exports");
        let (mut query, profile) = program(
            body,
            input,
            false,
            "d.hit IS NULL AND d.work>0",
            &body.writes,
            ("d.file", "d.node", "d.work-1"),
            self.generation.entered(),
        )?;
        let width = 2 + body.writes.len();
        let (return_width, fill_column, replay_cost) = if body.effects.cache_fills {
            query = fill_cost::bracket(&query, width, "d.hit IS NULL AND d.work>0")?;
            (
                width + 1,
                format!(",COALESCE(r.c{width},0) AS fill_cost"),
                "MAX(0,d.work-d.remaining-d.fill_cost)",
            )
        } else {
            (width, String::new(), "(d.work-d.remaining)")
        };
        let exports = body
            .writes
            .iter()
            .enumerate()
            .map(|(index, s)| {
                format!(
                    ",CASE WHEN d.hit IS NOT NULL THEN d.hit{s} ELSE r.c{} END AS e{s}",
                    index + 2
                )
            })
            .collect::<String>();
        let tested = self.relation("clause utility reached miss or replay", format!("SELECT d.*,CASE WHEN d.hit IS NOT NULL THEN CASE WHEN d.work<d.cost THEN -2 ELSE d.hit END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN d.hit IS NOT NULL THEN MAX(0,d.work-d.cost) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining{exports}{fill_column} FROM {lookup} d {}", bridge(&query, return_width)), true, "demand,tested,remaining,exports");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"miss":profile});
        if body.effects.cache_fills {
            self.profile.last_mut().unwrap()["cacheFillAccounting"] = json!({
                "scope":"one reached utility miss",
                "returnColumns":return_width,
                "replayCost":"spent work minus native cache-fill delta",
                "ordering":"materialized before counter, materialized typed result, after counter"
            });
        }
        let supplied = body
            .writes
            .iter()
            .map(|s| {
                format!(
                    " AND m.v{} IS CASE WHEN d.tested=1 THEN d.e{s} END",
                    memo_column(*s)
                )
            })
            .collect::<String>();
        let columns = fields(&body.writes, |s| format!("d.e{s}"));
        let name = self.relation("clause utility completed answer publication", format!("SELECT d.k,d.remaining AS work,COALESCE(m.truth,d.tested) AS truth{columns} FROM {tested} d LEFT JOIN relation_memo m ON m.mode IS CASE WHEN d.hit IS NULL AND d.tested>=0 THEN 1 END AND {identity}{keys} AND m.vtruth IS d.tested AND m.vcost IS {replay_cost}{supplied}"), true, "k,truth,work,exports");
        self.profile.last_mut().unwrap()["memo"] = json!({"lifetime":"statement clause/file","key":["clause","utility","file","node",if matches!(&body.op, Op::CallFrame { .. }) { "mapped caller captures" } else { "all incoming captures" }],"storage":"indexed integer scratch relation","recursiveStateColumnsAdded":0,"utility":utility});
        if layout.is_some() {
            self.profile.last_mut().unwrap()["canonicalMemo"] = json!({
                "body":utility,"identity":memo_identity,
                "scope":contract.and_then(|contract|contract.scope.as_ref()),
                "keyColumns":key_slots.iter().map(|slot|(*slot,memo_column(*slot))).collect::<Vec<_>>(),
                "exportColumns":body.writes.iter().map(|slot|(*slot,memo_column(*slot))).collect::<Vec<_>>(),
                "followingColumns":self.generation.following,
            });
        }
        Ok(Output { name })
    }
}
