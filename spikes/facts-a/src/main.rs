mod ast;
mod eval;
mod facts;
mod factfile;
mod fstring;
mod policy;
mod pyvalue;
mod rel;
mod repo;
mod sql;

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
        Some("repo") => repo::repo(&args),
        Some("repo-compare") => repo::compare(&args),
        Some("store") => factfile::store(&args),
        Some("inspect") => factfile::inspect(&args),
        Some("parse") => {
            let path = &args[2];
            let source = Arc::new(std::fs::read_to_string(path).unwrap());
            match facts::build(path, source).unwrap() {
                facts::Built::Facts(f) => println!("nodes {} errors {}", f.len(), f.syntax_errors),
                facts::Built::Unparsed => println!("unparsed"),
            }
        }
        _ => eprintln!("usage: facts-a replay|repo|repo-compare|store|inspect ..."),
    }
}

/// Evaluates the given files under compiled rules: findings and failures.
fn evaluate(rules: &[eval::Rule], files: &[(String, Arc<String>)]) -> eval::Output {
    let mut out = eval::Output::default();
    let mut repo = eval::RepoAcc::default();
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
            None => {}
        }
    }
    eval::finish(rules, repo, budget, &mut out);
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
