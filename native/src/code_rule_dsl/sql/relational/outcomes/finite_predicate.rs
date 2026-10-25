//! Total node predicates do not need a region result or a capture snapshot.
//! Guard by reached program counters and return scheduler columns directly.
//! Only total, effect-free plans can use this unmaterialized projection.
//! Use the same bounded condition emitter as the ordinary predicate region.
use super::super::super::{Op, Plan};
use crate::code_rule_dsl::sql::relational::emit::condition;

pub(super) fn test(plan: &Plan) -> Result<Option<String>, String> {
    let Op::Predicate(set) = &plan.op else {
        return Ok(None);
    };
    if !plan.mentions.is_empty()
        || !plan.writes.is_empty()
        || !plan.node_writes.is_empty()
        || plan.effects != Default::default()
    {
        return Ok(None);
    }
    condition(set, "n").map(Some)
}

pub(super) fn query(tests: &[(Vec<String>, String)], routes: Option<&str>) -> (String, String) {
    let pcs = tests
        .iter()
        .flat_map(|(pcs, _)| pcs.iter().cloned())
        .collect::<Vec<_>>();
    let guard = format!("machine.pc IN ({})", pcs.join(","));
    let cases = tests
        .iter()
        .map(|(pcs, test)| {
            format!(
                "WHEN machine.pc IN ({}) THEN CASE WHEN {test} THEN 1 ELSE 0 END",
                pcs.join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let test = format!("CASE {cases} END");
    // The indexed route key already contains the evaluated Boolean. Reuse its
    // low bits instead of materializing another complete scheduler row.
    let (pc, truth, join) = if let Some(routes) = routes {
        (
            "CASE WHEN route.route_key IS NULL THEN json('missing static CFG route') ELSE route.target END",
            "coalesce(route.mapped,route.route_key%4)".to_owned(),
            format!("LEFT JOIN {routes} route ON route.route_key=machine.pc*4+({test})"),
        )
    } else {
        ("0", test, String::new())
    };
    (
        format!(
            "SELECT machine.node,{pc},machine.level,machine.work,machine.handle,{truth},machine.env,machine.scope FROM node n {join} WHERE ({guard}) AND n.file=d.file AND n.id=machine.node"
        ),
        guard,
    )
}
