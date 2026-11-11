//! Inline positive acyclic CTE projections for set reconciliation.
use super::*;
use crate::code_rule_dsl::logic::sql::select::Join;

pub(in crate::code_rule_dsl) fn positive(block: &Select) -> bool {
    let mut positive = total(block);
    expressions(block).for_each(|expr| {
        expr.visit(&mut |e| match e {
            E::Scalar(_) | E::Exists { .. } | E::Column(_) | E::All => positive = false,
            E::Call { name, arguments }
                if *name == "tape_nearest"
                    || matches!(*name, "count" | "sum" | "avg" | "total")
                    || matches!(*name, "min" | "max") && arguments.len() == 1 =>
            {
                positive = false
            }
            _ => {}
        })
    });
    positive
}
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
fn rewrite(block: &mut Select, mut visit: impl FnMut(&mut E)) {
    for expression in block
        .columns
        .iter_mut()
        .map(|(e, _)| e)
        .chain(&mut block.predicates)
        .chain(block.from.iter_mut().flat_map(|f| match &mut f.join {
            Join::Cross => &mut [][..],
            Join::On(on) => on.as_mut_slice(),
        }))
    {
        expression.visit_mut(&mut visit);
    }
}
fn nested_safe(block: &Select) -> bool {
    if block
        .from
        .iter()
        .any(|f| matches!(f.relation, Relation::Cte(_)))
    {
        return false;
    }
    let mut safe = true;
    for expression in expressions(block) {
        expression.visit(&mut |e| match e {
            E::Column(_) => safe = false,
            E::Scalar(q) | E::Exists { query: q, .. } => safe &= nested_safe(q),
            _ => {}
        });
    }
    safe
}
fn outer_safe(block: &Select) -> bool {
    if !total(block) {
        return false;
    }
    let mut outer = block.clone();
    let mut safe = true;
    rewrite(&mut outer, |e| {
        if let E::Scalar(q) | E::Exists { query: q, .. } = e {
            safe &= nested_safe(q);
            *e = E::Value(crate::code_rule_dsl::logic::sql::sql_ast::SqlValue::Integer(1));
        }
    });
    safe && positive(&outer)
}
fn all_aliases(block: &Select, aliases: &mut BTreeSet<String>) {
    aliases.extend(block.from.iter().map(|f| f.alias.clone()));
    for expression in expressions(block) {
        expression.visit(&mut |e| {
            if let E::Scalar(q) | E::Exists { query: q, .. } = e {
                all_aliases(q, aliases);
            }
        });
    }
}
fn substitute(block: &mut Select, alias: &str, columns: &[String], inner: &Select) -> bool {
    let mut valid = true;
    rewrite(block, |expr| match expr {
        E::Qualified {
            alias: input,
            column,
        } if input == alias => {
            if let Some(at) = columns.iter().position(|c| c == column) {
                *expr = inner.columns[at].0.clone();
            } else {
                valid = false;
            }
        }
        E::Scalar(q) | E::Exists { query: q, .. } if !q.from.iter().any(|f| f.alias == alias) => {
            valid &= substitute(q, alias, columns, inner);
        }
        _ => {}
    });
    valid
}
pub(in crate::code_rule_dsl) fn expand(query: &Query) -> Option<Query> {
    if query.ctes.is_empty() {
        return Some(query.clone());
    }
    if !outer_safe(&query.body) {
        return None;
    }
    let mut definitions = BTreeMap::<String, (Vec<String>, Select)>::new();
    let mut next_alias = 0;
    for cte in &query.ctes {
        if cte.arms.len() != 1 || !positive(&cte.arms[0]) {
            return None;
        }
        let mut body = cte.arms[0].clone();
        inline(&mut body, &definitions, &mut next_alias)?;
        let columns = cte.columns.clone().or_else(|| {
            body.columns
                .iter()
                .map(|(expr, name)| {
                    name.clone().or_else(|| {
                        if let E::Qualified { column, .. } = expr {
                            Some(column.clone())
                        } else {
                            None
                        }
                    })
                })
                .collect()
        })?;
        definitions.insert(cte.name.clone(), (columns, body));
    }
    let mut body = query.body.clone();
    inline(&mut body, &definitions, &mut next_alias)?;
    Some(Query {
        ctes: Vec::new(),
        body,
    })
}
fn inline(
    body: &mut Select,
    definitions: &BTreeMap<String, (Vec<String>, Select)>,
    next: &mut usize,
) -> Option<()> {
    loop {
        let Some(at) = body
            .from
            .iter()
            .position(|f| matches!(f.relation, Relation::Cte(_)))
        else {
            return Some(());
        };
        let Relation::Cte(name) = &body.from[at].relation else {
            unreachable!()
        };
        let (columns, definition) = definitions.get(name)?;
        let mut inner = definition.clone();
        let mut aliases = BTreeMap::new();
        let mut occupied = BTreeSet::new();
        all_aliases(body, &mut occupied);
        for item in &inner.from {
            loop {
                let name = format!("__rr_inline_{}", *next);
                *next += 1;
                if occupied.insert(name.clone()) {
                    aliases.insert(item.alias.clone(), name);
                    break;
                }
            }
        }
        rewrite(&mut inner, |expr| {
            if let E::Qualified { alias, .. } = expr {
                if let Some(new) = aliases.get(alias) {
                    *alias = new.clone();
                }
            }
        });
        for item in &mut inner.from {
            item.alias = aliases[&item.alias].clone();
        }
        let alias = body.from[at].alias.clone();
        if !substitute(body, &alias, columns, &inner) {
            return None;
        }
        let replaced = body.from.remove(at);
        if let Join::On(on) = replaced.join {
            body.predicates.extend(on);
        }
        body.predicates.extend(inner.predicates);
        body.from.splice(at..at, inner.from);
        if body.from.len() > 64 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::sql::select::{Cte, FromItem};
    #[test]
    fn cte_projection_preserves_aliases_and_rejects_recursive_or_checked_work() {
        let input = Select {
            distinct: true,
            columns: vec![(
                E::Qualified {
                    alias: "f".into(),
                    column: "value".into(),
                },
                None,
            )],
            from: vec![FromItem::cross(
                Relation::Input {
                    schema: String::new(),
                    rel: "facts".into(),
                },
                "f",
            )],
            predicates: Vec::new(),
        };
        let mut query = Query {
            ctes: vec![Cte {
                name: "keys".into(),
                columns: None,
                arms: vec![input.clone()],
                distinct: false,
                materialized: true,
            }],
            body: Select {
                distinct: true,
                columns: vec![(
                    E::Qualified {
                        alias: "k".into(),
                        column: "value".into(),
                    },
                    Some("value".into()),
                )],
                from: vec![FromItem::cross(Relation::Cte("keys".into()), "k")],
                predicates: Vec::new(),
            },
        };
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE facts(value INTEGER);INSERT INTO facts VALUES(1),(1),(2)")
            .unwrap();
        let rows = |sql: &str| {
            db.prepare(sql)
                .unwrap()
                .query_map([], |row| row.get::<_, i64>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let expanded = expand(&query).unwrap();
        assert!(expanded.ctes.is_empty());
        assert_eq!(rows(&query.render()), rows(&expanded.render()));
        query.ctes[0].arms[0].from[0].relation = Relation::Cte("keys".into());
        assert!(expand(&query).is_none());
        query.ctes[0].arms[0] = input;
        query.ctes[0].arms[0].columns[0].0 = E::Call {
            name: "abs",
            arguments: vec![E::Value(
                crate::code_rule_dsl::logic::sql::sql_ast::SqlValue::Integer(-1),
            )],
        };
        assert!(expand(&query).is_none());
    }
}
