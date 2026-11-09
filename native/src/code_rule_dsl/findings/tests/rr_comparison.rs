//! The authored `.rr` family through the production relational compiler.
use super::cpu_ns;
use crate::code_rule_dsl::{logic::{plan::corpus, profile}, tools, typing::file_local::Binding};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::{Path, PathBuf}, time::Instant};

type Keys = BTreeSet<(String, String, usize)>;
const RULES: [&str; 4] = [
    "ruff-async210-blocking-http-call-in-async-function.rr",
    "ruff-async221-run-process-in-async-function.rr",
    "ruff-async222-wait-for-process-in-async-function.rr",
    "ruff-async230-blocking-open-call-in-async-function.rr",
];

fn run(root: &Path, out: &Path) -> (Value, Keys) {
    let base = std::env::var_os("RERAIL_RR_RULE_DIR").map(PathBuf::from).unwrap_or_else(||Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python"));
    let began = cpu_ns();
    let wall = Instant::now();
    let programs: Vec<String> = RULES.iter().map(|r| tools::flatten(&base.join(r)).unwrap()).collect();
    let flatten_ns = cpu_ns() - began;
    let flatten_wall_ns = wall.elapsed().as_nanos() as u64;
    let texts: Vec<&str> = programs.iter().map(String::as_str).collect();
    // Use exactly the reference's Python files, including hidden directories.
    // The production directory walker otherwise has different exclusions.
    let mut paths = Vec::new();
    super::tests::collect_paths(root, &mut paths);
    paths.sort();
    let paths: Vec<_> = paths.iter().map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned()).collect();
    let options = json!({"inputFiles":paths});
    // The corpus runner owns the profiler lock. Profiling is a separate run,
    // enabled by RERAIL_PROFILE; timed runs leave it disabled.
    let began = cpu_ns();
    let wall = Instant::now();
    let report = corpus::run_with_options(&texts, root, None, out, "program", Binding::Cpu,
        1 << 18, &corpus::Typing::default(), &options).unwrap();
    let cpu = cpu_ns() - began;
    assert_eq!(report.files, paths.len(), "production workload must include every input file");
    let wall_ns = wall.elapsed().as_nanos() as u64;
    let keys: Keys = match std::fs::read_to_string(out.join("violation.csv")) {
        Ok(text) => text.lines().map(|line| {
            let columns: Vec<_> = line.split('\t').collect();
            assert_eq!(columns.len(), 5, "unexpected violation row: {line}");
            (columns[2].to_owned(), columns[3].to_owned(), columns[4].parse().unwrap())
        }).collect(),
        Err(e) => panic!("read violations: {e}"),
    };
    let spans: Vec<Value> = profile::spans().into_iter().map(|(name,s)| json!({
        "name":name,"calls":s.calls,"cpuSeconds":s.total,"ownCpuSeconds":s.own,
        "allocatedBytes":s.alloc,"ownAllocatedBytes":s.own_alloc,"freedBytes":s.freed,
        "footprintRiseBytes":s.rise,"ownFootprintRiseBytes":s.own_rise})).collect();
    let statements: Vec<Value> = profile::statements().into_iter().map(|(a,b,s)| json!({
        "group":a,"sql":b,"runs":s.runs,"nanoseconds":s.ns,"vmSteps":s.vm,
        "fullScanSteps":s.scan,"sorts":s.sort,"autoindexRows":s.autoindex})).collect();
    let decisions: Vec<Value> = profile::decisions().into_iter().map(|((topic,text),count)|
        json!({"topic":topic,"text":text,"count":count})).collect();
    let profile_dir = profile::directory();
    let result = json!({"schema":"rerail-async-authored-rr-comparison-v1",
        "rules":RULES,"root":root,"out":out,"execution":"combined production corpus::run_with_options; program; CPU",
        "suppression":"declarations retained; raw violation.csv keys",
        "profiling":profile::on(),
        "flattenCpuNs":flatten_ns,"flattenWallNs":flatten_wall_ns,"cpuNs":cpu,"wallNs":wall_ns,
        "report":{"phases":report.phases,"files":report.files,"nodes":report.nodes,
            "native":report.native,"rows":report.rows,"chunks":report.chunks,"text":report.text,
            "memo":report.memo,"cross":report.cross,"prepared":report.prepared,
            "crossStrata":report.cross_strata,"crossMaintained":report.cross_maintained,
            "crossChanged":report.cross_changed,"crossAgain":report.cross_again,
            "crossDeltaCounts":report.cross_delta_counts,"consulted":report.consulted,
            "rounds":{"run":report.rounds.run,"converged":report.rounds.converged,"pending":report.rounds.pending,"rows":report.rounds.rows},"typing":format!("{:?}",report.typing),
            "typingRunNames":report.typing_runs.iter().map(|(name,_)|name).collect::<Vec<_>>(),
            "phaseRunCount":report.phase_runs.len()},
        "profile":{"directory":profile_dir,"spans":spans,"statements":statements,"decisions":decisions},
        "keyCount":keys.len(),"keys":keys});
    (result, keys)
}

#[test]
#[ignore = "controlled authored .rr corpus comparison"]
fn complete_async_family_authored_rr_corpus_profile() {
    let root = PathBuf::from(std::env::var("RERAIL_SHARED_CORPUS").expect("RERAIL_SHARED_CORPUS"));
    let out = PathBuf::from(std::env::var("RERAIL_RR_OUT").expect("RERAIL_RR_OUT"));
    let report_path = PathBuf::from(std::env::var("RERAIL_RR_REPORT").expect("RERAIL_RR_REPORT"));
    let (report, _) = run(&root, &out);
    std::fs::write(report_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    let executions: u64=report["report"]["phases"].as_array().unwrap().iter().map(|p|p[4].as_u64().unwrap()).sum();
    assert!(executions<=100,"whole-job SQL execution budget exceeded: {executions} > 100; profile preserved");
    eprintln!("authored .rr: {} files, {} keys, {} CPU ns, {} wall ns",
        report["report"]["files"],report["keyCount"],report["cpuNs"],report["wallNs"]);
}

#[test]
fn authored_rr_batched_positive_documents_preserve_all_four_labels() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let base = std::env::temp_dir().join(format!("rerail-async-rr-positive-{}-{nonce}",std::process::id()));
    let root = base.join("source");
    std::fs::create_dir_all(&root).unwrap();
    let mut expected = Keys::new();
    for i in 0..24 {
        let path = format!("positive{i:02}.py");
        let source = format!("import requests as r\nimport os as o\nfrom pathlib import Path as P\nasync def f{i}():\n r.get('url')\n o.system('cmd')\n o.spawnl(o.P_WAIT, 'x')\n o.waitpid(1, 0)\n P('x').open()\n");
        std::fs::write(root.join(&path), source).unwrap();
        for (code,line) in [("ASYNC210",5),("ASYNC221",6),("ASYNC221",7),("ASYNC222",8),("ASYNC230",9)] {
            expected.insert((code.to_owned(),path.clone(),line));
        }
    }
    let (mut report, actual) = run(&root, &base.join("out"));
    report["expectedKeys"] = json!(expected);
    report["missingKeys"] = json!(expected.difference(&actual).collect::<Vec<_>>());
    report["unexpectedKeys"] = json!(actual.difference(&expected).collect::<Vec<_>>());
    if let Ok(path) = std::env::var("RERAIL_RR_POSITIVE_REPORT") {
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
    assert_eq!(expected.len(),120);
    assert_eq!(actual,expected,"authored .rr positive comparison: {report}");
}

/// The async family must not build the general name-analysis universe before target filtering.
#[test]
fn async_demand_uses_semantic_batches_without_global_name_derivations() {
    use crate::code_rule_dsl::logic::{front, rr};
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
    for rule in RULES {
        let p = rr::parse(&tools::flatten(&base.join(rule)).unwrap()).unwrap();
        let mut facts=std::collections::BTreeMap::new();
        crate::code_rule_dsl::logic::program_facts::write(&mut facts,"p",&p);
        let restored=crate::code_rule_dsl::logic::program_facts::program(&facts,"p").unwrap();
        assert_eq!(p.natives,restored.natives,"semantic producer metadata lost in prepared program");
        let needed = front::needed(&p, p.outputs.clone());
        let local=crate::code_rule_dsl::typing::file_local::split(&p);
        for name in p.natives.keys().filter(|n|needed.contains(*n)) {
            assert!(local.contains(name),"{rule}: demanded operator escaped its syntax batch: {name}");
        }
        assert!(!needed.contains("unbound_ref"), "{rule} still derives every unbound reference");
        assert!(p.natives.iter().any(|(name,_)| needed.contains(name) && name.contains("python_qualified")), "{rule} has no demanded semantic producer");
    }
}


#[test]
fn semantic_operators_do_not_claim_document_locality_for_cross_file_demands() {
    use crate::code_rule_dsl::logic::{front,rr};
    let p=rr::parse(r#"
#include "core.rr"
.decl wanted(n:node)
.decl targets(q:symbol)
.decl out(n:node,q:symbol)
.output out
wanted(n) :- node(_,n,_,_,_).
targets(q) :- text(_,q).
out(n,q) :- python_qualified(n,q,wanted,targets).
"#).unwrap();
    let local=crate::code_rule_dsl::typing::file_local::split(&p);
    assert!(p.natives.iter().filter(|(_,n)|matches!(n,front::Native::Python{..})).all(|(name,_)|!local.contains(name)));
}


#[test]
fn semantic_helpers_share_exact_contracts_without_relation_name_collisions() {
    use crate::code_rule_dsl::logic::{front::Native,rr};
    let p=rr::parse(r#"
#include "core.rr"
.decl a_b(n:node)
.decl a(n:node)
.decl c(q:symbol)
.decl b_c(q:symbol)
.input a_b,a,c,b_c
.decl out(n:node,q:symbol)
.output out
out(n,q) :- python_qualified(n,q,a_b,c).
out(n,q) :- python_qualified(n,q,a,b_c).
out(n,q) :- python_qualified(n,q,a_b,c).
"#).unwrap();
    let contracts:BTreeSet<_>=p.natives.values().filter_map(|n|match n {
        Native::Python{input,targets,..}=>Some((input.as_str(),targets.as_deref().unwrap())), _=>None,
    }).collect();
    assert_eq!(contracts,BTreeSet::from([("a_b","c"),("a","b_c")]));
    assert_eq!(p.natives.len(),2,"identical contracts should still share one producer");
}
