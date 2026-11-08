//! Preserve nullable SQL seed values while retained fact relations remain integer sets.
use super::super::super::logic::sql::sql_ast::{
    SqlBinaryOperator as B, SqlUnaryOperator as U, SqlValue as V,
};
use super::*;

#[derive(Clone, Default)]
pub(super) struct Layout {
    pub columns: Vec<String>,
    flags: BTreeMap<usize, usize>,
}
impl Layout {
    pub fn new(columns: Vec<String>) -> Self {
        Self {
            columns,
            flags: BTreeMap::new(),
        }
    }
    pub fn width(&self) -> usize {
        self.columns.len() + self.flags.len()
    }
    pub fn access(&self, alias: &str, column: &str) -> E {
        let at = self
            .columns
            .iter()
            .position(|c| c == column)
            .expect("checked walk field");
        let value = E::Qualified {
            alias: alias.into(),
            column: format!("c{at}"),
        };
        if let Some(flag) = self.flags.get(&at) {
            E::Case {
                branches: vec![(
                    E::Qualified {
                        alias: alias.into(),
                        column: format!("c{flag}"),
                    },
                    value,
                )],
                fallback: Box::new(E::Value(V::Null)),
            }
        } else {
            value
        }
    }
    pub fn encode(&self, block: &mut Select) {
        let mut flags = Vec::new();
        for (at, (value, _)) in block.columns.iter_mut().enumerate() {
            if self.flags.contains_key(&at) {
                flags.push((
                    E::Unary {
                        operator: U::IsNotNull,
                        input: Box::new(value.clone()),
                    },
                    None,
                ));
                *value = E::Call {
                    name: "coalesce",
                    arguments: vec![value.clone(), E::Value(V::Integer(0))],
                };
            }
        }
        block.columns.extend(flags);
        for (at, (_, name)) in block.columns.iter_mut().enumerate() {
            *name = Some(format!("c{at}"));
        }
    }
    pub fn probe(&self, q: &Query, name: &str) -> Query {
        let mut q = super::probe(q, name);
        q.body.columns = self
            .columns
            .iter()
            .map(|column| {
                (
                    E::Qualified {
                        alias: "rr_walk".into(),
                        column: column.clone(),
                    },
                    None,
                )
            })
            .collect();
        self.encode(&mut q.body);
        q
    }
}

pub(super) fn infer(q: &Query, names: &mut BTreeMap<String, (String, Layout)>) {
    // Recursive arms can thread a nullable seed through several CTEs. Find the finite
    // fixed point over column nullability before allocating physical layouts.
    loop {
        let mut added = Vec::new();
        for cte in &q.ctes {
            for arm in &cte.arms {
                for (at, (e, _)) in arm.columns.iter().enumerate() {
                    if nullable(e, arm, names) && !names[&cte.name].1.flags.contains_key(&at) {
                        added.push((cte.name.clone(), at));
                    }
                }
            }
        }
        if added.is_empty() {
            break;
        }
        for (name, at) in added {
            names.get_mut(&name).unwrap().1.flags.insert(at, 0);
        }
    }
    for (_, layout) in names.values_mut() {
        for (offset, flag) in layout.flags.values_mut().enumerate() {
            *flag = layout.columns.len() + offset;
        }
    }
}
fn nullable(e: &E, block: &Select, names: &BTreeMap<String, (String, Layout)>) -> bool {
    let n = |e| nullable(e, block, names);
    match e {
        E::Value(V::Null) => true,
        E::Value(_) | E::All | E::Exists { .. } => false,
        E::Qualified { alias, column } => block
            .from
            .iter()
            .find(|i| &i.alias == alias)
            .is_some_and(|i| {
                if let Rel::Cte(cte) = &i.relation {
                    let layout = &names[cte].1;
                    layout
                        .columns
                        .iter()
                        .position(|c| c == column)
                        .is_some_and(|at| layout.flags.contains_key(&at))
                } else {
                    false
                }
            }),
        E::Unary {
            operator: U::IsNull | U::IsNotNull,
            ..
        } => false,
        E::Unary { input, .. } => n(input),
        E::Binary {
            operator: B::Divide,
            ..
        } => true,
        E::Binary { left, right, .. } => n(left) || n(right),
        E::Case { branches, fallback } => n(fallback) || branches.iter().any(|(_, value)| n(value)),
        E::Call {
            name: "count" | "rr_join",
            ..
        } => false,
        E::Call {
            name: "coalesce",
            arguments,
        } => arguments.iter().all(n),
        E::Call {
            name: "sym_atom" | "sym_text",
            arguments,
        } => arguments.iter().any(n),
        E::Scalar(query) => {
            let mut aggregate = false;
            for (e, _) in &query.columns {
                e.visit(&mut |e| {
                    if matches!(
                        e,
                        E::Call {
                            name: "count" | "sum" | "min" | "max" | "rr_join",
                            ..
                        }
                    ) {
                        aggregate = true;
                    }
                });
            }
            !aggregate || query.columns.iter().any(|(e, _)| nullable(e, query, names))
        }
        E::In { input, values } => n(input) || values.iter().any(n),
        E::Between { input, low, high } => n(input) || n(low) || n(high),
        _ => true,
    }
}
