mod ast;
mod bindings;
mod cgnative;
mod closure;
mod counts;
mod derived_check;
mod eval;
mod facts;
mod factfile;
mod fstring;
mod leaf;
mod lowered;
mod memo;
mod patcheck;
mod physplan;
mod volume;
use rerail_facts::policy;
mod pyvalue;
mod rel;
mod mutant;
mod repo;
mod rx;
mod spill;
mod stagecheck;
mod sql;
mod trits;
mod xexport;
mod typedefs;
mod views;

use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

const BATCH6: &[&str] = &[
    "serverplan_bypass",
    "serverplan_steps",
    "serverplan_state_tripwires",
    "serverplan_dispatch_architecture",
    "serverplan_quality",
    "serverplan_target_architecture",
    "execution_authority_bypass",
    "impl_direct_execution",
    "raw_execute_regression",
    "legacy_execution_surfaces",
];

/// Runs `f`, turning a panic into its message: the harness paths report a panic as a failure of the
/// call, file or rule it hit (I9: refused by name, never an answer, never a lost batch).
fn caught<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|p| {
        p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "non-string panic".into())
    })
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}
fn args_all(args: &[String], name: &str) -> Vec<String> {
    args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1).cloned()).collect()
}

fn policy_doc(dir: &str, check: &str) -> Json {
    let p = format!("{dir}/data-layer-{}.json", check.replace('_', "-"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("replay") => replay(&args),
        Some("calls") => calls(&args),
        Some("repo") => repo::repo(&args),
        Some("repo-compare") => repo::compare(&args),
        Some("store") => factfile::store(&args),
        Some("inspect") => factfile::inspect(&args),
        Some("derived-check") => derived_check::run(&args),
        Some("cg-native") => cgnative::run(&args),
        Some("factfile-bench") => factfile::bench(&args),
        Some("census") => census(&args),
        Some("xexport") => xexport::main(&args),
        Some("explain") => explain(&args),
        Some("export-volume") => volume::run(&args),
        Some("pattern-check") => patcheck::run(&args),
        Some("stage-check") => stagecheck::run(&args),
        Some("parse") => {
            let path = &args[2];
            let source = Arc::new(std::fs::read_to_string(path).unwrap());
            match facts::build(path, source).unwrap() {
                facts::Built::Facts(f) => println!("nodes {} errors {}", f.len(), f.syntax_errors),
                facts::Built::Unparsed => println!("unparsed"),
            }
        }
        _ => eprintln!("usage: facts-a replay|repo|repo-compare|calls|store|inspect|census|explain ..."),
    }
}

/// The harness bridge (`conformance/data-layer/*.ts --engine facts`): evaluates one call per line of
/// `--input` (JSONL `{id, policy, files: [{path, source}]}`, `policy` the document the harness
/// resolved for the call: options in `define.config`, its final scope and clauses) and writes one
/// line per call to `--output`: `{id, findings: [{check, clause, path, line, code}], failures:
/// [{check, clause, path, message}]}`, or `{id, compile_error}` when the policy does not compile
/// (the harness reports it as a failure, never as an answer). Compiled policies are cached by text.
fn calls(args: &[String]) {
    use std::io::{BufRead, Write};
    let input = arg(args, "--input").expect("--input");
    let output = arg(args, "--output").expect("--output");
    let reader = std::io::BufReader::new(std::fs::File::open(&input).unwrap_or_else(|e| panic!("{input}: {e}")));
    let mut out = std::io::BufWriter::new(std::fs::File::create(&output).unwrap_or_else(|e| panic!("{output}: {e}")));
    let mut compiled: HashMap<String, Result<Rc<Vec<eval::Rule>>, String>> = HashMap::new();
    for line in reader.lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let call: Json = serde_json::from_str(&line).unwrap();
        let policy = &call["policy"];
        let key = serde_json::to_string(policy).unwrap();
        let rules = compiled
            .entry(key)
            .or_insert_with(|| {
                caught(|| eval::Rule::compile(policy, &Map::new(), None, &facts::GRAMMAR))
                    .unwrap_or_else(|panic| Err(format!("compiler panic: {panic}")))
                    .map(|r| Rc::new(vec![r]))
            })
            .clone();
        let result = match rules {
            Err(e) => json!({"id": call["id"], "compile_error": e}),
            Ok(rules) => {
                let files: Vec<(String, Arc<String>)> = call["files"].as_array().expect("files").iter().map(|f| (f["path"].as_str().unwrap().to_owned(), Arc::new(f["source"].as_str().unwrap().to_owned()))).collect();
                let t = Instant::now();
                // A panic is a failure of this call, its findings discarded; it never loses the batch.
                let o = caught(|| evaluate(&rules, &files)).unwrap_or_else(|panic| {
                    let mut o = eval::Output::default();
                    o.failures.push(eval::Failure { check: rules[0].check.clone(), clause: String::new(), path: String::new(), message: format!("engine panic: {panic}") });
                    o
                });
                json!({"id": call["id"], "ms": t.elapsed().as_secs_f64() * 1e3, "findings": o.findings, "failures": o.failures})
            }
        };
        serde_json::to_writer(&mut out, &result).unwrap();
        out.write_all(b"\n").unwrap();
    }
    out.flush().unwrap();
}

/// Evaluates the given files under compiled rules: findings and failures.
fn evaluate(rules: &[eval::Rule], files: &[(String, Arc<String>)]) -> eval::Output {
    let mut out = eval::Output::default();
    let mut repo = spill::Memory::default();
    let mut budget = BTreeMap::new();
    let mut sorted: Vec<&(String, Arc<String>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, source) in sorted {
        let built = if path.ends_with(".rs") { None } else { Some(facts::build(path, source.clone())) };
        match built {
            Some(Ok(facts::Built::Facts(f))) => {
                let ctx = eval::FileCtx { f: &f, path, source };
                eval::eval_file(rules, &ctx, &mut out, &mut repo, &mut budget);
            }
            Some(Ok(facts::Built::Unparsed)) | Some(Err(_)) => {
                out.failures.push(eval::Failure { check: String::new(), clause: String::new(), path: path.clone(), message: "file could not be parsed".into() });
            }
            None => eval::eval_text_file(rules, path, source, &mut out),
        }
    }
    eval::finish(rules, |k| repo.0.remove(&k).unwrap_or_default(), budget, &mut out);
    out
}

/// The call configuration `replay.ts` resolves for a recorded event: `oracleAnalyzer =
/// !production_scope` where the policy declares that option, merged with
/// `oracle_config.options`; the event scope replaces the policy scope unless `production_scope`
/// is set (an empty include list matches nothing).
fn call_config(doc: &Json, e: &Json) -> (Map<String, Json>, Option<Json>) {
    let production = e.get("production_scope").and_then(Json::as_bool).unwrap_or(false);
    let mut config = Map::new();
    let declares_oracle = doc["clauses"].as_array().unwrap().iter().any(|c| c.get("define").and_then(|d| d.get("options")).and_then(|o| o.get("oracleAnalyzer")).is_some());
    if declares_oracle {
        config.insert("oracleAnalyzer".into(), Json::Bool(!production));
    }
    if let Some(opts) = e.get("oracle_config").and_then(|c| c.get("options")).and_then(Json::as_object) {
        for (k, v) in opts {
            config.insert(k.clone(), v.clone());
        }
    }
    let scope = if production {
        None
    } else {
        let s = e["scope"].clone();
        if s["include"].as_array().is_none_or(|a| a.is_empty()) {
            Some(json!({"include": ["**/*"], "exclude": ["**/*"]}))
        } else {
            Some(json!({"include": s["include"], "exclude": s.get("exclude").cloned().unwrap_or(json!([]))}))
        }
    };
    (config, scope)
}

fn replay(args: &[String]) {
    let policies = arg(args, "--policies").unwrap_or("packs/data-layer/candidates/rules".into());
    let out_path = arg(args, "--output").expect("--output");
    let only = arg(args, "--check");
    let only_test = arg(args, "--test");
    let mut events = Vec::new();
    for rec in args_all(args, "--recording") {
        let d: Json = serde_json::from_str(&std::fs::read_to_string(&rec).unwrap()).unwrap();
        for e in d["events"].as_array().unwrap() {
            events.push(e.clone());
        }
    }
    let mut compiled: HashMap<String, Rc<Vec<eval::Rule>>> = HashMap::new();
    let mut docs: HashMap<String, Json> = HashMap::new();
    let mut per_check: BTreeMap<String, Map<String, Json>> = BTreeMap::new();
    let mut rows = Vec::new();
    let t0 = Instant::now();
    for e in &events {
        let check = e["check"].as_str().unwrap().to_owned();
        if !BATCH6.contains(&check.as_str())
            || only.as_ref().is_some_and(|o| o != &check)
            || only_test.as_ref().is_some_and(|t| !e["test"].as_str().unwrap_or("").contains(t.as_str()))
        {
            continue;
        }
        if e.get("retained_reason").is_some() || e.get("upstream_passed") == Some(&Json::Bool(false)) || e.get("files").is_none() {
            continue;
        }
        let doc = docs.entry(check.clone()).or_insert_with(|| policy_doc(&policies, &check)).clone();
        let (config, scope) = call_config(&doc, e);
        let key = format!("{check}\0{}\0{}", serde_json::to_string(&config).unwrap(), serde_json::to_string(&scope).unwrap());
        let rules = match compiled.get(&key) {
            Some(r) => r.clone(),
            None => match eval::Rule::compile(&doc, &config, scope.as_ref(), &facts::GRAMMAR) {
                Ok(r) => {
                    let r = Rc::new(vec![r]);
                    compiled.insert(key.clone(), r.clone());
                    r
                }
                Err(err) => {
                    eprintln!("compile {check}: {err}");
                    let entry = per_check.entry(check.clone()).or_default();
                    *entry.entry("compile_failures").or_insert(json!(0)) = json!(entry.get("compile_failures").and_then(Json::as_u64).unwrap_or(0) + 1);
                    rows.push(json!({"check": check, "test": e["test"], "call": e["call"], "compile_error": err}));
                    continue;
                }
            },
        };
        let files: Vec<(String, Arc<String>)> = e["files"].as_array().unwrap().iter().map(|f| (f["path"].as_str().unwrap().to_owned(), Arc::new(f["source"].as_str().unwrap().to_owned()))).collect();
        let out = evaluate(&rules, &files);
        let key_of = |p: &str, l: u64, c: &str| format!("{p}\u{1f}{l}\u{1f}{c}");
        let mut expected: Vec<String> = e["findings"].as_array().unwrap().iter().map(|f| key_of(f["path"].as_str().unwrap(), f["line"].as_u64().unwrap(), f["code"].as_str().unwrap())).collect();
        let mut actual: Vec<String> = out.findings.iter().map(|f| key_of(&f.path, f.line as u64, &f.code)).collect();
        expected.sort();
        actual.sort();
        let fp = multiset_minus(&actual, &expected);
        let fnn = multiset_minus(&expected, &actual);
        let failures: Vec<Json> = out.failures.iter().map(|f| json!({"clause": f.clause, "path": f.path, "message": f.message})).collect();
        let entry = per_check.entry(check.clone()).or_default();
        let bump = |m: &mut Map<String, Json>, k: &str, n: u64| {
            let v = m.get(k).and_then(Json::as_u64).unwrap_or(0) + n;
            m.insert(k.into(), json!(v));
        };
        bump(entry, "rows", 1);
        bump(entry, if expected.is_empty() { "clean_rows" } else { "positive_rows" }, 1);
        bump(entry, "false_positives", fp.len() as u64);
        bump(entry, "false_negatives", fnn.len() as u64);
        bump(entry, "failures", failures.len() as u64);
        bump(entry, "expected_findings", expected.len() as u64);
        if !fp.is_empty() || !fnn.is_empty() || !failures.is_empty() {
            rows.push(json!({"check": check, "test": e["test"], "call": e["call"], "entry": e["entry"], "false_positives": fp, "false_negatives": fnn, "failures": failures}));
        }
    }
    let mut totals = Map::new();
    for m in per_check.values() {
        for (k, v) in m {
            let t = totals.get(k).and_then(Json::as_u64).unwrap_or(0) + v.as_u64().unwrap_or(0);
            totals.insert(k.clone(), json!(t));
        }
    }
    let result = json!({"seconds": t0.elapsed().as_secs_f64(), "totals": totals, "checks": per_check, "mismatches": rows});
    std::fs::write(&out_path, serde_json::to_string_pretty(&result).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&json!({"totals": result["totals"], "checks": result["checks"]})).unwrap());
}

fn multiset_minus(a: &[String], b: &[String]) -> Vec<String> {
    let mut counts: HashMap<&String, i64> = HashMap::new();
    for x in b {
        *counts.entry(x).or_default() += 1;
    }
    let mut out = Vec::new();
    for x in a {
        let c = counts.entry(x).or_default();
        if *c > 0 {
            *c -= 1;
        } else {
            out.push(x.clone());
        }
    }
    out
}

/// Phase 2 census: every policy in `--rules` compiled by `rerail-facts` with this crate's leaf
/// executors (default option values), written as `census.json` and `census.md` in `--out`.
fn census(args: &[String]) {
    let rules = arg(args, "--rules").unwrap_or_else(|| "packs/data-layer/candidates/rules".into());
    let out = std::path::PathBuf::from(arg(args, "--out").expect("--out"));
    std::fs::create_dir_all(&out).unwrap();
    let mut leaves = leaf::Leaves::new();
    let c = rerail_facts::census::run(std::path::Path::new(&rules), &Map::new(), &mut leaves).expect("census");
    std::fs::write(out.join("census.json"), serde_json::to_string_pretty(&c.json).unwrap()).unwrap();
    std::fs::write(out.join("census.md"), &c.markdown).unwrap();
    println!("{}", c.markdown);
    // IR-level pass: every pattern and regex leaf accepted, so the table shows what the IR, the
    // front ends and the classifier decide whatever the leaf executor covers.
    let ir = rerail_facts::census::run(std::path::Path::new(&rules), &Map::new(), &mut rerail_facts::census::IrOnly).expect("census");
    std::fs::write(out.join("census-ir.json"), serde_json::to_string_pretty(&ir.json).unwrap()).unwrap();
    std::fs::write(out.join("census-ir.md"), &ir.markdown).unwrap();
    // The option variant that makes state-flow-1 / state-projection-recovery-2 repository clauses.
    let mut cfg = Map::new();
    cfg.insert("entryPoint".into(), Json::String("scan_project_sources".into()));
    let ps = rerail_facts::census::run(std::path::Path::new(&rules), &cfg, &mut rerail_facts::census::IrOnly);
    match ps {
        Ok(ps) => {
            std::fs::write(out.join("census-ir-scan-project-sources.json"), serde_json::to_string_pretty(&ps.json).unwrap()).unwrap();
            std::fs::write(out.join("census-ir-scan-project-sources.md"), &ps.markdown).unwrap();
        }
        Err(e) => eprintln!("scan_project_sources variant: {e}"),
    }
}

/// Phase 2 `EXPLAIN`: each policy in `--rules` (or only `--rule <id>`) compiled under `--config`
/// (JSON object of option values, default `{}`), its logical plans printed as stable text;
/// refusals listed after the plans.
fn explain(args: &[String]) {
    if args.iter().any(|a| a == "--physical") {
        return physplan::run(args);
    }
    if args.iter().any(|a| a == "--analyze") {
        return physplan::analyze(args);
    }
    if args.iter().any(|a| a == "--classes") {
        return physplan::census(args);
    }
    let rules = arg(args, "--rules").unwrap_or_else(|| "packs/data-layer/candidates/rules".into());
    let only = arg(args, "--rule");
    let config: Map<String, Json> = arg(args, "--config").map(|c| serde_json::from_str(&c).expect("--config")).unwrap_or_default();
    let mut paths: Vec<_> = std::fs::read_dir(&rules).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    paths.sort();
    let mut leaves = leaf::Leaves::new();
    for p in paths {
        let doc: Json = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let id = doc["id"].as_str().unwrap_or("").to_owned();
        if only.as_ref().is_some_and(|o| *o != id && format!("data-layer-{o}") != id) {
            continue;
        }
        let c = rerail_facts::compile::compile_report(&doc, &config, &mut leaves).expect("compile");
        print!("{}", rerail_facts::explain::explain(&c.program));
        for r in &c.refusals {
            println!("refused {} {}: {}", r.clause, r.at, r.construct);
        }
    }
}
