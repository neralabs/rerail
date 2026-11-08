use super::*;
use crate::code_rule_dsl::logic::front::prune;
use crate::code_rule_dsl::logic::rr::parse;
use crate::code_rule_dsl::logic::magic::{Options, magic_adorn, magic_sets};
use crate::content_store::ContentStore;

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
    let read = |out: &std::path::Path, rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    for name in ["workload", "calls"] {
        let src = crate::code_rule_dsl::tools::program_text(&root().join(format!("../typing/programs/{name}.rr"))).unwrap();
        let with_t = dir.join(format!("out-{name}-t"));
        let report = corpus::run(&src, &dir, Some(&facts), &with_t, "program", Binding::Cpu, 1 << 18).unwrap();
        assert!(report.rows.values().any(|&n| n > 0), "{name}: no outputs");
        // The cross-file part runs in bounded rounds (`typing::cross_rounds`, k = 4 since decision 0047). On this
        // corpus both reach the fixpoint: workload's after round 1 (round 2's digest is round 1's), calls' after round
        // 2 (hop rows 5,367 → 5,427 → 5,435 → 5,435; round 2 is the third crossing k = 2 cut, O41). Both runs below are
        // bounded alike.
        println!("{name}: cross-file rounds {:?}", report.rounds);
        assert_eq!(report.rounds.run, if name == "workload" { 3 } else { 4 }, "{name}: {:?}", report.rounds);
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

// ---- aggregates ----

/// `join`, `sum` and aggregates inside terms, run over a corpus: join orders by its keys (numbers by value, symbols
/// by code point, a tuple key by its parts in turn), ties by the value, and gives "" of nothing; sum adds every row
/// of its body (two arguments of one length both count) and gives 0 of nothing.
#[test]
fn join_sum_and_aggregates_in_terms_run() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("aggregates");
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
.output in_order, by_text, reversed, numbers, tuple, total, ccn, more
"#;
    let out = dir.join("out");
    super::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let a = |order: &mut dyn Iterator<Item = i32>, sep: &str| order.map(|i| format!("a{i}")).collect::<Vec<_>>().join(sep);
    let mut text_order: Vec<String> = (0..=10).map(|i| format!("a{i}")).collect();
    text_order.sort();
    assert_eq!(read("in_order"), format!("f\tbb, a, cc\ng\t\nh\té, z, B\nm\t{}\n", a(&mut (0..=10), ", ")));
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(name);
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("places");
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("node-sets");
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("agg-builtin");
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("recursion");
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("negations");
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
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(name);
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
    let result = run(&p, &db, inputs, None, None);
    unsafe { rusqlite::ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut()); }
    let result = result.unwrap();
    assert_eq!(result.outputs["captured"].len(), 1000);
    assert!(count.load(Ordering::Relaxed) < 100, "1000 matches submitted {} SQLite executions", count.load(Ordering::Relaxed));
}
