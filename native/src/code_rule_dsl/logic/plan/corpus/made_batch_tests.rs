//! Decision 0464's requirements over whole runs, one test each (R1–R3 are `facts::vtab::syntax`'s, over batches):
//! a batch makes a document when first read. A requirement is not dropped or weakened here without the user.

use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Cross-file: `named` joins two files' identifiers on their text and tests it, so the cross part reads `text` of
/// both files and the text test (0037) bound by it.
const PROGRAM: &str = r#"
.decl named(t: symbol)
named(t) :- kind(a, "identifier"), text(a, t), docof(a, d), kind(b, "identifier"), text(b, t), docof(b, e), d < e, contains("o", t).
.output named
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
    runs: usize,
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("made-batch-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = format!("# {name} {:?}\n", std::time::SystemTime::now());
        let rule = dir.join("program.rr");
        std::fs::write(&rule, PROGRAM).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
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

    /// A run over the store `store`: its report and `named`'s rows.
    fn run_over(&mut self, store: &str) -> (super::Report, BTreeSet<Vec<String>>) {
        self.runs += 1;
        let out = self.dir.join(format!("out{}", self.runs));
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join(store)));
        let report = super::run(&self.program, &self.dir.join("src"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        (report, crate::code_rule_dsl::logic::plan::output::shown(&out, "named").unwrap_or_default().into_iter().collect())
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
        profile::set(true);
        profile::take_decisions();
        let (_, rows) = c.run_over("kept.sqlite");
        let made = profile::take_decisions();
        profile::set(false);
        let versioned: BTreeSet<String> = made.keys().filter(|(t, _)| t == "the base relations' versions (decision 0458)").map(|(_, x)| x.clone()).collect();
        unversioned.extend(versioned.into_iter().filter(|x| x.ends_with("a text test, no version (decision 0464)")));
        let (_, fresh) = c.run_over(&format!("fresh{i}.sqlite"));
        assert_eq!(rows, fresh, "edit {i} ({file}): the outputs are a cold run's");
    }
    assert!(!unversioned.is_empty(), "the cross part reads the text test, given no version");
}
