    use crate::code_rule_dsl::typing::file_local::Binding;

    /// Phase 1 (`sources.rr`, decision 0104): a run's documents are those phase 1 loads, one per source file's key.
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
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let text = crate::code_rule_dsl::logic::rr::print(
            &crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n").unwrap(),
        )
        .unwrap();
        let run = || crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::Value::Null).unwrap();
        let defs = || std::fs::read_to_string(out.join("defs.csv")).unwrap();
        let first = run();
        let sources = first.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.exports, vec![("document".to_owned(), 2), ("library".to_owned(), 0)], "each source file is a document of phase 1's, and no library is set");
        assert_eq!(defs(), "f\ng\n");
        std::fs::write(src.join("a.py"), "def h():\n    return 1\n").unwrap();
        let second = run();
        assert_eq!(defs(), "g\nh\n", "the findings follow the edit");
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
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let reader = "#include \"core.rr\"\n.decl sources__library(p: symbol, m: symbol, g: symbol, c: symbol)\n.decl lib(m: symbol)\nlib(m) :- sources__library(_, m, _, _).\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output lib, defs\n";
        let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reader).unwrap()).unwrap();
        let run = || crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &serde_json::Value::Null).unwrap();
        let rows = |rel: &str| {
            let mut v: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect();
            v.sort();
            v
        };
        let first = run();
        assert_eq!(first.phase_runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["sources"]);
        let sources = first.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.exports, vec![("document".to_owned(), 1), ("library".to_owned(), 3)], "m, and builtins, a and b: c is imported by none");
        assert_eq!(rows("lib"), ["a", "b", "builtins"]);
        assert_eq!(rows("defs"), ["fa", "fb", "own"], "the libraries are the reader's documents");
        std::fs::write(lib.join("b.pyi"), "import c\ndef fb2() -> None: ...\n").unwrap();
        let second = run();
        assert_eq!(rows("lib"), ["a", "b", "builtins", "c"], "the edited library's import is followed");
        assert_eq!(rows("defs"), ["fa", "fb2", "fc", "own"]);
        let sources = second.phase_runs.iter().find(|r| r.name == "sources").expect("phase 1 ran");
        assert_eq!(sources.report.memo.0, 3, "phase 1 reads m, builtins and a from the memo, and builds only the edited library and the new one");
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
            std::fs::write(phases.join("p1.rr"), format!(r#"
#include "core.rr"
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
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let run = |text: &str| crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options);
        let print = |text: &str| crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(text).unwrap()).unwrap();
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
        let r = || std::fs::read_to_string(out.join("r.csv")).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\n");
        assert_eq!(first.phase_runs.iter().find(|r| r.name == "p1").expect("p1 ran").exports, [("unit".to_owned(), 3), ("def".to_owned(), 3)]);
        assert_eq!(first.files, 0, "the reader reads only the phase's exports, so no file");
        // Unchanged, a comment, a body: the exports the reader reads are as they were.
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\n");
        write("a.py", "# note\ndef f():\n    x = 1\n    return x\n\n\ndef g():  # g\n    return 2\n");
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\n");
        // A function added, then one removed: the phase's run follows, and so does the reader.
        write("a.py", "def f():\n    return 1\n\ndef g():\n    return 2\n\ndef h():\n    pass\n");
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.g\nm.h\n");
        write("a.py", "def f():\n    return 1\n\ndef h():\n    pass\n");
        run(&text).unwrap();
        assert_eq!(r(), "m\nm.f\nm.h\n");
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
#include "core.rr"
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
        std::fs::write(phases.join("p1.rr"), p1).unwrap();
        std::fs::write(phases.join("p2.rr"), p2).unwrap();
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy() });
        let run = |text: &str| crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options);
        let reader = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse("#include \"core.rr\"\n.decl p2__anc(s: symbol, a: symbol)\n.decl r(s: symbol, a: symbol)\nr(s, a) :- p2__anc(s, a).\n.output r\n").unwrap()).unwrap();
        let report = run(&reader).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "B\tA\n");
        // Phase 1 (`sources`) first, which gives every run its documents, then both phases, listed flat, in the order
        // they ran; p2's own run checked p1 once, not again.
        let names: Vec<&str> = report.phase_runs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["sources", "p1", "p2"]);
        assert!(report.phase_runs[2].report.phase_runs.is_empty(), "p1 is not checked again inside p2's run");
        // Nothing changed: the same rows.
        run(&reader).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "B\tA\n");
        // p1 changed (its program: no class has a base now): p2, which reads it, is not kept.
        std::fs::write(phases.join("p1.rr"), p1.replace("text(e, b).", "text(e, b), b = \"none\".")).unwrap();
        run(&reader).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "", "p2 run again over p1's new exports");
        // A reader of p2 that loads documents itself is refused: p1 loaded none, so first give p1 a load.
        std::fs::write(phases.join("p1.rr"), format!("{p1}\nload(\"x.pyi\", \"x\", \"python\") :- def(\"A\", _).\n")).unwrap();
        let loading = reader.replace(".output r", "load(\"y.pyi\", \"y\", \"python\") :- r(_, _).\n.output r");
        let err = run(&loading).err().expect("a later phase's reader that loads").to_string();
        assert!(err.contains("loads documents itself"), "{err}");
        // p1 reading p2's export: each runs the other first.
        std::fs::write(phases.join("p1.rr"), format!("{p1}\n.decl p2__anc(s: symbol, a: symbol)\n.decl seen(s: symbol)\nseen(s) :- p2__anc(s, _).\n.output seen\n")).unwrap();
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
        std::fs::write(phases.join("p1.rr"), "#include \"core.rr\"\n.phase p1\n.export def\n.decl def(s: symbol, k: symbol)\ndef(x, \"class\") :- kind(n, \"class_definition\"), field(n, \"name\", i), text(i, x).\n").unwrap();
        let out = dir.join("out");
        let _no_session = crate::code_rule_dsl::logic::plan::corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        crate::code_rule_dsl::logic::plan::corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let options = serde_json::json!({ "phase_dir": phases.to_string_lossy(), "env": ["mode=strict"] });
        let reader = "#include \"core.rr\"\n.decl mode(m: symbol)\nmode(m) :- env(\"mode\", m).\n.decl p1__def(s: symbol, k: symbol)\n.decl r(s: symbol, m: symbol)\nr(s, m) :- p1__def(s, _), mode(m), kind(_, \"module\").\n.output r\n";
        let reader = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reader).unwrap()).unwrap();
        let report = crate::code_rule_dsl::logic::plan::corpus::run_with_options(&[&reader], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("r.csv")).unwrap(), "A\tstrict\n");
        assert_eq!(report.phase_runs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["sources", "p1"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
