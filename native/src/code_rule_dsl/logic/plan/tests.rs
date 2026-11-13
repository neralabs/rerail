use super::*;
use crate::code_rule_dsl::logic::front::prune;
use crate::code_rule_dsl::logic::rr::parse;
use crate::code_rule_dsl::logic::magic::{Options, magic_adorn, magic_sets};
use crate::content_store::ContentStore;

#[test]
fn native_scalar_consumers_match_sql_with_shadowed_values_and_constant_filters() {
    let p=parse(".decl event(k:number,id:number,g:number,v:number)\nevent(9,0,1,7).\nevent(9,1,1,7).\nevent(9,2,1,3).\nevent(8,3,1,90).\nevent(9,4,2,-2).\nevent(9,5,4,9223372036854775807).\nevent(9,6,4,1).\nevent(9,7,4,-1).\n.decl owner(g:number,v:number)\nowner(1,99).\nowner(2,99).\nowner(3,99).\nowner(4,99).\n.decl result(g:number,v:number,n:number,s:number,lo:number,hi:number)\nresult(g,v,n,s,lo,hi) :- owner(g,v), n=count : {event(9,_,g,_)}, s=sum v : {event(9,_,g,v)}, lo=min x : {event(9,_,g,x)}, hi=max y : {event(9,_,g,y)}, dead=sum z : {event(_,_,_,z)}, unused=dead.\n.decl repeated(g:number,s:number)\nrepeated(g,s) :- owner(g,_), s=sum x : {event(x,_,g,x)}.\n.output result,repeated\n").unwrap();
    let mut outputs=Vec::new();
    for modes in ["","program"] {
        let path=crate::code_rule_dsl::fixtures::temp();
        let store=ContentStore::open(&path).unwrap();
        let python=crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered=lower::lower(&p,python,&mut |s|store.intern_atoms(s).map_err(|e|Error::Plan(e.to_string()))).unwrap();
        let db=Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got=run_atoms_native_over(&lowered,&db,"main",Stats::empty(&lowered.program),BTreeSet::new(),modes).unwrap();
        if !modes.is_empty() {assert!(got.native.contains("result"),"scalar result must execute natively");}
        outputs.push(got.outputs.clone());
    }
    assert_eq!(outputs[0],outputs[1]);
    assert_eq!(outputs[1]["repeated"],vec!["1\t0","2\t0","3\t0","4\t0"]);
    assert_eq!(outputs[1]["result"],vec!["1\t99\t3\t17\t3\t7","2\t99\t1\t-2\t-2\t-2","4\t99\t3\t9223372036854775807\t-1\t9223372036854775807"]);
}

/// Two rules of one recursive stratum with one body and different heads (a magic relation and its reader's, as
/// decision 0145's demand writes them) each run their whole plan in the first wave: the plans are kept apart by
/// head, so neither head loses the rows the first wave derives.
#[test]
fn rules_of_one_body_and_two_heads_both_run_their_whole_plan() {
    let p=parse(".decl seed(x:number)\nseed(1).\n.decl e(x:number,y:number)\ne(1,2).\ne(2,3).\ne(3,4).\n.decl a(x:number)\n.decl b(x:number)\na(x) :- seed(x).\na(y) :- a(x), e(x,y).\nb(y) :- a(x), e(x,y).\na(x) :- b(x).\n.output a,b\n").unwrap();
    for modes in ["","program"] {
        let path=crate::code_rule_dsl::fixtures::temp();
        let store=ContentStore::open(&path).unwrap();
        let python=crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered=lower::lower(&p,python,&mut |s|store.intern_atoms(s).map_err(|e|Error::Plan(e.to_string()))).unwrap();
        let db=Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got=run_atoms_native_over(&lowered,&db,"main",Stats::empty(&lowered.program),BTreeSet::new(),modes).unwrap();
        assert_eq!(got.outputs["a"],vec!["1","2","3","4"],"modes {modes:?}");
        assert_eq!(got.outputs["b"],vec!["2","3","4"],"modes {modes:?}");
    }
}

/// Whether this process numbers nodes in preorder, as T's facts and outputs do (`RERAIL_NODE_IDS=preorder`). The
/// tests that compare with T run only then: under ids per definition (the default, decision 0094 §2a) a program that
/// orders nodes reads `node_order`, which T's facts do not have, and outputs show ids as `slot.index`.
fn t_numbering(test: &str) -> bool {
    let preorder = !crate::code_rule_dsl::facts::vtab::syntax::definition_ids();
    if !preorder {
        eprintln!("{test}: skipped, T's facts number nodes in preorder; run with RERAIL_NODE_IDS=preorder");
    }
    preorder
}

/// A head that qualifies a bound name by a constant scope (`sym_at($e, @cat(1539, $x))`, the builtins' symbols)
/// has a native step: the name is made into a slot before the head. A stratum the native engine declines runs in
/// SQLite, a wait on SQL tables between waves that are otherwise in memory.
#[test]
fn a_head_that_qualifies_by_a_constant_scope_runs_natively() {
    let p = parse(".decl p(e: number, x: symbol)\np(1, \"a\").\np(2, \"b\").\n.decl q(e: number, s: symbol)\nq(e, @dot(\"m\", x)) :- p(e, x).\n.output q\n").unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty(&lowered.program), BTreeSet::new(), modes).unwrap();
        if !modes.is_empty() {
            assert!(got.native.contains("q"), "q runs natively: {:?}", got.native);
        }
        let mut o = got.outputs["q"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1].len(), 2, "{:?}", outputs[1]);
}

/// Device waves (decision 0095 §5): three closures of one level, each over a graph with a fork and a cycle, wait
/// for their bit rows' launches, which run as one batch when the level ends; the rows are SQL's.
#[test]
fn closures_of_one_level_launch_in_one_wave() {
    let mut src = String::new();
    for (k, edges) in [(1, "(1,2),(1,3),(2,4),(3,4),(4,1),(5,6)"), (2, "(10,11),(10,12),(12,13),(13,10)"), (3, "(20,21),(21,22),(21,23),(23,20),(24,24)")] {
        src += &format!(".decl e{k}(a: number, b: number)\n");
        for e in edges.split("),(") {
            src += &format!("e{k}({}).\n", e.trim_matches(|c| c == '(' || c == ')'));
        }
        src += &format!(".decl r{k}(a: number, b: number)\nr{k}(a, b) :- e{k}(a, b).\nr{k}(a, c) :- r{k}(a, b), e{k}(b, c).\n");
    }
    src += ".decl o(k: number, a: number, b: number)\no(1, a, b) :- r1(a, b).\no(2, a, b) :- r2(a, b).\no(3, a, b) :- r3(a, b).\n.output o\n";
    let p = parse(&src).unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let before = super::super::lanes::wave::flushed();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty(&lowered.program), BTreeSet::new(), modes).unwrap();
        let after = super::super::lanes::wave::flushed();
        if !modes.is_empty() {
            for r in ["r1", "r2", "r3"] {
                assert!(got.native.contains(r), "{r} runs natively");
            }
            assert_eq!((after.0 - before.0, after.1 - before.1), (1, 3), "the level's three closures in one wave of three launches");
        }
        let mut o = got.outputs["o"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1].len(), 17 + 12 + 13, "{:?}", outputs[1]);
}

/// Device waves (decision 0095 §5) across components (decision 0113): a level of twenty closures is more than one
/// component takes (its estimate reaches `k` fixed costs first), so the scoreboard issues it as several. Their bit
/// rows' launches are still the level's one wave, not one wave per component.
#[test]
fn closures_of_a_level_wider_than_a_component_launch_in_one_wave() {
    const N: usize = 20;
    let mut src = String::new();
    for k in 0..N {
        let b = 100 * k as i64;
        src += &format!(".decl e{k}(a: number, b: number)\n");
        for (x, y) in [(1, 2), (1, 3), (2, 4), (3, 4), (4, 1)] {
            src += &format!("e{k}({}, {}).\n", b + x, b + y);
        }
        src += &format!(".decl r{k}(a: number, b: number)\nr{k}(a, b) :- e{k}(a, b).\nr{k}(a, c) :- r{k}(a, b), e{k}(b, c).\n");
    }
    src += ".decl o(k: number, a: number, b: number)\n";
    for k in 0..N {
        src += &format!("o({k}, a, b) :- r{k}(a, b).\n");
    }
    src += ".output o\n";
    let p = parse(&src).unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let before = super::super::lanes::wave::flushed();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty(&lowered.program), BTreeSet::new(), modes).unwrap();
        let after = super::super::lanes::wave::flushed();
        if !modes.is_empty() {
            for k in 0..N {
                assert!(got.native.contains(&format!("r{k}")), "r{k} runs natively");
            }
            assert_eq!((after.0 - before.0, after.1 - before.1), (1, N as u64), "the level's closures in one wave of {N} launches");
        }
        let mut o = got.outputs["o"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1].len(), 16 * N, "{:?}", outputs[1].len());
}

/// Decision 0123 §9: a probe with no kept plan is planned when its delta first has rows. In `{r, s, t}`, `s` never
/// has a row (`g` is empty, though not to the program's prune: no edge is a loop), so `r`'s probe by `s` is never
/// planned; the probes by `r` are planned when the first wave returns, and the run derives what the SQL run does.
#[test]
fn a_probe_is_planned_when_its_delta_first_has_rows() {
    let src = ".decl e(a: number, b: number)\ne(1, 2).\ne(2, 3).\ne(3, 4).\ne(4, 1).\n\
               .decl g(a: number)\ng(a) :- e(a, a).\n.decl h(a: number)\nh(3).\n\
               .decl r(a: number, b: number)\n.decl s(a: number, b: number)\n.decl t(a: number, b: number)\n\
               r(a, b) :- e(a, b).\nr(a, c) :- r(a, b), e(b, c).\nr(a, c) :- s(a, b), e(b, c).\nr(a, b) :- t(b, a).\n\
               s(a, b) :- r(a, b), g(a).\nt(a, b) :- r(a, b), h(b).\n\
               .decl o(k: number, a: number, b: number)\no(1, a, b) :- r(a, b).\no(2, a, b) :- t(a, b).\n.output o\n";
    let p = parse(src).unwrap();
    let mut outputs = Vec::new();
    for modes in ["", "program"] {
        let _plans = KeepPlans::start(None);
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let before = super::lazy_probes_counted();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty(&lowered.program), BTreeSet::new(), modes).unwrap();
        let after = super::lazy_probes_counted();
        if !modes.is_empty() {
            assert!(got.native.iter().any(|c| c.split('/').any(|r| r == "r")), "r's component runs natively: {:?}", got.native);
            // The probes searched: r by r, r by s, s by r, t by r (`r(a, b) :- t(b, a)` is a copy, with no order to
            // find). Other tests' runs only add to both counts, and never plan more probes than they left.
            let (left, planned) = (after.0 - before.0, after.1 - before.1);
            assert!(left >= 4 && planned >= 3, "left {left}, planned {planned}; native {:?}, inlined {:?}", got.native, got.inlined);
            assert!(left - planned >= 1, "r's probe by s is never planned: left {left}, planned {planned}");
        }
        let mut o = got.outputs["o"].clone();
        o.sort();
        outputs.push(o);
    }
    assert_eq!(outputs[0], outputs[1]);
    // r: the 4-cycle's closure (16) and t's reversals, all in it; t: r's rows into 3 (4).
    assert_eq!(outputs[1].len(), 16 + 4, "{:?}", outputs[1]);
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic")
}

/// The facts T's exporter and `stats.py` built for nine CPython 3.13 modules (`json/`, `ftplib`, `shlex`,
/// `fnmatch`, `glob`), loaded from their compressed dump into a new file.
fn facts() -> std::path::PathBuf {
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    let path = crate::code_rule_dsl::fixtures::temp();
    Connection::open(&path).unwrap().execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    path
}

/// T's `rdl.py sql` front: prune, demand (adornment for `.demand *`), prune.
fn program(name: &str) -> Program {
    let src = std::fs::read_to_string(root().join(format!("plan/golden/programs/{name}.rr"))).unwrap();
    let p = prune(parse(&src).unwrap());
    let options = Options::default();
    prune(if p.demand.iter().any(|d| d == "*") { magic_adorn(p, &options) } else { magic_sets(p, &options) }.unwrap())
}

fn chosen_line(c: &Chosen) -> String {
    let walks: Vec<String> =
        c.walks.iter().map(|(l, m)| format!("[{}, {}]", serde_json::to_string(l).unwrap(), serde_json::to_string(m).unwrap())).collect();
    format!("{}\t{}\t[{}]\t{}", c.head, c.anchor, walks.join(", "), c.work)
}

/// On T's facts, the port chooses T's plans (anchor, walks, estimated work, rule for rule) and derives T's outputs.
#[test]
fn t_programs_plan_and_run_like_t() {
    if !t_numbering("t_programs_plan_and_run_like_t") {
        return;
    }
    let facts = facts();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let golden = root().join(format!("plan/golden/{name}"));
        let got = run_sql(&program(name), &facts).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lines: Vec<String> = got.chosen.iter().map(chosen_line).collect();
        let expected = std::fs::read_to_string(golden.join("rules.txt")).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        for i in (0..lines.len().max(expected.len())).filter(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
            failures.push(format!("{name} plan {i}:\n  got      {:?}\n  expected {:?}", lines.get(i), expected.get(i)));
        }
        let mut files: Vec<String> = std::fs::read_dir(&golden)
            .unwrap()
            .filter_map(|e| e.unwrap().file_name().to_str()?.strip_suffix(".csv").map(str::to_owned))
            .collect();
        files.sort();
        assert_eq!(got.outputs.keys().cloned().collect::<Vec<_>>(), files, "{name}");
        assert!(!lines.is_empty() && got.outputs.values().any(|r| !r.is_empty()), "{name}");
        for (rel, rows) in &got.outputs {
            let expected = std::fs::read_to_string(golden.join(format!("{rel}.csv"))).unwrap();
            let expected: Vec<&str> = expected.lines().collect();
            if rows.iter().map(String::as_str).collect::<Vec<_>>() != expected {
                failures.push(format!("{name} {rel}: {} rows, expected {}", rows.len(), expected.len()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// T's native modes (`RDL_NATIVE`) on T's extension derive T's outputs: each component a mode runs natively gives
/// the rows T's pure SQL mode gives (the goldens). T runs one program per process, and its extension keeps
/// process-wide state (the loaded tape, the native inputs cached by their SQL, the symbols), so each case runs in a
/// child process of this test, as T's `rdl.py` does.
#[test]
fn t_programs_run_natively_like_t() {
    if !t_numbering("t_programs_run_natively_like_t") {
        return;
    }
    const CASE: &str = "RERAIL_RDL_NATIVE_CASE";
    if let Ok(case) = std::env::var(CASE) {
        let (modes, name) = case.split_once(' ').expect("modes and program");
        let load = |db: &Connection| {
            let rc = unsafe {
                crate::code_rule_dsl::logic::native::extension::sqlite3_extension_init(db.handle(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            assert_eq!(rc, rusqlite::ffi::SQLITE_OK);
            Ok(())
        };
        let got = run_sql_native(&program(name), &facts(), modes, load).unwrap_or_else(|e| panic!("{case}: {e}"));
        let golden = root().join(format!("plan/golden/{name}"));
        for (rel, rows) in &got.outputs {
            let expected = std::fs::read_to_string(golden.join(format!("{rel}.csv"))).unwrap();
            let expected: Vec<&str> = expected.lines().collect();
            assert_eq!(rows.iter().map(String::as_str).collect::<Vec<_>>(), expected, "{case} {rel}");
        }
        println!("NATIVE {}", got.native.iter().cloned().collect::<Vec<_>>().join(" "));
        return;
    }
    let mut failures = Vec::new();
    let mut ran = BTreeSet::new();
    let exe = std::env::current_exe().unwrap();
    let cases = ["nearest", "fixpoint", "fixpoint,kseed", "fixpoint,vtab", "fixpoint,direct", "upsert", "setfix", "setfix,join,nearest", "all", "program"]
        .into_iter()
        .flat_map(|modes| ["workload", "workload_demand", "calls"].map(move |name| (modes, name)))
        .collect::<Vec<_>>();
    // Six children at a time: the cases are independent, and six fit the memory budget.
    for chunk in cases.chunks(6) {
        let children = chunk
            .iter()
            .map(|&(modes, name)| {
                let child = std::process::Command::new(&exe)
                    .args(["--exact", "code_rule_dsl::logic::plan::tests::t_programs_run_natively_like_t", "--nocapture", "--test-threads=1"])
                    .env(CASE, format!("{modes} {name}"))
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                (modes, name, child)
            })
            .collect::<Vec<_>>();
        for (modes, name, child) in children {
            let out = child.wait_with_output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            if !out.status.success() {
                failures.push(format!("{modes} {name}: {}", String::from_utf8_lossy(&out.stderr).lines().filter(|l| l.contains("panicked") || l.contains(name)).collect::<Vec<_>>().join(" | ")));
            }
            // The harness prints the test's name first, on the same line.
            for line in stdout.lines().filter_map(|l| l.split_once("NATIVE ").map(|(_, c)| c)) {
                ran.extend(line.split_whitespace().map(|c| format!("{modes} {name} {c}")));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for mode in ["fixpoint", "setfix", "program"] {
        assert!(ran.iter().any(|c| c.starts_with(mode)), "no component ran natively under {mode}: {ran:?}");
    }
}

/// T's statistics atomized (`corpus::atomized_statistics`), for the fixture's Python files.
fn atomized_statistics(t: &Connection, db: &Connection, store: &ContentStore, first: &HashMap<i64, i64>) {
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    super::corpus::atomized_statistics(t, db, store, first, python).unwrap();
}

/// `got`'s outputs, node ids as T's (`first`: each document's first node in T's facts), against T's goldens.
fn outputs_match(name: &str, got: &Outcome, first: &HashMap<i64, i64>, failures: &mut Vec<String>) {
    let golden = root().join(format!("plan/golden/{name}"));
    assert!(!got.outputs.is_empty() && got.outputs.values().any(|r| !r.is_empty()), "{name}");
    for (rel, rows) in &got.outputs {
        let nodes = &got.nodes[rel];
        let mut rows: Vec<String> = rows
            .iter()
            .map(|row| {
                row.split('\t')
                    .enumerate()
                    .map(|(i, v)| match v.parse::<i64>() {
                        Ok(v) if nodes.contains(&i) => (first[&(v >> 32)] + (v & 0xffff_ffff)).to_string(),
                        _ => v.to_owned(),
                    })
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect();
        rows.sort();
        let expected = std::fs::read_to_string(golden.join(format!("{rel}.csv"))).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        if rows.iter().map(String::as_str).collect::<Vec<_>>() != expected {
            let i = (0..rows.len().max(expected.len())).find(|&i| rows.get(i).map(String::as_str) != expected.get(i).copied());
            failures.push(format!(
                "{name} {rel}: {} rows, expected {}; first difference {:?} vs {:?}",
                rows.len(),
                expected.len(),
                i.and_then(|i| rows.get(i)),
                i.and_then(|i| expected.get(i))
            ));
        }
    }
}

/// O13: lowered to atoms and run over the base-relation vtabs of the same files (`facts::vtab::syntax`), T's programs
/// get T's plans and, decoded, T's outputs.
#[test]
fn t_programs_over_atoms_plan_and_run_like_t() {
    if !t_numbering("t_programs_over_atoms_plan_and_run_like_t") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    let modules = Modules::default();
    for (name, source) in syntax::sources(&store, syntax::tests::documents()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    atomized_statistics(&t, &db, &store, &first);
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let golden = root().join(format!("plan/golden/{name}"));
        let p = program(name);
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let got = run_atoms(&lowered, &db, "s").unwrap_or_else(|e| panic!("{name}: {e}"));
        // The plans in the unlowered program's terms, as T printed them.
        let lines: Vec<String> = got
            .chosen
            .iter()
            .map(|c| {
                let rule = &p.rules[c.rule];
                chosen_line(&Chosen {
                    head: rule.head.to_string(),
                    anchor: rule.body[c.steps[0].0].to_string(),
                    walks: c.steps.iter().filter_map(|&(at, m)| m.map(|m| (rule.body[at].to_string(), m.name()))).collect(),
                    ..c.clone()
                })
            })
            .collect();
        let expected = std::fs::read_to_string(golden.join("rules.txt")).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        for i in (0..lines.len().max(expected.len())).filter(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
            failures.push(format!("{name} plan {i}:\n  got      {:?}\n  expected {:?}", lines.get(i), expected.get(i)));
        }
        outputs_match(name, &got, &first, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// O14: the planner's programs as wave programs (`emit`) run through the frontier loop
/// (`waves::schedule::run`) give the rows `run_atoms` gives with its own stratum loop, planned from
/// the estimates and again from the counts the first run's boundary recorded.
#[test]
fn t_programs_as_wave_programs_give_run_atoms_outputs() {
    if !t_numbering("t_programs_as_wave_programs_give_run_atoms_outputs") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::waves::{owner::Backing, schedule};
    use crate::content_store::Binding;
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let docs = syntax::tests::documents();
    store.bind_names(&docs.iter().map(|d| Binding { name: d.core.clone(), digest: d.digest }).collect::<Vec<_>>()).unwrap();
    let db = Connection::open(&path).unwrap();
    let modules = Modules::default();
    for (name, source) in syntax::sources(&store, docs.clone()).unwrap() {
        modules.bind(&db, name, source).unwrap();
    }
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    atomized_statistics(&t, &db, &store, &first);
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let p = program(name);
        let lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let expected = run_atoms(&lowered, &db, "s").unwrap_or_else(|e| panic!("{name}: {e}"));
        // Planned from the estimates, then from the counts the first run's boundary recorded.
        let mut counts = None;
        let mut first: Option<Vec<String>> = None;
        for pass in ["estimates", "counts"] {
            let emitted = emit::emit(&lowered, &db, "s", name, counts.as_ref(), &HashMap::new()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let backing = Backing::default();
            let scope = backing.enter(&store);
            let lease = scope.pool().take().unwrap();
            super::super::native::register_symbols(&lease).unwrap();
            // The steps read the documents' node intervals from the statistics schema, as `run_atoms`.
            lease.execute_batch("ATTACH ':memory:' AS s; CREATE TABLE s.stat_docrange(d INTEGER PRIMARY KEY, lo INTEGER, hi INTEGER)").unwrap();
            let ranges: Vec<(i64, i64, i64)> = db
                .prepare("SELECT d, lo, hi FROM s.stat_docrange")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            for range in ranges {
                lease.execute("INSERT INTO s.stat_docrange VALUES (?1, ?2, ?3)", [range.0, range.1, range.2]).unwrap();
            }
            for statement in &emitted.setup {
                lease.execute_batch(statement).unwrap();
            }
            let outcome = schedule::batches::run(&lease, &store, &emitted.program, "/w", docs.clone(), &schedule::Limits::default(), &mut |_| Ok(None))
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let (got, _) = emit::outputs(&lowered, &lease).unwrap();
            let sql: Vec<String> = emitted.program.steps.iter().map(|s| s.sql.clone()).collect();
            let replanned = first.as_ref().map_or(0, |f| f.iter().zip(&sql).filter(|(a, b)| a != b).count());
            eprintln!(
                "O14 {name} ({pass}): {} steps ({replanned} replanned) in {} waves; {} outputs",
                sql.len(),
                outcome.waves,
                got.len()
            );
            for (rel, rows) in &expected.outputs {
                if got.get(rel) != Some(rows) {
                    failures.push(format!("{name} {pass} {rel}: {} rows, run_atoms {}", got.get(rel).map_or(0, Vec::len), rows.len()));
                }
            }
            first.get_or_insert(sql);
            counts = Some(outcome.rows);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// O12 (option a): `nearest` made by the transducer's file-local FIX (`typing::file_local`) and read as the input
/// `nearest_<set>(n, a)`, on the CPU and on the device work queue: T's programs derive T's outputs.
#[test]
#[cfg_attr(debug_assertions, ignore = "O82: aborts in the metal crate on a debug build (`slice::from_raw_parts` precondition, metal-0.33.0 lib.rs:82)")]
fn t_programs_with_the_transducers_nearest_derive_t_outputs() {
    if !t_numbering("t_programs_with_the_transducers_nearest_derive_t_outputs") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::typing::file_local::Binding;
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let mut failures = Vec::new();
    for binding in [Binding::Cpu, Binding::Metal] {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        let modules = Modules::default();
        atomized_statistics(&t, &db, &store, &first);
        for name in ["workload", "workload_demand", "calls"] {
            let p = program(name);
            let mut lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
            let sets = lowered.transduce_nearest();
            assert!(!sets.is_empty(), "{name} reads nearest");
            assert!(!lowered.program.rules.iter().flat_map(|r| &r.body).any(|l| matches!(l, Literal::Nearest { .. })));
            for (rel, source) in syntax::with_nearest(&store, syntax::tests::documents(), &sets, &Default::default(), binding, &Default::default()).unwrap() {
                if rel.starts_with(crate::code_rule_dsl::logic::front::NEAREST) {
                    db.execute("INSERT OR REPLACE INTO s.stat_meta VALUES (?1, ?2)", rusqlite::params![format!("rows.{rel}"), source.len() as i64]).unwrap();
                    for (col, rows, ndv, sq, sqd) in syntax::statistics(source.as_ref()) {
                        db.execute("INSERT OR REPLACE INTO s.stat_col VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![rel, col, rows, ndv, sq, sqd]).unwrap();
                    }
                }
                modules.bind(&db, &rel, source).unwrap();
            }
            let got = run_atoms(&lowered, &db, "s").unwrap_or_else(|e| panic!("{binding:?} {name}: {e}"));
            let before = failures.len();
            outputs_match(name, &got, &first, &mut failures);
            for failure in &mut failures[before..] {
                *failure = format!("{binding:?} {failure}");
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// O13: T's native modes over atoms. Lowered with the transducer's `nearest` and run over the base-relation vtabs,
/// each component a mode runs in `logic::native` gives the rows T's pure SQL mode gives (the goldens).
#[test]
fn t_programs_run_natively_over_atoms_like_t() {
    if !t_numbering("t_programs_run_natively_over_atoms_like_t") {
        return;
    }
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::typing::file_local::Binding;
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    let modules = Modules::default();
    atomized_statistics(&t, &db, &store, &first);
    let mut failures = Vec::new();
    let mut ran = BTreeSet::new();
    for name in ["workload", "workload_demand", "calls"] {
        let p = program(name);
        let mut lowered = lower::lower(&p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let sets = lowered.transduce_nearest();
        for (rel, source) in syntax::with_nearest(&store, syntax::tests::documents(), &sets, &Default::default(), Binding::Cpu, &Default::default()).unwrap() {
            if rel.starts_with(crate::code_rule_dsl::logic::front::NEAREST) {
                db.execute("INSERT OR REPLACE INTO s.stat_meta VALUES (?1, ?2)", rusqlite::params![format!("rows.{rel}"), source.len() as i64]).unwrap();
                for (col, rows, ndv, sq, sqd) in syntax::statistics(source.as_ref()) {
                    db.execute("INSERT OR REPLACE INTO s.stat_col VALUES (?1, ?2, ?3, ?4, ?5, ?6)", rusqlite::params![rel, col, rows, ndv, sq, sqd]).unwrap();
                }
            }
            modules.bind(&db, &rel, source).unwrap();
        }
        for modes in ["fixpoint", "fixpoint,vtab", "fixpoint,direct", "upsert", "setfix", "setfix,join", "all", "program"] {
            match run_atoms_native(&lowered, &db, "s", modes) {
                Ok(got) => {
                    let before = failures.len();
                    outputs_match(name, &got, &first, &mut failures);
                    for failure in &mut failures[before..] {
                        *failure = format!("{modes} {failure}");
                    }
                    ran.extend(got.native.iter().map(|c| format!("{modes} {name} {c}")));
                }
                Err(e) => failures.push(format!("{modes} {name}: {e}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    for mode in ["fixpoint", "setfix", "program"] {
        assert!(ran.iter().any(|c| c.starts_with(mode)), "no component ran natively under {mode}: {ran:?}");
    }
}

/// The product's programs (`typing/programs/`, mutated from T's: no hand-written demand, list step 29) derive
/// T's outputs on T's facts, through the same front (prune, magic sets, prune). Their plans may differ from
/// T's; their outputs may not.
#[test]
#[ignore = "O95: T's facts predate relations calls.rr now reads (`value`, `str`, …); the facts need them, empty or rebuilt"]
fn product_programs_derive_t_outputs() {
    let facts = facts();
    // T's facts predate `strprefix` (which calls.rr's type relations read, decision 0047): empty here, so every
    // string is str. Likewise the guests (`parsed`, `host_node`, decision 0055): no string is parsed. No T output
    // reads them. Nor do they have `name`, `str`, `value` and `op_at` (core.rr), which calls.rr reads through
    // python/conditions.rr (Typing's B6, B10): empty here, so no `sys.version_info` or `sys.platform` test is
    // decided and every branch is taken, as T assumed.
    Connection::open(&facts).unwrap().execute_batch(
        "CREATE TABLE strprefix(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.strprefix', 0);
         INSERT INTO stat_col VALUES ('strprefix', 'n', 0, 0, 0, 0), ('strprefix', 's', 0, 0, 0, 0);
         CREATE TABLE parsed(s INTEGER, grammar TEXT, root INTEGER); INSERT INTO stat_meta VALUES ('rows.parsed', 0);
         INSERT INTO stat_col VALUES ('parsed', 's', 0, 0, 0, 0), ('parsed', 'grammar', 0, 0, 0, 0), ('parsed', 'root', 0, 0, 0, 0);
         CREATE TABLE host_node(n INTEGER, h INTEGER); INSERT INTO stat_meta VALUES ('rows.host_node', 0);
         INSERT INTO stat_col VALUES ('host_node', 'n', 0, 0, 0, 0), ('host_node', 'h', 0, 0, 0, 0);
         CREATE TABLE value(n INTEGER, v TEXT); INSERT INTO stat_meta VALUES ('rows.value', 0);
         INSERT INTO stat_col VALUES ('value', 'n', 0, 0, 0, 0), ('value', 'v', 0, 0, 0, 0);
         CREATE TABLE str(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.str', 0);
         INSERT INTO stat_col VALUES ('str', 'n', 0, 0, 0, 0), ('str', 's', 0, 0, 0, 0);
         CREATE TABLE name(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.name', 0);
         INSERT INTO stat_col VALUES ('name', 'n', 0, 0, 0, 0), ('name', 's', 0, 0, 0, 0);
         CREATE TABLE op_at(n INTEGER, i INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.op_at', 0);
         INSERT INTO stat_col VALUES ('op_at', 'n', 0, 0, 0, 0), ('op_at', 'i', 0, 0, 0, 0), ('op_at', 's', 0, 0, 0, 0)",
    ).unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let src = crate::code_rule_dsl::logic::settings::program_text(&root().join(format!("../typing/programs/{name}.rr"))).unwrap();
        let mut p = prune(parse(&src).unwrap());
        // T has no library stage (decision 0059): with no `stage` fact, the rules it opens are dead, and their
        // unbound reads of demanded relations (`need_attr(c, a) :- stage("library"), found(c, a, _)`) are not asked.
        p.rules.retain(|r| !r.body.iter().any(|l| l.atom().is_some_and(|a| a.rel == "stage" && !a.neg)));
        let options = Options::default();
        let p = prune(if p.demand.iter().any(|d| d == "*") { magic_adorn(p, &options) } else { magic_sets(p, &options) }.unwrap());
        // `found`'s demand is the magic-set relation, not a hand-written one.
        if name == "calls" {
            assert!(p.decls.keys().any(|rel| rel.starts_with("magic_found")), "calls: no magic relation for `found`");
        }
        let golden = root().join(format!("plan/golden/{name}"));
        let got = run_sql(&p, &facts).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The product's own outputs (calls.rr's type relations, decision 0047) are not T's: they have no golden.
        for (rel, rows) in got.outputs.iter().filter(|(rel, _)| golden.join(format!("{rel}.csv")).exists()) {
            let expected = std::fs::read_to_string(golden.join(format!("{rel}.csv"))).unwrap();
            let expected: Vec<&str> = expected.lines().collect();
            if rows.iter().map(String::as_str).collect::<Vec<_>>() != expected {
                let got: BTreeSet<&str> = rows.iter().map(String::as_str).collect();
                let want: BTreeSet<&str> = expected.iter().copied().collect();
                let (missing, extra): (Vec<_>, Vec<_>) = (want.difference(&got).take(3).collect(), got.difference(&want).take(3).collect());
                failures.push(format!("{name} {rel}: {} rows, expected {}; missing {missing:?}, extra {extra:?}", rows.len(), expected.len()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The typing programs' own ingestion (list step 29: no T facts file, every chunk's statistics measured as it is
/// bound) derives what the run on T's facts derives, on T's fixture corpus, as one chunk and as several. Node ids
/// differ only in how they are written (`path:index` against T's global ids), so the own outputs are mapped to T's.
#[test]
fn own_ingestion_derives_what_t_facts_do() {
    if !t_numbering("own_ingestion_derives_what_t_facts_do") {
        return;
    }
    use crate::code_rule_dsl::typing::file_local::Binding;
    let raw = zstd::decode_all(std::fs::File::open(root().join("../facts/vtab/golden/corpus.json.zst")).unwrap()).unwrap();
    let corpus: Vec<(String, String)> = serde_json::from_slice(&raw).unwrap();
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("corpus");
    for (path, source) in &corpus {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    let facts = facts();
    let t = Connection::open(&facts).unwrap();
    let first: HashMap<String, i64> =
        t.prepare("SELECT path, min(n) FROM doc JOIN node USING (d) GROUP BY d").unwrap().query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap().map(|row| row.unwrap()).collect();
    let to_t = |line: &str| -> String {
        line.split('\t')
            .map(|field| match field.rsplit_once(':') {
                Some((path, index)) if first.contains_key(path) => (first[path] + index.parse::<i64>().unwrap()).to_string(),
                _ => field.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\t")
    };
    let read = |out: &std::path::Path, rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_else(|e| panic!("{}: {rel}: {e}", out.display()));
    for name in ["workload", "calls"] {
        let src = crate::code_rule_dsl::logic::settings::program_text(&root().join(format!("../typing/programs/{name}.rr"))).unwrap();
        let with_t = dir.join(format!("out-{name}-t"));
        let report = corpus::run(&src, &dir, Some(&facts), &with_t, "program", Binding::Cpu, 1 << 18).unwrap();
        assert!(report.rows.values().any(|&n| n > 0), "{name}: no outputs");
        // The cross-file part is one fixpoint (decision 0065): one run, converged, nothing pending.
        println!("{name}: cross-file rounds {:?}", report.rounds);
        assert_eq!(report.rounds.run, 1, "{name}: {:?}", report.rounds);
        assert!(report.rounds.converged && !report.rounds.pending, "{name}: {:?}", report.rounds);
        for chunk in [1_usize << 18, 1 << 12] {
            let own = dir.join(format!("out-{name}-own-{chunk}"));
            let got = corpus::run(&src, &dir, None, &own, "program", Binding::Cpu, chunk).unwrap();
            assert_eq!(got.files, corpus.len(), "{name}");
            for rel in report.rows.keys() {
                let mut mapped: Vec<String> = read(&own, rel).lines().map(to_t).collect();
                mapped.sort();
                let expected: Vec<String> = read(&with_t, rel).lines().map(str::to_owned).collect();
                assert_eq!(mapped.len(), expected.len(), "{name} {rel} at {chunk}-node chunks: rows");
                assert!(mapped == expected, "{name} {rel} at {chunk}-node chunks: rows differ");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// O42: the corpus runner's typing inputs and outputs, end to end. A site package is summarized into the cache
/// (a declaration that differs from what it derives is reported, not used); an application importing it reads the
/// summaries and a user declaration as `declared_ret` facts, which type its calls, and each typed node gets a
/// union term; with an empty cache the package's modules are pending, not ⊤, and the calls stay untyped.
#[test]
fn typed_corpus_reads_declarations_and_dependency_summaries() {
    use super::corpus::{Typing, run_typed};
    use crate::code_rule_dsl::typing::file_local::Binding;
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("typed");
    let write = |p: &str, text: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("site/vendor/__init__.py", "from .core import make\n\ndef version():\n    return \"1.0\"\n");
    write("site/vendor/core.py", "def make():\n    return 1\n");
    write("site/vendor-1.0.dist-info/RECORD", "vendor/__init__.py,sha256=AAA,60\nvendor/core.py,sha256=BBB,30\nvendor-1.0.dist-info/RECORD,,\n");
    write("decl/make.pyi", "# rerail.types/v1\n# module: vendor.core\ndef make() -> str: ...\n");
    write("app/main.py", "from vendor import version\nfrom vendor.core import make\nimport other\na = version()\nb = make()\nc = other.thing()\n");
    write("decl/other.pyi", "# rerail.types/v1\n# module: other\ndef thing() -> bytes | None: ...\n");
    let src = std::fs::read_to_string(root().join("../typing/programs/workload.rr")).unwrap();
    let cache = dir.join("cache");

    let summarize = Typing { summarize: Some(cache.clone()), declarations: vec![dir.join("decl/make.pyi")], ..Default::default() };
    let r = run_typed(&[&src], &dir.join("site"), None, &dir.join("out-site"), "program", Binding::Cpu, 1 << 18, &summarize).unwrap();
    assert_eq!(r.typing.summaries, 2, "{:?}", r.typing);
    assert_eq!(r.typing.conflicts, vec![("vendor.core.make".to_owned(), "int".to_owned(), "str".to_owned())]);

    let app = Typing { declarations: vec![dir.join("decl/other.pyi")], site: Some((dir.join("site"), cache.clone())), ..Default::default() };
    let r = run_typed(&[&src], &dir.join("app"), None, &dir.join("out-app"), "program", Binding::Cpu, 1 << 18, &app).unwrap();
    println!("{:?}", r.typing);
    assert!(r.typing.pending.is_empty(), "{:?}", r.typing);
    assert_eq!(r.typing.declared, 4, "version: str, make: int, thing: bytes | None");
    let types: BTreeSet<String> = std::fs::read_to_string(dir.join("out-app/type_of.csv")).unwrap().lines().map(|l| l.rsplit('\t').next().unwrap().to_owned()).collect();
    assert_eq!(types, BTreeSet::from(["str", "int", "bytes", "None"].map(String::from)));
    // One term per typed node (the three calls and the three names they are assigned to): other.thing()'s, and
    // `c`'s through it, are the closed union bytes | None.
    let x = rusqlite::Connection::open(dir.join("out-app/cross.sqlite")).unwrap();
    let nodes: i64 = x.query_row("SELECT count(*) FROM node_type", [], |r| r.get(0)).unwrap();
    assert_eq!(nodes as usize, r.typing.typed_nodes);
    let unions: Vec<i64> = x.prepare("SELECT t.arity FROM node_type n JOIN type_term t USING (tid) WHERE t.ctor = 1").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(r.typing.typed_nodes, 6);
    assert_eq!(unions, vec![2, 2]);

    let cold = Typing { site: Some((dir.join("site"), dir.join("empty-cache"))), ..Default::default() };
    let r = run_typed(&[&src], &dir.join("app"), None, &dir.join("out-cold"), "program", Binding::Cpu, 1 << 18, &cold).unwrap();
    assert_eq!(r.typing.pending, vec!["vendor".to_owned(), "vendor.core".to_owned()]);
    assert_eq!(std::fs::read_to_string(dir.join("out-cold/type_of.csv")).unwrap(), "");
    let _ = std::fs::remove_dir_all(&dir);
}

const CORPUS_STORE_CHILD: &str = "RERAIL_CORPUS_STORE_CHILD";

/// The child of `a_second_corpus_run_deletes_and_creates_its_store_again`, run without the tape cache: each run's
/// store is its own `store.sqlite`, which the next run deletes and creates again. Once a run is over nothing in the
/// process holds the store open, and the second run's symbols are its own store's.
#[test]
#[ignore = "a child process of the corpus store test"]
fn corpus_store_child() {
    let Some(dir) = std::env::var_os(CORPUS_STORE_CHILD).map(std::path::PathBuf::from) else { return };
    assert_eq!(std::env::var("RERAIL_TAPE_CACHE").as_deref(), Ok("off"));
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.input node, text
.decl name(s: symbol)
name(s) :- node(_, n, "identifier", _, _), text(n, s).
.output name
"#;
    let out = dir.join("out");
    let store = out.join("store.sqlite");
    for (i, [a, b]) in [["alpha", "beta"], ["gamma", "delta"]].into_iter().enumerate() {
        let root = dir.join(format!("corpus-{i}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), format!("{a} = {b}\n")).unwrap();
        super::corpus::run(program, &root, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut names = [a, b];
        names.sort_unstable();
        assert_eq!(std::fs::read_to_string(out.join("name.csv")).unwrap(), format!("{}\n{}\n", names[0], names[1]));
        #[cfg(target_os = "linux")]
        {
            let open: Vec<std::path::PathBuf> = std::fs::read_dir("/proc/self/fd").unwrap().filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok()).collect();
            let held: Vec<_> = open.iter().filter(|file| file.to_string_lossy().starts_with(&*store.to_string_lossy())).collect();
            assert!(held.is_empty(), "run {i} left its store open: {held:?}");
        }
    }
    std::fs::write(dir.join("done"), "").unwrap();
}

/// A process runs the corpus twice over its own store (`corpus_store_child`).
#[test]
fn a_second_corpus_run_deletes_and_creates_its_store_again() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("restore");
    std::fs::create_dir_all(&dir).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "code_rule_dsl::logic::plan::tests::corpus_store_child", "--nocapture", "--test-threads=1"])
        .env(CORPUS_STORE_CHILD, &dir)
        .env("RERAIL_TAPE_CACHE", "off")
        .status()
        .unwrap();
    assert!(status.success() && dir.join("done").exists(), "the child failed: {status}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Step 30's primitives through the corpus runner: `line(n, l)`, and literal tests on names and paths (`glob`,
/// `prefix`, `suffix`) in place of regexes.
#[test]
fn rules_test_names_paths_and_lines_without_regexes() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("strings");
    for (p, text) in [
        ("impl/sqlite/fts/manager.py", "x = 1\n_hidden = 2\nfetch_async = 3\n"),
        ("impl/base.py", "_also = 1\n"),
        ("lib/other.py", "_outside = 1\n"),
    ] {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl line(n: node, l: number)
.input doc, node, text, line
.decl private_in_impl(p: symbol, s: symbol, l: number)
private_in_impl(p, s, l) :- doc(d, p, _), glob("impl/**/*.py", p), node(d, n, "identifier", _, _), text(n, s),
    prefix("_", s), line(n, l).
.decl async_in_fts(s: symbol)
async_in_fts(s) :- doc(d, p, _), glob("impl/*/fts/**", p), node(d, n, "identifier", _, _), text(n, s), suffix("_async", s).
.output private_in_impl, async_in_fts
"#;
    let out = dir.join("out");
    let r = super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(r.files, 3);
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("private_in_impl"), "impl/base.py\t_also\t1\nimpl/sqlite/fts/manager.py\t_hidden\t2\n");
    assert_eq!(read("async_in_fts"), "fetch_async\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The tree axes (decision 0036): `inside(n, a, set)` holds every ancestor of `n` whose kind is in the set, not
/// only the nearest; `has(a, n, set)` every descendant of `a` whose kind is in the set (`core.rr`: the set tests the
/// descendant); both negate.
#[test]
fn rules_read_inside_and_has_as_tape_relations() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("axes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def outer():\n    def inner():\n        g()\n    f()\nh()\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input node, field, text
.decl fn(kind: symbol)
fn("function_definition").
.decl call_k(kind: symbol)
call_k("call").
.decl callee(c: node, s: symbol)
callee(c, s) :- node(_, c, "call", _, _), field(c, "function", x), text(x, s).
.decl fname(f: node, s: symbol)
fname(f, s) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, s).
// Every function a call is inside.
.decl call_in(c: symbol, f: symbol)
call_in(c, f) :- callee(n, c), inside(n, a, fn), fname(a, f).
// Functions that have a call to g somewhere below them.
.decl calls_g(f: symbol)
calls_g(f) :- fname(a, f), has(a, n, call_k), callee(n, "g").
// Functions with a function inside them, and without.
.decl nests(f: symbol)
nests(f) :- fname(a, f), has(a, _, fn).
.decl flat(f: symbol)
flat(f) :- fname(a, f), !has(a, _, fn).
// Calls inside no function.
.decl top(c: symbol)
top(c) :- callee(n, c), !inside(n, _, fn).
.output call_in, calls_g, top, nests, flat
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("call_in"), "f\touter\ng\tinner\ng\touter\n");
    assert_eq!(read("calls_g"), "inner\nouter\n");
    assert_eq!(read("top"), "h\n");
    assert_eq!(read("nests"), "outer\n");
    assert_eq!(read("flat"), "inner\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A measurement, not a check: "calls inside a function" asked three ways over a corpus (`RERAIL_MEASURE_CORPUS`),
/// one way per run (`RERAIL_MEASURE` = `tape`, `interval` or `recursive`), each through the corpus runner. Prints the
/// rows and each phase's CPU (user + system).
/// - `tape`: `nearest` from the transducer's preorder pass (tape arithmetic);
/// - `interval`: byte-span containment against function definitions (a Datalog join, no recursion);
/// - `recursive`: an ancestor closure over `child` (Datalog recursion).
#[test]
#[ignore = "a measurement over a corpus on disk"]
fn measure_inside_three_ways() {
    let corpus = std::path::PathBuf::from(std::env::var("RERAIL_MEASURE_CORPUS").expect("RERAIL_MEASURE_CORPUS"));
    let way = std::env::var("RERAIL_MEASURE").unwrap_or_else(|_| "tape".into());
    let head = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.input node
.decl fn(kind: symbol)
fn("function_definition").
.decl hit(c: node)
.output hit
"#;
    let body = match way.as_str() {
        // No inside test: the fixed cost of the run and of its `hit` rows.
        "baseline" => "hit(c) :- node(_, c, \"call\", _, _).\n",
        "tape" => "hit(c) :- node(_, c, \"call\", _, _), nearest(c, f, fn).\n",
        "interval" => "hit(c) :- node(d, c, \"call\", s, e), node(d, f, \"function_definition\", fs, fe), fs <= s, e <= fe.\n",
        "recursive" => ".decl child(p: node, c: node, i: number)\n.input child\n.decl up(c: node, a: node)\nup(c, p) :- node(_, c, \"call\", _, _), child(p, c, _).\nup(c, a) :- up(c, p), child(a, p, _).\nhit(c) :- up(c, f), node(_, f, \"function_definition\", _, _).\n",
        other => panic!("no way {other}"),
    };
    let binding = if std::env::var("RERAIL_MEASURE_METAL").is_ok() { crate::code_rule_dsl::typing::file_local::Binding::Metal } else { crate::code_rule_dsl::typing::file_local::Binding::Cpu };
    let out = crate::code_rule_dsl::fixtures::temp().with_extension(format!("measure-{way}"));
    let r = super::corpus::run(&format!("{head}{body}"), &corpus, None, &out, "program", binding, 1 << 18).unwrap();
    let cpu: f64 = r.phases.iter().map(|p| p.1 + p.2).sum();
    println!("MEASURE {way}: {} files, {} nodes, {} chunks, hit rows {}, CPU {cpu:.2} s, peak RSS {:.0} MB", r.files, r.nodes, r.chunks, r.rows["hit"], r.phases.iter().map(|p| p.3).fold(0.0, f64::max));
    for (name, user, sys, rss, statements) in &r.phases {
        println!("  {name:<16} {:.2} s CPU  rss {rss:.0} MB  {statements} statements", user + sys);
    }
    let _ = std::fs::remove_dir_all(&out);
}

// ---- aggregates ----

/// `join`, `sum` and aggregates inside terms, run over a corpus: join orders by its keys (numbers by value, symbols
/// by code point, a tuple key by its parts in turn), ties by the value, and gives "" of nothing; sum adds every row
/// of its body (two arguments of one length both count) and gives 0 of nothing.
#[test]
fn join_sum_and_aggregates_in_terms_run() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("aggregates");
    std::fs::create_dir_all(&dir).unwrap();
    // Identifiers, whose `text` is their name: `é` sorts after `z` by code point, and `B` before both.
    let m = (0..=10).map(|i| format!("a{i}")).collect::<Vec<_>>().join(", ");
    std::fs::write(dir.join("a.py"), format!("f(bb, a, cc)\ng()\nh(é, z, B)\nm({m})\n")).unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl child(p: node, c: node, i: number)
.input node, field, text, child
.decl callee(c: node, s: symbol)
callee(c, s) :- node(_, c, "call", _, _), field(c, "function", x), text(x, s).
.decl arg(c: node, t: symbol, i: number)
arg(c, t, i) :- node(_, c, "call", _, _), field(c, "arguments", a), child(a, s, i), text(s, t).
.decl in_order(f: symbol, j: symbol)
in_order(f, j) :- callee(c, f), j = join t by i sep ", " : { arg(c, t, i) }.
.decl call_end(f: symbol, j: symbol)
call_end(f, j) :- callee(c, f), node(_, c, "call", _, e), j = to_string(e).
.decl call_end_joined(f: symbol, j: symbol)
call_end_joined(f, j) :- callee(c, f), j = join e by s sep "," : { node(_, c, "call", s, e) }.
.decl by_text(f: symbol, j: symbol)
by_text(f, j) :- callee(c, f), j = join t sep "|" : { arg(c, t, _) }.
.decl reversed(f: symbol, j: symbol)
reversed(f, j) :- callee(c, f), j = join t by k sep "" : { arg(c, t, i), k = 0 - i }.
.decl numbers(f: symbol, j: symbol)
numbers(f, j) :- callee(c, f), j = join i sep "+" : { arg(c, _, i) }.
.decl tuple(f: symbol, j: symbol)
tuple(f, j) :- callee(c, f), j = join t by (n, k) sep "," : { arg(c, t, i), n = strlen(t), k = 0 - i }.
.decl total(f: symbol, s: number, w: number, g: number)
total(f, s, w, g) :- callee(c, f), s = sum i : { arg(c, _, i) }, w = sum n : { arg(c, t, _), n = strlen(t) },
    g = sum strlen(t) - 1 : { arg(c, t, _) }.
.decl ccn(f: symbol, k: number)
ccn(f, 10 * count : { arg(c, _, _) } + sum i : { arg(c, _, i) }) :- callee(c, f).
.decl more(f: symbol)
more(f) :- callee(c, f), 1 + count : { arg(c, _, _) } > 3.
.output in_order, call_end, call_end_joined, by_text, reversed, numbers, tuple, total, ccn, more
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let a = |order: &mut dyn Iterator<Item = i32>, sep: &str| order.map(|i| format!("a{i}")).collect::<Vec<_>>().join(sep);
    let mut text_order: Vec<String> = (0..=10).map(|i| format!("a{i}")).collect();
    text_order.sort();
    assert_eq!(read("in_order"), format!("f\tbb, a, cc\ng\t\nh\té, z, B\nm\t{}\n", a(&mut (0..=10), ", ")));
    // An input read without its constant column (`kind`), which comes before x and k.
    assert_eq!(read("call_end_joined"), read("call_end"));
    assert_eq!(read("by_text"), format!("f\ta|bb|cc\ng\t\nh\tB|z|é\nm\t{}\n", text_order.join("|")));
    assert_eq!(read("reversed"), format!("f\tccabb\ng\t\nh\tBzé\nm\t{}\n", a(&mut (0..=10).rev(), "")));
    let numbers = (0..=10).map(|i| i.to_string()).collect::<Vec<_>>().join("+");
    assert_eq!(read("numbers"), format!("f\t0+1+2\ng\t\nh\t0+1+2\nm\t{numbers}\n"));
    // By length, then by position from the end: a (1), cc (2, -2), bb (2, 0); a9 … a0 (2), a10 (3).
    assert_eq!(read("tuple"), format!("f\ta,cc,bb\ng\t\nh\tB,z,é\nm\t{},a10\n", a(&mut (0..=9).rev(), ",")));
    // bb and cc have one length: both count in w.
    assert_eq!(read("total"), "f\t3\t5\t2\ng\t0\t0\t0\nh\t3\t3\t0\nm\t55\t23\t12\n");
    assert_eq!(read("ccn"), "f\t33\ng\t0\nh\t33\nm\t165\n");
    assert_eq!(read("more"), "f\nh\nm\n");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- functions ----

/// Runs `program` over one Python file, pure SQL (`""`) and on the native set engine (`"program"`): the outputs
/// `rels`, which both must give alike, and how many components the native run solved natively.
fn run_both(name: &str, program: &str, rels: &[&str]) -> (BTreeMap<String, String>, usize) {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    let mut got = Vec::new();
    let mut native = 0;
    for modes in ["", "program"] {
        let out = dir.join(format!("out-{}", if modes.is_empty() { "sql" } else { modes }));
        let r = super::corpus::run(program, &dir, None, &out, modes, crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{modes:?}: {e}"));
        native = r.native;
        got.push(rels.iter().map(|rel| (rel.to_string(), std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap())).collect::<BTreeMap<_, _>>());
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got[0], got[1], "SQL and native");
    (got.remove(0), native)
}

/// Soufflé's `to_number` (decimal, `0x`, `0b`, C's leading space and trailing text; no value where Soufflé stops
/// with an error), `max`, `min`, `%` (C's sign; no value for `% 0`), `cat` of any arity, and `@lower` as
/// JavaScript's `toLowerCase` (Unicode, final sigma). A function with no value gives no row, also through a binding
/// read only by a negation.
#[test]
fn souffle_functions_run() {
    let program = r#"
.decl s(t: symbol)
s("12"). s(" -7px"). s("0x1F"). s("-0b101"). s("abc"). s(""). s("99999999999999999999").
.decl num(t: symbol, n: number)
num(t, to_number(t)) :- s(t).
.decl small(n: number)
small(1). small(12).
.decl unlisted(t: symbol)
unlisted(t) :- s(t), k = to_number(t), !small(k).
.decl pair(a: number, b: number)
pair(7, 3). pair(-7, 3). pair(3, 0).
.decl ar(a: number, b: number, hi: number, lo: number)
ar(a, b, max(a, b), min(a, b)) :- pair(a, b).
.decl md(a: number, b: number, m: number)
md(a, b, m) :- pair(a, b), m = a % b.
.decl w(a: symbol, b: symbol)
w("x", "y"). w("ÀÉ İ", "ΑΣ").
.decl c(s: symbol)
c(cat(a, "-", b, "-", a)) :- w(a, b).
.decl low(s: symbol)
low(@lower(cat(a, " ", b))) :- w(a, b).
.output num, unlisted, ar, md, c, low
"#;
    let (got, _) = run_both("functions", program, &["num", "unlisted", "ar", "md", "c", "low"]);
    assert_eq!(got["num"], " -7px\t-7\n-0b101\t-5\n0x1F\t31\n12\t12\n");
    assert_eq!(got["unlisted"], " -7px\n-0b101\n0x1F\n");
    assert_eq!(got["ar"], "-7\t3\t3\t-7\n3\t0\t3\t0\n7\t3\t7\t3\n");
    assert_eq!(got["md"], "-7\t3\t-1\n7\t3\t1\n");
    assert_eq!(got["c"], "x-y-x\nÀÉ İ-ΑΣ-ÀÉ İ\n");
    assert_eq!(got["low"], "x y\nàé i\u{307} ας\n");
}

/// `@dot("", b)` is `b` (`core.rr`), built and split: a split of a bound whole gives `""` and the whole, and each
/// cut at a "." with a part before it; with a bound part, the other. Recursive (the native set engine) and not.
#[test]
fn dot_of_the_empty_name_is_the_name() {
    let program = r#"
.decl part(s: symbol)
part(""). part("a"). part("b.c").
.decl joined(m: symbol, x: symbol, q: symbol)
joined(m, x, @dot(m, x)) :- part(m), part(x).
.decl whole(q: symbol)
whole("a.b.c"). whole("x"). whole(".y").
.decl split(q: symbol, m: symbol, x: symbol)
split(q, m, x) :- whole(q), q = @dot(m, x).
.decl after(q: symbol, m: symbol, x: symbol)
after(q, m, x) :- whole(q), part(m), q = @dot(m, x).
.decl tail(x: symbol)
tail("b.c"). tail("x"). tail("y").
.decl before(q: symbol, m: symbol, x: symbol)
before(q, m, x) :- whole(q), tail(x), q = @dot(m, x).
.decl pre(q: symbol)
pre(q) :- whole(q).
pre(m) :- pre(q), q = @dot(m, x), x != "".
.output joined, split, after, before, pre
"#;
    let (got, native) = run_both("dot", program, &["joined", "split", "after", "before", "pre"]);
    assert!(native > 0, "the native set engine solved a component");
    assert_eq!(got["joined"], "\t\t\n\ta\ta\n\tb.c\tb.c\na\t\ta.\na\ta\ta.a\na\tb.c\ta.b.c\nb.c\t\tb.c.\nb.c\ta\tb.c.a\nb.c\tb.c\tb.c.b.c\n");
    assert_eq!(got["split"], ".y\t\t.y\na.b.c\t\ta.b.c\na.b.c\ta\tb.c\na.b.c\ta.b\tc\nx\t\tx\n");
    assert_eq!(got["after"], ".y\t\t.y\na.b.c\t\ta.b.c\na.b.c\ta\tb.c\nx\t\tx\n");
    assert_eq!(got["before"], "a.b.c\ta\tb.c\nx\t\tx\n");
    assert_eq!(got["pre"], "\n.y\na\na.b\na.b.c\nx\n");
}

// ---- violations ----

/// A `.violation` at a `loc` (`core.rr`): a node's finding is at its document's path and line; a `@place(path,
/// line)`'s at that path and line, line 0 for the file itself and a path that need not exist; a place passed through
/// another relation keeps its path and line.
#[test]
fn violations_at_places_are_found_at_their_path_and_line() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("places");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "f()\n\ng()\n").unwrap();
    let program = r#"
.policy "t:places" revision 1
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl doc(d: number, path: symbol, module: symbol)
.decl line(n: node, l: number)
.input node, doc, line
.violation LONG(at: loc, what: symbol) code "FILE_LENGTH" "{what}"
    suppress ruff "E501"
LONG(n, "call") :- node(_, n, "call", _, _).
LONG(@place(p, 0), "file") :- doc(_, p, _).
LONG(@place("missing.json", 3), "missing") :- doc(_, _, _).
.decl spot(d: number, at: loc)
spot(d, @place(p, 7)) :- doc(d, p, _).
.violation PIN(at: span) "pin"
PIN(s) :- spot(_, s).
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(
        read("violation"),
        "t:places\tLONG\tFILE_LENGTH\ta.py\t0\nt:places\tLONG\tFILE_LENGTH\ta.py\t1\nt:places\tLONG\tFILE_LENGTH\ta.py\t3\nt:places\tLONG\tFILE_LENGTH\tmissing.json\t3\nt:places\tPIN\tPIN\ta.py\t7\n"
    );
    // The violation's own rows show a place as one.
    assert_eq!(read("PIN"), "@place(\"a.py\", 7)\n");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- tree axes and node sets ----

/// The axes over a set of nodes (`fnode`) or of kinds rules derive (`fk`), read as rules; `!nearest` with the
/// ancestor named, `!within_stop`, `!earlier` and `!split_dot`; `precedes_stop` and `next_sibling` over `child`.
#[test]
fn axes_read_node_sets_and_negate() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("node-sets");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def a():\n    pass\ndef b():\n    def inner():\n        g()\n    f()\ndef skip():\n    def deep():\n        k()\nh()\nx = 1\n").unwrap();
    std::fs::write(dir.join("b.py"), "z()\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl child(p: node, c: node, i: number)
.decl next_sibling(a: node, b: node)
.decl docof(n: node, d: number)
.input node, field, text, child, next_sibling, docof
.decl fn(kind: symbol)
fn("function_definition").
.decl fname(f: node, s: symbol)
fname(f, s) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, s).
.decl stmt_call(e: node, c: symbol)
stmt_call(e, c) :- node(_, e, "expression_statement", _, _), child(e, x, 0), field(x, "function", y), text(y, c).
.decl label(n: node, s: symbol)
label(n, s) :- fname(n, s).
label(n, s) :- stmt_call(n, s).
.decl top_fn(f: node, s: symbol)
top_fn(f, s) :- fname(f, s), !nearest(f, _, fn).
// A set of nodes: the functions but inner; and a set of kinds that a rule derives.
.decl fnode(n: node)
fnode(n) :- fname(n, x), x != "inner".
.decl fk(k: symbol)
fk(k) :- node(_, f, k, _, _), field(f, "name", _).
.decl skipnode(n: node)
skipnode(n) :- fname(n, "skip").
.decl near_d(c: symbol, f: symbol)
near_d(c, f) :- stmt_call(e, c), nearest(e, a, fnode), fname(a, f).
.decl in_d(c: symbol, f: symbol)
in_d(c, f) :- stmt_call(e, c), inside(e, a, fnode), fname(a, f).
.decl any_d(c: symbol)
any_d(c) :- stmt_call(e, c), inside(e, _, fnode).
.decl top_d(c: symbol)
top_d(c) :- stmt_call(e, c), !nearest(e, _, fnode).
.decl has_d(f: symbol, g: symbol)
has_d(f, g) :- fname(a, f), has(a, n, fnode), fname(n, g).
.decl hasany_d(f: symbol)
hasany_d(f) :- fname(a, f), has(a, _, fnode).
.decl nohas_d(f: symbol)
nohas_d(f) :- fname(a, f), !has(a, _, fnode).
.decl near_k(c: symbol, f: symbol)
near_k(c, f) :- stmt_call(e, c), nearest(e, a, fk), fname(a, f).
.decl has_k(f: symbol)
has_k(f) :- fname(a, f), has(a, _, fk).
.decl has_s(f: symbol, g: symbol)
has_s(f, g) :- fname(a, f), has(a, n, fn), fname(n, g).
.decl stmts_in(f: symbol, c: symbol)
stmts_in(f, c) :- top_fn(a, f), has(a, n, "expression_statement"), stmt_call(n, c).
.decl not_near_b(c: symbol)
not_near_b(c) :- stmt_call(e, c), top_fn(a, "b"), !nearest(e, a, fn).
.decl not_near_b_d(c: symbol)
not_near_b_d(c) :- stmt_call(e, c), top_fn(a, "b"), !nearest(e, a, fnode).
.decl not_own_b(c: symbol)
not_own_b(c) :- stmt_call(e, c), top_fn(a, "b"), !within_stop(e, a, fn).
.decl not_own_b_d(c: symbol)
not_own_b_d(c) :- stmt_call(e, c), top_fn(a, "b"), !within_stop(e, a, fnode).
.decl after_a(s: symbol)
after_a(s) :- top_fn(m, "a"), precedes_stop(m, n, fn), label(n, s).
.decl after_a_skip(s: symbol)
after_a_skip(s) :- top_fn(m, "a"), precedes_stop(m, n, skipnode), label(n, s).
.decl after_b(s: symbol)
after_b(s) :- top_fn(m, "b"), precedes_stop(m, n, "expression_statement"), label(n, s).
.decl not_after_a(s: symbol)
not_after_a(s) :- top_fn(m, "a"), label(n, s), !precedes_stop(m, n, fn).
.decl nxt(x: symbol, y: symbol)
nxt(x, y) :- next_sibling(m, n), label(m, x), label(n, y).
.decl not_before(x: symbol, y: symbol)
not_before(x, y) :- stmt_call(m, x), stmt_call(n, y), !earlier(m, n).
.decl q(s: symbol)
q("os.path"). q("os.path.join").
.decl parts(m: symbol, x: symbol)
parts(m, x) :- q(s), split_dot(s, m, x).
.decl in_os(x: symbol)
in_os(x) :- q(s), split_dot(s, "os", x).
.decl of_join(m: symbol)
of_join(m) :- q(s), split_dot(s, m, "join").
.decl not_split(s: symbol)
not_split(s) :- q(s), !split_dot(s, "os", "path").
.decl not_split2(s: symbol)
not_split2(s) :- q(s), !split_dot(s, "os", "path.join").
.output near_d, in_d, any_d, top_d, has_d, hasany_d, nohas_d, near_k, has_k, has_s, stmts_in, not_near_b, not_near_b_d
.output not_own_b, not_own_b_d, after_a, after_a_skip, after_b, not_after_a, nxt, not_before, parts, in_os, of_join, not_split, not_split2
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let expected = [
        // g's nearest function is inner, which fnode leaves out.
        ("near_d", "f\tb\ng\tb\nk\tdeep\n"),
        ("in_d", "f\tb\ng\tb\nk\tdeep\nk\tskip\n"),
        ("any_d", "f\ng\nk\n"),
        ("top_d", "h\nz\n"),
        // `has` tests the set on the node below: b has only inner below it.
        ("has_d", "skip\tdeep\n"),
        ("hasany_d", "skip\n"),
        ("nohas_d", "a\nb\ndeep\ninner\n"),
        ("near_k", "f\tb\ng\tinner\nk\tdeep\n"),
        ("has_k", "b\nskip\n"),
        ("has_s", "b\tinner\nskip\tdeep\n"),
        ("stmts_in", "b\tf\nb\tg\nskip\tk\n"),
        ("not_near_b", "g\nh\nk\nz\n"),
        ("not_near_b_d", "h\nk\nz\n"),
        ("not_own_b", "g\nh\nk\nz\n"),
        ("not_own_b_d", "h\nk\nz\n"),
        ("after_a", "b\n"),
        ("after_a_skip", "b\nskip\n"),
        ("after_b", "h\nskip\n"),
        ("not_after_a", "a\ndeep\nf\ng\nh\ninner\nk\nskip\nz\n"),
        ("nxt", "a\tb\nb\tskip\ninner\tf\nskip\th\n"),
        ("not_before", "f\tf\nf\tg\nf\tz\ng\tg\ng\tz\nh\tf\nh\tg\nh\th\nh\tk\nh\tz\nk\tf\nk\tg\nk\tk\nk\tz\nz\tf\nz\tg\nz\th\nz\tk\nz\tz\n"),
        ("parts", "os\tpath\nos.path\tjoin\n"),
        ("in_os", "path\n"),
        ("of_join", "os.path\n"),
        ("not_split", "os.path.join\n"),
        ("not_split2", "os.path\nos.path.join\n"),
    ];
    let wrong: Vec<String> = expected.iter().filter(|(rel, rows)| read(rel) != *rows).map(|(rel, rows)| format!("{rel}: {:?}, expected {rows:?}", read(rel))).collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An aggregate whose body is one built-in the lowering writes out (`within`, `earlier`) counts its rows: the
/// built-in is no relation the aggregate could read in place.
#[test]
fn aggregates_over_one_built_in_count_its_rows() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("agg-builtin");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def f():\n    g(1)\n    h()\ndef e():\n    pass\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl docof(n: node, d: number)
.decl child(p: node, c: node, i: number)
.input node, field, text, docof, child
.decl fname(f: node, s: symbol)
fname(f, s) :- node(_, f, "function_definition", _, _), field(f, "name", x), text(x, s).
.decl below(s: symbol, k: number)
below(s, k) :- fname(f, s), k = count : { within(n, f) }.
.decl after(s: symbol, k: number)
after(s, k) :- fname(f, s), k = count : { earlier(f, n) }.
.output below, after
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let rows = |rel: &str| read(rel).lines().map(|l| l.split('\t').map(str::to_owned).collect::<Vec<_>>()).collect::<Vec<_>>();
    let count = |rel: &str, s: &str| rows(rel).iter().find(|r| r[0] == s).map(|r| r[1].parse::<u32>().unwrap());
    // f's subtree has more nodes than e's, and e comes after f, so fewer nodes follow it.
    assert!(count("below", "f").unwrap() > count("below", "e").unwrap(), "{}", read("below"));
    assert!(count("after", "f").unwrap() > count("after", "e").unwrap(), "{}", read("after"));
    assert!(count("below", "e").unwrap() > 0, "{}", read("below"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- recursion ----

/// Recursion that makes values bounded by the data runs to its fixpoint: a count along a call's arguments in
/// child order, their text joined in that order, a counter below a bound, and a text cut shorter at each step;
/// and negated conjunctions run as their helper relations, with a variable shared, a shared one only tested, an
/// alternative each, and none.
#[test]
fn bounded_value_recursion_and_negated_conjunctions_run() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("recursion");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "f(x, y, z)\nw = os.path.join\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.input node, child, text
.decl arg_at(c: node, a: node, k: number)
arg_at(c, a, 0) :- node(_, c, "argument_list", _, _), child(c, a, 0).
arg_at(c, b, k + 1) :- arg_at(c, a, k), child(c, a, i), child(c, b, i + 1).
.decl arg_pos(s: symbol, k: number)
arg_pos(s, k) :- arg_at(_, a, k), text(a, s).
.decl pos(s: symbol, k: number)
pos(s, k) :- arg_pos(s, k).
.decl joined(c: node, i: number, v: symbol)
joined(c, 0, "") :- node(_, c, "argument_list", _, _).
joined(c, i + 1, cat(v, w)) :- joined(c, i, v), child(c, a, i), text(a, w).
.decl whole(v: symbol)
whole(v) :- joined(c, i, v), !child(c, _, i).
.decl up(x: number)
up(1).
up(x + 1) :- up(x), x < 4.
.decl dotted(s: symbol)
dotted("os.path.join").
.decl rest(s: symbol, i: number)
rest(t, 0) :- dotted(t).
rest(s, i + 1) :- rest(t, i), match("[^.]*\\.(?<s>.*)", t).
.decl first(s: symbol)
first(s) :- arg_pos(s, _), !(arg_pos(s, k), k > 0).
.decl mid(s: symbol)
mid(s) :- arg_pos(s, k), !(k = 0 ; k = 2).
.decl none(s: symbol)
none(s) :- arg_pos(s, 0), !(arg_pos(_, 5), text(_, _)).
.decl gone(s: symbol)
gone(s) :- arg_pos(s, 0), !(arg_pos(_, 2)).
.output pos, whole, up, rest, first, mid, none, gone
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("pos"), "x\t0\ny\t1\nz\t2\n");
    assert_eq!(read("whole"), "xyz\n");
    assert_eq!(read("up"), "1\n2\n3\n4\n");
    assert_eq!(read("rest"), "join\t2\nos.path.join\t0\npath.join\t1\n");
    assert_eq!(read("first"), "x\n");
    assert_eq!(read("mid"), "y\n");
    assert_eq!(read("none"), "x\n");
    assert_eq!(read("gone"), "");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A negated conjunction is correlated on a rule variable its aggregate reads, or that a built-in in it only tests
/// (`split_dot`); and a regex group of a string the regex does not match is no value, so a negation over it gives
/// no row, on both the SQL and the native paths.
#[test]
fn negations_over_aggregates_built_ins_and_missing_groups() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("negations");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "f(1, 2, 3)\ng(1)\nos.path.join\nos.sep\n").unwrap();
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl child(p: node, c: node, i: number)
.decl text(n: node, s: symbol)
.decl name(n: node, s: symbol)
.input node, field, child, text, name
.decl callee(c: node, f: symbol)
callee(c, f) :- node(_, c, "call", _, _), field(c, "function", x), text(x, f).
.decl arg(c: node, a: node)
arg(c, a) :- callee(c, _), field(c, "arguments", l), child(l, a, _).
// Calls with at most two arguments.
.decl short(f: symbol)
short(f) :- callee(c, f), !(count : { arg(c, _) } > 2).
.decl attr(q: symbol)
attr(q) :- node(_, n, "attribute", _, _), name(n, q).
.decl keyword(x: symbol)
keyword("sep").
// Dotted names whose last segment is no keyword.
.decl plain(q: symbol)
plain(q) :- attr(q), !(split_dot(q, m, x), keyword(x)).
// Names with no `.`-separated tail that is a keyword; `g` has no tail, so it is kept.
.decl untailed(s: symbol)
untailed(s) :- callee(_, s), match("(?<t>[a-z]+)\\.(?<u>[a-z]+)", s), !keyword(u).
.decl tailless(s: symbol)
tailless(s) :- callee(_, s), !keyword(@rx_group(s, "^(?:[a-z]+\\.(?<u>[a-z]+))$", "u")).
.output short, plain, untailed, tailless
"#;
    for modes in ["", "program"] {
        let out = dir.join(format!("out{modes}"));
        super::corpus::run(program, &dir, None, &out, modes, crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
        assert_eq!(read("short"), "g\n", "{modes}");
        assert_eq!(read("plain"), "os.path\nos.path.join\n", "{modes}");
        assert_eq!(read("untailed"), "", "{modes}");
        assert_eq!(read("tailless"), "", "{modes}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `program` over `files` (path, source) by the corpus runner; each of `outputs` as written.
fn corpus_outputs(name: &str, files: &[(&str, &str)], program: &str, outputs: &[&str]) -> Vec<String> {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    let _ = std::fs::remove_dir_all(&dir);
    for (path, source) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, source).unwrap();
    }
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let got = outputs.iter().map(|rel| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default()).collect();
    let _ = std::fs::remove_dir_all(&dir);
    got
}

/// A relation given both facts and rules holds both: its facts are not lost to its rules' rows.
#[test]
fn a_relation_of_facts_and_rules_keeps_its_facts() {
    let program = r#"
.decl str(n: node, s: symbol)
.decl text(n: node, s: symbol)
.input str, text
.decl other(x: symbol)
other("SyncX").
.decl pat(x: symbol)
pat("Sequence").
pat(x) :- other(x).
.decl hit(s: symbol)
hit(c) :- str(_, c), pat(x), word(x, c).
.decl pair(y: symbol, x: symbol)
pair("legacy", "a").
pair(y, y) :- text(_, y), prefix("le", y).
.decl joined(y: symbol, x: symbol)
joined(y, x) :- pair(y, x), text(_, y).
.output hit, joined
"#;
    let got = corpus_outputs("facts-and-rules", &[("a.py", "def f(a: \"Sequence\", b: \"SyncX\"):\n    legacy = lemon\n")], program, &["hit", "joined"]);
    assert_eq!(got[0], "Sequence\nSyncX\n");
    assert_eq!(got[1], "legacy\ta\nlegacy\tlegacy\nlemon\tlemon\n");
}

/// `max t : { r(d, x, t, _, _) }` groups by the variables bound outside it (`d`, `x`), not by the wildcards.
#[test]
fn an_aggregate_groups_by_its_bound_variables_not_its_wildcards() {
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.input node
.decl def(d: number, x: symbol, t: number, k: symbol, l: number)
def(d, "f", s, "function", 1) :- node(d, _, "function_definition", s, _).
def(d, "f", s, "assign", 2) :- node(d, _, "assignment", s, _).
.decl last(x: symbol, s: number, k: symbol)
last(x, s, k) :- def(d, x, s, k, _), s = max t : { def(d, x, t, _, _) }.
.decl first(x: symbol, s: number, k: symbol)
first(x, s, k) :- def(d, x, s, k, _), s = min t : { def(d, x, t, _, _) }.
.output last, first
"#;
    let got = corpus_outputs("aggregate-wildcards", &[("a.py", "def f():\n    pass\nf = 1\n")], program, &["last", "first"]);
    assert_eq!(got[0], "f\t18\tassign\n");
    assert_eq!(got[1], "f\t0\tfunction\n");
}

/// A variable named `top` is a variable where an atom binds it, not the lattice's top: `child(p, top, _)` joins as
/// `child(p, c, _)` does.
#[test]
fn a_variable_named_top_is_a_variable_where_an_atom_binds_it() {
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.input node, child
.decl lists(n: node)
lists(n) :- node(_, n, "argument_list", _, _).
.decl via(p: node, c: node)
via(p, c) :- lists(p), child(p, c, _).
.decl direct(p: node, c: node)
direct(p, top) :- node(_, p, "argument_list", _, _), child(p, top, _).
.decl kids(p: node, c: node)
kids(p, c) :- child(p, c, _).
.decl grand(a: node, c: node)
grand(a, c) :- kids(a, b), kids(b, c).
.decl chained(a: node, c: node)
chained(a, top) :- child(a, b, _), child(b, top, _).
.output via, direct, grand, chained
"#;
    let got = corpus_outputs("child-chain", &[("a.py", "class Base(SqlRenderable):\n    pass\n")], program, &["via", "direct", "grand", "chained"]);
    assert!(!got[0].is_empty(), "{got:?}");
    assert_eq!(got[1], got[0]);
    assert!(!got[2].is_empty(), "{got:?}");
    assert_eq!(got[3], got[2]);
}

/// `s` as a `.rr` string literal (`rr::syntax`'s escapes: `\"`, `\\`, `\n`, `\t`, `\r`, `\u{…}`).
fn rr_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out + "\""
}

/// W's value of `sql`, a scalar query, in the SQLite its relational stages run (with its `rx_*` functions), over a
/// table `t(n, value)` of `rows`, `n` their positions from 1.
fn w_value(rows: &[&str], sql: &str, params: &[&str]) -> String {
    let db = Connection::open_in_memory().unwrap();
    crate::code_rule_dsl::waves::relational::install_regex_functions(&db).unwrap();
    db.execute_batch("CREATE TABLE t(n INTEGER PRIMARY KEY, value TEXT)").unwrap();
    for r in rows {
        db.execute("INSERT INTO t(value) VALUES (?1)", [r]).unwrap();
    }
    db.query_row(sql, rusqlite::params_from_iter(params), |r| r.get::<_, rusqlite::types::Value>(0))
        .map(|v| match v {
            rusqlite::types::Value::Text(s) => s,
            rusqlite::types::Value::Integer(i) => i.to_string(),
            other => panic!("{other:?}"),
        })
        .unwrap()
}

/// `rels` of `src` by plain SQL over text symbols (`run_sql`) and by the corpus runner over atoms (`plan::lower`, its
/// `sym_text`/`sym_atom`), as the runner writes them (tab-separated rows).
fn on_both_paths(name: &str, src: &str, rels: &[&str]) -> [Vec<String>; 2] {
    let text = run_sql(&parse(src).unwrap_or_else(|e| panic!("{e}")), &facts()).unwrap_or_else(|e| panic!("{e}"));
    let text = rels.iter().map(|r| text.outputs.get(*r).map(|rows| rows.iter().map(|row| row.clone() + "\n").collect()).unwrap_or_default()).collect();
    [text, corpus_outputs(name, &[("a.py", "x = 1\n")], src, rels)]
}

/// `join` and `@json_str` rebuild the messages W's signed-list stages sign (`data-layer-post-op-side-effects`'s
/// `'{"allowlist":'||json_group_array(value ORDER BY value)||…`): quotes, backslashes, control characters, non-ASCII
/// (kept, as `json_group_array` keeps it) and astral characters; duplicates kept by a key that tells them apart
/// (`by (v, n)`), none ("[]"). `join` orders numbers by value and symbols by code point.
#[test]
fn join_and_json_str_rebuild_ws_signed_messages() {
    let entries = ["b", "a\"q", "back\\slash", "tab\tnl\nret\r", "\u{1}\u{1f}\u{7f}", "é ü 中", "😀", "Z", "", "\u{2028}", "a"];
    let registry = ["b", "a", "b", "é"];
    let w = |rows: &[&str], linter: &str| {
        let sql = format!("SELECT '{{\"allowlist\":'||coalesce((SELECT json_group_array(value) FROM (SELECT value FROM t ORDER BY value)),'[]')||',\"linter\":\"{linter}\",\"version\":1}}'");
        w_value(rows, &sql, &[])
    };
    let facts = |rel: &str, rows: &[&str]| rows.iter().enumerate().map(|(i, e)| format!("{rel}({}, {}).\n", i + 1, rr_str(e))).collect::<String>();
    let src = format!(
        r#".decl entry(n: number, e: symbol)
{}.decl reg(n: number, v: symbol)
{}.decl none(n: number, v: symbol)
none(n, v) :- reg(n, v), n < 0.
.decl unit(x: number)
unit(0).
.decl msg(l: symbol, m: symbol)
msg("surface", cat("{{\"allowlist\":[", cat(b, "],\"linter\":\"post_op_surface\",\"version\":1}}"))) :- unit(0),
    b = join q by e sep "," : {{ entry(_, e), q = @json_str(e) }}.
msg("runtime", cat("{{\"allowlist\":[", cat(b, "],\"linter\":\"post_op_runtime_allowlist\",\"version\":1}}"))) :- unit(0),
    b = join q by (v, n) sep "," : {{ reg(n, v), q = @json_str(v) }}.
msg("none", cat("{{\"allowlist\":[", cat(b, "],\"linter\":\"none\",\"version\":1}}"))) :- unit(0),
    b = join q by v sep "," : {{ none(_, v), q = @json_str(v) }}.
.decl w(l: symbol, m: symbol)
w("surface", {}).
w("runtime", {}).
w("none", {}).
.decl same(l: symbol)
same(l) :- msg(l, m), w(l, m).
.decl num(k: number, x: symbol)
num(10, "ten"). num(9, "nine"). num(100, "hundred"). num(9, "neuf").
.decl by_number(s: symbol)
by_number(s) :- unit(0), s = join x by k sep " " : {{ num(k, x) }}.
.output same, msg, by_number
"#,
        facts("entry", &entries),
        facts("reg", &registry),
        rr_str(&w(&entries, "post_op_surface")),
        rr_str(&w(&registry, "post_op_runtime_allowlist")),
        rr_str(&w(&[], "none")),
    );
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("join-json", &src, &["same", "msg", "by_number"])) {
        assert_eq!(got[0], "none\nruntime\nsurface\n", "{path}: {}", got[1]);
        assert_eq!(got[2], "neuf nine ten hundred\n", "{path}");
    }
}

/// `@ed25519_ok` is W's `verify_ed25519`: RFC 8032's first vector verifies, a changed message or a malformed key or
/// signature does not, never an error (integrated-compatibility.test.ts, "verifies detached signatures without
/// treating malformed signatures as runtime failures"); and post-op-side-effects' reproducer gives 0.
#[test]
fn ed25519_ok_is_ws_verify_ed25519() {
    use base64::Engine as _;
    let key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    let sig = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
    let sig: Vec<u8> = (0..sig.len()).step_by(2).map(|i| u8::from_str_radix(&sig[i..i + 2], 16).unwrap()).collect();
    let sig = base64::engine::general_purpose::STANDARD.encode(sig);
    let src = format!(
        r#".decl unit(x: number)
unit(0).
.decl ok(c: symbol, v: number)
ok("rfc", v) :- unit(0), v = @ed25519_ok("{key}", "", "{sig}").
ok("changed", v) :- unit(0), v = @ed25519_ok("{key}", "changed", "{sig}").
ok("invalid", v) :- unit(0), v = @ed25519_ok("invalid", "", "invalid").
ok("unpadded", v) :- unit(0), v = @ed25519_ok("{key}", "", "{}").
.decl repro(v: number)
repro(v) :- m = join q by e sep "," : {{ e = "a", q = @json_str(e) }},
    v = @ed25519_ok("7e9776d87292ab633115bd6c07892a4bf56450fffb2dfcf13b3eba1866b9fbb6", cat("[", cat(m, "]")), "AAAA").
.output ok, repro
"#,
        sig.trim_end_matches('='),
    );
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("ed25519", &src, &["ok", "repro"])) {
        assert_eq!(got[0], "changed\t0\ninvalid\t0\nrfc\t1\nunpadded\t0\n", "{path}");
        assert_eq!(got[1], "0\n", "{path}");
    }
}

/// `@rx_replace(s, re, by)` is W's SQL `rx_replace`: every match replaced, `$1`, `$name`, `${name}` and `$$` as the
/// `regex` crate reads them (`$1x` is the group named `1x`, absent: empty), empty matches included.
#[test]
fn rx_replace_is_ws_rx_replace() {
    let cases = [
        ("a  b\tc\n", r"\s+", ""),
        ("x=1, y=2", r"(?P<k>\w)=(\d)", "$2$k"),
        ("abc", "b", "$$"),
        ("aaa", "a*", "-"),
        ("pkg.mod.name", r"(?:\.[^.]+){1}$", ""),
        ("cost 5", r"(\d)", "${1}0"),
        ("a1b", r"(\d)", "$1x"),
        ("AB+/=c-d", "[^A-Za-z0-9+/=]", ""),
        ("é中😀", ".", "<$0>"),
    ];
    let mut src = String::from(".decl replacing(i: number, s: symbol, re: symbol, by: symbol)\n.decl w(i: number, v: symbol)\n");
    for (i, (s, re, by)) in cases.iter().enumerate() {
        let w = w_value(&[], "SELECT rx_replace(?1, ?2, ?3)", &[*s, *re, *by]);
        src += &format!("replacing({i}, {}, {}, {}).\nw({i}, {}).\n", rr_str(s), rr_str(re), rr_str(by), rr_str(&w));
    }
    src += ".decl got(i: number, v: symbol)\ngot(i, @rx_replace(s, re, by)) :- replacing(i, s, re, by).\n";
    src += ".decl differs(i: number)\ndiffers(i) :- got(i, v), !w(i, v).\ndiffers(i) :- w(i, v), !got(i, v).\n";
    src += ".decl cases(k: number)\ncases(k) :- replacing(0, _, _, _), k = count : { got(_, _) }.\n.output differs, cases\n";
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("rx-replace", &src, &["differs", "cases"])) {
        assert_eq!(got[0], "", "{path}");
        assert_eq!(got[1], format!("{}\n", cases.len()), "{path}");
    }
}

/// `@nfkc(s)` is CPython's identifier normalization (PEP 3131): CPython 3.13.18's `unicodedata.normalize("NFKC", s)`,
/// which is also the `Name` id `ast.parse` gives each of these (but `x²`, no identifier): ligatures, letterlike and
/// mathematical letters, full- and half-width forms, a decomposed accent composed, titlecase digraphs, the Angstrom and
/// micro signs, a long s with a dot below; ASCII unchanged.
#[test]
fn nfkc_is_cpythons_identifier_normalization() {
    let cases = [
        ("\u{fb01}le", "file"),
        ("\u{210c}ello", "Hello"),
        ("x\u{b2}", "x2"),
        ("\u{ff21}b", "Ab"),
        ("\u{ff76}", "\u{30ab}"),
        ("e\u{301}t\u{e9}", "\u{e9}t\u{e9}"),
        ("ascii", "ascii"),
        ("\u{1c5}x", "D\u{17e}x"),
        ("\u{fb03}", "ffi"),
        ("\u{212b}", "\u{c5}"),
        ("\u{1d431}y", "xy"),
        ("\u{b5}s", "\u{3bc}s"),
        ("\u{1e9b}\u{323}", "\u{1e69}"),
    ];
    let mut src = String::from(".decl written(i: number, s: symbol)\n.decl cpython(i: number, v: symbol)\n");
    for (i, (s, v)) in cases.iter().enumerate() {
        src += &format!("written({i}, {}).\ncpython({i}, {}).\n", rr_str(s), rr_str(v));
    }
    src += ".decl got(i: number, v: symbol)\ngot(i, @nfkc(s)) :- written(i, s).\n";
    src += ".decl differs(i: number)\ndiffers(i) :- got(i, v), !cpython(i, v).\ndiffers(i) :- cpython(i, v), !got(i, v).\n";
    src += ".decl cases(k: number)\ncases(k) :- written(0, _), k = count : { got(_, _) }.\n.output differs, cases\n";
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("nfkc", &src, &["differs", "cases"])) {
        assert_eq!(got[0], "", "{path}");
        assert_eq!(got[1], format!("{}\n", cases.len()), "{path}");
    }
}

/// `sum x : { … }` is SQL's sum over the body's rows (rows that differ elsewhere count twice), grouped by the
/// rule's variables, 0 over none (Soufflé's), as W's `coalesce(sum(x), 0)`.
#[test]
fn sum_adds_the_bodys_rows() {
    let w = |rows: &[&str]| w_value(rows, "SELECT coalesce(sum(CAST(value AS INTEGER)), 0) FROM t", &[]);
    let src = format!(
        r#".decl r(g: symbol, k: number, x: number)
r("a", 1, 5). r("a", 2, 5). r("a", 3, 7). r("b", 1, -2).
.decl g(g: symbol)
g("a"). g("b"). g("c").
.decl total(g: symbol, s: number)
total(g, s) :- g(g), s = sum x : {{ r(g, _, x) }}.
.decl w(g: symbol, s: number)
w("a", {}). w("b", {}). w("c", {}).
.decl differs(g: symbol)
differs(g) :- total(g, s), !w(g, s).
differs(g) :- w(g, s), !total(g, s).
.decl doubled(s: number)
doubled(s) :- g("a"), s = sum y : {{ r("a", k, x), y = x * 2, k > 1 }}.
.decl shown(g: symbol, s: number)
shown(g, s) :- total(g, s).
.output shown, differs, doubled
"#,
        w(&["5", "5", "7"]),
        w(&["-2"]),
        w(&[]),
    );
    // `shown`, not `total`: the corpus runner writes an empty file for an output another rule reads (`total` read
    // by `differs` came out empty whatever derived it, `total(g, s) :- w(g, s)` too).
    for (path, got) in ["sql", "atoms"].iter().zip(on_both_paths("sum", &src, &["shown", "differs", "doubled"])) {
        assert_eq!(got[0], "a\t17\nb\t-2\nc\t0\n", "{path}: {got:?}");
        assert_eq!(got[1], "", "{path}");
        assert_eq!(got[2], "24\n", "{path}");
    }
}

/// A demanded symbol with many captures must not submit one SQLite statement per match.
#[test]
fn repeated_symbol_captures_use_bulk_sql() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    unsafe extern "C" fn trace(_: u32, state: *mut std::ffi::c_void, _: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> i32 {
        // SAFETY: the counter outlives the callback, which is removed before it drops.
        unsafe { &*state.cast::<AtomicUsize>() }.fetch_add(1, Ordering::Relaxed);
        0
    }
    let db = Connection::open_in_memory().unwrap();
    let source = (0..1000).map(|i| format!("k={i} ")).collect::<String>();
    let p = parse(&format!(r#"
.decl capture_input(s: symbol)
capture_input({}).
.decl captured(v: symbol)
captured(v) :- capture_input(s), match_all("k=(?<v>[0-9]+)", s).
.output captured
"#, serde_json::to_string(&source).unwrap())).unwrap();
    let count = AtomicUsize::new(0);
    unsafe { rusqlite::ffi::sqlite3_trace_v2(db.handle(), rusqlite::ffi::SQLITE_TRACE_STMT, Some(trace), std::ptr::from_ref(&count).cast_mut().cast()); }
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(Stats::empty(&p));
    let result = run(&p, &db, inputs, None, Hooks::default());
    unsafe { rusqlite::ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut()); }
    let result = result.unwrap();
    assert_eq!(result.outputs["captured"].len(), 1000);
    assert!(count.load(Ordering::Relaxed) < 100, "1000 matches submitted {} SQLite executions", count.load(Ordering::Relaxed));
}

/// A delta probe of a recursive component prices a lookup by the share of the binding's values the column holds
/// (containment): `fbind`'s shape, where `fcall` by `$a` (a few hundred rows, few distinct `$a`) is probed by
/// `pv`'s many `$a`. Priced as if every probe hit, `fcall` looked dearer than `tfix` by `$k`, and the plan read
/// `tfix` and `fgp` first (19 M rows of work for 220 in calls_cx).
#[test]
fn a_delta_probe_prices_a_lookup_by_the_bindings_it_can_hit() {
    let p = parse(
        ".decl pv(a:number, k:number, v:number)\n.decl fcall(c:number, f:number, i:number, a:number)\n.decl tfix(k:number, ka:number, w:number, x:number)\n\
         .decl fgp(f:number, i:number, ka:number, w:number, y:number)\n.decl fbind(c:number, f:number, y:number, x:number)\n\
         fbind(c, f, y, x) :- fcall(c, f, i, a), fgp(f, i, ka, w, y), pv(a, 1, k), tfix(k, ka, w, x).\n.output fbind\n",
    )
    .unwrap();
    let inputs = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), input_set: Default::default(), stats: Stats::empty(&p), ndv: HashMap::new(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    model.stats.nodes = 450_000.0;
    // Live counts, as a wave leaves them: rows, then distinct values by column.
    for (rel, rows, ndv) in [
        ("pv", 400_000.0, vec![300_000.0, 3.0, 5_000.0]),
        ("fcall", 400.0, vec![400.0, 50.0, 5.0, 100.0]),
        ("tfix", 550.0, vec![350.0, 50.0, 10.0, 300.0]),
        ("fgp", 100.0, vec![50.0, 5.0, 50.0, 10.0, 20.0]),
        ("fbind", 1.0, vec![1.0; 4]),
    ] {
        model.stats.card.insert(rel.to_owned(), rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel.to_owned(), c), n);
            model.stats.fan.insert((rel.to_owned(), c), rows / n);
        }
    }
    model.stats.solve(["pv", "fcall", "tfix", "fgp", "fbind"].into_iter().map(str::to_owned).collect());
    let r = &p.rules[0];
    let ((order, _, _), _) = model.plan_rule_explained(&p, &r.head, &r.body, Some(2)).unwrap();
    let order: Vec<&str> = order.iter().filter_map(|s| s.lit.atom()).map(|a| a.rel.as_str()).collect();
    assert_eq!(order[..2], ["pv", "fcall"], "{order:?}");
}

/// A restrictive relation outside the fixpoint has the same containment property as one inside it.
/// The delta's name domain is wider than the four names the restrictive input can match.
#[test]
fn a_delta_probe_prices_restrictive_external_inputs_by_the_values_they_can_hit() {
    let p = parse(".decl delta(k:number,a:number,t:number,f:number)\n.decl expand(o:number,t:number,k:number)\n.decl restrict(c:number,o:number,a:number)\n.input restrict\n.decl result(c:number,f:number)\nresult(c,f) :- restrict(c,o,a), expand(o,1,k), delta(k,a,2,f).\n.output result\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv, fan) in [
        ("delta",526.0,vec![526.0,173.0,1.0,526.0],vec![1.0,5.0,526.0,1.0]),
        ("expand",201_000.0,vec![10000.0,4.0,1878.0],vec![30.0,100000.0,200.0]),
        ("restrict",450.0,vec![450.0,450.0,4.0],vec![1.0,1.0,140.0]),
    ] {
        model.stats.card.insert(rel.into(), rows);
        for (c,(ndv,fan)) in ndv.into_iter().zip(fan).enumerate() {
            model.ndv.insert((rel.into(),c),ndv);
            model.stats.fan.insert((rel.into(),c),fan);
        }
    }
    model.stats.solve(["delta","expand","result"].into_iter().map(str::to_owned).collect());
    let r=&p.rules[0];
    let ((order,work,_),_)=model.plan_rule_explained(&p,&r.head,&r.body,Some(2)).unwrap();
    let order:Vec<_>=order.iter().filter_map(|s|s.lit.atom()).map(|a|a.rel.as_str()).collect();
    assert_eq!(order, ["delta","restrict","expand"], "restrict the delta before its high-fanout probe: {order:?}, work={work}");
    assert!(work<10_000.0, "price misses as well as hits: {work}");

    // A cached live plan must notice a new value distribution even when every relation's size is unchanged.
    let _plans=KeepPlans::start(None);
    model.ndv.insert(("delta".into(),1),1.0);
    let ((before,_,_),_)=model.plan_rule_live(&p,&r.head,&r.body,Some(2)).unwrap();
    let before:Vec<_>=before.iter().filter_map(|s|s.lit.atom()).map(|a|a.rel.as_str()).collect();
    assert_eq!(before,["delta","expand","restrict"],"the initial narrow-domain estimate makes expansion cheaper");
    model.ndv.insert(("delta".into(),1),173.0);
    let ((after,_,_),_)=model.plan_rule_live(&p,&r.head,&r.body,Some(2)).unwrap();
    let after:Vec<_>=after.iter().filter_map(|s|s.lit.atom()).map(|a|a.rel.as_str()).collect();
    assert_eq!(after,["delta","restrict","expand"],"a size-only cache must not keep the obsolete order");

    // A tiny delta can have fewer names than the input yet no matching names. Distinct counts alone cannot
    // detect this. Changing only the values must invalidate the plan even at fixed counts and fanouts.
    model.ndv.insert(("delta".into(),1),1.0);
    model.stats.dfreq.insert(("delta".into(),1),HashMap::from([(Constant::Int(10),526)]));
    model.stats.dfreq.insert(("restrict".into(),2),HashMap::from([(Constant::Int(10),450)]));
    let ((overlap,_,_),_)=model.plan_rule_live(&p,&r.head,&r.body,Some(2)).unwrap();
    assert_eq!(overlap.iter().filter_map(|s|s.lit.atom()).map(|a|a.rel.as_str()).collect::<Vec<_>>(),["delta","expand","restrict"]);
    model.stats.dfreq.insert(("delta".into(),1),HashMap::from([(Constant::Int(11),526)]));
    assert_eq!(model.delta_misses(&r.body,Some(2)),[0]);
    let ((disjoint,work,_),_)=model.plan_rule_live(&p,&r.head,&r.body,Some(2)).unwrap();
    assert_eq!(disjoint.iter().filter_map(|s|s.lit.atom()).map(|a|a.rel.as_str()).collect::<Vec<_>>(),["delta","restrict","expand"]);
    assert!(work<2000.0,"disjoint key values must avoid expansion: {work}");
    assert!(model.delta_misses(&r.body,None).is_empty(),"a later predicate is not an already-applied demand");

    // A partial SQL constant memo does not prove absence. Complete native postings do.
    model.stats.dfreq.remove(&("restrict".into(),2));
    let col=model.col("restrict",2);
    model.stats.col.insert(("restrict".into(),col.clone()),[450,2,0,0]);
    model.stats.memo.insert("restrict".into(),HashMap::from([((col.clone(),Constant::Int(10)),225)]));
    assert!(model.delta_misses(&r.body,Some(2)).is_empty());
    model.stats.memo.get_mut("restrict").unwrap().insert((col,Constant::Int(12)),225);
    assert_eq!(model.delta_misses(&r.body,Some(2)),[0]);
}

/// O125's retval regression: a kind-filtered node is not a random draw from the complete node domain.
#[test]
fn filtered_binding_domains_do_not_underprice_expansion_from_function_scopes() {
    let p = parse(".decl kind(n:number,k:number)\n.decl scope(n:number,f:number)\n.decl child(p:number,c:number,i:number)\n.input kind\n.input scope\n.input child\n.decl result(f:number,r:number)\nresult(f,r) :- kind(rs,1), child(rs,r,0), scope(rs,f), kind(f,2).\n.output result\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv, fan) in [
        ("kind",300_000.0,vec![300_000.0,100.0],vec![1.0,6000.0]),
        ("scope",160_836.0,vec![160_836.0,2181.0],vec![1.0,100.0]),
        ("child",180_000.0,vec![60_000.0,180_000.0,5.0],vec![3.0,1.0,60_000.0]),
    ] {
        model.stats.card.insert(rel.into(), rows);
        for (c,(ndv,fan)) in ndv.into_iter().zip(fan).enumerate() {
            model.ndv.insert((rel.into(),c),ndv);
            model.stats.fan.insert((rel.into(),c),fan);
        }
    }
    Arc::make_mut(&mut model.stats.kfreq).insert("1".into(),2063);
    Arc::make_mut(&mut model.stats.kfreq).insert("2".into(),2181);
    model.stats.dfreq.insert(("child".into(),2),HashMap::from([(Constant::Int(0),60_000)]));
    // The stated rule (the parse may give others first).
    let rule = p.rules.iter().find(|r| r.head.rel == "result").expect("the result rule");
    let functions = rule.body[3].atom().unwrap();
    assert_eq!(model.binding_domains(&p,functions)[0],Some(2181.0));
    let ((order,work,_),_)=model.plan_rule_explained(&p,&rule.head,&rule.body,None).unwrap();
    let first=order.iter().find_map(|s|s.lit.atom()).unwrap();
    assert_eq!(first,&rule.body[0].atom().unwrap().clone(),"return statements must precede scope expansion: {order:?}");
    assert!(work<20_000.0,"avoid the 160,836-row function-body expansion: {work}");

    // The same bound applies to an ordinary relation with a measured constant frequency.
    let atom=rule.body[1].atom().unwrap();
    assert_eq!(model.binding_domains(&p,atom)[1],Some(60_000.0));
    model.stats.dfreq.clear();
    assert_eq!(model.binding_domains(&p,atom)[1],Some(180_000.0),"unknown conditional counts must not invent selectivity");
}

/// A probe narrowed by a constant is priced by the constant's share of the rows where its postings are counted, not
/// by one value's share (`ndv`): `ifn`'s shape on the o125 replay, where `pv($o, 218, $k)` by `$k` holds 644 rows a
/// value and `218` is one of 7 values but 43% of the rows. Priced at 92 a binding, `pv` was read before `icall` by
/// `$a` (131): 382 rows a binding were read, then all rejected (0.2 M rows of work at step 07; 554 now).
#[test]
fn a_hot_constant_is_priced_by_its_share_of_the_rows() {
    let p = parse(
        ".decl icall(c:number, o:number, a:number)\n.decl pv(o:number, k:number, v:number)\n.decl fv(k:number, a:number, m:number, f:number)\n\
         .decl ifn(c:number, f:number)\n\
         ifn(c, f) :- icall(c, o, a), pv(o, 218, k), fv(k, a, 193, f).\n.output ifn\n",
    )
    .unwrap();
    let inputs = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), input_set: Default::default(), stats: Stats::empty(&p), ndv: HashMap::new(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    model.stats.nodes = 450_000.0;
    // Live counts, as the replay's step left them: rows, then by column distinct values and skew-weighted fanout.
    for (rel, rows, cols) in [
        ("icall", 438.0, vec![(438.0, 1.0), (438.0, 1.0), (4.0, 131.1)]),
        ("pv", 28_518.0, vec![(15_950.0, 5.3), (7.0, 12_373.0), (1_693.0, 644.4)]),
        ("fv", 695.0, vec![(60.0, 46.9), (153.0, 19.2), (3.0, 489.6), (201.0, 8.0)]),
        ("ifn", 1.0, vec![(1.0, 1.0); 2]),
    ] {
        model.stats.card.insert(rel.to_owned(), rows);
        for (c, (n, fan)) in cols.into_iter().enumerate() {
            model.ndv.insert((rel.to_owned(), c), n);
            model.stats.fan.insert((rel.to_owned(), c), fan);
        }
    }
    let values = |v: &[(i64, i64)]| v.iter().map(|&(k, n)| (Constant::Int(k), n)).collect::<HashMap<_, _>>();
    model.stats.dfreq.insert(("pv".to_owned(), 1), values(&[(218, 12_373), (202, 9_000), (156, 4_000), (1, 1_000), (2, 1_000), (3, 1_000), (4, 145)]));
    model.stats.dfreq.insert(("fv".to_owned(), 2), values(&[(193, 570), (1, 100), (2, 25)]));
    model.stats.solve(["pv", "fv", "ifn"].into_iter().map(str::to_owned).collect());
    let r = &p.rules[0];
    let ((order, _, _), _) = model.plan_rule_explained(&p, &r.head, &r.body, Some(2)).unwrap();
    let order: Vec<&str> = order.iter().filter_map(|s| s.lit.atom()).map(|a| a.rel.as_str()).collect();
    assert_eq!(order, ["fv", "icall", "pv"], "{order:?}");
}

/// An equality of a bound variable with an unbound one, `$s = $a` with `$s` bound, assigns `$a` (decision 0099 item
/// 4): the second atom is read by it, not crossed with the first and narrowed after (`fl4317_typing_root`: 14 k rows,
/// then 2.86 M, then 603). The rule runs to the same rows either way round.
#[test]
fn an_equality_binds_its_right_side_before_the_next_join() {
    let p = parse(
        ".decl p(s: number)\n.decl q(a: number)\n.decl o(s: number)\n.decl o2(s: number)\n\
         p(1). p(2). p(3). q(2). q(3). q(4).\n\
         o(s) :- p(s), q(a), s = a.\no2(s) :- p(s), q(a), 3 = a, s = a.\n.output o\n.output o2\n",
    )
    .unwrap();
    let db = Connection::open_in_memory().unwrap();
    let mut stats = Stats::empty(&p);
    stats.card.insert("p".into(), 14_000.0);
    stats.card.insert("q".into(), 14_000.0);
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(stats);
    let model = make_plan(&p, &db, inputs).unwrap();
    let r = &p.rules.iter().find(|r| r.head.rel == "o").unwrap();
    let (order, work, _) = model.plan_rule(&p, &r.head, &r.body, None).unwrap();
    let shown: Vec<String> = order.iter().map(|s| s.lit.to_string()).collect();
    assert_eq!(shown[1], "$s = $a", "the equality between the joins: {shown:?}, work {work}");
    assert!(work < 100_000.0, "a cross product priced: {work}");
    let mut inputs = Inputs::file();
    inputs.facts.clear();
    inputs.given = Some(Stats::empty(&p));
    let out = run(&p, &db, inputs, None, Hooks::default()).unwrap();
    let mut o: Vec<String> = out.outputs["o"].iter().map(|r| format!("{r:?}")).collect();
    o.sort();
    assert_eq!(o.len(), 2, "{o:?}");
    assert_eq!(out.outputs["o2"].len(), 1);
}

/// A concatenation given its inverse and a function made a join key (`simplify::concat_inverses`, `function_keys`)
/// derive the rows the rule did: a module's stub file found from its head (`modules.rr`'s `py_try` "stubs"), and
/// the rest of a name by `substr`, which runs natively.
#[test]
fn join_keys_from_functions_derive_the_rules_rows() {
    let src = r#"
        .decl want(q: symbol)
        want("a.b"). want("c.d"). want("e"). want("zz").
        .decl stubs(r: symbol)
        stubs("s").
        .decl sub(r: symbol, w: symbol)
        sub("s", "x"). sub("s", "y").
        .decl entry(d: symbol, n: symbol)
        entry("s/x", "a.pyi"). entry("s/x", "e.pyi"). entry("s/y", "zz.pyi"). entry("s/y", "c"). entry("t/x", "a.pyi").
        .decl hit(q: symbol, b: symbol, h: symbol)
        hit(q, b, h) :- want(q), stubs(r), sub(r, w), b = cat(r, "/", w), h = @head(q), x = cat(h, ".pyi"), entry(b, x).
        .decl rest(q: symbol, m: symbol)
        rest(q, m) :- want(q), h = @head(q), m = substr(q, strlen(h), strlen(q) - strlen(h)).
        .output hit
        .output rest
    "#;
    let plain = parse(src).unwrap();
    let mut keyed = plain.clone();
    let report = crate::code_rule_dsl::logic::simplify::simplify(&mut keyed);
    assert!(report.inverses >= 3, "the concatenations' parts: {report:?}");
    assert_eq!(report.keyed_by, ["want__key_head0"], "{report:?}");
    let rows = |p: &Program, modes: Option<&str>| -> Vec<Vec<String>> {
        let path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&path).unwrap();
        let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let lowered = lower::lower(p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
        let db = Connection::open(&path).unwrap();
        super::super::native::register(&db).unwrap();
        super::super::native::register_symbols(&db).unwrap();
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty(&lowered.program), BTreeSet::new(), modes.unwrap_or("")).unwrap();
        if modes.is_some() {
            assert!(got.native.contains("hit") && got.native.contains("rest"), "native: {:?}", got.native);
        }
        let mut out: Vec<Vec<String>> = ["hit", "rest"].iter().flat_map(|o| got.outputs[*o].iter().map(move |r| vec![o.to_string(), format!("{r:?}")])).collect();
        out.sort();
        out
    };
    let want = rows(&plain, None);
    assert_eq!(want.iter().filter(|r| r[0] == "hit").count(), 3, "{want:?}");
    assert_eq!(rows(&keyed, None), want);
    assert_eq!(rows(&keyed, Some("program")), want);
}

/// A gate (`answers::gate`'s `done_N_ad(t)` before `!N(t)`) holds only keys the rule's own literals give: after the
/// delta it is joined once its key is bound, not crossed in because an earlier wave's stats priced it at one row
/// (`py_try`'s "stubs": 404 rows crossed to 4 M, then rejected to 151).
#[test]
fn a_gate_is_joined_once_its_key_is_bound() {
    let mut p = parse(
        ".decl dir(b: symbol, x: symbol)\n.decl done_n_b(q: symbol)\n.decl n(q: symbol)\n.decl k(h: symbol, q: symbol)\n.decl o(q: symbol, b: symbol)\n\
         o(q, b) :- dir(b, x), done_n_b(q), !n(q), k(h, q), h = @str_between(x, \"\", \".pyi\").\n.output o\n",
    )
    .unwrap();
    for l in &mut p.rules[0].body {
        if let Literal::Atom(a) = l
            && a.neg
        {
            a.gated = true;
        }
    }
    let inputs = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), input_set: Default::default(), stats: Stats::empty(&p), ndv: HashMap::new(), docof: Default::default(), view_src: Default::default(), inputs: inputs.into(), derived: derived.into() };
    for (rel, rows) in [("dir", 400.0), ("done_n_b", 1.0), ("n", 10.0), ("k", 10_000.0)] {
        model.stats.card.insert(rel.to_owned(), rows);
    }
    let r = &p.rules[0];
    let ((order, _, _), _) = model.plan_rule_explained(&p, &r.head, &r.body, Some(0)).unwrap();
    let atoms: Vec<&str> = order.iter().filter_map(|s| s.lit.atom()).filter(|a| !a.neg).map(|a| a.rel.as_str()).collect();
    assert_eq!(atoms, ["dir", "k", "done_n_b"], "{:?}", order.iter().map(|s| s.lit.to_string()).collect::<Vec<_>>());
}

/// Maintenance must drive from measured changed scopes, including when the recursive delta is walk.
#[test]
fn changed_scope_demands_precede_walk_and_seed_caches_notice_constant_frequencies() {
    let p = parse(".decl walk(n:number,x:number,s:number)\n.decl bound(s:number,x:number)\n.decl changed__plus(s:number,k:number)\n.decl out(n:number,x:number,s:number)\nout(n,x,s) :- walk(n,x,s), !bound(s,x), changed__plus(s,1).\n.output out\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    for (rel, rows, ndv) in [("walk",71976.0,vec![50000.0,10000.0,1000.0]),("bound",40000.0,vec![1000.0,10000.0]),("changed__plus",7000.0,vec![7000.0,20.0])] {
        model.stats.card.insert(rel.into(),rows);
        for (c,n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel.into(),c),n);
            model.stats.fan.insert((rel.into(),c),rows/n);
        }
    }
    // Walk is highly skewed. Distinct changed scopes are key probes, not random walk rows.
    model.stats.fan.insert(("walk".into(),2),40000.0);
    model.stats.nodes=300000.0;
    let r=&p.rules[0];
    let _plans=KeepPlans::start(None);
    // Unknown delta distribution first, then an exact zero for the constant, at identical row counts.
    let _ = model.plan_rule_reused(&p,&r.head,&r.body,None).unwrap();
    model.stats.memo.insert("changed__plus".into(),HashMap::from([(("c1".into(),Constant::Int(1)),0)]));
    let ((order,work,_),_)=model.plan_rule_reused(&p,&r.head,&r.body,None).unwrap();
    assert_eq!(order[0].lit.atom().unwrap().rel,"changed__plus", "the seed must not retain the old full-walk scan");
    assert!(work < 10.0, "zero constant postings precede all walk/binding work: {work}");
    model.stats.solve(["walk","out"].into_iter().map(str::to_owned).collect());
    model.stats.memo.get_mut("changed__plus").unwrap().insert(("c1".into(),Constant::Int(1)),12);
    let ((order,work,_),_)=model.plan_rule_live(&p,&r.head,&r.body,Some(0)).unwrap();
    assert_eq!(order[0].lit.atom().unwrap().rel,"changed__plus", "a changed-input demand can precede a delta read");
    assert!(order.iter().position(|s|s.at==0).unwrap()>0);
    assert!(work<5000.0,"probe walk by the demanded scopes: {work}");
    // The same rule later has one new walk row. Rescanning the full changed-input demand each wave is worse.
    model.stats.delta_rows.insert("walk".into(),1.0);
    let ((small,_,_),_)=model.plan_rule_live(&p,&r.head,&r.body,Some(0)).unwrap();
    assert_eq!(small[0].at,0,"a one-row delta drives its own probe");

}

/// Smoking gun 2 (t34, 2026-11-13): a relation empty when its rule is planned (annotation demand's `rel_base`
/// before its guest round made `rr_ast_1_1`) priced every later step at 0 rows, so every order was free: `pkg` (40
/// rows) was scanned with nothing bound before `docof` was probed by what was bound (10,728 -> 25.8 M -> 10,728 rows
/// once `rr_ast_1_1` had rows; its EXPLAIN priced every step 0.00 rows). It is priced at least one row.
#[test]
fn a_relation_empty_when_planned_does_not_make_a_cross_product_free() {
    let p = parse(".decl e(x:number)\n.decl big(y:number)\n.decl link(x:number,y:number)\n.decl out(x:number,y:number)\nout(x,y) :- e(x), big(y), link(x,y).\n.output out\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    // `rel_base`'s sizes: `rr_ast_1_1` empty (no rows, so no fanout), `pkg` 40 rows, `docof` 90,100 keyed by its node.
    for (rel, rows, ndv) in [("e", 0.0, vec![1.0]), ("big", 40.0, vec![40.0]), ("link", 90_100.0, vec![90_100.0, 40.0])] {
        model.stats.card.insert(rel.into(), rows);
        for (c, n) in ndv.into_iter().enumerate() {
            model.ndv.insert((rel.into(), c), n);
            model.stats.fan.insert((rel.into(), c), rows / n);
        }
    }
    let r = &p.rules[0];
    let _plans = KeepPlans::start(None);
    let was = explaining(true);
    let (_, explained) = model.plan_rule_reused(&p, &r.head, &r.body, None).unwrap();
    explaining(was);
    let steps = explained.expect("explained").steps;
    let zero: Vec<&str> = steps.iter().filter(|s| s.rows <= 0.0).map(|s| s.literal.as_str()).collect();
    assert!(zero.is_empty(), "no step is priced at no rows, so no order after the empty relation is free: {zero:?} of {:?}", steps.iter().map(|s| (s.literal.as_str(), s.rows)).collect::<Vec<_>>());
}

/// Smoking gun 10 (t34, 2026-11-13), red until fixed. `frag_str(s) :- ann_str(s), d = @doc(s), doc_role(d,
/// "fragment")` is planned before the guest round makes the fragment documents: "fragment" counts 0, so `doc_role`
/// leads, and `d = @doc(s)` keys nothing, so each fragment then scans every string. Its line did about a row of work,
/// and the planning budget (decision 0122) measures growth by the relations' rows alone (`budget_takes`): `doc_role`
/// grew 9 times, under the budget, so the order was kept with 2,455 fragments (7.1 M rows, none kept). A constant's
/// count that moved from none to many is growth, and the line is searched again.
#[test]
fn a_constant_counted_none_when_planned_is_growth_when_it_has_rows() {
    let p = parse(".decl ann_str(s: node)\n.decl doc_role(d: number, r: number)\n.decl frag_str(s: node)\nfrag_str(s) :- ann_str(s), d = @doc(s), doc_role(d, 7).\n.output frag_str\n").unwrap();
    let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
    let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
    let mut model = Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: inputs.into(),derived: derived.into()};
    let size = |model: &mut Model, roles: f64, fragments: i64| {
        for (rel, rows, ndv) in [("ann_str", 100_000.0, vec![100_000.0]), ("doc_role", roles, vec![roles, 4.0])] {
            model.stats.card.insert(rel.into(), rows);
            for (c, n) in ndv.into_iter().enumerate() {
                model.ndv.insert((rel.into(), c), n);
                model.stats.fan.insert((rel.into(), c), (rows / n).max(1.0));
            }
        }
        model.stats.memo.insert("doc_role".into(), HashMap::from([(("r".into(), Constant::Int(7)), fragments)]));
    };
    model.stats.nodes = 300_000.0;
    let r = &p.rules[0];
    let _plans = KeepPlans::start(None);
    let lead = |order: &[Step]| order[0].lit.atom().map(|a| a.rel.to_string());
    // Planned before the guest round: about no fragment yet (production's estimate was 0.01 rows), and the line
    // ran over none.
    size(&mut model, 300.0, 1);
    let ((order, work, _), _) = model.plan_rule_reused(&p, &r.head, &r.body, None).unwrap();
    assert_eq!(lead(&order).as_deref(), Some("doc_role"), "with about no fragment, `doc_role` leads: {work} {:?}", order.iter().map(|s| s.lit.to_string()).collect::<Vec<_>>());
    model.ran_as_planned(&r.head, &r.body, None, 1.0);
    // The guest round made 2,455 fragment documents.
    size(&mut model, 2_755.0, 2_455);
    let ((order, _, _), _) = model.plan_rule_reused(&p, &r.head, &r.body, None).unwrap();
    assert_eq!(lead(&order).as_deref(), Some("ann_str"), "2,455 fragments, each scanning every string, are searched again");
}

#[test]
fn live_statistics_invalidate_only_changed_distribution_slots() {
    let p=parse(".decl a(x:number,y:number)\n").unwrap();
    let mut model=Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: Default::default()};
    let sample=|key| LiveRel {n:10,delta:2,ndv:vec![10,1],sq:vec![10,100],freq:vec![(1,HashMap::from([(Constant::Int(key),10)]))]};
    assert_eq!(live_stats(&mut model,"a",sample(1)),vec![0,1,2]);
    assert!(live_stats(&mut model,"a",sample(1)).is_empty());
    assert_eq!(live_stats(&mut model,"a",sample(2)),vec![2],"equal counts and fanouts with new keys must invalidate");
}

#[test]
fn projection_statistics_keep_demanded_constants_only_when_rows_are_preserved() {
    let p=parse(".decl raw(d:number,n:number,k:number)\n.input raw\n.decl tags(n:number,k:number)\ntags(n,k) :- raw(_,n,k).\n.decl filtered(n:number,k:number)\nfiltered(n,k) :- raw(1,n,k).\n.decl kinds(k:number)\nkinds(k) :- raw(_,_,k).\n").unwrap();
    let mut model=Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: Arc::new(p.rules.iter().map(|r|r.head.rel.to_string()).collect::<HashSet<String>>())};
    model.stats.card.insert("raw".into(),7000.0);
    for (c,n) in [10.0,7000.0,20.0].into_iter().enumerate() {
        model.ndv.insert(("raw".into(),c),n); model.stats.fan.insert(("raw".into(),c),7000.0/n);
    }
    model.stats.memo.insert("raw".into(),HashMap::from([(("c0".into(),Constant::Int(1)),100),(("c2".into(),Constant::Int(9)),12)]));
    for r in &p.rules { model.view_stats(&r.head.rel,&r.head,r.body[0].atom().unwrap()); }
    assert_eq!(model.stats.const_freq("tags","c1",&Constant::Int(9)),Some(12));
    assert_eq!(model.stats.const_freq("filtered","c1",&Constant::Int(9)),None,"filtered marginal counts are not exact");
    assert_eq!(model.stats.const_freq("kinds","c0",&Constant::Int(9)),None,"deduplicating projection changes frequencies");
}

#[test]
fn statistics_versions_accumulate_drift_and_publish_changed_keys() {
    let p=parse(".decl a(x:number,y:number)\n.decl b(x:number)\n.decl p(x:number)\np(x) :- a(x,_), b(x).\n").unwrap();
    let mut model=Model { shapes: Default::default(),docs_by_shift:false,analysis:Default::default(),input_set:Default::default(),stats:Stats::empty(&p),ndv:HashMap::new(),docof: Default::default(),view_src: Default::default(),inputs: Inputs::file().into(),derived: Default::default()};
    let mut watch=replanning::Watch::new(&BTreeMap::from([("a".into(),0)]),&[(&p.rules[0],0,vec![])]);
    watch.delta(0,5);
    model.stats.card.insert("a".into(),100.0); watch.record(0,0,&model,"a"); watch.record(0,1,&model,"a");
    assert!(watch.take(0));
    model.stats.card.insert("a".into(),150.0); watch.record(0,0,&model,"a");
    assert!(!watch.take(0),"small drift keeps the priced signature");
    model.stats.card.insert("a".into(),210.0); watch.record(0,0,&model,"a");
    assert!(watch.take(0),"small changes accumulate against the last published size");
    for key in [1,2] {
        model.stats.dfreq.insert(("a".into(),0),HashMap::from([(Constant::Int(key),210)]));
        watch.record(0,1,&model,"a");
        assert!(watch.take(0),"new key domains publish even at identical row counts");
    }
    model.stats.fan.insert(("a".into(),1),999.0); watch.record(0,2,&model,"a");
    assert!(!watch.take(0),"an unused wildcard column has no dependent plan");
    watch.delta(0,0);
    model.stats.card.insert("a".into(),1000.0); watch.record(0,0,&model,"a");
    assert!(!watch.take(0)); watch.delta(0,1); assert!(watch.take(0));
}

/// Decision 0123 §7: an order kept at a price under a search's cost (taken greedily, or found by a search) is searched
/// again only when its line did a search's work when it last ran: the search then costs less than the line did.
#[test]
fn an_order_priced_under_a_search_is_searched_once_it_ran_over_one() {
    let p = parse(".decl a(x:number,y:number)\n.decl b(y:number,z:number)\n.decl c(z:number,w:number)\n.decl d(w:number)\nd(w) :- a(x,y), b(y,z), c(z,w).\n").unwrap();
    let (body, first) = (&p.rules[0].body, Some(0));
    let search = search_rows(body, first);
    // Three joins led by the delta: a fixed 400 rows and five candidates.
    assert_eq!(search, 405.0);
    assert!(!mispriced(10.0, None, body, first), "a line that never ran");
    assert!(!mispriced(10.0, Some(search.max(SEARCH_WORK) - 1.0), body, first), "it ran under a search's work");
    assert!(mispriced(10.0, Some(search.max(SEARCH_WORK)), body, first), "it ran a search's work, priced at 10 rows");
    assert!(!mispriced(search + 1.0, Some(1e9), body, first), "priced over a search: the re-price and replan rules hold it");
}

#[test]
fn a_plan_priced_on_an_empty_relation_is_not_kept_once_it_fills() {
    // Within the drift, both ways; an empty relation against one row is not (a cross product after it was free).
    assert!(near(&[1.0, 10.0], &[1.5, 19.0]));
    assert!(near(&[0.0, 10.0], &[0.0, 10.0]));
    assert!(!near(&[0.0, 10.0], &[1.0, 10.0]), "empty, then one row");
    assert!(!near(&[1.0], &[0.0]), "one row, then empty");
}

#[test]
fn child_ordinal_lookup_is_unique_despite_hot_marginal_statistics() {
    let p = parse(".decl parents(p:number)\n.input parents\n.decl child(p:number,c:number,idx:number)\n.input child\n.decl out(c:number)\nout(c) :- parents(p), child(p,c,0).\n").unwrap();
    let mut model = Model { shapes: Default::default(), docs_by_shift:false, analysis:Default::default(), input_set:Default::default(),
        stats:Stats::empty(&p), ndv:HashMap::new(), docof:Default::default(), view_src:Default::default(),
        inputs:Inputs::file().into(), derived:Default::default() };
    model.stats.card.insert("parents".into(),100.0);
    model.ndv.insert(("parents".into(),0),100.0);
    model.stats.card.insert("child".into(),10000.0);
    model.ndv.insert(("child".into(),0),100.0);
    model.ndv.insert(("child".into(),1),10000.0);
    model.ndv.insert(("child".into(),2),100.0);
    model.stats.fan.insert(("child".into(),0),100.0);
    model.stats.memo.insert("child".into(),HashMap::from([(("idx".into(),Constant::Int(0)),9000)]));
    let (order,work,rows) = model.plan_rule(&p,&p.rules[0].head,&p.rules[0].body,None).unwrap();
    assert_eq!(order[0].lit.atom().unwrap().rel,"parents");
    assert!(rows<=100.0 && work<=400.0,"a parent/ordinal lookup has at most one result: {rows} rows, {work} work");
}

/// A planner change's measurement (O148): T's programs on T's facts, each statement's SQLite VM steps, the plans
/// chosen and whether each output is T's. Run on a build before the change and one after (`O148_OUT`, with
/// `RERAIL_NODE_IDS=preorder`) and compare them (`scripts/profiling/o148/compare.py`): the goldens (`rules.txt`) pin
/// plans that did no more work than the ones they replace.
#[test]
#[ignore = "measurement: a planner change's plans and their work, compared across two builds"]
fn o148_plans_and_their_work() {
    use rusqlite::ffi;
    use std::sync::Mutex;
    static COSTS: Mutex<Vec<(String, u64, u64)>> = Mutex::new(Vec::new());
    unsafe extern "C" fn trace(event: u32, _: *mut std::ffi::c_void, p: *mut std::ffi::c_void, x: *mut std::ffi::c_void) -> std::ffi::c_int {
        if event == ffi::SQLITE_TRACE_PROFILE as u32 {
            let stmt = p.cast::<ffi::sqlite3_stmt>();
            // SAFETY: for SQLITE_TRACE_PROFILE, P is the statement and X its time in nanoseconds.
            let (ns, vm, text) = unsafe {
                (*x.cast::<i64>() as u64, ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 1) as u64,
                 std::ffi::CStr::from_ptr(ffi::sqlite3_sql(stmt)).to_string_lossy().split_whitespace().collect::<Vec<_>>().join(" "))
            };
            COSTS.lock().unwrap().push((text, vm, ns));
        }
        0
    }
    let out = std::env::var("O148_OUT").expect("O148_OUT: the file to write");
    let mut lines = Vec::new();
    let facts = facts();
    for name in ["workload", "workload_demand", "calls"] {
        let db = Connection::open_in_memory().unwrap();
        db.execute("ATTACH ?1 AS f", [format!("file:{}?mode=ro", facts.display())]).unwrap();
        db.execute_batch("PRAGMA f.mmap_size = 4294967296; PRAGMA temp_store = MEMORY;").unwrap();
        // SAFETY: the connection outlives the run; the callback reads only its arguments.
        unsafe { ffi::sqlite3_trace_v2(db.handle(), ffi::SQLITE_TRACE_PROFILE as u32, Some(trace), std::ptr::null_mut()) };
        COSTS.lock().unwrap().clear();
        let got = super::run(&program(name), &db, Inputs::file(), None, Hooks::default()).unwrap_or_else(|e| panic!("{name}: {e}"));
        for c in &got.chosen {
            lines.push(format!("PLAN\t{name}\t{}", chosen_line(c)));
        }
        for (rel, rows) in &got.outputs {
            let expected = std::fs::read_to_string(root().join(format!("plan/golden/{name}/{rel}.csv"))).unwrap();
            let same = rows.iter().map(String::as_str).collect::<Vec<_>>() == expected.lines().collect::<Vec<_>>();
            lines.push(format!("OUT\t{name}\t{rel}\t{}\t{same}", rows.len()));
        }
        for (text, vm, ns) in COSTS.lock().unwrap().drain(..) {
            lines.push(format!("STMT\t{name}\t{vm}\t{ns}\t{text}"));
        }
    }
    std::fs::write(out, lines.join("\n") + "\n").unwrap();
}

#[test]
fn empty_required_inputs_defer_both_plan_shapes_until_growth() {
    let program = crate::code_rule_dsl::logic::rr::parse(
        ".decl a(x:number)\n.decl b(x:number)\n.decl p(x:number)\np(x) :- a(x), b(x).\n").unwrap();
    let mut w = replanning::Watch::new(&BTreeMap::from([("a".into(),0),("b".into(),1)]), &[(&program.rules[0],0,vec![])]);
    w.rows(0,100); w.rows(1,0); w.delta(0,100);
    assert!(!w.take(0));
    assert!(!w.take_whole(0), "a whole join over an empty input needs no search");
    w.rows(1,1);
    assert!(w.take(0)); assert!(w.take_whole(0));
    assert!(!w.take(0)); assert!(!w.take_whole(0));
}

/// A kept plan's distributions at another scale (O-replan-churn): a count of node ids in proportion to the nodes, a
/// column of few values as it was, and the bit per literal as it is.
#[test]
fn kept_distributions_are_near_in_proportion_to_the_nodes() {
    use super::near_distributions;
    let scale = 61997.0 / 22146.0;
    // ndv of node ids, fan, a kind's ndv; then one literal's bit.
    assert!(near_distributions(&[22089.0, 1.0, 81.0, 1.0], &[61994.0, 1.0, 85.0, 1.0], scale, 1));
    assert!(!near_distributions(&[22089.0, 1.0, 81.0, 1.0], &[61994.0, 1.0, 85.0, 4.0], scale, 1), "a literal's bit moved");
    assert!(!near_distributions(&[22089.0, 1.0, 81.0, 1.0], &[400000.0, 1.0, 85.0, 1.0], scale, 1), "grew far past the scale");
    assert!(!near_distributions(&[-2.0, 1.0], &[5.0, 1.0], scale, 1), "proven empty against present");
}

/// O-join-shape-directives: `pv__ins`'s shape at O125 in small. With `retparam` tiny, the planner crosses `pv` with
/// it (no shared variable) before `posarg`; `.shape retparam bound f` holds it back until `cfn` binds `f`, in the rule
/// as written and in a delta's form of it (`pv__ins`). With `posarg` sized by no statistic of its own, the guard takes
/// no cross product against it.
#[test]
fn a_join_shape_directive_holds_a_join_back_in_the_rule_and_its_forms() {
    let rule = ".decl pv(a:number,k:number,v:number)\n.decl posarg(c:number,i:number,a:number)\n.decl cfn(c:number,e:number,f:number)\n\
        .decl retparam(f:number,i:number,x:number)\n.input pv, posarg, cfn, retparam\n.decl out(c:number,k:number,v:number)\n.output out\n\
        out(c, k, v) :- pv(a, k, v), posarg(c, i, a), cfn(c, _, f), retparam(f, i, _).\n";
    let model_for = |p: &Program, sized: &[(&str, f64, Vec<f64>)]| {
        let inputs = Inputs { facts:"main.".into(), stats:"main.".into(), atoms:Some([0;3]), native:BTreeSet::new(), batch:HashMap::new(), given:None, kept:BTreeSet::new() };
        let derived: HashSet<String> = p.rules.iter().map(|r| r.head.rel.to_string()).collect();
        let mut model = Model { shapes: std::sync::Arc::new(p.shapes.clone()), docs_by_shift:false, analysis:Default::default(), input_set:Default::default(), stats:Stats::empty(p), ndv:HashMap::new(), docof:Default::default(), view_src:Default::default(), inputs:inputs.into(), derived:derived.into() };
        for (rel, rows, ndv) in sized {
            model.stats.card.insert((*rel).into(), *rows);
            for (c, n) in ndv.iter().enumerate() {
                model.ndv.insert(((*rel).into(), c), *n);
                model.stats.fan.insert(((*rel).into(), c), rows / n);
            }
        }
        model.stats.nodes = 300000.0;
        model
    };
    let sizes = vec![("pv", 1000.0, vec![1000.0, 50.0, 500.0]), ("posarg", 100000.0, vec![50000.0, 5.0, 100.0]), ("cfn", 50000.0, vec![50000.0, 100.0, 2000.0]), ("retparam", 2.0, vec![2.0, 2.0, 2.0])];
    let order_of = |text: &str, sized: &[(&str, f64, Vec<f64>)], head: Option<&str>, first: Option<usize>| -> Vec<String> {
        let mut p = parse(text).unwrap();
        if let Some(h) = head {
            // A delta's form of the rule: its head renamed as maintenance names it.
            p.rules[0].head.rel = h.to_owned().into();
        }
        let model = model_for(&p, sized);
        let _plans = KeepPlans::start(None);
        let r = &p.rules[0];
        let (order, _, _) = model.plan_rule(&p, &r.head, &r.body, first).unwrap();
        order.iter().filter_map(|s| s.lit.atom().map(|a| a.rel.to_string())).collect()
    };
    let at = |o: &[String], r: &str| o.iter().position(|x| x == r).unwrap();
    let free = order_of(rule, &sizes, None, None);
    assert!(at(&free, "retparam") < at(&free, "cfn"), "without the directive the planner crosses retparam in early: {free:?}");
    let shaped = format!("{rule}.shape retparam bound f\n");
    for head in [None, Some("out__ins")] {
        let o = order_of(&shaped, &sizes, head, None);
        assert!(at(&o, "retparam") > at(&o, "cfn"), "retparam waits for f ({head:?}): {o:?}");
    }
    // Profiled, a directive the planner's own order already keeps is reported as no longer needed; one that changed
    // the order is reported with both estimates.
    {
        struct Off;
        impl Drop for Off {
            fn drop(&mut self) {
                crate::code_rule_dsl::logic::profile::set(false);
            }
        }
        let _off = Off;
        crate::code_rule_dsl::logic::profile::set(true);
        crate::code_rule_dsl::logic::profile::reset();
        // EXPLAIN says which directive shaped the delta form's plan.
        {
            let mut p = parse(&shaped).unwrap();
            p.rules[0].head.rel = "out__ins".to_owned().into();
            let model = model_for(&p, &sizes);
            let _plans = KeepPlans::start(None);
            let r = &p.rules[0];
            let (_, x) = model.plan_rule_explained(&p, &r.head, &r.body, None).unwrap();
            assert_eq!(x.map(|x| x.shaped), Some(vec!["retparam bound f".to_owned()]));
        }
        let _ = order_of(&format!("{rule}.shape pv after posarg\n"), &sizes, None, None);
        let _ = order_of(&shaped, &sizes, None, None);
        let made: Vec<(String, String)> = crate::code_rule_dsl::logic::profile::decisions().into_keys().filter(|(t, _)| t.contains("join shape directive")).collect();
        crate::code_rule_dsl::logic::profile::set(false);
        assert!(made.iter().any(|(t, x)| t.contains("no longer needed") && x.contains("pv after posarg")), "{made:?}");
        assert!(made.iter().any(|(t, x)| t.starts_with("join shape directives") && x.contains("retparam bound f")), "{made:?}");
    }
    let after = order_of(&format!("{rule}.shape retparam after cfn\n"), &sizes, None, None);
    assert!(at(&after, "retparam") > at(&after, "cfn"), "{after:?}");
    // The directive names what its rule reads.
    let err = format!("{:?}", parse(&format!("{rule}.shape nothing after cfn\n")).unwrap_err());
    assert!(err.contains("no rule of `out` reads `nothing`"), "{err}");
    let err = format!("{:?}", parse(".shape retparam bound f\n").unwrap_err());
    assert!(err.contains("follows no rule"), "{err}");
    // No directive, `pv`'s delta driving, and `posarg` unsized: no cross product priced against its default.
    let no_posarg: Vec<(&str, f64, Vec<f64>)> = sizes.iter().filter(|s| s.0 != "posarg").cloned().collect();
    let guarded = order_of(rule, &no_posarg, None, Some(0));
    assert!(at(&guarded, "retparam") > at(&guarded, "posarg"), "{guarded:?}");
}
