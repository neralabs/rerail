//! Ignored whole-workspace driver for live SELECT replays. The ordinary native and SQL
//! workspace APIs supply the same selectors, contexts, batching, captures, and failure reporting.

use super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask, WorkspaceOptions};
use crate::code_rule_dsl::{CodeRuleDslRuntime, sql::query_shapes};
use napi::Task;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[cfg(unix)]
fn process_cpu_ns() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: usage is live writable storage for this process's resource counters.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
    let ns = |t: libc::timeval| t.tv_sec as u64 * 1_000_000_000 + t.tv_usec as u64 * 1_000;
    ns(usage.ru_utime) + ns(usage.ru_stime)
}

#[cfg(not(unix))]
fn process_cpu_ns() -> u64 {
    0
}

fn outcomes(response: &Value) -> Value {
    let sorted = |value: &Value| {
        let mut values = value.as_array().cloned().unwrap_or_default();
        values.sort_by_key(Value::to_string);
        values
            .into_iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
    };
    json!({"status":response["evaluation"]["status"],"violations":sorted(&response["evaluation"]["violations"]),
        "failures":sorted(&response["evaluation"]["failures"]),"blindFiles":sorted(&response["blindFiles"])})
}

#[test]
#[ignore = "live SQL shape replay; set RERAIL_SQL_SHAPE_CONFIG and RERAIL_SQL_SHAPE_OUTPUT"]
fn replay_sql_shapes_on_workload() {
    let config_path = std::env::var("RERAIL_SQL_SHAPE_CONFIG").expect("workload config");
    let output = std::env::var("RERAIL_SQL_SHAPE_OUTPUT").expect("new output directory");
    let directory = Path::new(&output);
    assert!(!directory.exists(), "use a fresh artifact directory");
    std::fs::create_dir_all(directory).unwrap();
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let capabilities = std::fs::read_to_string(config["capabilities"].as_str().unwrap()).unwrap();
    let policies: Vec<_> = config["policies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| {
            CodeRuleDslRuntime::compile(
                std::fs::read_to_string(path.as_str().unwrap()).unwrap(),
                "json".to_owned(),
                capabilities.clone(),
            )
            .unwrap()
        })
        .collect();
    let corpus = Path::new(config["corpus"].as_str().unwrap());
    let files:Vec<_> = config["files"].as_array().unwrap().iter().map(|path| {
        let relative = path.as_str().unwrap();
        let absolute = corpus.join(relative);
        let mut file = json!({"language":"python","path":relative,"source":std::fs::read_to_string(&absolute).unwrap()});
        if let Some(module) = crate::code_rule_dsl::language::adapter::module_path("python",&absolute) { file["module"] = module; }
        file
    }).collect();
    let request = json!({"revision":1,"files":files,"removed":[]}).to_string();
    let mut results = Vec::new();
    let mut timings = Vec::new();
    for engine in ["native", "sql"] {
        let mut workspace = CodeRuleDslWorkspace::with_options(Some(WorkspaceOptions {
            engine: Some(engine.to_owned()),
            sql_batch_size: Some(config["batchSize"].as_u64().unwrap_or(2) as u32),
        }))
        .unwrap();
        for policy in &policies {
            workspace.add_code_rule(policy, None).unwrap();
        }
        if engine == "sql" {
            if config["shapePhase"] == "rewrite" {
                let choices = config["shapeRewrite"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned()))
                    .collect();
                query_shapes::start_rewrite(choices);
            } else {
                query_shapes::start(
                    directory.join("cases"),
                    config["shapeSamples"].as_u64().unwrap_or(16) as usize,
                    config["shapeRounds"].as_u64().unwrap_or(3) as usize,
                    config["shapeTargets"]
                        .as_array()
                        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect())
                        .unwrap_or_default(),
                );
            }
        }
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: request.clone(),
            request_buffer: None,
            request_direct: None,
        };
        let cpu_before = process_cpu_ns();
        let started = Instant::now();
        let output = task.compute();
        let cpu_ns = process_cpu_ns() - cpu_before;
        let wall_ns = started.elapsed().as_nanos();
        if engine == "sql" {
            std::fs::write(
                directory.join("replay.json"),
                serde_json::to_vec_pretty(&query_shapes::finish()).unwrap(),
            )
            .unwrap();
        }
        let response: Value = serde_json::from_str(&output.unwrap()).unwrap();
        timings.push(
            json!({"engine":engine,"applyCpuNs":cpu_ns,"applyWallNs":wall_ns,
            "report":serde_json::from_str::<Value>(&workspace.engine_report().unwrap()).unwrap()}),
        );
        let outcome = outcomes(&response);
        assert_eq!(outcome["failures"], json!([]));
        std::fs::write(
            directory.join(format!("{engine}-outcomes.json")),
            serde_json::to_vec_pretty(&outcome).unwrap(),
        )
        .unwrap();
        results.push(outcome);
    }
    assert_eq!(
        results[0], results[1],
        "complete native/SQL workspace output parity"
    );
    let canonical = format!(
        "{{\"status\":{},\"violations\":{},\"failures\":{},\"blindFiles\":{}}}",
        results[0]["status"],
        results[0]["violations"],
        results[0]["failures"],
        results[0]["blindFiles"]
    );
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    if let Some(expected) = config["expectedDigest"].as_str() {
        assert_eq!(digest, expected, "stored native workload digest");
    }
    let summary = json!({"configPath":config_path,"config":config,"completeWorkspaceParity":true,"digest":digest,
        "findings":results[0]["violations"].as_array().unwrap().len(),"files":files.len(),"timings":timings});
    std::fs::write(
        directory.join("workload.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    eprintln!(
        "SQL_SHAPES {}",
        json!({"output":output,"digest":digest,"findings":results[0]["violations"].as_array().unwrap().len(),"completeWorkspaceParity":true})
    );
}
