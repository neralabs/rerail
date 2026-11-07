//! Retain generated SQL tree-walk CTEs as positive relations for overdelete/rederive.
//! Seeds and outputs can contain retained scalar reducers; recursive walk arms stay positive.
use super::super::logic::sql::{
    select::{FromItem, Join, Query, Relation as Rel, Select},
    sql_ast::SqlExpression as E,
};
use super::*;
use maintenance::Work;
mod nullable;
use nullable::Layout;

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

fn rewrite(block: &mut Select, names: &BTreeMap<String, (String, Layout)>) {
    rewrite_scoped(block, names, &BTreeMap::new());
}
fn rewrite_scoped(
    block: &mut Select,
    names: &BTreeMap<String, (String, Layout)>,
    outer: &BTreeMap<String, Layout>,
) {
    let mut aliases = outer.clone();
    for item in &block.from {
        aliases.remove(&item.alias);
    }
    aliases.extend(
        block
            .from
            .iter()
            .filter_map(|i| {
                let columns = match &i.relation {
                    Rel::Cte(name) => names.get(name)?.1.clone(),
                    Rel::Input { rel, .. } => Layout::new(
                        select::input_columns(rel)?
                            .iter()
                            .map(|c| c.to_string())
                            .collect(),
                    ),
                    _ => return None,
                };
                Some((i.alias.clone(), columns))
            })
            .collect::<BTreeMap<_, _>>(),
    );
    let mut replace = |e: &mut E| {
        if let E::Scalar(query) | E::Exists { query, .. } = e {
            rewrite_scoped(query, names, &aliases);
        }
        if let E::Qualified { alias, column } = e {
            if let Some(columns) = aliases.get(alias) {
                *e = columns.access(&format!("rr_lift_{alias}"), column);
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
        if aliases.contains_key(&item.alias) {
            item.alias = format!("rr_lift_{}", item.alias);
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
            || q.ctes.iter().any(|c| {
                !["seed", "walk", "near"].iter().any(|p| {
                    c.name
                        .strip_prefix(p)
                        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                }) || c.arms.iter().any(|a| {
                    !plain(a)
                        && a.from
                            .iter()
                            .any(|i| i.relation == Rel::Cte(c.name.clone()))
                })
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
                (
                    format!("__sql_walk_{}_{}_{}", step.output, id, at),
                    Layout::new(columns),
                ),
            );
        }
        nullable::infer(&q, &mut names);
        // Initialize against old facts, in the same savepoint as fact mutation and ownership.
        for cte in &q.ctes {
            let (name, columns) = &names[&cte.name];
            let width = columns.width();
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
                        columns.probe(&q, &cte.name).render()
                    ),
                    [],
                )
                .map_err(error)?;
            run.counts.insert(name.clone(), count as u64);
            e.program.relations.push((name.clone(), width));
            for arm in &cte.arms {
                let mut body = arm.clone();
                rewrite(&mut body, &names);
                columns.encode(&mut body);
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
        order_dependencies(e);
        for (table, cols) in indexes::wanted(e) {
            session::index(w.db, &table, &cols).map_err(error)?;
        }
    }
    Ok(initialized.into_iter().collect())
}

/// Give the lifted seed, recursive walk, and output their dependency components.
/// The original stratum treated the entire CTE as one indivisible SQL statement.
fn order_dependencies(e: &mut plan::emit::Emitted) {
    let names = e
        .program
        .relations
        .iter()
        .map(|(r, _)| r.clone())
        .collect::<Vec<_>>();
    let ids = names
        .iter()
        .enumerate()
        .map(|(i, r)| (r.clone(), i))
        .collect::<BTreeMap<_, _>>();
    let mut deps = vec![BTreeSet::new(); names.len()];
    let mut users = deps.clone();
    for (step, q) in e.program.steps.iter().zip(&e.queries) {
        let output = ids[&step.output];
        if let Some(q) = q {
            for r in q.reads().iter().filter_map(|r| r.program_relation()) {
                if let Some(&input) = ids.get(r) {
                    deps[output].insert(input);
                    users[input].insert(output);
                }
            }
        }
    }
    fn finish(
        at: usize,
        graph: &[BTreeSet<usize>],
        seen: &mut BTreeSet<usize>,
        out: &mut Vec<usize>,
    ) {
        if !seen.insert(at) {
            return;
        }
        for &next in &graph[at] {
            finish(next, graph, seen, out);
        }
        out.push(at);
    }
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    for at in 0..names.len() {
        finish(at, &users, &mut seen, &mut order);
    }
    seen.clear();
    let mut positions = vec![0u32; names.len()];
    let mut position = 0;
    for at in order.into_iter().rev() {
        if seen.contains(&at) {
            continue;
        }
        position += 1;
        let mut component = Vec::new();
        finish(at, &deps, &mut seen, &mut component);
        for member in component {
            positions[member] = position;
        }
    }
    for step in &mut e.program.steps {
        step.stratum = positions[ids[&step.output]];
    }
}
