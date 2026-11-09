//! Delay an independent bound-target lookup until a generated climb reaches its ancestor.
use super::super::super::logic::sql::sql_ast::{SqlBinaryOperator as B, SqlValue as V};
use super::*;

fn field(alias: &str, column: &str) -> E {
    E::Qualified {
        alias: alias.into(),
        column: column.into(),
    }
}
fn expressions(s: &Select) -> impl Iterator<Item = &E> {
    s.columns
        .iter()
        .map(|(e, _)| e)
        .chain(&s.predicates)
        .chain(s.from.iter().flat_map(|i| match &i.join {
            Join::On(on) => on.as_slice(),
            Join::Cross => &[],
        }))
}
fn uses(e: &E, alias: &str, column: Option<&str>) -> bool {
    let mut yes = false;
    e.visit(&mut |e| {
        if let E::Qualified {
            alias: a,
            column: c,
        } = e
        {
            yes |= a == alias && column.is_none_or(|want| want == c)
        }
    });
    yes
}
fn safe(e: &E) -> bool {
    let mut yes = true;
    e.visit(&mut |e| {
        yes &= !matches!(
            e,
            E::Call { .. } | E::Scalar(_) | E::Exists { .. } | E::Parameter(_)
        )
    });
    yes
}

fn attempt(q: &mut Query, near_at: usize) -> Option<()> {
    let suffix = q.ctes[near_at].name.strip_prefix("near")?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let seed_name = format!("seed{suffix}");
    let walk_name = format!("walk{suffix}");
    let seed_at = q.ctes.iter().position(|c| c.name == seed_name)?;
    let walk_at = q.ctes.iter().position(|c| c.name == walk_name)?;
    if !(seed_at < walk_at && walk_at < near_at)
        || q.ctes[seed_at].arms.len() != 1
        || q.ctes[near_at].arms.len() != 1
    {
        return None;
    }
    let seed = &q.ctes[seed_at].arms[0];
    let near = &q.ctes[near_at].arms[0];
    if !seed.distinct || !expressions(seed).all(safe) {
        return None;
    }
    let ancestor_alias = &near
        .from
        .iter()
        .find(|i| i.relation == Rel::Cte(walk_name.clone()))?
        .alias;
    let (predicate_at, carried) = near.predicates.iter().enumerate().find_map(|(at, e)| {
        let E::Binary {
            operator: B::Equal,
            left,
            right,
        } = e
        else {
            return None;
        };
        [
            (left.as_ref(), right.as_ref()),
            (right.as_ref(), left.as_ref()),
        ]
        .into_iter()
        .find_map(|(a, b)| {
            if a != &field(ancestor_alias, "cur") {
                return None;
            }
            let E::Qualified { alias, column } = b else {
                return None;
            };
            (alias == ancestor_alias && column != "cur").then_some((at, column.clone()))
        })
    })?;
    let seed_columns = q.ctes[seed_at].columns.clone().unwrap_or_else(|| {
        seed.columns
            .iter()
            .map(|(_, a)| a.clone().unwrap_or_default())
            .collect()
    });
    let column_at = seed_columns.iter().position(|c| c == &carried)?;
    let E::Qualified {
        alias: target_alias,
        column: target_column,
    } = &seed.columns[column_at].0
    else {
        return None;
    };
    let target_at = seed.from.iter().position(|i| i.alias == *target_alias)?;
    seed.from[target_at].relation.program_relation()?;
    if seed
        .columns
        .iter()
        .enumerate()
        .any(|(at, (e, _))| at != column_at && uses(e, target_alias, None))
    {
        return None;
    }
    let mut target_predicates = Vec::new();
    let mut retained = Vec::new();
    for e in &seed.predicates {
        if uses(e, target_alias, None) {
            let mut local = true;
            e.visit(&mut |e| {
                if let E::Qualified { alias, .. } = e {
                    local &= alias == target_alias
                }
            });
            if !local {
                return None;
            }
            target_predicates.push(e.clone());
        } else {
            retained.push(e.clone());
        }
    }
    for (at, item) in seed.from.iter().enumerate() {
        if let Join::On(on) = &item.join {
            if at == target_at {
                if on.iter().any(|e| {
                    let mut local = safe(e);
                    e.visit(&mut |e| {
                        if let E::Qualified { alias, .. } = e {
                            local &= alias == target_alias
                        }
                    });
                    !local
                }) {
                    return None;
                }
                target_predicates.extend(on.iter().cloned());
            } else if on.iter().any(|e| uses(e, target_alias, None)) {
                return None;
            }
        }
    }
    // The carried target must be passive throughout the climb. Other readers must use near.
    for cte in &q.ctes {
        for arm in &cte.arms {
            if cte.name != walk_name
                && cte.name != q.ctes[near_at].name
                && arm.from.iter().any(|i| {
                    i.relation == Rel::Cte(seed_name.clone())
                        || i.relation == Rel::Cte(walk_name.clone())
                })
            {
                return None;
            }
        }
    }
    if q.body.from.iter().any(|i| {
        i.relation == Rel::Cte(seed_name.clone()) || i.relation == Rel::Cte(walk_name.clone())
    }) {
        return None;
    }
    let walk_columns = q.ctes[walk_at].columns.as_ref()?;
    let walk_column = walk_columns.iter().position(|c| c == &carried)?;
    for arm in &q.ctes[walk_at].arms {
        for item in &arm.from {
            if item.relation == Rel::Cte(seed_name.clone())
                || item.relation == Rel::Cte(walk_name.clone())
            {
                if arm.columns[walk_column].0 != field(&item.alias, &carried) {
                    return None;
                }
                if arm
                    .columns
                    .iter()
                    .enumerate()
                    .any(|(at, (e, _))| at != walk_column && uses(e, &item.alias, Some(&carried)))
                {
                    return None;
                }
                if arm
                    .predicates
                    .iter()
                    .chain(arm.from.iter().flat_map(|i| match &i.join {
                        Join::On(on) => on.as_slice(),
                        Join::Cross => &[],
                    }))
                    .any(|e| uses(e, &item.alias, Some(&carried)))
                {
                    return None;
                }
            }
        }
    }
    if near
        .predicates
        .iter()
        .enumerate()
        .any(|(at, e)| at != predicate_at && uses(e, ancestor_alias, Some(&carried)))
    {
        return None;
    }
    if near
        .from
        .iter()
        .flat_map(|i| match &i.join {
            Join::On(on) => on.as_slice(),
            Join::Cross => &[],
        })
        .any(|e| uses(e, ancestor_alias, Some(&carried)))
    {
        return None;
    }
    let mut target = seed.from[target_at].clone();
    let old_alias = target.alias.clone();
    target.alias = "rr_walk_target".into();
    target.join = Join::Cross;
    for e in &mut target_predicates {
        e.visit_mut(&mut |e| {
            if let E::Qualified { alias, .. } = e {
                if alias == &old_alias {
                    *alias = target.alias.clone();
                }
            }
        });
    }
    target_predicates.push(E::Binary {
        operator: B::Equal,
        left: Box::new(field(&target.alias, target_column)),
        right: Box::new(field(ancestor_alias, "cur")),
    });
    let terminal = E::Exists {
        negated: false,
        query: Box::new(Select {
            columns: vec![(E::Value(V::Integer(1)), None)],
            from: vec![target],
            predicates: target_predicates,
            distinct: false,
        }),
    };
    let ancestor_alias = ancestor_alias.clone();
    let seed = &mut q.ctes[seed_at].arms[0];
    seed.columns[column_at].0 = E::Value(V::Integer(0));
    seed.from.remove(target_at);
    seed.predicates = retained;
    let near = &mut q.ctes[near_at].arms[0];
    near.predicates[predicate_at] = terminal;
    for (e, name) in &mut near.columns {
        if *e == field(&ancestor_alias, &carried) && name.is_none() {
            // The generated near block can inherit its column name from w.f.
            *name = Some(carried.clone());
        }
        e.visit_mut(&mut |e| {
            if *e == field(&ancestor_alias, &carried) {
                *e = field(&ancestor_alias, "cur")
            }
        });
    }
    Some(())
}

pub(super) fn optimize(q: &mut Query) -> bool {
    let mut changed = false;
    for at in 0..q.ctes.len() {
        changed |= attempt(q, at).is_some();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::super::super::super::logic::sql::select::Cte;
    use super::*;
    fn equal(a: E, b: E) -> E {
        E::Binary {
            operator: B::Equal,
            left: Box::new(a),
            right: Box::new(b),
        }
    }
    fn item(table: &str, alias: &str) -> FromItem {
        FromItem::cross(
            Rel::Bound {
                rel: table.into(),
                table: table.into(),
            },
            alias,
        )
    }
    fn projected(alias: &str, fields: &[&str]) -> Vec<(E, Option<String>)> {
        fields
            .iter()
            .map(|c| (field(alias, c), Some(c.to_string())))
            .collect()
    }
    fn query() -> Query {
        let seed = Select {
            distinct: true,
            columns: vec![
                (field("s", "n"), Some("n".into())),
                (field("t", "n"), Some("f".into())),
                (field("s", "v"), Some("v".into())),
            ],
            from: vec![item("seed_input", "s"), item("target", "t")],
            predicates: vec![E::Binary {
                operator: B::GreaterOrEqual,
                left: Box::new(field("t", "k")),
                right: Box::new(E::Value(V::Integer(1))),
            }],
        };
        let mut base_columns = projected("s", &["n", "f", "v"]);
        base_columns.push((field("p", "p"), None));
        let base = Select {
            distinct: false,
            columns: base_columns,
            from: vec![
                FromItem::cross(Rel::Cte("seed0".into()), "s"),
                item("parent", "p"),
            ],
            predicates: vec![equal(field("p", "n"), field("s", "n"))],
        };
        let mut next_columns = projected("w", &["n", "f", "v"]);
        next_columns.push((field("p", "p"), None));
        let next = Select {
            distinct: false,
            columns: next_columns,
            from: vec![
                FromItem::cross(Rel::Cte("walk0".into()), "w"),
                item("parent", "p"),
            ],
            predicates: vec![equal(field("p", "n"), field("w", "cur"))],
        };
        let mut near_columns = projected("w", &["n", "f", "v"]);
        for (_, name) in &mut near_columns {
            *name = None;
        }
        near_columns.push((field("w", "cur"), Some("near_out".into())));
        let near = Select {
            distinct: false,
            columns: near_columns,
            from: vec![FromItem::cross(Rel::Cte("walk0".into()), "w")],
            predicates: vec![equal(field("w", "cur"), field("w", "f"))],
        };
        Query {
            ctes: vec![
                Cte::walk("seed0", None, vec![seed]),
                Cte::walk(
                    "walk0",
                    Some(
                        ["n", "f", "v", "cur"]
                            .into_iter()
                            .map(str::to_string)
                            .collect(),
                    ),
                    vec![base, next],
                ),
                Cte::walk("near0", None, vec![near]),
            ],
            body: Select {
                distinct: true,
                columns: projected("n", &["n", "f", "v"]),
                from: vec![FromItem::cross(Rel::Cte("near0".into()), "n")],
                predicates: vec![],
            },
        }
    }
    #[test]
    fn deferred_target_checks_preserve_duplicates_empty_sets_and_distinct_seed_values() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE seed_input(n,v); INSERT INTO seed_input VALUES(1,7),(2,8),(5,9); CREATE TABLE target(n,k); INSERT INTO target VALUES(2,1),(3,1),(3,2),(6,1); CREATE TABLE parent(n,p); INSERT INTO parent VALUES(1,2),(2,3),(3,4),(5,6)").unwrap();
        let original = query();
        let mut revised = original.clone();
        assert!(optimize(&mut revised));
        assert_eq!(revised.ctes[0].arms[0].from.len(), 1);
        assert!(!optimize(&mut revised));
        let rows = |q: &Query| {
            db.prepare(&format!("SELECT * FROM ({}) ORDER BY 1,2,3", q.render()))
                .unwrap()
                .query_map([], |r| Ok(vec![r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?]))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(
            rows(&original),
            vec![vec![1, 2, 7], vec![1, 3, 7], vec![2, 3, 8], vec![5, 6, 9]]
        );
        assert_eq!(rows(&original), rows(&revised));
        db.execute("DELETE FROM target", []).unwrap();
        assert!(rows(&original).is_empty());
        assert_eq!(rows(&original), rows(&revised));
        let mut correlated = original.clone();
        correlated.ctes[0].arms[0]
            .predicates
            .push(equal(field("t", "n"), field("s", "n")));
        let before = correlated.clone();
        assert!(!optimize(&mut correlated));
        assert_eq!(before, correlated);
        let mut active = original;
        active.ctes[1].arms[1]
            .predicates
            .push(equal(field("w", "f"), E::Value(V::Integer(1))));
        assert!(!optimize(&mut active));
    }
}
