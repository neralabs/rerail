//! Decide complete-empty admission from evidence before acquiring a SQL reader.
use super::{Inputs, Result, invalid};
use crate::code_rule_dsl::{
    eval,
    model::{ClauseKind, CompiledClause, CompiledCodeRule, Metrics, Snapshot},
};

pub(super) fn limits(rule: &CompiledCodeRule, snapshot: &Snapshot) -> Result<()> {
    let (files, bytes) = snapshot
        .files
        .iter()
        .filter(|f| rule.scope.matches(&f.path))
        .fold((0usize, 0usize), |(n, bytes), f| {
            (n + 1, bytes.saturating_add(f.source.len()))
        });
    if files > rule.capabilities.limits.max_files
        || bytes > rule.capabilities.limits.max_source_bytes
    {
        return Err(invalid("repository input budget exceeded"));
    }
    Ok(())
}

pub(super) fn check(
    rule: &CompiledCodeRule,
    clause: &CompiledClause,
    inputs: &mut Inputs,
    metrics: &mut Metrics,
) -> Result<bool> {
    limits(rule, &inputs.snapshot)?;
    if crate::code_rule_dsl::budget::matcher_budget_exhausted() {
        return Err(invalid("relational input work budget exceeded"));
    }
    let ClauseKind::Relational(plan) = &clause.kind else {
        return Err(invalid("expected relational job"));
    };
    let Some(slot) = plan.require_nonempty else {
        return Ok(true);
    };
    if inputs.gate.is_none() {
        inputs.gate = Some(match &mut inputs.rows {
            Some(rows) => std::mem::take(&mut rows[slot]),
            None => {
                let _corpus = crate::code_rule_dsl::matcher::CorpusSlot::enter();
                let _calls = if inputs.file_local {
                    eval::stateless_calls(
                        rule,
                        &inputs.snapshot,
                        std::slice::from_ref(clause),
                        metrics,
                    )?
                } else {
                    None
                };
                eval::stateless_rows(
                    rule,
                    clause,
                    (&plan.inputs[slot].1, eval::evidence::Side::Counted),
                    &inputs.snapshot,
                    metrics,
                )?
            }
        });
    }
    let rows = inputs.gate.as_ref().expect("prepared gate");
    if !rows.incomplete.is_empty() {
        return Err(invalid("A relational input is incomplete"));
    }
    let admitted = !rows.witnesses.is_empty();
    #[cfg(feature = "code-rule-profile")]
    {
        let mut trace =
            crate::code_rule_dsl::profile::trace::Span::start("input-admission", &clause.id);
        trace.detail(|| serde_json::json!({"input":plan.inputs[slot].0,"rows":rows.witnesses.len(),"complete":true,"admitted":admitted}));
    }
    Ok(admitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{CodeRuleDslRuntime, budget::Context, model::FileInput};
    use std::{
        collections::BTreeSet,
        sync::{Arc, atomic::AtomicBool},
    };
    fn rule() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
        CodeRuleDslRuntime::compile(serde_json::json!({"format":"rerail.policy/v1","id":"gate","revision":1,"mode":"log","clauses":[{"id":"gate","relational":{"requireNonemptyInput":"gate","inputs":{"gate":{"evidence":{"as":"n","query":{"ast":{"language":"python","syntax":"valid","rule":{"kind":"module"}}},"project":{"path":"file.path"}}}},"query":"SELECT __path path,__start start_byte,__end end_byte FROM gate"},"violation":{"code":"GATE","message":"Gate","anchor":"witness.span"}}]}).to_string(),"json".into(),include_str!("../capabilities.json").into()).unwrap()
    }
    fn cached() -> Inputs {
        Inputs {
            gate: Some(eval::evidence::Rows::default()),
            rows: None,
            facts: vec![],
            file_local: false,
            snapshot: Snapshot {
                revision: 1,
                files: vec![
                    FileInput::new(
                        "a.py".into(),
                        "python".into(),
                        Arc::new("pass\n".into()),
                        None,
                        None,
                        "test",
                    )
                    .unwrap(),
                ],
            },
        }
    }
    #[test]
    fn published_empty_evidence_does_not_repeat_matching_and_unknown_evidence_fails() {
        let rule = rule();
        let clause = &rule.inner.clauses[0];
        let mut inputs = cached();
        let mut metrics = Metrics::default();
        assert!(!check(&rule.inner, clause, &mut inputs, &mut metrics).unwrap());
        assert_eq!(metrics.visited_nodes, 0);
        inputs.gate.as_mut().unwrap().incomplete = BTreeSet::from(["a.py".into()]);
        assert!(check(&rule.inner, clause, &mut inputs, &mut metrics).is_err());
        assert_eq!(metrics.visited_nodes, 0);
    }
    #[test]
    fn cancellation_is_not_successful_emptiness_and_retry_uses_a_new_context() {
        let rule = rule();
        let clause = &rule.inner.clauses[0];
        let mut inputs = cached();
        let mut metrics = Metrics::default();
        let mut cancelled = Context::work(&Arc::new(AtomicBool::new(true)), 100);
        assert!(
            cancelled
                .run(|| check(&rule.inner, clause, &mut inputs, &mut metrics))
                .is_err()
        );
        assert!(cancelled.observed());
        let mut retry = Context::work(&Arc::new(AtomicBool::new(false)), 100);
        assert!(
            !retry
                .run(|| check(&rule.inner, clause, &mut inputs, &mut metrics))
                .unwrap()
        );
        assert!(!retry.observed());
    }
}
