//! The addon's `.rerail` entry points: `CodeRuleDslRuntime.describeRr` (what each rule file states, checked, nothing
//! run) and `runRr` (the policies over a corpus root, each finding with its span). The product calls them in a child
//! process of its own (`src/engine/rr/rr-runner.ts`): the run path's thread pool and store are the process's.
//! `suppressions` is the findings layer's stage every run of `tools` applies: the linters' comments a finding's
//! `suppress` lines name.

mod describe;
mod findings;
mod run;
mod spans;
pub(crate) mod suppressions;
#[cfg(test)]
mod tests;

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::json::{invalid, json_error};
use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Result, Task};
use napi_derive::napi;

#[napi]
impl CodeRuleDslRuntime {
    /// Each `.rerail` file of `paths_json` (a JSON array of paths) read with its includes and checked, nothing run:
    /// `[{path, ok: true, id, revision, description, mode, language, scope, codes, tests, suppresses, fixes, files}]`,
    /// or `{path, ok: false, error}` for a file that does not read or check.
    #[napi(catch_unwind)]
    pub fn describe_rr(paths_json: String) -> Result<String> {
        let paths: Vec<std::path::PathBuf> = serde_json::from_str(&paths_json).map_err(json_error)?;
        serde_json::to_string(&describe::describe(&paths)).map_err(json_error)
    }

    /// The policies of `request_json` (`{root, policies, out, inputFiles?}`) run over the corpus at `root`, off the
    /// JavaScript thread: `{findings, failures, files}` as JSON.
    #[napi]
    pub fn run_rr(request_json: String) -> AsyncTask<RrRunTask> {
        AsyncTask::new(RrRunTask { request: request_json })
    }
}

pub struct RrRunTask {
    request: String,
}

impl Task for RrRunTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<String> {
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || run::run(&self.request).map_err(invalid))
    }

    fn resolve(&mut self, _env: Env, output: String) -> Result<String> {
        Ok(output)
    }
}
