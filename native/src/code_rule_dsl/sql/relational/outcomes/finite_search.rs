//! Ordered structural continuations for the shared finite SQL machine.
use crate::code_rule_dsl::sql::relational::outcomes::{
    Axis, WalkFilter,
    finite_lower::{CountBounds, Instruction, Node, Program},
};

pub(super) fn push(parent: &str, cells: &[String], guard: &str) -> String {
    let values = cells
        .iter()
        .enumerate()
        .map(|(i, v)| format!(" AND frame.v{i} IS ({v})"))
        .collect::<String>();
    format!(
        "LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN {guard} THEN 1 END AND frame.parent_input IS {parent}{values}"
    )
}
pub(super) fn filter(value: Option<&WalkFilter>, alias: &str) -> String {
    let Some(value) = value else {
        return "1".into();
    };
    let mut tests = vec![if value.kinds.is_empty() {
        "0".into()
    } else {
        format!(
            "{alias}.kind IN ({})",
            value
                .kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    }];
    tests.extend(
        value
            .text
            .iter()
            .map(|t| format!("node_regex({t},d.file,{alias}.id)")),
    );
    tests.join(" AND ")
}
pub(super) fn fresh(program: &Program, scope: &str, env: &str) -> String {
    let slots: std::collections::BTreeSet<_> = program
        .code
        .iter()
        .flat_map(|n| match &n.op {
            Instruction::Binding { children, .. } => {
                children.iter().map(|(s, _)| *s).collect::<Vec<_>>()
            }
            _ => vec![],
        })
        .collect();
    let (lo, hi) = slots.into_iter().fold((0_u64, 0_u64), |(lo, hi), slot| {
        if slot < 64 {
            (lo | (1_u64 << slot), hi)
        } else {
            (lo, hi | (1_u64 << (slot - 64)))
        }
    });
    format!(
        "relation_state_project({scope},{env},{},{})",
        lo as i64, hi as i64
    )
}
fn final_truth(bounds: Option<CountBounds>, low: &str, uncertain: &str, open: &str) -> String {
    if let Some((min, max, binding)) = bounds {
        let high = format!("({low}+{uncertain})");
        let too_many = max.map_or("0".into(), |max| format!("{low}>{max}"));
        let within = max.map_or("1".into(), |max| format!("NOT {open} AND {high}<={max}"));
        let exact = if binding.is_some() {
            format!("NOT {open} AND {low}={high}")
        } else {
            "1".into()
        };
        format!(
            "CASE WHEN (NOT {open} AND {high}<{min}) OR {too_many} THEN 0 WHEN {low}>={min} AND ({within}) AND ({exact}) THEN 1 ELSE 2 END"
        )
    } else {
        format!("CASE WHEN {low}>0 THEN 1 WHEN {open} OR {uncertain}>0 THEN 2 ELSE 0 END")
    }
}

pub(super) fn enter(
    program: &Program,
    pc: usize,
    node: &Node,
    seed: &str,
) -> Result<String, String> {
    let Instruction::Search {
        axis,
        end,
        field,
        body,
        stop,
        bounds,
        filter: pruning,
    } = &node.op
    else {
        return Err("finite search: expected search".into());
    };
    let first=match (axis,field,end) {
        (Axis::Child,Some(field),_)=>format!("(SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=s.node AND candidate.field={field} AND candidate.ffirst=1 LIMIT 1)"),
        (Axis::Child,None,_)=>"(SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=s.node ORDER BY candidate.id LIMIT 1)".into(),
        (Axis::Parent,Some(field),false)=>format!("CASE WHEN origin.field={field} AND origin.ffirst=1 THEN origin.parent END"),
        (Axis::Parent,_,_)=>"origin.parent".into(),
        (Axis::Previous,_,_)=>"origin.prev".into(),
        (Axis::Next,_,_)=>"origin.next".into(),
    };
    let child = matches!(axis, Axis::Child);
    let stop_first = !child && stop.is_some();
    let kind = if stop_first { 7 } else { 1 };
    let target = if stop_first { stop.unwrap() } else { *body };
    let eligible = if !child && *end {
        field.map_or("1".into(), |f| {
            format!("origin.field={f} AND origin.ffirst=1")
        })
    } else {
        "1".into()
    };
    let guard = if stop_first || bounds.is_some() {
        "1".into()
    } else {
        format!(
            "({eligible}) AND ({})",
            filter(pruning.as_ref(), "candidate")
        )
    };
    let count = bounds.is_some() && !stop_first;
    let work = if count { "MAX(0,s.work-1)" } else { "s.work" };
    let targetpc = format!(
        "CASE WHEN {} THEN 0 WHEN ({guard}) THEN {} ELSE 0 END",
        if count { "s.work=0" } else { "0" },
        target + 10
    );
    let truth = if count {
        "CASE WHEN s.work=0 THEN -2 ELSE 0 END"
    } else {
        "0"
    };
    let env = if stop_first {
        fresh(program, "s.scope", "s.env")
    } else {
        "s.env".into()
    };
    let cells = vec![
        kind.to_string(),
        "s.node".into(),
        "s.level".into(),
        pc.to_string(),
        "s.first".into(),
        "s.bound".into(),
        "s.env".into(),
        "0".into(),
        "0".into(),
        "0".into(),
        "0".into(),
        "s.node".into(),
    ];
    let empty = final_truth(*bounds, "0", "0", "0");
    let empty_env = if let Some((_, _, Some((slot, offset)))) = bounds {
        format!(
            "CASE WHEN ({empty})=1 THEN relation_state_put(s.scope,s.env,{slot},{offset}) ELSE s.env END"
        )
    } else {
        "s.env".into()
    };
    let bound = if *end && matches!(axis, Axis::Child) && field.is_some() {
        "candidate.\"last\""
    } else {
        "origin.\"last\""
    };
    Ok(format!(
        "WITH seed AS MATERIALIZED ({seed}), first AS MATERIALIZED (SELECT s.*,{first} AS first FROM seed s JOIN node origin ON origin.file=d.file AND origin.id=s.node), picked AS MATERIALIZED (SELECT s.*,{bound} AS bound FROM first s JOIN node origin ON origin.file=d.file AND origin.id=s.node LEFT JOIN node candidate ON candidate.file=d.file AND candidate.id=s.first) SELECT s.first,{targetpc},s.level,{work},frame.handle,{truth},{env},s.scope FROM picked s JOIN node origin ON origin.file=d.file AND origin.id=s.node JOIN node candidate ON candidate.file=d.file AND candidate.id=s.first {} WHERE s.first IS NOT NULL UNION ALL SELECT s.node,0,s.level,s.work,s.handle,{empty},{empty_env},s.scope FROM picked s WHERE s.first IS NULL",
        push("s.handle", &cells, "s.first IS NOT NULL")
    ))
}

pub(super) fn finish(
    program: &Program,
    pc: usize,
    node: &Node,
    seed: &str,
) -> Result<String, String> {
    let Instruction::Search {
        axis,
        end,
        field,
        body,
        stop,
        bounds,
        filter: pruning,
    } = &node.op
    else {
        return Err("finite search: expected search".into());
    };
    let child = matches!(axis, Axis::Child);
    let own = if child && stop.is_some() {
        "CASE WHEN p.c0=7 THEN p.c10 ELSE p.truth END"
    } else {
        "p.truth"
    };
    let stopped = if stop.is_some() {
        if child {
            "CASE WHEN p.c0=7 THEN p.truth ELSE 0 END"
        } else {
            "CASE WHEN p.c0=7 THEN p.truth ELSE p.c10 END"
        }
    } else {
        "0"
    };
    let own_done = if child && stop.is_some() {
        "p.c0=7 OR p.own<0 OR p.decisive"
    } else if !child && stop.is_some() {
        "p.c0=1 OR p.truth<0"
    } else {
        "1"
    };
    let low = "(p.c7+(p.own=1))";
    let uncertain = "(p.c8+(p.own=2))";
    let open = "(p.c9 OR p.stopped=2)";
    let decisive = if let Some((min, max, binding)) = bounds {
        max.map_or_else(
            || {
                if binding.is_none() {
                    format!("p.own=1 AND {low}>={min}")
                } else {
                    "0".into()
                }
            },
            |max| format!("{low}>{max}"),
        )
    } else if node.writes.is_empty() {
        "p.own=1".into()
    } else {
        "p.own IN (1,2) OR p.c9".into()
    };
    let next=match (axis,end,field) {
        (Axis::Child,false,None)=>"(SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=p.c1 AND candidate.id>n.\"last\" ORDER BY candidate.id LIMIT 1)".into(),
        (Axis::Child,true,_)=>format!("CASE WHEN {}<=p.c5 THEN {} END",if stop.is_some(){"CASE WHEN p.stopped=0 THEN n.id+1 ELSE n.\"last\"+1 END"}else{"n.id+1"},if stop.is_some(){"CASE WHEN p.stopped=0 THEN n.id+1 ELSE n.\"last\"+1 END"}else{"n.id+1"}),
        (_,false,_)=>"NULL".into(),
        (Axis::Parent,true,_)=>"CASE WHEN p.stopped=0 THEN n.parent END".into(),
        (Axis::Previous,true,_)=>"CASE WHEN p.stopped=0 THEN n.prev END".into(),
        (Axis::Next,true,_)=>"CASE WHEN p.stopped=0 THEN n.next END".into(),
    };
    let done = format!("({own_done}) AND (p.own<0 OR p.stopped<0 OR p.decisive OR p.next IS NULL)");
    let continuing = format!("({own_done}) AND NOT ({done})");
    let mut arms = vec![];
    // Complete one candidate, preserving only a successful search body's captures.
    let verdict = final_truth(*bounds, low, uncertain, open);
    let truth =
        format!("CASE WHEN p.own<0 THEN p.own WHEN p.stopped<0 THEN p.stopped ELSE {verdict} END");
    let mut env = if bounds.is_none() {
        "CASE WHEN p.own=1 THEN p.env ELSE p.c6 END".into()
    } else {
        "p.c6".into()
    };
    if let Some((_, _, Some((slot, offset)))) = bounds {
        env = format!(
            "CASE WHEN ({truth})=1 THEN relation_state_put(p.scope,p.c6,{slot},{low}+({offset})) ELSE p.c6 END"
        );
    }
    arms.push(format!(
        "SELECT p.c1,0,p.c2,p.work,p.parent,{truth},{env},p.scope FROM ready p WHERE {done}"
    ));
    // Re-enter a stop/body phase or begin the next candidate after popping the old frame.
    for transition in 0..3 {
        let (
            guard,
            kind,
            candidate,
            prior,
            own_cell,
            stopped_cell,
            low_cell,
            uncertain_cell,
            open_cell,
            target,
            stop_phase,
        ) = match transition {
            0 if child && stop.is_some() => (
                "p.c0=1 AND p.own>=0 AND NOT p.decisive".to_owned(),
                7,
                "p.c4",
                "p.c11",
                "p.own",
                "0",
                "p.c7",
                "p.c8",
                "p.c9",
                stop.unwrap(),
                true,
            ),
            1 if !child && stop.is_some() => (
                "p.c0=7 AND p.truth>=0".into(),
                1,
                "p.c4",
                "p.c11",
                "0",
                "p.truth",
                "p.c7",
                "p.c8",
                "p.c9",
                *body,
                false,
            ),
            2 => (
                continuing.clone(),
                if !child && stop.is_some() { 7 } else { 1 },
                "p.next",
                "p.c4",
                "0",
                "0",
                low,
                uncertain,
                open,
                if !child && stop.is_some() {
                    stop.unwrap()
                } else {
                    *body
                },
                !child && stop.is_some(),
            ),
            _ => continue,
        };
        let cells = vec![
            kind.to_string(),
            "p.c1".into(),
            "p.c2".into(),
            pc.to_string(),
            candidate.into(),
            "p.c5".into(),
            "p.c6".into(),
            low_cell.into(),
            uncertain_cell.into(),
            open_cell.into(),
            if kind == 7 {
                own_cell.into()
            } else {
                stopped_cell.into()
            },
            prior.into(),
        ];
        let eligible = if !child && *end {
            field.map_or("1".into(), |f| format!("edge.field={f} AND edge.ffirst=1"))
        } else {
            "1".into()
        };
        let body_guard = if stop_phase || bounds.is_some() {
            "1".into()
        } else {
            format!(
                "({eligible}) AND ({})",
                filter(pruning.as_ref(), "candidate")
            )
        };
        let count = bounds.is_some() && !stop_phase;
        let work = if count { "MAX(0,p.work-1)" } else { "p.work" };
        let targetpc = format!(
            "CASE WHEN {} THEN 0 WHEN ({body_guard}) THEN {} ELSE 0 END",
            if count { "p.work=0" } else { "0" },
            target + 10
        );
        let truth = if count {
            "CASE WHEN p.work=0 THEN -2 ELSE 0 END"
        } else {
            "0"
        };
        let env = if stop_phase {
            fresh(program, "p.scope", "p.c6")
        } else {
            "p.c6".into()
        };
        arms.push(format!("SELECT {candidate},{targetpc},p.c2,{work},frame.handle,{truth},{env},p.scope FROM ready p LEFT JOIN node candidate ON candidate.file=d.file AND candidate.id={candidate} LEFT JOIN node edge ON edge.file=d.file AND edge.id={prior} {} WHERE {guard}",push("p.parent",&cells,&guard)));
    }
    Ok(format!(
        "WITH seed AS MATERIALIZED ({seed}), parts AS MATERIALIZED (SELECT p.*,{own} AS own,{stopped} AS stopped FROM seed p), decision AS MATERIALIZED (SELECT p.*,{decisive} AS decisive FROM parts p), ready AS MATERIALIZED (SELECT p.*,{next} AS next FROM decision p JOIN node n ON n.file=d.file AND n.id=p.c4) {}",
        arms.join(" UNION ALL ")
    ))
}
