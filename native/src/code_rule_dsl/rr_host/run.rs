//! `runRr`: the request's policies run over its corpus root by `tools::run_policies`, as `rerail-engine run-policies`
//! runs them, and their findings with their spans and fixes (`findings`). A policy whose run fails is reported as a
//! failure of its own and the others run without it.
//!
//! A run's options are its policies' option values (`facts::repository::options`, the `option` facts), beside the
//! run's own settings (`RUN_SETTINGS`). The request gives each policy its values (decision 0720): the policies given
//! the same values run together, in a run of their own.

use super::findings;
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
/// own `out/<k>`. A policy given a value under a run setting's name does not run (`refused`).
fn run_groups(request: &Request) -> Result<Outcome, String> {
    let mut outcome = Outcome { failures: request.policies.iter().filter_map(|policy| refused(request, policy)).collect(), ..Outcome::default() };
    for (k, (options, policies)) in groups(request).into_iter().enumerate() {
        let ran = run_each(&policies, &request.root, &request.out.join(k.to_string()), &run_options(request, options))?;
        outcome.findings.extend(findings::of(&request.root, &ran.out, &ran.report)?);
        outcome.failures.extend(ran.failures);
        let files = ran.report["runs"].as_array().into_iter().flatten().filter_map(|run| run["files"].as_u64()).max();
        outcome.files = outcome.files.max(files.unwrap_or(0));
    }
    Ok(outcome)
}

/// The request's policies, each group those given the same option values (none: no option is set), in the order of
/// `policies`. Policies run together read one set of `option` facts, so only those given the same values share a run.
fn groups(request: &Request) -> Vec<(Map<String, Value>, Vec<PathBuf>)> {
    let mut groups: Vec<(Map<String, Value>, Vec<PathBuf>)> = Vec::new();
    for policy in request.policies.iter().filter(|policy| refused(request, policy).is_none()) {
        let values = request.options.get(policy).cloned().unwrap_or_default();
        match groups.iter_mut().find(|(other, _)| *other == values) {
            Some((_, members)) => members.push(policy.clone()),
            None => groups.push((values, vec![policy.clone()])),
        }
    }
    groups
}

/// The failure of a policy the request gives a value under the name of a run's own setting (`RUN_SETTINGS`): the
/// value would change the run itself, so the policy does not run. `None` for any other policy.
fn refused(request: &Request, policy: &Path) -> Option<Value> {
    let name = request.options.get(policy)?.keys().find(|name| RUN_SETTINGS.contains(&name.as_str()))?;
    Some(json!({"policy": policy, "message": format!("{}: it is given a value for {name}, a run's own setting, which no option takes", policy.display())}))
}

/// The options of a run of one group: the group's option values, and the request's files to read.
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
