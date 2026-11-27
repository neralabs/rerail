    use crate::code_rule_dsl::typing::file_local::Binding;

    /// Phase 1 (`sources.rerail`, decision 0104): a run's documents are those phase 1 loads, one per source file's key.
    /// After an edit, only the edited file is built again, in either phase: the others are their memos' entries.
    #[test]
    fn a_runs_documents_come_from_phase_1_and_an_edit_builds_only_its_file() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("sources");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"), "def f():\n    return 1\n").unwrap();
        std::fs::write(src.join("b.py"), "def g():\n    return 2\n").unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version("#include \"core.rerail\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap(),
        )
        .unwrap();
        let run = || crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::Value::Null).unwrap();
        let defs = || crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap();
        let first = run();
        let sources = first.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.exports, vec![("document".to_owned(), 2), ("library".to_owned(), 0), ("source_library".to_owned(), 0)], "each source file is a document of phase 1's, and no library is set");
        assert_eq!(defs(), [["f"], ["g"]]);
        std::fs::write(src.join("a.py"), "def h():\n    return 1\n").unwrap();
        let second = run();
        assert_eq!(defs(), [["g"], ["h"]], "the findings follow the edit");
        let sources = second.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.report.memo.0, 1, "phase 1 builds only the edited file");
        assert_eq!(second.memo.0, 1, "so does the run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 1 resolves the imports to their fixed point: a library an import reaches is loaded, and its own imports in
    /// turn. A program that reads `sources__library` takes those documents as its own loaded documents, and loads
    /// nothing. An edit of a library file is followed; the tree's file is not built again.
    #[test]
    fn phase_1_loads_the_libraries_the_imports_reach_and_a_reader_takes_them() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("sources-libraries");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, lib) = (dir.join("src"), dir.join("ts/stdlib"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(src.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
        std::fs::write(src.join("m.py"), "import a\n\ndef own():\n    return 1\n").unwrap();
        std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\na: 3.0-\nb: 3.0-\nc: 3.0-\n").unwrap();
        std::fs::write(lib.join("builtins.pyi"), "class object: ...\n").unwrap();
        std::fs::write(lib.join("a.pyi"), "import b\ndef fa() -> None: ...\n").unwrap();
        std::fs::write(lib.join("b.pyi"), "def fb() -> None: ...\n").unwrap();
        std::fs::write(lib.join("c.pyi"), "def fc() -> None: ...\n").unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let reader = "#include \"core.rerail\"\n.decl sources__library(p: symbol, m: symbol, g: symbol, c: symbol)\n.decl lib(m: symbol)\nlib(m) :- sources__library(_, m, _, _).\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output lib, defs\n";
        let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(reader).unwrap()).unwrap();
        let run = || crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::Value::Null).unwrap();
        let rows = |rel: &str| {
            let mut v: Vec<Vec<String>> = crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
            v.sort();
            v
        };
        let first = run();
        assert_eq!(first.phase_runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["sources"]);
        let sources = first.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.exports, vec![("document".to_owned(), 1), ("library".to_owned(), 3), ("source_library".to_owned(), 0)], "m, and builtins, a and b: c is imported by none; each a stub");
        assert_eq!(rows("lib"), [["a"], ["b"], ["builtins"]]);
        assert_eq!(rows("defs"), [["fa"], ["fb"], ["own"]], "the libraries are the reader's documents");
        std::fs::write(lib.join("b.pyi"), "import c\ndef fb2() -> None: ...\n").unwrap();
        let second = run();
        assert_eq!(rows("lib"), [["a"], ["b"], ["builtins"], ["c"]], "the edited library's import is followed");
        assert_eq!(rows("defs"), [["fa"], ["fb2"], ["fc"], ["own"]]);
        let sources = second.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.report.memo.0, 3, "phase 1 reads m, builtins and a from the memo, and builds only the edited library and the new one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A library read from its source (an installed package with neither stubs nor `py.typed`, under `python.untyped =
    /// read`) is a `source_library` of phase 1's as well as a library. A program that reads `sources__source_library`
    /// takes those documents as its own, and no other library's: typeshed's stub is not among them.
    #[test]
    fn a_reader_of_the_source_libraries_takes_those_alone() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("sources-source-libraries");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, lib, site) = (dir.join("src"), dir.join("ts/stdlib"), dir.join("venv/lib/python3.12/site-packages/u"));
        for d in [&src, &lib, &site] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(src.join("rerail.env"), "python.typeshed = ../ts\npython.venv = ../venv\npython.untyped = read\n").unwrap();
        std::fs::write(src.join("m.py"), "import a\nimport u\n\ndef own():\n    return 1\n").unwrap();
        std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\na: 3.0-\n").unwrap();
        std::fs::write(lib.join("builtins.pyi"), "class object: ...\n").unwrap();
        std::fs::write(lib.join("a.pyi"), "def fa() -> None: ...\n").unwrap();
        std::fs::write(site.join("__init__.py"), "def fu():\n    return 1\n").unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let reader = "#include \"core.rerail\"\n.decl sources__source_library(p: symbol)\n.decl srclib(p: symbol)\nsrclib(p) :- sources__source_library(p).\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output srclib, defs\n";
        let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(reader).unwrap()).unwrap();
        let report = crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::Value::Null).unwrap();
        let rows = |rel: &str| {
            let mut v: Vec<Vec<String>> = crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
            v.sort();
            v
        };
        let sources = report.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.exports, vec![("document".to_owned(), 1), ("library".to_owned(), 3), ("source_library".to_owned(), 1)], "m; builtins, a and u; u");
        let srclib = rows("srclib");
        assert!(srclib.len() == 1 && srclib[0][0].replace(std::path::MAIN_SEPARATOR, "/").ends_with("site-packages/u/__init__.py"), "{srclib:?}");
        assert_eq!(rows("defs"), [["fu"], ["own"]], "u's source is the reader's document, a's stub is not");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A phase runs in its own space: a reader sees its exports and nothing else, and an export naming a node is
    /// refused. The phase keeps nothing of its own: across runs, its run keeps its relations as any run does, and the
    /// reader's rows follow every edit.
    #[test]
    fn a_phase_hands_on_its_exports_and_its_reader_follows_each_edit() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        let write = |name: &str, text: &str| std::fs::write(src.join(name), text).unwrap();
        write("a.py", "def f():\n    return 1\n\ndef g():\n    return 2\n");
        let phase = |exports: &str, extra: &str| {
            std::fs::write(phases.join("p1.rerail"), format!(r#"
#include "core.rerail"
.phase p1
.export {exports}
.decl unit(s: symbol, h: symbol)
unit("m", h) :- def_digest(n, h), kind(n, "module").
unit(@cat("m", x), h) :- def_digest(n, h), field(n, "name", i), text(i, x).
.decl def(s: symbol, k: symbol)
def(@cat("m", x), k) :- kind(n, k), k = "function_definition", field(n, "name", i), text(i, x).
def("m", "module") :- kind(_, "module").
.decl private(s: symbol)
private(s) :- def(s, _).
.decl at(n: node, s: symbol)
at(n, s) :- kind(n, "function_definition"), field(n, "name", i), text(i, s).
{extra}
"#)).unwrap();
        };
        let reader = |reads: &str| format!("{reads}\n.decl r(s: symbol)\nr(s) :- p1__def(s, _).\n.output r\n");
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let run = |text: &str| crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options);
        let print = |text: &str| crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()).unwrap();
        // A private relation is not the reader's to read.
        phase("unit, def", "");
        let err = run(&print(&reader(".decl p1__def(s: symbol, k: symbol)\n.decl p1__private(s: symbol)\n.decl q(s: symbol)\nq(s) :- p1__private(s).\n.output q"))).err().expect("a private relation refused").to_string();
        assert!(err.contains("does not export private"), "{err}");
        // An export naming a node is refused.
        phase("unit, def, at", "");
        let err = run(&print(&reader(".decl p1__def(s: symbol, k: symbol)"))).err().expect("a node export refused").to_string();
        assert!(err.contains("is a node"), "{err}");
        phase("unit, def", "");
        let text = print(&reader(".decl p1__def(s: symbol, k: symbol)"));
        let first = run(&text).unwrap();
        let r = || crate::code_rule_dsl::logic::plan::output::shown(&out, "r").unwrap();
        assert_eq!(r(), [["m"], ["m.f"], ["m.g"]]);
        // The reader reads `def` alone, so the phase's run makes `def` alone (`unit` is no output of it).
        assert_eq!(first.phase_runs.iter().find(|r| r.name == "p1").expect("p1 ran").exports, [("def".to_owned(), 3)]);
        assert_eq!(first.files, 0, "the reader reads only the phase's exports, so no file");
        // Unchanged, a comment, a body: the exports the reader reads are as they were.
        run(&text).unwrap();
        assert_eq!(r(), [["m"], ["m.f"], ["m.g"]]);
        write("a.py", "# note\ndef f():\n    x = 1\n    return x\n\n\ndef g():  # g\n    return 2\n");
        run(&text).unwrap();
        assert_eq!(r(), [["m"], ["m.f"], ["m.g"]]);
        // A function added, then one removed: the phase's run follows, and so does the reader.
        write("a.py", "def f():\n    return 1\n\ndef g():\n    return 2\n\ndef h():\n    pass\n");
        run(&text).unwrap();
        assert_eq!(r(), [["m"], ["m.f"], ["m.g"], ["m.h"]]);
        write("a.py", "def f():\n    return 1\n\ndef h():\n    pass\n");
        run(&text).unwrap();
        assert_eq!(r(), [["m"], ["m.f"], ["m.h"]]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A phase's run makes only the exports its reader reads: the others are no outputs of it, so preparation drops
    /// them with the rules only they reach, and the phases only they read do not run. `bandit-b113` read none of
    /// `py_names`' Django facts, which made 700,000 rows over 224 files for nothing.
    #[test]
    fn a_phase_makes_only_the_exports_its_reader_reads() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases-wanted");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        std::fs::write(src.join("a.py"), "def f():\n    return 1\n\ndef g():\n    return 2\n").unwrap();
        std::fs::write(phases.join("p2.rerail"), r#"
#include "core.rerail"
.phase p2
.export tag
.decl tag(s: symbol)
tag(x) :- kind(n, "function_definition"), field(n, "name", i), text(i, x).
"#).unwrap();
        std::fs::write(phases.join("p1.rerail"), r#"
#include "core.rerail"
.phase p1
.export def, tagged
.decl def(s: symbol, k: symbol)
def(@cat("m", x), k) :- kind(n, k), k = "function_definition", field(n, "name", i), text(i, x).
.decl p2__tag(s: symbol)
.decl tagged(s: symbol)
tagged(@cat("t", s)) :- p2__tag(s).
"#).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let print = |text: &str| crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()).unwrap();
        let run = |text: &str| crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&print(text)], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let rows = |rel: &str| {
            let mut v: Vec<Vec<String>> = crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap();
            v.sort();
            v
        };
        let one = run(".decl p1__def(s: symbol, k: symbol)\n.decl r(s: symbol)\nr(s) :- p1__def(s, _).\n.output r\n");
        assert_eq!(rows("r"), [["m.f"], ["m.g"]]);
        assert_eq!(one.phase_runs.iter().filter(|p| p.name != "sources").map(|p| (p.name.clone(), p.exports.clone())).collect::<Vec<_>>(), [("p1".to_owned(), vec![("def".to_owned(), 2)])], "p1 makes `def` alone, and p2, which only `tagged` reads, does not run");
        let both = run(".decl p1__def(s: symbol, k: symbol)\n.decl p1__tagged(s: symbol)\n.decl r(s: symbol)\nr(s) :- p1__def(s, _).\nr(s) :- p1__tagged(s).\n.output r\n");
        assert_eq!(rows("r"), [["m.f"], ["m.g"], ["t.f"], ["t.g"]]);
        assert_eq!(
            both.phase_runs.iter().filter(|p| p.name != "sources").map(|p| (p.name.clone(), p.exports.clone())).collect::<Vec<_>>(),
            [("p2".to_owned(), vec![("tag".to_owned(), 2)]), ("p1".to_owned(), vec![("def".to_owned(), 2), ("tagged".to_owned(), 2)])],
            "read, `tagged` is made, and p2 runs first for it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phases chain: a phase may read an earlier phase's exports (0083: struct, then types, then the analysis), and
    /// the reader sees only the last phase's. Phases that read each other are refused, not run forever.
    #[test]
    fn a_phase_reads_an_earlier_phase_and_a_cycle_is_refused() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases-chain");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        std::fs::write(src.join("a.py"), "class A:\n    pass\n\nclass B(A):\n    pass\n").unwrap();
        let p1 = r#"
#include "core.rerail"
.phase p1
.export def, base
.decl def(s: symbol, k: symbol)
def(x, "class") :- kind(n, "class_definition"), field(n, "name", i), text(i, x).
.decl base(s: symbol, b: symbol)
base(x, b) :- kind(n, "class_definition"), field(n, "name", i), text(i, x), field(n, "superclasses", l), child(l, e, _), text(e, b).
"#;
        // The typing phase reads only p1's exports: the classes' ancestors, as a later phase would read them.
        let p2 = r#"
.phase p2
.export anc
.decl p1__def(s: symbol, k: symbol)
.decl p1__base(s: symbol, b: symbol)
.decl anc(s: symbol, a: symbol)
anc(s, a) :- p1__base(s, a), p1__def(a, _).
anc(s, a) :- anc(s, b), p1__base(b, a).
"#;
        std::fs::write(phases.join("p1.rerail"), p1).unwrap();
        std::fs::write(phases.join("p2.rerail"), p2).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let run = |text: &str| crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options);
        let reader = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version("#include \"core.rerail\"\n.decl p2__anc(s: symbol, a: symbol)\n.decl r(s: symbol, a: symbol)\nr(s, a) :- p2__anc(s, a).\n.output r\n").unwrap()).unwrap();
        let report = run(&reader).unwrap();
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "r").unwrap(), [["B", "A"]]);
        // Phase 1 (`sources`) first, which gives every run its documents, then both phases, listed flat, in the order
        // they ran; p2's own run checked p1 once, not again.
        let names: Vec<&str> = report.phase_runs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["sources", "p1", "p2"]);
        assert!(report.phase_runs[2].report.phase_runs.is_empty(), "p1 is not checked again inside p2's run");
        // Nothing changed: the same rows.
        run(&reader).unwrap();
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "r").unwrap(), [["B", "A"]]);
        // p1 changed (its program: no class has a base now): p2, which reads it, is not kept.
        std::fs::write(phases.join("p1.rerail"), p1.replace("text(e, b).", "text(e, b), b = \"none\".")).unwrap();
        run(&reader).unwrap();
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "r").unwrap(), Vec::<Vec<String>>::new(), "p2 run again over p1's new exports");
        // A reader of p2 that loads documents itself is refused: p1 loaded none, so first give p1 a load.
        std::fs::write(phases.join("p1.rerail"), format!("{p1}\nload(\"x.pyi\", \"x\", \"python\") :- def(\"A\", _).\n")).unwrap();
        let loading = reader.replace(".output r", "load(\"y.pyi\", \"y\", \"python\") :- r(_, _).\n.output r");
        let err = run(&loading).err().expect("a later phase's reader that loads").to_string();
        assert!(err.contains("loads documents itself"), "{err}");
        // p1 reading p2's export: each runs the other first.
        std::fs::write(phases.join("p1.rerail"), format!("{p1}\n.decl p2__anc(s: symbol, a: symbol)\n.decl seen(s: symbol)\nseen(s) :- p2__anc(s, _).\n.output seen\n")).unwrap();
        let err = run(&reader).err().expect("a cycle refused").to_string();
        assert!(err.contains("phases read each other"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A reader with a constant of its own (made from the run's settings alone, folded first by `constants`) reads a
    /// phase: the constants' run starts no phase, so the phase runs once, as itself, and writes its exports.
    #[test]
    fn a_reader_with_constants_runs_its_phases_once_as_themselves() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases-constants");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        std::fs::write(src.join("a.py"), "class A:\n    pass\n").unwrap();
        std::fs::write(phases.join("p1.rerail"), "#include \"core.rerail\"\n.phase p1\n.export def\n.decl def(s: symbol, k: symbol)\ndef(x, \"class\") :- kind(n, \"class_definition\"), field(n, \"name\", i), text(i, x).\n").unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy(), "env": ["mode=strict"] });
        let reader = "#include \"core.rerail\"\n.decl mode(m: symbol)\nmode(m) :- env(\"mode\", m).\n.decl p1__def(s: symbol, k: symbol)\n.decl r(s: symbol, m: symbol)\nr(s, m) :- p1__def(s, _), mode(m), kind(_, \"module\").\n.output r\n";
        let reader = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(reader).unwrap()).unwrap();
        let report = crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&reader], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "r").unwrap(), [["A", "strict"]]);
        assert_eq!(report.phase_runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["sources", "p1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A constant folded first (`constants::fold`) whose listing a request reads, where another request reads the
    /// files' rows: `stub_dir` lists `stubs` (one value for the run), and `listed` asks for each directory a hit names,
    /// so the requests' cone reads `want`, each file's. The constants' run is over no documents, so its copy of
    /// `stub_dir` is one set of rows for the run (`cross_file`, decision 0652), not a union of the files' rows: it was
    /// none, and gamma's stubs were not found.
    #[test]
    fn a_constant_whose_listing_a_request_reads_is_the_runs_where_another_request_reads_the_files() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("phases-constant-cone");
        let _ = std::fs::remove_dir_all(&dir);
        let (src, phases) = (dir.join("src"), dir.join("phases"));
        std::fs::create_dir_all(src.join("stubs/types-gamma/gamma")).unwrap();
        std::fs::create_dir_all(&phases).unwrap();
        std::fs::write(src.join("main.py"), "import gamma\n").unwrap();
        std::fs::write(src.join("stubs/types-gamma/gamma/__init__.pyi"), "def f() -> None: ...\n").unwrap();
        std::fs::write(phases.join("s.rerail"), r#"#include "core.rerail"
.phase s
.export hit
.decl want(q: symbol)
want(q) :- kind(i, "import_statement"), field(i, "name", n), text(n, q).
.decl stub_dir(b: symbol)
stub_dir(b) :- dir_entry("stubs", w, 1), b = cat("stubs/", w).
.decl hit(q: symbol, l: symbol, b: symbol)
hit(q, "declarations", "") :- want(q).
hit(q, "stubs", b) :- want(q), dir_entry(b, q, 1), stub_dir(b).
.decl listed(d: symbol)
listed(d) :- hit(_, "stubs", d), dir_entry(d, _, _).
"#).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let reader = "#include \"core.rerail\"\n.decl s__hit(q: symbol, l: symbol, b: symbol)\n.decl r(q: symbol, l: symbol, b: symbol)\nr(q, l, b) :- s__hit(q, l, b).\n.output r\n";
        let reader = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(reader).unwrap()).unwrap();
        crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&reader], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "r").unwrap(), [["gamma", "declarations", ""], ["gamma", "stubs", "stubs/types-gamma"]]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plain program whose request reads a listing at a folder its files' rows name: `want` is each file's (its
    /// import), `sd` one folder for the run, `dbg_try`'s stubs row joins them with that folder's listing, and `listed`
    /// asks for the listing of each folder a `dbg_try` row names. The request reads `dbg_try`, so every rule of the
    /// output is the cone of requests' and none the cross part's: the cross part still publishes its rows. It ran
    /// only for rules or facts of its own, and `dbg_try` was written empty.
    #[test]
    fn an_output_whose_rules_are_all_the_cone_of_requests_is_published() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("plain-listing-cone");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("ws");
        std::fs::create_dir_all(src.join("stubs/types-gamma/gamma")).unwrap();
        std::fs::write(src.join("main.py"), "import gamma\n").unwrap();
        let program = r#"#include "core.rerail"
.decl want(q: symbol)
want(q) :- kind(i, "import_statement"), q = "gamma".
.decl sd(b: symbol)
sd("stubs/types-gamma").
.decl dbg_try(q: symbol, l: symbol, b: symbol, t: symbol)
dbg_try(q, "stubs", b, q) :- want(q), dir_entry(b, q, 1), sd(b).
dbg_try(q, "decl", "", q) :- want(q).
.decl listed(d: symbol)
listed(d) :- dbg_try(_, _, d, _), dir_entry(d, _, _).
.output dbg_try
"#;
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        crate::code_rule_dsl::logic::plan::corpus::run(program, &src, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        assert_eq!(
            crate::code_rule_dsl::logic::plan::output::shown(&out, "dbg_try").unwrap(),
            [["gamma", "decl", "", "gamma"], ["gamma", "stubs", "stubs/types-gamma", "gamma"]]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An asked phase (decision 0705), `q`: a file's asks of other modules by key (`asked_at`, "m.x"), each sent to
    /// the file of the module it names (`ask_in`), answered there in the next round (`answer`), and followed through
    /// a star import into another module's file in the round after (`forward`, `resolved`).
    const ASKED_PHASE: &str = r#"
#include "core.rerail"
.phase q
.rounds ask_in
.export asked_at by_path 1, ask_in by_path, answer by_key, resolved by_key
.decl asked_at(p: symbol, k: symbol)
asked_at(p, @cat(m, x)) :- kind(s, "import_from_statement"), field(s, "module_name", mn), text(mn, m), field(s, "name", nn), text(nn, x), docof(s, d), doc(d, p, _).
.decl imp(m: symbol, x: symbol)
imp(m, x) :- kind(s, "import_from_statement"), field(s, "module_name", mn), text(mn, m), field(s, "name", nn), text(nn, x).
.decl asked(m: symbol, x: symbol)
asked(m, x) :- imp(m, x).
asked(m2, y) :- asked(m, x), k = @cat(m, x), forward(k, m2, y).
.decl ask_in(p: symbol, x: symbol)
ask_in(p, x) :- asked(m, x), doc(_, p, m).
.decl q__ask_in(p: symbol, x: symbol)
.decl answer(k: symbol, q: symbol)
answer(@cat(m, x), @cat(m, t)) :- doc(d, p, m), q__ask_in(p, x), kind(f, "function_definition"), docof(f, d), field(f, "name", i), text(i, x), field(f, "parameters", ps), text(ps, t0), t = @cat(x, t0).
.decl forward(k: symbol, m2: symbol, y: symbol)
forward(@cat(m, x), m2, x) :- doc(d, p, m), q__ask_in(p, x), kind(s, "import_from_statement"), docof(s, d), field(s, "module_name", mn), text(mn, m2), child(s, w, _), kind(w, "wildcard_import").
.decl resolved(k: symbol, q: symbol)
resolved(k, q) :- answer(k, q).
resolved(k, q) :- forward(k, m2, y), k2 = @cat(m2, y), resolved(k2, q).
"#;

    /// `q`'s reader: each file's names resolved through its own asks, the asks no round answered (negated: read once
    /// the rounds are over), and what the phase answered and was asked, whole.
    const ASKED_READER: &str = r#"
#include "core.rerail"
.decl q__asked_at(p: symbol, k: symbol)
.decl q__resolved(k: symbol, q: symbol)
.decl q__answer(k: symbol, q: symbol)
.decl q__ask_in(p: symbol, x: symbol)
.decl r(p: symbol, q: symbol)
r(p, q) :- doc(_, p, _), q__asked_at(p, k), q__resolved(k, q).
.decl unresolved(p: symbol, k: symbol)
unresolved(p, k) :- doc(_, p, _), q__asked_at(p, k), !q__resolved(k, _).
.decl answered(k: symbol, q: symbol)
answered(k, q) :- q__answer(k, q).
.decl asks(p: symbol, x: symbol)
asks(p, x) :- q__ask_in(p, x).
.output r, unresolved, answered, asks
"#;

    /// A tree for an asked phase's tests: its files, its phase directory, its store, and a run of a reader there.
    struct AskedTree {
        dir: std::path::PathBuf,
        src: std::path::PathBuf,
        out: std::path::PathBuf,
        options: serde_json::Value,
    }

    impl AskedTree {
        fn new(tag: &str, phases: &[(&str, &str)], files: &[(&str, &str)]) -> AskedTree {
            let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
            let _ = std::fs::remove_dir_all(&dir);
            let (src, phase_dir) = (dir.join("src"), dir.join("phases"));
            std::fs::create_dir_all(&phase_dir).unwrap();
            for (name, text) in phases {
                std::fs::write(phase_dir.join(format!("{name}.rerail")), text).unwrap();
            }
            let t = AskedTree { out: dir.join("out"), options: serde_json::json!({ "phase_dir": phase_dir.to_string_lossy() }), dir, src };
            t.write(files);
            t.store("store.sqlite");
            t
        }

        fn write(&self, files: &[(&str, &str)]) {
            for (path, text) in files {
                let p = self.src.join(path);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, text).unwrap();
            }
        }

        fn store(&self, name: &str) {
            crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(self.dir.join(name)));
        }

        fn run(&self, reader: &str) -> Result<crate::code_rule_dsl::logic::plan::corpus::Report, String> {
            let text = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(reader).unwrap()).unwrap();
            crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &self.src, None, &self.out, "program", Binding::Cpu, 1 << 18, &Default::default(), &self.options).map_err(|e| e.to_string())
        }

        fn rows(&self, rel: &str) -> Vec<Vec<String>> {
            let mut v: Vec<Vec<String>> = crate::code_rule_dsl::logic::plan::output::shown(&self.out, rel).unwrap();
            v.sort();
            v
        }
    }

    impl Drop for AskedTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// `q`'s run in a report: its rounds, each the rows of its asks, and the files it built (not read from its memo).
    fn rounds_of(report: &crate::code_rule_dsl::logic::plan::corpus::Report) -> Vec<(usize, usize)> {
        report.phase_runs.iter().find(|r| r.name == "q").expect("q ran").rounds.iter().map(|r| (r.asks, r.files - r.memo.0)).collect()
    }

    /// The files a run built, not read from its memo (`Report::memo`'s first count is those read from it).
    fn built(report: &crate::code_rule_dsl::logic::plan::corpus::Report) -> usize {
        report.files - report.memo.0
    }

    /// An asked phase answers only what a file asks (decision 0705): `a.py` asks `m.x`, so `m.py` answers `x` and not
    /// `y`, and `k.py`, which no file imports, answers nothing. Round 1 makes the asks, round 2 answers them and
    /// finds no new ask, and in round 2 only `m.py`, whose asks changed, is built again.
    #[test]
    fn an_asked_phase_answers_only_what_is_asked() {
        let t = AskedTree::new("asked-only", &[("q", ASKED_PHASE)], &[("a.py", "from m import x\n"), ("m.py", "def x(a):\n    return a\n\ndef y():\n    return 2\n"), ("k.py", "def z():\n    return 3\n")]);
        let report = t.run(ASKED_READER).unwrap();
        assert_eq!(t.rows("r"), [["a.py", "m.x.(a)"]]);
        assert_eq!(t.rows("answered"), [["m.x", "m.x.(a)"]], "no answer for y or z, which no file asks");
        assert_eq!(t.rows("asks"), [["m.py", "x"]]);
        assert!(t.rows("unresolved").is_empty());
        assert_eq!(rounds_of(&report), [(1, 3), (1, 1)], "round 1 builds every file and makes the ask; round 2 builds m.py alone, answers it, and asks nothing new");
    }

    /// An ask follows a star import into another module's file in the round after (decision 0705): `a.py` asks `x`
    /// of `pkg`, whose `__init__.py` takes every name of `pkg.impl`, which defines it; three rounds. A cycle of star
    /// imports (`c1` of `c2`, `c2` of `c1`) ends with no answer, and the reader's negation sees the closed rounds:
    /// `pkg.x`, answered only in round 3, is not unresolved.
    #[test]
    fn an_asked_phase_follows_a_reexport_in_rounds() {
        let t = AskedTree::new(
            "asked-chain",
            &[("q", ASKED_PHASE)],
            &[
                ("a.py", "from pkg import x\n"),
                ("pkg/__init__.py", "from pkg.impl import *\n"),
                ("pkg/impl.py", "def x(b):\n    return b\n"),
                ("d.py", "from c1 import w\n"),
                ("c1.py", "from c2 import *\n"),
                ("c2.py", "from c1 import *\n"),
            ],
        );
        let report = t.run(ASKED_READER).unwrap();
        assert_eq!(t.rows("r"), [["a.py", "pkg.impl.x.(b)"]]);
        assert_eq!(t.rows("unresolved"), [["d.py", "c1.w"]], "the cycle answers nothing, and only it is unresolved");
        assert_eq!(rounds_of(&report).len(), 3, "the ask, its forward, its answer: {:?}", rounds_of(&report));
    }

    /// An asked phase whose asks grow forever (each round's ask makes a new key of itself) fails past its rounds'
    /// bound, naming the asks the last round added: no partial answer is handed on (decision 0705).
    #[test]
    fn an_asked_phase_past_its_rounds_fails() {
        let grows = r#"
#include "core.rerail"
.phase g
.rounds ask_in
.export ask_in by_path
.decl g__ask_in(p: symbol, x: symbol)
.decl ask_in(p: symbol, x: symbol)
ask_in(p, "x") :- doc(_, p, _).
ask_in(p, y) :- doc(_, p, _), g__ask_in(p, x), y = @cat(x, "x").
"#;
        let t = AskedTree::new("asked-bound", &[("g", grows)], &[("a.py", "v = 1\n")]);
        let err = t.run("#include \"core.rerail\"\n.decl g__ask_in(p: symbol, x: symbol)\n.decl r(p: symbol, x: symbol)\nr(p, x) :- g__ask_in(p, x).\n.output r\n").err().unwrap_or_default();
        assert!(err.contains("after 16 rounds") && err.contains("x.x.x"), "the run fails past 16 rounds, naming what the last added: {err}");
    }

    /// An edit reruns the files its asks reach (decision 0705), each run after the last on the same stores: a body
    /// edit with the same imports is one round, and only its file is built; a new import builds its file in round 1
    /// and the asked file in round 2; a def nobody imports reruns no reader but its own file; a changed answer reruns
    /// its file and its askers alone. Then the rows equal a fresh run's.
    #[test]
    fn an_edit_reruns_the_files_its_asks_reach() {
        let m = "def x(a):\n    return a\n\ndef y():\n    return 2\n\ndef w():\n    return 4\n";
        let t = AskedTree::new("asked-edits", &[("q", ASKED_PHASE)], &[("a.py", "from m import x\n"), ("b.py", "from m import y\n"), ("m.py", m), ("k.py", "def z():\n    return 3\n")]);
        t.run(ASKED_READER).unwrap();
        assert_eq!(t.rows("r"), [["a.py", "m.x.(a)"], ["b.py", "m.y.()"]]);
        // A body edit, the same imports: one round (it starts from the last asks), a.py alone built.
        t.write(&[("a.py", "from m import x\nv = 1\n")]);
        let one = t.run(ASKED_READER).unwrap();
        assert_eq!(rounds_of(&one), [(2, 1)], "one round, a.py built");
        assert_eq!(built(&one), 1, "the reader builds a.py alone");
        // A new import: a.py in round 1, m.py, newly asked, in round 2.
        t.write(&[("a.py", "from m import x\nfrom m import w\nv = 1\n")]);
        let two = t.run(ASKED_READER).unwrap();
        assert_eq!(rounds_of(&two), [(3, 1), (3, 1)], "a.py, then m.py");
        assert_eq!(built(&two), 2, "the reader builds a.py, and m.py, whose own rows of `q__ask_in` (the reader's `asks`) grew; not b.py, whose answers did not change");
        assert_eq!(t.rows("r"), [["a.py", "m.w.()"], ["a.py", "m.x.(a)"], ["b.py", "m.y.()"]]);
        // A def nobody imports: m.py's round alone, and of the readers m.py alone (its own content).
        t.write(&[("m.py", &format!("{m}\ndef u():\n    return 5\n"))]);
        let three = t.run(ASKED_READER).unwrap();
        assert_eq!(rounds_of(&three), [(3, 1)], "m.py, no ask changed");
        assert_eq!(built(&three), 1, "no asker of m.py is built again");
        // A changed answer: m.py, and a.py, which asks m.x; b.py, which asks m.y alone, is not built.
        t.write(&[("m.py", &format!("{}\ndef u():\n    return 5\n", m.replace("def x(a)", "def x(a, c)")))]);
        let four = t.run(ASKED_READER).unwrap();
        assert_eq!(rounds_of(&four), [(3, 1)]);
        assert_eq!(built(&four), 2, "m.py and a.py, and not b.py");
        let after: Vec<Vec<Vec<String>>> = ["r", "unresolved", "answered", "asks"].iter().map(|r| t.rows(r)).collect();
        assert_eq!(after[0], [["a.py", "m.w.()"], ["a.py", "m.x.(a, c)"], ["b.py", "m.y.()"]]);
        t.store("fresh.sqlite");
        t.run(ASKED_READER).unwrap();
        assert_eq!(["r", "unresolved", "answered", "asks"].iter().map(|r| t.rows(r)).collect::<Vec<_>>(), after, "the same rows as a fresh run's");
    }

    /// A reader keyed through two hops (decision 0705, `by_key C`): its asks at its own path (`asked_at by_path 1`),
    /// each ask's answer by the ask (`resolved by_key 1`), the facts about the answer by it (`params by_key`). An edit
    /// to a fact no file's ask resolves to builds no reader but its own file's; one to a fact an ask resolves to,
    /// through a re-export or directly, builds the files asking it alone.
    #[test]
    fn a_reader_is_keyed_by_the_answers_to_its_asks() {
        const PHASE: &str = r#"
#include "core.rerail"
.phase s
.export asked_at by_path 1, resolved by_key 1, params by_key
.decl imp(p: symbol, m: symbol, x: symbol)
imp(p, m, x) :- kind(s, "import_from_statement"), field(s, "module_name", mn), text(mn, m), field(s, "name", nn), text(nn, x), docof(s, d), doc(d, p, _).
.decl asked_at(p: symbol, k: symbol)
asked_at(p, @cat(m, x)) :- imp(p, m, x).
.decl defn(m: symbol, x: symbol, t: symbol)
defn(m, x, t) :- kind(f, "function_definition"), field(f, "name", i), text(i, x), field(f, "parameters", ps), text(ps, t), docof(f, d), doc(d, _, m).
.decl resolved(k: symbol, q: symbol)
resolved(k, q) :- asked_at(_, k), split_dot(k, m, x), imp(p2, m2, x), doc(_, p2, m), q = @cat(m2, x).
resolved(k, k) :- asked_at(_, k), split_dot(k, m, x), defn(m, x, _).
.decl params(q: symbol, t: symbol)
params(@cat(m, x), t) :- defn(m, x, t).
"#;
        const READER: &str = r#"
#include "core.rerail"
.decl s__asked_at(p: symbol, k: symbol)
.decl s__resolved(k: symbol, q: symbol)
.decl s__params(q: symbol, t: symbol)
.decl r(p: symbol, t: symbol)
r(p, t) :- doc(_, p, _), s__asked_at(p, k), s__resolved(k, q), s__params(q, t).
.output r
"#;
        let impl_ = "def x(a):\n    return a\n\ndef z():\n    return 3\n";
        let t = AskedTree::new("keyed-hops", &[("s", PHASE)], &[("a.py", "from m import x\n"), ("b.py", "from m import y\n"), ("m.py", "from impl import x\n\ndef y():\n    return 2\n"), ("impl.py", impl_)]);
        t.run(READER).unwrap();
        assert_eq!(t.rows("r"), [["a.py", "(a)"], ["b.py", "()"], ["m.py", "(a)"]]);
        let report = t.run(READER).unwrap();
        let s = report.phase_runs.iter().find(|r| r.name == "s").unwrap();
        assert!(s.keys.contains(&("s__resolved".to_owned(), crate::code_rule_dsl::logic::plan::phased::Keyed::ByKey(Some(1)))), "{:?}", s.keys);
        // impl.z, which no ask resolves to: impl.py alone (its own content).
        t.write(&[("impl.py", &impl_.replace("def z()", "def z(b)"))]);
        assert_eq!(built(&t.run(READER).unwrap()), 1, "impl.py alone");
        // impl.x, which a.py's m.x resolves to through m's re-export, and m.py's own import: impl.py, a.py and m.py,
        // not b.py, which reads m.py's own y.
        t.write(&[("impl.py", &impl_.replace("def z()", "def z(b)").replace("def x(a)", "def x(a, c)"))]);
        assert_eq!(built(&t.run(READER).unwrap()), 3, "impl.py, a.py and m.py");
        assert_eq!(t.rows("r"), [["a.py", "(a, c)"], ["b.py", "()"], ["m.py", "(a, c)"]]);
    }

    /// An export's key in its readers' memo is the phase's to declare (`.export asked_at by_path 1, resolved
    /// by_key`), its name in neither of `file_local`'s lists (decision 0705): the reader's run reports it.
    #[test]
    fn an_exports_key_is_declared_by_its_phase() {
        use crate::code_rule_dsl::logic::plan::phased::Keyed;
        for name in ["q__asked_at", "q__resolved", "q__answer", "q__ask_in"] {
            assert!(!crate::code_rule_dsl::typing::file_local::setting_input(name), "{name} is a setting only where q's declaration is bound");
        }
        let t = AskedTree::new("asked-keys", &[("q", ASKED_PHASE)], &[("a.py", "from m import x\n"), ("m.py", "def x():\n    return 1\n")]);
        let report = t.run(ASKED_READER).unwrap();
        let q = report.phase_runs.iter().find(|r| r.name == "q").unwrap();
        assert_eq!(
            q.keys,
            [("q__ask_in".to_owned(), Keyed::ByPath(None)), ("q__asked_at".to_owned(), Keyed::ByPath(Some(1))), ("q__answer".to_owned(), Keyed::ByKey(None)), ("q__resolved".to_owned(), Keyed::ByKey(None))]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        let bad = AskedTree::new("asked-keys-bad", &[("q", &ASKED_PHASE.replace("asked_at by_path 1", "asked_at by_path 3"))], &[("a.py", "from m import x\n")]);
        let err = bad.run(ASKED_READER).err().unwrap_or_default();
        assert!(err.contains("column 3"), "a key column the export lacks is refused: {err}");
    }

    /// `ASKED_PHASE` with its answer read through a chain of per-file relations that read no ask (`defn`, `sig`,
    /// `named`), as py_names' `ans_def` reads `def_name` and `qualname`.
    const ASKED_HELD_PHASE: &str = r#"
#include "core.rerail"
.phase q
.rounds ask_in
.export asked_at by_path 1, ask_in by_path, answer by_key, resolved by_key
.decl asked_at(p: symbol, k: symbol)
asked_at(p, @cat(m, x)) :- kind(s, "import_from_statement"), field(s, "module_name", mn), text(mn, m), field(s, "name", nn), text(nn, x), docof(s, d), doc(d, p, _).
.decl asked(m: symbol, x: symbol)
asked(m, x) :- kind(s, "import_from_statement"), field(s, "module_name", mn), text(mn, m), field(s, "name", nn), text(nn, x).
.decl ask_in(p: symbol, x: symbol)
ask_in(p, x) :- asked(m, x), doc(_, p, m).
.decl q__ask_in(p: symbol, x: symbol)
.decl defn(f: node, x: symbol)
defn(f, x) :- kind(f, "function_definition"), field(f, "name", i), text(i, x).
.decl sig(f: node, x: symbol, t: symbol)
sig(f, x, t) :- defn(f, x), field(f, "parameters", ps), text(ps, t0), t = @cat(x, t0).
.decl named(f: node, x: symbol, t: symbol)
named(f, x, t) :- sig(f, x, t), docof(f, _).
.decl answer(k: symbol, q: symbol)
answer(@cat(m, x), @cat(m, t)) :- doc(d, p, m), q__ask_in(p, x), named(f, x, t), docof(f, d).
.decl resolved(k: symbol, q: symbol)
resolved(k, q) :- answer(k, q).
"#;

    /// A later round's partial hits derive the rules that read the asks alone (decision 0705): the relations those
    /// read without reading an ask (`named`, a per-file relation made by three rules) are held in the file's entry
    /// from round 1, so round 2's chunk of `m.py` runs `answer`'s rule and `resolved`'s (a union of `answer`'s rows,
    /// per-file by decision 0680, located through `answer`'s twin), not `named`'s, `sig`'s and `defn`'s. Its
    /// constants are round 1's, not measured again (`phases::RoundsMeasured`).
    #[test]
    fn a_later_round_derives_only_the_rules_that_read_its_asks() {
        let t = AskedTree::new("asked-held", &[("q", ASKED_HELD_PHASE)], &[("a.py", "from m import x\n"), ("m.py", "def x(a):\n    return a\n\ndef y():\n    return 2\n")]);
        let report = t.run(ASKED_READER).unwrap();
        assert_eq!(t.rows("r"), [["a.py", "m.x.(a)"]]);
        assert_eq!(t.rows("answered"), [["m.x", "m.x.(a)"]]);
        let q = report.phase_runs.iter().find(|r| r.name == "q").expect("q ran");
        assert_eq!(q.rounds.iter().map(|r| (r.asks, r.files - r.memo.0)).collect::<Vec<_>>(), [(1, 2), (1, 1)], "round 2 builds m.py alone");
        assert_eq!(q.rounds[0].partial_rules, 0, "round 1 has no partial hit");
        assert_eq!(q.rounds[1].partial_rules, 2, "round 2's partial hit of m.py runs answer's rule and resolved's alone, named read from its entry");
        assert_eq!((q.rounds[0].constants_measured, q.rounds[1].constants_measured), (1, 0), "round 2 reads its chunk's constants as round 1 measured them");
    }

    /// A held relation is stored, never recomputed in its readers (decision 0705). tsx_names' `exported_decl`, which
    /// `ans_def` reads beside the asks, is held; over a 560-file sample the estimates inlined it, so the chunk made no
    /// rows of it, kept no entry of its files, and round 2 built every file again. Told to inline it here
    /// (`RERAIL_INLINE_ONLY`), the chunks store it all the same: round 2 builds `d.ts`, the asked file, alone.
    #[test]
    fn a_held_relation_is_never_inlined() {
        let _inline = crate::code_rule_dsl::logic::plan::corpus::switches::set(&[("RERAIL_INLINE_ONLY", "exported_decl")]);
        let reader = ".language typescript\n#include \"tsx/names.rerail\"\n.decl s(p: symbol, q: symbol)\ns(p, q) :- sym_of(n, q), docof(n, d), doc(d, p, _).\n.output s\n";
        let t = AskedTree::new("asked-held-inline", &[], &[("a.ts", "import { g } from \"./d\";\ng();\n"), ("d.ts", "export function g() {}\nexport function h() {}\n")]);
        let report = t.run(reader).unwrap();
        assert_eq!(t.rows("s"), [["a.ts", "d#g"]]);
        let rounds: Vec<(usize, usize)> = report.phase_runs.iter().find(|r| r.name == "tsx_names").expect("tsx_names ran").rounds.iter().map(|r| (r.asks, r.files - r.memo.0)).collect();
        assert_eq!(rounds, [(1, 2), (1, 1)], "round 2 builds d.ts alone: round 1's entries were kept");
    }

    /// An asked phase's run starts from the asks its last run over the root ended with (decision 0705): over an
    /// unchanged tree it runs one round; when the asks shrink, the smaller set is the run's, and a removed import's
    /// ask is in no export after it.
    #[test]
    fn an_asked_phase_starts_from_its_last_asks() {
        let t = AskedTree::new("asked-kept", &[("q", ASKED_PHASE)], &[("a.py", "from m import x\nfrom m import y\n"), ("m.py", "def x():\n    return 1\n\ndef y():\n    return 2\n")]);
        assert_eq!(rounds_of(&t.run(ASKED_READER).unwrap()).len(), 2);
        assert_eq!(rounds_of(&t.run(ASKED_READER).unwrap()).len(), 1, "unchanged: one round");
        t.write(&[("a.py", "from m import x\n")]);
        let shrunk = t.run(ASKED_READER).unwrap();
        assert_eq!(rounds_of(&shrunk), [(1, 1), (1, 1)], "a.py's asks shrink in round 1, and m.py, which lost one, answers again in round 2");
        assert_eq!(t.rows("asks"), [["m.py", "x"]], "the removed import's ask is in no export");
        assert_eq!(t.rows("answered"), [["m.x", "m.x.()"]]);
    }

    /// The asked `py_names` (decision 0705) resolves the names a file uses through a re-export and a star import, and
    /// the attributes of a class of the run (rule 3), as the eager program it replaced did: the rows below are the
    /// eager program's. It runs in rounds.
    #[test]
    fn the_asked_py_names_resolves_names_through_reexports_and_stars() {
        let files = [
            ("pkg/__init__.py", "from .core import App\nfrom .util import *\n"),
            ("pkg/core.py", "class App:\n    def run(self):\n        return 0\n\n    class Inner:\n        def go(self):\n            return 1\n\ndef unused():\n    return 0\n\nclass Spare:\n    pass\n"),
            ("pkg/util.py", "def helper():\n    return 1\n\ndef other():\n    return 2\n"),
            ("use.py", "from pkg import App, helper\nimport pkg.core\n\nclass Here:\n    def mine(self):\n        return 2\n\nApp()\nApp.run\nApp.Inner.go\nhelper()\npkg.core.App.run\nHere.mine\n"),
        ];
        // r: a name through an import, re-exports followed (rule 1). s: every symbol a node denotes, an attribute of a
        // class of the run through `class` and `defined` (rule 3) among them.
        let reader = ".language python\n#include \"python/names.rerail\"\n.decl r(q0: symbol, q: symbol)\nr(q0, q) :- export_at(_, q0, q).\n.output r\n.decl s(q: symbol)\ns(q) :- sym_of(_, q).\n.output s\n";
        let t = AskedTree::new("asked-py-names", &[], &files);
        let asked = t.run(reader).unwrap();
        let (rows, syms) = (t.rows("r"), t.rows("s"));
        for want in ["pkg.core.App.run", "pkg.core.App.Inner.go", "use.Here.mine"] {
            assert!(syms.iter().any(|r| r[0] == want), "{want} by rule 3, in {syms:?}");
        }
        assert_eq!(rows, [["pkg.App", "pkg.core.App"], ["pkg.core.App", "pkg.core.App"], ["pkg.helper", "pkg.util.helper"]], "the names resolved");
        let want_syms = ["pkg", "pkg.App.Inner", "pkg.App.Inner.go", "pkg.App.run", "pkg.core", "pkg.core.App", "pkg.core.App.Inner", "pkg.core.App.Inner.go", "pkg.core.App.run", "pkg.util.helper", "use.Here", "use.Here.mine"];
        assert_eq!(syms, want_syms.map(|q| vec![q.to_owned()]), "the symbols");
        assert!(!asked.phase_runs.iter().find(|r| r.name == "py_names").unwrap().rounds.is_empty(), "it ran in rounds");
    }

    /// A relative specifier spelled as the file's output names its TypeScript file, as TypeScript's ESM resolution
    /// does: "./c.js" is c.ts, "./m.mjs" m.mts, "./n.cjs" n.cts. (A `.tsx` or `.js` file is a document of its own
    /// dialect's run, not of a `.language typescript` one.)
    #[test]
    fn tsx_names_resolves_a_js_specifier_to_its_typescript_file() {
        let files = [
            ("a.ts", "import { g } from \"./c.js\";\nimport { k } from \"./m.mjs\";\nimport { w } from \"./n.cjs\";\ng();\nk();\nw();\n"),
            ("c.ts", "export function g() {}\n"),
            ("m.mts", "export function k() {}\n"),
            ("n.cts", "export function w() {}\n"),
        ];
        let reader = ".language typescript\n#include \"tsx/names.rerail\"\n.decl s(p: symbol, q: symbol)\ns(p, q) :- sym_of(n, q), docof(n, d), doc(d, p, _).\n.output s\n";
        let t = AskedTree::new("tsx-js-spelled", &[], &files);
        t.run(reader).unwrap();
        assert_eq!(t.rows("s"), [["a.ts", "c#g"], ["a.ts", "m#k"], ["a.ts", "n#w"]]);
    }

    /// The asked `tsx_names` (decision 0705) resolves an import through a re-export (`export { g } from`), a star
    /// (`export * from`) and a namespace import's member, each ask answered by the file of the module that defines it.
    /// An export nobody imports added to `d.ts` builds `d.ts` alone, in the phase and in the reader; an answer that
    /// changes (`b.ts` defines `g` itself) builds `b.ts` and `a.ts`, its asker, not `e.ts`. Then the rows equal a fresh
    /// run's.
    #[test]
    fn the_asked_tsx_names_answers_only_what_is_imported() {
        let files = [
            ("a.ts", "import { g, h } from \"./b\";\nimport * as E from \"./e\";\ng();\nh();\nE.k();\n"),
            ("b.ts", "export { g } from \"./c\";\nexport * from \"./d\";\n"),
            ("c.ts", "export function g() {}\nexport function unused() {}\n"),
            ("d.ts", "export function h() {}\n"),
            ("e.ts", "export function k() {}\n"),
        ];
        let reader = ".language typescript\n#include \"tsx/names.rerail\"\n.decl s(p: symbol, q: symbol)\ns(p, q) :- sym_of(n, q), docof(n, d), doc(d, p, _).\n.output s\n";
        let t = AskedTree::new("asked-tsx-names", &[], &files);
        let cold = t.run(reader).unwrap();
        let rounds = |r: &crate::code_rule_dsl::logic::plan::corpus::Report| -> Vec<(usize, usize)> {
            r.phase_runs.iter().find(|r| r.name == "tsx_names").expect("tsx_names ran").rounds.iter().map(|r| (r.asks, r.files - r.memo.0)).collect()
        };
        assert_eq!(t.rows("s"), [["a.ts", "c#g"], ["a.ts", "d#h"], ["a.ts", "e#k"]], "g through b's re-export, h through its star, E.k a namespace member");
        assert_eq!(rounds(&cold), [(6, 5), (6, 4)], "round 1 makes every ask (b's g and h, c's g, d's g and h, e's k); round 2 builds the four files asked, and asks nothing new");
        t.write(&[("d.ts", "export function h() {}\nexport function spare() {}\n")]);
        let spare = t.run(reader).unwrap();
        assert_eq!(rounds(&spare), [(6, 1)], "d.ts alone, no ask changed");
        assert_eq!(built(&spare), 1, "the reader builds d.ts alone: no asker of d is built again");
        t.write(&[("b.ts", "export function g() {}\nexport * from \"./d\";\n")]);
        let moved = t.run(reader).unwrap();
        assert_eq!(t.rows("s"), [["a.ts", "b#g"], ["a.ts", "d#h"], ["a.ts", "e#k"]]);
        assert_eq!(rounds(&moved), [(5, 1), (5, 1)], "b.ts in round 1; c.ts, whose ask is gone, in round 2");
        assert_eq!(built(&moved), 2, "the reader builds b.ts and a.ts, whose answer changed; not e.ts or c.ts");
        let after = t.rows("s");
        t.store("fresh.sqlite");
        t.run(reader).unwrap();
        assert_eq!(t.rows("s"), after, "the same rows as a fresh run's");
    }
