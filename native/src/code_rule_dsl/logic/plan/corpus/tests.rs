//! The corpus runner's tests. The engine compiles them (its facade of `logic::plan::corpus`): they parse files, and
//! the languages are registered by the crates above `rerail-logic` as the engine loads.

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
/// Decision 0076: cross-file rows memoised per import component. b imports a; c imports nothing. An edit
/// re-derives the edited file's component and its importers' only; `verify` derives everything again and
/// compares; the contract refuses a relation that reads an undeclared cross-file one, and a row naming a node
/// outside its closure.
#[test]
fn cross_file_rows_are_memoised_per_import_component() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("cross-memo");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |file: &str, body: &str| std::fs::write(dir.join(file), format!("{marker}{body}")).unwrap();
    write("a.py", "def f():\n    pass\n");
    write("b.py", "from a import f\ndef g():\n    pass\n");
    write("c.py", "def h():\n    pass\n");
    let program = |pragma: &str, extra: &str| {
        let rule = dir.join("uses.rr");
        std::fs::write(&rule, format!(r#"
.pragma "cross-memo" "{pragma}"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
{extra}
.output uses
"#)).unwrap();
        crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap()
    };
    let out = dir.join("out");
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let run = |text: &str| super::run(text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
    let uses = || std::fs::read_to_string(out.join("uses.csv")).unwrap();
    let text = program("uses", "");
    let first = run(&text).unwrap();
    assert_eq!(first.cross, (0, 3), "three components, none stored yet");
    let derived = uses();
    assert_eq!(derived.lines().count(), 1, "{derived}");
    assert!(derived.starts_with("b.py:"), "{derived}");
    assert_eq!(run(&text).unwrap().cross, (3, 0), "unchanged: every component found");
    assert_eq!(uses(), derived, "replayed rows are the derived ones");
    // An edit to c, which nothing imports: c alone is derived.
    write("c.py", "def h():\n    return 1\n");
    assert_eq!(run(&text).unwrap().cross, (2, 1));
    assert_eq!(uses(), derived);
    // An edit to a: a and b, which imports it, are derived; c is found.
    write("a.py", "def f():\n    return 2\n");
    assert_eq!(run(&text).unwrap().cross, (1, 2));
    assert_eq!(uses(), derived);
    // Verify: nothing replayed, every found component's stored rows equal to those derived.
    // SAFETY: only this test reads RERAIL_CROSS_MEMO while it is set (the others declare no pragma).
    unsafe { std::env::set_var("RERAIL_CROSS_MEMO", "verify") };
    let verified = run(&text);
    unsafe { std::env::remove_var("RERAIL_CROSS_MEMO") };
    assert_eq!(verified.unwrap().cross, (3, 0));
    assert_eq!(uses(), derived);
    // The contract: a memoised relation reading a cross-file relation the pragma does not name is refused.
    let reads = program("wide", ".decl wide(n: node)\nwide(n) :- uses(n, _).\n.output wide");
    let err = run(&reads).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("wide") && err.contains("uses"), "{err}");
    // A row about a whose other node is in b, which a does not import, is outside a's closure.
    let back = program("uses back", ".decl back(f: node, n: node)\nback(f, n) :- uses(n, f).\n.output back");
    let err = run(&back).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(err.contains("back") && err.contains("outside its import closure"), "{err}");
    // A memoised relation is stored, though inlining would unfold it into its readers (decision 0099): its derived
    // rows are read from its table to be recorded ("no such table: near" before).
    let near = program("uses near", ".decl near(n: node, f: node)\nnear(n, f) :- uses(n, f).\n.decl far(n: node)\nfar(n) :- near(n, _).\n.output far");
    // SAFETY: only this test reads RERAIL_INLINE_ONLY while it is set (the others declare no pragma).
    unsafe { std::env::set_var("RERAIL_INLINE_ONLY", "near") };
    let stored = run(&near);
    unsafe { std::env::remove_var("RERAIL_INLINE_ONLY") };
    stored.unwrap();
    assert_eq!(std::fs::read_to_string(out.join("far.csv")).unwrap().lines().count(), 1);
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
    let rule = dir.join("outer.rr");
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
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let store = dir.join("store.sqlite");
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.clone()));
    let run = |on: bool| {
        let options = if on { Value::Null } else { json!({"factorize": "off"}) };
        let report = super::run_with_options(&[&program], &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18, &super::Typing::default(), &options).unwrap();
        (report.memo.0, std::fs::read_to_string(out.join("outer.csv")).unwrap())
    };
    let (hits, whole) = run(false);
    assert_eq!(hits, 0, "a new memo");
    assert!(whole.lines().count() >= 4, "{whole}");
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
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let base = std::env::temp_dir().join(format!("rerail-k4-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let out = base.join("out");
    let findings = || {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
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
    let rules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
    let programs: Vec<String> = ["bandit-b301-pickle.rr", "bandit-b307-eval.rr"].iter().map(|r| crate::code_rule_dsl::logic::settings::program_text(&rules.join(r)).unwrap()).collect();
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
    let rule = base.join("reach.rr");
    std::fs::write(
        &rule,
        ".policy \"test-import-cycle\" revision 1\n.description \"An import that closes a cycle of imports.\"\n.language python\n.mode steer\n\
         .scope include \"**/*.py\"\n#include \"python/names.rr\"\n.decl dep(a: number, b: number)\ndep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).\n\
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
#include "python/names.rr"
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
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let base = std::env::temp_dir().join(format!("rerail-reach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (src, out) = (base.join("src"), base.join("out"));
    std::fs::create_dir_all(&src).unwrap();
    for (path, text) in REACH_START {
        std::fs::write(src.join(path), text).unwrap();
    }
    let rule = base.join("reach.rr");
    std::fs::write(&rule, REACH_RULE).unwrap();
    let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
    let findings = || {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
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
    assert!(first.iter().any(|f| f.contains("CYC")), "the first cycle's imports: {first:?}");
    let mut codes = std::collections::BTreeSet::new();
    for (path, text, what) in REACH_EDITS {
        std::fs::write(src.join(path), text).unwrap();
        let got = run(true);
        let want = run(false);
        assert_eq!(got, want, "after editing {path} ({what}), the findings are those of a run with an empty store");
        codes.extend(got.iter().flat_map(|f| ["CYC", "INCYC", "TOF", "SEESE", "DIRECT"].into_iter().filter(|c| f.contains(c))));
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
    let rule = dir.join("uses.rr");
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
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let read = |f: &str| std::fs::read_to_string(out.join(f)).unwrap();
        (report.cross_kept, read("uses.csv") + &read("unused.csv"))
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
    let rule = dir.join("p.rr");
    std::fs::write(&rule, "#include \"python/modules.rr\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut loaded: Vec<String> = std::fs::read_to_string(out.join("n_loaded.csv")).unwrap().lines().map(|l| l.rsplit('/').next().unwrap().to_owned()).collect();
        loaded.sort();
        (report.cone_kept.expect("a run with a tape cache keeps its cone's strata"), loaded)
    };
    let ((ran, all), first) = run("kept.sqlite");
    // The first run evaluates the cone's strata that phase 1 did not: phase 1 (`sources.rr`) resolves the same
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

/// Decision 0108: the cone run as rounds from the last run's final round finds what the gated fixpoint over
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
    let rule = dir.join("p.rr");
    std::fs::write(&rule, "#include \"python/modules.rr\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = std::cell::Cell::new(0);
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &ws, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let mut loaded: Vec<String> = std::fs::read_to_string(out.join("n_loaded.csv")).unwrap().lines().map(|l| l.rsplit('/').next().unwrap().to_owned()).collect();
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
    // back (decision 0108).
    std::fs::write(ws.join("c.py"), "x = 2\n").unwrap();
    both("a library shadowed by the workspace's module", &["a.pyi", "builtins.pyi"]);
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
    let rule = dir.join("uses.rr");
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
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        (report.cross_maintained.0, std::fs::read_to_string(out.join("unused.csv")).unwrap())
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
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("kept.sqlite")));
    let run = |extra: &str| {
        let rule = dir.join("p.rr");
        std::fs::write(&rule, format!("{shared}{extra}")).unwrap();
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
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
/// `compare.rr` the same, a third asks something else, a fourth asks nothing and reads the library's own copy.
#[test]
fn libraries_are_held_once_and_equal_requests_share_one_cone() {
    let ask = |kind: &str, own: &str| {
        format!(
            "#include \"python/compare.rr\"\ncmp_same_ask(a, b) :- kind(a, \"{kind}\"), kind(b, \"{kind}\"), a < b.\n.decl {own}(a: node)\n{own}(a) :- cmp_same(a, _).\n.output {own}\n"
        )
    };
    let quiet = "#include \"python/compare.rr\"\n.decl quiet(a: node)\nquiet(a) :- cmp_same(a, _).\n.output quiet\n".to_owned();
    let texts = [ask("call", "one"), ask("call", "two"), ask("name", "three"), quiet];
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let prog = super::combine_texts(&texts, "").unwrap();
    let heads = |rel: &str| prog.rules.iter().filter(|r| r.head.rel == rel).count();
    // The library's rule once, unprefixed, whatever the programs read.
    assert_eq!(heads("cmp_same"), 1);
    // Programs 0 and 1 ask the same: one copy of the cone; program 2 its own.
    let copies: std::collections::BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).filter(|r| r.starts_with('q') && r.ends_with("_cmp_same")).collect();
    assert_eq!(copies.len(), 2, "{copies:?}");
    assert!(copies.iter().all(|r| heads(r) == 1));
    assert!(!prog.rules.iter().any(|r| r.head.rel.starts_with('p') && r.head.rel.ends_with("_cmp_same")));
    // Each asker reads its request's copy; the quiet one reads the library's.
    let reads = |head: &str| -> Vec<String> {
        prog.rules.iter().filter(|r| r.head.rel == head).flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.to_string())).collect()
    };
    let (one, two, three) = (reads("p0_one"), reads("p1_two"), reads("p2_three"));
    assert_eq!(one, two);
    assert_ne!(one, three);
    assert_eq!(reads("p3_quiet"), ["cmp_same"]);
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
    let identity = count(&template, "SELECT id FROM store_identity");
    let probe = |n: usize| {
        let template = template.clone();
        // A thread named as the harness names a test's.
        std::thread::Builder::new().name(format!("corpus::tests::probe{n}")).spawn(move || {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("a.py"), "def f():\n    return 1\n").unwrap();
            let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap()).unwrap();
            let out = dir.path().join("out");
            super::run(&text, dir.path(), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            assert_eq!(std::fs::read_to_string(out.join("defs.csv")).unwrap().trim(), "f");
            let path = super::tape_cache_path().unwrap();
            assert_ne!(path, template);
            assert!(path.to_string_lossy().contains("rerail-test-store-"), "{}", path.display());
            let count = |sql: &str| -> i64 { rusqlite::Connection::open(&path).unwrap().query_row(sql, [], |r| r.get(0)).unwrap() };
            assert!(count("SELECT count(*) FROM tape_objects") > tapes, "the library's tapes and the test's own");
            assert_ne!(count("SELECT id FROM store_identity"), identity, "a copy is a store of its own");
            path.parent().unwrap().to_path_buf()
        }).unwrap()
    };
    let (a, b) = (probe(0), probe(1));
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    assert_ne!(a, b, "two tests, two stores");
    for dir in [a, b] {
        assert!(!dir.exists(), "{} is removed with its test", dir.display());
        #[cfg(target_vendor = "apple")]
        assert_eq!(super::test_store::open_under(&dir), 0, "nothing of {} is held open after its test", dir.display());
    }
}
