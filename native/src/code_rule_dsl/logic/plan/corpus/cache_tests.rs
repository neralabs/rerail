//! A cache is two rules (decision 0103): its output rule runs only for keys that are new, in a run with no session
//! (the kept strata) and in a session (maintenance); a gone key's rows go; a fact the key leaves out is not seen.

use crate::code_rule_dsl::typing::file_local::Binding;
use std::path::Path;

/// The key: each function a file imports from another that defines it. The cache: the function's body text, by its
/// name alone. The body is a fact the key leaves out.
const PROGRAM: &str = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(q: symbol)
imp(@dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl body(q: symbol, b: symbol)
body(q, b) :- def(n, q), field(n, "body", bn), text(bn, b).
.decl key(q: symbol)
key(q) :- imp(q), def(_, q).
.cache info by key
.decl info(q: symbol, b: symbol)
info(q, b) :- key(q), body(q, b).
.output info
"#;

struct Corpus {
    dir: std::path::PathBuf,
    marker: String,
    text: String,
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(name);
        std::fs::create_dir_all(dir.join("ws")).unwrap();
        let rule = dir.join("cache.rr");
        std::fs::write(&rule, PROGRAM).unwrap();
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        Corpus { dir, marker: format!("# {:?}\n", std::time::SystemTime::now()), text }
    }
    fn write(&self, file: &str, body: &str) {
        std::fs::write(self.dir.join("ws").join(file), format!("{}{body}", self.marker)).unwrap();
    }
    /// The run's `info` rows, sorted, with the store `store` (each name its own tape cache).
    fn run(&self, store: &str) -> Vec<String> {
        super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join(store)));
        let out = self.dir.join("out");
        let _ = std::fs::remove_dir_all(&out);
        super::run(&self.text, &self.dir.join("ws"), None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        rows(&out)
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn rows(out: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_to_string(out.join("info.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
    v.sort();
    v
}

fn has(rows: &[String], q: &str, body: &str) -> bool {
    rows.iter().any(|r| r.contains(q) && r.contains(body))
}

/// Decision 0103 §2-3, a run with no session: the cache's rows come from the kept strata, and the output rule runs
/// for the new keys only.
#[test]
#[ignore = "decision 0103: without a session a cache runs as its plain rules until its previous rows are read where every relation is kept (kept strata, per-file memo); no cache store"]
fn a_cache_runs_its_output_rule_only_for_new_keys() {
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let c = Corpus::new("cache-kept");
    c.write("a.py", "def f():\n    pass\n");
    c.write("b.py", "from a import f\n");
    let first = c.run("kept.sqlite");
    assert!(has(&first, "a.f", "pass"), "the first run makes the cache for its key: {first:?}");
    // f's body changes and its key does not: the kept rows stand, though a run with nothing kept sees the new body.
    c.write("a.py", "def f():\n    return 1\n");
    let kept = c.run("kept.sqlite");
    assert_eq!(kept, first, "a key in both runs keeps its rows; a fact the key leaves out is not seen");
    let fresh = c.run("fresh-1.sqlite");
    assert!(has(&fresh, "a.f", "return 1"), "{fresh:?}");
    // A new key: its rows are made, and the other key's rows are kept as they were.
    c.write("a.py", "def f():\n    return 1\ndef g():\n    return 2\n");
    c.write("b.py", "from a import f, g\n");
    let grown = c.run("kept.sqlite");
    assert!(has(&grown, "a.g", "return 2"), "a new key's rows are made: {grown:?}");
    assert!(has(&grown, "a.f", "pass"), "the kept key's rows are kept: {grown:?}");
    assert_eq!(grown.len(), 2, "{grown:?}");
    // A gone key: its rows go.
    c.write("b.py", "from a import g\n");
    let shrunk = c.run("kept.sqlite");
    assert_eq!(shrunk.len(), 1, "a gone key's rows go: {shrunk:?}");
    assert!(has(&shrunk, "a.g", "return 2"), "{shrunk:?}");
}

/// Decision 0103 §2-3, in a session: the same, by maintenance (`maintain::rewrite`), with the key's delta the
/// maintenance's own.
#[test]
fn a_cache_in_a_session_runs_its_output_rule_only_for_new_keys() {
    use super::session;
    let _one = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let c = Corpus::new("cache-session");
    c.write("a.py", "def f():\n    pass\n");
    c.write("b.py", "from a import f\n");
    session::start(&c.dir.join("session")).unwrap();
    let first = c.run("session.sqlite");
    assert!(has(&first, "a.f", "pass"), "{first:?}");
    c.write("a.py", "def f():\n    return 1\n");
    let kept = c.run("session.sqlite");
    c.write("a.py", "def f():\n    return 1\ndef g():\n    return 2\n");
    c.write("b.py", "from a import f, g\n");
    let grown = c.run("session.sqlite");
    c.write("b.py", "from a import g\n");
    let shrunk = c.run("session.sqlite");
    session::stop();
    assert_eq!(kept, first, "a key in both runs keeps its rows; a fact the key leaves out is not seen");
    assert!(has(&grown, "a.g", "return 2") && has(&grown, "a.f", "pass") && grown.len() == 2, "{grown:?}");
    assert!(shrunk.len() == 1 && has(&shrunk, "a.g", "return 2"), "{shrunk:?}");
}

/// Decision 0103 in the file-local part: an edited file's chunk runs again (its memo entry is stale), and reads the
/// store's rows of its keys; the output rule runs only for a key the store has not seen.
#[test]
#[ignore = "decision 0103: without a session a cache runs as its plain rules until its previous rows are read where every relation is kept (kept strata, per-file memo); no cache store"]
fn a_cache_of_the_file_local_part_runs_its_output_rule_only_for_new_keys() {
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let c = Corpus::of("cache-local", LOCAL);
    c.write("a.py", "def f():\n    pass\n");
    let first = c.run("kept.sqlite");
    assert!(has(&first, "f", "pass"), "{first:?}");
    // a.py's chunk runs again; f's key is in the store, so its rows are the store's.
    c.write("a.py", "def f():\n    return 1\n");
    let kept = c.run("kept.sqlite");
    assert_eq!(kept, first, "a key the store has keeps its rows; a fact the key leaves out is not seen");
    let fresh = c.run("fresh-1.sqlite");
    assert!(has(&fresh, "f", "return 1"), "{fresh:?}");
    // Another file's new key: made; f's kept.
    c.write("b.py", "def g():\n    return 2\n");
    let grown = c.run("kept.sqlite");
    assert!(has(&grown, "g", "return 2") && has(&grown, "f", "pass") && grown.len() == 2, "{grown:?}");
    // f gone from a.py: no row of it.
    c.write("a.py", "x = 1\n");
    let shrunk = c.run("kept.sqlite");
    assert!(shrunk.len() == 1 && has(&shrunk, "g", "return 2"), "{shrunk:?}");
}
