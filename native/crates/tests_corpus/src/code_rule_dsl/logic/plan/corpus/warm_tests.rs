//! Warm runs (Q332): what a run over the store a cold run kept does to the files that did not change. It parses none,
//! builds no document of one, and makes no row of one (0104, decision 0458's corpus batch, 0494). Each test is a few
//! files, a cold run and the warm one, and checks the counts the run reports (`Report::corpus_docs`, `file_tapes`):
//! a change that makes a warm run redo a cold run's work fails here, by name, in seconds.
//!
//! The warm tier, these with the warm-run tests elsewhere (`.config/nextest.toml`'s `warm` profile):
//!
//! ```text
//! cargo nextest run -p rerail-tests-corpus --lib --profile warm
//! ```

use super::CorpusDocs;
use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Cross-file: identifiers of two files joined on their text, so the cross part reads its base relations (`kind`,
/// `text`, `docof`) off the corpus batch, every file's documents.
const PROGRAM: &str = r#"
.decl named(t: symbol)
named(t) :- kind(a, "identifier"), text(a, t), docof(a, d), kind(b, "identifier"), text(b, t), docof(b, e), d < e, contains("o", t).
.output named
"#;

const FILES: &[(&str, &str)] = &[
    ("a.py", "def foo(x):\n    return x\n\ndef bar():\n    pass\n"),
    ("b.py", "from a import foo\n\ndef go(y):\n    return foo(y)\n"),
    ("c.py", "from b import go\n\ndef h():\n    pass\n"),
    ("d.py", "def solo(z):\n    return z\n"),
];

/// The edit: one file of `FILES`, a name added that another file holds.
const EDIT: (&str, &str) = ("c.py", "from b import go\n\ndef h():\n    return go(1)\n\ndef foo():\n    pass\n");

struct Corpus {
    dir: PathBuf,
    marker: String,
    program: String,
    /// The relation whose rows a run reads back (`Ran::rows`).
    output: &'static str,
    runs: usize,
}

/// What a run reported, and `named`'s rows.
#[derive(Debug)]
struct Ran {
    /// The corpus batches' documents: the run's and every phase's it ran (phase 1, `sources.rerail`, holds every
    /// source file's document too), summed.
    corpus: CorpusDocs,
    /// The run's own, no phase's.
    main: CorpusDocs,
    /// Each phase's own, by name.
    phases: Vec<(String, CorpusDocs)>,
    /// Every parse the run made, wherever (`core::parses`), by the path that made it and its language.
    parsed_by: std::collections::BTreeMap<(&'static str, String), usize>,
    /// The cross-file strata it evaluated, of all (`Report::cross_kept`).
    cross_kept: Option<(usize, usize)>,
    /// The files built in the waves with a kept tape, and those parsed.
    waves: (usize, usize),
    rows: BTreeSet<Vec<String>>,
    /// Phase 1's frame (`phases::frame`): taken (`Some(true)`), made by its run (`Some(false)`), or none.
    frame: Option<bool>,
}

impl Ran {
    /// The corpus's files it parsed: they are Python; a run parses its rule files (`.rerail`) apart, once a process.
    fn parsed(&self) -> usize {
        self.parsed_by.iter().filter(|((_, language), _)| language == "python").map(|(_, n)| n).sum()
    }
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        Corpus::of(name, PROGRAM, FILES, "named")
    }

    /// `files` under `program`, whose `output` a run reads back.
    fn of(name: &str, program: &str, files: &[(&str, &str)], output: &'static str) -> Corpus {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("warm-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = format!("# {name} {:?}\n", std::time::SystemTime::now());
        let rule = dir.join("program.rerail");
        std::fs::write(&rule, program).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap()).unwrap();
        let c = Corpus { dir, marker, program, output, runs: 0 };
        for (file, body) in files {
            c.write(file, body);
        }
        c
    }

    fn write(&self, file: &str, body: &str) {
        let path = self.dir.join("src").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{}{body}", self.marker)).unwrap();
    }

    /// A run over the store `store`, kept from run to run: what a new process does.
    fn run_over(&mut self, store: &str) -> Ran {
        self.runs += 1;
        let out = self.dir.join(format!("out{}", self.runs));
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join(store)));
        let report = super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let rows = crate::code_rule_dsl::logic::plan::output::shown(&out, self.output).unwrap_or_default().into_iter().collect();
        let phases: Vec<(String, CorpusDocs)> = report.phase_runs.iter().map(|p| (p.name.clone(), p.report.corpus_docs)).collect();
        let corpus = phases.iter().fold(report.corpus_docs, |a, (_, p)| CorpusDocs {
            from_tape: a.from_tape + p.from_tape,
            parsed: a.parsed + p.parsed,
            held: a.held + p.held,
            made: a.made + p.made,
            documents: a.documents + p.documents,
        });
        let frame = report.phase_runs.iter().find(|p| p.name == "sources").and_then(|p| p.frame.as_ref()).map(|(_, made)| made.is_none());
        Ran { corpus, main: report.corpus_docs, phases, parsed_by: report.parsed, cross_kept: report.cross_kept, waves: report.file_tapes, rows, frame }
    }

    /// A cold run, then the warm one over the same store after `edit` (none: the files as they were), and a cold run
    /// over the files as they are then, from an empty store: R10's truth.
    fn cold_then_warm(name: &str, edit: Option<(&str, &str)>) -> (Ran, Ran, BTreeSet<Vec<String>>) {
        Corpus::new(name).then_warm(FILES.len(), edit)
    }

    /// This corpus's cold run, of at least `documents` documents, then the warm one after `edit`, and a fresh cold
    /// run's rows (`cold_then_warm`).
    fn then_warm(mut self, documents: usize, edit: Option<(&str, &str)>) -> (Ran, Ran, BTreeSet<Vec<String>>) {
        let c = &mut self;
        let cold = c.run_over("kept.sqlite");
        assert!(cold.corpus.documents >= documents, "the cold run's corpus batch holds every file's document: {cold:?}");
        if let Some((file, body)) = edit {
            c.write(file, body);
        }
        let warm = c.run_over("kept.sqlite");
        let fresh = c.run_over("fresh.sqlite").rows;
        (cold, warm, fresh)
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A warm run over files none of which changed parses none, builds no document and makes no row of one, and its
/// outputs are a cold run's.
#[test]
fn a_warm_run_over_unchanged_files_parses_builds_and_makes_nothing() {
    let (_, warm, fresh) = Corpus::cold_then_warm("unchanged", None);
    assert_eq!(warm.rows, fresh, "the warm run's outputs are a cold run's");
    assert_eq!(warm.parsed(), 0, "no file is parsed: {} parsed (warm run: {warm:?})", warm.parsed());
    assert_eq!(warm.waves, (0, 0), "no file is built in the waves (found, parsed): every one is a memo hit (warm run: {warm:?})");
    assert_eq!(warm.cross_kept.map(|(ran, _)| ran), Some(0), "every cross-file stratum is kept, none evaluated (warm run: {warm:?})");
    assert_eq!(warm.corpus.parsed, 0, "Q332: no unchanged file is parsed for the corpus batch: {:?} (warm run: {warm:?})", warm.corpus);
    assert_eq!(warm.corpus.from_tape, 0, "Q332: no unchanged file's document is built from its tape: {:?} (warm run: {warm:?})", warm.corpus);
    assert_eq!(warm.corpus.made, 0, "Q332: no unchanged document's rows are made: {:?} (warm run: {warm:?})", warm.corpus);
}

/// Phase 1's run kept across processes (t33 K4, `phases::frame`): a warm run over unchanged files takes its frame,
/// every read of its run as it was, and does not run the phase; an edit hands it another file, so the run makes a new
/// frame. Each run's outputs are a cold run's.
#[test]
fn phase_one_is_taken_from_its_frame_and_an_edit_runs_it_again() {
    let mut c = Corpus::new("phase-frame");
    let cold = c.run_over("kept.sqlite");
    assert_eq!(cold.frame, Some(false), "the cold run runs phase 1 and makes its frame (cold run: {cold:?})");
    let warm = c.run_over("kept.sqlite");
    assert_eq!(warm.frame, Some(true), "the warm run takes phase 1's frame (warm run: {warm:?})");
    let phase: Vec<&CorpusDocs> = warm.phases.iter().filter(|(name, _)| name == "sources").map(|(_, d)| d).collect();
    assert!(phase.iter().all(|d| d.documents == 0 && d.parsed == 0), "phase 1 not run: no corpus batch of its own (warm run: {warm:?})");
    assert_eq!(warm.rows, cold.rows, "the warm run's outputs are the cold run's");
    c.write(EDIT.0, EDIT.1);
    let edited = c.run_over("kept.sqlite");
    assert_eq!(edited.frame, Some(false), "an edit hands phase 1 another file: it runs, and makes its frame (edited run: {edited:?})");
    let again = c.run_over("kept.sqlite");
    assert_eq!(again.frame, Some(true), "the run after the edit takes the edit's frame (run: {again:?})");
    let fresh = c.run_over("fresh.sqlite").rows;
    assert_eq!(edited.rows, fresh, "the edited run's outputs are a cold run's");
    assert_eq!(again.rows, fresh, "the run after it gives the same");
}

/// A warm run after a one-file edit parses the changed file alone, and builds it alone in the waves.
#[test]
fn a_warm_edit_parses_the_changed_file_alone() {
    let (cold, warm, fresh) = Corpus::cold_then_warm("edit-parse", Some(EDIT));
    assert_eq!(warm.rows, fresh, "the warm edit's outputs are a cold run's");
    assert_eq!(cold.parsed(), FILES.len(), "the cold run parses each file once (cold run: {cold:?})");
    assert_eq!(warm.parsed(), 1, "Q332: the changed file is parsed, no other ({} parsed) (warm run: {warm:?})", warm.parsed());
    assert_eq!(warm.waves.0 + warm.waves.1, 1, "the changed file alone is built in the waves (found, parsed): {:?} (warm run: {warm:?})", warm.waves);
    assert_eq!(warm.corpus.parsed, 0, "Q332: no unchanged file is parsed again for the corpus batch: {:?} (warm run: {warm:?})", warm.corpus);
}

/// A warm run after a one-file edit builds no unchanged file's document: the corpus batch reads the unchanged files'
/// rows where the store keeps them, or never makes them (Q332's done-when).
#[test]
fn a_warm_edit_builds_no_unchanged_files_document() {
    let (_, warm, fresh) = Corpus::cold_then_warm("edit-build", Some(EDIT));
    assert_eq!(warm.rows, fresh, "the warm edit's outputs are a cold run's");
    assert_eq!(warm.corpus.from_tape, 0, "Q332: no unchanged file's document is built from its tape ({} files unchanged): {:?} (warm run: {warm:?})", FILES.len() - 1, warm.corpus);
}

/// A warm run after a one-file edit makes the rows of the changed file's document alone (decision 0494: a batch
/// makes a document when first read, and the edit reads the changed one's).
#[test]
fn a_warm_edit_makes_the_changed_documents_rows_alone() {
    let (_, warm, fresh) = Corpus::cold_then_warm("edit-made", Some(EDIT));
    assert_eq!(warm.rows, fresh, "the warm edit's outputs are a cold run's");
    // Each batch (the run's, and each phase's that holds documents) makes the changed one's alone.
    let batches = 1 + warm.phases.iter().filter(|(_, p)| p.documents > 0).count();
    assert!(warm.corpus.made <= batches, "Q332: of the corpus batches' documents only the changed one's rows are made, once a batch ({batches}): {:?} (warm run: {warm:?})", warm.corpus);
}

/// A warm run after a one-file edit: the run's own corpus batch (no phase's) builds no unchanged file's document and
/// makes the changed one's rows alone. Its plans' statistics of a view of `node` held in place (`kind`) are `node`'s,
/// kept and patched by the edit, not counted off every document (Q332).
#[test]
fn a_warm_edits_own_batch_makes_the_changed_document_alone() {
    let (_, warm, fresh) = Corpus::cold_then_warm("edit-own", Some(EDIT));
    assert_eq!(warm.rows, fresh, "the warm edit's outputs are a cold run's");
    assert_eq!(warm.main.from_tape, 0, "Q332: the run's batch builds no unchanged file's document from its tape: {:?} (warm run: {warm:?})", warm.main);
    assert!(warm.main.made <= 1, "Q332: the run's batch makes the changed document's rows alone: {:?} (warm run: {warm:?})", warm.main);
}

/// A JavaScript run over the scripts HTML files hold (`.embedded on`, decision 0470 step 2): each name a script calls
/// that `d.js` defines, at its line. The cross-file rule reads `line` of a node another file's rule found, so it reads
/// the corpus batch's base relations, the host files' documents among them.
const HOSTED: &str = ".language javascript\n.embedded on\n.decl called(c: node, x: symbol)\ncalled(c, x) :- kind(c, \"identifier\"), text(c, x).\n.decl defined(x: symbol)\ndefined(x) :- kind(d, \"function_declaration\"), field(d, \"name\", n), text(n, x).\n.decl named(x: symbol, l: number)\nnamed(x, l) :- called(c, x), defined(x), line(c, l).\n.output named\n";

/// The host files, HTML files whose scripts call `foo`, and the script of the run's own grammar that defines it.
const HOSTS: &[(&str, &str)] = &[
    ("d.js", "function foo(a) {\n  return a;\n}\n"),
    ("a.html", "<p>x</p>\n<script>foo(1)</script>\n"),
    ("b.html", "<script>foo(2); bar()</script>\n"),
    ("c.html", "<script>solo()</script>\n"),
];

/// Decision 0512 on a warm run: a file read for its programs is two documents, its text and its tree (a guest of the
/// file), and an unchanged one's are made when first read, as any unchanged file's are (Q332). The warm run builds
/// neither from its tape, so it builds no document at all.
#[test]
fn a_warm_run_builds_no_unchanged_host_files_documents() {
    let (_, warm, fresh) = Corpus::of("hosts", HOSTED, HOSTS, "named").then_warm(2 * HOSTS.len() - 1, None);
    assert_eq!(warm.rows, fresh, "the warm run's outputs are a cold run's");
    // Each file starts with its marker's line: foo is defined at d.js's line 2 and called at a.html's 3 and b.html's 2, the
    // files' lines, not the scripts' own (decision 0512).
    assert_eq!(fresh, [["foo", "2"], ["foo", "3"]].iter().map(|r| r.iter().map(|v| (*v).to_owned()).collect()).collect(), "foo at its definition's and its calls' lines");
    assert_eq!(warm.corpus.from_tape, 0, "Q332: no unchanged host file's documents are built from their tape: {:?} (warm run: {warm:?})", warm.corpus);
    assert_eq!(warm.corpus.parsed, 0, "Q332: no unchanged host file is parsed for the corpus batch: {:?} (warm run: {warm:?})", warm.corpus);
}

/// An edit of one host file: the unchanged host files' texts, trees and programs are pending, the changed one's old
/// documents are made whole (`hit_document`, its programs with them), and the edit's outputs are a cold run's.
#[test]
fn a_warm_edit_of_a_host_file_reads_as_a_cold_run() {
    let (_, warm, fresh) = Corpus::of("hosts-edit", HOSTED, HOSTS, "named").then_warm(2 * HOSTS.len() - 1, Some(("c.html", "<p>y</p>\n<p>z</p>\n<script>solo(); foo(4)</script>\n")));
    assert_eq!(warm.rows, fresh, "the warm edit's outputs are a cold run's");
    // Q332 for host files: the run's batch builds no unchanged host file's documents from their tape, and makes the
    // changed file's three alone (its text, its tree and its script's: `d.js`'s one and each host file's three are
    // the batch's 10). Its plans' statistics of `kind` are `node`'s kind column's, patched by the edit
    // (`Stats::kinds_counted`), not counted off every document.
    assert_eq!(warm.main.from_tape, 0, "Q332: the run's batch builds no unchanged host file's documents: {:?} (warm run: {warm:?})", warm.main);
    assert!(warm.main.made <= 3, "Q332: the run's batch makes the changed file's three documents' rows alone: {:?} (warm run: {warm:?})", warm.main);
    assert!(fresh.contains(&vec!["foo".to_owned(), "4".to_owned()]), "the edit's call of foo, at c.html's line 4 (its marker's, two paragraphs, its script): {fresh:?}");
}
