//! Field stop rules have their own native ordering and finite child geometry.
use super::{Axis, Demand, Op, Output, Plan, Sql, fields, ordered, value};
use serde_json::json;
impl Sql {
    pub(super) fn field_stopped(
        &mut self,
        inside: bool,
        field: u16,
        child: &Plan,
        stop: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        if child.effects.walk_depth.max(stop.effects.walk_depth) + 1 > 4 {
            return Err("nested ordered traversal depth exceeds 4".into());
        }
        if inside {
            self.field_stopped_inside(field, child, stop, input)
        } else {
            self.field_stopped_has(field, child, stop, input)
        }
    }
    fn field_stopped_has(
        &mut self,
        field: u16,
        child: &Plan,
        stop: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        let selected=self.relation("first stopped field child",format!("SELECT d.*,(SELECT n.id FROM node n WHERE n.file=d.file AND n.parent=d.node AND n.field={field} AND n.ffirst=1 LIMIT 1) selected FROM {} d",input.name),true,"caller captures and first requested field child");
        let filter = ordered::candidate_filter(child, "n");
        let (body, body_profile) = ordered::nested::program(
            child,
            input,
            false,
            &format!("d.selected IS NOT NULL AND ({filter})"),
            &child.writes,
            ("d.file", "d.selected", "d.work"),
            self.generation.clone(),
        )?;
        let body_values = child
            .writes
            .iter()
            .enumerate()
            .map(|(index, slot)| format!(",r.c{} v{slot}", index + 2))
            .collect::<String>();
        let tested=self.relation("stopped field root body first",format!("SELECT d.*,COALESCE(r.c0,0) own,COALESCE(r.c1,d.work) remaining{body_values} FROM {selected} d LEFT JOIN node n ON n.file=d.file AND n.id=d.selected {}",ordered::nested::bridge(&body,2+child.writes.len())),true,"root child body precedes root stop; typed trial captures");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"rootBody":body_profile});
        let pending = if child.writes.is_empty() {
            "d.own IN (0,2)"
        } else {
            "d.own=0"
        };
        let (stop_query, stop_profile) = ordered::nested::program(
            stop,
            input,
            true,
            &format!("d.selected IS NOT NULL AND ({pending})"),
            &super::Slots::new(),
            ("d.file", "d.selected", "d.remaining"),
            self.generation.clone(),
        )?;
        let stopped=self.relation("stopped field root fresh stop",format!("SELECT d.*,COALESCE(r.c0,0) stopped,COALESCE(r.c1,d.remaining) after_stop FROM {tested} d {}",ordered::nested::bridge(&stop_query,2)),true,"stop reads fresh environment and never commits captures");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"rootStop":stop_profile});
        // The callback-bearing prefix has exactly one consumer. Build the child
        // search from this row's scalars rather than referring to `stopped` again
        // inside a second correlated CTE, which can instantiate that prefix twice.
        let op = Op::Search(Axis::Child, false, Box::new(child.clone()), None);
        let search = Plan {
            effects: super::super::effects::Effects::of(&op),
            op,
            mentions: child.mentions.clone(),
            writes: child.writes.clone(),
            node_writes: child.node_writes.clone(),
            walk_filter: None,
        };
        let (query, profile) = ordered::nested::program(
            &search,
            input,
            false,
            &format!("d.selected IS NOT NULL AND ({pending}) AND d.stopped=0"),
            &child.writes,
            ("d.file", "d.selected", "d.after_stop"),
            self.generation.clone(),
        )?;
        let returned = child
            .writes
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                format!(
                    ",CASE WHEN d.own=1 THEN d.v{slot} WHEN r.c0=1 THEN r.c{} ELSE {} END c{slot}",
                    index + 2,
                    value(input, "d", *slot)
                )
            })
            .collect::<String>();
        let truth = "CASE WHEN d.own<0 THEN d.own WHEN d.own=1 THEN 1 WHEN d.stopped<0 THEN d.stopped WHEN r.c0<0 THEN r.c0 WHEN r.c0=1 THEN 1 WHEN d.own=2 OR d.stopped=2 OR r.c0=2 THEN 2 ELSE 0 END";
        let name=self.relation("completed stopped field has",format!("SELECT d.k,COALESCE(r.c1,d.after_stop) work,{truth} truth{returned} FROM {stopped} d {}",ordered::nested::bridge(&query,2+child.writes.len())),true,"one linear callback chain; root then direct children only; transactional exports");
        self.profile.last_mut().unwrap()["childPrograms"] =
            json!({"children":profile,"singleReachedDemand":true});
        Ok(Output { name })
    }

    fn field_stopped_inside(
        &mut self,
        field: u16,
        child: &Plan,
        stop: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        let selected=self.relation("stopped field ancestors seed",format!("SELECT d.k,d.file,n.parent node,n.id prior,d.work FROM {} d JOIN node n ON n.file=d.file AND n.id=d.node",input.name),true,"ancestor and previous ascent node");
        let (stop_query, stop_profile) = ordered::nested::program(
            stop,
            input,
            true,
            "1",
            &super::Slots::new(),
            ("w.file", "w.node", "w.work"),
            self.generation.clone(),
        )?;
        let filter = ordered::candidate_filter(child, "n");
        let (body, body_profile) = ordered::nested::program(
            child,
            input,
            false,
            &format!("w.stopped>=0 AND edge.field={field} AND edge.ffirst=1 AND ({filter})"),
            &child.writes,
            ("w.file", "w.node", "w.work"),
            self.generation.clone(),
        )?;
        let walk = format!("outcome{}", self.definitions.len());
        let seeds = child
            .writes
            .iter()
            .map(|slot| format!(",NULL v{slot}"))
            .collect::<String>();
        let nulls = child.writes.iter().map(|_| ",NULL").collect::<String>();
        let body_values = child
            .writes
            .iter()
            .enumerate()
            .map(|(index, _)| format!(",r.c{}", index + 2))
            .collect::<String>();
        let keep = child
            .writes
            .iter()
            .map(|slot| format!(",w.v{slot}"))
            .collect::<String>();
        let decisive = if child.writes.is_empty() {
            "w.own<0 OR w.own=1"
        } else {
            "w.own<0 OR w.own IN (1,2)"
        };
        self.relation("ordered stopped field ancestors",format!(
            "SELECT s.k,s.file,s.node,s.prior,0 phase,0 own,0 stopped,0 open,s.work{seeds} FROM {selected} s WHERE s.node IS NOT NULL \
            UNION ALL SELECT w.k,w.file,w.node,w.prior,1,0,COALESCE(r.c0,0),w.open,COALESCE(r.c1,w.work){nulls} FROM {walk} w CROSS JOIN {} d ON d.k=w.k {} WHERE w.phase=0 \
            UNION ALL SELECT w.k,w.file,w.node,w.prior,2,COALESCE(r.c0,0),w.stopped,w.open,COALESCE(r.c1,w.work){body_values} FROM {walk} w JOIN node n ON n.file=w.file AND n.id=w.node JOIN node edge ON edge.file=w.file AND edge.id=w.prior CROSS JOIN {} d ON d.k=w.k {} WHERE w.phase=1 \
            UNION ALL SELECT w.k,w.file,n.parent,w.node,0,0,0,(w.open OR w.own=2),w.work{nulls} FROM {walk} w JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND NOT ({decisive}) AND w.stopped=0 AND n.parent IS NOT NULL",
            input.name,ordered::nested::bridge(&stop_query,2),input.name,ordered::nested::bridge(&body,2+child.writes.len())),true,"stop first on every ancestor, eligible inclusive body second, then ascent");
        self.profile.last_mut().unwrap()["childPrograms"] =
            json!({"stop":stop_profile,"body":body_profile});
        let terminal=self.relation("completed stopped field ancestor",format!("SELECT w.k,w.work,CASE WHEN w.stopped<0 THEN w.stopped WHEN w.own<0 THEN w.own WHEN w.own=1 THEN 1 WHEN w.open OR w.own=2 OR w.stopped=2 THEN 2 ELSE 0 END truth{keep} FROM {walk} w JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND (({decisive}) OR w.stopped<>0 OR n.parent IS NULL)"),true,"first inclusive result or open/closed stop");
        let columns = fields(&child.writes, |slot| {
            format!(
                "CASE WHEN t.truth=1 THEN t.v{slot} ELSE {} END",
                value(input, "d", slot)
            )
        });
        Ok(Output{name:self.relation("stopped field ancestor outcome",format!("SELECT d.k,COALESCE(t.work,d.work) work,COALESCE(t.truth,0) truth{columns} FROM {} d LEFT JOIN {terminal} t ON t.k=d.k",input.name),true,"committed eligible body exports; stop bindings discarded")})
    }
}
