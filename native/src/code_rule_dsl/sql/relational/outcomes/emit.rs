use super::super::ir::Set;
#[path = "chain.rs"]
mod chain;
#[path = "constraints.rs"]
mod constraints;
#[path = "field.rs"]
mod field;
#[path = "field_end.rs"]
mod field_end;
#[path = "file_memo.rs"]
mod file_memo;
#[path = "memo.rs"]
pub(super) mod memo;
#[path = "ordered.rs"]
mod ordered;
#[path = "predicate.rs"]
mod predicate;
#[path = "publication.rs"]
mod publication;
#[path = "repeat_cost.rs"]
mod repeat_cost;
#[path = "repeat_walk.rs"]
mod repeat_walk;
use super::{Axis, Op, Plan, Slot, Slots};
pub(super) use publication::compile_jobs;
use serde_json::{Value, json};

pub(super) struct Demand {
    name: String,
    slots: Slots,
}
pub(super) struct Output {
    name: String,
}
pub(super) struct Sql {
    definitions: Vec<String>,
    profile: Vec<Value>,
    correlated: bool,
    generation: Generation,
}

/// One clause budget shared by every nested SQL program. Count operators only
/// after the emitter selects a miss body, a replay, or a fused scalar expression.
#[derive(Clone, Default)]
pub(super) struct Generation {
    operators: std::rc::Rc<std::cell::Cell<usize>>,
    memo_clause: Option<usize>,
    utility_depth: u32,
    components: std::rc::Rc<std::cell::RefCell<chain::Components>>,
}
impl Generation {
    fn entered(&self) -> Self {
        Self {
            utility_depth: self.utility_depth.saturating_add(1),
            ..self.clone()
        }
    }
    fn operator(&self) -> Result<(), String> {
        let count = self.operators.get();
        if count == 256 {
            return Err("outcome relations: emitted composition operator count exceeds 256".into());
        }
        self.operators.set(count + 1);
        Ok(())
    }

    /// Fused scalar/ordered programs consume their entire emitted tree. Utility
    /// replays must never use this path: their miss body is charged separately.
    fn tree(&self, plan: &Plan) -> Result<(), String> {
        self.operator()?;
        match &plan.op {
            Op::All(items) | Op::Any(items) => {
                for child in items {
                    self.tree(child)?;
                }
            }
            Op::Constrained(core, constraints) => {
                self.tree(core)?;
                for (_, child) in constraints {
                    self.tree(child)?;
                }
            }
            Op::FieldNeighbor { child, .. } | Op::FieldEnd { child, .. } => self.tree(child)?,
            Op::Not(child) => self.tree(child)?,
            Op::Charge(_, child) => self.tree(child)?,
            Op::Search(_, _, child, stop)
            | Op::Count {
                rule: child, stop, ..
            } => {
                self.tree(child)?;
                if let Some(stop) = stop {
                    self.tree(stop)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn value(input: &Demand, alias: &str, slot: usize) -> String {
    if input.slots.contains(&slot) {
        format!("{alias}.c{slot}")
    } else {
        "NULL".into()
    }
}

fn fields(slots: &Slots, mut expression: impl FnMut(usize) -> String) -> String {
    slots
        .iter()
        .map(|&slot| format!(",{} AS c{slot}", expression(slot)))
        .collect()
}

fn needed(plan: &Plan) -> Slots {
    plan.mentions.union(&plan.writes).copied().collect()
}

/// Necessary kinds for every nonfalse answer. Unknown never permits a false kind test.
fn predicate_kinds(predicate: &Set) -> Option<std::collections::BTreeSet<u16>> {
    match predicate {
        Set::Kinds(kinds) => Some(kinds.iter().copied().collect()),
        Set::Empty => Some(Default::default()),
        Set::Intersect(items) => items
            .iter()
            .filter_map(predicate_kinds)
            .reduce(|a, b| a.intersection(&b).copied().collect()),
        Set::Union(items) => {
            let mut kinds = std::collections::BTreeSet::new();
            for item in items {
                kinds.extend(predicate_kinds(item)?);
            }
            Some(kinds)
        }
        _ => None,
    }
}

fn kinds(plan: &Plan) -> Option<std::collections::BTreeSet<u16>> {
    // Constraints run only after core success and inspect captured targets.
    // Only the core can prove necessary root kinds, including its effect order.
    if let Op::Constrained(core, _) = &plan.op {
        return kinds(core);
    }
    // Arguments reject other root kinds before reading scopes or spending work.
    if let Op::BoundLeaf { kinds, .. } = &plan.op {
        return kinds.as_ref().map(|k| k.iter().copied().collect());
    }
    if plan.fallible() {
        // Only a total conjunction prefix can reject a root before its first effect.
        return if let Op::All(items) = &plan.op {
            items
                .iter()
                .take_while(|p| !p.fallible())
                .enumerate()
                .filter(|(i, p)| *i == 0 || p.mentions.is_empty())
                .filter_map(|(_, p)| kinds(p))
                .reduce(|a, b| a.intersection(&b).copied().collect())
        } else {
            None
        };
    }
    match &plan.op {
        Op::Predicate(predicate) => predicate_kinds(predicate),
        Op::Pattern(_, _, kinds) => kinds.as_ref().map(|k| k.iter().copied().collect()),
        Op::All(items) => items
            .iter()
            .enumerate()
            // An earlier unknown can withhold this whole term, including its kind tests.
            .filter(|(i, p)| *i == 0 || p.mentions.is_empty())
            .filter_map(|(_, p)| kinds(p))
            .reduce(|a, b| a.intersection(&b).copied().collect()),
        Op::Any(items) => {
            let mut result = std::collections::BTreeSet::new();
            for item in items {
                result.extend(kinds(item)?);
            }
            Some(result)
        }
        _ => None,
    }
}

fn kind_filter(plan: &Plan, alias: &str) -> String {
    kinds(plan).map_or("1".into(), |k| {
        format!(
            "{alias}.kind IN ({})",
            k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
        )
    })
}

impl Sql {
    /// Node-local operations need no demand/result join. CASE preserves reached work.
    fn scalar(plan: &Plan, input: &Demand) -> Result<Option<(String, bool)>, String> {
        if !plan.fallible()
            && plan.mentions.is_empty()
            && plan.writes.is_empty()
            && matches!(plan.op, Op::All(_) | Op::Any(_))
            && let Ok(test) = ordered::truth(plan, 0, "n")
        {
            return Ok(Some((test, true)));
        }
        Ok(Some(match &plan.op {
            Op::Predicate(set) => (
                format!(
                    "CASE WHEN {} THEN 1 ELSE 0 END",
                    predicate::condition(set, "n")?
                ),
                true,
            ),
            Op::Value(_) | Op::Charge(..) => return Ok(None),
            Op::Unknown(term) => (
                format!("CASE WHEN node_regex({term},d.file,d.node) THEN 2 ELSE 1 END"),
                false,
            ),
            Op::Bind(slot) => (
                format!(
                    "CASE WHEN n.named AND (CASE WHEN {} IS NULL THEN 1 ELSE relation_capture_equal(d.file,{},d.node) END) THEN 1 ELSE 0 END",
                    value(input, "d", *slot),
                    value(input, "d", *slot)
                ),
                true,
            ),
            Op::Not(child) => {
                if child.fallible() {
                    return Ok(None);
                }
                let Some((expression, node)) = Self::scalar(child, input)? else {
                    return Ok(None);
                };
                (
                    format!("CASE ({expression}) WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE 2 END"),
                    node,
                )
            }
            _ => return Ok(None),
        }))
    }

    fn scalar_step(
        &mut self,
        state: &Demand,
        plan: &Plan,
        reached: &str,
    ) -> Result<Option<String>, String> {
        if let Op::Value(term) = &plan.op {
            self.generation.operator()?;
            return Ok(Some(self.relation("guarded budgeted value",format!("SELECT d.*,v.truth AS tested,v.work AS remaining FROM {} d LEFT JOIN relation_value_budget({term},d.file,d.node,CASE WHEN {reached} THEN d.work END) v ON 1",state.name),true,"input state,tested outcome,remaining allowance")));
        }
        let Some((expression, node)) = Self::scalar(plan, state)? else {
            return Ok(None);
        };
        self.generation.tree(plan)?;
        let source = if node {
            " CROSS JOIN node n ON n.file=d.file AND n.id=d.node"
        } else {
            ""
        };
        Ok(Some(self.relation("guarded node-local outcome",format!("SELECT d.*,d.work AS remaining,CASE WHEN {reached} THEN {expression} ELSE NULL END AS tested FROM {} d{source}",state.name),true,"input columns,tested outcome (NULL means unreached)")))
    }

    fn relation(&mut self, operator: &str, body: String, stored: bool, columns: &str) -> String {
        let name = format!("outcome{}", self.definitions.len());
        self.definitions.push(format!(
            "{name} AS {}(\n{body}\n)",
            if stored { "MATERIALIZED " } else { "" }
        ));
        self.profile.push(json!({"relation":name,"operator":operator,"columns":columns,"storage":if stored {"materialized"} else {"planner"}}));
        name
    }

    fn input(&mut self, plan: &Plan, source: &Demand, filter: &str) -> Demand {
        let mut slots = needed(plan);
        if plan.uses_memo() {
            slots.extend(&source.slots);
        }
        let columns = fields(&slots, |slot| value(source, "d", slot));
        Demand {
            name: self.relation(
                "reached demand",
                format!(
                    "SELECT d.k,d.file,d.node,d.work{columns} FROM {} d WHERE {filter}",
                    source.name
                ),
                true,
                "k,file,node,required bindings",
            ),
            slots,
        }
    }

    fn emit(&mut self, plan: &Plan, input: &Demand) -> Result<Output, String> {
        if self.generation.memo_clause.is_none() && memo::region(plan)? {
            return memo::shared(self, plan, input);
        }
        self.generation.operator()?;
        let operator = match &plan.op {
            Op::Predicate(_) => "total predicate",
            Op::Unknown(_) => "unknown leaf",
            Op::Value(_) => "reached value leaf",
            Op::Bind(_) => "capture equality",
            Op::Pattern(..) => "typed pattern witnesses",
            Op::BoundLeaf { .. } => "typed atomic captures",
            Op::FieldNeighbor { .. } => "field neighbor",
            Op::Constrained(..) => "ordered capture constraints",
            Op::Not(_) => "negation",
            Op::Count { .. } => "count outcome",
            _ => "outcome",
        };
        let body = match &plan.op {
            Op::Chain(id, chain) => return self.chain(*id, chain, input),
            Op::BoundLeaf {
                term,
                inputs,
                outputs,
                ..
            } => {
                let arguments = inputs
                    .iter()
                    .enumerate()
                    .map(|(index, slot)| format!(" AND p.i{index} IS {}", value(input, "d", *slot)))
                    .collect::<String>();
                let exports = outputs
                    .iter()
                    .enumerate()
                    .map(|(index, slot)| format!(",p.c{index} AS c{slot}"))
                    .collect::<String>();
                format!(
                    "SELECT d.k,p.work,p.truth{exports} FROM {} d CROSS JOIN relation_bound_budget p ON p.term={term} AND p.file=d.file AND p.node=d.node AND p.allowance=d.work{arguments}",
                    input.name
                )
            }
            Op::FieldNeighbor {
                inside,
                field,
                child,
            } => return self.field_neighbor(*inside, *field, child, input),
            Op::Constrained(core, constraints) => {
                return self.constrained(plan, core, constraints, input);
            }
            Op::Pattern(term, slots, _) => {
                let inputs = slots
                    .iter()
                    .enumerate()
                    .map(|(index, slot)| format!(" AND p.i{index} IS {}", value(input, "d", *slot)))
                    .collect::<String>();
                let exports = slots
                    .iter()
                    .enumerate()
                    .map(|(index, slot)| format!(",p.c{index} AS c{slot}"))
                    .collect::<String>();
                format!(
                    "SELECT d.k,d.work,p.truth{exports} FROM {} d CROSS JOIN relation_pattern_typed p ON p.term={term} AND p.file=d.file AND p.node=d.node{inputs}",
                    input.name
                )
            }
            Op::Predicate(set) => format!(
                "SELECT d.k,d.work,CASE WHEN {} THEN 1 ELSE 0 END AS truth FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node",
                predicate::condition(set, "n")?,
                input.name
            ),
            Op::Value(term) => format!(
                "SELECT d.k,v.work,v.truth FROM {} d CROSS JOIN relation_value_budget({term},d.file,d.node,d.work) v",
                input.name
            ),
            Op::Unknown(term) => format!(
                "SELECT d.k,d.work,CASE WHEN node_regex({term},d.file,d.node) THEN 2 ELSE 1 END AS truth FROM {} d",
                input.name
            ),
            Op::Bind(slot) => format!(
                "SELECT d.k,d.work,CASE WHEN n.named AND (CASE WHEN {} IS NULL THEN 1 ELSE relation_capture_equal(d.file,{},d.node) END) THEN 1 ELSE 0 END AS truth,d.node AS c{slot} FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node",
                value(input, "d", *slot),
                value(input, "d", *slot),
                input.name
            ),
            Op::Not(child) => {
                let demand = self.input(child, input, "1");
                let output = self.emit(child, &demand)?;
                format!(
                    "SELECT k,work,CASE truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE truth END AS truth FROM {}",
                    output.name
                )
            }
            Op::Charge(utility, child) => {
                if self.generation.memo_clause.is_some() && child.reads_value() {
                    return self.memo_utility(*utility, child, input);
                }
                let (query, profile) = ordered::nested::program(
                    child,
                    input,
                    false,
                    "d.work>0",
                    &child.writes,
                    ("d.file", "d.node", "d.work-1"),
                    self.generation.entered(),
                )?;
                let columns = fields(&child.writes, |slot| {
                    let index = child.writes.iter().position(|s| s == &slot).unwrap();
                    format!("r.c{}", index + 2)
                });
                let name = self.relation("utility entry charge", format!("SELECT d.k,COALESCE(r.c1,0) AS work,CASE WHEN d.work=0 THEN -2 ELSE r.c0 END AS truth{columns} FROM {} d {}",input.name,ordered::nested::bridge(&query,2+child.writes.len())),true,"k,work,truth,captures");
                self.profile.last_mut().unwrap()["childPrograms"] = json!({"utility":profile});
                return Ok(Output { name });
            }
            Op::All(items) => return self.all(plan, items, input),
            Op::Any(items) => return self.any(plan, items, input),
            Op::FieldEnd {
                inside,
                field,
                child,
            } => return self.field_end(*inside, *field, child, input),
            Op::Search(axis, end, child, stop) => {
                if stop.is_some() || child.fallible() {
                    return self.ordered(*axis, *end, child, stop.as_deref(), None, input);
                }
                return self.search(*axis, *end, child, input);
            }
            Op::Count {
                axis,
                end,
                rule,
                stop,
                min,
                max,
                binding,
            } => {
                return self.ordered(
                    *axis,
                    *end,
                    rule,
                    stop.as_deref(),
                    Some((*min, *max, *binding)),
                    input,
                );
            }
        };
        Ok(Output {
            name: self.relation(operator, body, true, "k,truth,observable bindings"),
        })
    }

    fn all(&mut self, plan: &Plan, items: &[Plan], input: &Demand) -> Result<Output, String> {
        let mut slots = needed(plan);
        if plan.uses_memo() {
            slots.extend(&input.slots);
        }
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let withheld: String = plan
            .node_writes
            .iter()
            .map(|slot| format!(",0 AS u{slot}"))
            .collect();
        let mut state = Demand {
            name: self.relation(
                "conjunction input",
                format!(
                    "SELECT d.k,d.file,d.node,d.work,1 AS truth{columns}{withheld} FROM {} d",
                    input.name
                ),
                true,
                "k,file,node,truth,live bindings,withheld flags",
            ),
            slots,
        };
        let mut memo = memo::Local::new(&state.slots, items);
        let mut skip_until = 0;
        for (index, child) in items.iter().enumerate() {
            if index < skip_until {
                continue;
            }
            if index > 0 {
                memo.wrote(&items[index - 1]);
            }
            let carried = memo.carry("d");
            let blocked = child
                .mentions
                .intersection(&plan.node_writes)
                .map(|slot| format!("d.u{slot}"))
                .collect::<Vec<_>>();
            let filter = format!(
                "d.truth>0 AND NOT ({})",
                if blocked.is_empty() {
                    "0".into()
                } else {
                    blocked.join(" OR ")
                }
            );
            if let Some(tested) = self.scalar_step(&state, child, &filter)? {
                let columns = fields(&state.slots, |slot| {
                    if child.writes.contains(&slot) {
                        format!("CASE WHEN d.tested=1 THEN d.node ELSE d.c{slot} END")
                    } else {
                        format!("d.c{slot}")
                    }
                });
                let withheld: String = plan.node_writes.iter().map(|slot| if child.node_writes.contains(slot) {
                    format!(",(d.u{slot} OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u{slot}")
                } else {format!(",d.u{slot}")}).collect();
                state.name=self.relation("conjunction scalar barrier",format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth{columns}{withheld}{carried} FROM {tested} d"),true,"k,file,node,truth,live bindings,withheld flags");
                continue;
            }
            // An atomic witness step must reference its predecessor once. A
            // demand/result diamond expands that predecessor during SQLite prepare.
            if self.correlated || matches!(child.op, Op::Pattern(..)) {
                let repeats = if self.generation.memo_clause.is_some() {
                    repeat_walk::length(&items[index..])
                } else {
                    1
                };
                let tested = if repeats > 1 {
                    skip_until = index + repeats;
                    self.repeat_step(&state, child, &filter, repeats, repeat_walk::Boolean::All)?
                } else if let Some(tested) = memo.step(self, &state, child, &filter)? {
                    if let Some(group) = memo.group(&items[index..]) {
                        skip_until = index + group.calls;
                        memo.replay_group(self, &tested, &group, repeat_cost::Boolean::All)?
                    } else {
                        tested
                    }
                } else {
                    self.correlated_step(&state, child, &filter)?
                };
                let carried = memo.carry("d");
                let columns = fields(&state.slots, |slot| {
                    if child.writes.contains(&slot) {
                        format!("CASE WHEN d.tested=1 THEN d.e{slot} ELSE d.c{slot} END")
                    } else {
                        format!("d.c{slot}")
                    }
                });
                let withheld:String=plan.node_writes.iter().map(|slot|if child.node_writes.contains(slot){format!(",(d.u{slot} OR COALESCE(d.tested=2,0) OR (d.tested IS NULL AND d.truth<>0)) AS u{slot}")}else{format!(",d.u{slot}")}).collect();
                state.name=self.relation("correlated conjunction barrier",format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth{columns}{withheld}{carried} FROM {tested} d"),true,"k,file,node,truth,live bindings,withheld flags");
                continue;
            }
            let reached = self.input(child, &state, &filter);
            let output = self.emit(child, &reached)?;
            let columns = fields(&state.slots, |slot| {
                if child.writes.contains(&slot) {
                    format!("CASE WHEN o.truth=1 THEN o.c{slot} ELSE d.c{slot} END")
                } else {
                    format!("d.c{slot}")
                }
            });
            let withheld: String = plan
                .node_writes
                .iter()
                .map(|slot| {
                    if child.node_writes.contains(slot) {
                        format!(
                            ",(d.u{slot} OR o.truth=2 OR (o.k IS NULL AND d.truth<>0)) AS u{slot}"
                        )
                    } else {
                        format!(",d.u{slot}")
                    }
                })
                .collect();
            state.name = self.relation("conjunction sequencing barrier",format!("SELECT d.k,d.file,d.node,COALESCE(o.work,d.work) AS work,CASE WHEN d.truth<0 THEN d.truth WHEN o.truth<0 THEN o.truth WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth{columns}{withheld} FROM {} d LEFT JOIN {} o ON o.k=d.k",state.name,output.name),true,"k,file,node,truth,live bindings,withheld flags");
        }
        // Bindings are observable only on true; callers keep their input on false/unknown.
        let columns = fields(&plan.writes, |slot| format!("s.c{slot}"));
        Ok(Output {
            name: self.relation(
                "conjunction commit",
                format!("SELECT s.k,s.work,s.truth{columns} FROM {} s", state.name),
                true,
                "k,truth,observable bindings",
            ),
        })
    }

    fn any(&mut self, plan: &Plan, items: &[Plan], input: &Demand) -> Result<Output, String> {
        let mut slots = needed(plan);
        if plan.uses_memo() {
            slots.extend(&input.slots);
        }
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let mut state = Demand {
            name: self.relation(
                "alternative input",
                format!(
                    "SELECT d.k,d.file,d.node,d.work,0 AS truth,0 AS open{columns} FROM {} d",
                    input.name
                ),
                true,
                "k,file,node,truth,open,live bindings",
            ),
            slots,
        };
        let mut memo = memo::Local::new(&state.slots, items);
        let mut skip_until = 0;
        for (index, child) in items.iter().enumerate() {
            if index < skip_until {
                continue;
            }
            if index > 0 {
                memo.wrote(&items[index - 1]);
            }
            let carried = memo.carry("d");
            if let Some(tested) = self.scalar_step(&state, child, "d.truth=0")? {
                let columns = fields(&state.slots, |slot| {
                    if child.writes.contains(&slot) {
                        format!("CASE WHEN d.tested=1 THEN d.node ELSE d.c{slot} END")
                    } else {
                        format!("d.c{slot}")
                    }
                });
                let strict = usize::from(!child.writes.is_empty());
                state.name=self.relation("alternative scalar barrier",format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<>0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.tested=1 THEN 1 WHEN d.tested=2 AND {strict} THEN 2 ELSE 0 END AS truth,(d.open OR COALESCE(d.tested=2,0)) AS open{columns}{carried} FROM {tested} d"),true,"k,file,node,truth,open,live bindings");
                continue;
            }
            // An atomic witness step must reference its predecessor once. A
            // demand/result diamond expands that predecessor during SQLite prepare.
            if self.correlated || matches!(child.op, Op::Pattern(..)) {
                let repeats = if self.generation.memo_clause.is_some() {
                    repeat_walk::length(&items[index..])
                } else {
                    1
                };
                let tested = if repeats > 1 {
                    skip_until = index + repeats;
                    self.repeat_step(
                        &state,
                        child,
                        "d.truth=0",
                        repeats,
                        repeat_walk::Boolean::Any,
                    )?
                } else if let Some(tested) = memo.step(self, &state, child, "d.truth=0")? {
                    if let Some(group) = memo.group(&items[index..]) {
                        skip_until = index + group.calls;
                        memo.replay_group(self, &tested, &group, repeat_cost::Boolean::Any)?
                    } else {
                        tested
                    }
                } else {
                    self.correlated_step(&state, child, "d.truth=0")?
                };
                let carried = memo.carry("d");
                let columns = fields(&state.slots, |slot| {
                    if child.writes.contains(&slot) {
                        format!("CASE WHEN d.tested=1 THEN d.e{slot} ELSE d.c{slot} END")
                    } else {
                        format!("d.c{slot}")
                    }
                });
                let strict = usize::from(!child.writes.is_empty());
                state.name=self.relation("correlated alternative barrier",format!("SELECT d.k,d.file,d.node,COALESCE(d.remaining,d.work) AS work,CASE WHEN d.truth<>0 THEN d.truth WHEN d.tested<0 THEN d.tested WHEN d.tested=1 THEN 1 WHEN d.tested=2 AND {strict} THEN 2 ELSE 0 END AS truth,(d.open OR COALESCE(d.tested=2,0)) AS open{columns}{carried} FROM {tested} d"),true,"k,file,node,truth,open,live bindings");
                continue;
            }
            let reached = self.input(child, &state, "d.truth=0");
            let output = self.emit(child, &reached)?;
            let columns = fields(&state.slots, |slot| {
                if child.writes.contains(&slot) {
                    format!("CASE WHEN o.truth=1 THEN o.c{slot} ELSE d.c{slot} END")
                } else {
                    format!("d.c{slot}")
                }
            });
            let strict = usize::from(!child.writes.is_empty());
            state.name = self.relation("ordered alternative barrier",format!("SELECT d.k,d.file,d.node,COALESCE(o.work,d.work) AS work,CASE WHEN d.truth<>0 THEN d.truth WHEN o.truth<0 THEN o.truth WHEN o.truth=1 THEN 1 WHEN o.truth=2 AND {strict} THEN 2 ELSE 0 END AS truth,(d.open OR COALESCE(o.truth=2,0)) AS open{columns} FROM {} d LEFT JOIN {} o ON o.k=d.k",state.name,output.name),true,"k,file,node,truth,open,live bindings");
        }
        // Bindings are observable only on true; callers keep their input on false/unknown.
        let columns = fields(&plan.writes, |slot| format!("s.c{slot}"));
        Ok(Output {name:self.relation("alternative commit",format!("SELECT s.k,s.work,CASE WHEN s.truth=0 AND s.open THEN 2 ELSE s.truth END AS truth{columns} FROM {} s",state.name),true,"k,truth,observable bindings")})
    }

    fn candidates(&mut self, axis: Axis, end: bool, plan: &Plan, input: &Demand) -> Demand {
        let mut slots = needed(plan);
        if plan.uses_memo() {
            slots.extend(&input.slots);
        }
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let guard = kind_filter(plan, "n");
        let (source, rank) = if end && !matches!(axis, Axis::Child) {
            let edge = match axis {
                Axis::Parent => "parent",
                Axis::Previous => "prev",
                Axis::Next => "next",
                _ => unreachable!(),
            };
            let walk = format!("outcome{}", self.definitions.len());
            self.relation("narrow structural walk",format!("SELECT d.k AS parent,n.file,n.{edge} AS node,1 AS distance FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node WHERE n.{edge} IS NOT NULL UNION ALL SELECT w.parent,n.file,n.{edge},w.distance+1 FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.{edge} IS NOT NULL",input.name),true,"parent,file,node,distance");
            (
                format!(
                    "{walk} w JOIN {} d ON d.k=w.parent CROSS JOIN node n ON n.file=w.file AND n.id=w.node",
                    input.name
                ),
                "w.distance",
            )
        } else {
            let condition = match (axis, end) {
                (Axis::Child, true) => "n.id>origin.id AND n.id<=origin.\"last\"",
                (Axis::Child, false) => "n.parent=origin.id",
                (Axis::Parent, _) => "n.id=origin.parent",
                (Axis::Previous, _) => "n.id=origin.prev",
                (Axis::Next, _) => "n.id=origin.next",
            };
            (
                format!(
                    "{} d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node CROSS JOIN node n ON n.file=origin.file AND {condition}",
                    input.name
                ),
                "n.id",
            )
        };
        Demand {name:self.relation("candidate demand",format!("SELECT ROW_NUMBER() OVER () AS k,d.k AS parent,n.file,n.id AS node,d.work,{rank} AS rank{columns} FROM {source} WHERE {guard}"),true,"k,parent,file,node,rank,required bindings"),slots}
    }

    fn search(
        &mut self,
        axis: Axis,
        end: bool,
        plan: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        let candidates = self.candidates(axis, end, plan, input);
        let output = self.emit(plan, &candidates)?;
        let order = if plan.writes.is_empty() {
            "CASE o.truth WHEN 1 THEN 0 ELSE 1 END,d.rank"
        } else {
            "d.rank"
        };
        let columns = fields(&plan.writes, |slot| format!("o.c{slot}"));
        let ranked = self.relation("ordered search reduction",format!("SELECT d.parent,o.truth{columns},ROW_NUMBER() OVER (PARTITION BY d.parent ORDER BY {order}) AS choice FROM {} d JOIN {} o ON o.k=d.k WHERE o.truth<>0",candidates.name,output.name),false,"parent,truth,observable bindings,choice");
        let selected = self.relation(
            "first decisive candidate",
            format!("SELECT * FROM {ranked} WHERE choice=1"),
            true,
            "parent,truth,observable bindings,choice",
        );
        let columns = fields(&plan.writes, |slot| {
            format!(
                "CASE WHEN o.truth=1 THEN o.c{slot} ELSE {} END",
                value(input, "d", slot)
            )
        });
        Ok(Output {name:self.relation("search outcome",format!("SELECT d.k,d.work,COALESCE(o.truth,0) AS truth{columns} FROM {} d LEFT JOIN {selected} o ON o.parent=d.k",input.name),true,"k,truth,observable bindings")})
    }
}

pub(super) fn compile(
    plan: &Plan,
    slots: &[Slot],
    applies: bool,
) -> Result<(String, Value), String> {
    let mut sql = Sql {
        definitions: vec![],
        profile: vec![],
        correlated: false,
        generation: Generation::default(),
    };
    let roots = Demand {
        name: sql.relation(
            "root demand",
            format!(
                "SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,9223372036854775807 AS work FROM node n WHERE {} AND {}",
                usize::from(applies),
                kind_filter(plan, "n")
            ),
            true,
            "k,file,node",
        ),
        slots: Slots::new(),
    };
    let output = sql.emit(plan, &roots)?;
    let mut observed: Vec<_> = plan.writes.iter().copied().collect();
    observed.sort_by_key(|&slot| &slots[slot].name);
    let payload = observed
        .iter()
        .map(|slot| slots[*slot].kind.payload(&format!("o.c{slot}"), "d.file"))
        .collect::<Vec<_>>()
        .join(",");
    let query = format!(
        "WITH RECURSIVE\n{}\n-- final matches\nSELECT 0,d.file,d.node,json_array({payload}) FROM {} d JOIN {} o ON o.k=d.k WHERE o.truth=1 ORDER BY d.file,d.node",
        sql.definitions.join(",\n"),
        roots.name,
        output.name
    );
    if query.len() > 256_000 {
        return Err("outcome relations: SQL text exceeds 256000 bytes".into());
    }
    Ok((
        query,
        json!({
            "scope":"Explicit outcomes with reached node-local value leaves and traversal stops",
            "outcomeOutput":output.name,"outcomeDemand":roots.name,"outcomePayload":payload,
            "outcomes":{"false":0,"true":1,"unknown":2},"nativeComposition":false,
            "captures":observed.iter().map(|&id|json!({"name":slots[id].name,"slot":id,"kind":slots[id].kind.name()})).collect::<Vec<_>>(),
            "relations":sql.profile,"compositionOperators":[sql.generation.operators.get()],
            "limitations":["Native work and stack/depth budgets remain unproved.","Following values, traversal depth beyond four, recursion, enumeration, and full-file failure publication remain unproved."],
            "flags":["Ordered node-local capture programs admit at most 61 slots and Boolean depth 16; masks keep recursive state at ten columns.","Only total structural search plans may evaluate candidates before ordered reduction.","Counts use reached candidate visits and carry a native work allowance."]
        }),
    ))
}
