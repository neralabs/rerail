//! `compile(rule) -> Result<Program, Unsupported>`: front ends, then classification.
use crate::classify::{ClauseReport, classify};
use crate::front::rule::{LeafCheck, front};
use crate::ir::{Program, Refusal, Unsupported};
use serde_json::{Map, Value as Json};

/// Everything the compiler decided about one rule, refused or not (the census reads this).
pub struct Compiled {
    /// The plans of every clause that compiled.
    pub program: Program,
    /// Every refused construct, front end and classifier.
    pub refusals: Vec<Refusal>,
    pub reports: Vec<ClauseReport>,
    /// Clauses whose `when` does not hold under the configuration.
    pub disabled: Vec<String>,
}

/// Compiles every clause it can and records every refusal. `Err` only for a document the
/// front end cannot read at all (not a policy, unresolvable `$option`).
pub fn compile_report(document: &Json, config: &Map<String, Json>, leaves: &mut dyn LeafCheck) -> Result<Compiled, String> {
    let f = front(document, config, leaves)?;
    let rule = f.rule.clone();
    let inputs = f.inputs.clone();
    let report_when = f.report_when;
    let runtime_refusals = f.runtime_refusals.clone();
    let disabled = f.disabled.clone();
    let mut refusals = f.refusals.clone();
    // Clauses refused in the front end still report their declared partition.
    let mut reports: Vec<ClauseReport> = Vec::new();
    for c in &f.clauses {
        if c.body.is_none() {
            reports.push(ClauseReport {
                clause: c.id.clone(),
                form: "refused",
                declared: c.declared,
                inferred: None,
                file_stages: 0,
                repo_stages: 0,
                exports: vec![],
                order: vec![],
            });
        }
    }
    let cl = classify(f);
    refusals.extend(cl.refusals);
    reports.extend(cl.reports);
    let program = Program { rule, inputs, per_file: cl.per_file, repository: cl.repository, exported: cl.exported, report_when, runtime_refusals };
    Ok(Compiled { program, refusals, reports, disabled })
}

/// Compiles a rule under `config`, or names every construct it refuses.
pub fn compile(document: &Json, config: &Map<String, Json>, leaves: &mut dyn LeafCheck) -> Result<Program, Unsupported> {
    let rule = document.get("id").and_then(Json::as_str).unwrap_or("").to_owned();
    let c = compile_report(document, config, leaves).map_err(|e| Unsupported { rule: rule.clone(), refusals: vec![Refusal { clause: String::new(), at: "document".into(), construct: e }] })?;
    if c.refusals.is_empty() { Ok(c.program) } else { Err(Unsupported { rule, refusals: c.refusals }) }
}
