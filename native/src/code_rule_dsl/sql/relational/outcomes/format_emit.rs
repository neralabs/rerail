//! Reached format candidates run in native order; unknown candidates never commit captures.
use super::{Demand, Output, Plan, Sql, fields, needed, ordered, value};
use serde_json::json;
impl Sql {
    fn format_side(
        &mut self,
        child: &Plan,
        state: &Demand,
        missing: bool,
    ) -> Result<(String, serde_json::Value), String> {
        let part = if missing { "missing" } else { "unused" };
        let guard = if missing {
            "d.prepared=1 AND d.truth IN (1,2)"
        } else {
            "d.prepared=1 AND d.truth=1"
        };
        let node = "json_extract(c.value,'$[0]')";
        let file = if missing { "d.parsed_file" } else { "d.file" };
        let (query, profile) = ordered::nested::program(
            child,
            state,
            missing,
            "w.truth=0",
            &child.writes,
            (file, node, "w.work"),
            self.generation.clone(),
        )?;
        let seed = fields(&child.writes, |slot| value(state, "d", slot));
        let columns=child.writes.iter().enumerate().map(|(index,slot)|format!(",CASE WHEN r.c0=1 AND json_extract(c.value,'$[1]') THEN r.c{} ELSE w.c{slot} END AS c{slot}",index+2)).collect::<String>();
        let exports = fields(&child.writes, |slot| format!("w.c{slot}"));
        let recursive = format!(
            "WITH RECURSIVE candidates AS MATERIALIZED (SELECT CAST(key AS INTEGER) AS rank,value FROM json_each(d.{part}_json)),walk AS (SELECT -1 AS pos,d.work,0 AS truth,d.{part}_open AS open{seed} WHERE {guard} UNION ALL SELECT c.rank,r.c1,CASE WHEN r.c0<0 THEN r.c0 WHEN r.c0=1 AND json_extract(c.value,'$[1]') THEN 1 ELSE 0 END,w.open OR r.c0=2 OR (r.c0=1 AND NOT json_extract(c.value,'$[1]')){columns} FROM walk w JOIN candidates c ON c.rank=w.pos+1 {} WHERE w.truth=0) SELECT CASE WHEN w.truth=0 AND w.open THEN 2 ELSE w.truth END,w.work{exports} FROM walk w ORDER BY w.pos DESC LIMIT 1",
            ordered::nested::bridge(&query, 2 + child.writes.len())
        );
        Ok((recursive, profile))
    }
    pub(super) fn format_args(
        &mut self,
        plan: &Plan,
        term: usize,
        inputs: &[usize],
        unused: Option<&Plan>,
        missing: Option<&Plan>,
        input: &Demand,
    ) -> Result<Output, String> {
        let mut slots = needed(plan);
        if plan.uses_memo() {
            slots.extend(&input.slots);
        }
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let mut state=Demand{name:self.relation("atomic format preparation",format!("SELECT d.k,d.file,d.node,f.work,f.truth,f.truth AS prepared,f.unused_json,f.missing_json,f.unused_open,f.missing_open,f.parsed_file{columns} FROM {} d CROSS JOIN relation_format_budget f ON f.term={term} AND f.file=d.file AND f.node=d.node AND f.allowance=d.work AND f.string IS {} AND f.args IS {}",input.name,value(input,"d",inputs[0]),value(input,"d",inputs[1])),true,"caller context,ordered candidate facts,atomic preparation outcome"),slots};
        for (child, is_missing) in [(unused, false), (missing, true)] {
            let Some(child) = child else { continue };
            let (query, profile) = self.format_side(child, &state, is_missing)?;
            let guard = if is_missing {
                "d.prepared=1 AND d.truth IN (1,2)"
            } else {
                "d.prepared=1 AND d.truth=1"
            };
            let query = format!("SELECT * FROM ({query}) WHERE {guard}");
            let exports = child
                .writes
                .iter()
                .enumerate()
                .map(|(index, slot)| format!(",r.c{} AS e{slot}", index + 2))
                .collect::<String>();
            let tested = self.relation(
                "reached format candidate search",
                format!(
                    "SELECT d.*,r.c0 AS tested,r.c1 AS remaining{exports} FROM {} d {}",
                    state.name,
                    ordered::nested::bridge(&query, 2 + child.writes.len())
                ),
                true,
                "one ordered SQL candidate search and measured result",
            );
            self.profile.last_mut().unwrap()["childPrograms"] = json!({"formatSide":profile});
            let columns = fields(&state.slots, |slot| {
                if child.writes.contains(&slot) {
                    format!("CASE WHEN d.tested=1 THEN d.e{slot} ELSE d.c{slot} END")
                } else {
                    format!("d.c{slot}")
                }
            });
            state.name=self.relation("format side conjunction barrier",format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.tested IS NULL THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth,d.prepared,d.unused_json,d.missing_json,d.unused_open,d.missing_open,d.parsed_file{columns} FROM {tested} d"),true,"ordered side verdict and provisional host captures");
        }
        let columns = fields(&plan.writes, |slot| format!("d.c{slot}"));
        Ok(Output {
            name: self.relation(
                "format transaction result",
                format!("SELECT d.k,d.work,d.truth{columns} FROM {} d", state.name),
                true,
                "caller commits host captures on true only",
            ),
        })
    }
}
