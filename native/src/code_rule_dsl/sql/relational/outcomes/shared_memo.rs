//! A single dependency chain for utility answers spanning Boolean branches.
//! Correlated materialized CTEs can be evaluated separately at each reference in
//! SQLite. Each sequencing barrier therefore references its predecessor once.
use super::super::{Demand, Op, Output, Plan, Slots, Sql, fields, value};
use super::Local;
#[path = "nested_utility.rs"]
mod nested_utility;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn calls(plan: &Plan, counts: &mut BTreeMap<usize, usize>) {
    match &plan.op {
        Op::Charge(id, body) if body.reads_value() => {
            *counts.entry(*id).or_default() += 1;
            calls(body, counts);
        }
        Op::All(items) | Op::Any(items) => {
            for item in items {
                calls(item, counts);
            }
        }
        Op::Not(child) => calls(child, counts),
        _ => {}
    }
}

/// Direct siblings already have a cheaper specialization. A shared scope is
/// necessary only when a utility occurs in different child subtrees, at least
/// one of which is nested. Deeper sharing stays inside its own child program.
pub(in super::super) fn region(plan: &Plan) -> bool {
    if let Op::Charge(_, body) = &plan.op {
        return body.uses_memo();
    }
    let (Op::All(items) | Op::Any(items)) = &plan.op else {
        return false;
    };
    let mut seen = BTreeMap::<usize, bool>::new();
    for item in items {
        let mut counts = BTreeMap::new();
        calls(item, &mut counts);
        for id in counts.keys() {
            let nested = match &item.op {
                Op::Charge(direct, body) => direct != id || body.uses_memo(),
                _ => true,
            };
            if let Some(previous) = seen.insert(*id, nested)
                && (previous || nested)
            {
                return true;
            }
        }
    }
    false
}

fn keys(plan: &Plan, slots: &mut Slots) {
    slots.extend(&plan.mentions);
    slots.extend(&plan.writes);
    match &plan.op {
        Op::All(items) | Op::Any(items) => {
            for child in items {
                keys(child, slots);
            }
        }
        Op::Not(child) => keys(child, slots),
        Op::Charge(_, child) => keys(child, slots),
        Op::Search(_, _, child, stop)
        | Op::Count {
            rule: child, stop, ..
        } => {
            keys(child, slots);
            if let Some(stop) = stop {
                keys(stop, slots);
            }
        }
        _ => {}
    }
}

struct Shared<'a> {
    sql: &'a mut Sql,
    demand: Demand,
    memo: Local,
    live: BTreeSet<String>,
    serial: usize,
}

impl Shared<'_> {
    fn name(&mut self, prefix: &str) -> String {
        self.serial += 1;
        format!("{prefix}{}", self.serial)
    }

    /// Explicit projection retires child temporaries and memo columns at last use.
    fn project(
        &mut self,
        operator: &str,
        updates: BTreeMap<String, String>,
        drop: &[String],
        materialize: bool,
    ) -> Result<(), String> {
        for key in updates.keys() {
            if key != "work" && !key.starts_with('c') {
                self.live.insert(key.clone());
            }
        }
        for key in drop {
            self.live.remove(key);
        }
        let expression = |key: &str| {
            updates
                .get(key)
                .cloned()
                .unwrap_or_else(|| format!("d.{key}"))
        };
        let columns = fields(&self.demand.slots, |s| expression(&format!("c{s}")));
        let temporaries = self
            .live
            .iter()
            .map(|s| format!(",{} AS {s}", expression(s)))
            .collect::<String>();
        let carried = self.memo.carry("d");
        let width = 4 + self.demand.slots.len() + self.live.len() + carried.matches(',').count();
        if width > 512 {
            return Err("utility memo: shared sequencing state exceeds 512 typed columns".into());
        }
        self.demand.name = self.sql.relation(
            operator,
            format!(
                "SELECT d.k,d.file,d.node,{} AS work{columns}{temporaries}{carried} FROM {} d",
                expression("work"),
                self.demand.name
            ),
            materialize,
            "one predecessor,live bindings,Boolean state,live typed answers",
        );
        self.sql.profile.last_mut().unwrap()["typedStateColumns"] = json!(width);
        Ok(())
    }

    fn save(&mut self, slots: &Slots) -> Result<BTreeMap<usize, String>, String> {
        let saved: BTreeMap<_, _> = slots.iter().map(|s| (*s, self.name("saved"))).collect();
        if !saved.is_empty() {
            self.project(
                "Boolean binding snapshot",
                saved
                    .iter()
                    .map(|(s, n)| (n.clone(), format!("d.c{s}")))
                    .collect(),
                &[],
                true,
            )?;
        }
        Ok(saved)
    }

    fn emit(
        &mut self,
        plan: &Plan,
        guard: &str,
    ) -> Result<(String, Option<(usize, usize)>), String> {
        match &plan.op {
            Op::Charge(id, body) if body.uses_memo() && !self.memo.proven.contains_key(id) => {
                self.utility(*id, body, guard)
            }
            Op::All(items) | Op::Any(items) => self.boolean(plan, items, guard),
            Op::Not(child) => {
                // Double negation changes neither outcomes nor charges. Bindings must
                // still roll back when the grandchild can export any.
                if let Op::Not(grandchild) = &child.op
                    && grandchild.writes.is_empty()
                {
                    return self.emit(grandchild, guard);
                }
                let saved = self.save(&child.writes)?;
                let (truth, proven) = self.emit(child, guard)?;
                let mut updates: BTreeMap<_, _> = saved
                    .iter()
                    .map(|(s, n)| (format!("c{s}"), format!("d.{n}")))
                    .collect();
                updates.insert(
                    truth.clone(),
                    format!("CASE d.{truth} WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE d.{truth} END"),
                );
                self.project(
                    "negation rollback with shared answers",
                    updates,
                    &saved.into_values().collect::<Vec<_>>(),
                    true,
                )?;
                Ok((truth, proven))
            }
            _ => {
                let truth = self.name("truth");
                let before = self.memo.entries.len();
                let scalar = self.sql.scalar_step(&self.demand, plan, guard)?;
                let scalar_result = scalar.is_some();
                self.demand.name = if let Some(tested) = scalar {
                    tested
                } else if let Some(tested) = self.memo.step(self.sql, &self.demand, plan, guard)? {
                    tested
                } else {
                    self.sql.correlated_step(&self.demand, plan, guard)?
                };
                if let Op::Charge(_, body) = &plan.op
                    && body.uses_memo()
                {
                    let mut skipped = BTreeMap::new();
                    calls(body, &mut skipped);
                    for (id, count) in skipped {
                        *self.memo.remaining.get_mut(&id).expect("skipped utility") -= count;
                    }
                }
                let mut updates: BTreeMap<_, _> = plan
                    .writes
                    .iter()
                    .map(|s| {
                        (
                            format!("c{s}"),
                            format!(
                                "CASE WHEN d.tested=1 THEN {} ELSE d.c{s} END",
                                if scalar_result {
                                    "d.node".into()
                                } else {
                                    format!("d.e{s}")
                                }
                            ),
                        )
                    })
                    .collect();
                updates.insert(truth.clone(), "d.tested".into());
                updates.insert("work".into(), "COALESCE(d.remaining,d.work)".into());
                // The predecessor already materializes the reached leaf or replay.
                // This projection only renames its columns and commits bindings.
                self.project(
                    "shared utility or atomic outcome projection",
                    updates,
                    &[],
                    false,
                )?;
                self.memo.wrote(plan);
                let proven = if self.demand.slots.is_empty() {
                    if let Op::Charge(id, _) = plan.op {
                        let entry = if self.memo.entries.len() > before {
                            Some(before)
                        } else {
                            self.memo.proven.get(&id).copied()
                        };
                        entry.map(|e| (id, e))
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok((truth, proven))
            }
        }
    }

    fn boolean(
        &mut self,
        plan: &Plan,
        items: &[Plan],
        guard: &str,
    ) -> Result<(String, Option<(usize, usize)>), String> {
        let inherited = self.memo.proven.clone();
        let mut guaranteed = None;
        let all = matches!(plan.op, Op::All(_));
        let saved = self.save(&plan.writes)?;
        let truth = self.name("truth");
        let open = self.name("unknown");
        let withheld: BTreeMap<_, _> = if all {
            plan.node_writes
                .iter()
                .map(|s| (*s, self.name("withheld")))
                .collect()
        } else {
            BTreeMap::new()
        };
        let mut seed = BTreeMap::from([(
            truth.clone(),
            format!("CASE WHEN {guard} THEN {} END", usize::from(all)),
        )]);
        if !all {
            seed.insert(open.clone(), "0".into());
        }
        seed.extend(withheld.values().map(|n| (n.clone(), "0".into())));
        self.project("shared Boolean entry", seed, &[], true)?;
        for (index, child) in items.iter().enumerate() {
            let guard = if all {
                let blocked = child
                    .mentions
                    .iter()
                    .filter_map(|s| withheld.get(s))
                    .map(|n| format!("d.{n}"))
                    .collect::<Vec<_>>();
                format!(
                    "COALESCE(d.{truth}>0 AND NOT ({}),0)",
                    if blocked.is_empty() {
                        "0".into()
                    } else {
                        blocked.join(" OR ")
                    }
                )
            } else {
                format!("COALESCE(d.{truth}=0,0)")
            };
            let available = self.memo.proven.clone();
            let (tested, proven) = self.emit(child, &guard)?;
            // A following sibling can be reached only after this child was reached.
            // Export only its unconditional first call, not calls from optional arms.
            self.memo.proven = available;
            if let Some((id, entry)) = proven {
                self.memo.proven.insert(id, entry);
            }
            if index == 0 {
                guaranteed = proven;
            }
            let mut updates = BTreeMap::new();
            let result = if all {
                for s in &child.node_writes {
                    let n = &withheld[s];
                    updates.insert(n.clone(),format!("d.{n} OR COALESCE(d.{tested}=2,0) OR (d.{tested} IS NULL AND d.{truth}<>0)"));
                }
                format!(
                    "CASE WHEN d.{truth} IS NULL THEN NULL WHEN d.{truth}<0 THEN d.{truth} WHEN d.{tested}<0 THEN d.{tested} WHEN d.{truth}=0 OR d.{tested}=0 THEN 0 WHEN d.{truth}=2 OR d.{tested}=2 THEN 2 ELSE 1 END"
                )
            } else {
                updates.insert(
                    open.clone(),
                    format!("d.{open} OR COALESCE(d.{tested}=2,0)"),
                );
                format!(
                    "CASE WHEN d.{truth} IS NULL THEN NULL WHEN d.{truth}<>0 THEN d.{truth} WHEN d.{tested}<0 THEN d.{tested} WHEN d.{tested}=1 THEN 1 WHEN d.{tested}=2 AND {} THEN 2 ELSE 0 END",
                    usize::from(!child.writes.is_empty())
                )
            };
            updates.insert(truth.clone(), result);
            self.project(
                "shared Boolean sequencing barrier",
                updates,
                &[tested],
                true,
            )?;
        }
        let result = if all {
            format!("d.{truth}")
        } else {
            format!("CASE WHEN d.{truth}=0 AND d.{open} THEN 2 ELSE d.{truth} END")
        };
        let mut updates: BTreeMap<_, _> = saved
            .iter()
            .map(|(s, n)| {
                (
                    format!("c{s}"),
                    format!("CASE WHEN ({result})=1 THEN d.c{s} ELSE d.{n} END"),
                )
            })
            .collect();
        updates.insert(truth.clone(), result);
        let mut retired: Vec<_> = saved.into_values().chain(withheld.into_values()).collect();
        if !all {
            retired.push(open);
        }
        self.project("shared Boolean commit or rollback", updates, &retired, true)?;
        self.memo.proven = inherited;
        Ok((truth, guaranteed))
    }
}

pub(in super::super) fn compile(
    sql: &mut Sql,
    plan: &Plan,
    input: &Demand,
) -> Result<Output, String> {
    let mut slots = input.slots.clone();
    keys(plan, &mut slots);
    let mut memo = Local::new(&slots, &[]);
    memo.direct = false;
    calls(plan, &mut memo.remaining);
    let columns = fields(&slots, |s| value(input, "d", s));
    let demand = Demand {
        name: sql.relation(
            "shared Boolean demand",
            format!(
                "SELECT d.k,d.file,d.node,d.work{columns} FROM {} d",
                input.name
            ),
            true,
            "one root demand,all live input bindings",
        ),
        slots,
    };
    sql.profile.last_mut().unwrap()["memoScope"] = json!({"scope":"smallest shared Boolean demand","identity":["utility","file","node","all input captures"],"singlePredecessor":true,"bindingFreeDominance":demand.slots.is_empty()});
    let mut shared = Shared {
        sql,
        demand,
        memo,
        live: BTreeSet::new(),
        serial: 0,
    };
    let (truth, _) = shared.emit(plan, "1")?;
    let columns = fields(&plan.writes, |s| format!("d.c{s}"));
    let name = shared.sql.relation(
        "shared Boolean outcome",
        format!(
            "SELECT d.k,d.work,d.{truth} AS truth{columns} FROM {} d",
            shared.demand.name
        ),
        true,
        "k,work,truth,observable bindings",
    );
    Ok(Output { name })
}
