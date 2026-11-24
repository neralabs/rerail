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
    /// The last edit's outputs, not yet checked against a fresh run: checked as the session ends, where no edit
    /// followed (`fresh_tier`: by default a session checks its last state only).
    unchecked: std::cell::RefCell<Option<(String, Vec<BTreeSet<Vec<String>>>)>>,
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
        let s = Session { _one: one, dir, marker, program, outputs: outputs.to_vec(), handle, fresh: 0, unchecked: Default::default() };
        for (file, body) in files {
            s.write(file, body);
        }
        s
    }

    fn write(&self, file: &str, body: &str) {
        // An edit follows the one left unchecked: its state was not the last.
        self.unchecked.borrow_mut().take();
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
        // Its store is the session's (`session/session.sqlite`, which its binding selects), not a test's.
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
        // The run's own scope, as the session's run holds it: its shape is the run's count (decision 0442). Without
        // it the test's thread read a fresh count each time, every one 0, and R2's assertions held of nothing.
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

    /// The session's run after an edit, checked against a fresh run (R10): at once in the full tier, else as the
    /// session ends if no edit follows (`fresh_tier`).
    fn edit(&mut self, what: &str) -> Ran {
        let ran = self.run();
        if super::fresh_tier::every() {
            assert_eq!(ran.rows, self.fresh(), "R10: after {what}, the session's outputs are a fresh run's");
        } else {
            *self.unchecked.borrow_mut() = Some((what.to_owned(), ran.rows.clone()));
        }
        ran
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // The last edit's state (R10), where the test did not fail before it.
        let unchecked = self.unchecked.borrow_mut().take();
        if let Some((what, rows)) = unchecked
            && !std::thread::panicking()
        {
            assert_eq!(rows, self.fresh(), "R10: after {what}, the session's outputs are a fresh run's");
        }
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
/// derived again alike. A session's runs are its store's (t33 step 3): its inputs' deltas are found by their changed
/// documents, as a new process's are (`r2_a_new_process_…`), never by every row.
#[test]
fn r1_r2_an_edit_holds_no_maintenance_relation_and_copies_no_rows() {
    let mut s = kinds("r1r2");
    s.run();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        assert_eq!(ran.shape.maintenance_held, 0, "R1: after {what}, no relation is held under a maintenance name");
        assert_eq!(ran.shape.rows_copied, 0, "R2: after {what}, no row is copied to state a change");
        // The inputs' deltas are found by their changed documents: a base relation's from its changed documents' old
        // and new rows, a kept input's by its changed documents' rows hashed, never every row.
        assert!(ran.shape.inputs_from_owners + ran.shape.inputs_hashed > 0, "R2: after {what}, the inputs' deltas are found by their documents ({:?})", ran.shape);
        assert_eq!(ran.shape.inputs_hashed_whole, 0, "R2: after {what}, no input's every row is hashed ({} inputs)", ran.shape.inputs_hashed);
    }
}

/// R2 in a new process: its inputs' deltas are found by their changed documents, never by every row. A base relation
/// keeps no rows, only its documents' digest (0458 step 4), so its delta is its changed documents' old and new rows,
/// given beside it (`input_delta::Before`, `shape::inputs_from_owners`); a kept input's is its changed documents'
/// rows hashed (`retain::document_delta_except`, `shape::inputs_hashed`).
#[test]
fn r2_a_new_process_hashes_only_the_changed_documents_rows_of_its_inputs() {
    let mut s = kinds("r2-kept");
    s.run_kept();
    for (i, (file, body, what)) in SMALL_EDITS.iter().enumerate() {
        s.write(file, body);
        let ran = s.run_kept();
        if super::fresh_tier::compared(i, SMALL_EDITS.len()) {
            assert_eq!(ran.rows, s.fresh(), "R10: after {what}, the kept run's outputs are a fresh run's");
        }
        assert!(
            ran.shape.inputs_from_owners + ran.shape.inputs_hashed > 0,
            "R2: after {what}, the inputs' deltas are found by their documents ({:?})",
            ran.shape
        );
        assert_eq!(ran.shape.inputs_hashed_whole, 0, "R2: after {what}, no input's every row is hashed, only the changed documents' ({} inputs)", ran.shape.inputs_hashed);
        // Its changed inputs' old rows of the unchanged documents are their new rows, read in place
        // (`retain::OldInput`, 0353): only the changed documents' rows are read from the frames.
        assert_eq!(ran.shape.inputs_read_whole, 0, "R2: after {what}, no changed input's old rows are read whole from the frames");
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
    for (i, (file, body, what)) in SMALL_EDITS.iter().enumerate() {
        s.write(file, body);
        let ran = s.run_kept();
        if super::fresh_tier::compared(i, SMALL_EDITS.len()) {
            assert_eq!(ran.rows, s.fresh(), "R10: after {what}, the kept run's outputs are a fresh run's");
        }
        assert!(ran.shape.frames_read > 0, "R2: after {what}, the kept entries are bound where they lie");
        assert_eq!(ran.shape.rows_copied, 0, "R2: after {what}, no kept row is copied");
    }
}

/// R1 in the source: no engine code outside tests names a maintenance relation. The names live in one list
/// (`maintain::shape::MAINTENANCE_SUFFIXES`, the run's `run::shape`), which this reads.
#[test]
fn r1_the_engine_source_names_no_maintenance_relation() {
    let mut found = Vec::new();
    // The logic's sources, in whichever crates keep them.
    let mut stack = crate::source_tree::source_paths("code_rule_dsl/logic");
    assert!(!stack.is_empty(), "the logic's sources");
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                stack.push(p);
            } else if name.ends_with(".rs") && !name.contains("tests") && !p.ends_with("logic/run/shape.rs") {
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
    // the corpus's. Held as a switch while the test runs (`switches::set`): set on the process and left, it was every
    // later test's floor (a_session_maintains_a_cross_file_aggregate derived a stratum again under `cargo test`).
    let files: Vec<(String, String)> = (0..40).map(|i| (format!("m{i}.py"), format!("import m{}\ndef f{i}():\n    pass\n", (i + 1) % 40))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    let mut s = Session::new("r4", KINDS, KIND_OUTPUTS, &refs);
    // Taken after the session, which holds the maintain tests one at a time: taken before, it waited on a test
    // whose runs waited on it.
    let _floor = super::switches::set(&[("RERAIL_AGAIN_MIN", "50")]);
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

/// R4: a line inserted moves every node after it (`node`'s `start` and `end`), and no node's kind: a stratum that
/// reads nodes' kinds alone is maintained, its delta the rows the lines added (`plan::unchanged_as_read`). Its every
/// moved node was a change to it: ep2's one-line edit derived 391 strata again past the threshold, 66 over `kind`.
#[test]
fn r4_a_line_inserted_is_no_change_to_a_stratum_that_reads_no_offset() {
    // One file holds most nodes, so its nodes moved are most of `node`'s rows.
    let big: String = (0..60).map(|i| format!("def f{i}():\n    pass\n")).collect();
    // `used_kind` reads `kind` in the cross part, past a join of two files' rows, as `width` reads `child`.
    let program = format!("{KINDS}\n.decl used_kind(f: node, k: symbol)\nused_kind(f, k) :- uses(_, f), kind(f, k).\n.output used_kind\n");
    let outputs: Vec<&'static str> = KIND_OUTPUTS.iter().copied().chain(["used_kind"]).collect();
    let mut s = Session::new("r4-moved", &program, &outputs, &[("a.py", big.as_str()), ("b.py", "from a import f1\ndef g():\n    pass\n")]);
    // Taken after the session, which holds the maintain tests one at a time: taken before, it waited on a test
    // whose runs waited on it.
    let _floor = super::switches::set(&[("RERAIL_AGAIN_MIN", "50")]);
    s.run();
    // A line inserted in the first function's body, as ep2's: every node after it moves, and no child's index but
    // the function's own.
    s.write("a.py", &big.replacen("def f0():\n", "def f0():\n    x = 1\n", 1));
    let moved = s.edit("a line inserted above every node but one function's");
    assert!(!moved.report.cross_again.contains_key(maintain::THRESHOLD), "R4: a line inserted is maintained: {:?}", moved.report.cross_again);
    assert!(moved.report.cross_maintained.0 > 0, "R4: the mode maintained strata: {:?}", moved.report.cross_again);
}

/// R4: a line inserted at a file's top moves every place in it (`line`'s delta every row of the file's), and a
/// stratum that reads findings' places is maintained: past the floor its lines start from the findings and probe
/// the moved places by their nodes (`Stats::lead_free`), priced under the derive. Led by the moved places, the mode
/// was priced over the derive and derived the stratum again: ep2's import edit derived `violation` again, and its
/// findings' places made 33 documents. The append before it, past the floor too, adds fewer places than there are
/// findings: its line is led by them, and kept. At the insert, re-priced under a search's saving (`REPLAN_ROWS`),
/// that order is searched before the comparison.
#[test]
fn a_finding_s_places_moved_are_maintained() {
    // For this test's runs alone (`switches::set`): set on the process and left, the floor was every later test's in
    // the process (a session test's cross-file aggregate derived again, a warm host-file edit made every document).
    let _floor = super::switches::set(&[("RERAIL_AGAIN_MIN", "50")]);
    let program = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl used(f: node)
used(f) :- imp(_, q), def(f, q).
.decl placed(f: node, l: number)
placed(f, l) :- used(f), line(f, l).
.output placed
"#;
    // `used`: 200 functions of z.py that b.py imports, and a0.py's `g0`, and its `h0` once the append defines it
    // (the append reaches `placed`). a0.py to a10.py hold the places an edit moves.
    let z: String = (0..200).map(|i| format!("def f{i}():\n    pass\n")).collect();
    let b: String = (0..200).map(|i| format!("from z import f{i}\n")).chain(["from a0 import g0\nfrom a0 import h0\n".to_owned()]).collect();
    let big: String = (0..30).map(|i| format!("def g{i}():\n    pass\n")).collect();
    let mut files: Vec<(String, String)> = vec![("z.py".into(), z), ("b.py".into(), b)];
    files.extend((0..11).map(|i| (format!("a{i}.py"), big.clone())));
    let refs: Vec<(&str, &str)> = files.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    let mut s = Session::new("r4-places", program, &["placed"], &refs);
    s.run();
    let more: String = (0..12).map(|i| format!("def h{i}():\n    pass\n")).collect();
    s.write("a0.py", &format!("{big}{more}"));
    s.edit("functions appended");
    s.write("a0.py", &format!("import os\n{big}{more}"));
    let moved = s.edit("an import inserted at the top");
    assert!(!moved.report.cross_again.contains_key(maintain::THRESHOLD), "R4: the findings' places moved are maintained: {:?}", moved.report.cross_again);
    assert!(moved.report.cross_maintained.0 > 0, "R4: the mode maintained strata: {:?}", moved.report.cross_again);
}

/// R4 at scale: a maintained line's work follows its changed read's delta, not the corpus. A negated read's line is
/// led by its delta: read positively first, the body planned with its variables bound; a positive one's delta leads
/// where it is the cheapest start (`a_finding_s_places_moved_are_maintained`).
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
    let files: Vec<(String, String)> = (0..40).map(|i| (format!("m{i}.py"), format!("import m{}\ndef f{i}():\n    pass\n", (i + 1) % 40))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    let mut s = Session::new("r4-readers", AGAIN_THEN_MAINTAINED, &["c", "first"], &refs);
    // Taken after the session, which holds the maintain tests one at a time: taken before, it waited on a test
    // whose runs waited on it.
    let _floor = super::switches::set(&[("RERAIL_AGAIN_MIN", "50")]);
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
}

/// R2 past θ: with θ 0 the ring's stratum is derived again; its kept rows have levels and the derive's rows none
/// (O125's `cls_q`). It is still made a version of its kept rows in place, its levels dropped as a derive resets them:
/// no row is copied to state its change. A session's runs are its store's (t33 step 3), and a kept entry that keeps
/// its rows' levels is read in with them (`kept_read::Entry::rel_as`, 0353's `copy: load`) until its levels are read
/// where they lie.
#[test]
#[ignore = "R2: a kept entry's levels are read in with its rows (kept_read::Entry::rel_as, 0353 copy) until they are read in place (0401 step 7)"]
fn r2_a_levelled_stratum_derived_again_is_a_version_of_its_kept_rows() {
    let ring: Vec<(String, String)> = (1..=6).map(|i| (format!("r{i}.py"), format!("import r{}\n", i % 6 + 1))).collect();
    let mut files: Vec<(&str, &str)> = ring.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    files.extend([("x.py", "import e\n"), ("e.py", "import r1\n")]);
    let mut s = Session::new("r2-levels", KINDS, &["reach"], &files);
    s.run();
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

/// R8: a session's kept rows stay in place between its runs (0328 §5). A session's runs are its store's (t33 step 3):
/// its kept rows stay where its store keeps them, mapped once a process while the chain's rows hold
/// (`kept_read::Bound`), and an edit's run binds what it reads of them there: it copies none to state a change and
/// reads no changed input whole. (`frames_read` counts those binds: a kept entry bound is a view, not a read back.)
#[test]
fn r8_a_session_edit_reads_its_kept_rows_in_place() {
    let mut s = kinds("r8");
    s.run();
    for (file, body, what) in SMALL_EDITS {
        s.write(file, body);
        let ran = s.edit(what);
        assert_eq!(ran.shape.rows_copied, 0, "R8: after {what}, the session copied no kept row");
        assert_eq!(ran.shape.inputs_read_whole, 0, "R8: after {what}, no changed input's old rows were read whole");
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

/// Q421: a view of an input's columns (`kind`, `docof` of `node`) is never kept as rows (0353), cold or after an edit,
/// on the kept frames' route and in a session: `retain::after` refuses a view by the program's rules
/// (`Cut::views`) whatever form a run held it in, and the maintain mode holds it as a view.
#[test]
fn an_edit_keeps_no_rows_of_an_inputs_view() {
    const P: &str = r#"
.decl doc(d: number, path: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input doc, node, child, field, text
.decl kind(n: node, k: symbol)
kind(n, k) :- node(_, n, k, _, _).
.decl docof(n: node, d: number)
docof(n, d) :- node(d, n, _, _, _).
.decl def(n: node, x: symbol)
def(n, x) :- kind(n, "function_definition"), field(n, "name", c), text(c, x).
.decl named(x: symbol, d: number)
named(x, d) :- kind(n, "call"), field(n, "function", f), text(f, x), def(m, x), docof(m, d), kind(m, _).
.output named
"#;
    let mut s = Session::new("q361", P, &["named"], &[("a.py", "def f():\n    pass\n"), ("b.py", "f()\n")]);
    let views = |s: &Session, f: &str, when: &str| {
        let kept: Vec<String> = crate::content_store::kept_facts::by_relation(&s.dir.join(f))
            .unwrap()
            .into_iter()
            .filter(|((r, _), _)| r == "kind" || r == "docof")
            .map(|((r, w), (n, rows, _))| format!("{r}/{w}: {n} entries, {rows} rows"))
            .collect();
        assert!(kept.is_empty(), "{when}: {f} keeps rows of a view: {kept:?}");
    };
    s.run_kept();
    views(&s, "kept-frames.sqlite", "cold");
    s.write("b.py", "f()\nf()\n");
    s.run_kept();
    views(&s, "kept-frames.sqlite", "after an edit");
    s.run();
    views(&s, "session/session.sqlite", "a session's first run");
    s.write("b.py", "f()\n");
    s.run();
    views(&s, "session/session.sqlite", "a session's edit");
}

/// Decision 0094 §8, the incremental layout-invariance gate: a move costs about the moved definitions. t22's move,
/// in small: a definition many files call goes to a module of its own (`_moved_0.py`), its old module re-exporting it
/// where it was, so no importer changes. Answers are a fresh run's (R10, `fresh_tier`). Its cost is the joins' work
/// (`shape::join_work`, counted, not timed), set against an edit of the same definition in place, a statement added to
/// its body, which touches one file where the move touches two and changes no import: the move may cost a few times
/// the edit, and at most half a cold run (it touches 2 files of 27). Strata derived again are not the measure: one that is cheap to derive is derived (0090 §4's priced
/// clause), and the reach indexes over the import graph are made again whenever it changes (decision 0119 §3), their
/// readers maintained from their delta.
///
/// It fails today (2026-11-24): joins' work cold 61,264, the edit in place 17,738, the move 93,291. The move derives
/// nothing again but the two reach indexes (their readers maintained from their delta), and maintains 145 strata;
/// four lines of the big recursive component cost 58,623 of it. Each is led by its delta, `docof`'s changed nodes,
/// then reads `rr_cx_reach` and `docof` by document: every node of every file that imports the changed one, before
/// `qv` binds it to nothing (`bv(s, x, k, v) :- ibind(s, x, q), qv(m9, q, k, v), docof(s, d9), docof(m9, e9),
/// rr_cx_reach(d9, e9)`, and `pv`'s like it). The edit in place pays the same lines, 10,142 of its 17,738.
#[test]
#[ignore = "0094 §8: a mode line led by a delta walks whole documents (docof through rr_cx_reach), so the move costs more than a cold run (edit-cost, 0364 R4 at scale)"]
fn a_moved_definition_costs_about_itself() {
    let program = std::fs::read_to_string(crate::source_tree::native().join("src/code_rule_dsl/typing/programs/calls_cx.rerail")).unwrap();
    let helpers = "class Reader:\n    def read(self, s):\n        return s.strip()\n\n\ndef parse(s):\n    n = int(s)\n    return n\n\n\ndef keep(x):\n    return x\n";
    let mut files: Vec<(String, String)> = vec![("pkg/__init__.py".into(), String::new()), ("pkg/helpers.py".into(), helpers.into())];
    // Callers of `parse`, and modules of their own that call one another, so the move is a small part of the corpus.
    for i in 0..12 {
        files.push((format!("pkg/use{i}.py"), format!("from pkg.helpers import parse, Reader\n\n\ndef run{i}(s):\n    r = Reader()\n    return parse(r.read(s))\n")));
    }
    for i in 0..12 {
        files.push((format!("pkg/own{i}.py"), format!("from pkg.own{} import step{}\n\n\nclass Box{i}:\n    def get(self):\n        return step{}(1)\n\n\ndef step{i}(x):\n    return Box{i}().get() if x else x\n", (i + 1) % 12, (i + 1) % 12, (i + 1) % 12)));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(f, b)| (f.as_str(), b.as_str())).collect();
    let mut s = Session::new("move", &program, &["call_target", "call_open", "type_of"], &refs);
    let cold = s.run();
    s.write("pkg/helpers.py", &helpers.replace("    n = int(s)\n", "    n = int(s)\n    n = n + 0\n"));
    let edit = s.edit("a statement added to parse");
    s.write("pkg/helpers.py", helpers);
    s.edit("the statement taken out again");
    let moved_def = "def parse(s):\n    n = int(s)\n    return n\n";
    s.write("pkg/_moved_0.py", moved_def);
    s.write("pkg/helpers.py", &helpers.replace(moved_def, "from pkg._moved_0 import parse\n"));
    let moved = s.edit("parse moved to a module of its own");
    let again = |r: &Ran| r.report.cross_again.iter().map(|(w, n)| (*w, *n)).collect::<BTreeMap<&str, usize>>();
    let report = format!(
        "joins' work: cold {}, edit in place {}, move {}; edit: again {:?}, maintained {:?}; move: again {:?}, maintained {:?}",
        cold.shape.join_work, edit.shape.join_work, moved.shape.join_work, again(&edit), edit.report.cross_maintained, again(&moved), moved.report.cross_maintained
    );
    eprintln!("0094 §8: {report}");
    assert!(edit.shape.join_work > 0, "the edit in place ran joins: {report}");
    assert!(moved.shape.join_work <= 3 * edit.shape.join_work, "0094 §8: the move costs past 3 times the edit in place: {report}");
    assert!(2 * moved.shape.join_work <= cold.shape.join_work, "0094 §8: the move costs past half a cold run: {report}");
}
