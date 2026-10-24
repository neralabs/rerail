//! Neighbor fields select one node before evaluating its SQL-owned child plan.
use super::{Demand, Output, Plan, Sql, fields, needed, ordered, value};
use serde_json::json;

impl Sql {
    pub(super) fn field_neighbor(
        &mut self,
        inside: bool,
        field: u16,
        child: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        let guard = ordered::candidate_filter(child, "n");
        let source = if inside {
            format!(
                "{} d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node \
                 LEFT JOIN node n ON n.file=origin.file AND n.id=origin.parent \
                 AND origin.field={field} AND origin.ffirst=1 AND ({guard})",
                input.name
            )
        } else {
            format!(
                "{} d LEFT JOIN node n ON n.file=d.file AND n.parent=d.node \
                 AND n.field={field} AND n.ffirst=1 AND ({guard})",
                input.name
            )
        };
        // ffirst is essential: a grammar can assign one field label to several siblings.
        let selected = self.relation(
            "first field neighbor",
            format!("SELECT d.*,n.id AS selected FROM {source}"),
            true,
            "caller context,optional selected node",
        );
        if self.correlated {
            let (query, profile) = ordered::nested::program(
                child,
                input,
                false,
                "d.selected IS NOT NULL",
                &child.writes,
                ("d.file", "d.selected", "d.work"),
                self.generation.clone(),
            )?;
            let columns = fields(&child.writes, |slot| {
                let index = child.writes.iter().position(|s| *s == slot).unwrap();
                format!(
                    "CASE WHEN r.c0=1 THEN r.c{} ELSE {} END",
                    index + 2,
                    value(input, "d", slot)
                )
            });
            let name = self.relation(
                "correlated field result",
                format!(
                    "SELECT d.k,COALESCE(r.c1,d.work) AS work,COALESCE(r.c0,0) AS truth{columns} \
                     FROM {selected} d {}",
                    ordered::nested::bridge(&query, child.writes.len() + 2)
                ),
                true,
                "caller key,remaining work,truth,committed child exports",
            );
            self.profile.last_mut().unwrap()["childPrograms"] = json!({"field":profile});
            return Ok(Output { name });
        }

        let mut slots = needed(child);
        if child.uses_memo() {
            slots.extend(&input.slots);
        }
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let demand = Demand {
            name: self.relation(
                "reached field demand",
                format!(
                    "SELECT d.k,d.file,d.selected AS node,d.work{columns} \
                     FROM {selected} d WHERE d.selected IS NOT NULL"
                ),
                true,
                "caller key,file,selected node,work,required bindings",
            ),
            slots,
        };
        let output = self.emit(child, &demand)?;
        let columns = fields(&child.writes, |slot| {
            format!(
                "CASE WHEN o.truth=1 THEN o.c{slot} ELSE {} END",
                value(input, "d", slot)
            )
        });
        Ok(Output {
            name: self.relation(
                "field result",
                format!(
                    "SELECT d.k,COALESCE(o.work,d.work) AS work,COALESCE(o.truth,0) AS truth{columns} \
                     FROM {} d LEFT JOIN {} o ON o.k=d.k",
                    input.name, output.name
                ),
                true,
                "caller key,remaining work,truth,committed child exports",
            ),
        })
    }
}
