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
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let run = |text: &str| super::run(text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18);
    let uses = || crate::code_rule_dsl::logic::plan::output::shown(&out, "uses").unwrap();
    let text = program("uses", "");
    let first = run(&text).unwrap();
    assert_eq!(first.cross, (0, 3), "three components, none stored yet");
    let derived = uses();
    assert_eq!(derived.len(), 1, "{derived:?}");
    let outputs = crate::code_rule_dsl::logic::plan::output::Outputs::open(&out).unwrap();
    assert_eq!(outputs.rows("uses")[0].get(0).path(), Some("b.py"), "{derived:?}");
    drop(outputs);
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
    let switched = super::switches::set(&[("RERAIL_CROSS_MEMO", "verify")]);
    let verified = run(&text);
    drop(switched);
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
    let switched = super::switches::set(&[("RERAIL_INLINE_ONLY", "near")]);
    let stored = run(&near);
    drop(switched);
    stored.unwrap();
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "far").unwrap().len(), 1);
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
    let rule = dir.join("p.rr");
    std::fs::write(&rule, "#include \"python/modules.rr\"\n.decl n_loaded(p: symbol)\nn_loaded(p) :- loaded(p, _, _).\n.output n_loaded\n").unwrap();
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
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

/// Decision 0173 (t36 item 7): a maintained run reads its programs as versions. The maintained part is a tree of
/// its version (`maintain::rewrite`), decoded by no reader, and the run's copy of the prepared split shares the
/// parts' trees, copying none. The outputs are a run's with an empty store.
#[test]
fn a_maintained_run_decodes_and_copies_no_tree() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("no-trees");
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
    let run = |store: &str| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let report = super::run(&text, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        (report.cross_maintained.0, crate::code_rule_dsl::logic::plan::output::shown(&out, "unused").unwrap())
    };
    run("kept.sqlite");
    write("c.py", "def h():\n    pass\n");
    crate::code_rule_dsl::logic::program_image::forget_trees_decoded();
    let (maintained, rows) = run("kept.sqlite");
    // The run's program is a version (`lower::Lowered`): no tree to copy or read beside it. A tree is made only by
    // decoding one (`Version::program`), counted.
    let decoded = crate::code_rule_dsl::logic::program_image::trees_decoded();
    let (_, fresh) = run("fresh.sqlite");
    assert_eq!(rows, fresh);
    assert!(maintained > 0, "a changed stratum maintained from the kept rows ({maintained})");
    assert_eq!(decoded.0, 0, "the maintained run decoded trees from their versions, the last at {:?}", decoded.1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Decision 0101 §3: a maintained stratum's pieces run as one component per level, ordered by unit gates (each
/// negation of a piece, `used__minus :- used__del, !used__ins`, reads it once it is whole). Edits that remove, add
/// and move imports delete, insert and derive again; after each, the outputs are a run's with an empty store. Under
/// the device's binding the pieces stay apart (the device takes no gates).
#[test]
fn a_maintained_strata_pieces_run_as_one_component() {
    let marker = format!("# {:?}\n", std::time::SystemTime::now());
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("together");
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
.decl lone(f: node)
lone(f) :- unused(f), !used(f), def(f, q), !imp(_, q).
.output unused
.output lone
"#).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = dir.join("out");
    use crate::code_rule_dsl::typing::file_local::Binding;
    let run_on = |store: &str, binding: Binding| {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join(store)));
        let before = super::maintain::pieces_together();
        let report = super::run(&text, &dir, None, &out, "program", binding, 1 << 18).unwrap();
        let rows = ["unused", "lone"].map(|r| crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap());
        (report.cross_maintained.0, rows, super::maintain::pieces_together() - before)
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
    assert!(together > 0, "a stratum's pieces ran together");
    // Under the device's binding the pieces stay apart: the device takes no gates. The outputs are the same.
    write("c.py", "from a import k\ndef h():\n    pass\n");
    let (maintained, rows, pieces) = run_on("kept.sqlite", Binding::Metal);
    let (_, fresh, _) = run("fresh-metal.sqlite");
    assert_eq!(rows, fresh, "under the device's binding");
    assert!(maintained > 0, "under the device's binding: maintained");
    assert_eq!(pieces, 0, "under the device's binding the pieces stay apart");
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
    let rule = dir.join("reach.rr");
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
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
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
    let prog = super::combine_texts(&texts).unwrap();
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
    assert_eq!(count(&template, "SELECT count(*) FROM store_identity"), 0, "the template is no store until copied");
    let probe = |n: usize| {
        let template = template.clone();
        // A thread named as the harness names a test's.
        std::thread::Builder::new().name(format!("corpus::tests::probe{n}")).spawn(move || {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("a.py"), "def f():\n    return 1\n").unwrap();
            let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap()).unwrap();
            let out = dir.path().join("out");
            super::run(&text, dir.path(), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap(), [["f"]]);
            let path = super::tape_cache_path().unwrap();
            assert_ne!(path, template);
            assert!(path.to_string_lossy().contains("rerail-test-stores"), "{}", path.display());
            let count = |sql: &str| -> i64 { rusqlite::Connection::open(&path).unwrap().query_row(sql, [], |r| r.get(0)).unwrap() };
            let held = count("SELECT count(*) FROM tape_objects");
            assert!(held > tapes, "the library's tapes and the test's own: {held} tapes, the template's {tapes}");
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
        assert_eq!(super::test_store::open_under(&dir), 0, "nothing of {} is held open after its test", dir.display());
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
#[cfg(target_os = "macos")]
#[test]
#[ignore = "a benchmark: the steps of a test's store"]
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
    let rule = base.join("names.rr");
    std::fs::write(
        &rule,
        ".language python\n.scope include \"**/*.py\"\n#include \"python/names.rr\"\n\
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

/// A request's context is every literal beside it (`rr::lower`'s `request_context`): `type_ctx(r) :- type_ctx(s),
/// str_node(s), parsed(s, "annotation", r)` asks to parse the strings in a type context, not every string of the
/// file. Its context was the inputs alone (`str_node`), and a module of 111k string pairs (netbox's `un_locode.py`)
/// asked for every one: 297k parses, past 25 minutes and 33 GB.
#[test]
fn a_parse_request_asks_only_for_the_strings_its_rule_reads() {
    let base = crate::code_rule_dsl::fixtures::temp().with_extension("request-context");
    let _ = std::fs::remove_dir_all(&base);
    let src = base.join("tree");
    std::fs::create_dir_all(&src).unwrap();
    let data: String = (0..50).map(|i| format!("    ('AD-{i:03}', 'Place {i}'),\n")).collect();
    std::fs::write(src.join("data.py"), format!("CODES = (\n{data})\n")).unwrap();
    std::fs::write(src.join("types.py"), "from typing import List\nx: \"List[int]\" = []\ny = \"not a type\"\n").unwrap();
    let rule = base.join("typing.rr");
    std::fs::write(&rule, ".language python\n.scope include \"**/*.py\"\n#include \"python/typing.rr\"\n.decl typed(n: node)\ntyped(n) :- type_ctx(n).\n.output typed\n").unwrap();
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
    let rule = base.join("annotated.rr");
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
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap()).unwrap();
    let out = dir.path().join("out");
    super::run(&text, dir.path(), None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    // One output image, no csv (decision 0239): the run wrote its rows there.
    assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap_or_default(), [["f"]]);
    let names: Vec<String> = super::observe(|o| o.listing(dir.path()).unwrap().iter().map(|(n, _)| n.clone()).collect());
    assert_eq!(names, ["a.py"], "the outputs are not in the listing");
}

/// The run after a cold one, over what the cold run read unchanged, writes nothing (`RERAIL_WARM`): it names the same
/// sources and plans, so it records no manifest of its own, and it makes no choice the cold run did not keep. It was
/// news three ways: the cold run named a source no later run reads (a chunk's tree counts, still written though frames
/// carry their own statistics since); a nested run reused whole (`reuse::reused`) named none of its plans; and the
/// cross part whose every stratum was kept, a program of no rules, kept planning choices of its own. Each run is a
/// process of its own, as the CLI's are: this test binary again, running `one_corpus_run_for_a_parent`.
#[test]
fn the_run_after_a_cold_one_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    // `RERAIL_TEST_KEEP=<dir>`: the runs' store and outputs left there, to look at.
    let kept = std::env::var_os("RERAIL_TEST_KEEP").map(std::path::PathBuf::from);
    let base = kept.as_deref().unwrap_or(temp.path());
    let src = base.join("src");
    std::fs::create_dir_all(src.join("pkg")).unwrap();
    std::fs::write(src.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(src.join("pkg/loader.py"), "import pickle as p\n\ndef load(b):\n    return p.loads(b)\n").unwrap();
    std::fs::write(src.join("main.py"), "from pkg.loader import load\nload(b'x')\n").unwrap();
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
        if warm {
            child.env("RERAIL_WARM", "list");
        }
        let output = child.output().unwrap();
        // Each fault and the line after it, which says what was written.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let faults: Vec<String> = lines.iter().enumerate().filter(|(_, l)| l.starts_with("rerail: RERAIL_WARM")).map(|(i, l)| format!("{l}{}", lines.get(i + 1).map_or(String::new(), |n| format!(" ({})", n.trim())))).collect();
        assert!(output.status.success(), "the run's process: {}; {}", output.status, faults.join("; "));
    };
    run(false);
    let cold = manifests();
    assert!(!cold.is_empty(), "the cold run recorded its manifests");
    run(true);
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
        std::fs::write(&path, format!("#include \"core.rr\"\n.decl idents(s: number, e: number)\nidents(s, e) :- node(_, n, \"identifier\", s, e), kind(n, \"identifier\"){extra}.\n.output idents\n")).unwrap();
        path
    };
    let (before, edited) = (rule("before.rr", ""), rule("edited.rr", ", s >= 0"));
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
        let output = child.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let fault: Vec<&str> = stderr.lines().skip_while(|l| !l.starts_with("rerail: RERAIL_SQL_STRICT")).take(2).collect();
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
    // `RERAIL_CHILD_PROGRAM=<file.rr>`: that rule file, else the pickle pack's.
    let rules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
    let file = std::env::var_os("RERAIL_CHILD_PROGRAM").map_or_else(|| rules.join("bandit-b301-pickle.rr"), std::path::PathBuf::from);
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
/// manifest left them out, and the next process's run, naming them again, recorded one more.
#[test]
fn a_second_run_in_one_process_records_the_first_runs_manifest() {
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
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "code_rule_dsl::logic::plan::corpus::tests::one_corpus_run_for_a_parent", "--ignored", "--test-threads", "1", "--nocapture"])
            .env("RERAIL_TAPE_CACHE", &store)
            .env("RERAIL_CHILD_RUN", &src)
            .env("RERAIL_CHILD_OUT", base.join("out"))
            .env("RERAIL_CHILD_RUNS", runs.to_string())
            .output()
            .unwrap();
        assert!(output.status.success(), "the runs' process: {}", output.status);
    };
    // A process first keeps phase 0's graphs as images (decision 0246): every later run reads them, and so names none
    // of the bootstrap run's plans (`phase0::Images`). Without it, the baseline's run derived the graphs and named
    // those plans, which the runs after it, reading the images, do not.
    run(1);
    // A process of one run: what a run names.
    run(1);
    let one = manifests();
    // A process of two: the second names the same.
    run(2);
    let two = manifests();
    let new: Vec<&(Vec<u8>, Vec<u8>)> = two.iter().filter(|m| !one.contains(m)).collect();
    assert!(new.is_empty(), "each run named what a process's first run names: no manifest of its own ({} new)", new.len());
}

/// Decision 0275: one program handed to its run (`run_read`) is hidden once and kept under its texts' digest, as a
/// pack is: the hidden program read back is the one hiding makes, and a second combine is the store's.
#[test]
fn a_handed_programs_hidden_form_is_kept_under_its_texts() {
    let src = "#include \"core.rr\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n";
    let read = crate::code_rule_dsl::logic::rr::parse(src).unwrap();
    let (made, hidden, kept) = super::combine_handed(&read).unwrap();
    assert_eq!(format!("{made:?}"), format!("{hidden:?}"), "a miss hides the program handed");
    assert_eq!(kept.map(|k| format!("{k:?}")), Some(format!("{hidden:?}")), "kept under the texts' digest, read back as hidden");
    let (again, _, _) = super::combine_handed(&read).unwrap();
    assert_eq!(format!("{again:?}"), format!("{hidden:?}"), "the store's, the second time");
}

/// Two programs combined with the library they share stratify (decision 0174, E32): PYI062's `literal_down` negates
/// `typing_subscript`, which reads `value` through `exports`. The node attributes are answered on request over
/// the combined program's readers; each part lowered alone made `rr_value_req` read `literal_leaf`, recursive
/// with `value` through that negation, and the pair was refused as not stratifiable.
#[test]
fn a_pair_of_programs_asking_for_an_attribute_through_the_library_stratifies() {
    use crate::code_rule_dsl::typing::file_local::Binding;
    let dir = std::env::temp_dir().join(format!("rerail-pyi062-pyi059-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.py"), "from typing import Generic, Literal, TypeVar\nT = TypeVar('T')\nx: Literal[1, 1]\nclass C(Generic[T], int): pass\n").unwrap();
    let rules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
    let programs: Vec<String> = ["ruff-pyi062-duplicate-literal-member.rr", "ruff-pyi059-generic-not-last-base-class.rr"]
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
    let rule = dir.join("named.rr");
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
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
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
    let rule = dir.join("named.rr");
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
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
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
        // A measurement aid (`version_trace`): each program made from another written to a directory; the programs
        // are the same with it on or off.
        "RERAIL_VERSION_TRACE",
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
        "RERAIL_SESSION",
        // The library's directory: its files' paths and digests are in the texts' digest (the module graph).
        "RERAIL_RR_LIB",
        // The cost model's file: its estimates steer plan choices, which keep the program's meaning (`plan::run`).
        "RERAIL_COSTS",
        // Keyed through what it adds: the observer's facts (the source files), which the prepared key hashes.
        "RERAIL_SOURCES_PHASE",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl");
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
    // `corpus.rs` itself: its preparation (`run_observed` up to the prepared program) and `combine_read`.
    let corpus = std::fs::read_to_string(root.join("logic/plan/corpus.rs")).unwrap();
    let region = |from: &str, to: &str| -> String {
        let a = corpus.find(from).unwrap_or_else(|| panic!("corpus.rs has no {from:?}"));
        let b = a + corpus[a..].find(to).unwrap_or_else(|| panic!("corpus.rs has no {to:?} after {from:?}"));
        corpus[a..b].to_owned()
    };
    // Combining: `combine_read`, what it calls that reads a switch (`scoping::guard`), and the `.rr` lowering its
    // `rr::parse` runs (`logic/rr`, `front`). The combined program is kept under `COMBINING` alone.
    let mut combining = sources(&["logic/rr", "logic/front.rs", "logic/front", "logic/plan/corpus/scoping.rs"]);
    combining.push(("corpus.rs, combine_read".to_owned(), region("\nfn combine_read(", "\n}\n")));
    // Preparing: `run_observed` up to the prepared program, what it calls that reads a switch (`hiding::fold_own`,
    // memo, session, profile, costs) and the passes. The prepared key holds `PREPARING` and the combined program's
    // key. Not `plan/keeps.rs` and `plan/choices.rs`: what a relation keeps is chosen in `plan::run` and kept under
    // `choices::key`, which holds the switches those read (`RERAIL_INLINE`, `RERAIL_INLINE_ONLY`,
    // `RERAIL_OBSERVED_STEPS`).
    let mut preparing = sources(&[
        "logic/simplify.rs", "logic/strings.rs", "logic/reach_form.rs", "logic/magic.rs", "logic/magic", "logic/plan/lower.rs",
        "logic/plan/corpus/cross_memo.rs", "logic/plan/corpus/hiding.rs", "logic/plan/corpus/memo.rs", "logic/plan/corpus/prepared.rs",
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
    // hangs off the prepared key (the run's extraction: `with_long_text`'s text nodes and anchors, the span
    // prefilter, both in `facts/vtab/syntax.rs`; the rest of `run_observed`). One read nowhere there keys nothing it
    // changes: a session's next run cold for nothing.
    for text in sources(&["facts/vtab/syntax.rs"]).into_iter().map(|(_, t)| t).chain([region("\nfn run_observed(", "\n}\n")]) {
        seen.extend(reads(&text));
    }
    let unread: Vec<&str> = super::COMBINING.iter().chain(super::PREPARING).copied().filter(|s| !seen.contains(*s)).collect();
    assert!(unread.is_empty(), "switches listed as combining or preparing that nothing there reads: {unread:?}");
}
