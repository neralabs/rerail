//! Decide complete-empty admission from evidence before acquiring a SQL reader.
use super::{Inputs, Result, invalid};
use crate::code_rule_dsl::{
    eval,
    model::{ClauseKind, CompiledClause, CompiledCodeRule, Metrics, Snapshot},
};

pub(in crate::code_rule_dsl) fn limits(rule: &CompiledCodeRule, snapshot: &Snapshot) -> Result<()> {
    let files = snapshot
        .files
        .iter()
        .filter(|f| rule.scope.matches(&f.path))
        .collect::<Vec<_>>();
    if files.len() > rule.capabilities.limits.max_files
        || files
            .iter()
            .any(|f| f.source.len() > rule.capabilities.limits.max_source_bytes)
    {
        return Err(invalid("repository input budget exceeded"));
    }
    Ok(())
}

pub(in crate::code_rule_dsl) fn check(
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
        return Err(invalid(format!(
            "A relational input is incomplete: {}: {}",
            plan.inputs[slot].0,
            rows.incomplete
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )));
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
mod tests;
