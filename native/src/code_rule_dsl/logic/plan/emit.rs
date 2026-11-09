//! The planner's program as a wave program (open item O14): the plans `run` would choose, as the
//! steps of a `waves::schedule::Program`, so the frontier loop runs it instead of `run`'s own
//! stratum loop. Each strongly connected component is a stratum, in `strata`' order:
//! - a rule of a non-recursive component is one step over whole relations;
//! - a recursive component's full steps wait on lower strata and recursive aggregate inputs.
//!   Each positive own atom also has a delta variant, planned with that atom first,
//!   which reads its `delta_` table and runs again whenever it grew;
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

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Emitted {
    pub(in crate::code_rule_dsl) value_types: std::sync::Arc<BTreeMap<String, Vec<String>>>,
    pub(in crate::code_rule_dsl) analysis: std::sync::Arc<super::super::analysis::ProgramAnalysis>,
    pub(in crate::code_rule_dsl) program: schedule::Program,
    /// Statements the steps read besides the program's relations: `rdl_int`, then the views.
    pub(in crate::code_rule_dsl) setup: Vec<String>,
    /// Each step's checked query, as `program.steps` (`None`: a relation's constant rows).
    pub(in crate::code_rule_dsl) queries: Vec<Option<Query>>,
    /// The views among `setup`, each as its query.
    pub(in crate::code_rule_dsl) views: Vec<(String, Query)>,
    /// The program's inputs, which its statements may read.
    pub(in crate::code_rule_dsl) inputs: Vec<String>,
}

impl Emitted {
    /// Connection-local extension views must not shadow the public fact modules. All references
    /// are rewritten through the checked query tree; SQL literals and user SQL are untouched.
    pub(in crate::code_rule_dsl) fn for_session(&mut self, namespace: &str, bound: &BTreeSet<String>) {
        let names: BTreeMap<String, String> = self.views.iter().filter(|(rel,_)|bound.contains(rel)).map(|(rel, _)| (rel.clone(), format!("__sql_{namespace}_{rel}"))).collect();
        for (old, new) in &names {
            if let Some(types) = self.value_types.get(old).cloned() { std::sync::Arc::make_mut(&mut self.value_types).insert(new.clone(),types); }
            if let Some(keys) = self.analysis.relation_keys.get(old).cloned() { std::sync::Arc::make_mut(&mut self.analysis).relation_keys.insert(new.clone(), keys); }
        }
        let old: HashSet<String> = self.views.iter().map(|(rel, q)| view_sql(rel, q)).collect();
        self.setup.retain(|sql| !old.contains(sql));
        let rewrite = |query: &mut Query| {
            for block in query.ctes.iter_mut().flat_map(|c| &mut c.arms).chain([&mut query.body]) {
                block.relations_mut(&mut |relation| {
                    if let Relation::Derived { rel, .. } = relation {
                        if let Some(private) = names.get(rel) { *rel = private.clone(); }
                    }
                });
                block.sqlite_order();
            }
        };
        for (rel, query) in &mut self.views {
            rewrite(query);
            if let Some(private) = names.get(rel) { *rel = private.clone(); }
            self.setup.push(view_sql(rel, query));
        }
        for (step, query) in self.program.steps.iter_mut().zip(&mut self.queries) {
            if let Some(query) = query {
                rewrite(query);
                step.sql = format!("INSERT OR IGNORE INTO new_{} {}", step.output, query.render());
            }
        }
    }
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
    batch: &HashMap<String, (Source, f64)>,
) -> Result<Emitted> {
    let inputs = Inputs {
        facts: "main.".to_owned(),
        stats: format!("{stats}."),
        atoms: Some(lowered.ops),
        native: BTreeSet::new(),
        batch: batch.clone(),
        given: None,
        kept: BTreeSet::new(),
    };
    emit_inputs(&lowered.program,db,name,counts,inputs)
}

pub(in crate::code_rule_dsl) fn emit_inputs(prog:&Program, db:&Connection, name:&str, counts:Option<&BTreeMap<String,u64>>, inputs:Inputs)->Result<Emitted>{
    let atoms=inputs.atoms.is_some();
    let mut model = make_plan(prog, db, inputs)?;
    let (sccs, _) = strata(prog)?;
    let mut rules_by: IndexMap<&str, Vec<&Rule>> = IndexMap::new();
    for r in &prog.rules {
        rules_by.entry(r.head.rel.as_str()).or_default().push(r);
    }
    let mut setup = vec![RDL_INT.to_owned()];
    let planned_views = plan_views(prog, &mut model, &rules_by)?;
    let mut views = HashSet::new();
    for (rel, view) in &planned_views {
        setup.push(view_sql(rel, view));
        views.insert(rel.clone());
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
    let mut queries = Vec::new();
    let mut step = |output: &str, stratum: usize, reads: Vec<String>, select: &str, query: Option<Query>| {
        steps.push(schedule::Step {
            output: output.to_owned(),
            stratum: stratum as u32,
            reads,
            sql: format!("INSERT OR IGNORE INTO new_{output} {select}"),
        });
        queries.push(query);
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
        step(rel, 0, Vec::new(), &format!("VALUES {rows}"), None);
    }
    for (at, scc) in sccs.iter().enumerate() {
        let stratum = at + 1;
        if scc.iter().all(|r| views.contains(r)) {
            continue;
        }
        let rules: Vec<&Rule> = scc.iter().flat_map(|rel| rules_by.get(rel.as_str()).into_iter().flatten().copied()).collect();
        let profile_key = super::super::profile::on().then(||format!("{name} stratum {stratum}: {}",scc.iter().cloned().collect::<Vec<_>>().join(",")));
        let record = |rule: &Rule, kind: &str, x: Option<super::super::profile::RulePlan>, took: f64| {
            if let (Some(key),Some(x)) = (&profile_key,x) {
                let number = prog.rules.iter().position(|r|std::ptr::eq(r,rule));
                super::super::profile::stratum(key,"SQL emission",||"compile-time search; runtime SQL counters are recorded separately".into());
                super::super::profile::planned(key,kind,number,&x,took,0,None,None);
                if let Some(number)=number { super::super::profile::note(format!("rule #{number:04}"),||rule.to_string()); }
            }
        };
        let mut planned = Vec::new();
        for rule in &rules {
            let started = profile_key.as_ref().map(|_|super::super::profile::now());
            let ((order, _, _), explain) = model.plan_rule_explained(prog, &rule.head, &rule.body, None)?;
            record(rule,"full query",explain,started.map_or(0.0,|t|super::super::profile::now()-t));
            planned.push((rule, order));
        }
        let own = |l: &Literal| matches!(l, Literal::Atom(a) if !a.neg && scc.contains(&a.rel));
        let recursive = scc.len() > 1 || planned.iter().flat_map(|(_, o)| o).any(|s| own(&s.lit));
        for (rule, order) in &planned {
            let query = model.checked_query(prog, &rule.head, order, None, None)?;
            // Positive own atoms have delta variants. An aggregate needs the complete
            // current input, so wake the full query when that recursive input changes.
            let aggregates=order.iter().filter_map(|s|match &s.lit {Literal::Agg{atom,..}=>Some(atom.rel.as_str()),_=>None}).collect::<BTreeSet<_>>();
            let reads = reads(order, &|rel| !recursive || !scc.contains(rel) || aggregates.contains(rel));
            step(&rule.head.rel, stratum, reads, &query.render(), Some(query));
        }
        if recursive {
            model.stats.solving = scc.clone();
            for rule in &rules {
                for (i, l) in rule.body.iter().enumerate() {
                    if !own(l) {
                        continue;
                    }
                    let started = profile_key.as_ref().map(|_|super::super::profile::now());
                    let ((order, _, _), explain) = model.plan_rule_explained(prog, &rule.head, &rule.body, Some(i))?;
                    record(rule,"delta query",explain,started.map_or(0.0,|t|super::super::profile::now()-t));
                    let at = order.iter().position(|s| s.at == i).ok_or_else(|| Error::Plan(format!("{}: no delta atom", rule.head)))?;
                    let query = model.checked_query(prog, &rule.head, &order, Some(at), None)?;
                    let Literal::Atom(a) = l else { unreachable!("an own atom") };
                    step(&rule.head.rel, stratum, vec![a.rel.clone()], &query.render(), Some(query));
                }
            }
            model.stats.solving.clear();
        }
    }
    let lattices = prog.lattice.iter().filter(|(rel, _)| derived(rel)).map(|(rel, l)| (rel.clone(), l.keys, super::super::native::TOP)).collect();
    Ok(Emitted {
        value_types: std::sync::Arc::new(prog.decls.iter().map(|(r,c)|(r.clone(),c.iter().map(|(_,t)|t.clone()).collect())).collect()),
        analysis: std::sync::Arc::clone(&model.analysis),
        program: schedule::Program { name: name.to_owned(), column_types:relations.iter().map(|(rel,_)|(rel.clone(),prog.decls[rel].iter().map(|(_,ty)|if !atoms && ty=="sym" {"TEXT".into()} else {"INTEGER".into()}).collect())).collect(), relations, steps, file_local: false, partitions: Vec::new(), lattices, lattice_operations:BTreeMap::new() },
        setup,
        queries,
        views: planned_views,
        inputs: prog.inputs.clone(),
    })
}

/// `source` (a program over batch inputs alone) as the wave program `name`: parsed, lowered (its
/// names interned by `intern`) and emitted, with no statistics to read.
pub(in crate::code_rule_dsl) fn emit_batch(
    source: &str,
    name: &str,
    batch: &HashMap<String, (Source, f64)>,
    intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>,
) -> Result<Emitted> {
    emit_program(&crate::code_rule_dsl::logic::rr::parse(source)?, name, batch, intern)
}

/// `emit_batch` of a program already parsed (or built, as a wave's demand program is).
pub(in crate::code_rule_dsl) fn emit_program(
    prog: &crate::code_rule_dsl::logic::front::Program,
    name: &str,
    batch: &HashMap<String, (Source, f64)>,
    intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>,
) -> Result<Emitted> {
    let lowered = lower::lower(prog, crate::code_rule_dsl::language::GrammarDialect::Python, intern)?;
    emit(&lowered, &Connection::open_in_memory()?, "main", name, None, batch)
}

/// A non-recursive `emitted` program as one statement deriving `output` (architecture §8: one layer
/// of full SQL, which no fixpoint runs), composed from the steps' checked queries (decision 0031):
/// each relation below it a materialized common table expression, the distinct union of its steps in
/// stratum order, the views among them, then `output`'s union as the statement. Each step's reads of
/// derived relations are rebound to their expressions, and the whole query is checked again: it reads
/// the program's inputs, and each expression only after it is defined. The program it returns has
/// `output` alone, and keeps the setup that is not a view (`rdl_int`). Refused where a step reads a
/// relation of its own stratum or a later one (recursion), reads a walk or constant rows, or the
/// program has lattices.
pub(in crate::code_rule_dsl) fn single(emitted: Emitted, output: &str) -> Result<Emitted> {
    let Emitted { value_types, analysis, program, setup, queries, views, inputs } = emitted;
    let fail = |message: String| Error::Plan(format!("{}: {message}", program.name));
    if !program.lattices.is_empty() {
        return Err(fail("a lattice needs the fixpoint".to_owned()));
    }
    let arity = |rel: &str| program.relations.iter().find(|(name, _)| name == rel).map(|(_, arity)| *arity);
    let positional = |n: usize| (0..n).map(|i| format!("c{i}")).collect::<Vec<_>>();
    // A step's block, its derived reads rebound to their expressions.
    let block = |owner: &str, query: Query| -> Result<Select> {
        if !query.ctes.is_empty() {
            return Err(fail(format!("{owner} reads a walk, which one statement does not fold")));
        }
        let mut body = query.body;
        let mut delta = None;
        body.relations_mut(&mut |relation| {
            if let Relation::Derived { rel, delta: read_delta } = relation {
                if *read_delta {
                    delta = Some(rel.clone());
                }
                *relation = Relation::Cte(rel.clone());
            }
        });
        match delta {
            Some(rel) => Err(fail(format!("{owner} reads the delta of {rel}"))),
            None => Ok(body),
        }
    };
    let view_statements: HashSet<String> = views.iter().map(|(rel, query)| view_sql(rel, query)).collect();
    let kept = setup.into_iter().filter(|statement| !view_statements.contains(statement)).collect();
    let mut ctes = Vec::new();
    for (rel, query) in views {
        let arm = block(&rel, query)?;
        ctes.push(Cte { name: rel, columns: None, arms: vec![arm], distinct: false, materialized: true });
    }
    let mut steps = program.steps.iter().zip(queries).collect::<Vec<_>>();
    steps.sort_by_key(|(step, _)| step.stratum);
    let stratum = |rel: &str| program.steps.iter().find(|step| step.output == rel).map(|step| step.stratum);
    let mut arms = IndexMap::<String, Vec<Select>>::new();
    for (step, query) in steps {
        for read in &step.reads {
            if stratum(read).is_some_and(|s| s >= step.stratum) {
                return Err(fail(format!("{} reads {read} in its own stratum or a later one", step.output)));
            }
        }
        let query = query.ok_or_else(|| fail(format!("{} has constant rows, which one statement does not fold", step.output)))?;
        arms.entry(step.output.clone()).or_default().push(block(&step.output, query)?);
    }
    let n = arity(output).ok_or_else(|| fail(format!("{output} is not a relation")))?;
    if !arms.contains_key(output) {
        return Err(fail(format!("no step derives {output}")));
    }
    for (rel, arms) in arms {
        let columns = positional(arity(&rel).ok_or_else(|| fail(format!("{rel} is not a relation")))?);
        // The output is read once, by the statement; every relation below it is evaluated once.
        let materialized = rel != output;
        ctes.push(Cte { name: rel, columns: Some(columns), arms, distinct: true, materialized });
    }
    // The output's expression last, so the statement is the union of its steps.
    let at = ctes.iter().position(|cte| cte.name == output).expect("the output has steps");
    let last = ctes.remove(at);
    ctes.push(last);
    let body = Select {
        columns: positional(n).into_iter().map(|c| (Expr::Qualified { alias: "o".to_owned(), column: c }, None)).collect(),
        from: vec![FromItem::cross(Relation::Cte(output.to_owned()), "o")],
        ..Select::default()
    };
    let query = Query { ctes, body };
    query
        .check(&|relation| match relation {
            Relation::Input { rel, .. } | Relation::Batch { rel, .. } | Relation::Bound { rel, .. } => inputs.contains(rel),
            Relation::Derived { .. } => false,
            _ => true,
        })
        .map_err(|e| fail(format!("the statement for {output}: {e}")))?;
    let sql = format!("INSERT OR IGNORE INTO new_{output} {}", query.render());
    let program = schedule::Program {
        relations: vec![(output.to_owned(), n)],
        steps: vec![schedule::Step { output: output.to_owned(), stratum: 0, reads: Vec::new(), sql }],
        ..program
    };
    Ok(Emitted { value_types, analysis, program, setup: kept, queries: vec![Some(query)], views: Vec::new(), inputs })
}

/// The emitted program's outputs on `db` after it ran, decoded as `run_atoms` decodes its own.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub(in crate::code_rule_dsl) fn outputs(
    lowered: &lower::Lowered,
    db: &Connection,
) -> Result<(BTreeMap<String, Vec<String>>, BTreeMap<String, Vec<usize>>)> {
    decoded_outputs(&lowered.program, db, Some(lowered), "main.", true, &BTreeSet::new())
}
