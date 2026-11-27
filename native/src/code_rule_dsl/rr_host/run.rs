//! `runRr`: the request's policies run over its corpus root by `tools::run_policies`, as `rerail-engine run-policies`
//! runs them, and their findings with their spans and fixes (`findings`). A policy whose run fails is reported as a
//! failure of its own and the others run without it.
//!
//! A run's options are its policies' option values (`facts::repository::options`, the `option` facts), beside the
//! run's own settings (`RUN_SETTINGS`). The request gives each policy its values (decision 0720), over the defaults
//! its `.options` declare: the policies that run with the same options run together, in a run of their own.

use super::findings;
use crate::code_rule_dsl::facts::repository::with_declared;
use crate::code_rule_dsl::logic::rr;
use rerail_rr_syntax::syntax::Item;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The run's own settings in the options a run is given (`corpus`, `tools`): no policy option takes one's name
/// (`describe::options`).
pub(super) const RUN_SETTINGS: [&str; 9] = ["inputFiles", "dialect", "toolCalls", "reuse", "memo", "env", "factorize", "phase_dir", "phase_no_loads"];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    root: PathBuf,
    policies: Vec<PathBuf>,
    /// An empty directory the run writes its outputs in.
    out: PathBuf,
    /// Root-relative paths: the run reads these files alone (`run_policies`' `inputFiles`).
    #[serde(default)]
    input_files: Option<Vec<String>>,
    /// Option values by policy (its path, as in `policies`); a policy not named takes its declared defaults.
    #[serde(default)]
    options: BTreeMap<PathBuf, Map<String, Value>>,
}

/// The policies' run's outcome as JSON: `{findings, failures, files}`. An error where the request does not read or a
/// failure is no one policy's.
pub fn run(request: &str) -> Result<String, String> {
    let request: Request = serde_json::from_str(request).map_err(|e| format!("a .rerail run request: {e}"))?;
    // The runner's child process ends with its run, as a command of the CLI does (decision 0272): its run's end serves
    // the join searches it leaves waiting.
    crate::code_rule_dsl::logic::plan::joins::drain_at_end(true);
    let ran = run_groups(&request);
    // The cost samples the run made are written, as the CLI's commands write theirs (decision 0100 §14).
    crate::code_rule_dsl::logic::costs::flush();
    let ran = ran?;
    serde_json::to_string(&json!({"findings": ran.findings, "failures": ran.failures, "files": ran.files})).map_err(|e| e.to_string())
}

/// What the request's runs gave together: every finding, every policy that failed, and the most files a run read.
#[derive(Default)]
struct Outcome {
    findings: Vec<Value>,
    failures: Vec<Value>,
    files: u64,
}

/// The request's policies grouped by their option values, in the order of `policies`: one run a group, each into its
/// own `out/<k>`.
fn run_groups(request: &Request) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();
    for (k, (options, policies)) in groups(request).into_iter().enumerate() {
        let ran = run_each(&policies, &request.root, &request.out.join(k.to_string()), &run_options(request, options))?;
        outcome.findings.extend(findings::of(&request.root, &ran.out, &ran.report)?);
        outcome.failures.extend(ran.failures);
        let files = ran.report["runs"].as_array().into_iter().flatten().filter_map(|run| run["files"].as_u64()).max();
        outcome.files = outcome.files.max(files.unwrap_or(0));
    }
    Ok(outcome)
}

/// The request's policies, each group those that run with the same options (`run_with`); in the order of `policies`.
fn groups(request: &Request) -> Vec<(Map<String, Value>, Vec<PathBuf>)> {
    let mut groups: Vec<(Map<String, Value>, Vec<PathBuf>)> = Vec::new();
    for policy in &request.policies {
        let options = run_with(request, policy);
        match groups.iter_mut().find(|(other, _)| *other == options) {
            Some((_, members)) => members.push(policy.clone()),
            None => groups.push((options, vec![policy.clone()])),
        }
    }
    groups
}

/// The options `policy` runs with: the defaults its own `.options` declare, then the values the request gives it.
/// Policies run together read one set of `option` facts, so only those with the same options share a run: a policy
/// that declares none and is given none runs with every other such policy.
fn run_with(request: &Request, policy: &Path) -> Map<String, Value> {
    let declared: Vec<(String, String)> = rr::read(policy).map(|(statements, _)| statements).unwrap_or_default().into_iter().filter_map(|(item, _)| match item {
        Item::Setting { key, text } if key == "options" => Some((key, text)),
        _ => None,
    }).collect();
    let mut options = match with_declared(&declared, &Value::Null) {
        Value::Object(defaults) => defaults,
        _ => Map::new(),
    };
    options.extend(request.options.get(policy).cloned().unwrap_or_default());
    options
}

/// The options of a run of one group: the group's options, and the request's files to read.
fn run_options(request: &Request, mut options: Map<String, Value>) -> Value {
    if let Some(files) = &request.input_files {
        options.insert("inputFiles".to_owned(), json!(files));
    }
    if options.is_empty() { Value::Null } else { Value::Object(options) }
}

/// A run of the policies that ran: its report, where its outputs are, and the policies that failed before it.
struct Ran {
    report: Value,
    out: PathBuf,
    failures: Vec<Value>,
}

/// `policies` run over `root` with `options`; where a run fails with one policy's error, that policy is a failure and
/// the others run again without it, each attempt into its own directory under `out`.
fn run_each(policies: &[PathBuf], root: &Path, out: &Path, options: &Value) -> Result<Ran, String> {
    let mut left = policies.to_vec();
    let mut failures = Vec::new();
    // Each failed attempt takes one policy out: at most one attempt more than there are policies.
    for attempt in 0..=policies.len() {
        let out = out.join(attempt.to_string());
        let error = match run_policies(&left, root, &out, options) {
            Ok(report) => return Ok(Ran { report, out, failures }),
            Err(error) => error,
        };
        let k = failed_policy(&left, &error).ok_or(error.clone())?;
        failures.push(json!({"policy": left.remove(k), "message": error}));
    }
    Err("a .rerail run failed with no policy left to run".to_owned())
}

/// `tools::run_policies`, or no run where no policy is left.
fn run_policies(policies: &[PathBuf], root: &Path, out: &Path, options: &Value) -> Result<Value, String> {
    if policies.is_empty() {
        return Ok(json!({"diagnostics": [], "runs": []}));
    }
    crate::tools::run_policies(policies, root, out, options)
}

/// Which of `policies` the run's `error` is: the one it starts with (`run_policies` names the policy it ran alone).
fn failed_policy(policies: &[PathBuf], error: &str) -> Option<usize> {
    policies.iter().position(|policy| error.starts_with(&format!("{}: ", Path::new(policy).display())))
}
