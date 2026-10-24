//! Constraint sequencing retains the caller's node and tests the bound node only when reached.
use super::{Demand, Output, Plan, Sql, fields, needed, ordered, value};
use serde_json::json;

impl Sql {
    fn constraint_step(
        &mut self,
        state: &Demand,
        child: &Plan,
        target: Option<usize>,
    ) -> Result<String, String> {
        let node = target.map_or("d.node".into(), |slot| {
            if self
                .generation
                .memo_layout
                .as_ref()
                .is_some_and(|layout| layout.span_capable(slot))
            {
                format!("relation_capture_node(d.c{slot})")
            } else {
                format!("d.c{slot}")
            }
        });
        let reached = target.map_or("d.truth=1".into(), |_| {
            format!("d.truth=1 AND {node} IS NOT NULL")
        });
        if self.correlated {
            let (query, profile) = ordered::nested::program(
                child,
                state,
                false,
                &reached,
                &child.writes,
                ("d.file", &node, "d.work"),
                self.generation.clone(),
            )?;
            let exports = child
                .writes
                .iter()
                .enumerate()
                .map(|(index, slot)| format!(",r.c{} AS e{slot}", index + 2))
                .collect::<String>();
            let result = self.relation(
                "correlated reached constraint",
                format!(
                    "SELECT d.*,r.c0 AS tested,r.c1 AS remaining{exports} FROM {} d {}",
                    state.name,
                    ordered::nested::bridge(&query, child.writes.len() + 2)
                ),
                true,
                "caller context,tested outcome,remaining work,provisional child exports",
            );
            self.profile.last_mut().unwrap()["childPrograms"] = json!({"constraint":profile});
            return Ok(result);
        }
        let mut slots = needed(child);
        if child.uses_memo() || self.generation.peak_tracking {
            slots.extend(&state.slots);
        }
        let columns = fields(&slots, |slot| value(state, "d", slot));
        let demand = Demand {
            name: self.relation(
                "reached constraint demand",
                format!(
                    "SELECT d.k,d.file,{node} AS node,d.work{columns} FROM {} d WHERE {reached}",
                    state.name
                ),
                true,
                "k,file,captured node,work,required bindings",
            ),
            slots,
        };
        let output = self.emit(child, &demand)?;
        let exports = child
            .writes
            .iter()
            .map(|slot| format!(",o.c{slot} AS e{slot}"))
            .collect::<String>();
        Ok(self.relation(
            "reached constraint result",
            format!("SELECT d.*,o.truth AS tested,o.work AS remaining{exports} FROM {} d LEFT JOIN {} o ON o.k=d.k", state.name, output.name),
            true,
            "caller context,tested outcome,remaining work,provisional child exports",
        ))
    }

    pub(super) fn constrained(
        &mut self,
        plan: &Plan,
        core: &Plan,
        constraints: &[(usize, Plan)],
        input: &Demand,
    ) -> Result<Output, String> {
        let mut slots = needed(plan);
        if plan.uses_memo() || self.generation.peak_tracking {
            slots.extend(&input.slots);
        }
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let mut state = Demand {
            name: self.relation(
                "constraint transaction input",
                format!(
                    "SELECT d.k,d.file,d.node,d.work,1 AS truth{columns} FROM {} d",
                    input.name
                ),
                true,
                "k,file,caller node,work,truth,provisional bindings",
            ),
            slots,
        };
        for (target, child) in std::iter::once((None, core))
            .chain(constraints.iter().map(|(slot, child)| (Some(*slot), child)))
        {
            let tested = self.constraint_step(&state, child, target)?;
            let columns = fields(&state.slots, |slot| {
                if child.writes.contains(&slot) {
                    format!("CASE WHEN d.tested=1 THEN d.e{slot} ELSE d.c{slot} END")
                } else {
                    format!("d.c{slot}")
                }
            });
            state.name = self.relation(
                "constraint sequencing barrier",
                format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,COALESCE(d.tested,d.truth) AS truth{columns} FROM {tested} d"),
                true,
                "k,file,caller node,work,truth,provisional bindings",
            );
        }
        let columns = fields(&plan.writes, |slot| format!("s.c{slot}"));
        Ok(Output {
            name: self.relation(
                "constraint transaction result",
                format!("SELECT s.k,s.work,s.truth{columns} FROM {} s", state.name),
                true,
                "k,work,truth,exports committed by callers only on true",
            ),
        })
    }
}
