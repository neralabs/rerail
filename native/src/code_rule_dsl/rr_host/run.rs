//! `runRr`: the request's policies run over its corpus root by `tools::run_policies`, as `rerail-engine run-policies`
//! runs them, and their findings with their spans (`findings`). A policy whose run fails is reported as a failure of
//! its own and the others run without it.

use super::findings;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

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
}

/// The policies' run's outcome as JSON: `{findings, failures, files}`. An error where the request does not read or a
/// failure is no one policy's.
pub fn run(request: &str) -> Result<String, String> {
    let request: Request = serde_json::from_str(request).map_err(|e| format!("a .rerail run request: {e}"))?;
    // The runner's child process ends with its run, as a command of the CLI does (decision 0272): its run's end serves
    // the join searches it leaves waiting.
    crate::code_rule_dsl::logic::plan::joins::drain_at_end(true);
    let ran = run_each(&request);
    // The cost samples the run made are written, as the CLI's commands write theirs (decision 0100 §14).
    crate::code_rule_dsl::logic::costs::flush();
    let ran = ran?;
    let findings = findings::of(&request.root, &ran.out, &ran.report)?;
    let files = ran.report["runs"].as_array().into_iter().flatten().filter_map(|run| run["files"].as_u64()).max().unwrap_or(0);
    serde_json::to_string(&json!({"findings": findings, "failures": ran.failures, "files": files})).map_err(|e| e.to_string())
}

/// A run of the policies that ran: its report, where its outputs are, and the policies that failed before it.
struct Ran {
    report: Value,
    out: PathBuf,
    failures: Vec<Value>,
}

/// The request's policies run; where a run fails with one policy's error, that policy is a failure and the others run
/// again without it, each attempt into its own directory.
fn run_each(request: &Request) -> Result<Ran, String> {
    let options = request.input_files.as_ref().map_or(Value::Null, |files| json!({"inputFiles": files}));
    let mut left = request.policies.clone();
    let mut failures = Vec::new();
    // Each failed attempt takes one policy out: at most one attempt more than there are policies.
    for attempt in 0..=request.policies.len() {
        let out = request.out.join(attempt.to_string());
        let error = match run_policies(&left, &request.root, &out, &options) {
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
