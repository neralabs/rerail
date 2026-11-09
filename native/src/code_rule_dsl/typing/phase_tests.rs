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
    std::fs::write(src.join("pkg/m.py"), "class B:\n    def run(self) -> int:\n        return 1\n    size: int = 0\n    again = run\n\nclass C(B):\n    def own(self) -> 'B':\n        return self\n\nAlias = C\nDotted = pkg.m.B\nimport pkg.m\n").unwrap();
    std::fs::write(src.join("pkg/star.py"), "from pkg.m import *\n").unwrap();
    std::fs::write(src.join("n.py"), "from pkg import Exported\nimport pkg.m\nfrom pkg.star import B\n\nlimit: int = 3\n\ndef f(c: Exported, d: pkg.m.B) -> None:\n    pass\n\ndef g(u: B | None) -> list[B] | None:\n    pass\n").unwrap();
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
    assert!(b.contains(&"n\tB\tpkg.m.B".to_owned()), "through `from pkg.m import *`: {b:?}");
    assert_eq!(rows("k"), ["pkg.m.C\t0\tpkg.m.B"]);
    let a = rows("a");
    assert!(a.contains(&"pkg.m.C\trun\tpkg.m.B.run\tpkg.m.B".to_owned()), "inherited: {a:?}");
    // Aliases followed: a module's name assigned a name or a dotted name, and a class's member assigned another.
    assert!(b.contains(&"pkg.m\tAlias\tpkg.m.C".to_owned()), "{b:?}");
    assert!(b.contains(&"pkg.m\tDotted\tpkg.m.B".to_owned()), "{b:?}");
    assert!(a.contains(&"pkg.m.C\tagain\tpkg.m.B.run\tpkg.m.B".to_owned()), "a member's alias, inherited: {a:?}");
    assert!(a.contains(&"pkg.m.C\town\tpkg.m.C.own\tpkg.m.C".to_owned()), "{a:?}");
    assert!(a.contains(&"pkg.m.C\tsize\t\tpkg.m.B".to_owned()), "a value in a base's body: {a:?}");
    let s = rows("s");
    assert!(s.contains(&"n.f\t0\tc\tpkg.m.C".to_owned()), "an annotation through the re-export: {s:?}");
    assert!(s.contains(&"n.f\t1\td\tpkg.m.B".to_owned()), "a dotted annotation through `import pkg.m`: {s:?}");
    // A union: each alternative's head, None as builtins.None.
    assert!(s.contains(&"n.g\t0\tu\tpkg.m.B".to_owned()) && s.contains(&"n.g\t0\tu\tbuiltins.None".to_owned()), "{s:?}");
    let r = rows("r");
    assert!(r.contains(&"n.g\tlist[B] | None\tbuiltins.None".to_owned()), "{r:?}");
    assert!(r.contains(&"pkg.m.C.own\t'B'\tpkg.m.B".to_owned()), "a quoted forward reference: {r:?}");
    let v = rows("v");
    assert!(v.contains(&"pkg.m.B.size\t".to_owned()) || v.iter().any(|l| l.starts_with("pkg.m.B.size\t")), "{v:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Spike for note t28 step 4: P1 then P2 over a corpus with typeshed (`RERAIL_PYTHON_CALLS_ROOT`,
/// `RERAIL_PYTHON_TYPESHED`, `RERAIL_TAPE_CACHE`): each phase's CPU and export rows, cold and unchanged, and how much
/// of the workspace's imports and annotations P2 resolves to a symbol, and how much of that is a library's.
#[test]
#[ignore = "a spike: minutes over the 84-file corpus with typeshed"]
fn spike_types_over_the_corpus() {
    let root = std::path::PathBuf::from(std::env::var("RERAIL_PYTHON_CALLS_ROOT").expect("RERAIL_PYTHON_CALLS_ROOT"));
    let settings: Vec<String> = std::env::var("RERAIL_PYTHON_TYPESHED").map(|t| vec![format!("python.typeshed={t}")]).unwrap_or_default();
    let out = std::env::temp_dir().join(format!("rerail-types-{}", std::process::id()));
    let reader = r#"
#include "core.rr"
.decl struct__module_file(m: symbol, p: symbol)
.decl struct__binds(m: symbol, x: symbol, how: symbol, t: symbol)
.decl types__binds(m: symbol, x: symbol, d: symbol)
.decl types__owner(s: symbol, m: symbol)
.decl types__sig(f: symbol, i: number, x: symbol, a: symbol, d: symbol)
.decl types__rtype(f: symbol, a: symbol, d: symbol)
.decl types__vtype(s: symbol, a: symbol, d: symbol)
.decl types__attr(c: symbol, x: symbol, d: symbol, k: symbol)
.decl ws(m: symbol)
ws(m) :- struct__module_file(m, p), !contains("typeshed", p).
.decl lib(m: symbol)
lib(m) :- struct__module_file(m, p), contains("typeshed", p).
.decl imp(m: symbol, x: symbol, t: symbol)
imp(m, x, t) :- struct__binds(m, x, "import", t), ws(m).
.decl imp_ok(m: symbol, x: symbol, t: symbol)
imp_ok(m, x, t) :- imp(m, x, t), types__binds(m, x, _).
.decl imp_lib(m: symbol, x: symbol, t: symbol)
imp_lib(m, x, t) :- imp(m, x, t), types__binds(m, x, d), types__owner(d, o), lib(o).
.decl imp_miss(m: symbol, x: symbol, t: symbol)
imp_miss(m, x, t) :- imp(m, x, t), !imp_ok(m, x, t), !imp_absent(m, x, t).
.decl struct__def(s: symbol, k: symbol)
// An import of a module the corpus does not have (the fixture is part of a repository) or of a name in one.
.decl imp_absent(m: symbol, x: symbol, t: symbol)
imp_absent(m, x, t) :- imp(m, x, t), !imp_ok(m, x, t), !struct__def(t, "module"), split_dot(t, p, y), !struct__def(p, "module").
imp_absent(m, x, t) :- imp(m, x, t), !imp_ok(m, x, t), !contains(".", t), !struct__def(t, "module").
.decl ann(f: symbol, i: number, a: symbol)
ann(f, i, a) :- types__sig(f, i, _, a, _), a != "", types__owner(f, m), ws(m).
ann(f, -1, a) :- types__rtype(f, a, _), types__owner(f, m), ws(m).
ann(s, -2, a) :- types__vtype(s, a, _), types__owner(s, m), ws(m).
.decl ann_ok(f: symbol, i: number, a: symbol)
ann_ok(f, i, a) :- types__sig(f, i, _, a, d), a != "", d != "", types__owner(f, m), ws(m).
ann_ok(f, -1, a) :- types__rtype(f, a, d), d != "", types__owner(f, m), ws(m).
ann_ok(s, -2, a) :- types__vtype(s, a, d), d != "", types__owner(s, m), ws(m).
.decl ann_lib(f: symbol, i: number, a: symbol)
ann_lib(f, i, a) :- types__sig(f, i, _, a, d), a != "", types__owner(f, m), ws(m), types__owner(d, o), lib(o).
ann_lib(f, -1, a) :- types__rtype(f, a, d), types__owner(f, m), ws(m), types__owner(d, o), lib(o).
ann_lib(s, -2, a) :- types__vtype(s, a, d), types__owner(s, m), ws(m), types__owner(d, o), lib(o).
.decl ann_miss(f: symbol, i: number, a: symbol)
ann_miss(f, i, a) :- ann(f, i, a), !ann_ok(f, i, a).
.decl lib_attr(c: symbol, x: symbol)
lib_attr(c, x) :- types__attr(c, x, _, _), types__owner(c, o), lib(o).
.output ws, lib, imp, imp_ok, imp_lib, imp_miss, imp_absent, ann, ann_ok, ann_lib, ann_miss, lib_attr
"#;
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reader).unwrap()).unwrap();
    let run = || corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings })).unwrap();
    let count = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).map_or(0, |t| t.lines().count());
    let show = |what: &str, r: &corpus::Report| {
        for p in &r.phase_runs {
            let cpu: f64 = p.report.phases.iter().map(|x| x.1 + x.2).sum();
            let rss = p.report.phases.iter().map(|x| x.3).fold(0.0, f64::max);
            let rows: Vec<String> = p.exports.iter().map(|(rel, n, _, _)| format!("{rel} {n}")).collect();
            println!("TYPES {what}: {} {cpu:.2} s CPU, peak RSS {rss:.0} MB, how: {}; exports: {}", p.name, p.full_because.as_deref().map_or_else(|| p.report.phases.first().map_or("", |x| x.0).to_owned(), |w| format!("in full: {w}")), rows.join(", "));
        }
        let all: f64 = r.phases.iter().map(|x| x.1 + x.2).sum();
        println!("TYPES {what}: the reader's run, phases included: {all:.2} s");
    };
    let cold = run();
    show("cold", &cold);
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    println!("TYPES modules: workspace {}, library {}", count("ws"), count("lib"));
    println!("TYPES workspace imports: {} bound, {} resolved ({:.1}%), {} to a library's symbol; {} of a module the corpus lacks, {} missed ({:.1}% of the rest resolved)", count("imp"), count("imp_ok"), pct(count("imp_ok"), count("imp")), count("imp_lib"), count("imp_absent"), count("imp_miss"), pct(count("imp_ok"), count("imp") - count("imp_absent")));
    println!("TYPES workspace annotations: {} written, {} resolved ({:.1}%), {} to a library's symbol", count("ann"), count("ann_ok"), pct(count("ann_ok"), count("ann")), count("ann_lib"));
    println!("TYPES library class lookups answered by attr: {}", count("lib_attr"));
    for rel in ["imp_miss", "ann_miss"] {
        let text = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
        for l in text.lines().take(25) {
            println!("TYPES {rel}: {l}");
        }
    }
    show("unchanged", &run());
    let _ = std::fs::remove_dir_all(&out);
}
