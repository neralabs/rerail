//! The corpus planner keeps its statistics and native dispatch; SQL components use the wave kernel.
use super::*;
use crate::code_rule_dsl::{
    logic::sql::lattices,
    waves::schedule,
};

fn program_for(run: &Run<'_, '_>, rels: &BTreeSet<String>) -> Result<schedule::Program> {
    let f = run.fields();
    let mut operations = BTreeMap::new();
    let mut lattices_meta = Vec::new();
    for rel in rels {
        if let Some(l) = f.lattice_decl(rel) {
            let lattice = f.text(l.lattice);
            let top = if lattice == "flat" {
                TOP
            } else {
                let mut ops =
                    lattices::registered(&run.db, lattice).map_err(|e| Error::Plan(e.reason))?;
                ops.after = l.widen;
                let top = ops.top;
                operations.insert(rel.clone(), ops);
                top
            };
            lattices_meta.push((rel.clone(), l.keys, top));
        }
    }
    Ok(schedule::Program {
        name: "corpus-sql".into(),
        relations: rels
            .iter()
            .map(|r| (r.clone(), f.width(r)))
            .collect(),
        steps: vec![],
        file_local: false,
        partitions: vec![],
        lattices: lattices_meta,
        lattice_operations: operations,
        column_types: rels
            .iter()
            .map(|r| {
                (
                    r.clone(),
                    f.types(r)
                        .iter()
                        .map(|t| {
                            if *t == "sym" && run.model.inputs.atoms.is_none() {
                                "TEXT".into()
                            } else {
                                "INTEGER".into()
                            }
                        })
                        .collect(),
                )
            })
            .collect(),
    })
}

pub(super) fn merge(run: &Run<'_, '_>, rel: &str, select: &str) -> Result<usize> {
    let mut p = program_for(run, &BTreeSet::from([rel.into()]))?;
    p.steps.push(schedule::Step {
        output: rel.into(),
        stratum: 0,
        reads: vec![],
        sql: format!("INSERT OR IGNORE INTO new_{rel} {select}"),
    });
    schedule::execute_existing(&run.db, &p, 1024).map_err(|e| Error::Plan(e.to_string()))?;
    Ok(run.db.query_row(
        "SELECT coalesce(sum(rows),0) FROM wave_delta WHERE rel=?1",
        [rel],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

pub(super) fn component(
    run: &mut Run<'_, '_>,
    scc: &BTreeSet<String>,
    planned: &[(super::body::RuleKey, Vec<Step>)],
    recursive: bool,
) -> Result<u32> {
    let mut p = program_for(run, scc)?;
    let mut queries = Vec::new();
    let vs = std::sync::Arc::clone(&run.versions);
    let pf = vs.whole(run.version);
    for (rule, order) in planned {
        let b = vs.at(*rule);
        let q = run.model.checked_query(pf, &b, order, None, None)?;
        p.steps.push(schedule::Step {
            output: b.rel(b.head()).to_string(),
            stratum: 0,
            reads: vec![],
            sql: String::new(),
        });
        queries.push(Some(q));
    }
    if recursive {
        run.model.stats.solve(scc.clone());
        for (rule, _) in planned {
            let b = vs.at(*rule);
            for i in 0..b.len() {
                if let Some(a) = b.plain_atom(i)
                    && !a.neg()
                    && scc.contains(b.rel(a))
                {
                    let (order, _, _) = run.model.plan_rule(pf, &b, Some(i))?;
                    let at = order.iter().position(|s| s.at == i).expect("delta atom");
                    let q = run.model.checked_query(pf, &b, &order, Some(at), None)?;
                    p.steps.push(schedule::Step {
                        output: b.rel(b.head()).to_string(),
                        stratum: 0,
                        reads: vec![b.rel(a).to_string()],
                        sql: String::new(),
                    });
                    queries.push(Some(q));
                }
            }
        }
        run.model.stats.solve(BTreeSet::new());
    }
    let f = vs.whole(run.version);
    // In the lattices' order (by name).
    let mut lats: Vec<(&str, &super::super::program_image::LatticeRef)> = f.lattices().iter().map(|l| (f.rel_name(l.rel), l)).collect();
    lats.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let tops = lats
        .into_iter()
        .map(|(r, l)| {
            super::super::native::lattice(f.text(l.lattice)).map(|lat| (r.to_owned(), (l.keys, lat.top())))
        })
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    let mut emitted = emit::Emitted {
        constant_rows: Default::default(),
        value_types: std::sync::Arc::new(f.decls_by_name().into_iter().map(|(r, c)| (r.to_owned(), c.into_iter().map(String::from).collect())).collect()),
        analysis: run.model.analysis_of(run.versions.placed(0))?,
        program: p,
        queries,
        setup: vec![],
        views: vec![],
        inputs: f.inputs().map(str::to_owned).collect(),
    };
    lattices::transfers(&mut emitted, &tops);
    for (step, q) in emitted
        .program
        .steps
        .iter_mut()
        .zip(emitted.queries.iter_mut())
    {
        let q = q.as_mut().unwrap();
        for block in q
            .ctes
            .iter_mut()
            .flat_map(|c| &mut c.arms)
            .chain([&mut q.body])
        {
            block.sqlite_order();
        }
        step.sql = format!("INSERT OR IGNORE INTO new_{} {}", step.output, q.render());
    }
    schedule::execute_existing(&run.db, &emitted.program, 1024)
        .map_err(|e| Error::Plan(e.to_string()))
}
