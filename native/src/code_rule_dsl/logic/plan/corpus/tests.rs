//! The corpus runner's tests. The engine compiles them (its facade of `logic::plan::corpus`): they parse files, and
//! the languages are registered by the crates above `rerail-logic` as the engine loads.

#[cfg(feature = "tape-cache")]
#[test]
fn edits_keep_other_documents_facts_when_the_input_omits_its_owner() {
    use crate::code_rule_dsl::logic::plan::output;
    use crate::code_rule_dsl::typing::file_local::Binding;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("sources");
    let out = temp.path().join("out");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("keep.py"), "keep = 1\n").unwrap();
    std::fs::write(root.join("base.py"), "base = 1\n").unwrap();
    let rr = r#".decl doc(d: number, path: symbol, module: symbol)
.input doc
.decl request(k: number)
request(1) :- doc(_, "keep.py", _).
request(2) :- doc(_, "remove.py", _).
.output request
"#;
    let run = || {
        super::run(rr, &root, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let mut rows = output::shown(&out, "request").unwrap();
        rows.sort();
        rows
    };
    let previous = super::TEST_TAPE_CACHE.with(|p| p.replace(Some(temp.path().join("kept.sqlite"))));
    assert_eq!(run(), [["1"]]);
    std::fs::write(root.join("remove.py"), "remove = 2\n").unwrap();
    assert_eq!(run(), [["1"], ["2"]]);
    std::fs::remove_file(root.join("remove.py")).unwrap();
    assert_eq!(run(), [["1"]]);
    super::TEST_TAPE_CACHE.with(|p| p.replace(previous));
}

/// Phase 0 alone over `RERAIL_P0_ROOT`: the corpus's files (from Git in a Git work tree, else the walk) and
/// every Python document's digest, with the observer's counts and the CPU. Run it twice for the warm figure; the
/// observer's store is `<RERAIL_TAPE_CACHE>.files`.
#[test]
#[ignore = "a benchmark: RERAIL_P0_ROOT=<a worktree>"]
fn benchmark_phase_0() {
    let root = std::path::PathBuf::from(std::env::var("RERAIL_P0_ROOT").expect("RERAIL_P0_ROOT"));
    let before = super::observe(|o| {
        o.begin();
        o.counts
    });
    let start = super::thread_usage();
    let files = super::corpus_files(&root).unwrap();
    let listed = super::thread_usage();
    let mut digests = 0;
    for f in files.by_label.get("python").into_iter().flatten() {
        digests += super::observe(|o| o.digest(&root.join(f))).map_or(0, |_| 1);
    }
    let digested = super::thread_usage();
    let did = super::observe(|o| {
        o.flush();
        // Flush queues the final observations. Waiting before it leaves a cold
        // observer store when the process exits immediately after this run.
        if !super::phase0::active() { o.written(); }
        o.counts.since(&before)
    });
    let flushed = super::thread_usage();
    let cpu = |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) + (b.1 - a.1);
    println!(
        "phase 0 over {}: {} python, {} other files; {digests} digests; files {:.2} s, digests {:.2} s, flush {:.2} s CPU; peak RSS {:.0} MB\n  {did}",
        root.display(),
        files.count("python"),
        files.parsed() - files.count("python") + files.text.len(),
        cpu(start, listed),
        cpu(listed, digested),
        cpu(digested, flushed),
        super::usage().2
    );
}
/// Q203 item 3 (ii): a corpus run's chunks read one normalized program, made once, not one each (each chunk's
/// own version held every analysis of it again); the findings are those of one chunk.
#[test]
fn a_runs_chunks_normalize_the_program_once() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("normal-once");
    std::fs::create_dir_all(&dir).unwrap();
    for (i, name) in ["a", "b", "c", "d", "e", "f", "g", "h"].iter().enumerate() {
        std::fs::write(dir.join(format!("{name}.py")), format!("def f{i}():\n    x = {i}\n    return x\n\ndef g{i}():\n    return f{i}()\n")).unwrap();
    }
    let rule = dir.join("calls.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, x: symbol)
def(n, x) :- kind(n, "function_definition"), field(n, "name", c), text(c, x).
.decl calls(n: node, x: symbol)
calls(n, x) :- kind(n, "call"), field(n, "function", c), text(c, x), def(_, x).
.output calls
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |chunk_nodes: usize, store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(format!("store-{store}.sqlite"))));
        // The run's own scope, held here: what it normalized is the run's count (decision 0442), read before it ends.
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        let report = super::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, chunk_nodes).unwrap();
        let normalized = crate::code_rule_dsl::logic::plan::run_counts().normalized.load(std::sync::atomic::Ordering::Relaxed);
        (report.chunks, normalized, crate::code_rule_dsl::logic::plan::output::shown(&out, "calls").unwrap())
    };
    // The process's first run of the program does more than any later one (its images made): a warm-up, then
    // each measured run over a store of its own.
    run(1 << 20, "warm-up");
    let (one, once, whole) = run(1 << 20, "one");
    let (few, fewer, chunked) = run(80, "few");
    let (many, normalized, more_chunked) = run(4, "many");
    assert_eq!(one, 1);
    assert!(few >= 2 && many > few, "the chunked runs' chunks: {few}, {many}");
    assert_eq!(chunked, whole, "the findings of one chunk");
    assert_eq!(more_chunked, whole, "the findings of one chunk");
    assert!(whole.len() >= 4, "{whole:?}");
    // The same at any chunk count (the file-local part's once, the cross part's, ...), none more a chunk; one chunk's
    // run takes a part of its own besides.
    assert_eq!(normalized, fewer, "{normalized} normalizations for {many} chunks, {fewer} for {few}");
    assert!(normalized <= once, "{normalized} normalizations for {many} chunks, {once} for one");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The import closure (`components.rerail`, decision 0450 §3): a program that reads `rr_cx_reach` has its rules added. b
/// imports a; c imports nothing. The cross-file memo's pragma is refused: the memo is removed.
#[test]
fn a_program_reading_the_import_closure_has_its_rules() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("import-closure");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "def h():\n    pass\n");
    let program = |text: &str| {
        let rule = dir.join("uses.rerail");
        std::fs::write(&rule, text).unwrap();
        crate::code_rule_dsl::logic::rr::parse_file_version(&rule).map(|p| crate::code_rule_dsl::logic::rr::print_version(&p).unwrap())
    };
    let out = dir.join("out");
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let run = |text: &str| super::run(text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
    let text = program(
        r#"
#include "core.rerail"
.decl seen(p: symbol, q: symbol)
seen(p, q) :- rr_cx_reach(d, e), d != e, doc(d, p, _), doc(e, q, _).
.output seen
"#,
    )
    .unwrap();
    run(&text).unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "seen").unwrap(), [["b.py", "a.py"]]);
    let err = program(".pragma \"cross-memo\" \"seen\"\n.decl seen(p: symbol)\nseen(p) :- doc(_, p, _).\n.output seen\n").err().map(|e| format!("{e:?}")).unwrap_or_default();
    assert!(err.contains("cross-memo") && err.contains("removed"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0099, the factorized store across the chunk boundary. `outer`'s join of `inside(f, p)` and
/// `bound(p, x)` is handed on as its two parts: the findings are those of the product handed on whole, the memo's
/// entries hold the parts (whose join is the product's rows) and not the product, and a run whose choice differs
/// from the warm memo's (`factorize: off` then on, and back) misses the memo and finds the same.
#[cfg(feature = "tape-cache")]
#[test]
#[ignore = "O-factorize-nothing: `uses` is no longer file-local, so no export is handed on as its parts and the factorized run's plan is the unfactorized one's"]
fn an_export_handed_on_as_its_parts_finds_the_same_and_a_changed_choice_misses_the_memo() {
    use serde_json::{Value, json};
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("factorized");
    std::fs::create_dir_all(&dir).unwrap();
    let body = "def f():\n    x = 1\n    y = 2\n    print(x, y)\n    def g():\n        print(x, y)\n        z = x\n    return y\n";
    std::fs::write(dir.join("a.py"), format!("{marker}{body}")).unwrap();
    std::fs::write(dir.join("b.py"), format!("{marker}{body}def h():\n    x = 3\n    print(x)\n")).unwrap();
    let rule = dir.join("outer.rerail");
    // `outer`: a statement of a function's body, and a name the function assigns that some file reads. Its
    // file-local join, `encl(f, p), bound(p, x)`, meets at the function `p`, which it projects away. Its
    // cross-file filter, `named(x)`, binds only `x`: one that bound `f` and `x` both (`uses(f, x)`) would keep the
    // join in the cross part, unpushed (t31 A1), and nothing would be handed on to factorize.
    std::fs::write(&rule, r#"
.decl defined(s: symbol)
defined(s) :- kind(n, "identifier"), text(n, s).
.decl named(x: symbol)
named(x) :- kind(i, "identifier"), text(i, x), defined(x).
.decl encl(f: node, p: node)
encl(f, p) :- kind(p, "function_definition"), child(p, b, _), kind(b, "block"), child(b, f, _).
.decl bound(p: node, x: symbol)
bound(p, x) :- kind(p, "function_definition"), child(p, b, _), kind(b, "block"), child(b, e, _), child(e, a, _), kind(a, "assignment"), field(a, "left", l), text(l, x).
.decl outer(f: node, x: symbol)
outer(f, x) :- named(x), encl(f, p), bound(p, x).
.output outer
"#).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let store = dir.join("store.sqlite");
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.clone()));
    let run = |on: bool| {
        let options = if on { Value::Null } else { json!({"factorize": "off"}) };
        let report = super::run_with_options(&[&program], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &super::Typing::default(), &options).unwrap();
        (report.memo.0, crate::code_rule_dsl::logic::plan::output::shown(&out, "outer").unwrap())
    };
    let (hits, whole) = run(false);
    assert_eq!(hits, 0, "a new memo");
    assert!(whole.len() >= 4, "{whole:?}");
    assert_eq!(run(false), (2, whole.clone()), "the same choice: every file from the memo");
    assert_eq!(run(true), (0, whole.clone()), "factorized: another file-local part, so every file a miss, and the same findings");
    assert_eq!(run(true), (2, whole.clone()), "the factorized run's own entries");
    // The rows the memo holds: of the parts, as the last run left them; then of the product, after a run with it.
    let bound = |prefix: &str| -> (Vec<String>, Vec<Vec<Value>>) {
        let mut executor = crate::code_rule_dsl::sql_executor::CodeRuleSqlExecutor::new(store.to_string_lossy().into_owned()).unwrap();
        let got: Value = serde_json::from_str(&executor.bind_corpus(dir.to_string_lossy().into_owned(), None).unwrap()).unwrap();
        let mut names: Vec<String> = got["relations"].as_object().unwrap().keys().filter(|r| r.starts_with(prefix)).cloned().collect();
        names.sort();
        let rows = names
            .iter()
            .map(|r| serde_json::from_str::<Value>(&executor.query(format!("SELECT * FROM \"{r}\""), None, None).unwrap()).unwrap()["rows"].as_array().unwrap().clone())
            .collect::<Vec<Vec<Value>>>()
            .into_iter()
            .flatten()
            .map(|r| r.as_array().unwrap().clone())
            .collect();
        (names, rows)
    };
    let (parts, _) = bound("fz");
    assert_eq!(parts.len(), 2, "two parts handed on: {parts:?}");
    assert!(bound("fl").0.iter().all(|r| !r.contains("_outer")), "no product handed on");
    // Each part's rows: (the head's column, the key); a node column names the row's document.
    let part = |name: &str| bound(name).1.into_iter().map(|r| (r[0].to_string(), r[1].to_string())).collect::<std::collections::BTreeSet<_>>();
    let (a, b) = (part(&parts[0]), part(&parts[1]));
    let mut joined: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    for (u, k) in &a {
        for (v, k2) in &b {
            if k == k2 {
                joined.insert((u.clone(), v.clone()));
            }
        }
    }
    // The parts are reduced: every row stored joins.
    assert!(a.iter().all(|(_, k)| b.iter().any(|(_, k2)| k == k2)) && b.iter().all(|(_, k)| a.iter().any(|(_, k2)| k == k2)));
    assert_eq!(run(false), (2, whole.clone()), "back to the product: its entries are still kept");
    let (products, _) = bound("fl");
    let product: Vec<&String> = products.iter().filter(|r| r.contains("_outer")).collect();
    assert_eq!(product.len(), 1, "{products:?} of {:?}", bound("").0);
    let rows: std::collections::BTreeSet<(String, String)> = bound(product[0]).1.into_iter().map(|r| (r[0].to_string(), r[1].to_string())).collect();
    assert!(!rows.is_empty());
    // The product's rows, in either order of the head's columns, are the parts' join.
    let flipped: std::collections::BTreeSet<(String, String)> = joined.iter().map(|(u, v)| (v.clone(), u.clone())).collect();
    assert!(rows == joined || rows == flipped, "the parts' join is the product: {} rows against {}", joined.len(), rows.len());
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// t33 K4: with no session, a run starts from the cross-file part its store's frames hold and maintains it from
/// the delta of its inputs, as a session does from memory. After each edit the findings are those of a run with
/// an empty store, and an edit's strata are maintained from deltas; an import cycle made and broken maintains a
/// recursive stratum.
#[test]
fn the_cross_file_part_is_maintained_from_its_frames_in_a_new_process() {
    use crate::code_rule_dsl::typing::file_local::Binding;
    // It ran past its timeout once in 34 runs under load, with no output: a run past 60 s leaves its stacks.
    let _watchdog = watchdog("k4", std::time::Duration::from_secs(60));
    let base = std::env::temp_dir().join(format!("rerail-k4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let out = base.join("out");
    let findings = || {
        let mut rows: Vec<Vec<String>> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap_or_default();
        rows.sort();
        rows
    };
    // Each run is a new process's as far as the cross-file part goes: no session, the store on disk. One with a
    // store of its own keeps nothing.
    // Each scenario keeps its own store: relations are shared by identity across programs (0103), so a later
    // scenario's first run would find clean those of an earlier one whose inputs have the same rows.
    let fresh = std::cell::Cell::new(0);
    let scenario = std::cell::Cell::new(0);
    let run = |texts: &[&str], src: &std::path::Path, kept: bool| {
        let path = if kept {
            base.join(format!("kept{}.sqlite", scenario.get()))
        } else {
            fresh.set(fresh.get() + 1);
            base.join(format!("fresh{}.sqlite", fresh.get()))
        };
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(path));
        let report = super::run_all(texts, src, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        (report, findings())
    };
    let edit = |texts: &[&str], src: &std::path::Path, edits: &[(&str, &str)]| {
        scenario.set(scenario.get() + 1);
        let (first, expected) = run(texts, src, true);
        let (ran, all) = first.cross_kept.expect("a run with a store keeps its cross-file part");
        assert_eq!(ran, all, "the first run evaluates every stratum");
        assert_eq!(run(texts, src, false).1, expected);
        let (mut maintained, mut recursive) = (0, 0);
        for (path, text) in edits {
            std::fs::write(src.join(path), text).unwrap();
            let (report, got) = run(texts, src, true);
            let (_, want) = run(texts, src, false);
            assert_eq!(got, want, "after editing {path}, the findings are those of a run with an empty store");
            eprintln!("{path}: {:?} strata, {:?} maintained (recursive, derived again; {:?}); {} findings", report.cross_kept, report.cross_maintained, report.cross_again, got.len());
            maintained += report.cross_maintained.0;
            recursive += report.cross_maintained.1;
        }
        (maintained, recursive)
    };
    // Joins and negations: two bandit rules.
    let src = base.join("src");
    std::fs::create_dir_all(src.join("pkg")).unwrap();
    std::fs::write(src.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(src.join("pkg/loader.py"), "import pickle as p\n\ndef load(b):\n    return p.loads(b)\n").unwrap();
    std::fs::write(src.join("main.py"), "from pkg.loader import load\nload(b'x')\n").unwrap();
    std::fs::write(src.join("other.py"), "import json\njson.loads('1')\n").unwrap();
    let rules = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
    let programs: Vec<String> = ["bandit-b301-pickle.rerail", "bandit-b307-eval.rerail"].iter().map(|r| crate::code_rule_dsl::logic::settings::program_text(&rules.join(r)).unwrap()).collect();
    let texts: Vec<&str> = programs.iter().map(String::as_str).collect();
    let (maintained, _) = edit(
        &texts,
        &src,
        &[
            ("other.py", "import json\nimport pickle\njson.loads('1')\npickle.loads(b'2')\n"),
            ("main.py", "from pkg.loader import load\nload(b'x')\neval('1')\n"),
            ("pkg/loader.py", "import marshal as p\n\ndef load(b):\n    return p.loads(b)\n"),
        ],
    );
    assert!(maintained > 0, "an edit's strata are maintained from deltas");
    // Recursion: an import cycle, broken and closed again.
    let src = base.join("cycle");
    std::fs::create_dir_all(&src).unwrap();
    for (path, text) in [("a.py", "from b import x\n"), ("b.py", "from c import y\n"), ("c.py", "from a import z\n"), ("d.py", "x = 1\n")] {
        std::fs::write(src.join(path), text).unwrap();
    }
    let rule = base.join("reach.rerail");
    std::fs::write(
        &rule,
        ".policy \"test-import-cycle\" revision 1\n.description \"An import that closes a cycle of imports.\"\n.language python\n.mode steer\n\
         .scope include \"**/*.py\"\n#include \"python/names.rerail\"\n.decl dep(a: number, b: number)\ndep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).\n\
         .decl reach(a: number, b: number) no_reach\nreach(a, b) :- dep(a, b).\nreach(a, c) :- reach(a, b), dep(b, c).\n\
         .violation CYC(at: node) code \"CYC\" \"This import closes a cycle of imports.\"\nCYC(i) :- impmod(i, m), docof(i, a), doc(b, _, m), reach(b, a).\n",
    )
    .unwrap();
    let program = crate::code_rule_dsl::logic::settings::program_text(&rule).unwrap();
    let (_, recursive) = edit(&[program.as_str()], &src, &[("c.py", "from d import z\n"), ("d.py", "from a import w\n"), ("b.py", "x = 2\n"), ("b.py", "from c import y\n")]);
    assert!(findings().len() >= 3, "the cycle's imports: {:?}", findings());
    assert!(recursive > 0, "the import cycle's stratum is maintained");
    let _ = std::fs::remove_dir_all(&base);
}

/// A guard that, when the test it guards runs past `after`, samples this process's stacks (macOS `sample`) into
/// a file beside the temp dir and says where: a hang leaves what each thread was doing. Dropped, it does nothing.
/// Imports held as a reach index (decision 0119), declared `reach` so a rule that lost the form fails, not runs as
/// rows. Its readers probe it every way: both columns (`CYC`, an import that closes a cycle), the start alone
/// (`TOF`, a document that reaches `f`), and the relation through it (`SEESE`, a document that can see module `e`).
/// `cyclic`, `reachesf` and `seese` read nothing an edit changes but the index (`no_inline`: strata of their own),
/// so a run that kept them clean when the index changed would give stale findings.
pub(super) const REACH_RULE: &str = r#".policy "test-reach-index" revision 1
.description "Imports through the reach index."
.language python
.mode steer
.scope include "**/*.py"
#include "python/names.rerail"
.decl dep(a: number, b: number)
dep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).
.decl own(a: number)
own(a) :- doc(a, _, _).
.decl reach(a: number, b: number) reach
reach(a, a) :- own(a).
reach(a, c) :- reach(a, b), dep(b, c).
.decl modof(e: number, q: symbol)
modof(e, q) :- doc(e, _, q).
.decl sees(a: number, q: symbol) reach
sees(a, q) :- reach(a, e), modof(e, q).
.decl cyclic(a: number) no_inline
cyclic(a) :- reach(a, b), reach(b, a), a != b.
.decl reachesf(a: number) no_inline
reachesf(a) :- reach(a, f), doc(f, _, "f").
.decl seese(a: number) no_inline
seese(a) :- sees(a, "e").
.violation CYC(at: node) code "CYC" "This import closes a cycle of imports."
CYC(i) :- impmod(i, m), docof(i, a), doc(b, _, m), reach(b, a).
.violation INCYC(at: node) code "INCYC" "This import is in a document on a cycle."
INCYC(i) :- impmod(i, _), docof(i, a), cyclic(a).
.violation TOF(at: node) code "TOF" "This import is in a document that reaches f."
TOF(i) :- impmod(i, _), docof(i, a), reachesf(a).
.violation SEESE(at: node) code "SEESE" "This import is in a document that can see module e."
SEESE(i) :- impmod(i, _), docof(i, a), seese(a).
.violation DIRECT(at: node) code "DIRECT" "This import is in a document that reaches f, read in place."
DIRECT(i) :- impmod(i, _), docof(i, a), reach(a, f), doc(f, _, "f"), sees(a, "e").
"#;

/// Two import components (a cycle `a b c`, a chain `d e`) and a lone `f`, then edits that connect them, close a
/// cycle across them, break it, disconnect them, make a second cycle, and merge every cycle into one.
pub(super) const REACH_START: [(&str, &str); 6] =
    [("a.py", "from b import x\n"), ("b.py", "from c import y\n"), ("c.py", "from a import z\n"), ("d.py", "from e import w\n"), ("e.py", "x = 1\n"), ("f.py", "x = 1\n")];
pub(super) const REACH_EDITS: [(&str, &str, &str); 10] = [
    ("c.py", "from a import z\nfrom d import v\n", "connect the cycle to the chain"),
    ("e.py", "from a import u\n", "close a cycle across both: a b c d e"),
    ("c.py", "from d import v\n", "break the first cycle (a b c d e still one)"),
    ("e.py", "x = 2\n", "break the last cycle"),
    ("c.py", "x = 3\n", "disconnect the parts"),
    ("e.py", "from d import t\n", "a second cycle, d e"),
    ("b.py", "from c import y\nfrom d import s\n", "connect a b c to the d e cycle"),
    ("f.py", "from a import r\n", "connect the lone f"),
    ("a.py", "from b import x\nfrom f import q\n", "a cycle a f"),
    ("d.py", "from a import p\nfrom e import w\n", "merge every cycle into one"),
];

/// The reach index read again after each edit: from a store a new process reads (the frames), every run's findings
/// those of a run with an empty store, whether the edit changed a cycle, joined two parts or split them.
#[test]
fn the_reach_index_is_derived_again_when_cycles_and_connections_change_in_a_new_process() {
    use crate::code_rule_dsl::typing::file_local::Binding;
    let _watchdog = watchdog("reach", std::time::Duration::from_secs(90));
    let base = std::env::temp_dir().join(format!("rerail-reach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (src, out) = (base.join("src"), base.join("out"));
    std::fs::create_dir_all(&src).unwrap();
    for (path, text) in REACH_START {
        std::fs::write(src.join(path), text).unwrap();
    }
    let rule = base.join("reach.rerail");
    std::fs::write(&rule, REACH_RULE).unwrap();
    let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
    let findings = || {
        let mut rows: Vec<Vec<String>> = crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap_or_default();
        rows.sort();
        rows
    };
    let fresh = std::cell::Cell::new(0);
    let run = |kept: bool| {
        let path = if kept {
            base.join("kept.sqlite")
        } else {
            fresh.set(fresh.get() + 1);
            base.join(format!("fresh{}.sqlite", fresh.get()))
        };
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(path));
        super::run_all(&[program.as_str()], &src, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        findings()
    };
    let first = run(true);
    assert_eq!(run(false), first);
    assert!(first.iter().any(|f| f.iter().any(|v| v.contains("CYC"))), "the first cycle's imports: {first:?}");
    let mut codes = std::collections::BTreeSet::new();
    for (path, text, what) in REACH_EDITS {
        std::fs::write(src.join(path), text).unwrap();
        let got = run(true);
        let want = run(false);
        assert_eq!(got, want, "after editing {path} ({what}), the findings are those of a run with an empty store");
        codes.extend(got.iter().flat_map(|f| ["CYC", "INCYC", "TOF", "SEESE", "DIRECT"].into_iter().filter(|c| f.iter().any(|v| v.contains(c)))));
    }
    assert_eq!(codes.len(), 5, "every reader's findings appeared along the edits: {codes:?}");
    let _ = std::fs::remove_dir_all(&base);
}

pub(super) fn watchdog(name: &str, after: std::time::Duration) -> Watchdog {
    let done = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let seen = std::sync::Arc::clone(&done);
    let name = name.to_owned();
    std::thread::spawn(move || {
        let (lock, cv) = &*seen;
        let (finished, _) = cv.wait_timeout_while(lock.lock().unwrap(), after, |d| !*d).unwrap();
        if *finished {
            return;
        }
        let out = std::env::temp_dir().join(format!("rerail-watchdog-{name}-{}.txt", std::process::id()));
        let sampled = std::process::Command::new("sample").arg(std::process::id().to_string()).arg("3").arg("-file").arg(&out).output();
        eprintln!("watchdog {name}: still running after {after:?}; stacks in {} ({:?})", out.display(), sampled.map(|o| o.status));
    });
    Watchdog(done)
}

pub(super) struct Watchdog(std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);

impl Drop for Watchdog {
    fn drop(&mut self) {
        let (lock, cv) = &*self.0;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_all();
    }
}

/// Decision 0077 §4: with no session, the cross part's strata are kept in the tape cache. A run over unchanged
/// files evaluates fewer strata than the first; an edit that changes no cross-file row leaves them clean too; an
/// edit that does derives what a run with an empty cache derives.
#[test]
fn cross_file_strata_are_kept_across_runs_in_the_tape_cache() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("kept-strata");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "def h():\n    pass\n");
    let rule = dir.join("uses.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl used(f: node)
used(f) :- uses(_, f).
.decl unused(f: node)
unused(f) :- def(f, _), !used(f).
.output uses
.output unused
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
        (report.cross_kept, [read("uses"), read("unused")])
    };
    let (first, derived) = run("kept.sqlite");
    let (ran, all) = first.expect("a run with a tape cache keeps its cross-file strata");
    assert_eq!(ran, all, "the first run evaluates every stratum");
    let (again, rows) = run("kept.sqlite");
    assert_eq!(rows, derived);
    assert!(again.unwrap().0 < all, "unchanged: strata kept ({again:?} of {all})");
    // c's function returns: no cross-file row changes.
    write("c.py", "def h():\n    return 1\n");
    let (edited, rows) = run("kept.sqlite");
    assert_eq!(rows, derived);
    assert!(edited.unwrap().0 < all, "no cross-file row changed: strata kept ({edited:?} of {all})");
    // b no longer imports f: uses is empty, f unused; as a run with no kept strata derives.
    write("b.py", "def g():\n    pass\n");
    let (_, rows) = run("kept.sqlite");
    let (_, fresh) = run("fresh.sqlite");
    assert_eq!(rows, fresh);
    assert_ne!(rows, derived);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `finding` of another shape than the findings form (`finding(key, n)`) is an output like any other: held in the
/// cross-file part, where no nodes or documents are bound, it is read where it is held, not from a table nothing
/// wrote ("no such table: finding" on internal_dead_code's `finding(category, symbol, n)`).
#[test]
fn a_finding_of_three_columns_held_cross_file_is_read_where_it_is() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("finding3");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def f():\n    pass\n").unwrap();
    std::fs::write(dir.join("b.py"), "from a import f\ndef g():\n    pass\n").unwrap();
    let rule = dir.join("finding.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl used(f: node)
used(f) :- imp(_, q), def(f, q).
.decl finding(category: symbol, q: symbol, n: node)
finding("unused", q, f) :- def(f, q), !used(f).
.output finding
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let rows = crate::code_rule_dsl::logic::plan::output::shown(&out, "finding").unwrap();
    let names: Vec<&str> = rows.iter().map(|row| row[1].as_str()).collect();
    assert!(rows.iter().all(|row| row.len() == 3 && row[0] == "unused"), "{rows:?}");
    assert!(names.iter().any(|q| q.ends_with("g")) && !names.iter().any(|q| q.ends_with(".f")), "g is unused, f is imported: {rows:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0434 §4, step 6: a run's parses are stored with their presences after them, each tape's home as one: a
/// tape read back from the store holds its document's kinds.
#[test]
fn a_runs_tapes_are_stored_with_their_presences() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("stored-presences");
    std::fs::create_dir_all(&dir).unwrap();
    let bodies = [("a.py", "def f():\n    pass\n"), ("b.py", "class B:\n    x = 1\n"), ("c.py", "print(1)\n")];
    for (file, body) in bodies {
        std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    }
    let rule = ".decl f(n: node)\nf(n) :- kind(n, \"function_definition\").\n.output f\n";
    let out = dir.join("out");
    let store = dir.join("stored.sqlite");
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.clone()));
    super::run(rule, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
    let cache = crate::content_store::ContentStore::open(&store).unwrap();
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let tag = i64::from(crate::code_rule_dsl::language::tape_tag(python).unwrap());
    let (gv, fv) = (crate::code_rule_dsl::build_ids::grammar_version(), i64::from(crate::code_rule_dsl::tape::formats::stored_version()));
    // The kind of a `function_definition` node.
    let function = ast_grep_language::LanguageExt::get_ts_language(&python).id_for_node_kind("function_definition", true);
    for (file, body) in bodies {
        let source = cache.source_by_hash(crate::content_store::HashKey::sha256(format!("{marker}{body}").as_bytes())).unwrap().expect("the source, stored");
        let frame = cache.tape_frames_for(&[source], tag, gv, fv).unwrap().remove(0).unwrap_or_else(|| panic!("{file}'s tape, stored"));
        let bytes = frame.bytes().unwrap();
        let presence = crate::code_rule_dsl::tape::stored_presence(&bytes, tag as u8).unwrap_or_else(|| panic!("{file}'s presence, stored with its tape"));
        assert_eq!(presence.has_kind(u32::MAX, function), body.starts_with("def"), "{file}: its document's kinds");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The cone of requests is kept like any relation (t33 K1, K3): its stratum that asks for directories, files and
/// loads and answers them inside its fixpoint is clean while what it reads and its answers are as they were. An
/// edit that changes neither evaluates none of the cone; a new import, or a library file whose own imports
/// change, runs it, and the documents loaded are those a run with nothing kept loads.
#[test]
fn the_cone_of_requests_is_kept_while_its_reads_and_answers_are() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("cone-kept");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\na: 3.0-\nb: 3.0-\nc: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\n").unwrap();
    for m in ["a", "b", "c"] {
        std::fs::write(lib.join(format!("{m}.pyi")), format!("def {m}() -> None: ...\n")).unwrap();
    }
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(ws.join("main.py"), "import a\n").unwrap();
    let rule = dir.join("p.rerail");
    std::fs::write(&rule, "#include \"python/modules.rerail\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        // Each loaded document's file name.
        let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::open(&out).unwrap();
        let mut loaded: Vec<String> = outputs.rows("n_loaded").iter().map(|r| std::path::Path::new(r.get(0).text().expect("a path")).file_name().unwrap().to_string_lossy().into_owned()).collect();
        drop(outputs);
        loaded.sort();
        (report.cone_kept.expect("a run with a tape cache keeps its cone's strata"), loaded)
    };
    let ((ran, all), first) = run("kept.sqlite");
    // The first run evaluates the cone's strata that phase 1 did not: phase 1 (`sources.rerail`) resolves the same
    // imports first, and a relation is kept under its identity, not its program's (decision 0103).
    assert!(ran > 0 && ran <= all, "the first run evaluates the cone ({ran} of {all} strata)");
    assert_eq!(first, ["a.pyi", "builtins.pyi"]);
    // A comment: the cone reads nothing new, and its answers are as they were.
    std::fs::write(ws.join("main.py"), "import a\n# a note\n").unwrap();
    let ((ran, all), same) = run("kept.sqlite");
    assert_eq!((ran, &same), (0, &first), "an edit that changes no read and no answer evaluates none of the cone (of {all})");
    // A new import: a read changes, and the cone runs.
    std::fs::write(ws.join("main.py"), "import a\nimport b\n# a note\n").unwrap();
    let ((ran, _), grown) = run("kept.sqlite");
    assert!(ran > 0, "a new import runs the cone");
    assert_eq!(grown, run("fresh-1.sqlite").1, "the documents loaded are those a run with nothing kept loads");
    assert_eq!(grown, ["a.pyi", "b.pyi", "builtins.pyi"]);
    // A library file's own import: the answer to its load changes, and the cone runs.
    std::fs::write(lib.join("b.pyi"), "import c\ndef b() -> None: ...\n").unwrap();
    let ((ran, _), deeper) = run("kept.sqlite");
    assert!(ran > 0, "a loaded document's change is a changed answer: the cone runs");
    assert_eq!(deeper, ["a.pyi", "b.pyi", "builtins.pyi", "c.pyi"]);
    assert_eq!(deeper, run("fresh-2.sqlite").1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0145: the cone run as rounds from the last run's final round finds what the gated fixpoint over
/// nothing kept finds, edit after edit: a new import, a library's own import, an import taken away (its load is
/// not asked again), and a library file that goes.
#[test]
fn the_cone_maintained_by_rounds_loads_what_a_fresh_cone_loads() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("cone-rounds");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(ws.join("pkg")).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\na: 3.0-\nb: 3.0-\nc: 3.0-\nd: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\n").unwrap();
    for m in ["a", "b", "c", "d"] {
        std::fs::write(lib.join(format!("{m}.pyi")), format!("def {m}() -> None: ...\n")).unwrap();
    }
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(ws.join("main.py"), "import a\nimport pkg.m\n").unwrap();
    std::fs::write(ws.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(ws.join("pkg/m.py"), "import b\n").unwrap();
    let rule = dir.join("p.rerail");
    std::fs::write(&rule, "#include \"python/modules.rerail\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let fresh = std::cell::Cell::new(0);
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        // Each loaded document's file name.
        let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::open(&out).unwrap();
        let mut loaded: Vec<String> = outputs.rows("n_loaded").iter().map(|r| std::path::Path::new(r.get(0).text().expect("a path")).file_name().unwrap().to_string_lossy().into_owned()).collect();
        drop(outputs);
        loaded.sort();
        (report.cone_rounds, loaded)
    };
    let both = |what: &str, want: &[&str]| {
        let (rounds, kept) = run("kept.sqlite");
        fresh.set(fresh.get() + 1);
        let (none, again) = run(&format!("fresh-{}.sqlite", fresh.get()));
        assert!(rounds > 0, "{what}: the kept run runs rounds");
        assert_eq!(none, 0, "{what}: a run with nothing kept runs the fixpoint");
        assert_eq!(kept, again, "{what}: rounds load what the fixpoint loads");
        assert_eq!(kept, want, "{what}");
    };
    assert_eq!(run("kept.sqlite"), (0, vec!["a.pyi".to_owned(), "b.pyi".to_owned(), "builtins.pyi".to_owned()]), "the first run is the fixpoint");
    std::fs::write(ws.join("main.py"), "import a\nimport pkg.m\nimport c\n").unwrap();
    both("a new import", &["a.pyi", "b.pyi", "builtins.pyi", "c.pyi"]);
    std::fs::write(lib.join("c.pyi"), "import d\ndef c() -> None: ...\n").unwrap();
    both("a library's own import", &["a.pyi", "b.pyi", "builtins.pyi", "c.pyi", "d.pyi"]);
    std::fs::write(ws.join("pkg/m.py"), "x = 1\n").unwrap();
    both("an import taken away", &["a.pyi", "builtins.pyi", "c.pyi", "d.pyi"]);
    std::fs::remove_file(lib.join("d.pyi")).unwrap();
    both("a library file gone", &["a.pyi", "builtins.pyi", "c.pyi"]);
    // The workspace's own module now: the library it shadowed was loaded from the last run's gates, and is taken
    // back (decision 0145).
    std::fs::write(ws.join("c.py"), "x = 2\n").unwrap();
    both("a library shadowed by the workspace's module", &["a.pyi", "builtins.pyi"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Q224: a run whose loaded document's chunk fails ends with that chunk's error, not waits. Two loads here, one from
/// a file's lines and one from a directory's entries. The hang needed a second load posted ahead and answered after
/// the first's error (the full shape on Metal, 3 of 13 runs); this program's cone does not post them so, and the test
/// passes without `Builder::next`'s stopped check too: it keeps the error's way out, not that order.
#[test]
fn a_load_whose_chunk_fails_ends_the_run_with_its_error() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("load-fails");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("lib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    let (fails, other) = (lib.join(super::FAILS_ITS_CHUNK), lib.join("b.pyi"));
    std::fs::write(&fails, "def f() -> None: ...\n").unwrap();
    std::fs::write(&other, "def b() -> None: ...\n").unwrap();
    std::fs::write(ws.join("rerail.env"), "test.libraries = ../lib\n").unwrap();
    std::fs::write(ws.join("main.py"), "x = 1\n").unwrap();
    std::fs::write(ws.join("later.txt"), format!("{}\n", fails.display())).unwrap();
    let rule = dir.join("p.rerail");
    std::fs::write(&rule, format!(
        "#include \"core.rerail\"\nload(p, \"f\", \"python\") :- file_line(\"later.txt\", 1, p).\nload(p, \"b\", \"python\") :- dir_entry(\"{lib}\", \"b.pyi\", 0), p = cat(\"{lib}\", \"/b.pyi\").\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n",
        // Spelled with `/` in the rule's string literal, where `\` would start an escape (Windows takes either).
        lib = lib.display().to_string().replace('\\', "/")
    )).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let (out, store) = (dir.join("out"), dir.join("store.sqlite"));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store));
        let r = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
        let _ = tx.send(r.err().map(|e| e.to_string()));
    });
    let err = rx.recv_timeout(std::time::Duration::from_secs(120)).expect("the run ends, not waits for ever");
    let err = err.expect("the run fails");
    assert!(err.contains(super::FAILS_ITS_CHUNK), "the run ends with the chunk's error: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0327, mode (b): a translation unit's answer is kept while the files it read are unchanged. An edit to a
/// header asks again for the units that read it, and for no other; a header made where a unit's search looked first
/// is one it read (its path was looked at). The outputs are those of a run with nothing kept. The workspace is
/// reached through a symbolic link: a unit's reads are kept under their canonical names, and the edit is named
/// under the workspace's as given too (`Observer::changed_since`), as where the temporary directory is a link (macOS's
/// `/var`).
#[test]
fn a_header_edit_asks_again_only_for_the_units_that_read_it() {
    // Its runs compare with each other: the switches held over all of them (`switches::reading`), or a test setting
    // its own on another thread (`RERAIL_INLINE_ONLY`, `RERAIL_CROSS_MEMO`) gave one run a mode another had not.
    let _switches = super::switches::reading();
    crate::install_store_functions();
    // The journal names what changed (decision 0105): the engine's tests run without it, and this one turns it on
    // (after the first observer, which turns it off once a process).
    super::observe(|_| ());
    crate::content_store::observe::set_journal(true);
    // Its answer whatever it costs: slower than the `stat`s, a fresh header's would ask again for its unit (Q221).
    crate::content_store::observe::set_journal_wait_least(std::time::Duration::from_secs(2));
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("c-units-kept");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("real/sys")).unwrap();
    let ws = dir.join("ws");
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("real"), &ws).unwrap();
    #[cfg(not(unix))]
    let ws = dir.join("real");
    std::fs::write(ws.join("one.c"), "#include \"h.h\"\nint one = H;\n").unwrap();
    std::fs::write(ws.join("two.c"), "#include <g.h>\nint two = G;\n").unwrap();
    std::fs::write(ws.join("three.c"), "int three;\n").unwrap();
    std::fs::write(ws.join("h.h"), "#define H 1\n").unwrap();
    std::fs::write(ws.join("sys/g.h"), "#define G 2\n").unwrap();
    let rule = dir.join("p.rerail");
    std::fs::write(&rule, r#"
.language c
#include "c/preprocess.rerail"
.decl c_file(p: symbol)
c_file(p) :- kind(n, "translation_unit"), docof(n, d), doc(d, p, _).
c_lang("c", "c", "gnu17").
c_include_dir("c", 0, "sys", "angle").
c_tu(p, "c") :- c_file(p).
.decl entered(t: symbol, p: symbol)
entered(t, p) :- c_pp_included(t, _, _, _, p, _).
.decl uses(t: symbol, name: symbol, at: symbol)
uses(t, n, p) :- c_pp_expansion(t, _, _, n, p, _, _, _, _).
.decl active(t: symbol, p: symbol, end: number)
active(t, p, e) :- c_pp_active(t, _, p, _, e).
.output entered, uses, active
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut asked: Vec<String> = report.language_asked.iter().filter(|(r, _)| r == "c_pp_tu_req").map(|(_, k)| std::path::Path::new(k).file_name().unwrap().to_string_lossy().into_owned()).collect();
        asked.sort();
        let shown = ["entered", "uses", "active"].map(|r| crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap());
        (asked, shown)
    };
    let (asked, first) = run("kept.sqlite");
    assert_eq!(asked, ["one.c", "three.c", "two.c"], "the first run asks for every unit");
    std::fs::write(ws.join("h.h"), "#define H 3\n#define K 4\n").unwrap();
    let (asked, edited) = run("kept.sqlite");
    assert_eq!(asked, ["one.c"], "an edit to h.h asks again for the unit that read it alone");
    assert_eq!(edited, run("fresh-1.sqlite").1, "the kept answers are a fresh run's");
    // `<g.h>` searches the search path alone, not the includer's directory: a `g.h` made there is no file a unit
    // looked at.
    std::fs::write(ws.join("g.h"), "#define G 5\n").unwrap();
    let (asked, _) = run("kept.sqlite");
    assert_eq!(asked, Vec::<String>::new(), "a file no unit looked at asks for none");
    std::fs::write(ws.join("sys/g.h"), "#define G 6\n").unwrap();
    let (asked, last) = run("kept.sqlite");
    assert_eq!(asked, ["two.c"]);
    assert_eq!(last, run("fresh-2.sqlite").1);
    assert_ne!(first, last);
    let _ = std::fs::remove_dir_all(&dir);
}

/// t32 4.1: a new process maintains its cross-file part from the rows the last process kept, as a session does.
/// After an edit that changes a cross-file row, a changed stratum is maintained from its deltas, not derived
/// again, and the outputs are a run's with an empty store.
#[test]
fn a_new_process_maintains_the_cross_file_part_from_the_kept_rows() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("kept-part");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\ndef k():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "from a import k\ndef h():\n    pass\n");
    let rule = dir.join("uses.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl used(f: node)
used(f) :- uses(_, f).
.decl unused(f: node)
unused(f) :- def(f, _), !used(f).
.output unused
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        (report.cross_maintained.0, crate::code_rule_dsl::logic::plan::output::shown(&out, "unused").unwrap())
    };
    let (cold, derived) = run("kept.sqlite");
    assert_eq!(cold, 0, "nothing kept: every stratum derived");
    // c no longer imports k: k is unused; uses and used are maintained from their deltas.
    write("c.py", "def h():\n    pass\n");
    let (maintained, rows) = run("kept.sqlite");
    let (_, fresh) = run("fresh.sqlite");
    assert_eq!(rows, fresh);
    assert_ne!(rows, derived);
    assert!(maintained > 0, "a changed stratum maintained from the kept rows ({maintained})");
    // And back: the next process maintains from what this one kept.
    write("c.py", "from a import k\ndef h():\n    pass\n");
    let (again, rows) = run("kept.sqlite");
    assert_eq!(rows, derived);
    assert!(again > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0173 (t36 item 7): a maintained run reads its programs as versions, in place: there is no tree form to
/// decode one into (`front::tests::no_tree_program_comes_back`). The outputs are a run's with an empty store.
#[test]
fn a_maintained_run_decodes_and_copies_no_tree() {
    // Its runs compare with each other: the switches held over all of them (`switches::reading`), or a test setting
    // its own on another thread (`RERAIL_INLINE_ONLY`, `RERAIL_CROSS_MEMO`) gave one run a mode another had not. Its
    // child processes take them as they start.
    let _switches = super::switches::reading();
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("no-trees");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\ndef k():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "from a import k\ndef h():\n    pass\n");
    let rule = dir.join("uses.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl used(f: node)
used(f) :- uses(_, f).
.decl unused(f: node)
unused(f) :- def(f, _), !used(f).
.output unused
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        (report.cross_maintained.0, crate::code_rule_dsl::logic::plan::output::shown(&out, "unused").unwrap())
    };
    run("kept.sqlite");
    write("c.py", "def h():\n    pass\n");
    let (maintained, rows) = run("kept.sqlite");
    let (_, fresh) = run("fresh.sqlite");
    assert_eq!(rows, fresh);
    assert!(maintained > 0, "a changed stratum maintained from the kept rows ({maintained})");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0101 §3: a maintained stratum's pieces run as one component per level, ordered by unit gates (each
/// negation of a piece, `used__minus :- used__del, !used__ins`, reads it once it is whole); a stratum the
/// evaluator's maintain mode takes (decision 0364) is one component with no pieces. Edits that remove, add and move
/// imports delete, insert and derive again; after each, the outputs are a run's with an empty store, under the
/// device's binding too.
#[test]
fn a_maintained_strata_pieces_run_as_one_component() {
    // The counts are the run's (decision 0442): the test holds the run they are read in, which each run joins.
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("together");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\ndef k():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "from a import k\ndef h():\n    pass\n");
    let rule = dir.join("uses.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl used(f: node)
used(f) :- uses(_, f).
.decl unused(f: node)
unused(f) :- def(f, _), !used(f).
.decl lone(f: node)
lone(f) :- unused(f), !used(f), def(f, q), !imp(_, q).
.output unused
.output lone
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    use crate::code_rule_dsl::typing::file_local::Binding;
    let run_on = |store: &str, binding: Binding| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        // The run's own scope, held here: the maintain mode's count is the run's (decision 0442), read before it ends.
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        let moded = crate::code_rule_dsl::logic::plan::mode_strata();
        let report = super::run(&text, &dir, None, &out, "program", binding, 1 << 18).unwrap();
        let rows = ["unused", "lone"].map(|r| crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap());
        // A stratum the evaluator's maintain mode takes (decision 0364) has no pieces: it is one component.
        (report.cross_maintained.0, rows, (crate::code_rule_dsl::logic::plan::mode_strata() - moded) as usize)
    };
    let run = |store: &str| run_on(store, Binding::Cpu);
    run("kept.sqlite");
    let mut together = 0;
    let edits = [
        ("c.py", "def h():\n    pass\n"),
        ("b.py", "from a import f\nfrom a import k\ndef g():\n    pass\n"),
        ("c.py", "from b import g\ndef h():\n    pass\n"),
        ("b.py", "def g():\n    pass\n"),
    ];
    for (i, (file, body)) in edits.into_iter().enumerate() {
        write(file, body);
        let (maintained, rows, pieces) = run("kept.sqlite");
        together += pieces;
        let (_, fresh, _) = run(&format!("fresh{i}.sqlite"));
        assert_eq!(rows, fresh, "after edit {i} ({file})");
        assert!(maintained > 0, "edit {i}: a changed stratum maintained from the kept rows");
    }
    assert!(together > 0, "a stratum's pieces ran together, or the maintain mode ran it as one component");
    // Under the device's binding the pieces stay apart: the device takes no gates. The outputs are the same.
    write("c.py", "from a import k\ndef h():\n    pass\n");
    let (maintained, rows, pieces) = run_on("kept.sqlite", Binding::Metal);
    let (_, fresh, _) = run("fresh-metal.sqlite");
    assert_eq!(rows, fresh, "under the device's binding");
    assert!(maintained > 0, "under the device's binding: maintained");
    let _ = pieces;
    let _ = std::fs::remove_dir_all(&dir);
}

/// A recursive stratum is maintained by levels (decision 0090) though one relation's cycle is closed only by a rule
/// over a relation with no rows: the cold run leaves that rule out, but every relation of the stratum keeps its rows'
/// levels, so the next run finds them all. (At O125, `garg` and `gcall` closed their cycle through `tmap`, empty,
/// and their stratum of 61 relations was maintained by delete-and-rederive.) The next run is a new process's, so
/// the levels come back from the store's frames. The outputs are a run's with an empty store.
#[test]
fn a_cycle_closed_by_an_empty_relation_still_keeps_levels() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("levels-kept");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\ndef k():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "from b import g\ndef h():\n    pass\n");
    let rule = dir.join("reach.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl never(n: node)
never(n) :- kind(n, "no_such_kind").
.decl reach(a: node, b: node)
reach(a, b) :- uses(a, b).
reach(a, c) :- reach(a, b), uses(b, c).
.decl side(a: node)
side(a) :- reach(a, _).
reach(a, a) :- side(a), never(a).
.output reach
.output side
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let before = super::maintain::strata_by_levels();
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows = ["reach", "side"].map(|r| crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap());
        (report.cross_maintained, rows, super::maintain::strata_by_levels() - before)
    };
    run("kept.sqlite");
    write("b.py", "from a import f\nfrom a import k\ndef g():\n    pass\n");
    let ((maintained, recursive, _), rows, by_levels) = run("kept.sqlite");
    let (_, fresh, _) = run("fresh.sqlite");
    assert_eq!(rows, fresh);
    assert!(maintained > 0 && recursive > 0, "the recursive stratum is maintained ({maintained}, {recursive})");
    assert!(by_levels > 0, "and by levels: `side` kept its rows' levels");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0364: the evaluator's maintain mode keeps an aggregate by its groups and a rule whose head computes a
/// term. `fanout` counts each definition's users, grouped by the definition; `most` is the largest count, over no
/// group; `named` computes its head. Each edit's run starts from the last run's rows. Its outputs are those of a run
/// with an empty store, and the mode maintained strata (none derived again by a form).
#[test]
fn the_maintain_mode_keeps_aggregates_by_their_groups_and_computed_heads() {
    // The counts are the run's (decision 0442): the test holds the run they are read in, which each run joins.
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("mode-aggregates");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\ndef k():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "from a import f\nfrom b import g\ndef h():\n    pass\n");
    let rule = dir.join("agg.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl fanout(f: node, n: number)
fanout(f, n) :- def(f, _), n = count : { uses(_, f) }.
.decl most(m: number)
most(m) :- fanout(_, _), m = max x : { fanout(_, x) }.
.decl named(n: node, q: symbol)
named(n, @dot(q, q)) :- uses(n, f), def(f, q).
.output fanout
.output most
.output named
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        // The run's own scope, held here: the maintain mode's count is the run's (decision 0442), read before it ends.
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        let before = crate::code_rule_dsl::logic::plan::mode_strata();
        super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows = ["fanout", "most", "named"].map(|r| {
            let mut rows = crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap();
            rows.sort();
            rows
        });
        (rows, crate::code_rule_dsl::logic::plan::mode_strata() - before)
    };
    run("kept.sqlite");
    let edits = [
        ("b.py", "from a import f\nfrom a import k\ndef g():\n    pass\n", "a use of k added: its group's count and the most move"),
        ("c.py", "def h():\n    pass\n", "two uses gone: f's count falls, g's group empties"),
        ("b.py", "def g():\n    pass\n", "every use gone"),
    ];
    for (i, (file, body, what)) in edits.into_iter().enumerate() {
        write(file, body);
        let (rows, mode) = run("kept.sqlite");
        let (fresh, _) = run(&format!("fresh{i}.sqlite"));
        assert_eq!(rows, fresh, "after {what}");
        assert!(mode > 0, "after {what}: the maintain mode maintained strata");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0103 (t33 K1): a relation is kept as a frame under its identity, not its program's. A second program
/// with the same rules for `def`, `imp` and `uses` finds them clean on its first run, and the first program's
/// relations are still clean after it. A rule edit makes new only the relations it reaches.
#[test]
fn a_relation_two_programs_share_is_kept_once_and_a_rule_edit_reaches_only_its_readers() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("frames-shared");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    let shared = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.output uses
"#;
    let unused = ".decl used(f: node)\nused(f) :- uses(_, f).\n.decl unused(f: node)\nunused(f) :- def(f, _), !used(f).\n.output unused\n";
    let named = ".decl named(q: symbol)\nnamed(q) :- uses(_, f), def(f, q).\n.output named\n";
    let out = dir.join("out");
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("kept.sqlite")));
    let run = |extra: &str| {
        let rule = dir.join("p.rerail");
        std::fs::write(&rule, format!("{shared}{extra}")).unwrap();
        let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        report.cross_kept.expect("a run with a tape cache keeps its cross-file strata")
    };
    let (ran, all) = run(unused);
    assert_eq!(ran, all, "the first run evaluates every stratum");
    let (ran_b, all_b) = run(named);
    assert!(ran_b < all_b, "the shared relations are clean for another program ({ran_b} of {all_b})");
    let (ran_a, _) = run(unused);
    assert!(ran_a < all, "the first program's relations are still kept ({ran_a} of {all})");
    // An edit to `unused`'s rule: `uses` and what it reads stay clean.
    let (edited, all_e) = run(&unused.replace("!used(f)", "!used(f), f != 0"));
    assert!(edited < all_e, "a rule edit reruns only what it reaches ({edited} of {all_e})");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0089: a library is held once, and a request's cone once per distinct request. Two programs ask
/// `compare.rerail` the same, a third asks something else, a fourth asks nothing and reads the library's own copy.
#[test]
fn libraries_are_held_once_and_equal_requests_share_one_cone() {
    let ask = |kind: &str, own: &str| {
        format!(
            "#include \"python/compare.rerail\"\ncmp_same_ask(a, b) :- kind(a, \"{kind}\"), kind(b, \"{kind}\"), a < b.\n.decl {own}(a: node)\n{own}(a) :- cmp_same(a, _).\n.output {own}\n"
        )
    };
    let quiet = "#include \"python/compare.rerail\"\n.decl quiet(a: node)\nquiet(a) :- cmp_same(a, _).\n.output quiet\n".to_owned();
    let texts = [ask("call", "one"), ask("call", "two"), ask("name", "three"), quiet];
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let rules = super::combine_texts(&texts).unwrap().heads_and_reads();
    let heads = |rel: &str| rules.iter().filter(|(h, _)| h == rel).count();
    // The library's rule once, unprefixed, whatever the programs read.
    assert_eq!(heads("cmp_same"), 1);
    // Programs 0 and 1 ask the same: one copy of the cone; program 2 its own.
    let copies: std::collections::BTreeSet<&str> = rules.iter().map(|(h, _)| h.as_str()).filter(|r| r.starts_with('q') && r.ends_with("_cmp_same")).collect();
    assert_eq!(copies.len(), 2, "{copies:?}");
    assert!(copies.iter().all(|r| heads(r) == 1));
    assert!(!rules.iter().any(|(h, _)| h.starts_with('p') && h.ends_with("_cmp_same")));
    // Each asker reads its request's copy; the quiet one reads the library's.
    let reads = |head: &str| -> Vec<String> {
        rules.iter().filter(|(h, _)| h == head).flat_map(|(_, reads)| reads.iter().map(|(_, r)| r.clone())).collect()
    };
    let (one, two, three) = (reads("p0_one"), reads("p1_two"), reads("p2_three"));
    assert_eq!(one, two);
    assert_ne!(one, three);
    assert_eq!(reads("p3_quiet"), ["cmp_same"]);
}

/// A library's kind set that its own rules read only as a set (`tsx/react-class.rerail`'s `inside(c, m, keyed_kind)`) is a
/// relation of the library held once all the same: a program that reads it as one (`kind(p, k), keyed_kind(k)`, react's
/// no-access-state-in-setstate) finds its rows in a pack. The library linked alone dropped it, and the react pack
/// failed on `no such table: main.keyed_kind`. PR #508's 363b91c0b (t47).
#[test]
fn a_librarys_kind_set_is_a_relation_its_includers_read_in_a_pack() {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as _;
    let reads = ".language typescript\n#include \"tsx/react-class.rerail\"\n.decl keyed(p: node)\nkeyed(p) :- kind(p, k), keyed_kind(k).\n.output keyed\n";
    let other = ".language typescript\n#include \"tsx/react-class.rerail\"\n.decl state(c: node)\nstate(c) :- set_state_in(c, _).\n.output state\n";
    let v = super::combine_texts(&[reads, other]).unwrap_or_else(|e| panic!("{e}"));
    assert!(reads_of(&v, "p0_keyed").iter().any(|r| r == "keyed_kind"), "{:?}", reads_of(&v, "p0_keyed"));
    assert!(v.declares("keyed_kind"), "keyed_kind is no relation of the pack");
    assert_eq!(v.fact_rows("keyed_kind").len(), 5);
}

/// A value cycle that a library relation bounds (`deparen(e, a)`: a is e or below it) is bounded in a pack too, where
/// a program is lowered without its library's rules: typescript-eslint's no-duplicate-enum-values (`enum_number`) and
/// vitest's expect-expect (`name_of`) failed the typescript pack as unbounded value cycles, though each is bounded
/// alone. PR #508's 363b91c0b (t47).
#[test]
fn a_value_cycle_a_library_relation_bounds_is_bounded_in_a_pack() {
    let cycle = ".language typescript\n#include \"tsx/ts-b.rerail\"\n\
                 .decl ua(x: node, a: node)\nua(x, a) :- kind(x, \"unary_expression\"), field(x, \"argument\", e), deparen(e, a).\n\
                 .decl num(x: node, f: float)\nnum(x, to_float(v)) :- kind(x, \"number\"), value(x, v).\n\
                 num(x, 0.0 - f) :- ua(x, a), num(a, f).\n.output num\n";
    let other = ".language typescript\n#include \"tsx/ts-b.rerail\"\n.decl d(e: node, a: node)\nd(e, a) :- deparen(e, a).\n.output d\n";
    super::combine_texts(&[cycle]).unwrap_or_else(|e| panic!("alone: {e}"));
    let v = super::combine_texts(&[cycle, other]).unwrap_or_else(|e| panic!("in a pack: {e}"));
    assert!(v.heads_and_reads().iter().any(|(h, _)| h == "p0_num"));
}

/// A program that states facts of a library relation (`lit_repr_threshold(16384).`, an `extensible` one) asks of the
/// library as one that derives it does (decision 0089 §3): its readers in the library (`dec_lit_repr`) are copied into
/// its request, over its facts. Taken as the library's, they read the library's relation without them, and
/// clippy-decimal-literal-representation found nothing in a pack. PR #508's 363b91c0b (t47).
#[test]
fn a_programs_facts_of_a_library_relation_ask_of_the_library() {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as _;
    let stating = ".language rust\n#include \"rust/syntax-literals.rerail\"\n.decl hit(n: node)\nhit(n) :- dec_lit_repr(n).\n.output hit\nlit_repr_threshold(16384).\n";
    let other = ".language rust\n#include \"rust/syntax-literals.rerail\"\n.decl lit(n: node, v: number)\nlit(n, v) :- int_lit_value(n, v).\n.output lit\n";
    let v = super::combine_texts(&[stating, other]).unwrap_or_else(|e| panic!("{e}"));
    let read = reads_of(&v, "p0_hit");
    let copy = read.iter().find(|r| r.starts_with('q') && r.ends_with("_dec_lit_repr")).unwrap_or_else(|| panic!("p0_hit reads {read:?}"));
    assert!(!read.iter().any(|r| r == "dec_lit_repr"), "p0_hit reads the library's dec_lit_repr: {read:?}");
    let threshold = copy.replace("_dec_lit_repr", "_lit_repr_threshold");
    assert_eq!(v.fact_rows(&threshold).len(), 1, "{threshold}: {:?}", v.fact_names().into_iter().filter(|k| k.contains("threshold")).collect::<Vec<_>>());
}

/// A library's kind set that a program reads by `nearest` (ruff-n807's `nearest(f, s, scope)` over python/names.rerail's
/// `scope`) has the library's kinds in a pack, as alone. Lowered in its library's scope, the program was seeded the
/// set's declaration but not its facts, so it held `scope` empty, and its `nearest` climbed to nothing: ruff-n807,
/// RUF033, AIR303 and F406 found nothing in the python pack. PR #508's afeb076e5 (t47).
#[test]
fn a_programs_nearest_over_a_library_kind_set_finds_in_a_pack_what_it_finds_alone() {
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as _;
    let near = ".language python\n#include \"python/names.rerail\"\n.decl top(x: symbol)\n\
                top(x) :- kind(f, \"function_definition\"), field(f, \"name\", c), text(c, x), nearest(f, s, scope), kind(s, \"module\").\n.output top\n";
    let other = ".language python\n#include \"python/names.rerail\"\nrotgt(s, \"zz\") :- kind(s, \"module\").\n\
                 .decl bound(x: symbol)\nbound(x) :- bound_in(_, x).\n.output bound\n";
    let (alone, together) = alone_and_together("library-kind-set", ("a.py", "def f():\n    def g():\n        pass\nclass C:\n    def m(self):\n        pass\n"), near, other, "top");
    // Every set a rule climbs to is a set of the combined program, with its kinds.
    let v = super::combine_texts(&[near, other]).unwrap_or_else(|e| panic!("{e}"));
    let sets = v.kindsets();
    for r in v.rules() {
        let shown = v.show_rule(r);
        for (_, rest) in shown.match_indices("nearest(").map(|(i, _)| shown.split_at(i)) {
            let set = rest.split(')').next().and_then(|args| args.rsplit(", ").next()).unwrap_or("");
            assert!(sets.get(set).is_some_and(|k| !k.is_empty()), "{shown}: {set} is no set of the pack: {:?}", sets.get(set));
        }
    }
    assert_eq!(alone, [["f"]], "f's nearest scope is the module; g's is f, m's is C");
    assert_eq!(together, alone);
}

/// A library relation of kinds that no library rule climbs to (eslint-a-control.rerail's `breakable_kind(k) :-
/// loop_kind(k)`), and a program's own set over one (`stop(k) :- function_kind(k)`), hold the library's kinds where a
/// program climbs to them in a pack, as alone: eslint's no-extra-label found nothing in a pack, no-unsafe-finally
/// climbed past functions. PR #508's 4146acbd9 (t47).
#[test]
fn a_programs_nearest_over_a_library_relation_of_kinds_finds_in_a_pack_what_it_finds_alone() {
    let near = ".language typescript\n#include \"tsx/eslint-a-control.rerail\"\n.decl left(k: symbol)\n\
                left(k) :- kind(j, \"break_statement\"), nearest(j, s, breakable_kind), kind(s, k).\n.output left\n\
                .decl stop(k: symbol)\nstop(\"program\").\nstop(k) :- function_kind(k).\n\
                .decl stopped(k: symbol)\nstopped(k) :- kind(j, \"return_statement\"), nearest(j, s, stop), kind(s, k).\n.output stopped\n";
    let other = ".language typescript\n#include \"tsx/eslint-a-control.rerail\"\n.decl j(n: node)\nj(n) :- jump(n, _).\n.output j\n";
    let ts = "function f() { for (;;) { if (a) { break; } return 1; } }\nswitch (b) { case 1: break; }\n";
    let (alone, together) = alone_and_together("library-kind-relation", ("a.ts", ts), near, other, "left");
    assert_eq!(alone, [["for_statement"], ["switch_statement"]]);
    assert_eq!(together, alone);
    let (alone, together) = alone_and_together("library-kind-relation-own", ("a.ts", ts), near, other, "stopped");
    assert_eq!(alone, [["function_declaration"]]);
    assert_eq!(together, alone);
}

/// `first`'s `rel` over the one file `file` (its name and text): run alone, and in a pack with `other`.
fn alone_and_together(name: &str, file: (&str, &str), first: &str, other: &str, rel: &str) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file.0), file.1).unwrap();
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let run = |programs: &[&str], out: &str| {
        let out = dir.join(out);
        super::run_all(programs, &dir, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{e}"));
        let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default();
        let rows = read(&format!("p0_{rel}"));
        if rows.is_empty() { read(rel) } else { rows }
    };
    let alone = run(&[first], "alone");
    let together = run(&[first, other], "together");
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
    (alone, together)
}

/// A `match_all` over a library relation's node (FAST003's over `route_path`'s `s`, a string by `plain_str`) scans
/// the spans the library's rules bind it to in a pack, as alone: the program held the library relation declared and
/// not derived, ran no pattern, and ruff-fast003 found nothing in a pack. PR #508's 4146acbd9 (t47).
#[test]
fn a_match_over_a_library_relations_node_finds_in_a_pack_what_it_finds_alone() {
    let words = ".language python\n#include \"python/ast.rerail\"\n.decl word(k: symbol)\n\
                 word(k) :- plain_str(s, _), match_all(\"(?<K>[a-z]+)\", s), text(K, k).\n.output word\n";
    let other = ".language python\n#include \"python/ast.rerail\"\n.decl inner(e: node)\ninner(m) :- unparen(_, m).\n.output inner\n";
    let (alone, together) = alone_and_together("library-span", ("a.py", "x = \"ab cd\"\n"), words, other, "word");
    assert_eq!(alone, [["ab"], ["cd"]]);
    assert_eq!(together, alone);
}

/// Each test's store is its own: a copy of the template (the library's phase 0 in it), with an identity of its own,
/// and closed and removed when the test ends (`test_store`). Two tests at once have two stores.
#[test]
fn a_tests_store_is_a_copy_of_the_template_and_goes_with_its_test() {
    if std::env::var_os("RERAIL_TAPE_CACHE").is_some() {
        return;
    }
    let template = super::test_store::template().to_path_buf();
    let count = |path: &std::path::Path, sql: &str| -> i64 { rusqlite::Connection::open(path).unwrap().query_row(sql, [], |r| r.get(0)).unwrap() };
    let tapes = count(&template, "SELECT count(*) FROM tape_objects");
    assert!(tapes > 0, "the template holds the library's tapes");
    assert_eq!(count(&template, "SELECT count(*) FROM store_identity"), 0, "the template is no store until copied");
    let probe = |n: usize| {
        let template = template.clone();
        // A thread named as the harness names a test's.
        std::thread::Builder::new().name(format!("corpus::tests::probe{n}")).spawn(move || {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("a.py"), "def f():\n    return 1\n").unwrap();
            let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version("#include \"core.rerail\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap()).unwrap();
            let out = dir.path().join("out");
            super::run(&text, dir.path(), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap(), [["f"]]);
            let path = super::tape_cache_path().unwrap();
            assert_ne!(path, template);
            assert!(path.to_string_lossy().contains("rerail-test-stores"), "{}", path.display());
            let count = |sql: &str| -> i64 { rusqlite::Connection::open(&path).unwrap().query_row(sql, [], |r| r.get(0)).unwrap() };
            assert!(count("SELECT count(*) FROM tape_objects") > tapes, "the library's tapes and the test's own");
            assert_eq!(crate::content_store::holding(&path), (0, 0), "the run left store handles or connections open (handles, connections)");
            (path.parent().unwrap().to_path_buf(), count("SELECT id FROM store_identity"))
        }).unwrap()
    };
    let (a, b) = (probe(0), probe(1));
    let ((a, ia), (b, ib)) = (a.join().unwrap(), b.join().unwrap());
    assert_ne!(a, b, "two tests, two stores");
    assert_ne!(ia, ib, "each copy took an identity of its own");
    for dir in [a, b] {
        assert!(!dir.exists(), "{} is removed with its test", dir.display());
        #[cfg(target_vendor = "apple")]
        assert_eq!(super::test_store::held_under(&dir), Vec::<String>::new(), "nothing of {} is held open after its test (handles, connections: {:?})", dir.display(), crate::content_store::holding(&dir.join("store.sqlite")));
    }
}

/// What a test's store costs to make and remove (`test_store::copy`), the template already made: wall, CPU and the
/// process's instructions per copy, over 200.
#[test]
#[ignore = "a benchmark: the cost of a test's store"]
fn benchmark_test_store_copy() {
    let _ = super::test_store::template();
    drop(super::test_store::copy());
    let n = 200;
    let (i0, _) = rerail_compiler::native_cpu_codegen::retired();
    let cpu0 = super::usage();
    let t = std::time::Instant::now();
    for _ in 0..n {
        let store = super::test_store::copy();
        assert!(store.path().exists());
    }
    let wall = t.elapsed().as_secs_f64();
    let cpu1 = super::usage();
    let (i1, _) = rerail_compiler::native_cpu_codegen::retired();
    eprintln!(
        "test store copy: {:.0} µs wall, {:.0} µs user+sys, {:.2} M instructions, each (template {} bytes)",
        wall * 1e6 / n as f64,
        ((cpu1.0 - cpu0.0) + (cpu1.1 - cpu0.1)) * 1e6 / n as f64,
        (i1 - i0) as f64 / 1e6 / n as f64,
        std::fs::metadata(super::test_store::template()).unwrap().len()
    );
}

/// `benchmark_test_store_copy`'s steps apart: the clone, the identity's write (as `copy` makes it, and without a
/// journal or a sync), a first open as a store, and the removal.
/// APFS's `clonefile`: macOS only (elsewhere a test's store is a copy, `test_store::clone_file`).
#[cfg(target_vendor = "apple")]
#[test]
#[ignore = "a benchmark: the steps of a test's store"]
#[cfg(target_vendor = "apple")] // `clonefile` is APFS's
fn benchmark_test_store_copy_steps() {
    use std::os::unix::ffi::OsStrExt as _;
    let template = super::test_store::template().to_path_buf();
    let base = tempfile::tempdir().unwrap();
    let n = 100;
    let step = |label: &str, f: &mut dyn FnMut(usize)| {
        let (i0, _) = rerail_compiler::native_cpu_codegen::retired();
        let t = std::time::Instant::now();
        for k in 0..n {
            f(k);
        }
        let (i1, _) = rerail_compiler::native_cpu_codegen::retired();
        eprintln!("{label:<44} {:>7.0} µs wall {:>7.2} M instructions", t.elapsed().as_secs_f64() * 1e6 / n as f64, (i1 - i0) as f64 / 1e6 / n as f64);
    };
    let path = |round: &str, k: usize| base.path().join(format!("{round}-{k}.sqlite"));
    let clone = |to: &std::path::Path| {
        let a = std::ffi::CString::new(template.as_os_str().as_bytes()).unwrap();
        let b = std::ffi::CString::new(to.as_os_str().as_bytes()).unwrap();
        // SAFETY: NUL-terminated paths that outlive the call.
        assert_eq!(unsafe { libc::clonefile(a.as_ptr(), b.as_ptr(), 0) }, 0);
    };
    step("clonefile", &mut |k| clone(&path("a", k)));
    step("identity: UPDATE (journal, full sync)", &mut |k| {
        rusqlite::Connection::open(path("a", k)).unwrap().execute("UPDATE store_identity SET id = random() WHERE singleton = 1", []).unwrap();
    });
    step("clonefile again", &mut |k| clone(&path("b", k)));
    step("identity: UPDATE, journal off, no sync", &mut |k| {
        let db = rusqlite::Connection::open(path("b", k)).unwrap();
        db.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; UPDATE store_identity SET id = random() WHERE singleton = 1;").unwrap();
    });
    step("first open as a store (ContentStore::open)", &mut |k| drop(crate::content_store::ContentStore::open(&path("b", k)).unwrap()));
    step("remove the file and its sidecars", &mut |k| {
        for p in std::fs::read_dir(base.path()).unwrap().flatten().map(|e| e.path()) {
            if p.file_name().unwrap().to_string_lossy().starts_with(&format!("b-{k}.")) {
                let _ = std::fs::remove_file(p);
            }
        }
    });
}

/// A module is named as Python imports it, from its package root, not from the path below the corpus root: in a
/// `src` layout `src/shop/base.py` is `shop.base`, so `from shop.base import Base` reaches the class of the run
/// (`subclass`). A file in no package keeps its path's name. And a relative import of any number of dots binds its
/// names to their qualified names (`sym_of`), where only one dot did before: `from ..x import two` in `a.b.c.m` is
/// `a.b.x.two`. An import that climbs above the top package binds an unknown name, as before.
#[test]
fn modules_are_named_from_their_package_roots_and_relative_imports_of_any_depth_resolve() {
    let base = crate::code_rule_dsl::fixtures::temp().with_extension("package-roots");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("tree");
    for (path, text) in [
        ("src/shop/__init__.py", ""),
        ("src/shop/base.py", "class Base: pass\n"),
        ("src/shop/child.py", "from shop.base import Base\nclass Child(Base): pass\n"),
        ("src/manage.py", "x = 1\n"),
        ("a/__init__.py", ""),
        ("a/b/__init__.py", ""),
        ("a/b/c/__init__.py", ""),
        ("a/x.py", "def three(): pass\n"),
        ("a/b/x.py", "def two(): pass\n"),
        ("a/b/c/x.py", "def one(): pass\n"),
        ("a/b/c/m.py", "from .x import one\nfrom ..x import two\nfrom ...x import three\nfrom ....x import four\none(); two(); three(); four()\n"),
    ] {
        let p = src.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let rule = base.join("names.rerail");
    std::fs::write(
        &rule,
        ".language python\n.scope include \"**/*.py\"\n#include \"python/names.rerail\"\n\
         .decl mods(p: symbol, m: symbol)\nmods(p, m) :- doc(_, p, m).\n\
         .decl subs(q: symbol)\nsubs(q) :- subclass(q, \"shop.base.Base\").\n\
         .decl syms(q: symbol)\nsyms(q) :- kind(k, \"call\"), field(k, \"function\", c), sym_of(c, q).\n\
         .decl open_names(x: symbol)\nopen_names(x) :- rotgt(_, x).\n\
         .output mods, subs, syms, open_names\n",
    )
    .unwrap();
    let program = crate::code_rule_dsl::logic::settings::program_text(&rule).unwrap();
    let out = base.join("out");
    super::run(&program, &src, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default();
    let mods = read("mods");
    for row in [["src/shop/__init__.py", "shop"], ["src/shop/base.py", "shop.base"], ["src/shop/child.py", "shop.child"], ["src/manage.py", "src.manage"], ["a/b/c/m.py", "a.b.c.m"]] {
        assert!(mods.iter().any(|l| *l == row), "{row:?} in:\n{mods:?}");
    }
    assert_eq!(read("subs"), [["shop.child.Child"]]);
    assert_eq!(read("syms"), [["a.b.c.x.one"], ["a.b.x.two"], ["a.x.three"]]);
    assert_eq!(read("open_names"), [["four"]]);
    let _ = std::fs::remove_dir_all(&base);
}

/// `unpackaged(d)` holds for a Python file no regular package holds: one at the tree's root, or in a folder with no
/// `__init__.py`. Ruff names such a file by its stem and resolves no relative import in it, and
/// `python/references.rerail` does the same, from this fact alone, so its file-local part reads no other document. A
/// folder that gains an `__init__.py` makes its files packaged in the next run (the memo keys on the folders).
#[test]
fn a_python_file_no_package_holds_is_unpackaged() {
    let base = crate::code_rule_dsl::fixtures::temp().with_extension("unpackaged");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("tree");
    for (path, text) in [
        ("top.py", "x = 1\n"),
        ("loose/x.py", "x = 1\n"),
        ("pkg/__init__.py", ""),
        ("pkg/m.py", "x = 1\n"),
        ("pkg/sub/n.py", "x = 1\n"),
    ] {
        let p = src.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let rule = base.join("unpackaged.rerail");
    std::fs::write(
        &rule,
        ".language python\n.scope include \"**/*.py\"\n\
         .decl loose(p: symbol)\nloose(p) :- unpackaged(d), doc(d, p, _).\n.output loose\n",
    )
    .unwrap();
    let program = crate::code_rule_dsl::logic::settings::program_text(&rule).unwrap();
    let out = base.join("out");
    let run = || {
        let _ = std::fs::remove_dir_all(&out);
        super::run(&program, &src, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        crate::code_rule_dsl::logic::plan::output::shown(&out, "loose").unwrap_or_default()
    };
    assert_eq!(run(), [["loose/x.py"], ["pkg/sub/n.py"], ["top.py"]]);
    std::fs::write(src.join("loose/__init__.py"), "").unwrap();
    assert_eq!(run(), [["pkg/sub/n.py"], ["top.py"]]);
    let _ = std::fs::remove_dir_all(&base);
}

/// A request's context is every literal beside it (`rr::lower`'s `request_context`): `type_ctx(r) :- type_ctx(s),
/// str_node(s), parsed(s, "annotation", r)` asks to parse the strings in a type context, not every string of the
/// file. Its context was the inputs alone (`str_node`), and a module of 111k string pairs (netbox's `un_locode.py`)
/// asked for every one: 297k parses, past 25 minutes and 33 GB.
#[test]
#[ignore = "Q322: python/typing's file-local `pyi_sym` and the guests answered in the fixpoint (0431 §8) make one component that negates `typing_sym`"]
fn a_parse_request_asks_only_for_the_strings_its_rule_reads() {
    let base = crate::code_rule_dsl::fixtures::temp().with_extension("request-context");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("tree");
    std::fs::create_dir_all(&src).unwrap();
    let data: String = (0..50).map(|i| format!("    ('AD-{i:03}', 'Place {i}'),\n")).collect();
    std::fs::write(src.join("data.py"), format!("CODES = (\n{data})\n")).unwrap();
    std::fs::write(src.join("types.py"), "from typing import List\nx: \"List[int]\" = []\ny = \"not a type\"\n").unwrap();
    let rule = base.join("typing.rerail");
    std::fs::write(&rule, ".language python\n.scope include \"**/*.py\"\n#include \"python/typing.rerail\"\n.decl typed(n: node)\ntyped(n) :- type_ctx(n).\n.output typed\n").unwrap();
    let program = crate::code_rule_dsl::logic::settings::program_text(&rule).unwrap();
    let out = base.join("out");
    super::run(&program, &src, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let asked = crate::code_rule_dsl::logic::plan::output::shown(&out, "rr_embed_req").unwrap_or_default();
    assert_eq!(asked.len(), 1, "only \"List[int]\" is asked for:\n{asked:?}");
    let _ = std::fs::remove_dir_all(&base);
}

/// A file whose scope table cannot be built (nesting past `MAX_BUILD_DEPTH`) gives no `string_annotation` rows and
/// the run goes on: one such file ended a 30k-file run (posthog). The other files' annotations are read as before.
#[test]
fn a_file_too_deep_for_the_scope_table_leaves_the_run_going() {
    let base = crate::code_rule_dsl::fixtures::temp().with_extension("too-deep");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("tree");
    std::fs::create_dir_all(&src).unwrap();
    let depth = crate::code_rule_dsl::scope::MAX_BUILD_DEPTH as usize + 100;
    std::fs::write(src.join("deep.py"), format!("x = {}1{}\n", "[".repeat(depth), "]".repeat(depth))).unwrap();
    std::fs::write(src.join("typed.py"), "def f(a: \"int\") -> None: pass\n").unwrap();
    let rule = base.join("annotated.rerail");
    std::fs::write(&rule, ".language python\n.scope include \"**/*.py\"\n.decl annotated(p: symbol)\nannotated(p) :- string_annotation(s), docof(s, d), doc(d, p, _).\n.output annotated\n").unwrap();
    let program = crate::code_rule_dsl::logic::settings::program_text(&rule).unwrap();
    let out = base.join("out");
    let stack = std::thread::Builder::new().stack_size(64 << 20);
    let run = stack.spawn(move || super::run(&program, &src, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).map(|_| out)).unwrap();
    let out = run.join().unwrap().unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "annotated").unwrap_or_default(), [["typed.py"]]);
    let _ = std::fs::remove_dir_all(&base);
}

/// Decision 0284: the directory a run writes its outputs into is rerail's own: the observer leaves it out of a
/// listing, beside the corpus it read.
#[test]
fn a_runs_output_directory_is_rerails_own() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.py"), "def f():\n    return 1\n").unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version("#include \"core.rerail\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap()).unwrap();
    let out = dir.path().join("out");
    super::run(&text, dir.path(), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    // One output image, no csv (decision 0239): the run wrote its rows there.
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap_or_default(), [["f"]]);
    let names: Vec<String> = super::observe(|o| o.listing(dir.path()).unwrap().iter().map(|(n, _)| n.clone()).collect());
    assert_eq!(names, ["a.py"], "the outputs are not in the listing");
}

/// `start` (a process a test starts, `switches::started`) while another thread holds `switch` through `switches::set`,
/// as a test beside it does (`cross_file_rows_are_memoised_per_import_component`'s `RERAIL_INLINE_ONLY`, the session
/// tests' `RERAIL_CROSS_K4`): the process waits for it, and takes none of it.
///
/// A test holding the switches over its runs (`switches::reading`) lets them go for this (`held`), and takes them again
/// after: the other thread's `set` waits for every reader, the test's too, and the test waits for it to set.
fn beside_a_switch<T>(switch: (&'static str, &'static str), held: &mut Option<super::switches::Reading>, start: impl FnOnce() -> T) -> T {
    let reading = held.take().is_some();
    let (held_by_other, holding) = std::sync::mpsc::channel();
    let other = std::thread::spawn(move || {
        let _switched = super::switches::set(&[switch]);
        held_by_other.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
    });
    holding.recv().unwrap();
    let out = start();
    other.join().unwrap();
    if reading {
        *held = Some(super::switches::reading());
    }
    out
}

/// The run after a cold one, over what the cold run read unchanged, writes nothing (`RERAIL_WARM`): it names the same
/// sources and plans, so it records no manifest of its own, and it makes no choice the cold run did not keep. It was
/// news three ways: the cold run named a source no later run reads (a chunk's tree counts, still written though frames
/// carry their own statistics since); a nested run reused whole (`reuse::reused`) named none of its plans; and the
/// cross part whose every stratum was kept, a program of no rules, kept planning choices of its own. Each run is a
/// process of its own, as the CLI's are: this test binary again, running `one_corpus_run_for_a_parent`. The warm one is
/// started while another test holds `RERAIL_CROSS_K4=off` (`beside_a_switch`): started holding nothing, it took it and
/// wrote ("a write gathered for the store in a warm run").
#[test]
fn the_run_after_a_cold_one_writes_nothing() {
    // Its runs compare with each other: the switches held over all of them (`switches::reading`), or a test setting
    // its own on another thread (`RERAIL_INLINE_ONLY`, `RERAIL_CROSS_MEMO`) gave one run a mode another had not. Its
    // child processes take them as they start.
    let mut switches = Some(super::switches::reading());
    let temp = tempfile::tempdir().unwrap();
    // `RERAIL_TEST_KEEP=<dir>`: the runs' store and outputs left there, to look at.
    let kept = std::env::var_os("RERAIL_TEST_KEEP").map(std::path::PathBuf::from);
    cold_then_warm(kept.as_deref().unwrap_or(temp.path()), None, PICKLE_PACKAGE, &mut switches);
}

/// The same with tape values (decision 0431): a policy that parses string annotations. The warm run reads each
/// file's rows from its memo entry and parses no string, so the entry names the values it was made of (`memo::VALUES`)
/// and the warm run names their sources as the cold one did, which parsed them.
#[test]
fn the_run_after_a_cold_one_with_tape_values_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let c408 = rerail_store::source_tree::native().join("../docs/rule-syntax/packs/python/ruff-c408-unnecessary-collection-call.rerail");
    let files = [("pkg/__init__.py", ""), ("pkg/a.py", "from typing import List\n\na: \"dict()\" = {}\nb: List[\"list()\"] = []\n"), ("main.py", "c: \"tuple()\" = ()\n")];
    cold_then_warm(temp.path(), Some(&c408), &files, &mut None);
}

/// `cold_then_warm`'s sources by default: a small Python package and a file importing it.
const PICKLE_PACKAGE: &[(&str, &str)] = &[("pkg/__init__.py", ""), ("pkg/loader.py", "import pickle as p\n\ndef load(b):\n    return p.loads(b)\n"), ("main.py", "from pkg.loader import load\nload(b'x')\n")];

/// The same, over sources reached through a symlink, as every run under macOS's `$TMPDIR` is (`/var` is
/// `/private/var`): the run reads its files by their canonical paths, so the corpus walk reads the real part of the
/// root before the waves as well. Reading only the part named, the warm run asked for the real one inside the waves
/// and faulted "the observer's store not asked inside the waves". PR #508's 0206654a5 (t47).
#[cfg(unix)]
#[test]
fn the_run_after_a_cold_one_through_a_symlink_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("real")).unwrap();
    std::os::unix::fs::symlink(temp.path().join("real"), temp.path().join("link")).unwrap();
    cold_then_warm(&temp.path().join("link"), None, PICKLE_PACKAGE, &mut None);
}

/// A cold run of `program` (else the pickle pack's) over `files` under `base`, then a warm one under
/// `RERAIL_WARM=list`, which writes nothing.
fn cold_then_warm(base: &std::path::Path, program: Option<&std::path::Path>, files: &[(&str, &str)], switches: &mut Option<super::switches::Reading>) {
    let src = base.join("src");
    for (path, text) in files {
        let at = src.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }
    // Past the observer's racy window: a file changed within it of its answer is hashed again by the next run, and
    // its answer recorded again (`RACY_NS`). The claim is of sources older than that; a cold run fast enough to read
    // them inside it made the warm run's writes (the answers, the manifest, the statistics).
    std::thread::sleep(std::time::Duration::from_nanos(crate::content_store::RACY_NS as u64 + 100_000_000));
    let store = base.join("store.sqlite");
    let manifests = || -> Vec<(Vec<u8>, Vec<u8>)> {
        let db = rusqlite::Connection::open(&store).unwrap();
        let mut rows: Vec<(Vec<u8>, Vec<u8>)> = db.prepare("SELECT digest, sources FROM corpus_manifests").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().flatten().collect();
        rows.sort();
        rows
    };
    // `warm`: the run under `RERAIL_WARM=list`, which ends its process with status 3 if it wrote anything.
    let run = |warm: bool| {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "code_rule_dsl::logic::plan::corpus::tests::one_corpus_run_for_a_parent", "--ignored", "--test-threads", "1", "--nocapture"])
            .env("RERAIL_TAPE_CACHE", &store)
            .env("RERAIL_CHILD_RUN", &src)
            .env("RERAIL_CHILD_OUT", base.join("out"));
        if let Some(program) = program {
            child.env("RERAIL_CHILD_PROGRAM", program);
        }
        if warm {
            child.env("RERAIL_WARM", "list");
        }
        let output = super::switches::started(|| child.output()).unwrap();
        // Each fault and the two lines after it, which say what was written and in which span.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let faults: Vec<String> = lines.iter().enumerate().filter(|(_, l)| l.starts_with("rerail: RERAIL_WARM")).map(|(i, l)| format!("{l}{}", lines[i + 1..(i + 3).min(lines.len())].iter().map(|n| format!(" ({})", n.trim())).collect::<String>())).collect();
        // No fault named: the process failed some other way, and its stderr's tail says how.
        let tail = if faults.is_empty() { lines[lines.len().saturating_sub(30)..].join("\n") } else { String::new() };
        assert!(output.status.success(), "the run's process: {}; {}{tail}", output.status, faults.join("; "));
    };
    run(false);
    let cold = manifests();
    assert!(!cold.is_empty(), "the cold run recorded its manifests");
    beside_a_switch(("RERAIL_CROSS_K4", "off"), switches, || run(true));
    let warm = manifests();
    let new: Vec<&(Vec<u8>, Vec<u8>)> = warm.iter().filter(|m| !cold.contains(m)).collect();
    assert!(new.is_empty(), "the warm run named what the cold run named: no manifest of its own ({} new)", new.len());
}

/// A rule's edit over files the stat index knows writes their memo entries with no SQL in the waves (decision 0104,
/// `RERAIL_SQL_STRICT`). The edit keeps the memo's space (the same relations, `memo::space_digest`), so each file's entry is found
/// by its stat before the first wave, unhashed, and misses the edited relation: the file is built, its content a
/// stored source with no digest. Its entry's write, on the chunk's thread, needs that digest: the run hashes the text
/// it holds (`build_doc`); read from the store, it was `source_digests` inside the waves, and the strict run aborted
/// (e70f79dab). The runs are processes of their own, as `the_run_after_a_cold_one_writes_nothing`'s are.
#[test]
fn a_rule_edit_over_known_files_writes_their_entries_with_no_sql_in_the_waves() {
    // Its runs compare with each other: the switches held over all of them (`switches::reading`), or a test setting
    // its own on another thread (`RERAIL_INLINE_ONLY`, `RERAIL_CROSS_MEMO`) gave one run a mode another had not. Its
    // child processes take them as they start.
    let _switches = super::switches::reading();
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path();
    let src = base.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..3 {
        std::fs::write(src.join(format!("m{i}.py")), format!("x{i} = 1
y = x{i} + 2
")).unwrap();
    }
    let rule = |name: &str, extra: &str| {
        let path = base.join(name);
        std::fs::write(&path, format!("#include \"core.rerail\"\n.decl idents(s: number, e: number)\nidents(s, e) :- node(_, n, \"identifier\", s, e), kind(n, \"identifier\"){extra}.\n.output idents\n")).unwrap();
        path
    };
    let (before, edited) = (rule("before.rerail", ""), rule("edited.rerail", ", s >= 0"));
    // Past the observer's racy window, so the second run knows each file by its stat (`RACY_NS`).
    std::thread::sleep(std::time::Duration::from_nanos(crate::content_store::RACY_NS as u64 + 100_000_000));
    let store = base.join("store.sqlite");
    let run = |program: &std::path::Path, strict: bool| {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "code_rule_dsl::logic::plan::corpus::tests::one_corpus_run_for_a_parent", "--ignored", "--test-threads", "1", "--nocapture"])
            .env("RERAIL_TAPE_CACHE", &store)
            .env("RERAIL_CHILD_RUN", &src)
            .env("RERAIL_CHILD_OUT", base.join("out"))
            .env("RERAIL_CHILD_PROGRAM", program)
            .env("RERAIL_COSTS", base.join("costs.sqlite"))
            .env("RERAIL_IMAGE_STORE", base.join("images"));
        if strict {
            child.env("RERAIL_SQL_STRICT", "1");
        }
        let output = super::switches::started(|| child.output()).unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let fault: Vec<&str> = stderr.lines().skip_while(|l| !l.starts_with("rerail: RERAIL_SQL_STRICT")).take(200).collect();
        assert!(output.status.success(), "the run's process: {}; {}", output.status, fault.join(" "));
    };
    run(&before, false);
    run(&edited, true);
}

/// One corpus run in a process of its own, for `the_run_after_a_cold_one_writes_nothing`.
#[test]
#[ignore = "a parent test runs it in a process of its own"]
fn one_corpus_run_for_a_parent() {
    let (Some(src), Some(out)) = (std::env::var_os("RERAIL_CHILD_RUN"), std::env::var_os("RERAIL_CHILD_OUT")) else { return };
    // `RERAIL_CHILD_PROGRAM=<file.rerail>`: that rule file, else the pickle pack's.
    let rules = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
    let file = std::env::var_os("RERAIL_CHILD_PROGRAM").map_or_else(|| rules.join("bandit-b301-pickle.rerail"), std::path::PathBuf::from);
    let program = crate::code_rule_dsl::logic::settings::program_text(&file).unwrap();
    // `RERAIL_CHILD_RUNS=<n>`: n runs in this one process, as a daemon's or `repeat`'s; under `RERAIL_CHILD_SESSION`,
    // runs of one session.
    let runs: usize = std::env::var("RERAIL_CHILD_RUNS").ok().and_then(|n| n.parse().ok()).unwrap_or(1);
    let _session = std::env::var_os("RERAIL_CHILD_SESSION").map(|dir| super::session::start(std::path::Path::new(&dir)).unwrap());
    for _ in 0..runs {
        super::run_all(&[program.as_str()], std::path::Path::new(&src), std::path::Path::new(&out), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    }
}

/// A second run in one process names what the first named: no manifest of its own. Its process-memory answers (the
/// planning choices kept, the groups of automata kept) skipped the store reads that named their sources, so its
/// manifest left them out, and the next process's run, naming them again, recorded one more. Each process is started
/// holding the switches (`switches::started`): the process of two is started while another thread holds a switch, as
/// `cross_file_rows_are_memoised_per_import_component` holds `RERAIL_INLINE_ONLY`, and takes none of it (it took it,
/// ran another plan and recorded two manifests of its own).
#[test]
fn a_second_run_in_one_process_records_the_first_runs_manifest() {
    // Its runs compare with each other: the switches held over all of them (`switches::reading`), or a test setting
    // its own on another thread (`RERAIL_INLINE_ONLY`, `RERAIL_CROSS_MEMO`) gave one run a mode another had not. Its
    // child processes take them as they start.
    let mut switches = Some(super::switches::reading());
    let temp = tempfile::tempdir().unwrap();
    let kept = std::env::var_os("RERAIL_TEST_KEEP").map(std::path::PathBuf::from);
    let base = kept.as_deref().unwrap_or(temp.path());
    let src = base.join("src");
    std::fs::create_dir_all(src.join("pkg")).unwrap();
    std::fs::write(src.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(src.join("pkg/loader.py"), "import pickle as p\n\ndef load(b):\n    return p.loads(b)\n").unwrap();
    std::fs::write(src.join("main.py"), "from pkg.loader import load\nload(b'x')\n").unwrap();
    let store = base.join("store.sqlite");
    let manifests = || -> Vec<(Vec<u8>, Vec<u8>)> {
        let db = rusqlite::Connection::open(&store).unwrap();
        let mut rows: Vec<(Vec<u8>, Vec<u8>)> = db.prepare("SELECT digest, sources FROM corpus_manifests").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().flatten().collect();
        rows.sort();
        rows
    };
    let run = |runs: usize| {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "code_rule_dsl::logic::plan::corpus::tests::one_corpus_run_for_a_parent", "--ignored", "--test-threads", "1", "--nocapture"])
            .env("RERAIL_TAPE_CACHE", &store)
            .env("RERAIL_CHILD_RUN", &src)
            .env("RERAIL_CHILD_OUT", base.join("out"))
            .env("RERAIL_CHILD_RUNS", runs.to_string());
        let output = super::switches::started(|| child.output()).unwrap();
        assert!(output.status.success(), "the runs' process: {}", output.status);
    };
    // A process first keeps phase 0's graphs as images (decision 0246): every later run reads them, and so names none
    // of the bootstrap run's plans (`phase0::Images`). Without it, the baseline's run derived the graphs and named
    // those plans, which the runs after it, reading the images, do not.
    run(1);
    // A process of one run: what a run names.
    run(1);
    let one = manifests();
    // A process of two: the second names the same. Another test's switch is held as it is started.
    beside_a_switch(("RERAIL_INLINE_ONLY", "near"), &mut switches, || run(2));
    let two = manifests();
    let new: Vec<&(Vec<u8>, Vec<u8>)> = two.iter().filter(|m| !one.contains(m)).collect();
    assert!(new.is_empty(), "each run named what a process's first run names: no manifest of its own ({} new)", new.len());
}

/// Decision 0275: one program handed to its run (`run_read`) is hidden once and kept under its texts' digest, as a
/// pack is: the hidden program read back is the one hiding makes, and a second combine is the store's.
#[test]
fn a_handed_programs_hidden_form_is_kept_under_its_texts() {
    let src = "#include \"core.rerail\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n";
    let read = crate::code_rule_dsl::logic::rr::parse_version(src).unwrap();
    let (made, hidden, kept) = super::combine_handed(&read).unwrap();
    let id = |v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>| (v.size(), v.digest());
    assert_eq!(id(&made), id(&hidden), "a miss hides the program handed");
    assert_eq!(kept.as_ref().map(id), Some(id(&hidden)), "kept under the texts' digest, read back as hidden");
    let (again, _, _) = super::combine_handed(&read).unwrap();
    assert_eq!(id(&again), id(&hidden), "the store's, the second time");
}

/// Two programs combined with the library they share stratify (decision 0174, E32): PYI062's `literal_down` negates
/// `typing_subscript`, which reads `value` through `exports`. The node attributes are answered on request over
/// the combined program's readers; each part lowered alone made `rr_value_req` read `literal_leaf`, recursive
/// with `value` through that negation, and the pair was refused as not stratifiable.
#[test]
#[ignore = "Q322: python/typing's file-local `pyi_sym` and the guests answered in the fixpoint (0431 §8) make one component that negates `typing_sym`"]
fn a_pair_of_programs_asking_for_an_attribute_through_the_library_stratifies() {
    use crate::code_rule_dsl::typing::file_local::Binding;
    let dir = std::env::temp_dir().join(format!("rerail-pyi062-pyi059-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.py"), "from typing import Generic, Literal, TypeVar\nT = TypeVar('T')\nx: Literal[1, 1]\nclass C(Generic[T], int): pass\n").unwrap();
    let rules = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
    let programs: Vec<String> = ["ruff-pyi062-duplicate-literal-member.rerail", "ruff-pyi059-generic-not-last-base-class.rerail"]
        .iter()
        .map(|r| crate::code_rule_dsl::logic::settings::program_text(&rules.join(r)).unwrap())
        .collect();
    let out = dir.join("out");
    let ran = super::run_all(&programs.iter().map(String::as_str).collect::<Vec<_>>(), &dir.join("src"), &out, "program", Binding::Cpu, 1 << 18);
    let findings = format!("{:?}", crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap_or_default());
    let _ = std::fs::remove_dir_all(&dir);
    ran.unwrap();
    assert!(findings.contains("PYI062") && findings.contains("PYI059"), "both rules' findings: {findings}");
}

/// Programs combined that make guests are each hidden alone; each part's on-request rewrite (decision 0174) is undone
/// before that, not after: hiding copied `token`'s answer rules into its grammar-scoped copy (`rr_in_<g>_token`),
/// the undo then dropped the inputs they read (`rr_token_anon`, `rr_token_named`), and the combined run failed on an
/// undeclared relation (Q67: these three of the data-layer pack, on any file).
#[test]
fn programs_hidden_alone_keep_their_attribute_copies_declared() {
    use crate::code_rule_dsl::typing::file_local::Binding;
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("token-copies");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.py"), "def f(x: int) -> int:\n    if x is not None:\n        return 1\n    return 0\n").unwrap();
    // Pack test data (`packs/data-layer/upstream/lint/`, not in git since 5e0b69a104): pulled, or the test says so.
    let rules = crate::code_rule_dsl::test_support::pack_test_data("packs/data-layer/upstream/lint/authored/packs/data-layer/candidates/datalog/authored");
    let programs: Vec<String> = ["data-layer-schema-bootstrap-contracts.rerail", "data-layer-unresolved-forward-refs.rerail", "data-layer-versioned-merged-identity.rerail"]
        .iter()
        .map(|r| crate::code_rule_dsl::logic::settings::program_text(&rules.join(r)).unwrap())
        .collect();
    let ran = super::run_all(&programs.iter().map(String::as_str).collect::<Vec<_>>(), &dir.join("src"), &dir.join("out"), "program", Binding::Cpu, 1 << 18);
    let _ = std::fs::remove_dir_all(&dir);
    ran.unwrap();
}

/// A text test over `node_text` that a cross-file relation reads (`passes` reads `uses`, which joins two files) gives
/// a run's outputs with an empty store after each edit, and no cross-file stratum made natively is derived again.
/// The request for the text is narrowed to the node's own document (0286), so the text is answered, and tested, in
/// the chunk that holds it; a text test left in the cross-file part is maintained as a filter of its input's strings
/// (`maintain::rewrite_over`'s `L::Test`).
#[test]
fn a_cross_file_text_test_is_maintained_as_a_filter() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("text-filter");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\ndef k():\n    return 1\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "def h():\n    pass\n");
    let rule = dir.join("named.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl passes(f: node)
passes(f) :- uses(_, f), node_text(f, t), contains("pass", t).
.output passes
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    use crate::code_rule_dsl::typing::file_local::Binding;
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &dir, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        (report, crate::code_rule_dsl::logic::plan::output::shown(&out, "passes").unwrap())
    };
    run("kept.sqlite");
    let edits = [
        ("c.py", "from a import k\ndef h():\n    pass\n"),
        ("b.py", "from a import f\nfrom a import k\ndef g():\n    pass\n"),
        ("a.py", "def f():\n    return 2\ndef k():\n    pass\n"),
        ("b.py", "def g():\n    pass\n"),
    ];
    let mut tested = false;
    for (i, (file, body)) in edits.into_iter().enumerate() {
        write(file, body);
        let (report, rows) = run("kept.sqlite");
        let (_, fresh) = run(&format!("fresh{i}.sqlite"));
        assert_eq!(rows, fresh, "after edit {i} ({file})");
        // The program's one native is the text test.
        assert!(!report.cross_again.contains_key("a relation made natively"), "edit {i}: the text test is maintained, not derived again: {:?}", report.cross_again);
        tested |= report.text.0 > 0;
    }
    assert!(tested, "the program has a text test");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A cross-file reader of a node's text (`passes` reads `uses`, which joins two files) finds it (O-cross-file-answers):
/// its request is narrowed to the node's own document (`file_local::requests_file_local`, the hub's work-queue
/// question 2, option a), `uses` unfolded to `def(f, _)`, so the text is answered in the chunk that holds it. Asked in
/// the cross-file part, the chunk's batch was gone and the text was empty: no row.
#[test]
fn a_cross_file_reader_finds_the_text_of_another_files_node() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("cross-text");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "def f():\n    pass\ndef k():\n    return 1\n").unwrap();
    std::fs::write(dir.join("b.py"), "from a import f\nfrom a import k\n").unwrap();
    let rule = dir.join("named.rerail");
    std::fs::write(&rule, r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl passes(f: node)
passes(f) :- uses(_, f), node_text(f, t), contains("pass", t).
.output passes
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    // The output is its image (decision 0239): read back as rows, not a .csv.
    let rows = crate::code_rule_dsl::logic::plan::output::shown(&out, "passes").unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(rows.len(), 1, "f, whose body is `pass`, and not k: {rows:?}");
}

/// Every switch the lowering and preparation sources read is classified: it changes the prepared program (in its key,
/// `corpus::PREPARING`) or does not (`NOT_PREPARING`). An unlisted one would let a program prepared under
/// one value be reused under another, or make a session's next run cold.
#[test]
fn every_switch_lowering_reads_is_classified() {
    /// The switches combining, lowering and preparation read that change no prepared program: checks, dumps, traces
    /// and profiles, the store's and the cost model's paths, the session's binding.
    const NOT_PREPARING: &[&str] = &[
        "RERAIL_BLESS_CENSUS",
        "RERAIL_IDEMPOTENCY_DUMP",
        "RERAIL_MAGIC_CHECK",
        "RERAIL_MERGE_WHY",
        "RERAIL_SPLIT_CHECK",
        "RERAIL_SPLIT_DUMP",
        "RERAIL_DEMAND_DUMP",
        "RERAIL_PREPARED_TRACE",
        // How the combine's link is made (decision 0373): spliced into the last one, linked whole, or both and compared;
        // the program is the same (`check` asserts it).
        "RERAIL_COMBINE_LINK",
        // How the combine's tail is run (decision 0393): kept, whole, or both and compared; the program is the same
        // (`check` asserts it).
        "RERAIL_COMBINE_TAIL",
        // A measurement aid: the program as the passes before lowering leave it, written to a directory.
        "RERAIL_FRONT_DUMP",
        // A measurement aid (decision 0173): each version a program takes, written to a directory; the program is the same.
        "RERAIL_VERSION_TRACE",
        // A profile's live heap by allocating stack (`heap-frames.tsv`): a diagnostic; the program is the same.
        "RERAIL_HEAP_FRAMES",
        "RERAIL_SPIKE_PROJECTION",
        // A diagnostic: a refused component's members, printed (`front::unstratified_trace`).
        "RERAIL_STRATA_COUNTS",
        "RERAIL_TAPE_CACHE",
        // Where rule images are read from, kept and exported (decision 0264): an image is the program's stored form,
        // keyed by its bytes' digest, so the program is the same wherever it is found, or with no image at all.
        "RERAIL_SHIPPED_IMAGES",
        "RERAIL_IMAGE_STORE",
        "RERAIL_EXPORT_IMAGES",
        "RERAIL_MODULE_IMAGES",
        "RERAIL_DUMP_PROGRAMS",
        "RERAIL_TIMELINE",
        "RERAIL_PROFILE",
        "RERAIL_SQL_STRICT",
        "RERAIL_WARM",
        "RERAIL_MEMO_PROGRAM_DUMP",
        "RERAIL_STATS_KEEP_TRACE",
        // A measurement aid: writes each program made from another to a trace directory (`version_trace`).
        "RERAIL_VERSION_TRACE",
        "RERAIL_SESSION",
        // The library's directory: its files' paths and digests are in the texts' digest (the module graph).
        "RERAIL_RR_LIB",
        // The cost model's file: its estimates steer plan choices, which keep the program's meaning (`plan::run`).
        "RERAIL_COSTS",
        // The join model (decision 0354): its prices choose among a line's candidate orders, as the cost model's do.
        "RERAIL_JOIN_MODEL",
        // Keyed through what it adds: the observer's facts (the source files), which the prepared key hashes.
        "RERAIL_SOURCES_PHASE",
        // Where rule images are read from and kept (decision 0246 §6): an image is keyed by its text, its scope and
        // the format, so whichever store gives it, it is the same program; off, the text is lowered again.
        "RERAIL_MODULE_IMAGES",
        "RERAIL_IMAGE_STORE",
        "RERAIL_SHIPPED_IMAGES",
        // A bundle's export: the images written beside the store as well.
        "RERAIL_EXPORT_IMAGES",
        // A measurement aid (decision 0173): each version written out, the program unchanged.
        "RERAIL_VERSION_TRACE",
    ];
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl");
    // Every switch named, however it is read (`var`, `var_os`, a constant, a helper's argument).
    let read = regex::Regex::new(r#""(RERAIL_[A-Z0-9_]+)""#).unwrap();
    // A file's own test module (`#[cfg(test)]`, `#[cfg(all(test, …))]`), which ends it.
    let test_module = regex::Regex::new(r"\n#\[cfg\([^\]]*\btest\b[^\]]*\)\]\n(?:pub(?:\([^)]*\))? )?mod tests\b").unwrap();
    let sources = |places: &[&str]| -> Vec<(String, String)> {
        let mut files = Vec::new();
        let mut pending: Vec<std::path::PathBuf> = places.iter().map(|p| root.join(p)).collect();
        let missing: Vec<&std::path::PathBuf> = pending.iter().filter(|p| !p.exists()).collect();
        assert!(missing.is_empty(), "places that are gone: {missing:?}");
        while let Some(p) = pending.pop() {
            if p.is_dir() {
                pending.extend(std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()));
            } else if p.extension().is_some_and(|e| e == "rs") && !p.file_name().unwrap().to_string_lossy().contains("tests") {
                // Up to its own test module, as a test file is left out: a test's switches (a child process's) key
                // no program.
                let mut text = std::fs::read_to_string(&p).unwrap();
                if let Some(m) = test_module.find(&text) {
                    text.truncate(m.start());
                }
                files.push((p.strip_prefix(&root).unwrap().display().to_string(), text));
            }
        }
        files
    };
    // A switch read through a helper (`syntax::definition_ids()` for `RERAIL_NODE_IDS`) is read where the helper is
    // called: every short function of the tree (under 20 lines) that names a switch, by a name no other function has.
    let accessors: std::collections::HashMap<String, Vec<String>> = {
        let every = sources(&["."]);
        let fn_start = regex::Regex::new(r"(?m)^[ \t]*(?:pub(?:\([^)]*\))? )?(?:const )?fn ([a-z_][a-z0-9_]*)").unwrap();
        let mut found: std::collections::HashMap<String, (usize, Vec<String>)> = std::collections::HashMap::new();
        for (_, text) in &every {
            let starts: Vec<(usize, String)> = fn_start.captures_iter(text).map(|c| (c.get(0).unwrap().start(), c[1].to_owned())).collect();
            for (i, (at, name)) in starts.iter().enumerate() {
                let end = starts.get(i + 1).map_or(text.len(), |n| n.0);
                let body = &text[*at..end];
                let switches: Vec<String> = read.captures_iter(body).map(|c| c[1].to_owned()).collect();
                let e = found.entry(name.clone()).or_insert((0, Vec::new()));
                e.0 += 1;
                if body.lines().count() < 20 {
                    e.1.extend(switches);
                }
            }
        }
        found.into_iter().filter(|(_, (n, sw))| *n == 1 && !sw.is_empty()).map(|(name, (_, sw))| (name, sw)).collect()
    };
    let call = regex::Regex::new(r"\b([a-z_][a-z0-9_]*)\(").unwrap();
    let reads = |text: &str| -> Vec<String> {
        let mut all: Vec<String> = read.captures_iter(text).map(|c| c[1].to_owned()).collect();
        for c in call.captures_iter(text) {
            let before = &text[..c.get(0).unwrap().start()];
            if !before.ends_with("fn ") {
                all.extend(accessors.get(&c[1]).into_iter().flatten().cloned());
            }
        }
        all
    };
    // `corpus.rs` itself: its preparation (`run_observed` up to the prepared program) and `combine_tail`.
    let corpus = std::fs::read_to_string(root.join("logic/plan/corpus.rs")).unwrap();
    let region = |from: &str, to: &str| -> String {
        let a = corpus.find(from).unwrap_or_else(|| panic!("corpus.rs has no {from:?}"));
        let b = a + corpus[a..].find(to).unwrap_or_else(|| panic!("corpus.rs has no {to:?} after {from:?}"));
        corpus[a..b].to_owned()
    };
    // Combining: `combine_tail` (its link, `combine_linked_as`, and its tail, `tail_version`), what it calls that
    // reads a switch (`scoping`, the tail's passes), and the `.rerail` lowering its `rr::parse` runs
    // (`logic/rr`, `front`). The combined program is kept under `COMBINING` alone.
    let mut combining = sources(&[
        "logic/rr", "logic/front.rs", "logic/front", "logic/plan/corpus/scoping.rs", "logic/plan/corpus/preparation/whole.rs",
        "logic/plan/corpus/preparation/on_request.rs", "logic/plan/corpus/preparation/hide.rs", "logic/plan/corpus/link.rs",
    ]);
    combining.push(("corpus.rs, combine_tail".to_owned(), region("\nfn combine_tail(", "\n}\n")));
    combining.push(("corpus.rs, the combine's tail".to_owned(), region("\nenum Tail {", "\n/// The programs `programs` linked into one")));
    // The link: `combine_linked_as`, its modes, `combine_linked_spliced` and the splice (`combine_spliced`).
    combining.push(("corpus.rs, combine_linked_as".to_owned(), region("\nfn combine_linked_as<", "Ok((all, Tail::Many { hide, policies, kept }))")));
    // Preparing: `run_observed` up to the prepared program, what it calls that reads a switch (`combining::FoldOwn`,
    // memo, session, profile, costs) and the passes. The prepared key holds `PREPARING` and the combined program's
    // key. Not `plan/keeps.rs` and `plan/choices.rs`: what a relation keeps is chosen in `plan::run` and kept under
    // `choices::key`, which holds the switches those read (`RERAIL_INLINE`, `RERAIL_INLINE_ONLY`,
    // `RERAIL_OBSERVED_STEPS`).
    let mut preparing = sources(&[
        // The tree passes that were `logic/simplify.rs` and `logic/magic.rs` are the preparation's now (0276 step 6).
        "logic/strings.rs", "logic/reach_form.rs", "logic/magic", "logic/plan/lower.rs",
        "logic/plan/corpus/import_closure.rs", "logic/plan/corpus/hiding.rs", "logic/plan/corpus/memo.rs", "logic/plan/corpus/prepared.rs",
        // Its passes, on the version (`preparation`): the front's, the lowering's, `FoldOwn`'s and the merges, and
        // the combine's fact spaces.
        "logic/plan/corpus/preparation.rs", "logic/plan/corpus/preparation",
        "logic/plan/corpus/session.rs", "logic/profile.rs", "logic/costs.rs", "typing/file_local.rs", "typing/file_local",
    ]);
    preparing.push(("corpus.rs, run_observed's preparation".to_owned(), region("\nfn run_observed(", "Arc::new(Prepared {")));
    let mut unlisted = std::collections::BTreeSet::new();
    let mut seen = std::collections::BTreeSet::new();
    for (files, allowed) in [(&combining, vec![super::COMBINING]), (&preparing, vec![super::COMBINING, super::PREPARING])] {
        for (name, text) in files.iter() {
            for switch in reads(text) {
                seen.insert(switch.clone());
                if !allowed.iter().any(|l| l.contains(&switch.as_str())) && !NOT_PREPARING.contains(&switch.as_str()) {
                    let list = if allowed.len() == 1 { "COMBINING" } else { "PREPARING" };
                    unlisted.insert(format!("{switch} in {name} (wants {list})"));
                }
            }
        }
    }
    assert!(unlisted.is_empty(), "switches read in combining or preparing, in neither its list nor NOT_PREPARING: {unlisted:?}");
    // And the other way: a listed switch keys what it changes, read in combining or preparing, or after them by what
    // hangs off the prepared key (the run's extraction: `with_long_text_parts`'s text nodes and anchors, the span
    // prefilter, both in `facts/vtab/syntax.rs`; the rest of `run_observed`). One read nowhere there keys nothing it
    // changes: a session's next run cold for nothing.
    for text in sources(&["facts/vtab/syntax.rs"]).into_iter().map(|(_, t)| t).chain([region("\nfn run_observed(", "\n}\n")]) {
        seen.extend(reads(&text));
    }
    let unread: Vec<&str> = super::COMBINING.iter().chain(super::PREPARING).copied().filter(|s| !seen.contains(*s)).collect();
    assert!(unread.is_empty(), "switches listed as combining or preparing that nothing there reads: {unread:?}");
}

/// Q22, a guard: a run after a rule's edit parses no file the last run read. The edit makes a new lane (the programs'
/// texts) and plan, and the tapes are found by the root's last runs (`root_sources`), read before the first wave
/// (decision 0104). It regressed unseen: the tapes were looked up by lane, which every edit changes. A tape value's
/// tape is found the same way (decision 0431 R6, `a_run_stores_its_tape_values_by_their_text`).
#[test]
fn an_edit_parses_no_file_the_last_run_read() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("file-tapes");
    let _ = std::fs::remove_dir_all(&dir);
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("a.py"), "from typing import List, Optional\n\nx: \"List[int]\" = []\n\ndef f(a: \"Optional[Foo]\") -> \"Baz\":\n    pass\n").unwrap();
    std::fs::write(ws.join("b.py"), "y: \"Optional[int]\" = None\n").unwrap();
    let program = |extra: &str| format!(r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl names(x: symbol)
names(x) :- string_annotation(n), parsed(n, "annotation", r), within(m, r), node(_, m, "identifier", _, _), text(m, x){extra}.
.output names
"#);
    let out = dir.join("out");
    let run = |text: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        super::run(text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap()
    };
    // The first run's files are parsed by its sources phase, which runs first, and found by the run itself.
    let first = run(&program(""));
    assert_eq!(first.file_tapes.0 + first.file_tapes.1, 2, "{:?}", first.file_tapes);
    // An edited rule: a new lane and plan; the files are as they were.
    let edited = run(&program(", x != \"q22\""));
    assert_eq!(edited.file_tapes.1, 0, "no file is parsed again (found, parsed): {:?}", edited.file_tapes);
    assert!(edited.file_tapes.0 > 0, "the files' tapes are the last run's: {:?}", edited.file_tapes);
    // And again: a run that found its tapes names them, so the next finds them too.
    let again = run(&program(", x != \"q22b\""));
    assert_eq!(again.file_tapes.1, 0, "{:?}", again.file_tapes);
    assert!(again.file_tapes.0 > 0, "{:?}", again.file_tapes);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The relations `head`'s rules read, in a combined program.
fn reads_of(v: &crate::code_rule_dsl::logic::program_image::Version, head: &str) -> Vec<String> {
    v.heads_and_reads().into_iter().filter(|(h, _)| h == head).flat_map(|(_, reads)| reads.into_iter().map(|(_, r)| r)).collect()
}

/// Decision 0431 R6's runs: a workspace, a store, and a run of `names` (the annotations' identifiers, plus `extra` in
/// the rule's body) over it, giving the run's report.
fn tape_value_runs(name: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, impl Fn(&str) -> super::Report) {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    let _ = std::fs::remove_dir_all(&dir);
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    for (path, text) in files {
        std::fs::write(ws.join(path), text).unwrap();
    }
    let (d, w) = (dir.clone(), ws.clone());
    let run = move |extra: &str| {
        let program = format!(r#"
.decl names(x: symbol)
names(x) :- string_annotation(n), parsed(n, "annotation", r), within(m, r), kind(m, "identifier"), text(m, x){extra}.
.output names
"#);
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(d.join("store.sqlite")));
        super::run(&program, &w, None, &d.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap()
    };
    (dir, run)
}

/// Decision 0431 R7's program: a cross-file rule (`refers`) joins an annotation's identifier (a node of its tape value,
/// `guest_of`) in one file with a class of that name in another, and `shown` reads that node's facts across files:
/// its text, and its place in its file through the string (`guest_place`).
const CROSS_FILE_VALUES: &str = r#"
.decl ann_node(m: node, x: symbol)
ann_node(m, x) :- string_annotation(n), parsed(n, "annotation", _), guest_of(m, n), kind(m, "identifier"), text(m, x).
.decl cls(c: node, x: symbol)
cls(c, x) :- kind(c, "class_definition"), field(c, "name", i), text(i, x).
.decl refers(m: node, c: node)
refers(m, c) :- ann_node(m, x), cls(c, x).
.decl shown(x: symbol, p: symbol, l: number)
shown(x, p, l) :- refers(m, c), text(m, x), guest_place(m, _, l, _), docof(c, d), doc(d, p, _).
.output shown
"#;

/// A run of `CROSS_FILE_VALUES` over `ws` in the store and output of `dir`: its report and `shown`'s rows.
fn cross_file_values_run(dir: &std::path::Path) -> (super::Report, Vec<Vec<String>>) {
    crate::install_store_functions();
    let rule = dir.join("program.rerail");
    std::fs::write(&rule, CROSS_FILE_VALUES).unwrap();
    let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let report = super::run(&program, &dir.join("ws"), None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let rows = super::super::output::Outputs::open(&dir.join("out")).unwrap().get("shown").map(|r| r.shown()).unwrap_or_default();
    (report, rows)
}

/// Decision 0431 R7: a file read from the memo has its tape values for a cross-file rule. `a.py` is not edited, so
/// the second run reads it from the memo; its annotation's node, which `shown` reads across files, is still there,
/// and the rows are a fresh run's.
#[test]
fn a_file_read_from_the_memo_has_its_tape_values_for_a_cross_file_rule() {
    let files = [("a.py", "x: \"Foo\" = 1\n"), ("b.py", "class Foo:\n    pass\n")];
    let make = |name: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ws")).unwrap();
        for (path, text) in files {
            std::fs::write(dir.join("ws").join(path), text).unwrap();
        }
        dir
    };
    let dir = make("r7-values");
    let (_, first) = cross_file_values_run(&dir);
    assert_eq!(first, [["Foo", "b.py", "1"]].map(|r| r.map(str::to_owned).to_vec()).to_vec(), "a full run's rows");
    std::fs::write(dir.join("ws/b.py"), "# an edit\nclass Foo:\n    pass\n").unwrap();
    let (edited, rows) = cross_file_values_run(&dir);
    assert_eq!(edited.memo.0, 1, "a.py is read from the memo: {:?}", edited.memo);
    assert_eq!(edited.guest_parses, 0, "no text is parsed: {}", edited.guest_parses);
    let fresh = make("r7-values-fresh");
    std::fs::write(fresh.join("ws/b.py"), "# an edit\nclass Foo:\n    pass\n").unwrap();
    let (_, want) = cross_file_values_run(&fresh);
    assert_eq!(rows, want, "a.py's tape value, read from the memo, is a fresh run's");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&fresh);
}

/// Decision 0431 R11 (step 6): a file's programs are made once, at its parse, and kept with its tape. A fresh run
/// makes the HTML file's script at its parse and the batch reads it off the file's record; a run of another program
/// (no memo hit) finds the file's tape in the store and its script with it, parsing nothing; a tape whose record is
/// not known (stored before step 6) has its script made in the batch. The rows are the same each time.
#[test]
fn a_files_programs_are_kept_with_its_tape() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("programs-kept");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::write(dir.join("ws/a.html"), "<p>x</p>\n<script>f(1)</script>\n").unwrap();
    let run = |extra: &str| -> (super::Report, Vec<Vec<String>>) {
        crate::install_store_functions();
        let rule = dir.join("program.rerail");
        let text = format!(".language javascript\n.embedded on\n.decl calls(x: symbol, p: symbol, l: number)\ncalls(x, p, l) :- kind(c, \"call_expression\"), text(c, x), docof(c, d), doc(d, p, _), line(c, l).\n.output calls\n{extra}");
        std::fs::write(&rule, text).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let out = dir.join("out");
        let _ = std::fs::remove_dir_all(&out);
        let report = super::run(&program, &dir.join("ws"), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows = super::super::output::Outputs::open(&out).unwrap().get("calls").map(|r| r.shown()).unwrap_or_default();
        (report, rows)
    };
    let want = [["f(1)", "a.html", "2"]].map(|r| r.map(str::to_owned).to_vec()).to_vec();
    let (fresh, rows) = run("");
    assert_eq!(rows, want, "a fresh run's rows");
    assert_eq!(fresh.programs, (1, 0), "made at the parse, read off the record: none by an embedder in the batch");
    // Another program: no memo hit, the file's tape found by its text.
    let (stored, rows) = run(".decl other(x: symbol)\nother(x) :- kind(c, \"identifier\"), text(c, x).\n.output other\n");
    assert_eq!(rows, want, "a stored tape's rows");
    assert_eq!(stored.file_tapes, (1, 0), "the file's tape is found, not parsed: {:?}", stored.file_tapes);
    assert_eq!(stored.programs, (1, 0), "its script found with it: {:?}", stored.programs);
    // A tape stored with no record: its script made in the batch, as before step 6.
    rusqlite::Connection::open(dir.join("store.sqlite")).unwrap().execute("UPDATE tape_objects SET programs = -1", []).unwrap();
    let (unknown, rows) = run(".decl third(x: symbol)\nthird(x) :- kind(c, \"number\"), text(c, x).\n.output third\n");
    assert_eq!(rows, want, "an unrecorded tape's rows");
    assert_eq!(unknown.programs, (0, 1), "made by its embedder in the batch: {:?}", unknown.programs);
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0431 R5's run: `program` (`.rerail`, with its includes) over `files` in a store of its own, giving the report
/// and the rows of `output`, as shown.
fn batch_run(name: &str, files: &[(&str, &str)], program: &str, output: &str) -> (super::Report, Vec<Vec<String>>) {
    batch_run_on(name, files, program, output, crate::code_rule_dsl::typing::file_local::Binding::Cpu)
}

/// `batch_run` under `binding`.
fn batch_run_on(name: &str, files: &[(&str, &str)], program: &str, output: &str, binding: crate::code_rule_dsl::typing::file_local::Binding) -> (super::Report, Vec<Vec<String>>) {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    let _ = std::fs::remove_dir_all(&dir);
    let ws = dir.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    for (path, text) in files {
        std::fs::write(ws.join(path), text).unwrap();
    }
    let rule = dir.join("program.rerail");
    std::fs::write(&rule, program).unwrap();
    let flat = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let report = super::run(&flat, &ws, None, &dir.join("out"), "program", binding, 1 << 18).unwrap();
    let rows = super::super::output::Outputs::open(&dir.join("out")).unwrap().get(output).map(|r| r.shown()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    (report, rows)
}

/// Decision 0431 R5: a chunk's batch is built once: an HTML file's scripts are its grammar's programs (§4), made from
/// its tape at its load, so the batch is built with them; no slice round asks for them, the batch never grows, and
/// every script's calls are read.
#[test]
fn a_files_programs_build_no_second_batch() {
    let html = "<!doctype html>\n<script>\nlet a = f(1);\n</script>\n<script type=\"module\">g(2)</script>\n";
    let (_, calls) = batch_run("r5-programs", &[("a.html", html)], r#"
.language html
#include "html/embeds.rerail"
.decl calls(x: symbol)
calls(x) :- kind(r, "raw_text"), parsed(r, "javascript", p), within(c, p), kind(c, "call_expression"), text(c, x).
.output calls
"#, "calls");
    assert_eq!(calls, [["f(1)"], ["g(2)"]].map(|r| r.map(str::to_owned).to_vec()).to_vec(), "both scripts' calls");
}

/// Decision 0431 R10: a string a rule parses is parsed in the program's fixpoint (§8). No slice round runs and the
/// batch never grows: each value is admitted once at its first lookup, `'Foo'` (a string inside an annotation's value)
/// in the same run, and parsed once. A scan lists no value's nodes (`flat`: a.py's identifiers alone).
#[test]
fn tape_values_are_read_in_the_fixpoint() {
    let program = r#"
.language python
.decl ann(n: node)
ann(r) :- string_annotation(s), parsed(s, "annotation", r).
ann(r) :- ann(s), kind(s, "string"), parsed(s, "annotation", r).
ann(c) :- ann(p), child(p, c, _).
.decl names(x: symbol)
names(x) :- ann(m), kind(m, "identifier"), text(m, x).
.decl flat(x: symbol)
flat(x) :- kind(m, "identifier"), text(m, x).
.output names, flat
"#;
    let files = [("a.py", "x: \"List[int]\" = []\ndef f(a: \"Optional['Foo']\"): pass\n")];
    let (report, names) = batch_run("r10-names", &files, program, "names");
    assert_eq!(report.values_admitted, 3, "the two annotations' values and 'Foo''s, each admitted once");
    assert_eq!(report.guest_parses, 3, "each value parsed once");
    let set = |rows: Vec<Vec<String>>| rows.into_iter().map(|r| r.join("\t")).collect::<std::collections::BTreeSet<String>>();
    assert_eq!(set(names), ["Foo", "List", "Optional", "int"].map(str::to_owned).into(), "both levels' values are read");
    let (_, flat) = batch_run("r10-flat", &files, program, "flat");
    assert_eq!(set(flat), ["a", "f", "x"].map(str::to_owned).into(), "no scan lists a value's nodes");
}

/// Decision 0431 R8: CPU and Metal give the same rows, the device reading the same tapes, with both kinds: tape values
/// read in the fixpoint (R10's program: values, a value inside one, and a scan that lists none), and a file's programs
/// (an HTML file's scripts), whose `docof` and lines are the file's (R1).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn cpu_and_metal_read_the_same_tape_values() {
    use crate::code_rule_dsl::typing::file_local::Binding;
    let values = r#"
.language python
.decl ann(n: node)
ann(r) :- string_annotation(s), parsed(s, "annotation", r).
ann(r) :- ann(s), kind(s, "string"), parsed(s, "annotation", r).
ann(c) :- ann(p), child(p, c, _).
.decl names(x: symbol)
names(x) :- ann(m), kind(m, "identifier"), text(m, x).
.decl flat(x: symbol)
flat(x) :- kind(m, "identifier"), text(m, x).
.output names, flat
"#;
    let files = [("a.py", "x: \"List[int]\" = []\ndef f(a: \"Optional['Foo']\"): pass\n")];
    // Every stratum the device can run is placed there (`placement::Binding::Always`), and the rows it reads, the
    // chunk's and its values', are read where they are (copy 0353 step 6 is not taken for them): a run with values
    // uploads no more than the same program over a file with none (whose uploads are the engine's, its empty versions
    // and the like), but for `parsed`: the grammar its atoms give is a filter of the input, and an input a constant
    // filters is uploaded still (Q310), its rows' three columns.
    let q266 = |rows: u64| 8 * 3 * rows;
    let uploaded = || rerail_prof::copies::sites().into_iter().filter(|(site, _)| site.name.contains("DevRel::upload")).map(|(_, t)| t.bytes).sum::<u64>();
    let on_device = |name: &str, files: &[(&str, &str)], program: &str, output: &str| {
        let bound = crate::code_rule_dsl::logic::plan::placement::set_binding(crate::code_rule_dsl::logic::plan::placement::Binding::Always);
        let before = uploaded();
        let got = batch_run_on(name, files, program, output, Binding::Metal);
        drop(bound);
        (got, uploaded() - before)
    };
    let none = [("a.py", "x = []\ndef f(a): pass\n")];
    for output in ["names", "flat"] {
        let (cpu, on_cpu) = batch_run_on(&format!("r8-{output}-cpu"), &files, values, output, Binding::Cpu);
        let ((metal, on_metal), bytes) = on_device(&format!("r8-{output}-metal"), &files, values, output);
        let (_, base) = on_device(&format!("r8-{output}-none"), &none, values, output);
        // `parsed`'s rows: the three annotations' places.
        assert!(bytes <= base + q266(3), "{output}: the values' rows are read where they lie ({bytes} bytes uploaded, {base} with no values)");
        assert!(!on_cpu.is_empty(), "{output}: rows");
        assert_eq!(on_metal, on_cpu, "{output}: the device's rows are the CPU's");
        assert_eq!((metal.values_admitted, metal.guest_parses), (cpu.values_admitted, cpu.guest_parses), "{output}: the same values, read alike");
    }
    let html = "<!doctype html>\n<script>\nlet a = f(1);\n</script>\n<script type=\"module\">g(2)</script>\n";
    let programs = r#"
.language html
#include "html/embeds.rerail"
.decl calls(x: symbol, d: number, l: number)
calls(x, d, l) :- kind(r, "raw_text"), parsed(r, "javascript", p), within(c, p), kind(c, "call_expression"), text(c, x), docof(c, d), line(c, l).
.output calls
"#;
    let (_, on_cpu) = batch_run_on("r8-programs-cpu", &[("a.html", html)], programs, "calls", Binding::Cpu);
    // The programs are the grammar's, made with the batch (§4): their rows are read where they lie, as the file's are,
    // and a run uploads no more than over a file with no script, but for `parsed`'s rows (Q310), one a script.
    let ((_, on_metal), bytes) = on_device("r8-programs-metal", &[("a.html", html)], programs, "calls");
    let (_, base) = on_device("r8-programs-none", &[("a.html", "<!doctype html>\n<p>x</p>\n")], programs, "calls");
    assert!(bytes <= base + q266(2), "a file's programs are read where they lie ({bytes} bytes uploaded, {base} with none)");
    assert_eq!(on_cpu.len(), 2, "both scripts' calls: {on_cpu:?}");
    assert_eq!(on_cpu[0][1], on_cpu[1][1], "both scripts are the file's: {on_cpu:?}");
    assert_eq!(on_metal, on_cpu, "a file's programs: the device's rows are the CPU's");
}

/// Decision 0431 R6, a guard of today's: within one run, equal texts are parsed once, whichever files hold them.
#[test]
fn equal_texts_are_parsed_once_a_run() {
    let (dir, run) = tape_value_runs("tv-once", &[("a.py", "x: \"List[int]\" = []\ny: \"Baz\" = 1\n"), ("b.py", "z: \"List[int]\" = []\n")]);
    let first = run("");
    assert_eq!(first.guest_parses, 2, "\"List[int]\" and \"Baz\", once each");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0431 R6: a text the last run parsed is not parsed again when its file is edited elsewhere. The file is
/// evaluated again; its annotation's tape is the last run's, found by the text's key.
#[test]
fn an_edit_elsewhere_parses_no_text_the_last_run_parsed() {
    let (dir, run) = tape_value_runs("tv-edit", &[("a.py", "x: \"List[int]\" = []\n")]);
    assert_eq!(run("").guest_parses, 1);
    std::fs::write(dir.join("ws/a.py"), "# an edit elsewhere\nx: \"List[int]\" = []\n").unwrap();
    let edited = run("");
    assert_eq!(edited.guest_parses, 0, "the annotation's text is the last run's");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0431 R6: a new file holding a text a run parsed before parses nothing: one text is one tape, wherever.
#[test]
fn a_new_file_with_a_text_parsed_before_parses_nothing() {
    let (dir, run) = tape_value_runs("tv-new-file", &[("a.py", "x: \"List[int]\" = []\n")]);
    assert_eq!(run("").guest_parses, 1);
    std::fs::write(dir.join("ws/b.py"), "y: \"List[int]\" = []\n").unwrap();
    assert_eq!(run("").guest_parses, 0, "b.py's annotation is a.py's tape");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0431 R6: an edited rule (a new program, every file evaluated again) parses no text the last run parsed,
/// as an edit parses no file (`an_edit_parses_no_file_the_last_run_read`).
#[test]
fn an_edited_rule_parses_no_text_the_last_run_parsed() {
    let (dir, run) = tape_value_runs("tv-rule-edit", &[("a.py", "x: \"List[int]\" = []\n"), ("b.py", "y: \"Optional[Foo]\" = None\n")]);
    assert_eq!(run("").guest_parses, 2);
    assert_eq!(run(", x != \"r6\"").guest_parses, 0, "the annotations' tapes are the last run's");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The prepared program is keyed by the linked program's version (0276 §2): an edit of a finding's words alone finds
/// it prepared (decision 0225 §3: the words are the writer's), and an edit of a rule prepares another.
#[test]
fn a_message_edit_finds_the_program_prepared_and_a_rule_edit_does_not() {
    crate::install_store_functions();
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("prepared-key");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.py"), "assert x\nwhile y:\n    pass\n").unwrap();
    let policy = |words: &str, kind: &str| {
        format!(".policy \"p\" revision 1\n.language python\n.violation P(at: node) \"{words}\"\nP(n) :- kind(n, \"{kind}\").\n")
    };
    let keys = |text: &str| {
        super::run(text, &dir, None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        super::with_prepared(|kept| kept.keys().copied().collect::<std::collections::BTreeSet<_>>())
    };
    super::with_prepared(|kept| kept.clear());
    let first = keys(&policy("one", "assert_statement"));
    assert!(!first.is_empty(), "the run kept its prepared program");
    assert_eq!(keys(&policy("two words", "assert_statement")), first, "the words alone changed: the same key");
    let edited = keys(&policy("one", "while_statement"));
    assert_eq!(edited.difference(&first).count(), 1, "a rule edit prepares another program");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0431 R9 (django-bugs' 2e9d16bc8, ported): a finding's lowering derives `rr_unplaced(clause)` for a finding
/// at a node whose document is no file's, an output; a run whose `rr_unplaced` holds rows is refused, naming the
/// clauses. A finding at a file's node derives none, and its run is not refused.
#[test]
fn a_finding_no_file_holds_is_refused_naming_its_clause() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("unplaced");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::write(dir.join("ws/a.py"), "assert x\n").unwrap();
    let text = ".policy \"p\" revision 1\n.language python\n.violation Assert(at: node) \"an assert\"\nAssert(n) :- kind(n, \"assert_statement\").\n";
    let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
    let shown = v.show();
    assert!(shown.contains("rr_unplaced(\"Assert\") :- Assert(") && shown.contains("!doc("), "the lowering writes R9's rule:\n{shown}");
    assert!(v.output_names().iter().any(|o| o == crate::code_rule_dsl::logic::rr::UNPLACED));
    let out = dir.join("out");
    let run = |text: &str, out: &std::path::Path| super::run(text, &dir.join("ws"), None, out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
    run(text, &out).unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "violation").unwrap().len(), 1, "a file's node: found");
    assert!(crate::code_rule_dsl::logic::plan::output::shown(&out, crate::code_rule_dsl::logic::rr::UNPLACED).unwrap().is_empty(), "and placed");
    // A run whose `rr_unplaced` holds a row (here derived outright) is refused, naming the clause.
    // Its rule reads `kind(_, "assert_statement")`, merged into `Assert`: an export now, the first run's output alone.
    // The file's memo entry holds no exported rows of it, so the file is a miss, not a hit with none.
    let second = format!("{text}rr_unplaced(\"Assert\") :- kind(_, \"assert_statement\").\n");
    let out2 = dir.join("out2");
    let err = run(&second, &out2).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("Assert") && err.contains("tape value"), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rule whose head holds no variable, over a file's nodes (`zz("A") :- kind(_, "assert_statement")`), holds its row
/// when a node matches, as one with a variable does, in a language's run (its program split by file).
#[test]
fn a_constant_head_over_a_files_nodes_holds_its_row() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("constant-head");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ws")).unwrap();
    std::fs::write(dir.join("ws/a.py"), "assert x\n").unwrap();
    let text = "\
.policy \"p\" revision 1
.language python
.violation Assert(at: node) \"an assert\"
Assert(n) :- kind(n, \"assert_statement\").
.decl c1(c: symbol)
c1(\"A\") :- kind(_, \"assert_statement\").
.decl c2(c: symbol)
c2(c) :- kind(_, \"assert_statement\"), c = \"A\".
.decl c3(c: symbol)
c3(\"A\") :- kind(n, \"assert_statement\"), n = n.
.decl v(n: node)
v(n) :- kind(n, \"assert_statement\").
.output c1, c2, c3, v
";
    let out = dir.join("out");
    super::run(text, &dir.join("ws"), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let got: Vec<(String, usize)> = ["c1", "c2", "c3", "v"].iter().map(|r| (r.to_string(), crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap().len())).collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got, [("c1".to_owned(), 1), ("c2".to_owned(), 1), ("c3".to_owned(), 1), ("v".to_owned(), 1)]);
}
