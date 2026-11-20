//! Warm runs (Q332): what a run over the store a cold run kept does to the files that did not change. It parses none,
//! builds no document of one, and makes no row of one (0104, decision 0458's corpus batch, 0494). Each test is a few
//! files, a cold run and the warm one, and checks the counts the run reports (`Report::corpus_docs`, `file_tapes`):
//! a change that makes a warm run redo a cold run's work fails here, by name, in seconds.
//!
//! The warm tier, these with the warm-run tests elsewhere (`.config/nextest.toml`'s `warm` profile):
//!
//! ```text
//! cargo nextest run -p rerail-engine --lib --features test-hooks --profile warm
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
    runs: usize,
}

/// What a run reported, and `named`'s rows.
#[derive(Debug)]
struct Ran {
    corpus: CorpusDocs,
    /// The files the run parsed, wherever (`core::parses`), and by the path that parsed them and their language.
    parsed: usize,
    parsed_by: std::collections::BTreeMap<(&'static str, String), usize>,
    /// The files built in the waves with a kept tape, and those parsed.
    waves: (usize, usize),
    rows: BTreeSet<Vec<String>>,
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("warm-{name}"));
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

    /// A run over the store `store`, kept from run to run: what a new process does.
    fn run_over(&mut self, store: &str) -> Ran {
        self.runs += 1;
        let out = self.dir.join(format!("out{}", self.runs));
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join(store)));
        let report = super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let rows = crate::code_rule_dsl::logic::plan::output::shown(&out, "named").unwrap_or_default().into_iter().collect();
        Ran { corpus: report.corpus_docs, parsed: report.parsed.values().sum(), parsed_by: report.parsed, waves: report.file_tapes, rows }
    }

    /// A cold run, then the warm one over the same store after `edit` (none: the files as they were), and a cold run
    /// over the files as they are then, from an empty store: R10's truth.
    fn cold_then_warm(name: &str, edit: Option<(&str, &str)>) -> (Ran, Ran, BTreeSet<Vec<String>>) {
        let mut c = Corpus::new(name);
        let cold = c.run_over("kept.sqlite");
        assert_eq!(cold.corpus.documents, FILES.len(), "the cold run's corpus batch holds every file's document: {cold:?}");
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
    assert_eq!(warm.parsed, 0, "no file is parsed: {} parsed (warm run: {warm:?})", warm.parsed);
    assert_eq!(warm.waves, (0, 0), "no file is built in the waves (found, parsed): every one is a memo hit (warm run: {warm:?})");
    assert_eq!(warm.corpus.parsed, 0, "Q332: no unchanged file is parsed for the corpus batch: {:?} (warm run: {warm:?})", warm.corpus);
    assert_eq!(warm.corpus.from_tape, 0, "Q332: no unchanged file's document is built from its tape: {:?} (warm run: {warm:?})", warm.corpus);
    assert_eq!(warm.corpus.made, 0, "Q332: no unchanged document's rows are made: {:?} (warm run: {warm:?})", warm.corpus);
}

/// A warm run after a one-file edit parses the changed file alone, and builds it alone in the waves.
#[test]
fn a_warm_edit_parses_the_changed_file_alone() {
    let (cold, warm, fresh) = Corpus::cold_then_warm("edit-parse", Some(EDIT));
    assert_eq!(warm.rows, fresh, "the warm edit's outputs are a cold run's");
    assert_eq!(cold.parsed, FILES.len(), "the cold run parses each file once (cold run: {cold:?})");
    assert_eq!(warm.parsed, 1, "Q332: the changed file is parsed, no other ({} parsed) (warm run: {warm:?})", warm.parsed);
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
    assert!(warm.corpus.made <= 1, "Q332: of the corpus batch's documents only the changed one's rows are made: {:?} (warm run: {warm:?})", warm.corpus);
}
