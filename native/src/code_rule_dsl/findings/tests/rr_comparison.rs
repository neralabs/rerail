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
    let base = std::env::var_os("RERAIL_RR_RULE_DIR").map(PathBuf::from).unwrap_or_else(||crate::code_rule_dsl::logic::rr::library().parent().expect("rule library parent").join("packs/python"));
    let began = cpu_ns();
    let wall = Instant::now();
    let rule_paths: Vec<_> = RULES.iter().map(|r| base.join(r)).collect();
    let programs = tools::program_texts(&rule_paths.iter().map(PathBuf::as_path).collect::<Vec<_>>()).unwrap();
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
        "profiling":profile::on(),"workerThreads":rayon::current_num_threads(),
        "flattenCpuNs":flatten_ns,"flattenWallNs":flatten_wall_ns,"cpuNs":cpu,"wallNs":wall_ns,
        "report":{"phases":report.phases,"files":report.files,"nodes":report.nodes,
            "native":report.native,"rows":report.rows,"chunks":report.chunks,"text":report.text,
            "memo":report.memo,"cross":report.cross,"prepared":report.prepared,
            "crossStrata":report.cross_strata,"crossMaintained":report.cross_maintained,
            "crossChanged":report.cross_changed,"crossAgain":report.cross_again,
            "crossDeltaCounts":report.cross_delta_counts,
            "rounds":{"run":report.rounds.run,"converged":report.rounds.converged,"pending":report.rounds.pending},"typing":format!("{:?}",report.typing),
            "typingRunNames":report.typing_runs.iter().map(|(name,_)|name).collect::<Vec<_>>(),
            "phaseRunCount":report.phase_runs.len(),
            "observed":{"stats":report.files_observed.stats,"hashed":report.files_observed.hashed,
                "bytes":report.files_observed.bytes,"reused":report.files_observed.reused,
                "trusted":report.files_observed.trusted,"validated":report.files_observed.validated}},
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
    // Repetition keeps the process alive for a native stack sampler. Each measured
    // run still includes the complete input lifecycle and reports its own cost.
    let repeat = std::env::var("RERAIL_RR_REPEAT").ok().and_then(|n|n.parse::<usize>().ok()).unwrap_or(1).max(1);
    let mut report = Value::Null;
    for _ in 0..repeat { report = run(&root, &out).0; }
    std::fs::write(report_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    assert_eq!(report["workerThreads"].as_u64(),Some(crate::threads::max() as u64),
        "the workload must use its configured thread count, not rayon's machine-wide default");
    let executions: u64=report["report"]["phases"].as_array().unwrap().iter().map(|p|p[4].as_u64().unwrap()).sum();
    assert!(executions<=100,"whole-job SQL execution budget exceeded: {executions} > 100; profile preserved");
    if report["profiling"].as_bool()==Some(true) {
        let lifecycle = report["profile"]["decisions"].as_array().unwrap().iter()
            .filter(|d|d["topic"]=="run lifecycle CPU")
            .map(|d|serde_json::from_str::<Value>(d["text"].as_str().unwrap()).unwrap())
            .find(|d|d["root"]==json!(root)).expect("profile must report the whole lifecycle");
        assert_eq!(lifecycle["componentGraphRequested"],false,
            "the async family has no cross-file component consumer, even through includes");
    }
    if report["profiling"].as_bool()==Some(true)
        && report["report"]["files"].as_u64().is_some_and(|n| n>0 && report["report"]["memo"][0].as_u64()==Some(n))
    {
        let scheduling = report["profile"]["decisions"].as_array().unwrap().iter()
            .filter(|d|d["topic"]=="input scheduling")
            .map(|d|serde_json::from_str::<Value>(d["text"].as_str().unwrap()).unwrap())
            .find(|d|d["root"]==json!(root)).expect("profile must report input scheduling");
        assert_eq!(scheduling["buildJobs"],0,"a fully memoized workload must not schedule per-document builds");
        assert_eq!(scheduling["cachedDocuments"],report["report"]["files"]);
    }
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
    let base = crate::code_rule_dsl::logic::rr::library().parent().expect("rule library parent").join("packs/python");
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

/// Diagnostic replays deliberately execute additional statements. The production budget test stays unchanged.
#[test]
#[ignore = "controlled SQL shape experiments"]
fn complete_async_family_sql_shape_lab() {
    let root=PathBuf::from(std::env::var("RERAIL_SHARED_CORPUS").unwrap());
    let out=PathBuf::from(std::env::var("RERAIL_RR_OUT").unwrap());
    let path=PathBuf::from(std::env::var("RERAIL_RR_REPORT").unwrap());
    let (report,_)=run(&root,&out);
    std::fs::write(path,serde_json::to_string_pretty(&report).unwrap()).unwrap();
}

#[test]
fn candidate_contract_lowers_before_projection_and_roundtrips() {
    use crate::code_rule_dsl::logic::{front,rr,program_facts};
    let root=crate::code_rule_dsl::logic::rr::library().parent().expect("rule library parent").join("packs/python");
    for name in RULES {
        let p=rr::parse(&tools::flatten(&root.join(name)).unwrap()).unwrap();
        assert!(p.natives.values().any(|n|matches!(n,front::Native::PythonCandidates{context,member,..} if context=="async_function"&&member=="open")));
        let needed=front::needed(&p,p.outputs.to_vec());
        assert!(!needed.contains("text")&&!needed.contains("py_attribute")&&!needed.contains("py_exec_context"),"broad scalar dependencies remain for {name}");
        let local=crate::code_rule_dsl::typing::file_local::split(&p);
        assert!(p.natives.iter().filter(|(_,n)|matches!(n,front::Native::PythonCandidates{..})).all(|(rel,_)|local.contains(rel)));
        let mut facts=std::collections::BTreeMap::new();program_facts::write(&mut facts,"p",&p);
        let restored=program_facts::program(&facts,"p").unwrap();assert_eq!(p.natives,restored.natives);
        let printed=rr::print(&p).unwrap();let reparsed=rr::parse(&printed).unwrap();
        assert_eq!(p.natives.values().filter(|n|matches!(n,front::Native::PythonCandidates{..})).count(),reparsed.natives.values().filter(|n|matches!(n,front::Native::PythonCandidates{..})).count());
    }
}

#[test]
fn candidate_dynamic_targets_remain_cross_file() {
    use crate::code_rule_dsl::logic::{front,rr};
    let p=rr::parse(r#"
#include "core.rr"
.decl docs(d:number)
docs(d) :- doc(d,_,_).
.decl targets(q:symbol)
targets(q) :- text(_,q).
.decl out(c:node,f:node,r:node,o:node)
out(c,f,r,o) :- python_candidates(c,f,r,o,docs,targets,"async_function","open").
.output out
"#).unwrap();
    let local=crate::code_rule_dsl::typing::file_local::split(&p);
    assert!(p.natives.iter().filter(|(_,n)|matches!(n,front::Native::PythonCandidates{..})).all(|(rel,_)|!local.contains(rel)));
}

/// Empty positive inputs must reject a join before its expanding prefix. A later
/// request answer must still wake the same rule through its other delta probe.
#[test]
fn empty_required_input_avoids_expansion_and_resumes_after_growth() {
    use crate::code_rule_dsl::logic::native::{self, setfix};
    let db = rusqlite::Connection::open_in_memory().unwrap();
    native::register(&db).unwrap();
    native::register_symbols(&db).unwrap();
    db.execute_batch("CREATE TABLE many(c0 INTEGER); INSERT INTO many VALUES(1),(2),(3); CREATE TABLE ready(c0 INTEGER); CREATE TABLE result(c0 INTEGER);").unwrap();
    let hex = |s: &str| s.bytes().map(|b| format!("{b:02x}")).collect::<String>();
    let scan = format!("pos t:b:{}:{}:i v0",hex("many"),hex("c0"));
    let seed = format!("{scan}\npos c1 v1\nhead v0");
    let delta = format!("{scan}\npos d1 v1\nhead v0");
    setfix::open(&db, &format!("name empty_guard\nrel result i empty\nrel ready i empty\nproduced 1\nseed 0 {}\njoin 0 1 {}",hex(&seed),hex(&delta))).unwrap();
    setfix::run(&db,"empty_guard",0).unwrap();
    let counts=setfix::step_counts(&db,"empty_guard").unwrap();
    assert_eq!(counts.iter().flatten().map(|(n,_)|*n).sum::<i64>(),0,"the empty positive input must prevent the scan: {counts:?}");
    setfix::add(&db,"empty_guard","ready",&[vec![9]]).unwrap();
    setfix::run(&db,"empty_guard",0).unwrap();
    let result:Vec<i64>=db.prepare("SELECT c0 FROM result ORDER BY c0").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(result,vec![1,2,3],"an input that grows must wake the join");
    setfix::close(&db,"empty_guard").unwrap();
}

/// Binding a demand key does not make its remaining membership restrictions redundant.
#[test]
fn annotation_demand_keeps_string_membership_before_parsing() {
    let base = std::env::temp_dir().join(format!("rerail-annotation-demand-{}", std::process::id()));
    let root = base.join("source");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "x: int\ny: list[int]\nz: \"Target\"\n").unwrap();
    let src = r#"
#include "core.rr"
.decl qann(s:node,e:node)
qann(s,e) :- kind(t,"type"), child(t,s,0), kind(s,"string"), parsed(s,"annotation",e).
.decl asked(k:symbol)
asked(k) :- rr_embed_req(s,"annotation"), kind(s,k).
.output qann, asked
"#;
    corpus::run_with_options(&[src], &root, None, &base.join("out"), "program", Binding::Cpu,
        1 << 18, &corpus::Typing::default(), &json!({"inputFiles":["a.py"]})).unwrap();
    let asked = std::fs::read_to_string(base.join("out/asked.csv")).unwrap();
    assert_eq!(asked.trim(), "string", "annotation demand must not parse ordinary type expressions");
    assert_eq!(std::fs::read_to_string(base.join("out/qann.csv")).unwrap().lines().count(), 1);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn long_text_fallback_exposes_its_candidate_keys_to_planning() {
    use crate::code_rule_dsl::{facts::vtab::syntax, logic::{rr,front::{Literal,Term}}};
    let mut p=rr::parse(r#"
#include "core.rr"
.decl result(t:symbol)
result(t) :- node(_,n,_,_,_), text(n,t).
.output result
"#).unwrap();
    assert_eq!(syntax::long_text_twins(&mut p,&[]),1);
    for rule in &p.rules {
        for node in rule.body.iter().filter_map(|lit| match lit {
            Literal::Cmp { r:Term::Call(f,args),.. } if f=="rr_node_text" => args.first(),
            _=>None,
        }) {
            assert!(rule.body.iter().any(|lit| matches!(lit,Literal::Atom(a) if !a.neg && a.rel=="rr_long_text_node" && a.args==[node.clone()])),
                "long-text fallback must expose its sparse node domain before joins: {rule}");
        }
    }
}

#[test]
fn long_text_fallback_keeps_short_and_long_results_across_memo_replay() {
    let base = crate::code_rule_dsl::prefilter::tests::temp().with_extension("long-text-domain");
    let root = base.join("src");
    std::fs::create_dir_all(&root).unwrap();
    let long = format!("[{}]", vec!["123"; 100].join(","));
    std::fs::write(root.join("a.py"), format!("a = {long}\nb = [1]\n")).unwrap();
    let src = r#"
#include "core.rr"
.decl result(t:symbol)
result(t) :- field(_,"right",n), text(n,t).
.output result
"#;
    for _ in 0..2 {
        corpus::run_with_options(&[src], &root, None, &base.join("out"), "program", Binding::Cpu,
            1 << 18, &corpus::Typing::default(), &json!({"inputFiles":["a.py"]})).unwrap();
        let rows = std::fs::read_to_string(base.join("out/result.csv")).unwrap();
        let got: std::collections::BTreeSet<_> = rows.lines().collect();
        assert_eq!(got, [long.as_str(),"[1]"].into_iter().collect());
    }
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn demanded_component_graph_keeps_cross_file_cache_results() {
    let base = crate::code_rule_dsl::prefilter::tests::temp().with_extension("component-demand");
    let root = base.join("src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "def f():\n    pass\n").unwrap();
    std::fs::write(root.join("b.py"), "from a import f\ndef g():\n    pass\n").unwrap();
    let src = r#"
#include "core.rr"
.pragma "cross-memo" "uses"
.decl def(n:node,q:symbol)
def(n,@dot(m,x)) :- kind(n,"function_definition"), field(n,"name",c), text(c,x), docof(n,d), doc(d,_,m).
.decl imp(n:node,q:symbol)
imp(n,@dot(m,x)) :- kind(n,"import_from_statement"), field(n,"module_name",mn), text(mn,m), field(n,"name",xn), text(xn,x).
.decl uses(n:node,f:node)
uses(n,f) :- imp(n,q), def(f,q).
.output uses
"#;
    let out = base.join("out");
    let run = || corpus::run_with_options(&[src], &root, None, &out, "program", Binding::Cpu,
        1 << 18, &corpus::Typing::default(), &json!({"inputFiles":["a.py","b.py"]})).unwrap();
    let first = run();
    let rows = std::fs::read_to_string(out.join("uses.csv")).unwrap();
    assert_eq!(first.cross, (0, 2));
    assert_eq!(rows.lines().count(), 1);
    assert_eq!(run().cross, (2, 0));
    assert_eq!(std::fs::read_to_string(out.join("uses.csv")).unwrap(), rows);
    std::fs::remove_dir_all(base).unwrap();
}
