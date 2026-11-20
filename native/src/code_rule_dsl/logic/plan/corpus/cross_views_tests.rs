//! Decision 0458's requirements (R1–R5), one test each: the cross part reads the base relations off every file's
//! tape, and nothing keeps their rows. A test the code does not meet yet is ignored with the step of 0458 that meets
//! it; that step's commit removes the `ignore`. A requirement is not dropped or weakened here without the user.
//!
//! Each test runs one program over a few Python files whose cross-file rules read base relations of another file's
//! nodes: `node`, `child`, `field`, `text`, `kind`, `line`, `col` and `docof`, each of a definition another file uses.

use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Cross-file: `uses` joins one file's imports with another's definitions, and each output reads base relations of
/// the definitions used, so the split leaves those reads in the cross part.
const PROGRAM: &str = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl shape(f: node, k: symbol, l: number, c: number)
shape(f, k, l, c) :- uses(_, f), child(f, b, _), kind(b, k), line(b, l), col(b, c).
.decl named(f: node, t: symbol)
named(f, t) :- uses(_, f), field(f, "name", c), text(c, t).
.decl span(f: node, s: number, e: number)
span(f, s, e) :- uses(_, f), node(_, f, _, s, e).
.decl home(u: node, p: symbol)
home(u, p) :- uses(u, f), docof(f, d), doc(d, p, _).
.output shape
.output named
.output span
.output home
"#;
const OUTPUTS: &[&str] = &["shape", "named", "span", "home"];
/// The base relations the cross-file rules read.
const BASE: &[&str] = &["node", "child", "field", "text", "kind", "line", "col", "docof", "doc"];

const FILES: &[(&str, &str)] = &[
    ("a.py", "def f(x):\n    return x\n\ndef k():\n    pass\n"),
    ("b.py", "from a import f\n\ndef g(y, z):\n    return f(y)\n"),
    ("c.py", "from a import f\nfrom b import g\n\ndef h():\n    pass\n"),
    ("d.py", "from c import h\n\ndef i():\n    h()\n"),
];

/// A corpus of Python files and the program over it.
struct Corpus {
    dir: PathBuf,
    marker: String,
    program: String,
    runs: usize,
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("cross-views-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = format!("# {name} {:?}\n", std::time::SystemTime::now());
        let rule = dir.join("program.rerail");
        std::fs::write(&rule, PROGRAM).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let c = Corpus { dir, marker, program, runs: 0 };
        for (file, body) in FILES {
            c.write(file, body);
        }
        c
    }

    fn write(&self, file: &str, body: &str) {
        let path = self.dir.join("src").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{}{body}", self.marker)).unwrap();
    }

    fn store(&self) -> PathBuf {
        self.dir.join("kept.sqlite")
    }

    /// A run over the files as they are now, over the store `store`: its report and its outputs' rows.
    fn run_over(&mut self, store: &Path) -> (super::Report, Vec<BTreeSet<Vec<String>>>) {
        self.runs += 1;
        let out = self.dir.join(format!("out{}", self.runs));
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.to_owned()));
        let report = super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let rows = OUTPUTS.iter().map(|r| crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap_or_default().into_iter().collect()).collect();
        (report, rows)
    }

    /// A run over the corpus's own store, kept from run to run.
    fn run(&mut self) -> (super::Report, Vec<BTreeSet<Vec<String>>>) {
        let store = self.store();
        self.run_over(&store)
    }

    /// A run from an empty store: the truth an edit's run is checked against.
    fn fresh(&mut self) -> Vec<BTreeSet<Vec<String>>> {
        let store = self.dir.join(format!("fresh{}.sqlite", self.runs));
        self.run_over(&store).1
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Every output has rows: the program reads each base relation across files, so a test of no rows tests nothing.
fn every_output_has_rows(rows: &[BTreeSet<Vec<String>>]) {
    for (rel, rows) in OUTPUTS.iter().zip(rows) {
        assert!(!rows.is_empty(), "{rel} has no rows: the corpus does not exercise it");
    }
}

/// R1: on a cold run, each base relation the cross part reads gives the rows its files' tapes hold. The outputs
/// read them of another file's definitions: `f(x)`'s two children, its name and span, and the file it is in.
#[test]
fn r1_a_cold_run_reads_every_base_relation_of_another_files_nodes() {
    let mut c = Corpus::new("r1");
    let (_, rows) = c.run();
    every_output_has_rows(&rows);
    let named: BTreeSet<&str> = rows[1].iter().map(|r| r[1].as_str()).collect();
    assert_eq!(named, ["f", "g", "h"].into_iter().collect(), "the names of the definitions another file uses");
    let homes: BTreeSet<&str> = rows[3].iter().map(|r| r[1].as_str()).collect();
    assert_eq!(homes.len(), 3, "each used definition's file, {homes:?}");
    assert_eq!(rows, c.fresh(), "a second cold run reads the same rows");
}

/// R2: a file read from the per-file memo gives the cross part the rows an evaluated file gives. After one file's
/// edit the others are memo hits; the outputs are a cold run's over the edited tree.
#[test]
fn r2_a_file_read_from_the_memo_gives_the_rows_an_evaluated_file_gives() {
    let mut c = Corpus::new("r2");
    c.run();
    // d.py's edit: a, b and c are read from the memo, and their definitions are the ones read across files.
    c.write("d.py", "from c import h\nfrom a import f\n\ndef i():\n    h()\n");
    let (report, rows) = c.run();
    assert!(report.memo.0 >= 3, "the unchanged files are memo hits ({} hits)", report.memo.0);
    every_output_has_rows(&rows);
    assert_eq!(rows, c.fresh(), "the memo hits' base rows are an evaluated file's");
}

/// R3: after an edit that changes a definition another file uses, the base relations' versions are the changed
/// file's old and new rows, and the outputs are a cold run's over the edited tree.
#[test]
fn r3_an_edit_reads_the_changed_files_old_and_new_rows() {
    let mut c = Corpus::new("r3");
    c.run();
    let edits: &[(&str, &str)] = &[
        ("a.py", "def f(x, y, z):\n    return x\n\n\ndef k():\n    pass\n"),
        ("b.py", "from a import f\nfrom a import k\n\ndef g(y):\n    return f(y)\n"),
        ("a.py", "def k():\n    pass\n"),
    ];
    for (file, body) in edits {
        c.write(file, body);
        let (_, rows) = c.run();
        assert_eq!(rows, c.fresh(), "after {file}'s edit, the outputs are a cold run's");
    }
}

/// R4: no base relation's rows are kept: the chain names none of them after a cold run and an edit's run.
#[test]
#[ignore = "0458 step 4: the kept inputs' granules and the memo entries' base rows still keep them"]
fn r4_no_base_relation_is_kept() {
    let mut c = Corpus::new("r4");
    c.run();
    c.write("d.py", "from c import h\nfrom a import f\n\ndef i():\n    h()\n");
    c.run();
    let kept: BTreeSet<String> = crate::content_store::kept_facts::by_relation(&c.store()).unwrap().into_iter().map(|((rel, _), _)| rel).collect();
    let base: Vec<&&str> = BASE.iter().filter(|r| kept.contains(**r)).collect();
    assert!(base.is_empty(), "the chain keeps the base relations {base:?}");
}

/// R5: the runs pass in strict mode (`RERAIL_SQL_STRICT`): no SQL inside the waves (decision 0104), for the memo
/// hits' tapes too. The strict mode aborts a process that reads SQL in a wave, so each run is a process of its own,
/// as `a_rule_edit_over_known_files_writes_their_entries_with_no_sql_in_the_waves`' are.
#[test]
fn r5_the_runs_read_no_sql_inside_the_waves() {
    let c = Corpus::new("r5");
    let store = c.store();
    let run = |n: usize| {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "code_rule_dsl::logic::plan::corpus::tests::one_corpus_run_for_a_parent", "--ignored", "--test-threads", "1", "--nocapture"])
            .env("RERAIL_TAPE_CACHE", &store)
            .env("RERAIL_CHILD_RUN", c.dir.join("src"))
            .env("RERAIL_CHILD_OUT", c.dir.join(format!("out{n}")))
            .env("RERAIL_CHILD_PROGRAM", c.dir.join("program.rerail"))
            .env("RERAIL_COSTS", c.dir.join("costs.sqlite"))
            .env("RERAIL_IMAGE_STORE", c.dir.join("images"))
            .env("RERAIL_SQL_STRICT", "1");
        let output = child.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let fault: Vec<&str> = stderr.lines().skip_while(|l| !l.starts_with("rerail: RERAIL_SQL_STRICT")).take(200).collect();
        assert!(output.status.success(), "run {n}: {}; {}", output.status, fault.join(" "));
    };
    run(0);
    // Past the observer's racy window, so the next run knows the unchanged files by their stat (`RACY_NS`).
    std::thread::sleep(std::time::Duration::from_nanos(crate::content_store::RACY_NS as u64 + 100_000_000));
    c.write("a.py", "def f(x, y):\n    return y\n\ndef k():\n    pass\n");
    run(1);
    run(2);
}
