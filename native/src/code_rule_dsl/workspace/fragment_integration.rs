//! End-to-end qualification through the actual workspace API, in one process per engine.
use super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask, WorkspaceOptions};
use crate::code_rule_dsl::{CodeRuleDslRuntime, test_support::CAPABILITIES};
use napi::Task;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
fn cpu() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage writes a live, correctly sized structure.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
    ((usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as u64) * 1_000_000_000
        + ((usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as u64) * 1000
}
fn rss() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: same as cpu.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
    #[cfg(target_os = "macos")]
    {
        usage.ru_maxrss as u64
    }
    #[cfg(not(target_os = "macos"))]
    {
        (usage.ru_maxrss as u64) * 1024
    }
}
#[cfg(target_os = "macos")]
fn hardware_snapshot() -> Option<(u64, u64)> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: RUSAGE_INFO_V4 writes one rusage_info_v4 into this aligned,
    // correctly sized buffer. Despite its pointer-to-rusage_info_t signature,
    // proc_pid_rusage accepts the structure's address, not a pointer to a pointer.
    let status = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V4,
            usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    if status != 0 {
        return None;
    }
    // SAFETY: the successful V4 call initialized the requested structure.
    let usage = unsafe { usage.assume_init() };
    (usage.ri_instructions != 0 && usage.ri_cycles != 0)
        .then_some((usage.ri_instructions, usage.ri_cycles))
}
#[cfg(not(target_os = "macos"))]
fn hardware_snapshot() -> Option<(u64, u64)> {
    None
}
fn hardware_work(start: Option<(u64, u64)>, end: Option<(u64, u64)>) -> Option<Value> {
    let (start, end) = start.zip(end)?;
    let instructions = end.0.checked_sub(start.0)?;
    let cycles = end.1.checked_sub(start.1)?;
    (instructions != 0 && cycles != 0).then(|| {
        json!({"instructions":instructions,"cycles":cycles,"source":"proc_pid_rusage/RUSAGE_INFO_V4"})
    })
}
fn apply(w: &CodeRuleDslWorkspace, revision: u32, files: Vec<Value>) -> (Value, Value) {
    let wall = Instant::now();
    let started = cpu();
    let hardware_started = hardware_snapshot();
    let mut task = WorkspaceApplyTask {
        state: Arc::clone(&w.state),
        closed: Arc::clone(&w.closed),
        request_json: json!({"revision":revision,"files":files,"removed":[]}).to_string(),
        request_buffer: None,
        request_direct: None,
    };
    let mut output: Value = serde_json::from_str(&task.compute().unwrap()).unwrap();
    let mut evaluation = output["evaluation"].take();
    assert!(
        evaluation["violations"].is_array(),
        "workspace response must contain evaluation.violations"
    );
    // Storage-backed applies may return only a delta. Read the public result pages to compare
    // complete snapshots, including evicted files. Include that publication cost in this run.
    if w.state.lock().unwrap().storage.is_some() {
        let mut cursor = 0;
        let mut violations = vec![];
        let mut failures = vec![];
        loop {
            let mut page: Value = serde_json::from_str(
                &w.result_page_task(cursor, 4 * 1024 * 1024)
                    .compute()
                    .unwrap(),
            )
            .unwrap();
            violations.append(page["violations"].as_array_mut().unwrap());
            failures.append(page["failures"].as_array_mut().unwrap());
            if page["done"] == true {
                break;
            }
            let next = page["cursor"].as_u64().unwrap() as u32;
            assert!(next > cursor, "result paging must advance");
            cursor = next;
        }
        evaluation["violations"] = violations.into();
        evaluation["failures"] = failures.into();
    }
    let mut timing = json!({"cpuNs":cpu()-started,"wallNs":wall.elapsed().as_nanos() as u64,"peakRssBytes":rss()});
    if let Some(work) = hardware_work(hardware_started, hardware_snapshot()) {
        timing["hardwareWork"] = work;
    }
    (evaluation, timing)
}
fn keys(output: &Value) -> BTreeSet<String> {
    output["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| json!([v["ruleId"], v["code"], v["path"], v["span"]["startLine"]]).to_string())
        .collect()
}
fn child(engine: &str) -> Value {
    let root = PathBuf::from(std::env::var("RERAIL_FRAGMENT_CORPUS").expect("corpus path"));
    let policies = std::env::var("RERAIL_FRAGMENT_POLICIES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/python/rules"));
    let mut paths = std::fs::read_dir(&policies)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    capabilities["limits"]["maxFiles"] = 20000.into();
    capabilities["limits"]["maxSourceBytes"] = 10_000_000.into();
    capabilities["limits"]["maxVisitedNodes"] = 10_000_000.into();
    capabilities["limits"]["maxWitnesses"] = 1_000_000.into();
    let compile_start = cpu();
    let compile_wall = Instant::now();
    let mut workspace = CodeRuleDslWorkspace::with_options(Some(WorkspaceOptions {
        engine: Some(engine.into()),
        sql_batch_size: None,
    }))
    .unwrap();
    // A separate qualification dimension, not an engine switch: exercise production SQL-held
    // statistics with an explicitly chosen store. Keep native and SQL stores separate.
    if let Ok(store) = std::env::var("RERAIL_FRAGMENT_E2E_STORE") {
        let store = store.replace("{engine}", engine);
        let attached = workspace
            .storage_task(root.to_string_lossy().into_owned(), store)
            .unwrap()
            .compute()
            .unwrap();
        assert!(attached.attached);
    }
    for path in &paths {
        let policy = CodeRuleDslRuntime::compile(
            std::fs::read_to_string(path).unwrap(),
            "json".into(),
            capabilities.to_string(),
        )
        .unwrap();
        workspace.add_code_rule(&policy, None).unwrap();
    }
    let initial: Value = serde_json::from_str(&workspace.engine_report().unwrap()).unwrap();
    let compile =
        json!({"cpuNs":cpu()-compile_start,"wallNs":compile_wall.elapsed().as_nanos() as u64});
    let listed = std::process::Command::new("git")
        .args(["-C", root.to_str().unwrap(), "ls-files", "-z"])
        .output()
        .unwrap();
    assert!(listed.status.success());
    let file_limit = std::env::var("RERAIL_FRAGMENT_E2E_FILES")
        .ok()
        .map(|v| v.parse::<usize>().unwrap())
        .unwrap_or(usize::MAX);
    let mut files = vec![];
    for path in std::str::from_utf8(&listed.stdout)
        .unwrap()
        .split('\0')
        .filter(|p| p.ends_with(".py"))
    {
        if files.len() >= file_limit {
            break;
        }
        if workspace.accepts_path(path.into()) {
            let mut file = json!({"path":path,"language":"python","source":std::fs::read_to_string(root.join(path)).unwrap()});
            if let Some(module) =
                crate::code_rule_dsl::language::adapter::module_path("python", &root.join(path))
            {
                file["module"] = module;
            }
            files.push(file);
        }
    }
    assert!(!files.is_empty());
    let mut edit = files
        .iter()
        .find(|f| f["source"].as_str().unwrap().contains("def "))
        .unwrap_or(&files[0])
        .clone();
    let count = files.len();
    let bytes = files
        .iter()
        .map(|f| f["source"].as_str().unwrap().len())
        .sum::<usize>();
    let (output, cold) = apply(&workspace, 1, files);
    let report: Value = serde_json::from_str(&workspace.engine_report().unwrap()).unwrap();
    if engine == "sql" {
        assert!(
            report["statements"]["count"].as_u64().unwrap() <= 100,
            "{report}"
        );
    }
    if let Ok(path) = std::env::var("RERAIL_FRAGMENT_E2E_OUTPUT") {
        std::fs::write(format!("{path}.cold.json"),serde_json::to_vec_pretty(&json!({"engine":engine,"files":count,"sourceBytes":bytes,"compile":compile,"cold":cold,"keys":keys(&output),"failures":output["failures"],"report":report})).unwrap()).unwrap();
    }
    // Search indexing normally runs on a 30-second maintenance timer. Complete that existing
    // maintenance work explicitly in this diagnostic, so subsequent edits exercise real stored
    // statistics. This is one-off index preparation, measured separately from either engine.
    let store = workspace
        .state
        .lock()
        .unwrap()
        .storage
        .as_ref()
        .map(|s| s.store.clone());
    let statistics_preparation = if let Some(store) = store {
        let started = cpu();
        let wall = Instant::now();
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut cursor = None;
        loop {
            super::storage::index_stored_tapes(&store, &stop, &mut cursor);
            if cursor.is_none() {
                break;
            }
            assert!(
                wall.elapsed().as_secs() < 300,
                "statistics index maintenance must finish"
            );
        }
        json!({"cpuNs":cpu()-started,"wallNs":wall.elapsed().as_nanos() as u64,"source":"existing stored-tape search-index maintenance"})
    } else {
        Value::Null
    };
    let original_source = edit["source"].as_str().unwrap().to_owned();
    let mut edits = vec![];
    for revision in 2..5 {
        let suffix = match revision {
            2 => "\nassert True\n",
            3 => "\nassert False\nassert True\n",
            _ => "",
        };
        edit["source"] = format!("{original_source}{suffix}").into();
        let (out, time) = apply(&workspace, revision, vec![edit.clone()]);
        let mut evidence = json!({"time":time,"keys":keys(&out),"failures":out["failures"]});
        if std::env::var_os("RERAIL_FRAGMENT_E2E_STORE").is_some() && engine == "sql" {
            let report: Value = serde_json::from_str(&workspace.engine_report().unwrap()).unwrap();
            evidence["planning"] = report["matcherFragments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["planning"].clone())
                .collect::<Vec<_>>()
                .into();
        }
        edits.push(evidence);
    }
    assert_eq!(output["failures"], json!([]), "cold coverage");
    for edit in &edits {
        assert_eq!(edit["failures"], json!([]), "edit coverage");
    }
    assert!(!keys(&output).is_empty(), "positive workload control");
    assert_ne!(
        edits[0]["keys"],
        json!(keys(&output)),
        "adding a finding must change output"
    );
    assert_ne!(
        edits[1]["keys"], edits[0]["keys"],
        "a second finding must change output"
    );
    assert_eq!(
        edits[2]["keys"],
        json!(keys(&output)),
        "restoring source must retract added findings"
    );
    if !statistics_preparation.is_null() && engine == "sql" {
        assert!(
            edits
                .iter()
                .flat_map(|e| e["planning"].as_array().unwrap())
                .any(|p| p["kindCounts"]
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|v| v.as_u64().unwrap() > 0)),
            "attached-store qualification must exercise nonzero SQL-held statistics"
        );
    }
    workspace.close();
    json!({"engine":engine,"policies":paths.len(),"files":count,"sourceBytes":bytes,"compile":compile,"cold":cold,"keys":keys(&output),"failures":output["failures"],"edits":edits,"initial":initial,"report":report,"statisticsPreparation":statistics_preparation})
}
#[test]
#[ignore = "full repository qualification; explicit corpus path required"]
fn fragment_integration_workspace_job() {
    if let Ok(engine) = std::env::var("RERAIL_FRAGMENT_E2E_CHILD") {
        let report = child(&engine);
        std::fs::write(
            std::env::var("RERAIL_FRAGMENT_E2E_OUTPUT").unwrap(),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        return;
    }
    let directory = PathBuf::from(
        std::env::var("RERAIL_FRAGMENT_E2E_DIRECTORY")
            .unwrap_or_else(|_| "/tmp/rerail-fragment-e2e".into()),
    );
    std::fs::create_dir_all(&directory).unwrap();
    let mut reports = vec![];
    for engine in ["native", "sql"] {
        let output = directory.join(format!("{engine}.json"));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "fragment_integration_workspace_job",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RERAIL_FRAGMENT_E2E_CHILD", engine)
            .env("RERAIL_FRAGMENT_E2E_OUTPUT", &output)
            .status()
            .unwrap();
        assert!(status.success(), "{engine} child failed");
        let report: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        eprintln!(
            "{engine}: cold={} compile={} files={}",
            report["cold"], report["compile"], report["files"]
        );
        reports.push(report);
    }
    std::fs::write(
        directory.join("comparison.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
    assert_eq!(
        reports[0]["keys"], reports[1]["keys"],
        "whole-job finding set"
    );
    for i in 0..3 {
        assert_eq!(
            reports[0]["edits"][i]["keys"], reports[1]["edits"][i]["keys"],
            "edit {i}"
        );
    }
}

#[test]
fn fragment_integration_calls_retract_when_dependency_changes() {
    let policy = CodeRuleDslRuntime::compile(
        json!({
            "format":"rerail.policy/v1", "id":"fragment-calls", "revision":1, "mode":"steer",
            "clauses":[{
                "id":"blocking", "forbid":{"ast":{"language":"python", "rule":{
                    "kind":"function_definition", "regex":"^async ",
                    "has":{"stopBy":{"kind":"function_definition"}, "kind":"call",
                        "calls":{"through":"sync", "has":{"stopBy":"end", "kind":"call",
                            "has":{"field":"function", "resolves":{"import":["time.sleep"]}}
                        }}
                    }
                }}}, "violation":{"anchor":"witness.span", "code":"BLOCK", "message":"blocking"}
            }]
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let file = |path: &str, module: &str, source: &str| {
        json!({
            "path":path, "language":"python", "module":module, "source":source
        })
    };
    let mut runs = vec![];
    for engine in ["native", "sql"] {
        let mut workspace = CodeRuleDslWorkspace::with_options(Some(WorkspaceOptions {
            engine: Some(engine.into()),
            sql_batch_size: None,
        }))
        .unwrap();
        workspace.add_code_rule(&policy, None).unwrap();
        let mut results = vec![];
        for (revision, source) in [
            (1, "import time\ndef pause():\n    time.sleep(1)\n"),
            (2, "def pause():\n    pass\n"),
            (3, "import time\ndef pause():\n    time.sleep(1)\n"),
        ] {
            let mut files = vec![file("app/util.py", "app.util", source)];
            if revision == 1 {
                files.extend([
                    file("app/__init__.py", "app", ""),
                    file(
                        "app/api.py",
                        "app.api",
                        "from app.util import pause\nasync def handler():\n    pause()\n",
                    ),
                ]);
            }
            let (result, _) = apply(&workspace, revision, files);
            assert_eq!(result["failures"], json!([]), "{engine}: {result}");
            let found = keys(&result);
            assert_eq!(
                found.len(),
                usize::from(revision != 2),
                "{engine}: {found:?}"
            );
            results.push(found);
        }
        runs.push(results);
        workspace.close();
    }
    assert_eq!(runs[0], runs[1]);
}

#[test]
fn fragment_integration_refuses_execution_101_and_resets_for_the_next_job() {
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        sql::{ClauseRun, FileContext},
        tape::{Grammar, TapeDoc, TapeImage},
    };
    let policy = CodeRuleDslRuntime::compile(
        json!({
            "format":"rerail.policy/v1", "id":"budget-control", "revision":1, "mode":"steer",
            "clauses":[{"id":"c", "forbid":{"ast":{"language":"python", "rule":{"kind":"integer"}}},
                "violation":{"anchor":"witness.span", "code":"N", "message":"number"}}]
        })
        .to_string(),
        "json".into(),
        CAPABILITIES.into(),
    )
    .unwrap();
    let mut workspace = CodeRuleDslWorkspace::with_options(Some(WorkspaceOptions {
        engine: Some("sql".into()),
        sql_batch_size: None,
    }))
    .unwrap();
    workspace.add_code_rule(&policy, None).unwrap();
    let engine = super::sql::engine_of(&mut workspace.state.lock().unwrap()).unwrap();
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let image = TapeImage::parse(&grammar, "1\n", &|| false)
        .unwrap()
        .unwrap();
    let doc = TapeDoc::restore(Arc::new("1\n".into()), grammar, &image, &|| false).unwrap();
    let runs = [ClauseRun {
        statement: 0,
        files: vec![(0, None)],
    }];
    engine.begin_job();
    for _ in 0..100 {
        assert_eq!(
            engine
                .run(vec![(doc.clone(), FileContext::default())], &runs)
                .unwrap()[0][0]
                .len(),
            1
        );
    }
    let error = engine
        .run(vec![(doc.clone(), FileContext::default())], &runs)
        .unwrap_err();
    assert!(
        error.to_string().contains("100 matching SQL executions"),
        "{error}"
    );
    assert_eq!(engine.report()["statements"]["count"], 100);
    engine.begin_job();
    assert_eq!(
        engine
            .run(vec![(doc, FileContext::default())], &runs)
            .unwrap()[0][0]
            .len(),
        1
    );
    assert_eq!(engine.report()["jobMatchingExecutionsReserved"], 1);
    workspace.close();
}
