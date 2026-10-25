//! One SQL search schedule interprets immutable source instruction metadata.
//! Source/IR bounds still count every instruction; this shares physical SQL only.
use super::search::{filter, fresh, push};
use crate::code_rule_dsl::sql::relational::outcomes::{
    Axis,
    finite_lower::{Instruction, Program},
};
use std::collections::BTreeMap;

/// These kinds come from a proof of the whole candidate body before CFG
/// flattening. Omitted nodes return false without work, memo, depth, or effects.
fn candidate_filter(kinds: &BTreeMap<usize, Vec<u16>>, pc: &str) -> Option<(String, String)> {
    if kinds.is_empty() {
        return None;
    }
    let sites = kinds
        .keys()
        .map(|site| (site + 10).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cases = kinds
        .iter()
        .map(|(site, kinds)| {
            let predicate = if kinds.is_empty() {
                "0".into()
            } else {
                format!(
                    "candidate.kind IN ({})",
                    kinds
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            };
            format!("WHEN {} THEN ({predicate})", site + 10)
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some((sites, format!("CASE {pc} {cases} ELSE 1 END")))
}

pub(super) fn metadata(program: &Program) -> Option<String> {
    let rows = program
        .code
        .iter()
        .enumerate()
        .filter_map(|(pc, node)| {
            let Instruction::Search {
                axis,
                end,
                field,
                body,
                stop,
                bounds,
                ..
            } = &node.op
            else {
                return None;
            };
            let axis = match axis {
                Axis::Child => 0,
                Axis::Parent => 1,
                Axis::Previous => 2,
                Axis::Next => 3,
            };
            let optional = |v: Option<i64>| v.map_or("NULL".into(), |v| v.to_string());
            let (minimum, maximum, slot, offset) =
                bounds.map_or((None, None, None, 0), |(min, max, binding)| {
                    (
                        Some(min),
                        max,
                        binding.map(|(s, _)| s as i64),
                        binding.map_or(0, |(_, o)| o),
                    )
                });
            Some(format!(
                "({},{axis},{},{},{},{},{},{},{},{offset},{})",
                pc + 10,
                usize::from(*end),
                optional(field.map(i64::from)),
                body + 10,
                optional(stop.map(|pc| (pc + 10) as i64)),
                optional(minimum),
                optional(maximum),
                optional(slot),
                usize::from(!node.writes.is_empty())
            ))
        })
        .collect::<Vec<_>>();
    (!rows.is_empty()).then(||format!("SELECT column1 pc,column2 axis,column3 ending,column4 field,column5 body,column6 stop,column7 minimum,column8 maximum,column9 count_slot,column10 count_offset,column11 strict FROM (VALUES {})",rows.join(",")))
}
fn pruning(program: &Program, pc: &str, alias: &str) -> String {
    let branches = program
        .code
        .iter()
        .enumerate()
        .filter_map(|(id, node)| {
            let Instruction::Search {
                filter: Some(value),
                ..
            } = &node.op
            else {
                return None;
            };
            Some(format!(
                "WHEN {} THEN ({})",
                id + 10,
                filter(Some(value), alias)
            ))
        })
        .collect::<Vec<_>>();
    if branches.is_empty() {
        "1".into()
    } else {
        format!("CASE {pc} {} ELSE 1 END", branches.join(" "))
    }
}
fn verdict(low: &str, uncertain: &str, open: &str, p: &str) -> String {
    let high = format!("({low}+{uncertain})");
    format!(
        "CASE WHEN {p}.minimum IS NULL THEN CASE WHEN {low}>0 THEN 1 WHEN {open} OR {uncertain}>0 THEN 2 ELSE 0 END WHEN (NOT ({open}) AND {high}<{p}.minimum) OR ({p}.maximum IS NOT NULL AND {low}>{p}.maximum) THEN 0 WHEN {low}>={p}.minimum AND ({p}.maximum IS NULL OR (NOT ({open}) AND {high}<={p}.maximum)) AND ({p}.count_slot IS NULL OR (NOT ({open}) AND {low}={high})) THEN 1 ELSE 2 END"
    )
}
fn eligible(p: &str, edge: &str) -> String {
    format!(
        "({p}.axis<>1 OR {p}.ending=0 OR {p}.field IS NULL OR ({edge}.field={p}.field AND {edge}.ffirst=1))"
    )
}

pub(super) fn enter_all(
    program: &Program,
    table: &str,
    seed: &str,
    candidates: &BTreeMap<usize, Vec<u16>>,
) -> Result<String, String> {
    let first = r#"CASE s.axis WHEN 0 THEN CASE WHEN s.field IS NULL THEN (SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=s.node ORDER BY candidate.id LIMIT 1) ELSE (SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=s.node AND candidate.field=s.field AND candidate.ffirst=1 LIMIT 1) END WHEN 1 THEN CASE WHEN s.field IS NULL OR s.ending=1 OR (origin.field=s.field AND origin.ffirst=1) THEN origin.parent END WHEN 2 THEN origin.prev ELSE origin.next END"#;
    let first = candidate_filter(candidates,"s.pc").map_or_else(||first.to_string(),|(sites,filter)|format!(
        "CASE WHEN s.pc IN ({sites}) THEN CASE WHEN s.ending=1 THEN (SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.id>s.node AND candidate.id<=origin.\"last\" AND ({filter}) ORDER BY candidate.id LIMIT 1) ELSE (SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=s.node AND ({filter}) ORDER BY candidate.id LIMIT 1) END ELSE ({first}) END"
    ));
    let body_guard = format!(
        "CASE WHEN s.stop_first OR s.minimum IS NOT NULL THEN 1 ELSE ({}) AND ({}) END",
        eligible("s", "origin"),
        pruning(program, "s.pc", "candidate")
    );
    let target = "CASE WHEN s.stop_first THEN s.stop ELSE s.body END";
    let count = "s.minimum IS NOT NULL AND NOT s.stop_first";
    let empty = verdict("0", "0", "0", "s");
    let empty_env = format!(
        "CASE WHEN s.count_slot IS NOT NULL AND ({empty})=1 THEN relation_state_put(s.scope,s.env,s.count_slot,s.count_offset) ELSE s.env END"
    );
    let child_env = fresh(program, "s.scope", "s.env");
    let cells = vec![
        "CASE WHEN s.stop_first THEN 7 ELSE 1 END".into(),
        "s.node".into(),
        "s.level".into(),
        "s.pc".into(),
        "s.first".into(),
        "s.bound".into(),
        "s.env".into(),
        "0".into(),
        "0".into(),
        "0".into(),
        "0".into(),
        "s.node".into(),
    ];
    Ok(format!(
        "WITH seed AS MATERIALIZED ({seed}), parameters AS MATERIALIZED (SELECT s.*,i.axis,i.ending,i.field,i.body,i.stop,i.minimum,i.maximum,i.count_slot,i.count_offset,i.strict,(i.axis<>0 AND i.stop IS NOT NULL) stop_first FROM seed s JOIN {table} i ON i.pc=s.pc), first AS MATERIALIZED (SELECT s.*,{first} first FROM parameters s JOIN node origin ON origin.file=d.file AND origin.id=s.node), picked AS MATERIALIZED (SELECT s.*,CASE WHEN s.axis=0 AND s.ending=1 AND s.field IS NOT NULL THEN candidate.\"last\" ELSE origin.\"last\" END bound FROM first s JOIN node origin ON origin.file=d.file AND origin.id=s.node LEFT JOIN node candidate ON candidate.file=d.file AND candidate.id=s.first) SELECT COALESCE(s.first,s.node),CASE WHEN s.first IS NULL OR (({count}) AND s.work=0) THEN 0 WHEN ({body_guard}) THEN ({target}) ELSE 0 END,s.level,CASE WHEN s.first IS NOT NULL AND ({count}) THEN MAX(0,s.work-1) ELSE s.work END,COALESCE(frame.handle,s.handle),CASE WHEN s.first IS NULL THEN ({empty}) WHEN ({count}) AND s.work=0 THEN -2 ELSE 0 END,CASE WHEN s.first IS NULL THEN ({empty_env}) WHEN s.stop_first THEN ({child_env}) ELSE s.env END,s.scope FROM picked s JOIN node origin ON origin.file=d.file AND origin.id=s.node LEFT JOIN node candidate ON candidate.file=d.file AND candidate.id=s.first {}",
        push("s.handle", &cells, "s.first IS NOT NULL")
    ))
}

pub(super) fn finish_all(
    program: &Program,
    table: &str,
    seed: &str,
    candidates: &BTreeMap<usize, Vec<u16>>,
) -> Result<String, String> {
    let own = "CASE WHEN p.axis=0 AND p.stop IS NOT NULL AND p.c0=7 THEN p.c10 ELSE p.truth END";
    let stopped = "CASE WHEN p.stop IS NULL THEN 0 WHEN p.axis=0 THEN CASE WHEN p.c0=7 THEN p.truth ELSE 0 END ELSE CASE WHEN p.c0=7 THEN p.truth ELSE p.c10 END END";
    let decisive = "CASE WHEN p.minimum IS NOT NULL THEN CASE WHEN p.maximum IS NOT NULL THEN p.low>p.maximum WHEN p.count_slot IS NULL THEN p.own=1 AND p.low>=p.minimum ELSE 0 END WHEN p.strict THEN p.own IN (1,2) OR p.c9 ELSE p.own=1 END";
    let next = "CASE p.axis WHEN 0 THEN CASE WHEN p.ending=0 THEN CASE WHEN p.field IS NULL THEN (SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=p.c1 AND candidate.id>n.\"last\" ORDER BY candidate.id LIMIT 1) END ELSE CASE WHEN (CASE WHEN p.stopped<>0 THEN n.\"last\"+1 ELSE n.id+1 END)<=p.c5 THEN CASE WHEN p.stopped<>0 THEN n.\"last\"+1 ELSE n.id+1 END END END WHEN 1 THEN CASE WHEN p.ending=1 AND p.stopped=0 THEN n.parent END WHEN 2 THEN CASE WHEN p.ending=1 AND p.stopped=0 THEN n.prev END WHEN 3 THEN CASE WHEN p.ending=1 AND p.stopped=0 THEN n.next END END";
    let next = candidate_filter(candidates,"p.c3").map_or_else(||next.to_string(),|(sites,filter)|format!(
        "CASE WHEN p.c3 IN ({sites}) THEN CASE WHEN p.ending=1 THEN (SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.id>n.id AND candidate.id<=p.c5 AND ({filter}) ORDER BY candidate.id LIMIT 1) ELSE (SELECT candidate.id FROM node candidate WHERE candidate.file=d.file AND candidate.parent=p.c1 AND candidate.id>n.\"last\" AND ({filter}) ORDER BY candidate.id LIMIT 1) END ELSE ({next}) END"
    ));
    let result = verdict("p.low", "p.uncertain", "p.open", "p");
    let done_truth =
        format!("CASE WHEN p.own<0 THEN p.own WHEN p.stopped<0 THEN p.stopped ELSE ({result}) END");
    let body_guard = format!(
        "p.phase_kind=7 OR p.minimum IS NOT NULL OR (({}) AND ({}))",
        eligible("p", "edge"),
        pruning(program, "p.c3", "candidate")
    );
    let fresh_env = fresh(program, "p.scope", "p.c6");
    let cells = vec![
        "p.phase_kind".into(),
        "p.c1".into(),
        "p.c2".into(),
        "p.c3".into(),
        "p.chosen".into(),
        "p.c5".into(),
        "p.c6".into(),
        "CASE WHEN p.route=3 THEN p.low ELSE p.c7 END".into(),
        "CASE WHEN p.route=3 THEN p.uncertain ELSE p.c8 END".into(),
        "CASE WHEN p.route=3 THEN p.open ELSE p.c9 END".into(),
        "CASE WHEN p.route=1 THEN p.own WHEN p.route=2 THEN p.truth ELSE 0 END".into(),
        "p.edge_node".into(),
    ];
    Ok(format!(
        "WITH seed AS MATERIALIZED ({seed}), parameters AS MATERIALIZED (SELECT p.*,i.axis,i.ending,i.field,i.body,i.stop,i.minimum,i.maximum,i.count_slot,i.count_offset,i.strict FROM seed p JOIN {table} i ON i.pc=p.c3), answers AS MATERIALIZED (SELECT p.*,{own} own,{stopped} stopped FROM parameters p), counts AS MATERIALIZED (SELECT p.*,p.c7+(p.own=1) low,p.c8+(p.own=2) uncertain,(p.c9 OR p.stopped=2) open FROM answers p), decision AS MATERIALIZED (SELECT p.*,({decisive}) decisive,CASE WHEN p.stop IS NULL THEN 1 WHEN p.axis=0 THEN p.c0=7 OR p.own<0 OR ({decisive}) ELSE p.c0=1 OR p.truth<0 END own_done FROM counts p), advancing AS MATERIALIZED (SELECT p.*,CASE WHEN p.own_done AND p.own>=0 AND p.stopped>=0 AND NOT p.decisive THEN ({next}) END next FROM decision p JOIN node n ON n.file=d.file AND n.id=p.c4), routing AS MATERIALIZED (SELECT p.*,CASE WHEN p.axis=0 AND p.stop IS NOT NULL AND p.c0=1 AND p.own>=0 AND NOT p.decisive THEN 1 WHEN p.axis<>0 AND p.stop IS NOT NULL AND p.c0=7 AND p.truth>=0 THEN 2 WHEN p.own_done AND (p.own<0 OR p.stopped<0 OR p.decisive OR p.next IS NULL) THEN 0 ELSE 3 END route FROM advancing p), chosen AS MATERIALIZED (SELECT p.*,CASE WHEN p.route=3 THEN p.next ELSE p.c4 END chosen,CASE WHEN p.route=3 THEN p.c4 ELSE p.c11 END edge_node,CASE WHEN p.route=1 OR (p.route=3 AND p.axis<>0 AND p.stop IS NOT NULL) THEN 7 ELSE 1 END phase_kind FROM routing p) SELECT CASE WHEN p.route=0 THEN p.c1 ELSE p.chosen END,CASE WHEN p.route=0 OR (p.phase_kind=1 AND p.minimum IS NOT NULL AND p.work=0) THEN 0 WHEN ({body_guard}) THEN CASE WHEN p.phase_kind=7 THEN p.stop ELSE p.body END ELSE 0 END,p.c2,CASE WHEN p.route<>0 AND p.phase_kind=1 AND p.minimum IS NOT NULL THEN MAX(0,p.work-1) ELSE p.work END,COALESCE(frame.handle,p.parent),CASE WHEN p.route=0 THEN ({done_truth}) WHEN p.phase_kind=1 AND p.minimum IS NOT NULL AND p.work=0 THEN -2 ELSE 0 END,CASE WHEN p.route=0 THEN CASE WHEN p.minimum IS NULL THEN CASE WHEN p.own=1 THEN p.env ELSE p.c6 END WHEN p.count_slot IS NOT NULL AND ({done_truth})=1 THEN relation_state_put(p.scope,p.c6,p.count_slot,p.low+p.count_offset) ELSE p.c6 END WHEN p.phase_kind=7 THEN ({fresh_env}) ELSE p.c6 END,p.scope FROM chosen p LEFT JOIN node candidate ON candidate.file=d.file AND candidate.id=p.chosen LEFT JOIN node edge ON edge.file=d.file AND edge.id=p.edge_node {}",
        push("p.parent", &cells, "p.route<>0")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::sql::relational::outcomes::{
        Slots, effects::Effects, finite_lower::Node,
    };
    fn program() -> Program {
        Program {
            source: None,
            share_search: true,
            code: vec![Node {
                op: Instruction::Search {
                    axis: Axis::Child,
                    end: false,
                    field: Some(17),
                    body: 2,
                    stop: Some(3),
                    bounds: Some((1, Some(4), Some((70, -2)))),
                    filter: None,
                },
                effects: Effects::default(),
                mentions: Slots::new(),
                writes: [70].into_iter().collect(),
                node_writes: Slots::new(),
            }],
            functions: vec![],
            regions: vec![],
            entry: 0,
            mentions: Slots::new(),
            writes: Slots::new(),
            node_writes: Slots::new(),
        }
    }
    #[test]
    fn metadata_preserves_field_count_and_typed_control_targets() {
        let p = program();
        assert!(
            metadata(&p)
                .unwrap()
                .contains("(10,0,0,17,12,13,1,4,70,-2,1)")
        );
        let enter = enter_all(
            &p,
            "instructions",
            "SELECT seed.* FROM seed",
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(enter.contains(
            "candidate.parent=s.node AND candidate.field=s.field AND candidate.ffirst=1"
        ));
        assert!(enter.contains("relation_state_project(s.scope,s.env,0,0)"));
        assert!(!enter.contains("UNION"));
        let finish = finish_all(
            &p,
            "instructions",
            "SELECT popped.* FROM popped",
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(finish.contains("candidate.id>n.\"last\""));
        assert!(
            finish.contains("relation_state_put(p.scope,p.c6,p.count_slot,p.low+p.count_offset)")
        );
        assert!(!finish.contains("UNION"));
    }
}
