//! Fork a complete retained operator by renaming checked relations and sharing its indexed roots.
use super::super::super::logic::sql::select::Relation as Rel;
use super::*;

pub fn execution(
    db: &rusqlite::Connection,
    storage: &Store,
    old: &session::Execution,
    old_prefix: &str,
    new_prefix: &str,
    inputs: &BTreeMap<String, String>,
) -> Result<session::Execution> {
    let mut names = inputs.clone();
    for (name, _) in &old.objects {
        if !storage.contains(name) || !name.contains(old_prefix) {
            return Err(error("temporal fork requires versioned owned relations"));
        }
        let new = name.replacen(old_prefix, new_prefix, 1);
        storage.fork(db, name, &new)?;
        names.insert(name.clone(), new);
    }
    let rename = |name: &str| names.get(name).cloned().unwrap_or_else(|| name.into());
    let mut run = old.clone();
    let e = Arc::make_mut(&mut run.emitted);
    for q in e
        .queries
        .iter_mut()
        .flatten()
        .chain(e.views.iter_mut().map(|(_, q)| q))
    {
        q.body.relations_mut(&mut |r| {
            let Some(name) = r.program_relation().and_then(|r| names.get(r)).cloned() else {
                return;
            };
            match r {
                Rel::Input { rel, .. } | Rel::Derived { rel, .. } => *rel = name,
                Rel::Bound { rel, table } => {
                    *table = quote(&name);
                    *rel = name;
                }
                Rel::Batch { rel, source } => {
                    source.table = source.table.replacen(rel.as_str(), &name, 1);
                    *rel = name;
                }
                _ => {}
            }
        });
        for cte in &mut q.ctes {
            for arm in &mut cte.arms {
                arm.relations_mut(&mut |r| {
                    if let Some(name) = r.program_relation().and_then(|r| names.get(r)).cloned() {
                        *r = Rel::Bound {
                            rel: name.clone(),
                            table: quote(&name),
                        };
                    }
                });
            }
        }
    }
    for (step, q) in e.program.steps.iter_mut().zip(&e.queries) {
        step.output = rename(&step.output);
        step.reads = step.reads.iter().map(|r| rename(r)).collect();
        let q = q
            .as_ref()
            .ok_or_else(|| error("temporal constants require checked queries"))?;
        step.sql = format!("INSERT OR IGNORE INTO new_{} {}", step.output, q.render());
    }
    e.program.relations = e
        .program
        .relations
        .iter()
        .map(|(r, n)| (rename(r), *n))
        .collect();
    e.program.lattices = e
        .program
        .lattices
        .iter()
        .map(|(r, k, t)| (rename(r), *k, *t))
        .collect();
    e.program.lattice_operations = e
        .program
        .lattice_operations
        .iter()
        .map(|(r, o)| (rename(r), o.clone()))
        .collect();
    e.program.column_types = e
        .program
        .column_types
        .iter()
        .map(|(r, t)| (rename(r), t.clone()))
        .collect();
    e.inputs = e.inputs.iter().map(|r| rename(r)).collect();
    run.inputs = run.inputs.iter().map(|r| rename(r)).collect();
    run.outputs = run.outputs.iter().map(|r| rename(r)).collect();
    run.objects = run.objects.iter().map(|(r, t)| (rename(r), *t)).collect();
    run.counts = run.counts.iter().map(|(r, n)| (rename(r), *n)).collect();
    reductions::fork_plan(db, &mut run)?;
    Ok(run)
}
