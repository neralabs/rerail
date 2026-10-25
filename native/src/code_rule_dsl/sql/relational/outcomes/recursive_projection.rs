//! Pure caller regions over closed recursive answers. No memo or effect boundary
//! may enter this IR: ordered utility entries are sufficient for every prefix.
use super::{
    Op, Plan, Slots,
    utility_projection_lower::{Expr, Program},
};

fn append(plan: &Plan, expressions: &mut Vec<Expr>, source: &mut usize) -> Option<usize> {
    if !plan.mentions.is_empty()
        || !plan.writes.is_empty()
        || !plan.node_writes.is_empty()
        || plan.effects.uses_memo
        || plan.effects.reads_value
        || plan.effects.cache_fills
        || plan.effects.sibling_cache
    {
        return None;
    }
    let expr = match &plan.op {
        Op::Predicate(set) if super::utility_projection_lower::set_supported(set) => {
            Expr::Atom(set.clone())
        }
        Op::Shared(body) => return append(body, expressions, source),
        Op::PureRecursive(program) if program.projection.is_some() => {
            *source += program.source_operators;
            Expr::Recursive(program.clone())
        }
        Op::All(items) | Op::Any(items) => {
            let children = items
                .iter()
                .map(|p| append(p, expressions, source))
                .collect::<Option<Vec<_>>>()?;
            if matches!(plan.op, Op::All(_)) {
                Expr::All(children)
            } else {
                Expr::Any(children)
            }
        }
        Op::Not(body) => Expr::Not(append(body, expressions, source)?),
        _ => return None,
    };
    *source += 1;
    if *source > 256 {
        return None;
    }
    let id = expressions.len();
    expressions.push(expr);
    Some(id)
}
fn candidate(plan: &Plan) -> Option<Program> {
    let mut expressions = vec![];
    let mut source = 0;
    let entry = append(plan, &mut expressions, &mut source)?;
    Some(Program {
        expressions,
        functions: vec![],
        entry,
        original: None,
        source_operators: source,
    })
}
pub(super) fn lower(plan: &Plan) -> Option<Program> {
    let p = candidate(plan)?;
    (p.expressions.len() > 1
        && p.expressions
            .iter()
            .any(|e| matches!(e, Expr::Recursive(_))))
    .then_some(p)
}
/// Preserve source order and stop at every observable boundary. Consecutive
/// pure terms can form one region even when the surrounding caller has effects.
pub(super) fn group(items: &[Plan], all: bool) -> Vec<Plan> {
    let mut out = vec![];
    let mut start = 0;
    while start < items.len() {
        let mut end = start;
        while end < items.len() && candidate(&items[end]).is_some() {
            end += 1;
        }
        if end > start + 1 {
            let op = if all {
                Op::All(items[start..end].to_vec())
            } else {
                Op::Any(items[start..end].to_vec())
            };
            let grouped = Plan {
                effects: super::effects::Effects::of(&op),
                op,
                mentions: Slots::new(),
                writes: Slots::new(),
                node_writes: Slots::new(),
                walk_filter: None,
            };
            if lower(&grouped).is_some() {
                out.push(grouped);
            } else {
                out.extend_from_slice(&items[start..end]);
            }
        } else if end == start {
            out.push(items[start].clone());
            end += 1;
        } else {
            out.push(items[start].clone());
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::publication_tests::{check_work_compiler, policy_utils};
    use serde_json::json;
    #[test]
    fn recursive_caller_regions_keep_value_unknown_and_failure_boundaries() {
        let rules = [
            json!({"kind":"arguments","has":{"stopBy":"end","all":[{"not":{"regex":"^never$"}},{"matches":"outer"},{"matches":"v"},{"not":{"regex":"^never$"}},{"matches":"outer"}]}}),
        ];
        let policy = policy_utils(
            &rules,
            json!({
                "outer":{"any":[{"matches":"left"},{"matches":"right"},{"kind":"parenthesized_expression","has":{"stopBy":"neighbor","matches":"outer"}}]},
                "left":{"kind":"number","regex":"^[0123]$"},
                "right":{"kind":"number","regex":"^4$"},
                "v":{"testUnknown":"^2$","value":{"gt":0}}
            }),
        );
        let sources = ["f(0,2,3);g(1);h(4);", "f(((1)),0);g(2,0);h(a);"];
        for work in [0, 1, 2, 3, 5, 8, 32, i64::MAX] {
            for fault in [None, Some("3")] {
                check_work_compiler(&policy, &rules, &sources, fault, work, |g, w| {
                    let compiled = super::super::compile_job_finite(g, w)?;
                    assert!(
                        compiled
                            .relations
                            .to_string()
                            .contains("closed recursive caller"),
                        "composed region must be exercised"
                    );
                    Ok(compiled)
                });
            }
        }
    }
}
