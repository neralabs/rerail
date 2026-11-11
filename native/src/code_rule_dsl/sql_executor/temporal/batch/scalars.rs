//! Reconcile correlated scalar groups using keys exposed by the rule head.
use super::*;
use crate::code_rule_dsl::logic::analysis::Decision;
use crate::code_rule_dsl::logic::sql::select::{FromItem, Join, Select};
use crate::code_rule_dsl::logic::sql::sql_ast::{SqlBinaryOperator as B, SqlExpression as E};

fn expressions(block: &Select) -> impl Iterator<Item = &E> {
    block
        .columns
        .iter()
        .map(|(e, _)| e)
        .chain(&block.predicates)
        .chain(block.from.iter().flat_map(|f| match &f.join {
            Join::Cross => &[][..],
            Join::On(on) => on.as_slice(),
        }))
}
fn subqueries(block: &Select) -> Vec<Select> {
    let mut queries = Vec::new();
    for expression in expressions(block) {
        expression.visit(&mut |e| match e {
            E::Scalar(query) | E::Exists { query, .. } => queries.push((**query).clone()),
            _ => {}
        });
    }
    queries
}
fn has_subquery(expression: &E) -> bool {
    let mut found = false;
    expression.visit(&mut |e| found |= matches!(e, E::Scalar(_) | E::Exists { .. }));
    found
}
fn context(block: &Select, width: usize) -> Select {
    let mut block = block.clone();
    block.columns.truncate(width);
    block.predicates.retain(|e| !has_subquery(e));
    for item in &mut block.from {
        if let Join::On(on) = &mut item.join {
            on.retain(|e| !has_subquery(e));
        }
    }
    block.distinct = true;
    block
}
fn joined(outer: &Select, inner: &Select, width: usize, id: usize) -> Option<Select> {
    let mut inner = inner.clone();
    let aliases = inner
        .from
        .iter()
        .map(|f| (f.alias.clone(), format!("__rr_scalar_{id}_{}", f.alias)))
        .collect::<BTreeMap<_, _>>();
    for expression in inner
        .columns
        .iter_mut()
        .map(|(e, _)| e)
        .chain(&mut inner.predicates)
        .chain(inner.from.iter_mut().flat_map(|f| match &mut f.join {
            Join::Cross => &mut [][..],
            Join::On(on) => on.as_mut_slice(),
        }))
    {
        expression.visit_mut(&mut |e| {
            if let E::Qualified { alias, .. } = e {
                if let Some(name) = aliases.get(alias) {
                    *alias = name.clone();
                }
            }
        });
    }
    for item in &mut inner.from {
        item.alias = aliases[&item.alias].clone();
    }
    let mut result = context(outer, width);
    result.from.extend(inner.from);
    result.predicates.extend(inner.predicates);
    if result.from.len() > 8 || !plan::acyclic::queries::positive(&result) {
        return None;
    }
    Some(result)
}
pub(super) fn compile(
    original: &Query,
    current: &Query,
    trace: &Trace,
    db: &rusqlite::Connection,
    positions: &BTreeMap<&str, usize>,
    changes: &mut Vec<(incremental::Input, incremental::VersionKind)>,
    id: usize,
) -> Option<(Decision, Option<(incremental::Rule, String)>)> {
    let queries = subqueries(&original.body);
    if queries.is_empty() {
        return None;
    }
    let reject = |reason: &str| Some((Decision::new(false, reason), None));
    let mut hidden = false;
    for expression in expressions(&original.body) {
        expression.visit(&mut |e| {
            hidden |= matches!(
                e,
                E::Call {
                    name: "tape_nearest",
                    ..
                }
            )
        });
    }
    if hidden {
        return reject("Implicit tape reads retain their complete SQL evaluation");
    }
    if !plan::acyclic::total(&original.body) || queries.len() > 8 {
        return reject("Effectful or oversized scalar groups retain full evaluation");
    }
    let width = original
        .body
        .columns
        .iter()
        .take_while(|(e, _)| matches!(e, E::Qualified { .. }))
        .count();
    if width == 0 || original.body.from.is_empty() {
        return reject(
            "Global scalars retain their full aggregate; correlated groups require exposed head keys",
        );
    }
    let keys = original.body.columns[..width]
        .iter()
        .filter_map(|(e, _)| {
            let E::Qualified { alias, column } = e else {
                return None;
            };
            Some((alias.clone(), column.clone()))
        })
        .collect::<BTreeSet<_>>();
    let outer_aliases = original
        .body
        .from
        .iter()
        .map(|f| f.alias.as_str())
        .collect::<BTreeSet<_>>();
    for query in &queries {
        let locals = query
            .from
            .iter()
            .map(|f| f.alias.as_str())
            .collect::<BTreeSet<_>>();
        let mut valid = true;
        let mut correlated = false;
        for expression in expressions(query) {
            expression.visit(&mut |e| match e {
                E::Scalar(_) | E::Exists { .. } => valid = false,
                E::Call {
                    name: "tape_nearest",
                    ..
                } => valid = false,
                E::Qualified { alias, column } if !locals.contains(alias.as_str()) => {
                    correlated = true;
                    valid &= outer_aliases.contains(alias.as_str())
                        && keys.contains(&(alias.clone(), column.clone()));
                }
                _ => {}
            });
        }
        if !correlated {
            return reject("Uncorrelated scalar inputs invalidate the complete outer context");
        }
        if !valid {
            return reject(
                "Scalar correlations require complete head keys and explicit positive inputs",
            );
        }
    }
    let current_queries = subqueries(&current.body);
    if current_queries.len() != queries.len() {
        return reject("Scalar query layout changed during lowering");
    }
    let outer = context(&original.body, width);
    let outer_current = context(&current.body, width);
    if !plan::acyclic::queries::positive(&outer) {
        return reject("Scalar outer contexts retain their checked evaluation boundary");
    }
    let mut contexts = vec![(outer, outer_current)];
    for (at, (before, after)) in queries.iter().zip(&current_queries).enumerate() {
        let Some(before) = joined(&original.body, before, width, at) else {
            return reject("Scalar input context exceeds the positive SQL expansion bound");
        };
        let Some(after) = joined(&current.body, after, width, at) else {
            return reject("Scalar input context exceeds the positive SQL expansion bound");
        };
        contexts.push((before, after));
    }
    let mut inputs = Vec::new();
    let mut affected = Vec::new();
    for (before, after) in contexts {
        let Some((reads, arms)) = incremental::affected(
            &Query {
                ctes: Vec::new(),
                body: before,
            },
            &Query {
                ctes: Vec::new(),
                body: after,
            },
            trace,
            db,
            positions,
            changes,
        ) else {
            return reject(
                "Scalar input requires a complete immutable delta and positional schema",
            );
        };
        inputs.extend(reads);
        affected.extend(arms);
    }
    let keys_name = "__rr_scalar_keys";
    let mut next = current.clone();
    let key_alias = "__rr_scalar_target";
    for (at, (expr, _)) in current.body.columns[..width].iter().enumerate() {
        next.body.predicates.push(E::Binary {
            operator: B::Equal,
            left: Box::new(expr.clone()),
            right: Box::new(E::Qualified {
                alias: key_alias.into(),
                column: format!("c{at}"),
            }),
        });
    }
    next.body
        .from
        .insert(0, FromItem::cross(Rel::Cte(keys_name.into()), key_alias));
    maintenance::anchor(&mut next, 0);
    next.body.distinct = true;
    let arity = current.body.columns.len();
    let fields = names("", arity);
    let key_fields = names("", width);
    let prior = format!(
        "(SELECT {fields} FROM {PRIOR} WHERE rr_state={} AND rr_slot={id})",
        compile::PARENT
    );
    let old = names("p", arity);
    let equality = same("p", "k", width);
    let present = (0..arity)
        .map(|c| format!("c{c} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sentinel = (0..arity)
        .map(|c| format!("NULL AS c{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "(WITH {keys_name}({key_fields}) AS MATERIALIZED ({}),__rr_scalar_new({fields}) AS MATERIALIZED ({}) SELECT {COLLECT}({},{id},rr_present,{fields}) FROM (SELECT 0 AS rr_present,{sentinel} UNION ALL SELECT 2,{fields} FROM (SELECT {old} FROM {keys_name} k CROSS JOIN {prior} p WHERE {equality} UNION SELECT * FROM __rr_scalar_new WHERE {present}) UNION ALL SELECT 1,{fields} FROM __rr_scalar_new WHERE {present}))",
        affected.join(" UNION "),
        next.render(),
        compile::PARENT
    );
    Some((
        crate::code_rule_dsl::logic::analysis::deltas::reconcile_groups(true, true, true),
        Some((
            incremental::Rule {
                expression: 0,
                inputs,
                scalar: true,
            },
            sql,
        )),
    ))
}
