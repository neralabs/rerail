//! Adapt checked SQL expressions to logical proofs. This module does not execute SQL.
use super::{
    select::Query,
    sql_ast::{SqlBinaryOperator as B, SqlExpression as E, SqlValue as V},
};
use crate::code_rule_dsl::logic::analysis::{keys, scalars::Expression};
use std::collections::BTreeMap;

fn field(e: &E) -> Option<String> {
    if let E::Qualified { alias, column } = e {
        Some(format!("{alias}.{column}"))
    } else {
        None
    }
}
fn equality(model: &mut keys::Projection, a: &E, b: &E) {
    if let Some(a) = field(a) {
        if let Some(b) = field(b) {
            model.equalities.push((a, b));
        } else if matches!(b, E::Value(V::Integer(_) | V::Text(_))) {
            model.fixed.insert(a);
        }
    }
}
pub(in crate::code_rule_dsl) fn projection(
    q: &Query,
    columns: &[Vec<String>],
    known: &keys::Keys,
) -> keys::Projection {
    let mut model = keys::Projection {
        outputs: q.body.columns.iter().map(|(e, _)| field(e)).collect(),
        opaque: !q.ctes.is_empty(),
        ..Default::default()
    };
    for (item, columns) in q.body.from.iter().zip(columns) {
        let relation = item.relation.program_relation().unwrap_or("opaque");
        model.inputs.push(keys::Input {
            relation: relation.into(),
            columns: columns
                .iter()
                .map(|c| format!("{}.{c}", item.alias))
                .collect(),
            keys: known
                .get(relation)
                .cloned()
                .unwrap_or_else(|| vec![(0..columns.len()).collect()]),
        });
    }
    if columns.len() != q.body.from.len() {
        model.opaque = true;
    }
    let predicates =
        q.body
            .predicates
            .iter()
            .chain(q.body.from.iter().flat_map(|f| match &f.join {
                super::select::Join::On(on) => on.as_slice(),
                _ => &[],
            }));
    for e in predicates {
        if let E::Binary {
            operator: B::Equal,
            left,
            right,
        } = e
        {
            equality(&mut model, left, right);
            equality(&mut model, right, left);
        }
    }
    model
}

/// Column identities are supplied by the adapter; callbacks and subqueries remain opaque.
pub(in crate::code_rule_dsl) fn expression(
    e: &E,
    columns: &BTreeMap<(String, String), usize>,
) -> Expression {
    match e {
        E::Qualified { alias, column } => {
            columns.get(&(alias.clone(), column.clone())).map_or_else(
                || Expression::Opaque(format!("{e:?}")),
                |i| Expression::Column(*i),
            )
        }
        E::Value(V::Integer(n)) => Expression::Integer(*n),
        E::Value(V::Text(s)) => Expression::Text(s.clone()),
        E::Binary {
            operator,
            left,
            right,
        } => Expression::Apply(
            format!("{operator:?}"),
            vec![expression(left, columns), expression(right, columns)],
        ),
        E::Unary { operator, input } => {
            Expression::Apply(format!("{operator:?}"), vec![expression(input, columns)])
        }
        _ => Expression::Opaque(format!("{e:?}")),
    }
}
