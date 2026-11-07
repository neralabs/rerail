//! Retain generated SQL tree-walk CTEs as positive relations for overdelete/rederive.
//! Only nonrecursive rule outputs with plain seed/walk/near blocks use this path.
use super::super::logic::sql::{
    select::{FromItem, Join, Query, Relation as Rel, Select},
    sql_ast::SqlExpression as E,
};
use super::*;
use maintenance::Work;

fn plain(block: &Select) -> bool {
    let mut plain = true;
    for e in block
        .columns
        .iter()
        .map(|(e, _)| e)
        .chain(&block.predicates)
        .chain(block.from.iter().flat_map(|i| match &i.join {
            Join::On(on) => on.iter(),
            Join::Cross => [].iter(),
        }))
    {
        e.visit(&mut |e| {
            if matches!(e, E::Scalar(_) | E::Exists { .. } | E::Parameter(_)) {
                plain = false;
            }
        });
    }
    plain
}

fn rewrite(block: &mut Select, names: &BTreeMap<String, (String, Vec<String>)>) {
    let aliases = block
        .from
        .iter()
        .filter_map(|i| {
            let columns = match &i.relation {
                Rel::Cte(name) => names.get(name)?.1.clone(),
                Rel::Input { rel, .. } => select::input_columns(rel)?
                    .iter()
                    .map(|c| c.to_string())
                    .collect(),
                _ => return None,
            };
            Some((i.alias.clone(), columns))
        })
        .collect::<BTreeMap<_, _>>();
    let mut replace = |e: &mut E| {
        if let E::Qualified { alias, column } = e {
            if let Some(columns) = aliases.get(alias) {
                let i = columns
                    .iter()
                    .position(|c| c == column)
                    .expect("checked CTE column");
                *column = format!("c{i}");
            }
        }
    };
    for (e, _) in &mut block.columns {
        e.visit_mut(&mut replace);
    }
    for e in &mut block.predicates {
        e.visit_mut(&mut replace);
    }
    for item in &mut block.from {
        if let Join::On(on) = &mut item.join {
            for e in on {
                e.visit_mut(&mut replace);
            }
        }
        if let Rel::Cte(name) = &item.relation {
            item.relation = Rel::Derived {
                rel: names[name].0.clone(),
                delta: false,
            };
        } else if let Rel::Input { schema, rel } = &item.relation {
            if let Some(columns) = select::input_columns(rel) {
                item.relation = Rel::Batch {
                    rel: rel.clone(),
                    source: select::Source::positional(&format!("{schema}{rel}"), columns),
                };
            }
        }
    }
    for (i, (_, name)) in block.columns.iter_mut().enumerate() {
        *name = Some(format!("c{i}"));
    }
}

fn probe(query: &Query, name: &str) -> Query {
    Query {
        ctes: query.ctes.clone(),
        body: Select {
            columns: vec![(E::All, None)],
            from: vec![FromItem::cross(Rel::Cte(name.into()), "rr_walk")],
            ..Default::default()
        },
    }
}

pub(super) fn prepare(w: &mut Work<'_>, run: &mut session::Execution) -> Result<Vec<String>> {
    if !run
        .emitted
        .queries
        .iter()
        .flatten()
        .any(|q| !q.ctes.is_empty())
    {
        return Ok(vec![]);
    }
    let e = Arc::make_mut(&mut run.emitted);
    let mut added = Vec::new();
    let mut initialized = BTreeSet::new();
    for id in 0..e.program.steps.len() {
        let step = e.program.steps[id].clone();
        let Some(q) = e.queries[id].as_ref() else {
            continue;
        };
        if q.ctes.is_empty()
            || step.stratum == 0
            || e.program.lattices.iter().any(|(r, _, _)| r == &step.output)
            || e.program
                .steps
                .iter()
                .any(|s| s.stratum == step.stratum && s.output != step.output)
            || q.reads()
                .iter()
                .any(|r| r.program_relation() == Some(step.output.as_str()))
            || !plain(&q.body)
            || q.ctes.iter().any(|c| {
                !["seed", "walk", "near"].iter().any(|p| {
                    c.name
                        .strip_prefix(p)
                        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                }) || c.arms.iter().any(|a| !plain(a))
            })
        {
            continue;
        }
        let q = q.clone();
        initialized.insert(step.output.clone());
        let mut names = BTreeMap::new();
        for (at, cte) in q.ctes.iter().enumerate() {
            let query = probe(&q, &cte.name);
            let sql = query.render() + " LIMIT 0";
            let stmt = w.db.prepare(&sql).map_err(error)?;
            let columns = stmt
                .column_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            names.insert(
                cte.name.clone(),
                (format!("__sql_walk_{}_{}_{}", step.output, id, at), columns),
            );
        }
        // Initialize against old facts, in the same savepoint as fact mutation and ownership.
        for cte in &q.ctes {
            let (name, columns) = &names[&cte.name];
            let width = columns.len();
            let cols = (0..width)
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let definition = (0..width)
                .map(|i| format!("c{i} INTEGER NOT NULL"))
                .collect::<Vec<_>>()
                .join(",");
            for table in [name.clone(), format!("new_{name}"), format!("delta_{name}")] {
                w.db.execute_batch(&format!(
                    "CREATE TEMP TABLE {}({definition},PRIMARY KEY({cols})) WITHOUT ROWID",
                    quote(&table)
                ))
                .map_err(error)?;
                run.objects.push((table, true));
            }
            let count =
                w.db.execute(
                    &format!(
                        "INSERT OR IGNORE INTO {} {}",
                        quote(name),
                        probe(&q, &cte.name).render()
                    ),
                    [],
                )
                .map_err(error)?;
            run.counts.insert(name.clone(), count as u64);
            e.program.relations.push((name.clone(), width));
            for arm in &cte.arms {
                let mut body = arm.clone();
                rewrite(&mut body, &names);
                let query = Query { ctes: vec![], body };
                let reads = query
                    .reads()
                    .iter()
                    .filter_map(|r| r.program_relation().map(str::to_owned))
                    .collect();
                added.push((
                    schedule::Step {
                        output: name.clone(),
                        stratum: step.stratum,
                        reads,
                        sql: format!("INSERT OR IGNORE INTO new_{name} {}", query.render()),
                    },
                    Some(query),
                ));
            }
        }
        let mut body = q.body;
        rewrite(&mut body, &names);
        let query = Query { ctes: vec![], body };
        let s = &mut e.program.steps[id];
        s.reads = query
            .reads()
            .iter()
            .filter_map(|r| r.program_relation().map(str::to_owned))
            .collect();
        s.sql = format!("INSERT OR IGNORE INTO new_{} {}", s.output, query.render());
        e.queries[id] = Some(query);
    }
    let changed = !added.is_empty();
    for (s, q) in added {
        e.program.steps.push(s);
        e.queries.push(q);
    }
    if changed {
        for (table, cols) in indexes::wanted(e) {
            session::index(w.db, &table, &cols).map_err(error)?;
        }
    }
    Ok(initialized.into_iter().collect())
}
