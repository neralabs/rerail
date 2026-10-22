use super::super::ir::Set;
#[path = "ordered.rs"]
mod ordered;
use super::{Axis, Op, Plan, Slot, Slots};
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
fn kinds(plan: &Plan) -> Option<std::collections::BTreeSet<u16>> {
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
        Op::Predicate(Set::Kinds(k)) => Some(k.iter().copied().collect()),
        Op::Predicate(Set::Empty) => Some(Default::default()),
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
    /// Total node-local operations need no demand/result join. CASE preserves reached work.
    fn scalar(plan: &Plan, input: &Demand) -> Result<Option<(String, bool)>, String> {
        Ok(Some(match &plan.op {
            Op::Predicate(set) => (
                format!(
                    "CASE WHEN {} THEN 1 ELSE 0 END",
                    super::super::emit::condition(set, "n")?
                ),
                true,
            ),
            Op::Value(term) => (format!("relation_value({term},d.file,d.node)"), false),
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
        let Some((expression, node)) = Self::scalar(plan, state)? else {
            return Ok(None);
        };
        let source = if node {
            " CROSS JOIN node n ON n.file=d.file AND n.id=d.node"
        } else {
            ""
        };
        Ok(Some(self.relation("guarded node-local outcome",format!("SELECT d.*,CASE WHEN {reached} THEN {expression} ELSE NULL END AS tested FROM {} d{source}",state.name),true,"input columns,tested outcome (NULL means unreached)")))
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
        let slots = needed(plan);
        let columns = fields(&slots, |slot| value(source, "d", slot));
        Demand {
            name: self.relation(
                "reached demand",
                format!(
                    "SELECT d.k,d.file,d.node{columns} FROM {} d WHERE {filter}",
                    source.name
                ),
                true,
                "k,file,node,required bindings",
            ),
            slots,
        }
    }

    fn emit(&mut self, plan: &Plan, input: &Demand) -> Result<Output, String> {
        let operator = match &plan.op {
            Op::Predicate(_) => "total predicate",
            Op::Unknown(_) => "unknown leaf",
            Op::Value(_) => "reached value leaf",
            Op::Bind(_) => "capture equality",
            Op::Not(_) => "negation",
            Op::Count { .. } => "count outcome",
            _ => "outcome",
        };
        let body = match &plan.op {
            Op::Predicate(set) => format!(
                "SELECT d.k,CASE WHEN {} THEN 1 ELSE 0 END AS truth FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node",
                super::super::emit::condition(set, "n")?,
                input.name
            ),
            Op::Value(term) => format!(
                "SELECT d.k,relation_value({term},d.file,d.node) AS truth FROM {} d",
                input.name
            ),
            Op::Unknown(term) => format!(
                "SELECT d.k,CASE WHEN node_regex({term},d.file,d.node) THEN 2 ELSE 1 END AS truth FROM {} d",
                input.name
            ),
            Op::Bind(slot) => format!(
                "SELECT d.k,CASE WHEN n.named AND (CASE WHEN {} IS NULL THEN 1 ELSE relation_capture_equal(d.file,{},d.node) END) THEN 1 ELSE 0 END AS truth,d.node AS c{slot} FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node",
                value(input, "d", *slot),
                value(input, "d", *slot),
                input.name
            ),
            Op::Not(child) => {
                let demand = self.input(child, input, "1");
                let output = self.emit(child, &demand)?;
                format!(
                    "SELECT k,CASE truth WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE 2 END AS truth FROM {}",
                    output.name
                )
            }
            Op::All(items) => return self.all(plan, items, input),
            Op::Any(items) => return self.any(plan, items, input),
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
                if stop.is_some() || rule.fallible() {
                    return self.ordered(
                        *axis,
                        *end,
                        rule,
                        stop.as_deref(),
                        Some((*min, *max, *binding)),
                        input,
                    );
                }
                if *min == 1
                    && max.is_none()
                    && binding.is_none()
                    && (!end || matches!(axis, Axis::Child))
                    && let Some((holds, unknown)) = super::local::predicates(rule)?
                {
                    let edge = match (axis, end) {
                        (Axis::Child, true) => "n.id>origin.id AND n.id<=origin.\"last\"",
                        (Axis::Child, false) => "n.parent=origin.id",
                        (Axis::Parent, _) => "n.id=origin.parent",
                        (Axis::Previous, _) => "n.id=origin.prev",
                        (Axis::Next, _) => "n.id=origin.next",
                    };
                    let filter = kind_filter(rule, "n");
                    let probe = |predicate: &str| {
                        format!(
                            "EXISTS (SELECT 1 FROM node n WHERE n.file=d.file AND {edge} AND {filter} AND ({predicate}))"
                        )
                    };
                    return Ok(Output {name:self.relation("count existence probes",format!("SELECT d.k,CASE WHEN {} THEN 1 WHEN {} THEN 2 ELSE 0 END AS truth FROM {} d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node",probe(&holds),probe(&unknown),input.name),true,"k,truth")});
                }
                let candidates = self.candidates(*axis, *end, rule, input);
                let output = self.emit(rule, &candidates)?;
                let counts = self.relation("count interval",format!("SELECT d.parent,SUM(o.truth=1) AS low,SUM(o.truth<>0) AS high FROM {} d JOIN {} o ON o.k=d.k GROUP BY d.parent",candidates.name,output.name),true,"parent,low,high");
                let upper_false = max.map_or("0".into(), |max| format!("COALESCE(c.low,0)>{max}"));
                let upper_true = max.map_or("1".into(), |max| format!("COALESCE(c.high,0)<={max}"));
                let exact = if binding.is_some() {
                    "COALESCE(c.low,0)=COALESCE(c.high,0)"
                } else {
                    "1"
                };
                let captured = binding.map_or(String::new(), |(slot, offset)| {
                    format!(",COALESCE(c.low,0)+({offset}) AS c{slot}")
                });
                format!(
                    "SELECT d.k,CASE WHEN COALESCE(c.high,0)<{min} OR {upper_false} THEN 0 WHEN COALESCE(c.low,0)>={min} AND {upper_true} AND {exact} THEN 1 ELSE 2 END AS truth{captured} FROM {} d LEFT JOIN {counts} c ON c.parent=d.k",
                    input.name
                )
            }
        };
        Ok(Output {
            name: self.relation(operator, body, true, "k,truth,observable bindings"),
        })
    }

    fn all(&mut self, plan: &Plan, items: &[Plan], input: &Demand) -> Result<Output, String> {
        let slots = needed(plan);
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
                    "SELECT d.k,d.file,d.node,1 AS truth{columns}{withheld} FROM {} d",
                    input.name
                ),
                true,
                "k,file,node,truth,live bindings,withheld flags",
            ),
            slots,
        };
        for child in items {
            let blocked = child
                .mentions
                .intersection(&plan.node_writes)
                .map(|slot| format!("d.u{slot}"))
                .collect::<Vec<_>>();
            let filter = format!(
                "d.truth<>0 AND NOT ({})",
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
                state.name=self.relation("conjunction scalar barrier",format!("SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR d.tested=0 THEN 0 WHEN d.truth=2 OR d.tested=2 THEN 2 ELSE 1 END AS truth{columns}{withheld} FROM {tested} d"),true,"k,file,node,truth,live bindings,withheld flags");
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
            state.name = self.relation("conjunction sequencing barrier",format!("SELECT d.k,d.file,d.node,CASE WHEN d.truth=0 OR o.truth=0 THEN 0 WHEN d.truth=2 OR o.truth=2 THEN 2 ELSE 1 END AS truth{columns}{withheld} FROM {} d LEFT JOIN {} o ON o.k=d.k",state.name,output.name),true,"k,file,node,truth,live bindings,withheld flags");
        }
        // Bindings are observable only on true; callers keep their input on false/unknown.
        let columns = fields(&plan.writes, |slot| format!("s.c{slot}"));
        Ok(Output {
            name: self.relation(
                "conjunction commit",
                format!("SELECT s.k,s.truth{columns} FROM {} s", state.name),
                true,
                "k,truth,observable bindings",
            ),
        })
    }

    fn any(&mut self, plan: &Plan, items: &[Plan], input: &Demand) -> Result<Output, String> {
        let slots = needed(plan);
        let columns = fields(&slots, |slot| value(input, "d", slot));
        let mut state = Demand {
            name: self.relation(
                "alternative input",
                format!(
                    "SELECT d.k,d.file,d.node,0 AS truth,0 AS open{columns} FROM {} d",
                    input.name
                ),
                true,
                "k,file,node,truth,open,live bindings",
            ),
            slots,
        };
        for child in items {
            if let Some(tested) = self.scalar_step(&state, child, "d.truth=0")? {
                let columns = fields(&state.slots, |slot| {
                    if child.writes.contains(&slot) {
                        format!("CASE WHEN d.tested=1 THEN d.node ELSE d.c{slot} END")
                    } else {
                        format!("d.c{slot}")
                    }
                });
                let strict = usize::from(!child.writes.is_empty());
                state.name=self.relation("alternative scalar barrier",format!("SELECT d.k,d.file,d.node,CASE WHEN d.truth<>0 THEN d.truth WHEN d.tested=1 THEN 1 WHEN d.tested=2 AND {strict} THEN 2 ELSE 0 END AS truth,(d.open OR COALESCE(d.tested=2,0)) AS open{columns} FROM {tested} d"),true,"k,file,node,truth,open,live bindings");
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
            state.name = self.relation("ordered alternative barrier",format!("SELECT d.k,d.file,d.node,CASE WHEN d.truth<>0 THEN d.truth WHEN o.truth=1 THEN 1 WHEN o.truth=2 AND {strict} THEN 2 ELSE 0 END AS truth,(d.open OR COALESCE(o.truth=2,0)) AS open{columns} FROM {} d LEFT JOIN {} o ON o.k=d.k",state.name,output.name),true,"k,file,node,truth,open,live bindings");
        }
        // Bindings are observable only on true; callers keep their input on false/unknown.
        let columns = fields(&plan.writes, |slot| format!("s.c{slot}"));
        Ok(Output {name:self.relation("alternative commit",format!("SELECT s.k,CASE WHEN s.truth=0 AND s.open THEN 2 ELSE s.truth END AS truth{columns} FROM {} s",state.name),true,"k,truth,observable bindings")})
    }

    fn candidates(&mut self, axis: Axis, end: bool, plan: &Plan, input: &Demand) -> Demand {
        let slots = needed(plan);
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
        Demand {name:self.relation("candidate demand",format!("SELECT ROW_NUMBER() OVER () AS k,d.k AS parent,n.file,n.id AS node,{rank} AS rank{columns} FROM {source} WHERE {guard}"),true,"k,parent,file,node,rank,required bindings"),slots}
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
        Ok(Output {name:self.relation("search outcome",format!("SELECT d.k,COALESCE(o.truth,0) AS truth{columns} FROM {} d LEFT JOIN {selected} o ON o.parent=d.k",input.name),true,"k,truth,observable bindings")})
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
    };
    let roots = Demand {
        name: sql.relation(
            "root demand",
            format!(
                "SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node FROM node n WHERE {} AND {}",
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
        .map(|slot| {
            if slots[*slot].count {
                format!("CAST(o.c{slot} AS TEXT)")
            } else {
                format!("o.c{slot}")
            }
        })
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
            "captures":observed.iter().map(|&id|json!({"name":slots[id].name,"slot":id,"kind":if slots[id].count {"count"} else {"node"}})).collect::<Vec<_>>(),
            "relations":sql.profile,
            "limitations":["Native work and stack/depth budgets remain unproved.","Following values, nested ordered traversal bodies, capture-bearing stops, recursion, and enumeration remain unproved."],
            "flags":["Candidate outcomes precede ordered reduction only because all admitted operations are total and effect-free.","Counts may evaluate candidates after a native early threshold; resource-limit parity remains pending."]
        }),
    ))
}
