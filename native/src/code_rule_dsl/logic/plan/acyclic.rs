//! Collapse total acyclic relation stages into checked, shareable CTE graphs.
use super::super::analysis::{Decision, stages::Components};
use super::super::sql::{
    select::{Query, Relation, Select},
    sql_ast::SqlExpression as E,
};
use crate::code_rule_dsl::waves::schedule;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::code_rule_dsl) mod queries;

pub(in crate::code_rule_dsl) fn total(block: &Select) -> bool {
    effect_safe(block, false)
}
pub(in crate::code_rule_dsl) fn fenced(block: &Select) -> bool {
    effect_safe(block, true)
}
fn effect_safe(block: &Select, checked: bool) -> bool {
    let mut safe = true;
    for e in block
        .columns
        .iter()
        .map(|(e, _)| e)
        .chain(&block.predicates)
        .chain(block.from.iter().flat_map(|i| match &i.join {
            super::super::sql::select::Join::On(on) => on.as_slice(),
            _ => &[],
        }))
    {
        e.visit(&mut |e| match e {
            E::Function { .. } | E::Parameter(_) => safe = false,
            E::Call { name, .. } => safe &= super::super::analysis::stages::total_function(name)
                || checked && super::super::analysis::stages::checked_function(name),
            E::Scalar(q) | E::Exists { query: q, .. } => safe &= effect_safe(q, checked),
            _ => {}
        });
    }
    safe
}

pub(in crate::code_rule_dsl) fn reads(step:&schedule::Step,query:Option<&Query>)->BTreeSet<String> {
    query.map_or_else(||step.reads.iter().cloned().collect(),|q|q.reads().iter()
        .filter_map(|r|r.program_relation().map(str::to_owned)).collect())
}

pub(in crate::code_rule_dsl) fn order(steps: &mut [(schedule::Step, Option<Query>)]) {
    let mut edges = BTreeMap::<String, BTreeSet<String>>::new();
    for (s, q) in steps.iter() {
        edges.entry(s.output.clone()).or_default().extend(
            reads(s,q.as_ref()),
        );
    }
    let components = Components::analyze(&edges);
    let rank = components
        .order(&edges)
        .into_iter()
        .enumerate()
        .flat_map(|(rank, members)| members.into_iter().map(move |r| (r, rank)))
        .collect::<BTreeMap<_, _>>();
    // Stable within a recursive component: preserve merge and widening order.
    steps.sort_by_key(|(s, _)| rank[&s.output]);
}
/// Select stages whose incremental positive-support transitions can compose.
pub(in crate::code_rule_dsl) fn analyze(
    steps: &[(schedule::Step, Option<Query>)],
    widths: &BTreeMap<String, usize>,
    lattices: &BTreeSet<String>,
) -> BTreeMap<String, Decision> {
    let mut edges = BTreeMap::<String, BTreeSet<String>>::new();
    let mut rules = BTreeMap::<String, Vec<Option<&Query>>>::new();
    for (s, q) in steps {
        edges.entry(s.output.clone()).or_default().extend(
            reads(s,q.as_ref()).into_iter().filter(|r| widths.contains_key(r)),
        );
        rules.entry(s.output.clone()).or_default().push(q.as_ref());
    }
    let components = Components::analyze(&edges);
    rules.iter().map(|(name, qs)| {
        let opaque=qs.iter().any(Option::is_none);
        let qs=qs.iter().flatten().copied().collect::<Vec<_>>();
        let reason = if opaque { Some("Opaque SQL producer retains its resolved dependency gate and SQL visibility boundary") }
            else if components.recursive.contains(name) { Some("Recursive component retains fixed-point iteration") }
            else if lattices.contains(name) { Some("Lattice merge and widening retain their stage boundary") }
            else if qs.len() != 1 { Some("Multiple producers retain their general support reconciliation boundary") }
            else if qs.iter().any(|q| !q.ctes.is_empty()) { Some("Generated CTE keeps its existing kernel boundary") }
            else if qs.iter().any(|q| !total(&q.body)) { Some("Checked, ordered, or opaque expressions retain their evaluation boundary") }
            else if qs.iter().any(|q| {
                let mut scalar = false;
                for e in q.body.columns.iter().map(|(e,_)| e).chain(&q.body.predicates) {
                    e.visit(&mut |e| scalar |= matches!(e, E::Scalar(_) | E::Exists { .. }));
                }
                scalar
            }) { Some("Scalar and negative dependencies retain their summary cursor boundary") }
            else if qs.iter().any(|q| q.reads().iter().any(|r| matches!(r, Relation::Derived { delta: true, .. }))) { Some("Delta consumers retain their iteration boundary") }
            else { None };
        (name.clone(), Decision::new(reason.is_none(), reason.unwrap_or("Total acyclic positive stages compose retained context and signed-support transitions in one SQL graph")))
    }).collect()
}

mod loops;
pub(in crate::code_rule_dsl) use loops::{version_loop,compact_retained};
