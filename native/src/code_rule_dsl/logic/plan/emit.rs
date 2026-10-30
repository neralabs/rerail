//! The planner's program as a wave program (open item O14): the plans `run` would choose, as the
//! steps of a `waves::schedule::Program`, so the frontier loop runs it instead of `run`'s own
//! stratum loop. Each strongly connected component is a stratum, in `strata`' order:
//! - a rule of a non-recursive component is one step over whole relations;
//! - a recursive component's rules are each one step over whole relations, which reads (and so
//!   waits on) only lower strata, and per positive atom of the component one delta variant,
//!   planned with that atom first, which reads its `delta_` table and runs again whenever it grew;
//! - a relation's constant rows are one step, in a stratum before every component's.
//!
//! Lattice relations fold by key as `merge_lattice` merges them (`Program::lattices`). The views
//! `run` makes of one-atom projections are statements to run on the connection first, as is
//! `rdl_int`; a step that maps a node to its document reads `stat_docrange` of the statistics
//! schema, which the connection attaches as `run_atoms` has it.
//!
//! Plans are made before the program runs, since its steps are rows the frontier loop selects. A
//! derived relation's count is then the one a previous run's boundary recorded (`rows.<relation>`,
//! `schedule::Outcome::rows`), where `run` counts each stratum before planning the next; without
//! one, the planner's estimate. The counts change the plans, never their rows.
//!
//! A wave's own program reads its chunk's batch (`waves::matches`): its inputs are batch inputs,
//! read from their sources and planned from their row estimates, with no statistics (`batch`).

use super::*;
use crate::code_rule_dsl::waves::schedule;

pub(in crate::code_rule_dsl) struct Emitted {
    pub(in crate::code_rule_dsl) program: schedule::Program,
    /// Statements the steps read besides the program's relations: `rdl_int`, then the views.
    pub(in crate::code_rule_dsl) setup: Vec<String>,
}

/// `lowered` as the wave program `name`, planned over the statistics in schema `stats` of `db`,
/// in the product's input mode (`run_atoms`).
/// `counts` are a previous run's `rows.<relation>`; `batch` maps the inputs a wave reads from its
/// chunk's batch to their source and row estimate (`Inputs::batch`).
pub(in crate::code_rule_dsl) fn emit(
    lowered: &lower::Lowered,
    db: &Connection,
    stats: &str,
    name: &str,
    counts: Option<&BTreeMap<String, u64>>,
    batch: &HashMap<String, (String, f64)>,
) -> Result<Emitted> {
    let prog = &lowered.program;
    let inputs = Inputs {
        facts: "main.".to_owned(),
        stats: format!("{stats}."),
        atoms: Some(lowered.ops),
        native: BTreeSet::new(),
        batch: batch.clone(),
    };
    let mut model = make_plan(prog, db, inputs)?;
    let (sccs, _) = strata(prog)?;
    let mut rules_by: IndexMap<&str, Vec<&Rule>> = IndexMap::new();
    for r in &prog.rules {
        rules_by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    let mut setup = vec![RDL_INT.to_owned()];
    let mut views = HashSet::new();
    for (rel, view) in plan_views(prog, &mut model, &rules_by)? {
        setup.push(view);
        views.insert(rel);
    }
    let derived = |rel: &str| !prog.inputs.iter().any(|i| i == rel) && !views.contains(rel);
    for (rel, rows) in counts.into_iter().flatten() {
        if let Some(rel) = rel.strip_prefix("rows.")
            && derived(rel)
            && prog.decls.contains_key(rel)
        {
            model.stats.card.insert(rel.to_owned(), (*rows as f64).max(1.0));
        }
    }
    let relations: Vec<(String, usize)> = prog.decls.iter().filter(|(rel, _)| derived(rel)).map(|(rel, cols)| (rel.clone(), cols.len())).collect();
    // The derived relations a rule body reads, in order.
    let reads = |order: &[Step], keep: &dyn Fn(&str) -> bool| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in order {
            if let Literal::Atom(a) | Literal::Agg { atom: a, .. } = &s.lit
                && derived(&a.rel)
                && keep(&a.rel)
                && !out.contains(&a.rel)
            {
                out.push(a.rel.clone());
            }
        }
        out
    };
    let mut steps = Vec::new();
    let mut step = |output: &str, stratum: usize, reads: Vec<String>, select: &str| {
        steps.push(schedule::Step {
            output: output.to_owned(),
            stratum: stratum as u32,
            reads,
            sql: format!("INSERT OR IGNORE INTO new_{output} {select}"),
        });
    };
    // Constant rows first (stratum 0), each relation's in one statement: a relation of rows alone is
    // in no component.
    for (rel, rows) in prog.facts.iter().filter(|(rel, rows)| derived(rel) && !rows.is_empty()) {
        let rows = rows
            .iter()
            .map(|row| {
                let values = row
                    .iter()
                    .map(|v| match v {
                        Value::Int(i) => i.to_string(),
                        Value::Str(s) => format!("'{}'", s.replace('\'', "''")),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({values})")
            })
            .collect::<Vec<_>>()
            .join(", ");
        step(rel, 0, Vec::new(), &format!("VALUES {rows}"));
    }
    for (at, scc) in sccs.iter().enumerate() {
        let stratum = at + 1;
        if scc.iter().all(|r| views.contains(r)) {
            continue;
        }
        let rules: Vec<&Rule> = scc.iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect();
        let mut planned = Vec::new();
        for rule in &rules {
            let (order, _, _) = model.plan_rule(prog, &rule.head, &rule.body, None)?;
            planned.push((rule, order));
        }
        let own = |l: &Literal| matches!(l, Literal::Atom(a) if !a.neg && scc.contains(&a.rel));
        let recursive = scc.len() > 1 || planned.iter().flat_map(|(_, o)| o).any(|s| own(&s.lit));
        for (rule, order) in &planned {
            let select = model.sql_select(prog, &rule.head, order, None, None)?;
            // A recursive component's rule over whole relations runs once: its own atoms' later
            // rows reach it through the delta variants.
            let reads = reads(order, &|rel| !recursive || !scc.contains(rel));
            step(&rule.head.rel, stratum, reads, &select);
        }
        if recursive {
            model.stats.solving = scc.clone();
            for rule in &rules {
                for (i, l) in rule.body.iter().enumerate() {
                    if !own(l) {
                        continue;
                    }
                    let (order, _, _) = model.plan_rule(prog, &rule.head, &rule.body, Some(i))?;
                    let at = order.iter().position(|s| s.at == i).ok_or_else(|| Error::Plan(format!("{}: no delta atom", rule.head)))?;
                    let select = model.sql_select(prog, &rule.head, &order, Some(at), None)?;
                    let Literal::Atom(a) = l else { unreachable!("an own atom") };
                    step(&rule.head.rel, stratum, vec![a.rel.clone()], &select);
                }
            }
            model.stats.solving.clear();
        }
    }
    let lattices = prog.lattice.iter().filter(|(rel, _)| derived(rel)).map(|(rel, l)| (rel.clone(), l.keys, super::super::native::TOP)).collect();
    Ok(Emitted {
        program: schedule::Program { name: name.to_owned(), relations, steps, file_local: false, partitions: Vec::new(), lattices },
        setup,
    })
}

/// `source` (a program over batch inputs alone) as the wave program `name`: parsed, lowered (its
/// names interned by `intern`) and emitted, with no statistics to read.
pub(in crate::code_rule_dsl) fn emit_batch(
    source: &str,
    name: &str,
    batch: &HashMap<String, (String, f64)>,
    intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>,
) -> Result<Emitted> {
    let prog = crate::code_rule_dsl::logic::front::parse(source)?;
    let lowered = lower::lower(&prog, crate::code_rule_dsl::language::GrammarDialect::Python, intern)?;
    emit(&lowered, &Connection::open_in_memory()?, "main", name, None, batch)
}

/// The emitted program's outputs on `db` after it ran, decoded as `run_atoms` decodes its own.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub(in crate::code_rule_dsl) fn outputs(
    lowered: &lower::Lowered,
    db: &Connection,
) -> Result<(BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<usize>>)> {
    decoded_outputs(&lowered.program, db, Some(lowered), "main.", true)
}
