use super::*;
use crate::code_rule_dsl::logic::analysis::stages::loops::{self, Analysis, Stage};
use crate::code_rule_dsl::logic::sql::select::Join;

/// Adapt checked SQL stages to the common component monotonicity analysis.
pub(in crate::code_rule_dsl) fn version_loop(
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
