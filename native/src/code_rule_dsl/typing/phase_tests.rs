//! Typing as phases (decision 0083 §1, note t28): P1 `struct.rr`, then P2 `types.rr` over its exports, read by a
//! later program by symbol.

use crate::code_rule_dsl::logic::plan::corpus;
use crate::code_rule_dsl::typing::file_local::Binding;

/// P2 resolves imports through re-exports, bases, inherited members and annotations, from struct's exports alone.
#[test]
fn types_resolves_names_bases_members_and_annotations_by_symbol() {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("types-phase");
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("pkg")).unwrap();
    std::fs::write(src.join("pkg/__init__.py"), "from pkg.m import C as Exported\n").unwrap();
    std::fs::write(src.join("pkg/m.py"), "class B:\n    def run(self) -> int:\n        return 1\n    size: int = 0\n\nclass C(B):\n    def own(self) -> 'B':\n        return self\n").unwrap();
    std::fs::write(src.join("n.py"), "from pkg import Exported\nimport pkg.m\n\nlimit: int = 3\n\ndef f(c: Exported, d: pkg.m.B) -> None:\n    pass\n").unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let reader = r#"
#include "core.rr"
.decl types__binds(m: symbol, x: symbol, d: symbol)
.decl types__bases(c: symbol, i: number, b: symbol)
.decl types__attr(c: symbol, x: symbol, d: symbol, k: symbol)
.decl types__sig(f: symbol, i: number, x: symbol, a: symbol, d: symbol)
.decl types__rtype(f: symbol, a: symbol, d: symbol)
.decl types__vtype(s: symbol, a: symbol, d: symbol)
.decl b(m: symbol, x: symbol, d: symbol)
b(m, x, d) :- types__binds(m, x, d), m != "builtins".
.decl k(c: symbol, i: number, x: symbol)
k(c, i, x) :- types__bases(c, i, x).
.decl a(c: symbol, x: symbol, d: symbol, k: symbol)
a(c, x, d, k) :- types__attr(c, x, d, k).
.decl s(f: symbol, i: number, x: symbol, d: symbol)
s(f, i, x, d) :- types__sig(f, i, x, _, d).
.decl r(f: symbol, a: symbol, d: symbol)
r(f, a, d) :- types__rtype(f, a, d).
.decl v(s: symbol, d: symbol)
v(s, d) :- types__vtype(s, _, d).
.output b, k, a, s, r, v
"#;
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reader).unwrap()).unwrap();
    let report = corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::json!({})).unwrap();
    let names: Vec<&str> = report.phase_runs.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["struct", "types"]);
    let rows = |rel: &str| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
        v.sort();
        v
    };
    let b = rows("b");
    // A re-export followed: n's `Exported` is pkg/__init__'s alias of pkg.m.C.
    assert!(b.contains(&"n\tExported\tpkg.m.C".to_owned()), "{b:?}");
    assert!(b.contains(&"pkg\tExported\tpkg.m.C".to_owned()), "{b:?}");
    assert!(b.contains(&"n\tlimit\tn.limit".to_owned()), "{b:?}");
    assert_eq!(rows("k"), ["pkg.m.C\t0\tpkg.m.B"]);
    let a = rows("a");
    assert!(a.contains(&"pkg.m.C\trun\tpkg.m.B.run\tpkg.m.B".to_owned()), "inherited: {a:?}");
    assert!(a.contains(&"pkg.m.C\town\tpkg.m.C.own\tpkg.m.C".to_owned()), "{a:?}");
    assert!(a.contains(&"pkg.m.C\tsize\t\tpkg.m.B".to_owned()), "a value in a base's body: {a:?}");
    let s = rows("s");
    assert!(s.contains(&"n.f\t0\tc\tpkg.m.C".to_owned()), "an annotation through the re-export: {s:?}");
    assert!(s.contains(&"n.f\t1\td\tpkg.m.B".to_owned()), "a dotted annotation through `import pkg.m`: {s:?}");
    let r = rows("r");
    assert!(r.contains(&"pkg.m.C.own\t'B'\tpkg.m.B".to_owned()), "a quoted forward reference: {r:?}");
    let v = rows("v");
    assert!(v.contains(&"pkg.m.B.size\t".to_owned()) || v.iter().any(|l| l.starts_with("pkg.m.B.size\t")), "{v:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
