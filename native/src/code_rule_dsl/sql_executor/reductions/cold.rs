//! Reuse safe retained summaries at the ordinary scheduler's dependency frontier.
use super::*;
use crate::code_rule_dsl::logic::analysis::Decision;

pub(super) fn eligibility(
    run: &session::Execution,
    output: &str,
    rules: &[Rule],
    lattice: bool,
) -> Decision {
    if lattice {
        return Decision::new(
            false,
            "Lattice folding and widening keep the ordinary cold schedule",
        );
    }
    if !run.producers.is_empty() {
        return Decision::new(
            false,
            "Opaque SQL producers keep the ordinary cold schedule",
        );
    }
    if rules.is_empty() {
        return Decision::new(false, "No retained scalar rules");
    }
    if rules.iter().all(|r| r.reducers.is_empty()) {
        return Decision::new(false, "No scalar summaries to reuse");
    }
    for rule in rules {
        if !rule.evaluation_order.proven {
            return rule.evaluation_order.clone();
        }
    }
    let steps = &run.emitted.program.steps;
    for step in steps.iter().filter(|s| s.output == output) {
        if steps.iter().any(|p| {
            p.stratum == step.stratum
                && step
                    .reads
                    .iter()
                    .any(|r| r == &p.output || r == &format!("delta_{}", p.output))
        }) {
            return Decision::new(
                false,
                "A same-stratum input can change after the initial evaluation",
            );
        }
    }
    Decision::new(
        true,
        "Independent scalar summaries and complete upstream strata produce the initial answer once",
    )
}

pub(in crate::code_rule_dsl::sql_executor) fn execute(
    db: &Database,
    w: &mut Work<'_>,
    run: &mut session::Execution,
    max_waves: u32,
) -> Result<(u32, Vec<String>)> {
    if run.reductions.is_none() {
        run.reductions = Some(planned(w.db, run, false)?);
    }
    let mut initialized = vec![];
    let mut emitted = BTreeSet::new();
    let program = Arc::clone(&run.emitted);
    let mut before = |w: &mut Work<'_>, id: usize| -> Result<Option<String>> {
        let output = &program.program.steps[id].output;
        let Some(state) = run.reductions.as_mut().unwrap().get_mut(output).filter(|s| s.cold_reuse.proven) else {
            return Ok(None);
        };
        if !state.ready {
            let _context = w.profile_scope(|| json!({"stage":"cold retained scalar initialization","relation":output,"logicalAnalysis":state.explain()}));
            state.initialize(w, &mut run.objects, w.versions)?;
            state.ready = true;
            state.cold_initialized = true;
            initialized.push(output.clone());
        }
        Ok(Some(if emitted.insert(output.clone()) { state.initial_result() } else {
            format!("SELECT {} WHERE 0", vec!["NULL";state.width].join(","))
        }))
    };
    let native = w.versions.is_some_and(|store| program.program.relations.iter()
        .all(|(r,_)| store.contains(&format!("new_{r}"))));
    let waves = if native {
        super::super::frontiers::execute(w, &program.program, max_waves, before)?
    } else {
        schedule::execute_declared_with(db, &program.program, max_waves, |id| {
            before(w,id).map(|q|q.map(|q|format!("INSERT OR IGNORE INTO {} {q}",quote(&format!("new_{}",program.program.steps[id].output)))))
                .map_err(|e|crate::content_store::StoreError::Invalid(e.reason))
        }).map_err(error)?
    };
    Ok((waves, initialized))
}

/// Walk lowering adds rules after cold execution. Keep initialized summaries while
/// planning the newly exposed rules against their final checked queries.
pub(in crate::code_rule_dsl::sql_executor) fn after_walks(
    w: &Work<'_>,
    run: &mut session::Execution,
) -> Result<()> {
    let old = run.reductions.take().unwrap_or_default();
    let mut states = plan_local(w.db, run, false)?;
    states.retain(|output, _| !old.states.contains_key(output));
    let mut plans = pool::Plans {
        states,
        storage: vec![],
    };
    pool::combine(&mut plans, &run.emitted.queries);
    // Walk lowering changes only previously unsupported CTE rules. Preserve
    // every existing descriptor, including consumers initialized later: splitting
    // a shared pool would apply the same source delta twice.
    for (output, state) in old.states {
        plans.states.insert(output, state);
    }
    plans.storage.extend(old.storage);
    run.reductions = Some(plans);
    Ok(())
}
