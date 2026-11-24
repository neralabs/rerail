use super::*;
use crate::code_rule_dsl::logic::analysis::stages::loops::{self, Analysis, Stage};
use crate::code_rule_dsl::logic::sql::select::Join;

/// Adapt checked SQL stages to the common component monotonicity analysis.
pub fn version_loop(
    steps: &[(schedule::Step, Option<Query>)],
    members: &BTreeSet<String>,
    lattices: &BTreeSet<String>,
) -> Analysis {
    fn stage(block: &Select, prefix: &str) -> Stage {
        fn name(relation: &Relation, prefix: &str) -> Option<String> {
            match relation {
                Relation::Cte(name) => Some(format!("{prefix}/{name}")),
                _ => relation.program_relation().map(str::to_owned),
            }
        }
        let mut result = Stage {
            total: total(block),
            ..Stage::default()
        };
        result
            .reads
            .extend(block.from.iter().filter_map(|i| name(&i.relation, prefix)));
        for expression in block
            .columns
            .iter()
            .map(|(e, _)| e)
            .chain(&block.predicates)
            .chain(block.from.iter().flat_map(|i| match &i.join {
                Join::On(on) => on.as_slice(),
                _ => &[],
            }))
        {
            expression.visit(&mut |e| match e {
                E::Scalar(body) | E::Exists { query: body, .. } => {
                    let nested = stage(body, prefix);
                    result.scalar_reads.extend(nested.reads.iter().cloned());
                    result.reads.extend(nested.reads);
                }
                E::Call { name, arguments } => {
                    result.aggregate |=
                        matches!(*name, "count" | "sum" | "rr_exact_sum" | "rr_join")
                            || matches!(*name, "min" | "max") && arguments.len() == 1
                }
                _ => {}
            });
        }
        result
    }
    let mut stages = BTreeMap::<String, Stage>::new();
    let mut checked = true;
    for (id, (_, query)) in steps
        .iter()
        .enumerate()
        .filter(|(_, (step, _))| members.contains(&step.output))
    {
        let Some(query) = query else {
            checked = false;
            continue;
        };
        let prefix = format!("rule {id}");
        for cte in &query.ctes {
            let target = stages.entry(format!("{prefix}/{}", cte.name)).or_default();
            for arm in &cte.arms {
                let arm = stage(arm, &prefix);
                target.reads.extend(arm.reads);
                target.scalar_reads.extend(arm.scalar_reads);
                target.total &= arm.total;
                target.aggregate |= arm.aggregate;
            }
        }
        stages.insert(prefix.clone(), stage(&query.body, &prefix));
    }
    loops::analyze(members, members.is_disjoint(lattices), checked, stages)
}

/// Keep one incremental context cache for each full emitted rule. Only the retained
/// context executor uses this schedule; ordinary SQL keeps its explicit delta variants.
pub fn compact_retained(
    steps: &mut Vec<(schedule::Step, Option<Query>)>,
    lattices: &BTreeSet<String>,
) -> Vec<serde_json::Value> {
    use super::super::super::analysis::stages::Components;
    let mut edges = BTreeMap::<String, BTreeSet<String>>::new();
    for (step, query) in steps.iter() {
        edges
            .entry(step.output.clone())
            .or_default()
            .extend(reads(step, query.as_ref()));
    }
    let components = Components::analyze(&edges);
    let groups = components
        .recursive
        .iter()
        .map(|r| components.members[r].clone())
        .collect::<BTreeSet<_>>();
    let delta = |q: &Query| {
        q.reads()
            .iter()
            .any(|r| matches!(r, Relation::Derived { delta: true, .. }))
    };
    let mut removed = BTreeSet::new();
    let mut reports = vec![];
    for members in groups {
        let full = steps
            .iter()
            .enumerate()
            .filter(|(_, (step, query))| {
                members.contains(&step.output) && query.as_ref().is_some_and(|q| !delta(q))
            })
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        let variants = steps
            .iter()
            .enumerate()
            .filter(|(_, (step, query))| {
                members.contains(&step.output) && query.as_ref().is_some_and(delta)
            })
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        let analysis = version_loop(steps, &members, lattices);
        let incremental = !full.is_empty()
            && full.iter().all(|&id| {
                let q = steps[id].1.as_ref().unwrap();
                q.ctes.is_empty()
                    || q.reads()
                        .iter()
                        .filter_map(|r| r.program_relation())
                        .all(|r| !members.contains(r))
            });
        let recursive_full_rules = full.iter().filter(|&&id| {
            !reads(&steps[id].0, steps[id].1.as_ref()).is_disjoint(&members)
        }).count();
        let eligibility = analysis.retained_full_rules(incremental);
        let savings = analysis.feedback_cache_savings(recursive_full_rules, variants.len());
        let decision = if eligibility.proven { savings.clone() } else { eligibility.clone() };
        if decision.proven {
            for &id in &full {
                let recursive = reads(&steps[id].0, steps[id].1.as_ref())
                    .into_iter()
                    .filter(|r| members.contains(r))
                    .collect::<Vec<_>>();
                let step = &mut steps[id].0;
                for relation in recursive {
                    if !step.reads.contains(&relation) {
                        step.reads.push(relation);
                    }
                }
            }
            removed.extend(variants.iter().copied());
        }
        reports.push(serde_json::json!({"component":members,"decision":decision,"eligibility":eligibility,
            "feedback_cache_savings":savings,"recursive_full_rules":recursive_full_rules,"full_rules":full.len(),
            "generated_delta_variants":variants.len(),"removed_delta_variants":if decision.proven {variants.len()}else{0}}));
    }
    let mut id = 0;
    steps.retain(|_| {
        let keep = !removed.contains(&id);
        id += 1;
        keep
    });
    reports
}
