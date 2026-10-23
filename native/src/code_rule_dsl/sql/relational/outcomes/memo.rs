//! Reached utility answers and measured semantic costs within one Boolean demand scope.
//! The demand retains file/node identity. Each typed memo slot specializes a utility ID.
use super::ordered::nested::{bridge, program};
use super::{Demand, Op, Plan, Slots, Sql, value};
use serde_json::json;
use std::collections::BTreeMap;

/// Traversal and nested utility keys still need a separate demand lifetime contract.
pub(in crate::code_rule_dsl::sql::relational::outcomes) fn admit(
    plan: &Plan,
) -> Result<(), String> {
    fn visit(plan: &Plan, moved: bool, utility: bool) -> Result<(), String> {
        match &plan.op {
            Op::Charge(_, body) => {
                if body.reads_value() && (moved || utility) {
                    return Err("utility memo: cross-root, traversal, or nested utility reuse requires a shared demand contract".into());
                }
                visit(body, moved, true)?;
            }
            Op::All(items) | Op::Any(items) => {
                for item in items {
                    visit(item, moved, utility)?;
                }
            }
            Op::Not(child) => visit(child, moved, utility)?,
            Op::Search(_, _, child, stop)
            | Op::Count {
                rule: child, stop, ..
            } => {
                visit(child, true, utility)?;
                if let Some(stop) = stop {
                    visit(stop, true, utility)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(plan, false, false)
}

#[path = "shared_memo.rs"]
mod shared;
pub(super) use shared::{compile as shared, region};

struct Entry {
    utility: usize,
    version: Vec<usize>,
    exports: Slots,
    columns: Vec<String>,
}
pub(super) struct Local {
    keys: Slots,
    direct: bool,
    proven: BTreeMap<usize, usize>,
    versions: BTreeMap<usize, usize>,
    remaining: BTreeMap<usize, usize>,
    entries: Vec<Entry>,
}
impl Local {
    pub(super) fn new(keys: &Slots, items: &[Plan]) -> Self {
        let mut remaining = BTreeMap::new();
        for item in items {
            if let Op::Charge(id, body) = &item.op
                && body.reads_value()
            {
                *remaining.entry(*id).or_default() += 1;
            }
        }
        Self {
            keys: keys.clone(),
            direct: true,
            proven: BTreeMap::new(),
            versions: keys.iter().map(|&s| (s, 0)).collect(),
            remaining,
            entries: vec![],
        }
    }
    pub(super) fn wrote(&mut self, plan: &Plan) {
        for s in &plan.writes {
            if let Some(v) = self.versions.get_mut(s) {
                *v += 1;
            }
        }
    }
    pub(super) fn carry(&self, alias: &str) -> String {
        self.entries
            .iter()
            .filter(|e| self.remaining[&e.utility] > 0)
            .flat_map(|e| &e.columns)
            .map(|c| format!(",{alias}.{c}"))
            .collect()
    }
    pub(super) fn step(
        &mut self,
        sql: &mut Sql,
        input: &Demand,
        plan: &Plan,
        guard: &str,
    ) -> Result<Option<String>, String> {
        let Op::Charge(utility, body) = &plan.op else {
            return Ok(None);
        };
        if !body.reads_value() {
            return Ok(None);
        }
        let remaining = self.remaining.get_mut(utility).expect("direct utility");
        *remaining -= 1;
        let last = *remaining == 0;
        if last && !self.entries.iter().any(|e| e.utility == *utility) {
            return Ok(None);
        }
        let version = self.versions.values().copied().collect::<Vec<_>>();
        let existing = if self.direct {
            self.entries
                .iter()
                .position(|e| e.utility == *utility && e.version == version)
        } else {
            self.proven.get(utility).copied()
        };
        if let Some(id) = existing {
            // A later direct operand cannot become reached after the same utility was
            // skipped: All withholding only grows, and Any stops permanently. Equal
            // static versions prove that every input binding is unchanged. Therefore
            // this reached invocation has a valid answer; no miss program is needed.
            // Shared scopes supply a particular dominating entry instead. That proof
            // is restricted to regions with no bindings, and expires at branch exit.
            let exports = body
                .writes
                .iter()
                .map(|slot| format!(",d.m{id}o{slot} AS e{slot}"))
                .collect::<String>();
            return Ok(Some(sql.relation(
                "utility memo statically identical replay",
                format!("SELECT d.*,CASE WHEN NOT ({guard}) THEN NULL WHEN d.work<d.m{id}cost THEN -2 ELSE d.m{id}truth END AS tested,CASE WHEN {guard} THEN MAX(0,d.work-d.m{id}cost) ELSE d.work END AS remaining{exports} FROM {} d", input.name),
                true,
                "demand state,replayed truth,remaining,exports",
            )));
        }
        let mut matches = vec![];
        let mut truths = vec![];
        let mut costs = vec![];
        for (id, entry) in self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.utility == *utility)
        {
            let key = self
                .keys
                .iter()
                .map(|slot| format!(" AND d.m{id}i{slot} IS {}", value(input, "d", *slot)))
                .collect::<String>();
            matches.push(format!("WHEN d.m{id}truth>=0{key} THEN {id}"));
            truths.push(format!("WHEN {id} THEN d.m{id}truth"));
            costs.push(format!("WHEN {id} THEN d.m{id}cost"));
            debug_assert_eq!(entry.exports, body.writes);
        }
        let lookup = if matches.is_empty() {
            "NULL".into()
        } else {
            format!("CASE WHEN {guard} THEN CASE {} END END", matches.join(" "))
        };
        let hit = sql.relation(
            "utility memo lookup",
            format!("SELECT d.*,{lookup} AS memo_hit FROM {} d", input.name),
            true,
            "demand state,typed prior answers,memo hit",
        );
        let (query, profile) = program(
            body,
            input,
            false,
            &format!("({guard}) AND d.memo_hit IS NULL AND d.work>0"),
            &body.writes,
            ("d.file", "d.node", "d.work-1"),
        )?;
        let choice = |arms: &[String]| {
            if arms.is_empty() {
                "NULL".into()
            } else {
                format!("CASE d.memo_hit {} END", arms.join(" "))
            }
        };
        let cost = choice(&costs);
        let truth = choice(&truths);
        let exports = body
            .writes
            .iter()
            .enumerate()
            .map(|(column, slot)| {
                let arms = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.utility == *utility)
                    .map(|(id, _)| format!("WHEN {id} THEN d.m{id}o{slot}"))
                    .collect::<Vec<_>>();
                format!(
                    ",CASE WHEN d.memo_hit IS NOT NULL THEN {} ELSE r.c{} END AS e{slot}",
                    choice(&arms),
                    column + 2
                )
            })
            .collect::<String>();
        let tested=sql.relation("utility memo replay or reached miss",format!("SELECT d.*,CASE WHEN NOT ({guard}) THEN NULL WHEN d.memo_hit IS NOT NULL THEN CASE WHEN d.work<({cost}) THEN -2 ELSE {truth} END WHEN d.work=0 THEN -2 ELSE r.c0 END AS tested,CASE WHEN NOT ({guard}) THEN d.work WHEN d.memo_hit IS NOT NULL THEN MAX(0,d.work-({cost})) WHEN d.work=0 THEN 0 ELSE r.c1 END AS remaining{exports} FROM {hit} d {}",bridge(&query,2+body.writes.len())),true,"demand state,tested,remaining,exports");
        sql.profile.last_mut().unwrap()["childPrograms"] = json!({"miss":profile});
        if last {
            return Ok(Some(tested));
        }
        let id = self.entries.len();
        let mut columns = vec![format!("m{id}truth"), format!("m{id}cost")];
        let mut projected = format!(
            ",CASE WHEN d.tested>=0 THEN d.tested END AS m{id}truth,d.work-d.remaining AS m{id}cost"
        );
        for slot in &self.keys {
            columns.push(format!("m{id}i{slot}"));
            projected.push_str(&format!(",{} AS m{id}i{slot}", value(input, "d", *slot)));
        }
        for slot in &body.writes {
            columns.push(format!("m{id}o{slot}"));
            projected.push_str(&format!(",d.e{slot} AS m{id}o{slot}"));
        }
        if id >= 8
            || self.entries.iter().map(|e| e.columns.len()).sum::<usize>() + columns.len() > 512
        {
            return Err("utility memo: local answer storage exceeds eight key versions or 512 typed columns".into());
        }
        self.entries.push(Entry {
            utility: *utility,
            version,
            exports: body.writes.clone(),
            columns,
        });
        let answer = sql.relation(
            "typed utility answer and semantic cost",
            format!("SELECT d.*{projected} FROM {tested} d"),
            true,
            "demand state,utility truth,cost,input bindings,output bindings",
        );
        let p = sql.profile.last_mut().unwrap();
        p["memoKey"] = json!({"utility":utility,"context":["file","node"],"captures":self.keys,"entry":id,"scope":"one Boolean demand"});
        p["memoColumns"] = json!(self.entries[id].columns.len());
        Ok(Some(answer))
    }
}
