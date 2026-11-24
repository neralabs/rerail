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
    outputs: &'static [&'static str],
    runs: usize,
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        Corpus::with(name, PROGRAM, OUTPUTS)
    }

    /// A corpus of `FILES` under `program`, whose outputs are `outputs`.
    fn with(name: &str, program: &str, outputs: &'static [&'static str]) -> Corpus {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("cross-views-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = format!("# {name} {:?}\n", std::time::SystemTime::now());
        let rule = dir.join("program.rerail");
        std::fs::write(&rule, program).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let c = Corpus { dir, marker, program, outputs, runs: 0 };
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
        let rows = self.outputs.iter().map(|r| crate::code_rule_dsl::logic::plan::output::shown(&out, r).unwrap_or_default().into_iter().collect()).collect();
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
    for (i, rows) in rows.iter().enumerate() {
        assert!(!rows.is_empty(), "output {i} has no rows: the corpus does not exercise it");
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
    for (i, (file, body)) in edits.iter().enumerate() {
        c.write(file, body);
        let (_, rows) = c.run();
        if super::fresh_tier::compared(i, edits.len()) {
            assert_eq!(rows, c.fresh(), "after {file}'s edit, the outputs are a cold run's");
        }
    }
}

/// R4: no base relation's rows are kept, after a cold run and an edit's run. What 0458 keeps of one is its
/// documents' digest (its documents' entry: one row, no granules) and its statistics beside it. The memo writer's
/// guard fails a run whose entries keep a base relation's rows, so the runs here passing is the memo's half.
#[test]
fn r4_no_base_relation_is_kept() {
    let mut c = Corpus::new("r4");
    c.run();
    c.write("d.py", "from c import h\nfrom a import f\n\ndef i():\n    h()\n");
    c.run();
    let kept = crate::content_store::kept_facts::by_relation(&c.store()).unwrap();
    let rows: Vec<_> = kept.iter().filter(|((rel, part), (entries, rows, _))| BASE.contains(&rel.as_str()) && match *part {
        "rows" => rows > entries,
        "beside" => false,
        _ => true,
    }).collect();
    assert!(rows.is_empty(), "the chain keeps the base relations' rows {rows:?}");
}

/// With R4: a base relation's statistics are kept beside its documents' table, not counted from every row each run.
/// A cold run counts them, the run after it takes them as kept, and an edit's run patches them by the changed file's
/// old and new documents. A run inside another's waves (a phase's) cannot read its changed files' old documents there
/// and counts them again after an edit (0458's Open item); it says so.
#[test]
fn r4_a_base_relations_statistics_are_kept_and_patched_by_an_edit() {
    use crate::code_rule_dsl::logic::profile;
    let mut c = Corpus::new("r4-stats");
    // The decisions are the run's (decision 0442): the test holds the run they are read in, which each run joins.
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    profile::set(true);
    let mut inside = false;
    let mut how = |c: &mut Corpus| {
        // The run's scope, which the corpus run joins: its decisions are the run's (decision 0442).
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::take_decisions();
        c.run();
        let made = profile::take_decisions();
        inside = made.keys().any(|(t, x)| t == "the base relations' old documents (decision 0458)" && x.contains("listed inside the waves"));
        made.into_keys().filter(|(t, _)| t == "the base relations' statistics (decision 0458)").map(|(_, x)| x).collect::<BTreeSet<String>>()
    };
    // Each relation's statistics, as the run took them: all the same way, `how`. A semi-join's are its kept rows' at the
    // held nodes (`retain::semijoin_rows`), a decision of their own.
    let all = |made: &BTreeSet<String>, how: &str, held: &str| !made.is_empty() && made.iter().all(|x| x.ends_with(&format!(": {how}")) || x.ends_with(&format!(": {held} at the held nodes")));
    let cold = how(&mut c);
    let warm = how(&mut c);
    c.write("d.py", "from c import h\nfrom a import f\n\ndef i():\n    h()\n");
    let edit = how(&mut c);
    profile::set(false);
    // Counted, or a source's own from its structure (`doc`'s from its documents, `within`'s from the tree): no row made.
    assert!(cold_counted(&cold), "a cold run counts them: {cold:?}");
    assert!(all(&warm, "kept", "kept"), "the run after it takes them as kept: {warm:?}");
    // A semi-join's at its held nodes too, by the nodes the changed files' exports held and hold (`held_counted`).
    assert!(all(&edit, "patched by the changed documents", "patched by the changed documents") || inside, "an edit patches them, but inside another run's waves: {edit:?}");
}

/// A session (0077) holds its corpus batch from run to run, its changed files' documents replaced (decision 0458), and
/// keeps its base relations' statistics as the kept route does: its first run counts them and makes every document,
/// the run after it makes none and takes them as kept, and an edit makes the changed file's documents alone and patches
/// the statistics by its old and new ones. Its outputs are a fresh run's.
#[test]
fn a_session_holds_its_corpus_batch_and_keeps_its_base_statistics() {
    use crate::code_rule_dsl::logic::profile;
    let mut c = Corpus::new("session-batch");
    let handle = super::session::Handle::open(&c.dir.join("session")).unwrap();
    // The decisions are the run's (decision 0442): the test holds the run they are read in, which each run joins.
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    profile::set(true);
    let run = |c: &mut Corpus| {
        profile::take_decisions();
        let (_, rows) = {
            let _bound = handle.bind();
            c.run()
        };
        let made = profile::take_decisions();
        let of = |topic: &str| made.keys().filter(|(t, _)| t == topic).map(|(_, x)| x.clone()).collect::<BTreeSet<String>>();
        (of("the base relations' statistics (decision 0458)"), of("the corpus batch (decision 0458)"), rows)
    };
    let all = |made: &BTreeSet<String>, how: &str, held: &str| !made.is_empty() && made.iter().all(|x| x.ends_with(&format!(": {how}")) || x.ends_with(&format!(": {held} at the held nodes")));
    let (cold, cold_batch, _) = run(&mut c);
    let (warm, warm_batch, _) = run(&mut c);
    c.write("d.py", "from c import h\nfrom a import f\n\ndef i():\n    h()\n");
    let (edit, edit_batch, rows) = run(&mut c);
    profile::set(false);
    assert!(cold_counted(&cold), "a session's first run counts them: {cold:?}");
    assert_eq!(cold_batch, BTreeSet::from(["4 documents made, none held".to_owned()]), "and makes every document");
    assert!(all(&warm, "kept", "kept"), "the run after it takes them as kept: {warm:?}");
    assert_eq!(warm_batch, BTreeSet::from(["0 documents made, 4 held from the session's last run".to_owned()]), "and makes none");
    assert!(all(&edit, "patched by the changed documents", "patched by the changed documents"), "an edit patches them: {edit:?}");
    assert_eq!(edit_batch, BTreeSet::from(["1 documents made, 3 held from the session's last run".to_owned()]), "and makes the changed file's alone");
    every_output_has_rows(&rows);
    assert_eq!(rows, c.fresh(), "the session's outputs after the edit are a fresh run's");
}

/// `line` and `col` read only at the nodes `uses` holds, each alone in its rule (a join of two would be pushed into the
/// file-local part whole), and a span read (`src_line`), which makes `line` and `col` rows of the batch's documents
/// rather than views of the tape (`Extra::spans`), as the data layer's are; and `docof` joined in a maintained
/// stratum, whose plan asks its estimate by a column (`setfix`'s lead), so each is a semi-join input whose first column is the node,
/// not the document (the data layer's `line`, `col`, `op`, `visit_index`): its count at the held nodes was refused a
/// walk of the corpus batch by its documents, and read it whole, making every document (t43 step 21).
const AT_HELD: &str = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl at_line(f: node, l: number)
at_line(f, l) :- uses(_, f), line(f, l).
.decl at_col(f: node, c: number)
at_col(f, c) :- uses(_, f), col(f, c).
.decl lines(d: number, l: number)
lines(d, l) :- src_line(d, l, _).
.decl at_home(u: node, p: symbol)
at_home(u, p) :- uses(u, f), docof(f, d), doc(d, p, _).
.output at_line
.output at_col
.output lines
.output at_home
"#;
const AT_HELD_OUTPUTS: &[&str] = &["at_line", "at_col", "lines", "at_home"];

/// An edit pays for the changed documents, not the corpus (t43 step 21): a semi-join input's rows at the nodes the
/// exports hold (Q396) are kept beside the documents' table (`retain::semijoin_rows`), and an edit takes those of the
/// documents it did not change, walking only the changed documents' nodes; so no read of the corpus batch makes an
/// unchanged document. In a session and in a new process over the kept store alike.
#[test]
fn an_edit_takes_a_semi_join_inputs_kept_rows_and_makes_no_unchanged_document() {
    use crate::code_rule_dsl::logic::profile;
    // Every case's failures, said together.
    let mut failed: Vec<String> = Vec::new();
    for (program, outputs, session) in [(PROGRAM, OUTPUTS, false), (PROGRAM, OUTPUTS, true), (AT_HELD, AT_HELD_OUTPUTS, false), (AT_HELD, AT_HELD_OUTPUTS, true)] {
        let mut c = Corpus::with(&format!("semi-patch-{}-{}", outputs[0], session), program, outputs);
        let handle = session.then(|| super::session::Handle::open(&c.dir.join("session")).unwrap());
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        let run = |c: &mut Corpus| {
            let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
            profile::take_decisions();
            let (_, rows) = {
                let _bound = handle.as_ref().map(|h| h.bind());
                c.run()
            };
            let made = profile::take_decisions();
            let of = |topic: &str| made.keys().filter(|(t, _)| t == topic).map(|(_, x)| x.clone()).collect::<BTreeSet<String>>();
            let space = of("the corpus batch's row space (decision 0494)");
            let kept = of("cross-file part kept (decision 0077)");
            (of("a semi-join's rows at the held nodes"), of("a batch's every document made by a whole read of a relation (decision 0494)"), rows, space, kept)
        };
        let (cold, _, _, _, _) = run(&mut c);
        run(&mut c);
        c.write("d.py", "from c import h\nfrom a import f\n\ndef i():\n    h()\n");
        let (edit, whole, rows, space, kept) = run(&mut c);
        profile::set(false);
        let route = format!("{}, {}", if session { "a session" } else { "a new process" }, outputs[0]);
        // The edit maintains strata from deltas (a maintained join plans its lead by the inputs' estimates).
        if !kept.iter().any(|x| x.split(" maintained from deltas").next().and_then(|h| h.rsplit('(').next()).is_some_and(|n| n.trim() != "0")) {
            failed.push(format!("{route}: the edit maintains strata from deltas: {kept:?}"));
        }
        // The semi-join inputs: those the cold run counted at the held nodes.
        let held: BTreeSet<&str> = cold.iter().filter_map(|x| x.split(':').next()).collect();
        if held.is_empty() {
            failed.push(format!("{route}: the program has semi-join inputs: {cold:?}"));
        }
        // Each one's edit: some of its held nodes answered by its kept rows ("N of M nodes answered", N > 0).
        let answered = |x: &String| x.split_once(": ").and_then(|(_, t)| t.split(' ').next()?.parse::<usize>().ok()).unwrap_or(0);
        for rel in &held {
            if !edit.iter().any(|x| x.split(':').next() == Some(*rel) && answered(x) > 0) {
                failed.push(format!("{route}: an edit takes {rel}'s kept rows at the held nodes: {edit:?}"));
            }
        }
        // A whole read makes at most the changed file's document (1 of the batch's).
        let unmade = |x: &String| x.rsplit_once(": ").and_then(|(_, t)| t.split(' ').next()?.parse::<usize>().ok()).unwrap_or(0);
        // Every base relation's rows counted by its kept statistics, a semi-join's too: a `len` (a plan's estimate)
        // made every document of a relation without its count.
        let all_counted = |x: &String| x.split(" base relations' rows").next().and_then(|n| n.split_once(" of ")).is_some_and(|(a, b)| a == b);
        if !space.iter().filter(|x| x.contains("base relations' rows")).all(all_counted) {
            failed.push(format!("{route}: the corpus batch has every base relation's rows by its kept statistics: {space:?}"));
        }
        // And every unchanged document's views' counts (3 of the 4, d.py's changed): without them a read of a view
        // by its documents (a version's, `rows_of_documents`) projected every document to count its rows.
        let views = |x: &String| x.split(" documents' views' counts kept").next().and_then(|n| n.split_once(" of ")).map(|(a, b)| (a.parse::<usize>().unwrap_or(0), b.parse::<usize>().unwrap_or(0)));
        if !space.iter().filter_map(views).all(|(a, b)| a + 1 >= b) {
            failed.push(format!("{route}: the corpus batch has its unchanged documents' views' counts: {space:?}"));
        }
        if !whole.iter().all(|x| unmade(x) <= 1) {
            failed.push(format!("{route}: no read makes an unchanged document: {whole:?}"));
        }
        every_output_has_rows(&rows);
        if rows != c.fresh() {
            failed.push(format!("{route}: the outputs after the edit are a fresh run's"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// R5: the runs pass in strict mode (`RERAIL_SQL_STRICT`): no SQL inside the waves (decision 0104), for the memo
/// hits' tapes too. The strict mode aborts a process that reads SQL in a wave, so each run is a process of its own,
/// as `a_rule_edit_over_known_files_writes_their_entries_with_no_sql_in_the_waves`' are.
#[test]
fn r5_the_runs_read_no_sql_inside_the_waves() {
    let c = Corpus::new("r5");
    let store = c.store();
    // Begun from the template (`test_store::begun`): its child processes would each fill an empty one first.
    super::test_store::begun(&store);
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
            .env("RERAIL_SQL_STRICT", "1")
            .env("RERAIL_RACY_MS", super::test_store::TEST_RACY_MS);
        let output = child.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let fault: Vec<&str> = stderr.lines().skip_while(|l| !l.starts_with("rerail: RERAIL_SQL_STRICT")).take(200).collect();
        assert!(output.status.success(), "run {n}: {}; {}", output.status, fault.join(" "));
    };
    run(0);
    // Past the observer's racy window, so the next run knows the unchanged files by their stat (`TEST_RACY_MS`).
    super::test_store::past_test_racy_window();
    c.write("a.py", "def f(x, y):\n    return y\n\ndef k():\n    pass\n");
    run(1);
    run(2);
}

/// A semi-join's distinct values, as its estimates take them (`held_distinct`): its node column as many as its every
/// row, a node's row one; its other columns as its held nodes have them. Scaled to every row as the node's are, a
/// document column estimated one row a document, and a maintained join probed `docof` by its document, making every
/// one of the data layer's 518 (t43 step 21).
#[test]
fn a_semi_joins_estimates_scale_its_node_column_alone() {
    let stat = |c: &str, distinct: i64| (c.to_owned(), 0, distinct, 0, 0);
    // docof(n, d) at 1,000 held nodes of 300 documents; 50,000 rows in all.
    assert_eq!(super::held_distinct("docof", &[stat("n", 1_000), stat("d", 300)], 50_000), vec![Some(50_000), Some(300)]);
    // node(d, n, kind, start, end_): its node is its second column.
    let node = super::held_distinct("node", &[stat("d", 300), stat("n", 1_000), stat("kind", 40), stat("start", 900), stat("end_", 900)], 50_000);
    assert_eq!(node, vec![Some(300), Some(50_000), Some(40), Some(900), Some(900)]);
}

/// The fixture's program, and a string annotation's tree read across files (a guest, decision 0055; the data
/// layer's `parsed` annotations): `ann_kids` reads `child` at a guest's root, of the files a file imports from, and
/// `below` walks a guest's tree in the cross part, so `child` is read at nodes it derives, a base relation whole.
const GUESTS: &str = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(n: node, q: symbol)
imp(n, @dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl uses(n: node, f: node)
uses(n, f) :- imp(n, q), def(f, q).
.decl shape(f: node, k: symbol)
shape(f, k) :- uses(_, f), child(f, b, _), kind(b, k).
.decl ann(d: number, r: node)
ann(d, r) :- string_annotation(s), parsed(s, "annotation", r), docof(s, d).
.decl ann_kids(u: node, k: symbol)
ann_kids(u, k) :- uses(u, f), docof(f, d), ann(d, r), child(r, c, _), kind(c, k).
.decl below(u: node, c: node)
below(u, c) :- uses(u, f), docof(f, d), ann(d, r), child(r, c, _).
below(u, c) :- below(u, b), child(b, c, _).
.decl deep(u: node, k: symbol)
deep(u, k) :- below(u, c), kind(c, k).
.output shape
.output ann_kids
.output deep
"#;
const GUESTS_OUTPUTS: &[&str] = &["shape", "ann_kids", "deep"];

/// An edit of a file with guests (decision 0055: an f-string's tree, a string annotation's) patches every base
/// relation's statistics by the changed documents, as a file without them does: a guest's rows are its host's
/// document's. At the data layer's edits `child` was counted again off the corpus batch, its patched rows not the
/// relation's (t43 step 21). In a session and in a new process.
#[test]
fn an_edit_of_a_file_with_guests_patches_every_base_relations_statistics() {
    use crate::code_rule_dsl::logic::profile;
    let mut failed: Vec<String> = Vec::new();
    for session in [false, true] {
        let mut c = Corpus::with(if session { "guests-session" } else { "guests-kept" }, GUESTS, GUESTS_OUTPUTS);
        // Annotated definitions imported by another file: their guests are read across files.
        c.write("a.py", "def f(x: \"List[int]\"):\n    return x\n\ndef k(y: \"Dict[str, int]\"):\n    pass\n");
        c.write("d.py", "from c import h\nfrom a import k\n\ndef i(a: \"A\"):\n    h()\n");
        let handle = session.then(|| super::session::Handle::open(&c.dir.join("session")).unwrap());
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        let run = |c: &mut Corpus| {
            let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
            profile::take_decisions();
            let (_, rows) = {
                let _bound = handle.as_ref().map(|h| h.bind());
                c.run()
            };
            let made = profile::take_decisions();
            let of = |topic: &str| made.keys().filter(|(t, _)| t == topic).map(|(_, x)| x.clone()).collect::<BTreeSet<String>>();
            (of("the base relations' statistics (decision 0458)"), rows, of("the corpus batch (decision 0458)"))
        };
        let (_, _, batch) = run(&mut c);
        // The files' guests are documents of the batch: more of them than files.
        let made = |x: &String| x.split(' ').next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
        if !batch.iter().any(|x| made(x) > FILES.len()) {
            failed.push(format!("{}: the corpus batch holds the files' guests: {batch:?}", if session { "a session" } else { "a new process" }));
        }
        run(&mut c);
        // Two edits of the same file, each with annotations new to it: the second's old documents are the first's,
        // its guests among them.
        c.write("a.py", "def f(x: \"Tuple[int, str]\", z: \"B\"):\n    return x\n\ndef k(y: \"Dict[str, int]\"):\n    pass\n");
        let (first, _, _) = run(&mut c);
        c.write("a.py", "def f(x: \"Set[bytes]\", z: \"Optional[B]\"):\n    return x\n\ndef k(y: \"Dict[str, float]\", w: \"C\"):\n    pass\n");
        let (edit, rows, _) = run(&mut c);
        let edit: BTreeSet<String> = edit.into_iter().chain(first).collect();
        profile::set(false);
        let route = if session { "a session" } else { "a new process" };
        if edit.is_empty() || !edit.iter().all(|x| x.contains(": patched by the changed documents")) {
            failed.push(format!("{route}: an edit patches every base relation's statistics: {edit:?}"));
        }
        every_output_has_rows(&rows);
        if rows != c.fresh() {
            failed.push(format!("{route}: the outputs after the edit are a fresh run's"));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// A session's corpus batch is renewed from its last run's (decision 0616, R5): an edit takes the held groups whose
/// documents did not change, and its outputs are a fresh run's, at each of three edits, of a program without guests
/// and one whose string annotations' guests are read across files (their producer's knowledge carried over).
#[test]
fn a_sessions_edit_takes_its_unchanged_groups_from_the_last_run() {
    use crate::code_rule_dsl::logic::profile;
    let mut failed: Vec<String> = Vec::new();
    for (program, outputs) in [(PROGRAM, OUTPUTS), (GUESTS, GUESTS_OUTPUTS)] {
        let mut c = Corpus::with(&format!("renewed-{}", outputs[0]), program, outputs);
        if program == GUESTS {
            c.write("a.py", "def f(x: \"List[int]\"):\n    return x\n\ndef k(y: \"Dict[str, int]\"):\n    pass\n");
            c.write("d.py", "from c import h\nfrom a import k\n\ndef i(a: \"A\"):\n    h()\n");
        }
        let handle = super::session::Handle::open(&c.dir.join("session")).unwrap();
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        let run = |c: &mut Corpus| {
            let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
            profile::take_decisions();
            let (_, rows) = {
                let _bound = handle.bind();
                c.run()
            };
            let made = profile::take_decisions();
            let of = |topic: &str| made.keys().filter(|(t, _)| t == topic).map(|(_, x)| x.clone()).collect::<BTreeSet<String>>();
            (of("a session's corpus batch renewed (decision 0616)"), rows)
        };
        run(&mut c);
        run(&mut c);
        let taken = |x: &String| x.split(' ').next().and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
        let edits: [(&str, String); 3] = [
            ("b.py", "from a import f\n\ndef g(y, z):\n    return f(z)\n".into()),
            ("c.py", "from a import f\nfrom b import g\n\ndef h():\n    g(1, 2)\n".into()),
            ("a.py", if program == GUESTS { "def f(x: \"Set[bytes]\"):\n    return x\n\ndef k(y: \"Dict[str, float]\"):\n    pass\n".into() } else { "def f(x):\n    return [x]\n\ndef k():\n    pass\n".into() }),
        ];
        let mut any = false;
        for (path, text) in edits {
            c.write(path, &text);
            let (renewed, rows) = run(&mut c);
            let route = format!("{}, edit of {path}", outputs[0]);
            if renewed.is_empty() {
                failed.push(format!("{route}: the corpus batch is renewed from the last run's"));
            }
            any |= renewed.iter().any(|x| taken(x) > 0);
            if rows != c.fresh() {
                failed.push(format!("{route}: the outputs are a fresh run's"));
            }
        }
        profile::set(false);
        if !any {
            failed.push(format!("{}: an edit takes the held groups of unchanged documents", outputs[0]));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// A cold run's statistics: each counted (at the held nodes, for a semi-join's), or the source's own from its
/// structure, which makes no row (`Source::statistics`).
fn cold_counted(made: &BTreeSet<String>) -> bool {
    let own = ": the source's own, from its structure (no row made)";
    // Counted by the chunks where their rows lay (decision 0458's statistics), or off the batch.
    let how = [": counted", ": counted at the held nodes", ": counted by the chunks, where their rows lay", ": counted by the chunks at their held nodes", own];
    !made.is_empty() && made.iter().all(|x| how.iter().any(|h| x.ends_with(h)))
}
