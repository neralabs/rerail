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
    for rule in rules {
        if !rule.initial_evaluation.proven {
            return rule.initial_evaluation.clone();
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
    let waves = schedule::execute_declared_with(db, &program.program, max_waves, |id| {
        let output = &program.program.steps[id].output;
        let Some(state) = run.reductions.as_mut().unwrap().get_mut(output).filter(|s| s.cold_reuse.proven) else {
            return Ok(None);
        };
        if !state.ready {
            let _context = w.profile_scope(|| json!({"stage":"cold retained scalar initialization","relation":output,"logicalAnalysis":state.explain()}));
            state.initialize(w.db, &mut run.objects, None)
                .map_err(|e| crate::content_store::StoreError::Invalid(e.reason))?;
            state.ready = true;
            state.cold_initialized = true;
            initialized.push(output.clone());
        }
        // All producers are already in the retained support. Publish once; later
        // frontier visits still run SQL and preserve the scheduler's accounting.
        let query = if emitted.insert(output.clone()) { state.initial_result() } else {
            format!("SELECT {} WHERE 0", vec!["NULL";state.width].join(","))
        };
        Ok(Some(format!("INSERT OR IGNORE INTO {} {query}", quote(&format!("new_{output}")))))
    }).map_err(error)?;
    Ok((waves, initialized))
}

/// Walk lowering adds rules after cold execution. Keep initialized summaries while
/// planning the newly exposed rules against their final checked queries.
pub(in crate::code_rule_dsl::sql_executor) fn after_walks(
    w: &Work<'_>,
    run: &mut session::Execution,
) -> Result<()> {
    let old = run.reductions.take().unwrap_or_default();
    let mut states = planned(w.db, run, false)?;
    for (output, state) in old {
        if state.ready {
            states.insert(output, state);
        }
    }
    run.reductions = Some(states);
    Ok(())
}
