//! Give immutable fact steps checked queries, so maintenance never parses their SQL.
use super::super::logic::sql::{
    select::{FromItem, Query, Relation as Rel, Select},
    sql_ast::SqlExpression as E,
};
use super::*;

/// The split-position domain is a shared immutable input, including producer preparation.
pub(super) fn integer_domain(db:&Database)->Result<()> {
    if super::super::facts::vtab::bound_source(db,"rdl_int").is_none() {
        db.bind("rdl_int",Arc::new(live::Live::from_rows(
            vec!["i".into()],(1..=4096).map(|i|vec![i]),
        ))).map_err(error)?;
    }
    Ok(())
}

pub(super) fn uses_integer_domain(query: &Query) -> bool {
    fn block(select: &Select) -> bool {
        let mut found = select
            .from
            .iter()
            .any(|i| matches!(i.relation, Rel::Integers));
        for e in select
            .columns
            .iter()
            .map(|(e, _)| e)
            .chain(&select.predicates)
            .chain(select.from.iter().flat_map(|i| match &i.join {
                super::super::logic::sql::select::Join::On(on) => on.iter(),
                _ => [].iter(),
            }))
        {
            e.visit(&mut |e| {
                if let E::Scalar(q) | E::Exists { query: q, .. } = e {
                    found |= block(&q);
                }
            });
        }
        found
    }
    query
        .ctes
        .iter()
        .flat_map(|c| &c.arms)
        .chain([&query.body])
        .any(block)
}

pub(super) fn prepare(db: &rusqlite::Connection, run: &mut session::Execution, storage: Option<&versions::Store>) -> Result<usize> {
    let e = Arc::make_mut(&mut run.emitted);
    let mut count = 0;
    for (id, (step, query)) in e.program.steps.iter_mut().zip(&mut e.queries).enumerate() {
        // A SQL producer's step has no parsed query either, but it is not immutable: what it
        // reads changes, and maintenance recomputes it (`maintenance`).
        if query.is_some() || run.producers.contains_key(&step.output) {
            continue;
        }
        let width = e
            .program
            .relations
            .iter()
            .find(|(r, _)| r == &step.output)
            .unwrap()
            .1;
        let name = format!("__sql_constants_{}_{}", step.output, id);
        let columns = (0..width).map(|i| format!("c{i}")).collect::<Vec<_>>();
        if let Some((storage, rows)) = storage.zip(e.constant_rows.get(&step.output)) {
            storage.create_readonly(db, &name, versions::Schema::integers(width,width))?;
            storage.restrict_indexes(&name, &[])?;
            storage.load_rows(&name, rows.iter().map(|r|r.iter().copied().map(Some).collect()).collect(), true)?;
        } else {
        db.execute_batch(&format!(
            "CREATE TEMP TABLE {}({},PRIMARY KEY({})) WITHOUT ROWID",
            quote(&name),
            columns
                .iter()
                .map(|c| format!("{c} INTEGER NOT NULL"))
                .collect::<Vec<_>>()
                .join(","),
            columns.join(",")
        ))
        .map_err(error)?;
        // The cold wave loop has emptied new_<output>. Replaying this immutable step
        // captures exactly its rows without mixing in other rules' contributions.
        db.execute_batch(&step.sql).map_err(error)?;
        db.execute_batch(&format!(
            "INSERT INTO {} SELECT * FROM new_{}; DELETE FROM new_{}",
            quote(&name),
            step.output,
            step.output
        ))
        .map_err(error)?;
        }
        let q = Query {
            ctes: vec![],
            body: Select {
                columns: columns
                    .iter()
                    .map(|c| {
                        (
                            E::Qualified {
                                alias: "rr_constant".into(),
                                column: c.clone(),
                            },
                            Some(c.clone()),
                        )
                    })
                    .collect(),
                from: vec![FromItem::cross(
                    Rel::Bound {
                        rel: name.clone(),
                        table: quote(&name),
                    },
                    "rr_constant",
                )],
                ..Default::default()
            },
        };
        step.sql = format!("INSERT OR IGNORE INTO new_{} {}", step.output, q.render());
        *query = Some(q);
        run.objects.push((name, true));
        count += 1;
    }
    Ok(count)
}
