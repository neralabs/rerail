//! The authored `.rerail` family through the production relational compiler.
use super::cpu_ns;
use crate::code_rule_dsl::{logic::{plan::corpus, profile}, tools, typing::file_local::Binding};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::{Path, PathBuf}, time::Instant};

type Keys = BTreeSet<(String, String, usize)>;
const RULES: [&str; 4] = [
    "ruff-async210-blocking-http-call-in-async-function.rerail",
    "ruff-async221-run-process-in-async-function.rerail",
    "ruff-async222-wait-for-process-in-async-function.rerail",
    "ruff-async230-blocking-open-call-in-async-function.rerail",
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
    let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::open(out).unwrap_or_else(|e| panic!("read violations: {e}"));
    let violation = outputs.get("violation").unwrap_or_else(|| panic!("read violations: no violation in {}", out.display()));
    // rule, clause, code, path, line and column (0165).
    assert_eq!(violation.width(), 6, "unexpected violation columns: {:?}", violation.types());
    let keys: Keys = violation.rows().map(|r| (r.get(2).to_string(), r.get(3).to_string(), r.get(4).int().expect("a line").try_into().unwrap())).collect();
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
            "memo":report.memo,"prepared":report.prepared,
            "crossStrata":report.cross_strata,"crossMaintained":report.cross_maintained,
            "crossChanged":report.cross_changed,"crossAgain":report.cross_again,
            "crossDeltaCounts":report.cross_delta_counts,
            "typing":format!("{:?}",report.typing),
            "phaseRunCount":report.phase_runs.len(),
            "observed":{"stats":report.files_observed.stats,"hashed":report.files_observed.hashed,
                "bytes":report.files_observed.bytes,"reused":report.files_observed.reused,
                "trusted":report.files_observed.trusted,"validated":report.files_observed.validated}},
        "profile":{"directory":profile_dir,"spans":spans,"statements":statements,"decisions":decisions},
        "keyCount":keys.len(),"keys":keys});
    (result, keys)
}

#[test]
#[ignore = "controlled authored .rerail corpus comparison"]
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
    eprintln!("authored .rerail: {} files, {} keys, {} CPU ns, {} wall ns",
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
    assert_eq!(actual,expected,"authored .rerail positive comparison: {report}");
}

/// What `v`'s outputs need, by name (`analyses::needed`).
fn needed_of(v: &crate::code_rule_dsl::logic::program_image::Version) -> BTreeSet<String> {
    use crate::code_rule_dsl::logic::program_image::{RelId, about::About, analyses};
    let about = About::of(v);
    let need = analyses::needed(v, &about, v.outputs().map(|o| v.rel_of(o).expect("an output is a relation")));
    (0..v.rel_end()).map(RelId).filter(|r| need[r.index()]).map(|r| v.rel_name(r).to_owned()).collect()
}

/// `v`'s natives, by relation.
fn natives_of(v: &crate::code_rule_dsl::logic::program_image::Version) -> std::collections::BTreeMap<String, crate::code_rule_dsl::logic::front::Native> {
    crate::code_rule_dsl::logic::program_image::fields::ProgramFields::natives(v)
}

/// `v`'s natives as its image keeps them: opened again from its bytes.
fn natives_kept(v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> std::collections::BTreeMap<String, crate::code_rule_dsl::logic::front::Native> {
    let folded = v.fold().unwrap();
    natives_of(&crate::code_rule_dsl::logic::program_image::Version::open(folded.base_bytes().unwrap()).unwrap())
}

/// The relations `v`'s file-local split keeps local (`FileLocal`).
fn local_of(v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> BTreeSet<String> {
    crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::typing::file_local::FileLocal>(v).unwrap().local.iter().cloned().collect()
}

/// The async family must not build the general name-analysis universe before target filtering.
#[test]
fn async_demand_uses_semantic_batches_without_global_name_derivations() {
    use crate::code_rule_dsl::logic::rr;
    let base = crate::code_rule_dsl::logic::rr::library().parent().expect("rule library parent").join("packs/python");
    for rule in RULES {
        let p = rr::parse_version(&tools::flatten(&base.join(rule)).unwrap()).unwrap();
        let natives = natives_of(&p);
        assert_eq!(natives,natives_kept(&p),"semantic producer metadata lost in prepared program");
        let needed = needed_of(&p);
        let local=local_of(&p);
        for name in natives.keys().filter(|n|needed.contains(*n)) {
            assert!(local.contains(name),"{rule}: demanded operator escaped its syntax batch: {name}");
        }
        assert!(!needed.contains("unbound_ref"), "{rule} still derives every unbound reference");
        assert!(natives.iter().any(|(name,_)| needed.contains(name) && name.contains("python_qualified")), "{rule} has no demanded semantic producer");
    }
}


#[test]
fn semantic_operators_do_not_claim_document_locality_for_cross_file_demands() {
    use crate::code_rule_dsl::logic::{front,rr};
    let p=rr::parse_version(r#"
#include "core.rerail"
.decl wanted(n:node)
.decl targets(q:symbol)
.decl out(n:node,q:symbol)
.output out
wanted(n) :- node(_,n,_,_,_).
targets(q) :- text(_,q).
out(n,q) :- python_qualified(n,q,wanted,targets).
"#).unwrap();
    let local=local_of(&p);
    assert!(natives_of(&p).iter().filter(|(_,n)|matches!(n,front::Native::Python{..})).all(|(name,_)|!local.contains(name)));
}


#[test]
fn semantic_helpers_share_exact_contracts_without_relation_name_collisions() {
    use crate::code_rule_dsl::logic::{front::Native,rr};
    let p=rr::parse_version(r#"
#include "core.rerail"
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
    let natives=natives_of(&p);
    let contracts:BTreeSet<_>=natives.values().filter_map(|n|match n {
        Native::Python{input,targets,..}=>Some((input.as_str(),targets.as_deref().unwrap())), _=>None,
    }).collect();
    assert_eq!(contracts,BTreeSet::from([("a_b","c"),("a","b_c")]));
    assert_eq!(natives.len(),2,"identical contracts should still share one producer");
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
    use crate::code_rule_dsl::logic::{front,rr};
    let root=crate::code_rule_dsl::logic::rr::library().parent().expect("rule library parent").join("packs/python");
    for name in RULES {
        let p=rr::parse_version(&tools::flatten(&root.join(name)).unwrap()).unwrap();
        let natives=natives_of(&p);
        assert!(natives.values().any(|n|matches!(n,front::Native::PythonCandidates{context,member,..} if context=="async_function"&&member=="open")));
        let needed=needed_of(&p);
        assert!(!needed.contains("text")&&!needed.contains("py_attribute")&&!needed.contains("py_exec_context"),"broad scalar dependencies remain for {name}");
        let local=local_of(&p);
        assert!(natives.iter().filter(|(_,n)|matches!(n,front::Native::PythonCandidates{..})).all(|(rel,_)|local.contains(rel)));
        assert_eq!(natives,natives_kept(&p));
        let printed=rr::print_version(&p).unwrap();let reparsed=rr::parse_version(&printed).unwrap();
        assert_eq!(natives.values().filter(|n|matches!(n,front::Native::PythonCandidates{..})).count(),natives_of(&reparsed).values().filter(|n|matches!(n,front::Native::PythonCandidates{..})).count());
    }
}

#[test]
fn candidate_dynamic_targets_remain_cross_file() {
    use crate::code_rule_dsl::logic::{front,rr};
    let p=rr::parse_version(r#"
#include "core.rerail"
.decl docs(d:number)
docs(d) :- doc(d,_,_).
.decl targets(q:symbol)
targets(q) :- text(_,q).
.decl out(c:node,f:node,r:node,o:node)
out(c,f,r,o) :- python_candidates(c,f,r,o,docs,targets,"async_function","open").
.output out
"#).unwrap();
    let local=local_of(&p);
    assert!(natives_of(&p).iter().filter(|(_,n)|matches!(n,front::Native::PythonCandidates{..})).all(|(rel,_)|!local.contains(rel)));
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
#include "core.rerail"
.decl qann(s:node,e:node)
qann(s,e) :- kind(t,"type"), child(t,s,0), kind(s,"string"), parsed(s,"annotation",e).
.decl asked(k:symbol)
asked(k) :- rr_embed_req(s,"annotation"), kind(s,k).
.output qann, asked
"#;
    corpus::run_with_options(&[src], &root, None, &base.join("out"), "program", Binding::Cpu,
        1 << 18, &corpus::Typing::default(), &json!({"inputFiles":["a.py"]})).unwrap();
    let asked = crate::code_rule_dsl::logic::plan::output::shown(&base.join("out"), "asked").unwrap();
    assert_eq!(asked, [["string"]], "annotation demand must not parse ordinary type expressions");
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&base.join("out"), "qann").unwrap().len(), 1);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn long_text_fallback_exposes_its_candidate_keys_to_planning() {
    use crate::code_rule_dsl::logic::program_image::{Lit, TermRef};
    let p = crate::code_rule_dsl::logic::rr::parse_version(r#"
#include "core.rerail"
.decl result(t:symbol)
result(t) :- node(_,n,_,_,_), text(n,t).
.output result
"#).unwrap();
    let (p, made) = crate::code_rule_dsl::logic::plan::corpus::preparation::text::long_text_twins(p, &Vec::new()).unwrap();
    assert_eq!(made, 1);
    for rule in p.rules() {
        for node in rule.body().filter_map(|lit| match lit.view() {
            Lit::Cmp { r: TermRef::Call(f, mut args), .. } if p.text(f) == "rr_node_text" => args.next(),
            _ => None,
        }) {
            let node = p.show_term(rule, node);
            assert!(rule.body().filter_map(|l| l.atom()).any(|a| !a.neg() && p.rel_name(a.rel()) == "rr_long_text_node" && a.args().map(|t| p.show_term(rule, t)).collect::<Vec<_>>() == [node.clone()]),
                "long-text fallback must expose its sparse node domain before joins: {}", p.show_rule(rule));
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
#include "core.rerail"
.decl result(t:symbol)
result(t) :- field(_,"right",n), text(n,t).
.output result
"#;
    for run in ["first", "memo replay"] {
        corpus::run_with_options(&[src], &root, None, &base.join("out"), "program", Binding::Cpu,
            1 << 18, &corpus::Typing::default(), &json!({"inputFiles":["a.py"]})).unwrap();
        let rows = crate::code_rule_dsl::logic::plan::output::shown(&base.join("out"), "result").unwrap();
        let got: std::collections::BTreeSet<&str> = rows.iter().map(|r| r[0].as_str()).collect();
        assert_eq!(rows.iter().map(Vec::len).max(), Some(1));
        assert_eq!(got, [long.as_str(),"[1]"].into_iter().collect(), "the {run} run");
    }
    std::fs::remove_dir_all(base).unwrap();
}

/// Concurrent runs of one process, each with its own `a.py`: each reads its own long text (O174: the texts were
/// held process-wide by document key, the path, so a run could read another's `a.py`).
#[test]
fn concurrent_runs_read_their_own_long_texts() {
    let src = r#"
#include "core.rerail"
.decl result(t:symbol)
result(t) :- field(_,"right",n), text(n,t).
.output result
"#;
    let runs: Vec<_> = ["111", "222", "333", "444"].into_iter().map(|digits| std::thread::spawn(move || {
        let base = crate::code_rule_dsl::prefilter::tests::temp().with_extension(format!("long-text-{digits}"));
        let root = base.join("src");
        std::fs::create_dir_all(&root).unwrap();
        let long = format!("[{}]", vec![digits; 100].join(","));
        std::fs::write(root.join("a.py"), format!("a = {long}\n")).unwrap();
        for _ in 0..3 {
            corpus::run_with_options(&[src], &root, None, &base.join("out"), "program", Binding::Cpu,
                1 << 18, &corpus::Typing::default(), &json!({"inputFiles":["a.py"]})).unwrap();
            let rows = crate::code_rule_dsl::logic::plan::output::shown(&base.join("out"), "result").unwrap();
            assert_eq!(rows, vec![vec![long.clone()]], "the run of {digits}");
        }
        std::fs::remove_dir_all(base).unwrap();
    })).collect();
    for run in runs {
        run.join().unwrap();
    }
}
