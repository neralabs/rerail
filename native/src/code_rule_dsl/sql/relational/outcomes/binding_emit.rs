//! A binding transaction preserves its caller node while visiting targets in native order.
use super::{Demand, Op, Output, Plan, Sql, fields, needed, ordered, value};
use serde_json::json;
impl Sql {
    pub(super) fn binding(
        &mut self,
        plan: &Plan,
        core: &Plan,
        children: &[(usize, Plan)],
        occurrence_slot: usize,
        occurrence: Option<&Plan>,
        input: &Demand,
    ) -> Result<Output, String> {
        let mut slots = needed(plan);
        let Op::BoundLeaf { outputs, .. } = &core.op else {
            unreachable!()
        };
        let opaque_slots = &outputs[..4];
        if plan.uses_memo() {
            slots.extend(&input.slots);
        }
        for slot in opaque_slots {
            slots.remove(slot);
        }
        let resolved_opaque = opaque_slots
            .iter()
            .map(|slot| format!(",o.c{slot} AS c{slot}"))
            .collect::<String>();
        let carried_opaque = opaque_slots
            .iter()
            .map(|slot| format!(",d.c{slot} AS c{slot}"))
            .collect::<String>();
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let mut state = Demand {
            name: self.relation(
                "binding transaction input",
                format!(
                    "SELECT d.k,d.file,d.node,d.work,1 AS truth{columns} FROM {} d",
                    input.name
                ),
                true,
                "caller context,provisional captures",
            ),
            slots,
        };
        let core_demand = self.input(core, &state, "1");
        let Op::BoundLeaf {
            term,
            inputs,
            outputs,
            ..
        } = &core.op
        else {
            unreachable!()
        };
        self.generation.operator()?;
        let arguments = inputs
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(" AND p.i{index} IS {}", value(&core_demand, "d", *slot)))
            .collect::<String>();
        let exports = outputs
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",p.c{index} AS c{slot}"))
            .collect::<String>();
        let resolved = Output { name: self.relation("atomic binding facts",format!("SELECT d.k,p.work,p.truth{exports} FROM {} d CROSS JOIN relation_binding_budget p ON p.term={term} AND p.file=d.file AND p.node=d.node AND p.allowance=d.work{arguments}",core_demand.name),true,"target node ids,ordered occurrences,work,truth") };
        let columns = fields(&state.slots, |slot| {
            if core.writes.contains(&slot) {
                format!("CASE WHEN o.truth=1 THEN o.c{slot} ELSE d.c{slot} END")
            } else {
                format!("d.c{slot}")
            }
        });
        state.name = self.relation(
            "atomic binding resolution",
            format!(
                "SELECT d.k,d.file,d.node,o.work,o.truth{columns}{resolved_opaque} FROM {} d JOIN {} o ON o.k=d.k",
                state.name, resolved.name
            ),
            true,
            "caller context,target nodes,truth,work",
        );
        for (target, child) in children {
            let reached = format!("d.truth=1 AND d.c{target} IS NOT NULL AND d.work>0");
            let node = format!("d.c{target}");
            let mut child_input = Demand {
                name: state.name.clone(),
                slots: state.slots.clone(),
            };
            child_input.slots.insert(*target);
            let mut child_generation = self.generation.clone();
            child_generation.following.insert(*target);
            let (query, profile) = ordered::nested::program(
                child,
                &child_input,
                false,
                &reached,
                &child.writes,
                ("d.file", &node, "d.work-1"),
                child_generation,
            )?;
            let exports = child
                .writes
                .iter()
                .enumerate()
                .map(|(index, slot)| format!(",r.c{} AS e{slot}", index + 2))
                .collect::<String>();
            let tested = self.relation(
                "reached binding target",
                format!(
                    "SELECT d.*,r.c0 AS tested,r.c1 AS remaining{exports} FROM {} d {}",
                    state.name,
                    ordered::nested::bridge(&query, child.writes.len() + 2)
                ),
                true,
                "provisional child result",
            );
            self.profile.last_mut().unwrap()["childPrograms"] = json!({"binding":profile});
            let columns = fields(&state.slots, |slot| {
                if child.writes.contains(&slot) {
                    format!("CASE WHEN d.tested=1 THEN d.e{slot} ELSE d.c{slot} END")
                } else {
                    format!("d.c{slot}")
                }
            });
            state.name = self.relation("binding target sequencing barrier",format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<>1 THEN d.truth WHEN d.c{target} IS NULL THEN 0 WHEN d.work=0 THEN -2 ELSE d.tested END AS truth{columns}{carried_opaque} FROM {tested} d"),true,"caller context,ordered work,provisional captures");
        }
        if let Some(child) = occurrence {
            state.name = self.binding_occurrence(&state, occurrence_slot, child)?;
        }
        let columns = fields(&plan.writes, |slot| format!("s.c{slot}"));
        Ok(Output {
            name: self.relation(
                "binding transaction result",
                format!("SELECT s.k,s.work,s.truth{columns} FROM {} s", state.name),
                true,
                "captures committed by callers only on true",
            ),
        })
    }
}
