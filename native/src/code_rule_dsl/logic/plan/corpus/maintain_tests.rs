//! Decision 0364's required shape (its § "Required shape", R1–R10), one test per requirement: what the design the
//! user agreed says a session's edit does, checked on the run, not on how the code is laid out. A test the code does
//! not meet yet is ignored with the requirement and the step of t43's plan that meets it; that step's commit removes
//! the `ignore`. A requirement is not dropped or weakened here without the user.
//!
//! Each test runs a session over a few Python files: the edit's run starts from the last run's rows in memory, and
//! its outputs are compared with a run over an empty store (R10).

use super::maintain::{self, shape};
use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// A corpus of Python files, one program over it, and a session that keeps its runs' rows.
/// The shape counters (`maintain::shape`) and `strata_by_levels` are the process's: a test reads what its runs added
/// between two reads of them. Tests run in one process (Relax runs them on threads of one), so each session holds this
/// for its life, and no other test's run counts into its window (R8 read another test's 10 kept frames).
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Session {
    _one: std::sync::MutexGuard<'static, ()>,
    dir: PathBuf,
    marker: String,
    program: String,
    outputs: Vec<&'static str>,
    handle: super::session::Handle,
    fresh: usize,
}

/// What one edit's session run did.
struct Ran {
    report: super::Report,
    rows: Vec<BTreeSet<Vec<String>>>,
    shape: shape::Shape,
    by_levels: usize,
}

impl Session {
    fn new(name: &str, program: &str, outputs: &[&'static str], files: &[(&str, &str)]) -> Session {
        // A test that failed while holding it leaves it poisoned; the counters it guards are still sound.
        let one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("maintain-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = format!("# {name} {:?}\n", std::time::SystemTime::now());
        let rule = dir.join("program.rerail");
        std::fs::write(&rule, program).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let handle = super::session::Handle::open(&dir.join("session")).unwrap();
        let s = Session { _one: one, dir, marker, program, outputs: outputs.to_vec(), handle, fresh: 0 };
        for (file, body) in files {
            s.write(file, body);
        }
        s
    }

    fn write(&self, file: &str, body: &str) {
        let path = self.dir.join("src").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{}{body}", self.marker)).unwrap();
    }

    fn remove(&self, file: &str) {
        std::fs::remove_file(self.dir.join("src").join(file)).unwrap();
    }

    fn rows(&self, out: &std::path::Path) -> Vec<BTreeSet<Vec<String>>> {
        self.outputs.iter().map(|r| crate::code_rule_dsl::logic::plan::output::shown(out, r).unwrap_or_default().into_iter().collect()).collect()
    }

    /// The session's run over the files as they are now.
    fn run(&mut self) -> Ran {
        let out = self.dir.join("out");
        let _ = std::fs::remove_dir_all(&out);
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join("kept.sqlite")));
        // The run's own scope, held here: its shape is the run's count (decision 0442), read before it ends.
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        let (before, levels) = (shape::now(), maintain::strata_by_levels());
        let report = {
            let _bound = self.handle.bind();
            super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap()
        };
        let (shape, by_levels) = (shape::since(before), maintain::strata_by_levels() - levels);
        Ran { report, rows: self.rows(&out), shape, by_levels }
    }

    /// A run outside the session over a store kept from run to run (`kept-frames.sqlite`): what a new process
    /// does, its kept part read from the store's frames.
    fn run_kept(&mut self) -> Ran {
        let out = self.dir.join("out-kept");
        let _ = std::fs::remove_dir_all(&out);
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join("kept-frames.sqlite")));
        // The counts are the run's (decision 0441): read in a run of this test's own, which the corpus run joins, so
        // they include those of its loops' halves another of the pool's threads steals (`rerail_context::inside_pool`).
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        let (before, levels) = (shape::now(), maintain::strata_by_levels());
        let report = super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let (shape, by_levels) = (shape::since(before), maintain::strata_by_levels() - levels);
        Ran { report, rows: self.rows(&out), shape, by_levels }
    }

    /// A run over the files as they are now from an empty store, outside the session (R10's truth).
    fn fresh(&mut self) -> Vec<BTreeSet<Vec<String>>> {
        self.fresh += 1;
        let out = self.dir.join(format!("fresh{}", self.fresh));
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join(format!("fresh{}.sqlite", self.fresh))));
        super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        self.rows(&out)
    }

    /// The session's run after an edit, checked against a fresh run (R10).
    fn edit(&mut self, what: &str) -> Ran {
        let ran = self.run();
        assert_eq!(ran.rows, self.fresh(), "R10: after {what}, the session's outputs are a fresh run's");
        ran
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Every kind of stratum 0328 §6 and 0364 name, cross-file: a join with a projection (`uses`), a negation
/// (`unused`), an aggregate over a derived relation (`fanout`) and over an input (`width`), a computed head (`named`),
/// and recursion over the import graph, which can cycle (`reach`). Its comparison keeps it a recursive stratum: a
/// plain transitive closure is the native reach index (decision 0119), which is not maintained by levels. And a
/// relation that is a copy of an input only while its other rule's cross-file relation is empty (`defq`, as `cparam`
/// over its file-local export in the corpus): it is derived, kept and maintained, not held as a view.
const KINDS: &str = r#"
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
.decl fanout(f: node, n: number)
fanout(f, n) :- def(f, _), n = count : { uses(_, f) }.
.decl width(f: node, n: number)
width(f, n) :- uses(_, f), n = count : { child(f, _, _) }.
.decl named(n: node, q: symbol)
named(n, @dot(q, q)) :- uses(n, f), def(f, q).
.decl edge(a: symbol, b: symbol)
edge(a, b) :- kind(n, "import_statement"), field(n, "name", t), text(t, b), docof(n, d), doc(d, _, a).
edge(a, b) :- kind(n, "import_from_statement"), field(n, "module_name", t), text(t, b), docof(n, d), doc(d, _, a).
.decl reach(a: symbol, b: symbol)
reach(a, b) :- edge(a, b).
reach(a, c) :- reach(a, b), edge(b, c), b != "__none".
.decl lonely(q: symbol)
lonely(a) :- reach(a, a), a = "__none".
.decl defq(n: node, q: symbol)
defq(n, q) :- def(n, q).
defq(n, q) :- lonely(q), def(n, q).
.decl used_def(q: symbol)
used_def(q) :- defq(_, q), imp(_, q).
.output unused
.output fanout
.output width
.output named
.output reach
.output used_def
"#;
const KIND_OUTPUTS: &[&str] = &["unused", "fanout", "width", "named", "reach", "used_def"];

fn kinds(name: &str) -> Session {
    Session::new(name, KINDS, KIND_OUTPUTS, &[
        ("a.py", "def f():\n    pass\ndef k():\n    pass\n"),
        ("b.py", "from a import f\ndef g():\n    pass\n"),
        ("c.py", "from a import f\nfrom b import g\nimport d\ndef h():\n    pass\n"),
        ("d.py", "import c\ndef i():\n    pass\n"),
    ])
}

/// Small edits of every kind: a use added, uses removed, an import cycle cut and closed again.
const SMALL_EDITS: &[(&str, &str, &str)] = &[
    ("b.py", "from a import f\nfrom a import k\ndef g():\n    pass\n", "a use added"),
    ("c.py", "from b import g\nimport d\ndef h():\n    pass\n", "a use removed"),
    ("d.py", "def i():\n    pass\n", "the import cycle c → d → c cut"),
    ("d.py", "import c\ndef i():\n    pass\n", "the cycle closed again"),
];

/// R1 and R2: an edit's run holds no relation under a maintenance name (`__old`, `__minus`, `__plus`, `__del`, …),
/// and copies no rows to state a change. A relation's change is its version, read in place: the kept relation and
/// the mark of its rows and kills (0328 §5, 0364 stage 2), for inputs, for strata the mode maintains and for strata
/// derived again alike.
#[test]
fn r1_r2_an_edit_holds_no_maintenance_relation_and_copies_no_rows() {
    let mut s = kinds("r1r2");
    s.run();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        assert_eq!(ran.shape.maintenance_held, 0, "R1: after {what}, no relation is held under a maintenance name");
        assert_eq!(ran.shape.rows_copied, 0, "R2: after {what}, no row is copied to state a change");
    }
}

/// R2 in a session (t43 step 13): the changed documents' memo entries are found (under the memo plan's id and the
/// store path), so the inputs' deltas are read from them, not every document hashed. (A few still are: 0364's open
/// item.)
#[test]
#[ignore = "R2, Q405: at the cycle cut no input the session asks is read from the entries (`child`, a semi-join's; `edge_at`, rows of other documents' nodes)"]
fn r2_a_session_reads_its_inputs_deltas_from_the_changed_entries() {
    let mut s = kinds("r2-entries");
    s.run();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        assert!(ran.shape.inputs_from_entries > 0, "R2: after {what}, inputs' deltas are read from the changed documents' entries ({} hashed)", ran.shape.inputs_hashed);
    }
}

/// R2 in a new process: an input's delta is found by its documents, hashing only the changed documents' rows.
#[test]
fn r2_a_new_process_hashes_only_the_changed_documents_rows_of_its_inputs() {
    let mut s = kinds("r2-kept");
    s.run_kept();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.run_kept();
        assert_eq!(ran.rows, s.fresh(), "R10: after {what}, the kept run's outputs are a fresh run's");
        // Its last run's documents are the base documents' table (decision 0458, `retain::base_documents`), kept with
        // the memo plan it was made under: an input's rows of the documents as they were are not read again, only
        // the changed documents' rows are hashed.
        assert!(ran.shape.inputs_hashed > 0, "R2: after {what}, the inputs' deltas are found by their documents");
        assert_eq!(ran.shape.inputs_hashed_whole, 0, "R2: after {what}, no input's every row is hashed, only the changed documents' ({} inputs)", ran.shape.inputs_hashed);
        // An input's delta names its rows where they lie, old and new (`retain::rows_delta`), as a session's does.
        assert_eq!(ran.shape.rows_copied, 0, "R2: after {what}, a new process copies no row to state a change");
    }
}

/// R2 in a new process, whole: its kept rows are read where they lie (decision 0439: the kept entries bound in place,
/// `kept_read::gathered`'s views of their granules), not decoded into memory. No path decodes a kept row, so what
/// is checked is that the kept entries were bound and no row was copied to state the change.
#[test]
fn r2_a_new_process_reads_its_kept_rows_in_place() {
    let mut s = kinds("r2-in-place");
    s.run_kept();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.run_kept();
        assert_eq!(ran.rows, s.fresh(), "R10: after {what}, the kept run's outputs are a fresh run's");
        assert!(ran.shape.frames_read > 0, "R2: after {what}, the kept entries are bound where they lie");
        assert_eq!(ran.shape.rows_copied, 0, "R2: after {what}, no kept row is copied");
    }
}

/// R1 in the source: no engine code outside tests names a maintenance relation. The names live in one list
/// (`maintain::shape::MAINTENANCE_SUFFIXES`), which this reads.
#[test]
fn r1_the_engine_source_names_no_maintenance_relation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic");
    let root = if root.exists() { root } else { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/code_rule_dsl/logic") };
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                stack.push(p);
            } else if name.ends_with(".rs") && !name.contains("tests") && !p.ends_with("corpus/maintain.rs") {
                for (i, line) in std::fs::read_to_string(&p).unwrap().lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    if shape::MAINTENANCE_SUFFIXES.iter().any(|s| code.contains(s)) {
                        found.push(format!("{}:{}: {}", p.display(), i + 1, line.trim()));
                    }
                }
            }
        }
    }
    assert!(found.is_empty(), "R1: maintenance names in the engine:\n{}", found.join("\n"));
}

/// R3: a stratum an edit reaches is skipped, maintained by the mode, or derived again for one of two reasons only:
/// its part's first run (nothing kept) or 0090 §4's threshold (R4). Every kind 0328 §6 names is maintained. Each
/// edit here follows its part's first run, so none is a first run: a view of an input (`doc_tape`, `document`), which
/// keeps nothing, is held again in place, its version its input's, and is not derived again as one.
#[test]
fn r3_a_reached_stratum_is_derived_again_only_on_its_first_run_or_past_the_threshold() {
    let allowed: BTreeSet<&str> = [maintain::THRESHOLD].into();
    let mut s = kinds("r3");
    s.run();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        let other: BTreeMap<&str, usize> = ran.report.cross_again.iter().filter(|(why, _)| !allowed.contains(**why)).map(|(w, n)| (*w, *n)).collect();
        assert!(other.is_empty(), "R3: after {what}, strata derived again for other reasons: {other:?}");
        assert!(ran.report.cross_maintained.0 > 0, "R3: after {what}, the mode maintained strata");
    }
}

/// R4: an edit that reaches too much of a stratum (0090 §4: its changed reads' deltas past θ of the rows those reads
/// hold, or the mode's lines priced above a derive) derives it again, and its levels come fresh from that derive. A
/// small edit does not.
#[test]
fn r4_an_edit_that_reaches_most_of_a_stratum_derives_it_again() {
    // The test's relations are small: a floor under θ (0090 §4's is 1,024 rows) above a small edit's deltas and below
    // the corpus's. nextest runs a test as a process of its own.
    unsafe { std::env::set_var("RERAIL_AGAIN_MIN", "50") };
    let files: Vec<(String, String)> = (0..40).map(|i| (format!("m{i}.py"), format!("import m{}\ndef f{i}():\n    pass\n", (i + 1) % 40))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    let mut s = Session::new("r4", KINDS, KIND_OUTPUTS, &refs);
    s.run();
    s.write("m0.py", "import m1\nimport m7\ndef f0():\n    pass\n");
    let small = s.edit("one import added");
    assert!(!small.report.cross_again.contains_key(maintain::THRESHOLD), "R4: a small edit is maintained: {:?}", small.report.cross_again);
    // Every definition renamed: `def`'s deltas are 80 rows, twice the 40 rows of the strata that read it.
    for i in 0..40 {
        s.write(&format!("m{i}.py"), &format!("import m{}\ndef g{i}():\n    pass\n", (i + 1) % 40));
    }
    let big = s.edit("every definition renamed");
    assert!(big.report.cross_again.contains_key(maintain::THRESHOLD), "R4: an edit that reaches most of a stratum derives it again: {:?}", big.report.cross_again);
}

/// R4 at scale: a maintained line's work follows its changed read's delta, not the corpus. Each line is led by its
/// driver's delta, a negated read's too: its delta read positively first, the body planned with its variables bound.
/// Left where the plan tested it, a negated read's line walked its other reads whole (the data layer's 2-file edit
/// at 334 files: 113 removal joins, 10 s of 31).
#[test]
fn r4_a_changed_read_s_delta_leads_its_line() {
    let mut s = kinds("r4-led");
    s.run();
    let mut led = 0;
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        assert_eq!(ran.shape.drivers_unled, 0, "R4: after {what}, every changed read's delta leads its line");
        led += ran.shape.negated_led;
    }
    assert!(led > 0, "R4: `unused`'s negated read of `used` drove lines of its own");
}

/// R4 with readers: a stratum derived again past θ hands its readers its delta against its kept rows, and a reader
/// maintained over that delta has versions of its own for the strata above it. On the O125 replay, `cls_q` derived
/// again past θ, and so did `type_of`. Its reader `type_first`, a `min` over it (a `min` over a node here: one over a symbol is refused), was maintained and stopped:
/// "type_of has no version".
const AGAIN_THEN_MAINTAINED: &str = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl edge(a: symbol, b: symbol)
edge(a, b) :- kind(n, "import_statement"), field(n, "name", t), text(t, b), docof(n, d), doc(d, _, a).
.decl a(n: node, q: symbol)
a(n, q) :- def(n, q), q = "m0.f0".
.decl b(q: symbol, m: symbol)
b(q, m) :- a(_, q), edge(m, _).
.decl c(m: symbol)
c(m) :- b(_, m).
.decl first(n: node)
first(n) :- a(_, _), n = min n2 : { a(n2, _) }.
.output c
.output first
"#;

#[test]
fn r4_a_stratum_derived_again_past_theta_feeds_maintained_readers() {
    unsafe { std::env::set_var("RERAIL_AGAIN_MIN", "50") };
    let files: Vec<(String, String)> = (0..40).map(|i| (format!("m{i}.py"), format!("import m{}\ndef f{i}():\n    pass\n", (i + 1) % 40))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    let mut s = Session::new("r4-readers", AGAIN_THEN_MAINTAINED, &["c", "first"], &refs);
    s.run();
    for i in 0..40 {
        s.write(&format!("m{i}.py"), &format!("import m{}\ndef g{i}():\n    pass\n", (i + 1) % 40));
    }
    let big = s.edit("every definition renamed");
    assert!(big.report.cross_again.contains_key(maintain::THRESHOLD), "R4: `a` is derived again past θ: {:?}", big.report.cross_again);
    assert!(big.report.cross_maintained.0 > 0, "R4: its readers are maintained over its delta: {:?}", big.report.cross_maintained);
}

/// R5: levels. A ring of imports entered by one edge holds itself up through its rows; when the entry goes, every row
/// it held goes too (0364's ring: a recheck alone keeps all 12). The recursive stratum is maintained by levels.
#[test]
fn r5_a_ring_that_loses_its_entry_loses_every_row_it_held() {
    let ring: Vec<(String, String)> = (1..=6).map(|i| (format!("r{i}.py"), format!("import r{}\n", i % 6 + 1))).collect();
    let mut files: Vec<(&str, &str)> = ring.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    files.extend([("x.py", "import e\n"), ("e.py", "import r1\n")]);
    let mut s = Session::new("r5", KINDS, &["reach"], &files);
    s.run();
    s.write("e.py", "def e():\n    pass\n");
    let ran = s.edit("the ring's entry removed");
    assert!(ran.by_levels > 0, "R5: the recursive stratum is maintained by levels: maintained {:?}, again {:?}", ran.report.cross_maintained, ran.report.cross_again);
    // The ring's modules are named r1 … r6; x and e have no `r` in their names.
    let into_ring: Vec<_> = ran.rows[0].iter().filter(|r| !r[0].contains('r') && r[1].contains('r')).collect();
    assert!(into_ring.is_empty(), "R5: neither x nor e reaches the ring: {into_ring:?}");
    s.write("e.py", "import r1\n");
    s.edit("the entry restored");
    // R2 past θ: with θ 0 the ring's stratum is derived again; its kept rows have levels and the derive's rows none
    // (O125's `cls_q`). It is still made a version of its kept rows in place, its levels dropped as a derive resets
    // them: no row is copied to state its change.
    let switched = super::switches::set(&[("RERAIL_AGAIN_THETA", "0"), ("RERAIL_AGAIN_MIN", "0")]);
    s.write("e.py", "def e():\n    pass\n");
    let ran = s.edit("the entry removed, derived again past θ");
    drop(switched);
    assert!(ran.report.cross_again.contains_key(maintain::THRESHOLD), "R4: θ 0 derives it again: {:?}", ran.report.cross_again);
    assert_eq!(ran.shape.rows_copied, 0, "R2: a levelled stratum derived again is a version of its kept rows");
}

/// R6: a recursive component that cannot hold a row up by itself (every recursive step goes strictly down the tree:
/// the static check proves it) keeps no levels; its candidates are rechecked once each, as a non-recursive
/// stratum's are.
#[test]
fn r6_a_component_that_cannot_cycle_keeps_no_levels() {
    let program = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl used(f: node)
used(f) :- imp(_, q), def(f, q).
.decl below(a: node, c: node)
below(a, c) :- used(a), child(a, c, _).
below(a, c) :- below(a, b), child(b, c, _).
.decl depth(a: node, n: number)
depth(a, n) :- used(a), n = count : { below(a, _) }.
.output depth
"#;
    let mut s = Session::new("r6", program, &["depth"], &[("a.py", "def f():\n    x = 1\n"), ("b.py", "from a import f\n")]);
    s.run();
    s.write("a.py", "def f():\n    x = 1\n    y = [2, 3]\n");
    let ran = s.edit("the used function's body grows");
    assert!(ran.report.cross_maintained.1 > 0, "R6: the recursive stratum is maintained");
    assert_eq!(ran.by_levels, 0, "R6: a component that cannot cycle is not maintained by levels");
    // `below` is cross-file (it reads `used`), and the cross part holds no tree: its measure's key (`ast_depth`) is
    // an input there, so its candidates are ordered and each is rechecked once, not once per support that goes.
    assert_eq!(ran.shape.measures_unkeyed, 0, "R6: the cross part reads its measure's key");
    // Rows go: their candidates are rechecked in the measure's order (or again as their supports go), with no
    // levels, and the outputs are a fresh run's (`edit`).
    for (file, body, what) in [("a.py", "def f():\n    x = 1\n", "the used function's body shrinks"), ("b.py", "x = 2\n", "the import goes"), ("b.py", "from a import f\n", "the import comes back")] {
        s.write(file, body);
        let ran = s.edit(what);
        assert_eq!(ran.by_levels, 0, "R6: after {what}, not maintained by levels");
        assert_eq!(ran.shape.measures_unkeyed, 0, "R6: after {what}, the cross part reads its measure's key");
    }
}

/// R7: the mode's lines are the program's plans, priced (0328 §4, 0122): the recheck's order with its head bound is
/// priced as the other lines are, not taken from one greedy pass (Q208).
#[test]
fn r7_the_recheck_order_is_priced() {
    let mut s = kinds("r7");
    s.run();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        assert_eq!(ran.shape.rechecks_unpriced, 0, "R7: after {what}, every recheck line is priced");
    }
}

/// R8: a session's kept rows stay in place between its runs (0328 §5): an edit's run reads no kept frame back.
#[test]
#[ignore = "R8, Q405: a phase's run is in memory (a429546488), no session's, and binds its kept part from the store at each edit"]
fn r8_a_session_edit_reads_no_kept_frame() {
    let mut s = kinds("r8");
    s.run();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        assert_eq!(ran.shape.frames_read, 0, "R8: after {what}, the session read no kept frame");
    }
}

/// R9: the driver decides, for each stratum an edit reaches, one of three things: skip it (no read changed), maintain
/// it by the mode, or derive it again (R3's reasons). The report's counts are those three and cover every stratum.
#[test]
fn r9_each_reached_stratum_is_skipped_maintained_or_derived_again() {
    let mut s = kinds("r9");
    s.run();
    s.remove("d.py");
    let ran = s.edit("a file removed");
    let r = &ran.report;
    let again: usize = r.cross_again.values().sum();
    assert_eq!(r.cross_maintained.2, again, "R9: every stratum derived again has its reason");
    assert_eq!(r.cross_strata.expect("a session's run").0, r.cross_skipped + r.cross_maintained.0 + again, "R9: every reached stratum is skipped, maintained or derived again: {:?}", (r.cross_strata, r.cross_skipped, r.cross_maintained, &r.cross_again));
}
