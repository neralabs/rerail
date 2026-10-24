//! Atomic binding target discovery with SQL-owned ordered child continuations.
use crate::code_rule_dsl::sql::relational::outcomes::{
    Op,
    finite_lower::{Instruction, Node, Program},
};
fn push(parent: &str, cells: &[String], guard: &str) -> String {
    let values = cells
        .iter()
        .enumerate()
        .map(|(i, v)| format!(" AND frame.v{i} IS ({v})"))
        .collect::<String>();
    format!(
        "LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN {guard} THEN 1 END AND frame.parent_input IS {parent}{values}"
    )
}
fn target(outputs: &[usize], slot: usize, alias: &str) -> Result<String, String> {
    let index = outputs[..3]
        .iter()
        .position(|s| *s == slot)
        .ok_or("finite binding: unknown target slot")?;
    Ok(format!("{alias}.{}", ["c5", "c7", "c8"][index]))
}
fn merge(node: &Node, base: &str, values: &str, scope: &str) -> String {
    node.writes.iter().fold(base.into(), |env, slot| {
        format!("relation_state_put({scope},{env},{slot},relation_state_get({values},{slot}))")
    })
}

pub(super) fn enter(
    _program: &Program,
    pc: usize,
    node: &Node,
    seed: &str,
) -> Result<String, String> {
    let Instruction::Binding {
        core,
        children,
        occurrence,
        ..
    } = &node.op
    else {
        return Err("finite binding: expected binding".into());
    };
    if occurrence.is_some() {
        return Err(
            "finite binding: occurrence continuation requires ordered-list transport".into(),
        );
    }
    let Op::BoundLeaf {
        term,
        inputs,
        outputs,
        ..
    } = &core.op
    else {
        return Err("finite binding: expected atomic target facts".into());
    };
    let arguments = inputs
        .iter()
        .enumerate()
        .map(|(i, slot)| format!(" AND r.i{i} IS relation_state_get(s.env,{slot})"))
        .collect::<String>();
    let prefix = format!(
        "WITH seed AS MATERIALIZED ({seed}), resolved AS MATERIALIZED (SELECT s.*,r.truth AS resolved,r.work AS remaining,r.c0 AS t0,r.c1 AS t1,r.c2 AS t2 FROM seed s CROSS JOIN relation_binding_budget r ON r.term={term} AND r.file=d.file AND r.node=s.node AND r.allowance=s.work{arguments})"
    );
    let Some((slot, body)) = children.first() else {
        return Ok(format!(
            "{prefix} SELECT s.node,0,s.level,s.remaining,s.handle,s.resolved,s.env,s.scope FROM resolved s"
        ));
    };
    let index = outputs[..3]
        .iter()
        .position(|s| s == slot)
        .ok_or("finite binding: missing target")?;
    let selected = format!("s.t{index}");
    let reached = format!("s.resolved=1 AND {selected} IS NOT NULL AND s.remaining>0");
    let env = format!("relation_state_put(s.scope,s.env,{slot},{selected})");
    let cells = vec![
        "6".into(),
        "s.node".into(),
        "s.level".into(),
        pc.to_string(),
        "0".into(),
        "s.t0".into(),
        "s.env".into(),
        "s.t1".into(),
        "s.t2".into(),
        "0".into(),
        "0".into(),
        "0".into(),
    ];
    let skipped = format!(
        "CASE WHEN s.resolved<>1 THEN s.resolved WHEN {selected} IS NULL THEN 0 ELSE -2 END"
    );
    Ok(format!(
        "{prefix} SELECT CASE WHEN {reached} THEN {selected} ELSE s.node END,CASE WHEN {reached} THEN {} ELSE 0 END,s.level,CASE WHEN {reached} THEN s.remaining-1 ELSE s.remaining END,COALESCE(frame.handle,s.handle),CASE WHEN {reached} THEN 0 ELSE {skipped} END,CASE WHEN {reached} THEN {env} ELSE s.env END,s.scope FROM resolved s {}",
        body + 10,
        push("s.handle", &cells, &reached)
    ))
}

pub(super) fn finish(
    _program: &Program,
    pc: usize,
    node: &Node,
    seed: &str,
) -> Result<String, String> {
    let Instruction::Binding {
        core,
        children,
        occurrence,
        ..
    } = &node.op
    else {
        return Err("finite binding: expected binding".into());
    };
    if occurrence.is_some() {
        return Err(
            "finite binding: occurrence continuation requires ordered-list transport".into(),
        );
    }
    let Op::BoundLeaf { outputs, .. } = &core.op else {
        return Err("finite binding: expected atomic target facts".into());
    };
    let mut arms = vec![
        "SELECT p.c1,0,p.c2,p.work,p.parent,p.truth,p.c6,p.scope FROM frame p WHERE p.truth<>1"
            .into(),
    ];
    for (ordinal, (slot, _)) in children.iter().enumerate() {
        let restored =
            format!("relation_state_put(p.scope,p.env,{slot},relation_state_get(p.c6,{slot}))");
        let reached = format!("p.truth=1 AND p.c4={ordinal}");
        if let Some((next_slot, body)) = children.get(ordinal + 1) {
            let selected = target(outputs, *next_slot, "p")?;
            let advance = format!("{reached} AND {selected} IS NOT NULL AND p.work>0");
            let env = format!("relation_state_put(p.scope,{restored},{next_slot},{selected})");
            let cells = vec![
                "6".into(),
                "p.c1".into(),
                "p.c2".into(),
                pc.to_string(),
                (ordinal + 1).to_string(),
                "p.c5".into(),
                "p.c6".into(),
                "p.c7".into(),
                "p.c8".into(),
                "0".into(),
                "0".into(),
                "0".into(),
            ];
            arms.push(format!("SELECT {selected},{},p.c2,p.work-1,continuation.handle,0,{env},p.scope FROM frame p {} WHERE {advance}",body+10,push("p.parent",&cells,&advance).replace("frame.","continuation.").replace("relation_continuation frame ","relation_continuation continuation ")));
            arms.push(format!("SELECT p.c1,0,p.c2,p.work,p.parent,CASE WHEN {selected} IS NULL THEN 0 ELSE -2 END,p.c6,p.scope FROM frame p WHERE {reached} AND ({selected} IS NULL OR p.work=0)"));
        } else {
            let env = merge(node, "p.c6", "p.env", "p.scope");
            arms.push(format!(
                "SELECT p.c1,0,p.c2,p.work,p.parent,1,{env},p.scope FROM frame p WHERE {reached}"
            ));
        }
    }
    Ok(format!(
        "WITH frame AS MATERIALIZED ({seed}) {}",
        arms.join(" UNION ALL ")
    ))
}
