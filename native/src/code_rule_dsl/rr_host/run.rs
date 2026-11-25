//! `runRr`: the request's policies run over its corpus root by `tools::run_policies`, as `rerail-engine run-policies`
//! runs them, and their findings with their spans (`findings`). A policy whose run fails is reported as a failure of
//! its own: `run_policies` goes on past it, its run holding its error, and the others' findings are read.

use super::findings;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

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

/// A run of the policies: its report, where its outputs are, and the policies that failed in it.
struct Ran {
    report: Value,
    out: PathBuf,
    failures: Vec<Value>,
}

/// The request's policies run once; each that failed (a run of `run_policies`' report with an error) is a failure.
fn run_each(request: &Request) -> Result<Ran, String> {
    let options = request.input_files.as_ref().map_or(Value::Null, |files| json!({"inputFiles": files}));
    let out = request.out.clone();
    let report = if request.policies.is_empty() { json!({"diagnostics": [], "runs": []}) } else { crate::tools::run_policies(&request.policies, &request.root, &out, &options)? };
    let failures = report["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|run| Some(json!({"policy": run["path"].as_str()?, "message": run["error"].as_str()?})))
        .collect();
    Ok(Ran { report, out, failures })
}
