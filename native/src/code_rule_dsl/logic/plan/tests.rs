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
        let path=crate::code_rule_dsl::prefilter::tests::temp();
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
        let path = crate::code_rule_dsl::prefilter::tests::temp();
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

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic")
}

/// The facts T's exporter and `stats.py` built for nine CPython 3.13 modules (`json/`, `ftplib`, `shlex`,
/// `fnmatch`, `glob`), loaded from their compressed dump into a new file.
fn facts() -> std::path::PathBuf {
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    let path = crate::code_rule_dsl::prefilter::tests::temp();
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
    let facts = facts();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let golden = root().join(format!("plan/golden/{name}"));
        let got = run_sql(&program(name), &facts).unwrap_or_else(|e| panic!("{name}: {e}"));
        let lines: Vec<String> = got.chosen.iter().map(chosen_line).collect();
        let expected = std::fs::read_to_string(golden.join("rules.txt")).unwrap();
        let expected: Vec<&str> = expected.lines().collect();
        if let Some(i) = (0..lines.len().max(expected.len())).find(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
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
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    let path = crate::code_rule_dsl::prefilter::tests::temp();
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
        if let Some(i) = (0..lines.len().max(expected.len())).find(|&i| lines.get(i).map(String::as_str) != expected.get(i).copied()) {
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
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::waves::{owner::Backing, schedule};
    use crate::content_store::Binding;
    let path = crate::code_rule_dsl::prefilter::tests::temp();
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
            let outcome = schedule::run(&lease, &store, &emitted.program, "/w", docs.clone(), &schedule::Limits::default(), &mut |_| Ok(None))
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
        let path = crate::code_rule_dsl::prefilter::tests::temp();
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
    use crate::code_rule_dsl::facts::vtab::{Modules, syntax};
    use crate::code_rule_dsl::typing::file_local::Binding;
    let t = Connection::open_in_memory().unwrap();
    let dump = zstd::decode_all(std::fs::File::open(root().join("plan/golden/facts.sql.zst")).unwrap()).unwrap();
    t.execute_batch(std::str::from_utf8(&dump).unwrap()).unwrap();
    let first: HashMap<i64, i64> =
        t.prepare("SELECT d, min(n) FROM node GROUP BY d").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let path = crate::code_rule_dsl::prefilter::tests::temp();
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
         CREATE TABLE value(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.value', 0);
         INSERT INTO stat_col VALUES ('value', 'n', 0, 0, 0, 0), ('value', 's', 0, 0, 0, 0);
         CREATE TABLE str(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.str', 0);
         INSERT INTO stat_col VALUES ('str', 'n', 0, 0, 0, 0), ('str', 's', 0, 0, 0, 0);
         CREATE TABLE name(n INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.name', 0);
         INSERT INTO stat_col VALUES ('name', 'n', 0, 0, 0, 0), ('name', 's', 0, 0, 0, 0);
         CREATE TABLE op_at(n INTEGER, i INTEGER, s TEXT); INSERT INTO stat_meta VALUES ('rows.op_at', 0);
         INSERT INTO stat_col VALUES ('op_at', 'n', 0, 0, 0, 0), ('op_at', 'i', 0, 0, 0, 0), ('op_at', 's', 0, 0, 0, 0)",
    ).unwrap();
    let mut failures = Vec::new();
    for name in ["workload", "workload_demand", "calls"] {
        let src = crate::code_rule_dsl::tools::program_text(&root().join(format!("../typing/programs/{name}.rr"))).unwrap();
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
    use crate::code_rule_dsl::typing::file_local::Binding;
    let raw = zstd::decode_all(std::fs::File::open(root().join("../facts/vtab/golden/corpus.json.zst")).unwrap()).unwrap();
    let corpus: Vec<(String, String)> = serde_json::from_slice(&raw).unwrap();
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("corpus");
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
        let src = crate::code_rule_dsl::tools::program_text(&root().join(format!("../typing/programs/{name}.rr"))).unwrap();
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("typed");
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

/// Step 30's primitives through the corpus runner: `line(n, l)`, and literal tests on names and paths (`glob`,
/// `prefix`, `suffix`) in place of regexes.
#[test]
fn rules_test_names_paths_and_lines_without_regexes() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("strings");
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("axes");
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
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension(format!("measure-{way}"));
    let r = super::corpus::run(&format!("{head}{body}"), &corpus, None, &out, "program", binding, 1 << 18).unwrap();
    let cpu: f64 = r.phases.iter().map(|p| p.1 + p.2).sum();
    println!("MEASURE {way}: {} files, {} nodes, {} chunks, hit rows {}, CPU {cpu:.2} s, peak RSS {:.0} MB", r.files, r.nodes, r.chunks, r.rows["hit"], r.phases.iter().map(|p| p.3).fold(0.0, f64::max));
    for (name, user, sys, rss, statements) in &r.phases {
        println!("  {name:<16} {:.2} s CPU  rss {rss:.0} MB  {statements} statements", user + sys);
    }
    let _ = std::fs::remove_dir_all(&out);
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
