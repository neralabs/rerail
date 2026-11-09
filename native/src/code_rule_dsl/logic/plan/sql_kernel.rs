//! The corpus planner keeps its statistics and native dispatch; SQL components use the wave kernel.
use super::*;
use crate::code_rule_dsl::{
    logic::sql::{lattices, program},
    waves::schedule,
};

pub(super) fn normalize(prog: &Program) -> Result<(Program, BTreeSet<String>)> {
    let mut out = prog.clone();
    let mut sentinels: BTreeSet<String> = prog
        .decls
        .iter()
        .filter(|(r, c)| c.is_empty() || prog.lattice.get(*r).is_some_and(|l| l.keys == 0))
        .map(|(r, _)| r.clone())
        .collect();
    sentinels.extend(prog.nullary.iter().filter(|name|prog.decls.contains_key(*name)).cloned());
    program::physical(&mut out);
    if prog.lattice.values().any(|l| l.lattice() != "flat") {
        let tops = out
            .lattice
            .iter()
            .map(|(r, l)| {
                super::super::native::lattice(l.lattice())
                    .map(|lat| (r.clone(), (l.keys, lat.top())))
            })
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
        program::tops(&mut out, &tops).map_err(|e| Error::Plan(e.reason))?;
    }
    Ok((out, sentinels))
}

fn program_for(run: &Run<'_, '_>, rels: &BTreeSet<String>) -> Result<schedule::Program> {
    let mut operations = BTreeMap::new();
    let mut lattices_meta = Vec::new();
    for rel in rels {
        if let Some(l) = run.prog.lattice.get(rel) {
            let top = if l.lattice() == "flat" {
                TOP
            } else {
                let mut ops =
                    lattices::registered(run.db, l.lattice()).map_err(|e| Error::Plan(e.reason))?;
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
            .map(|r| (r.clone(), run.prog.decls[r].len()))
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
                    run.prog.decls[r]
                        .iter()
                        .map(|(_, t)| {
                            if t == "sym" && run.model.inputs.atoms.is_none() {
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
    schedule::execute_existing(run.db, &p, 1024).map_err(|e| Error::Plan(e.to_string()))?;
    Ok(run.db.query_row(
        "SELECT coalesce(sum(rows),0) FROM wave_delta WHERE rel=?1",
        [rel],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

pub(super) fn component(
    run: &mut Run<'_, '_>,
    scc: &BTreeSet<String>,
    rules: &[&Rule],
    planned: &[(Atom, Vec<Step>)],
    recursive: bool,
) -> Result<u32> {
    let mut p = program_for(run, scc)?;
    let mut queries = Vec::new();
    for (head, order) in planned {
        let q = run.model.checked_query(run.prog, head, order, None, None)?;
        p.steps.push(schedule::Step {
            output: head.rel.clone(),
            stratum: 0,
            reads: vec![],
            sql: String::new(),
        });
        queries.push(Some(q));
    }
    if recursive {
        run.model.stats.solve(scc.clone());
        for rule in rules {
            for (i, l) in rule.body.iter().enumerate() {
                if let Literal::Atom(a) = l
                    && !a.neg
                    && scc.contains(&a.rel)
                {
                    let (order, _, _) =
                        run.model
                            .plan_rule(run.prog, &rule.head, &rule.body, Some(i))?;
                    let at = order.iter().position(|s| s.at == i).expect("delta atom");
                    let q =
                        run.model
                            .checked_query(run.prog, &rule.head, &order, Some(at), None)?;
                    p.steps.push(schedule::Step {
                        output: rule.head.rel.clone(),
                        stratum: 0,
                        reads: vec![a.rel.clone()],
                        sql: String::new(),
                    });
                    queries.push(Some(q));
                }
            }
        }
        run.model.stats.solve(BTreeSet::new());
    }
    let tops = run
        .prog
        .lattice
        .iter()
        .map(|(r, l)| {
            super::super::native::lattice(l.lattice()).map(|lat| (r.clone(), (l.keys, lat.top())))
        })
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    let mut emitted = emit::Emitted {
        value_types: std::sync::Arc::new(run.prog.decls.iter().map(|(r,c)|(r.clone(),c.iter().map(|(_,t)|t.clone()).collect())).collect()),
        analysis: run.model.analysis(run.prog),
        program: p,
        queries,
        setup: vec![],
        views: vec![],
        inputs: run.prog.inputs.clone(),
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
    schedule::execute_existing(run.db, &emitted.program, 1024)
        .map_err(|e| Error::Plan(e.to_string()))
}
