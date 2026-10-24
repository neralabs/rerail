//! Reached node-local tests along a structural walk. Three phases preserve native stop/test order.
use super::{Axis, Demand, Op, Output, Plan, Sql};

#[path = "candidate.rs"]
mod candidate;
#[path = "nested.rs"]
pub(super) mod nested;

type Bounds = (i64, Option<i64>, Option<(usize, i64)>);

pub(super) fn candidate_filter(rule: &Plan, alias: &str) -> String {
    let Some(filter) = &rule.walk_filter else {
        return "1".into();
    };
    let mut terms = vec![if filter.kinds.is_empty() {
        "0".into()
    } else {
        format!(
            "{alias}.kind IN ({})",
            filter
                .kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    }];
    terms.extend(
        filter
            .text
            .iter()
            .map(|term| format!("node_regex({term},{alias}.file,{alias}.id)")),
    );
    terms.join(" AND ")
}

pub(super) fn boolean(plan: &Plan, alias: &str) -> Result<Option<String>, String> {
    Ok(Some(match &plan.op {
        Op::Predicate(set) => super::predicate::condition(set, alias)?,
        Op::NthChild {
            step,
            offset,
            reverse,
            of_rule: None,
            ..
        } => super::nth_emit::plain(*step, *offset, *reverse, alias),
        Op::Not(child) => {
            let Some(test) = boolean(child, alias)? else {
                return Ok(None);
            };
            format!("NOT ({test})")
        }
        Op::All(items) | Op::Any(items) => {
            let mut terms = vec![];
            for child in items {
                let Some(test) = boolean(child, alias)? else {
                    return Ok(None);
                };
                terms.push(format!("({test})"));
            }
            let all = matches!(plan.op, Op::All(_));
            if terms.is_empty() {
                usize::from(all).to_string()
            } else {
                terms.join(if all { " AND " } else { " OR " })
            }
        }
        _ => return Ok(None),
    }))
}

pub(super) fn truth(plan: &Plan, depth: usize, alias: &str) -> Result<String, String> {
    if depth > 16 || !plan.mentions.is_empty() || !plan.writes.is_empty() {
        return Err(
            "ordered outcomes: node-local capture-free tests of depth at most 16 required".into(),
        );
    }
    if let Some(test) = boolean(plan, alias)? {
        return Ok(format!("CASE WHEN {test} THEN 1 ELSE 0 END"));
    }
    Ok(match &plan.op {
        Op::Predicate(set) => format!(
            "CASE WHEN {} THEN 1 ELSE 0 END",
            super::predicate::condition(set, alias)?
        ),
        Op::Unknown(term) => {
            format!("CASE WHEN node_regex({term},{alias}.file,{alias}.id) THEN 2 ELSE 1 END")
        }
        Op::Not(child) => format!(
            "(WITH neg AS MATERIALIZED (SELECT {} AS t) SELECT CASE t WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE t END FROM neg)",
            truth(child, depth + 1, alias)?
        ),
        Op::All(items) | Op::Any(items) => {
            let all = matches!(plan.op, Op::All(_));
            let decisive = if all { 0 } else { 1 };
            let mut expression = if all { "1" } else { "0" }.to_string();
            for (index, child) in items.iter().enumerate() {
                // Each operand has one physical evaluation, and CASE prevents unreached calls.
                let next = truth(child, depth + 1, alias)?;
                if index == 0 {
                    expression = next;
                    continue;
                }
                expression = format!(
                    "(WITH prior AS MATERIALIZED (SELECT {expression} AS t) SELECT CASE WHEN t<0 THEN t WHEN t={decisive} THEN {decisive} ELSE (WITH following AS MATERIALIZED (SELECT {next} AS v) SELECT CASE WHEN v<0 THEN v WHEN v={decisive} THEN {decisive} WHEN v=2 OR prior.t=2 THEN 2 ELSE v END FROM following) END FROM prior)"
                );
            }
            expression
        }
        _ => return Err("ordered outcomes: traversal bodies and stops must be node-local".into()),
    })
}

impl Sql {
    pub(super) fn ordered(
        &mut self,
        axis: Axis,
        end: bool,
        rule: &Plan,
        stop: Option<&Plan>,
        bounds: Option<Bounds>,
        input: &Demand,
    ) -> Result<Output, String> {
        let span_inputs = |plan: &Plan| {
            self.generation.memo_layout.as_ref().is_some_and(|layout| {
                plan.mentions
                    .union(&plan.writes)
                    .any(|slot| layout.span_capable(*slot))
            })
        };
        if rule.fallible()
            || span_inputs(rule)
            || stop.is_some_and(span_inputs)
            || stop.is_some_and(Plan::fallible)
            || bounds.is_some()
            || nested::contains_walk(rule)
            || stop.is_some_and(nested::contains_walk)
        {
            return nested::emit(self, axis, end, rule, stop, bounds, input);
        }
        self.generation.tree(rule)?;
        if let Some(stop) = stop {
            self.generation.tree(stop)?;
        }
        let strict = bounds.is_none() && !rule.writes.is_empty();
        let captured = if !rule.mentions.is_empty() || !rule.writes.is_empty() {
            Some(candidate::compile(rule, input, false, strict)?)
        } else {
            None
        };
        let body = captured
            .as_ref()
            .map(|c| Ok(c.expression.clone()))
            .unwrap_or_else(|| truth(rule, 0, "n"))?;
        let stop_test = stop
            .map(|s| {
                if s.mentions.is_empty() && s.writes.is_empty() {
                    truth(s, 0, "n")
                } else {
                    candidate::compile(s, input, true, false).map(|c| c.expression)
                }
            })
            .transpose()?
            .unwrap_or("0".into());
        let own = if strict {
            "(CASE WHEN w.own<0 THEN w.own ELSE w.own&3 END)"
        } else {
            "w.own"
        };
        let child = matches!(axis, Axis::Child);
        let first = match axis {
            Axis::Child => "CASE WHEN origin.id<origin.\"last\" THEN origin.id+1 END",
            Axis::Parent => "origin.parent",
            Axis::Previous => "origin.prev",
            Axis::Next => "origin.next",
        };
        let next = match (axis, end) {
            (Axis::Child, false) => "n.next".to_string(),
            (Axis::Child, true) => {
                "CASE WHEN w.stopped=0 THEN n.id+1 ELSE n.\"last\"+1 END".to_string()
            }
            (_, false) => "NULL".to_string(),
            (axis, true) => format!(
                "CASE WHEN w.stopped=0 THEN n.{} END",
                match axis {
                    Axis::Parent => "parent",
                    Axis::Previous => "prev",
                    Axis::Next => "next",
                    _ => unreachable!(),
                }
            ),
        };
        let next = if child {
            format!("CASE WHEN ({next})<=w.bound THEN ({next}) END")
        } else {
            next
        };
        let low = format!("(w.low+({own}=1))");
        let uncertain = format!("(w.uncertain+({own}=2))");
        let open = "(w.open OR w.stopped=2)";
        let decisive = if let Some((min, max, binding)) = bounds {
            max.map(|m| format!("{low}>{m}")).unwrap_or_else(|| {
                if binding.is_none() {
                    format!("{low}>={min}")
                } else {
                    "0".into()
                }
            })
        } else if strict {
            format!("{own}=1 OR {own}=2 OR w.open")
        } else {
            format!("{own}=1")
        };
        let decisive = format!("w.own<0 OR w.stopped<0 OR ({decisive})");
        let first_test = if child {
            body.as_str()
        } else {
            stop_test.as_str()
        };
        let second_test = if child {
            format!("CASE WHEN {decisive} THEN 0 ELSE {stop_test} END")
        } else {
            format!("CASE WHEN w.stopped<0 THEN 0 ELSE {body} END")
        };
        let walk = format!("outcome{}", self.definitions.len());
        self.relation("ordered reached traversal",format!(
            "SELECT d.k,d.file,{first} AS node,origin.\"last\" AS bound,0 AS phase,0 AS own,0 AS stopped,0 AS low,0 AS uncertain,0 AS open FROM {} d CROSS JOIN node origin ON origin.file=d.file AND origin.id=d.node WHERE ({first}) IS NOT NULL\nUNION ALL\nSELECT w.k,w.file,CASE WHEN w.phase=2 THEN {next} ELSE w.node END,w.bound,(w.phase+1)%3,{}, {},CASE WHEN w.phase=2 THEN {low} ELSE w.low END,CASE WHEN w.phase=2 THEN {uncertain} ELSE w.uncertain END,CASE WHEN w.phase=2 THEN {open} ELSE w.open END FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase<2 OR (NOT ({decisive}) AND ({next}) IS NOT NULL)",
            input.name,
            if child {format!("CASE WHEN w.phase=0 THEN {first_test} ELSE w.own END")}else{format!("CASE WHEN w.phase=1 THEN {second_test} ELSE w.own END")},
            if child {format!("CASE WHEN w.phase=1 THEN {second_test} ELSE w.stopped END")}else{format!("CASE WHEN w.phase=0 THEN {first_test} ELSE w.stopped END")}
        ),true,"k,file,node,bound,phase,own,stopped,low,uncertain,open");
        let (witness, schema) = if strict {
            (
                ",w.node AS witness,w.own AS captured",
                "k,witness,captured,failure,low,uncertain,open",
            )
        } else {
            ("", "k,failure,low,uncertain,open")
        };
        let terminal=self.relation("completed ordered traversal",format!("SELECT w.k{witness},CASE WHEN w.own<0 THEN w.own WHEN w.stopped<0 THEN w.stopped ELSE 0 END AS failure,{low} AS low,{uncertain} AS uncertain,{open} AS open FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=2 AND (({decisive}) OR ({next}) IS NULL)"),true,schema);
        let low = "COALESCE(t.low,0)";
        let high = "(COALESCE(t.low,0)+COALESCE(t.uncertain,0))";
        let closed = "NOT COALESCE(t.open,0)";
        let (verdict, columns) = if let Some((min, max, binding)) = bounds {
            let too_many = max.map_or("0".into(), |m| format!("{low}>{m}"));
            let within = max.map_or("1".into(), |m| format!("{closed} AND {high}<={m}"));
            let exact = if binding.is_some() {
                format!("{closed} AND {low}={high}")
            } else {
                "1".into()
            };
            (
                format!(
                    "CASE WHEN ({closed} AND {high}<{min}) OR {too_many} THEN 0 WHEN {low}>={min} AND ({within}) AND ({exact}) THEN 1 ELSE 2 END"
                ),
                binding.map_or(String::new(), |(slot, offset)| {
                    format!(",{low}+({offset}) AS c{slot}")
                }),
            )
        } else {
            (
                format!(
                    "CASE WHEN {low}>0 THEN 1 WHEN COALESCE(t.open,0) OR COALESCE(t.uncertain,0)>0 THEN 2 ELSE 0 END"
                ),
                captured.as_ref().map_or(String::new(),|candidate| candidate.bits.iter().filter(|(slot,_)|rule.writes.contains(slot)).map(|(slot,bit)|format!(",CASE WHEN (t.captured&{})<>0 THEN t.witness ELSE {} END AS c{slot}",bit<<2,super::value(input,"d",*slot))).collect()),
            )
        };
        Ok(Output{name:self.relation("ordered traversal outcome",format!("SELECT d.k,d.work,CASE WHEN t.failure<0 THEN t.failure ELSE {verdict} END AS truth{columns} FROM {} d LEFT JOIN {terminal} t ON t.k=d.k",input.name),true,"k,truth,observable capture or count exports")})
    }
}
