//! Admit SQL consumers by resident capacity, then run broad structural waves.
use super::{Evaluation, Inputs, Job, Preparation, Session, failure, producer, sql_error};
use crate::code_rule_dsl::model::{CompiledCodeRule, Metrics};
use crate::code_rule_dsl::{budget::Context, model::CompiledClause};
use std::collections::VecDeque;
#[cfg(test)]
mod tests;

const SQL_RESIDENT_BYTES: usize = 256 * 1024 * 1024;
#[derive(Default)]
struct Stats {
    partitions: usize,
    peak_sessions: usize,
    peak_sql_bytes: usize,
    skipped_empty: usize,
}
struct Ready<'a> {
    index: usize,
    clause: &'a CompiledClause,
    session: Session<'a>,
    budget: Context,
    demand: producer::Demand,
    sql_bytes: usize,
}
fn error(result: &mut Evaluation, clause: &CompiledClause, code: &str, message: &str) {
    result
        .failures
        .push(failure(&clause.id, code, message, None));
}
fn observe(result: &mut Evaluation, clause: &CompiledClause, budget: &Context) {
    if budget.observed() {
        result.violations.clear();
        result.metrics.witnesses = 0;
        error(
            result,
            clause,
            "budget-exceeded",
            "relational work budget exceeded",
        );
    }
}
fn advance(
    session: &mut Session<'_>,
    context: &mut Context,
    result: &mut Evaluation,
) -> Option<producer::Demand> {
    #[cfg(feature = "code-rule-profile")]
    let _context = crate::code_rule_dsl::profile::trace::Context::enter(
        &session.code_rule.id,
        &session.clause.id,
        (session.syntax_files.len() == 1).then(|| session.syntax_files[0].0.as_str()),
    );
    #[cfg(feature = "code-rule-profile")]
    let _execute =
        crate::code_rule_dsl::profile::trace::Span::start("consumer-execute", &session.clause.id);
    match context.run(|| session.advance()) {
        Ok(Some(demand)) => Some(demand),
        Ok(None) => {
            match context.run(|| session.finish()) {
                Ok(findings) => {
                    result.metrics.witnesses = findings.len();
                    result.violations = findings;
                }
                Err(e) => error(result, session.clause, "relational-failed", &e.reason),
            }
            None
        }
        Err(e) => {
            error(result, session.clause, "relational-failed", &e.reason);
            None
        }
    }
}
pub(super) fn evaluate(jobs: Vec<Job<'_>>) -> Vec<Evaluation> {
    run(jobs, producer::MAX_RESIDENT_BYTES, SQL_RESIDENT_BYTES).0
}
pub(super) fn evaluate_prepared(
    jobs: Vec<Job<'_>>,
    prepare: impl FnMut(
        &mut Inputs,
        &CompiledCodeRule,
        &CompiledClause,
        Preparation,
    ) -> super::Result<()>,
) -> Vec<Evaluation> {
    run_prepared(
        jobs,
        producer::MAX_RESIDENT_BYTES,
        SQL_RESIDENT_BYTES,
        prepare,
    )
    .0
}
fn run(jobs: Vec<Job<'_>>, input_limit: usize, sql_limit: usize) -> (Vec<Evaluation>, Stats) {
    run_prepared(jobs, input_limit, sql_limit, |_, _, _, _| Ok(()))
}
fn run_prepared(
    jobs: Vec<Job<'_>>,
    input_limit: usize,
    sql_limit: usize,
    mut prepare: impl FnMut(
        &mut Inputs,
        &CompiledCodeRule,
        &CompiledClause,
        Preparation,
    ) -> super::Result<()>,
) -> (Vec<Evaluation>, Stats) {
    #[cfg(feature = "code-rule-profile")]
    let mut trace = crate::code_rule_dsl::profile::trace::Span::start(
        "relational-batch",
        "resident-partitions",
    );
    let mut results: Vec<_> = (0..jobs.len()).map(|_| Evaluation::default()).collect();
    // Locality changes admission order, never relation identities or output order.
    let mut jobs: Vec<_> = jobs.into_iter().enumerate().collect();
    jobs.sort_by(|a, b| {
        a.1.inputs
            .snapshot
            .files
            .first()
            .map(|f| &f.path)
            .cmp(&b.1.inputs.snapshot.files.first().map(|f| &f.path))
    });
    let mut jobs: VecDeque<_> = jobs.into();
    let mut deferred = None;
    let _syntax = super::syntax::Pool::install();
    let mut allowance = producer::Allowance::default();
    let corpus = crate::code_rule_dsl::matcher::CorpusSlot::current();
    let scope = corpus.sql_backing().enter();
    let pool = scope.pool();
    let mut stats = Stats::default();
    while deferred.is_some() || !jobs.is_empty() {
        allowance.clear_resident();
        let mut cohort = Vec::new();
        let mut footprint = producer::layout::Footprint::default();
        let mut sql_bytes = 0usize;
        loop {
            pool.trim(sql_limit.saturating_sub(sql_bytes));
            let ready = if let Some(ready) = deferred.take() {
                ready
            } else {
                let Some((index, mut job)) = jobs.pop_front() else {
                    break;
                };
                let mut extraction = Metrics::default();
                let session = job.budget.run(|| {
                    #[cfg(feature = "code-rule-profile")]
                    let _context = crate::code_rule_dsl::profile::trace::Context::enter(
                        &job.code_rule.id,
                        &job.clause.id,
                        (job.inputs.snapshot.files.len() == 1)
                            .then(|| job.inputs.snapshot.files[0].path.as_str()),
                    );
                    #[cfg(feature = "code-rule-profile")]
                    let _prepare = crate::code_rule_dsl::profile::trace::Span::start(
                        "producer-prepare",
                        "admitted-session",
                    );
                    let _corpus = crate::code_rule_dsl::matcher::CorpusSlot::enter();
                    let _tapes = crate::code_rule_dsl::eval::TapeCacheSlot::install();
                    let crate::code_rule_dsl::model::ClauseKind::Relational(plan) =
                        &job.clause.kind
                    else {
                        return Err(super::invalid("expected relational job"));
                    };
                    if plan.require_nonempty.is_some() {
                        super::admission::limits(job.code_rule, &job.inputs.snapshot)?;
                        prepare(
                            &mut job.inputs,
                            job.code_rule,
                            job.clause,
                            Preparation::Gate,
                        )?;
                        if !super::admission::check(
                            job.code_rule,
                            job.clause,
                            &mut job.inputs,
                            &mut extraction,
                        )? {
                            return Ok(None);
                        }
                    }
                    prepare(
                        &mut job.inputs,
                        job.code_rule,
                        job.clause,
                        Preparation::Body,
                    )?;
                    Session::new(
                        job.code_rule,
                        job.clause,
                        job.inputs,
                        &mut extraction,
                        &pool,
                    )
                    .map(Some)
                });
                results[index].metrics.merge(extraction);
                let mut session = match session {
                    Ok(Some(s)) => s,
                    Ok(None) => {
                        stats.skipped_empty += 1;
                        observe(&mut results[index], job.clause, &job.budget);
                        continue;
                    }
                    Err(e) => {
                        error(
                            &mut results[index],
                            job.clause,
                            "source-incomplete",
                            &e.reason,
                        );
                        observe(&mut results[index], job.clause, &job.budget);
                        continue;
                    }
                };
                let Some(demand) = advance(&mut session, &mut job.budget, &mut results[index])
                else {
                    observe(&mut results[index], job.clause, &job.budget);
                    drop(session);
                    pool.trim(sql_limit.saturating_sub(sql_bytes));
                    continue;
                };
                let bytes = match session.db.resident_bytes() {
                    Ok(n) => n,
                    Err(e) => {
                        error(
                            &mut results[index],
                            job.clause,
                            "relational-failed",
                            &sql_error(e).reason,
                        );
                        continue;
                    }
                };
                Ready {
                    index,
                    clause: job.clause,
                    session,
                    budget: job.budget,
                    demand,
                    sql_bytes: bytes,
                }
            };
            // Empty backing is retained only within the existing total SQL admission allowance.
            pool.trim(sql_limit.saturating_sub(sql_bytes.max(ready.sql_bytes)));
            // Include the one candidate held while the admitted partition drains.
            stats.peak_sessions = stats.peak_sessions.max(cohort.len() + 1);
            stats.peak_sql_bytes = stats.peak_sql_bytes.max(sql_bytes.max(ready.sql_bytes));
            if !cohort.is_empty()
                && (sql_bytes.max(ready.sql_bytes) > sql_limit
                    || !footprint.admit(&ready.demand, input_limit))
            {
                deferred = Some(ready);
                break;
            }
            if cohort.is_empty() && !footprint.admit(&ready.demand, input_limit) {
                error(
                    &mut results[ready.index],
                    ready.clause,
                    "relational-failed",
                    "producer capacity exceeded",
                );
                observe(&mut results[ready.index], ready.clause, &ready.budget);
                continue;
            }
            sql_bytes = sql_bytes.max(ready.sql_bytes);
            cohort.push(ready);
            // One consumer may exceed this scheduling target; its existing work and output limits still apply.
            if sql_bytes >= sql_limit {
                break;
            }
        }
        if cohort.is_empty() {
            continue;
        }
        stats.partitions += 1;
        if let Err(error) = pool.statistics_boundary() {
            for ready in cohort {
                self::error(
                    &mut results[ready.index],
                    ready.clause,
                    "relational-failed",
                    &error.reason,
                );
            }
            continue;
        }
        let cohort_ids = cohort
            .iter()
            .map(|r| (r.index, r.clause))
            .collect::<Vec<_>>();
        execute(cohort, &mut results, &mut allowance, input_limit);
        if let Err(maintenance) = pool.statistics_boundary() {
            pool.invalidate();
            for (index, clause) in cohort_ids {
                results[index].violations.clear();
                results[index].metrics.witnesses = 0;
                error(
                    &mut results[index],
                    clause,
                    "relational-failed",
                    &maintenance.reason,
                );
            }
            for (index, job) in jobs.drain(..) {
                error(
                    &mut results[index],
                    job.clause,
                    "relational-failed",
                    &maintenance.reason,
                );
            }
            if let Some(ready) = deferred.take() {
                error(
                    &mut results[ready.index],
                    ready.clause,
                    "relational-failed",
                    &maintenance.reason,
                );
            }
            break;
        }
    }
    #[cfg(feature = "code-rule-profile")]
    trace.detail(|| serde_json::json!({"emptyAdmissions":stats.skipped_empty,"capacityPartitions":stats.partitions,"peakSuspendedSessions":stats.peak_sessions,"peakSuspendedSqlBytes":stats.peak_sql_bytes,"inputCapacityBytes":input_limit,"sqlAdmissionBytes":sql_limit}));
    (results, stats)
}
fn execute(
    cohort: Vec<Ready<'_>>,
    results: &mut [Evaluation],
    allowance: &mut producer::Allowance,
    input_limit: usize,
) {
    let mut sessions = Vec::new();
    let mut contexts = Vec::new();
    let mut stages = Vec::new();
    let mut identities = Vec::new();
    let mut pending = VecDeque::new();
    for (local, ready) in cohort.into_iter().enumerate() {
        identities.push((ready.index, ready.clause));
        contexts.push(ready.budget);
        stages.push(producer::Stages::default());
        sessions.push(Some(ready.session));
        pending.push_back((local, ready.demand));
    }
    while !pending.is_empty() {
        // Reject depth exhaustion before it can consume the wave's resident capacity.
        pending.retain(|(local, _)| match stages[*local].reserve() {
            Ok(()) => true,
            Err(e) => {
                let (index, clause) = identities[*local];
                error(&mut results[index], clause, "relational-failed", &e.reason);
                sessions[*local] = None;
                false
            }
        });
        let mut next = VecDeque::new();
        while !pending.is_empty() {
            let mut footprint = producer::layout::Footprint::default();
            let mut group = Vec::new();
            while let Some((_, demand)) = pending.front() {
                if !footprint.admit(demand, input_limit) {
                    break;
                }
                group.push(pending.pop_front().unwrap());
            }
            if group.is_empty() {
                let Some((local, _)) = pending.pop_front() else {
                    break;
                };
                let (index, clause) = identities[local];
                error(
                    &mut results[index],
                    clause,
                    "relational-failed",
                    "producer capacity exceeded",
                );
                sessions[local] = None;
                continue;
            }
            let slots = group.iter().map(|(local, _)| *local).collect::<Vec<_>>();
            let outputs = producer::execute(group, &mut contexts, allowance);
            for (local, output) in slots.into_iter().zip(outputs) {
                let (index, clause) = identities[local];
                let session = sessions[local].as_mut().unwrap();
                let result = output.and_then(|out| contexts[local].run(|| session.publish(out)));
                if let Err(e) = result {
                    error(&mut results[index], clause, "relational-failed", &e.reason);
                    sessions[local] = None;
                    continue;
                }
                if let Some(demand) = advance(session, &mut contexts[local], &mut results[index]) {
                    next.push_back((local, demand));
                } else {
                    sessions[local] = None;
                }
            }
        }
        pending = next;
    }
    for (local, (index, clause)) in identities.into_iter().enumerate() {
        observe(&mut results[index], clause, &contexts[local]);
    }
}
