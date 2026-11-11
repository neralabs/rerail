//! A cache is two ordinary rules (decision 0103), and its facts are just facts: a run's cache rows are what a run with
//! nothing kept derives, in a run with no session and in a session; bumping the key rule's version tag derives the
//! cache again, the version visible in its rows.

use crate::code_rule_dsl::typing::file_local::Binding;
use std::path::Path;

/// The key: each function a file imports from another that defines it, with the version tag `V`. The cache: the
/// function's body text, by the key.
const PROGRAM: &str = r#"
.decl def(n: node, q: symbol)
def(n, @dot(m, x)) :- kind(n, "function_definition"), field(n, "name", c), text(c, x), docof(n, d), doc(d, _, m).
.decl imp(q: symbol)
imp(@dot(m, x)) :- kind(n, "import_from_statement"), field(n, "module_name", mn), text(mn, m), field(n, "name", xn), text(xn, x).
.decl body(q: symbol, b: symbol)
body(q, b) :- def(n, q), field(n, "body", bn), text(bn, b).
.cache info by key
.decl key(q: symbol, v: symbol)
key(q, "V") :- imp(q), def(_, q).
.decl info(q: symbol, v: symbol, b: symbol)
info(q, v, b) :- key(q, v), body(q, b).
.output info
"#;

fn program(version: &str) -> String {
    PROGRAM.replace("\"V\"", &format!("\"{version}\""))
}

struct Corpus {
    dir: std::path::PathBuf,
    marker: String,
    text: String,
}

impl Corpus {
    fn new(name: &str) -> Corpus {
        Corpus::of(name, &program("v1"))
    }
    fn of(name: &str, program: &str) -> Corpus {
        let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(name);
        std::fs::create_dir_all(dir.join("ws")).unwrap();
        let rule = dir.join("cache.rr");
        std::fs::write(&rule, program).unwrap();
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
        Corpus { dir, marker: format!("# {:?}\n", std::time::SystemTime::now()), text }
    }
    /// The program again, with the key rule's version tag `version`.
    fn bump(&mut self, version: &str) {
        let rule = self.dir.join("cache.rr");
        std::fs::write(&rule, program(version)).unwrap();
        self.text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
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

/// The steps every test takes: the cache's rows follow its facts, as a run with nothing kept derives them, and a
/// version bump derives them again.
fn steps(c: &mut Corpus, store: &str, fresh: &mut dyn FnMut(&Corpus, usize) -> Vec<String>) {
    c.write("a.py", "def f():\n    pass\n");
    c.write("b.py", "from a import f\n");
    let first = c.run(store);
    assert!(has(&first, "a.f", "pass") && first.len() == 1, "{first:?}");
    // f's body changes: the cache's rows are facts, so they follow it.
    c.write("a.py", "def f():\n    return 1\n");
    let edited = c.run(store);
    assert_eq!(edited, fresh(c, 1), "a cache's rows are what a run with nothing kept derives");
    assert!(has(&edited, "a.f", "return 1"), "{edited:?}");
    // A new key, then a gone one.
    c.write("a.py", "def f():\n    return 1\ndef g():\n    return 2\n");
    c.write("b.py", "from a import f, g\n");
    let grown = c.run(store);
    assert!(has(&grown, "a.g", "return 2") && has(&grown, "a.f", "return 1") && grown.len() == 2, "{grown:?}");
    c.write("b.py", "from a import g\n");
    let shrunk = c.run(store);
    assert!(shrunk.len() == 1 && has(&shrunk, "a.g", "return 2"), "{shrunk:?}");
    // The version bumped: every row is derived again, with the new version.
    c.bump("v2");
    let bumped = c.run(store);
    assert!(bumped.len() == 1 && has(&bumped, "v2", "return 2"), "{bumped:?}");
    assert_eq!(bumped, fresh(c, 2));
}

/// A run with no session: the cache's rows are kept as every relation's are (the kept strata).
#[test]
fn a_cache_is_facts_and_a_version_bump_derives_it_again() {
    let _no_session = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = Corpus::new("cache-kept");
    steps(&mut c, "kept.sqlite", &mut |c, i| c.run(&format!("fresh-{i}.sqlite")));
}

/// In a session: the same, by the session's maintenance of ordinary relations; a run with nothing kept is a full
/// run over a store of its own.
#[test]
fn a_cache_in_a_session_is_facts_and_a_version_bump_derives_it_again() {
    use super::session;
    let _one = super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = Corpus::new("cache-session");
    session::start(&c.dir.join("session")).unwrap();
    steps(&mut c, "session.sqlite", &mut |c, i| session::full(|| c.run(&format!("fresh-{i}.sqlite"))));
    session::stop();
}
