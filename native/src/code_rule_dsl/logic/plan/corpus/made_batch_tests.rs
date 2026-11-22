//! Decision 0494's requirements over whole runs, one test each (R1–R3 are `facts::vtab::syntax`'s, over batches):
//! a batch makes a document when first read. A requirement is not dropped or weakened here without the user.

use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Cross-file: `shared` joins two files' identifiers on their text, `spelled` reads each one's `text` by the node it
/// binds, and `named` tests the text, so the cross part reads `docof` and `text` of both files' identifiers and the
/// text test (0037), each bound by a cross-file relation. (A join of the base relations themselves by the text, each
/// file's part with its test, is pushed into the chunks: the cross part would read none,
/// `typing::file_local::tests::a_cross_file_join_by_a_value_pushes_each_files_part`; so the relations are outputs,
/// not inlined into their readers.)
const PROGRAM: &str = r#"
.decl ident(a: node, t: symbol)
ident(a, t) :- kind(a, "identifier"), text(a, t).
.decl shared(a: node)
shared(a) :- ident(a, t), docof(a, d), ident(b, t), docof(b, e), d < e.
.decl spelled(t: symbol)
spelled(t) :- shared(a), text(a, t).
.decl named(t: symbol)
named(t) :- spelled(t), contains("o", t).
.output ident, shared, spelled, named
"#;

const FILES: &[(&str, &str)] = &[
    ("a.py", "def foo(x):\n    return x\n\ndef bar():\n    pass\n"),
    ("b.py", "from a import foo\n\ndef go(y):\n    return foo(y)\n"),
    ("c.py", "from b import go\n\ndef h():\n    pass\n"),
];

struct Corpus {
    dir: PathBuf,
    marker: String,
    program: String,
    /// The output a run's rows are of (`run_over`).
    output: &'static str,
    runs: usize,
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("made-batch-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = format!("# {name} {:?}\n", std::time::SystemTime::now());
        let rule = dir.join("program.rerail");
        std::fs::write(&rule, PROGRAM).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let c = Corpus { dir, marker, program, output: "named", runs: 0 };
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

    /// A run over the store `store`: its report and its output's rows (`output`).
    fn run_over(&mut self, store: &str) -> (super::Report, BTreeSet<Vec<String>>) {
        self.runs += 1;
        let out = self.dir.join(format!("out{}", self.runs));
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join(store)));
        let report = super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        (report, crate::code_rule_dsl::logic::plan::output::shown(&out, self.output).unwrap_or_default().into_iter().collect())
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// R4: a text test is a test of a string. After an edit the cross part gives it no version (nothing of a test changes
/// with an edit), and its results are a cold run's over the edited tree.
#[test]
fn r4_a_text_test_has_no_version_after_an_edit() {
    use crate::code_rule_dsl::logic::profile;
    let mut c = Corpus::new("r4");
    let (_, cold) = c.run_over("kept.sqlite");
    assert!(!cold.is_empty(), "the corpus exercises the test: {cold:?}");
    let edits: &[(&str, &str)] = &[
        ("a.py", "def foo(x, y):\n    return x\n\ndef bar():\n    pass\n\ndef solo():\n    pass\n"),
        ("b.py", "from a import foo\nfrom a import solo\nfrom a import bar\n\ndef go(y):\n    return foo(y)\n"),
        ("a.py", "def bar():\n    pass\n\ndef solo():\n    pass\n"),
    ];
    let mut unversioned = BTreeSet::new();
    for (i, (file, body)) in edits.iter().enumerate() {
        c.write(file, body);
        // The run resets the profile as it starts; the test's scope keeps its records after (decision 0442).
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        profile::take_decisions();
        let (_, rows) = c.run_over("kept.sqlite");
        let made = profile::take_decisions();
        profile::set(false);
        let versioned: BTreeSet<String> = made.keys().filter(|(t, _)| t == "the base relations' versions (decision 0458)").map(|(_, x)| x.clone()).collect();
        unversioned.extend(versioned.into_iter().filter(|x| x.ends_with("a text test, no version (decision 0494)")));
        let (_, fresh) = c.run_over(&format!("fresh{i}.sqlite"));
        assert_eq!(rows, fresh, "edit {i} ({file}): the outputs are a cold run's");
    }
    assert!(!unversioned.is_empty(), "the cross part reads the text test, given no version");
}

/// R5: the documents' table keeps each document's views' counts. A cold run counts and keeps them; after a one-file
/// edit the corpus batch is given the kept counts of every document but the changed one, and its base relations'
/// rows by their kept statistics; the outputs are a cold run's. Over R6's program (`SHAPES`), whose cross part reads
/// base relations whole (`field`, `within`), not only as lookups of the nodes its exports hold (a semi-join's, whose
/// statistics are kept apart).
#[test]
fn r5_the_documents_counts_are_kept_and_given_to_the_corpus_batch() {
    use crate::code_rule_dsl::logic::profile;
    let mut c = Corpus::new("r5");
    let rule = c.dir.join("shapes.rerail");
    std::fs::write(&rule, SHAPES).unwrap();
    c.program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    c.output = "twins";
    c.write("c.py", "def foo(z):\n    return z\n\ndef h():\n    pass\n");
    let row_space = |c: &mut Corpus, store: &str| {
        // The run resets the profile as it starts; the test's scope keeps its records after (decision 0442).
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        profile::take_decisions();
        let (_, rows) = c.run_over(store);
        let made = profile::take_decisions();
        profile::set(false);
        let said: BTreeSet<String> = made.into_keys().filter(|(t, _)| t == "the corpus batch's row space (decision 0494)").map(|(_, x)| x).collect();
        (rows, said)
    };
    let (cold_rows, cold) = row_space(&mut c, "kept.sqlite");
    assert!(!cold_rows.is_empty(), "the corpus exercises the joins");
    assert!(cold.contains(&format!("0 of {} documents' views' counts kept", FILES.len())), "a cold run keeps none yet: {cold:?}");
    c.write("b.py", "from a import foo\nfrom a import bar\n\ndef go(y):\n    return foo(y)\n");
    let (rows, edit) = row_space(&mut c, "kept.sqlite");
    assert!(edit.contains(&format!("{} of {} documents' views' counts kept", FILES.len() - 1, FILES.len())), "all but the changed file's: {edit:?}");
    assert!(edit.iter().any(|x| x.ends_with("base relations' rows counted by their kept statistics") && !x.starts_with("0 of")), "the relations' rows by their kept statistics: {edit:?}");
    let (fresh, _) = row_space(&mut c, "fresh.sqlite");
    assert_eq!(rows, fresh, "the edit's outputs are a cold run's");
}

/// R6's program: cross-file pairs of two files' functions of one name, and of each pair the views (`child`, `field`)
/// and the containment axis (`within`) of its functions' nodes, read by the nodes the pair binds, so the cross part
/// reads them all as base relations. (A file's own part of a join, its nodes narrowed, is pushed into the chunks;
/// `pair` is an output, so it is not inlined into `twins`, whose file's parts would then be pushed.)
const SHAPES: &str = r#"
.decl named_in(f: node, t: symbol)
named_in(f, t) :- kind(f, "function_definition"), field(f, "name", c), text(c, t).
.decl pair(f: node, g: node, t: symbol)
pair(f, g, t) :- named_in(f, t), docof(f, d), named_in(g, t), docof(g, e), d < e.
.decl twins(t: symbol, b: node, x: node)
twins(t, b, x) :- pair(f, g, t), child(f, b, _), field(g, "body", y), within(x, y).
.output pair, twins
"#;

/// R6: after an edit, no base relation's changed rows are read through: each is found where the batch knows it lies
/// (by group, by offset, by its nodes' lookup), and the outputs are a cold run's.
#[test]
fn r6_an_edit_reads_no_base_relation_through() {
    use crate::code_rule_dsl::logic::profile;
    let mut c = Corpus::new("r6-through");
    let rule = c.dir.join("shapes.rerail");
    std::fs::write(&rule, SHAPES).unwrap();
    c.program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    c.write("c.py", "def foo(z):\n    return z\n\ndef h():\n    pass\n");
    let shown = |c: &mut Corpus, store: &str| {
        c.runs += 1;
        let out = c.dir.join(format!("out{}", c.runs));
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(c.dir.join(store)));
        super::run(&c.program, &c.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        crate::code_rule_dsl::logic::plan::output::shown(&out, "twins").unwrap_or_default().into_iter().collect::<BTreeSet<Vec<String>>>()
    };
    let cold = shown(&mut c, "kept.sqlite");
    assert!(!cold.is_empty(), "the corpus exercises the joins");
    c.write("a.py", "def foo(x, y):\n    w = x\n    return w\n\ndef bar():\n    pass\n");
    // The run resets the profile as it starts; the test's scope keeps its records after (decision 0442).
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    profile::set(true);
    profile::take_decisions();
    let rows = shown(&mut c, "kept.sqlite");
    let made = profile::take_decisions();
    profile::set(false);
    let versions: BTreeSet<String> = made.keys().filter(|(t, _)| t == "the base relations' versions (decision 0458)").map(|(_, x)| x.clone()).collect();
    assert!(!versions.iter().any(|x| x.contains("read through")), "a relation read through: {versions:?}");
    let stats: BTreeSet<String> = made.keys().filter(|(t, _)| t == "the base relations' statistics (decision 0458)").map(|(_, x)| x.clone()).collect();
    for rel in ["child", "field", "within"] {
        assert!(stats.iter().any(|x| x.starts_with(&format!("{rel}: "))), "the cross part reads {rel}: {stats:?}");
    }
    assert_eq!(rows, shown(&mut c, "fresh.sqlite"), "the edit's outputs are a cold run's");
}

/// R6: after an edit, each base relation's changed rows are found by group (the changed documents made as one, whose
/// rows are theirs): none is read through, and no kept statistic is counted again; the outputs are a cold run's.
#[test]
fn r6_an_edit_counts_no_kept_statistic_again() {
    use crate::code_rule_dsl::logic::profile;
    let mut c = Corpus::new("r6");
    c.run_over("kept.sqlite");
    let edits: &[(&str, &str)] = &[
        ("b.py", "from a import foo\nfrom a import bar\n\ndef go(y):\n    return foo(y)\n"),
        ("a.py", "def foo(x, y):\n    return x\n\ndef bar():\n    pass\n\ndef solo():\n    pass\n"),
    ];
    for (i, (file, body)) in edits.iter().enumerate() {
        c.write(file, body);
        // The run resets the profile as it starts; the test's scope keeps its records after (decision 0442).
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        profile::take_decisions();
        let (_, rows) = c.run_over("kept.sqlite");
        let made = profile::take_decisions();
        profile::set(false);
        let said = |title: &str| -> BTreeSet<String> { made.keys().filter(|(t, _)| t == title).map(|(_, x)| x.clone()).collect() };
        let versions = said("the base relations' versions (decision 0458)");
        assert!(!versions.iter().any(|x| x.contains("read through")), "edit {i}: a relation read through: {versions:?}");
        let stats = said("the base relations' statistics (decision 0458)");
        assert!(!stats.is_empty(), "edit {i}: the cross part reads base relations");
        let old = said("the base relations' old documents (decision 0458)");
        assert!(!stats.iter().any(|x| x.ends_with(": counted")), "edit {i}: a kept statistic counted again: {stats:?}; old documents: {old:?}");
        let (_, fresh) = c.run_over(&format!("fresh{i}.sqlite"));
        assert_eq!(rows, fresh, "edit {i} ({file}): the outputs are a cold run's");
    }
}

/// R7: a run after an edit with nothing changed since is a warm run: it builds, parses and makes no document. The
/// edit keeps its changed document's views' counts (an edit's batch is given the others', and kept none of its own,
/// so every later run built the changed file from its tape again), and a base relation the batch does not count
/// (`within`, an axis no solve answers) has its rows by its kept statistics, not its `len`, which made every node's
/// rows (Q332: ep2's run after its two file edits cost 32 G instructions against a warm run's 15). Over R6's
/// program (`SHAPES`), whose cross part reads `within` whole.
#[test]
fn r7_a_run_after_an_edit_makes_no_document() {
    use crate::code_rule_dsl::logic::profile;
    let mut c = Corpus::new("r7");
    let rule = c.dir.join("shapes.rerail");
    std::fs::write(&rule, SHAPES).unwrap();
    c.program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    c.output = "twins";
    c.write("c.py", "def foo(z):\n    return z\n\ndef h():\n    pass\n");
    let (_, cold) = c.run_over("kept.sqlite");
    assert!(!cold.is_empty(), "the corpus exercises the joins");
    c.write("a.py", "def foo(x, y):\n    w = x\n    return w\n\ndef bar():\n    pass\n");
    c.run_over("kept.sqlite");
    // The run resets the profile as it starts; the test's scope keeps its records after (decision 0442).
    let (report, rows, said) = {
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        profile::take_decisions();
        let (report, rows) = c.run_over("kept.sqlite");
        let made = profile::take_decisions();
        profile::set(false);
        let said: BTreeSet<String> = made.into_keys().filter(|(t, _)| t == "the corpus batch's row space (decision 0494)").map(|(_, x)| x).collect();
        (report, rows, said)
    };
    let docs = report.corpus_docs;
    assert!(said.contains(&format!("{n} of {n} documents' views' counts kept", n = FILES.len())), "the edit kept its changed document's counts: {said:?}");
    assert_eq!((docs.from_tape, docs.parsed), (0, 0), "Q332: no document is built after an edit with nothing changed since: {docs:?}");
    assert_eq!(docs.made, 0, "Q332: no document's rows are made after an edit with nothing changed since: {docs:?}");
    let (_, fresh) = c.run_over("fresh.sqlite");
    assert_eq!(rows, fresh, "the outputs are a cold run's");
}

/// A removal join that reads `docof` as it was, its node bound, makes no document: the node's document is its id's
/// (decision 0083 §3), so `d` is computed from the node, never probed in the old version, which made each node's
/// document (ep2's insert: a removal join over `future_annotations`'s gone row made 121 documents, 0.74 s). Here
/// `called` is cross-file (an identifier naming a function any file defines; an output, so not inlined into its
/// reader, decision 0099), so `late` is too, and an edit takes
/// a.py's `__future__` import away: its `lead` row goes, and the removal join walks `called`, every file's, through
/// `docof`. The walk computes each node's document (the decision says so), and the edit makes a.py's document alone:
/// probed by node, `docof` made b.py's and c.py's too, one built again from its tape (`Report::corpus_docs`). The
/// edit's rows are a cold run's.
#[test]
fn a_removal_join_computes_docof_from_the_node() {
    const LATE: &str = r#"
.decl lead(d: number)
lead(d) :- kind(i, "future_import_statement"), docof(i, d).
.decl defined(t: symbol)
defined(t) :- kind(f, "function_definition"), field(f, "name", x), text(x, t).
.decl called(n: node)
called(n) :- kind(c, "call"), field(c, "function", n), text(n, t), defined(t).
.decl late(n: node)
late(n) :- called(n), docof(n, d), lead(d).
.output called, late
"#;
    let mut c = Corpus::new("docof-removal");
    let rule = c.dir.join("late.rerail");
    std::fs::write(&rule, LATE).unwrap();
    c.program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
    c.output = "late";
    // Many nodes in each file and few calls: the removal join walks `called` and tests each node's document, as
    // ep2's did, rather than walk a.py's nodes.
    let pad: String = (0..150).map(|i| format!("v{i} = [{i}, {i} + 1, ({i}, {i})]\n")).collect();
    c.write("a.py", &format!("from __future__ import annotations\nfrom b import g\n{pad}\ndef f(x):\n    return g(x)\n"));
    c.write("b.py", &format!("from __future__ import annotations\n{pad}\ndef g(y):\n    return y\n"));
    c.write("c.py", &format!("from __future__ import annotations\nfrom b import g\n{pad}\ndef h(z):\n    return g(g(z))\n"));
    let (_, cold) = c.run_over("kept.sqlite");
    let in_a = |rows: &BTreeSet<Vec<String>>| rows.iter().filter(|r| r.iter().any(|x| x.contains("a.py"))).count();
    assert!(in_a(&cold) > 0 && cold.len() > in_a(&cold), "calls in a.py and c.py: {cold:?}");
    c.write("a.py", &format!("from b import g\n{pad}\ndef f(x):\n    return g(x)\n"));
    // The run resets the profile as it starts; the test's scope keeps its records after (decision 0442).
    let (report, rows, probed) = {
        use crate::code_rule_dsl::logic::profile;
        let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
        profile::set(true);
        profile::take_decisions();
        let (report, rows) = c.run_over("kept.sqlite");
        let made = profile::take_decisions();
        profile::set(false);
        (report, rows, made.into_keys().collect::<BTreeSet<(String, String)>>())
    };
    assert_eq!((in_a(&rows), rows.len()), (0, cold.len() - in_a(&cold)), "a.py's calls are no longer late: {rows:?}");
    // The effect: each of the edit's batches (the main run's and its phases') makes a.py's document alone. Probing
    // `docof` by node instead made b.py's and c.py's documents too, one built again from its tape, to tell each
    // `called` node's document.
    let docs: Vec<super::CorpusDocs> = std::iter::once(report.corpus_docs).chain(report.phase_runs.iter().map(|p| p.report.corpus_docs)).collect();
    assert!(docs.iter().all(|d| d.made <= 1 && d.from_tape + d.parsed == 0), "the edit makes only the changed file's document: {docs:?}");
    let computed = |t: &str| t == "a removal join's `docof` as it was, its node bound: the node's document, from its id (decision 0083 §3)";
    assert!(probed.iter().any(|(t, x)| computed(t) && x.ends_with("late")), "the removal join computes each `called` node's document: {probed:?}");
    assert!(!probed.contains(&("a batch's documents made by a probe (decision 0494)".to_owned(), "docof column 0".to_owned())), "no probe of `docof` by its node makes a document: {probed:?}");
    let (_, fresh) = c.run_over("fresh.sqlite");
    assert_eq!(rows, fresh, "the outputs are a cold run's");
}
