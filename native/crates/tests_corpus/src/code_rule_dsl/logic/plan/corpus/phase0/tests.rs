    use std::path::PathBuf;

    /// Each library module's text, included by a program: the files phase 0's graph gives, in its order, are the
    /// builtin reader's (the library first, then beside the includer; `core.rerail` first; includes first).
    #[test]
    fn bootstrap_resolves_as_the_builtin_reader_does() {
        let lib = crate::code_rule_dsl::logic::rr::library();
        let mut names: Vec<String> = Vec::new();
        for dir in ["python", "rust", "data-layer", "complexity", "php"] {
            for e in std::fs::read_dir(lib.join(dir)).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".rerail") {
                    names.push(format!("{dir}/{name}"));
                }
            }
        }
        names.sort();
        names.truncate(40);
        let order = |text: &str| -> Vec<PathBuf> { crate::code_rule_dsl::logic::rr::included_files(text).unwrap() };
        let texts: Vec<String> = names.iter().map(|name| format!("#include \"{name}\"\n.decl x(s: symbol)\n")).collect();
        let pairs = ["#include \"python/env.rerail\"\n#include \"python/imports.rerail\"\n", "#include \"python/env.rerail\"\n.decl doc(d: number, p: symbol, m: symbol)\n"];
        // Every text's graph derived in one run of `bootstrap.rerail`, as a run's program files' are, not one run each
        // (42 at the tests' opt-level were most of the test): a seed's graph is its own whatever run derived it
        // (`a_seeds_graph_is_its_own_whatever_run_derived_it`).
        let seeds: Vec<super::Seed> = texts.iter().map(String::as_str).chain(pairs).filter_map(|t| crate::code_rule_dsl::logic::rr::text_seed(t).unwrap()).collect();
        super::graphs(&seeds).unwrap();
        for text in &texts {
            let graph = order(text);
            let phase_0 = super::in_phase_0();
            let builtin = order(text);
            drop(phase_0);
            assert_eq!(graph, builtin, "{text}");
        }
        // Two names, and a text in T's shape (no `core.rerail`).
        for text in pairs {
            let graph = order(text);
            let phase_0 = super::in_phase_0();
            let builtin = order(text);
            drop(phase_0);
            assert_eq!(graph, builtin, "{text}");
        }
        // A text made from a program file keeps the graph a run of `bootstrap.rerail` would give it (`keep_text`).
        let programs = crate::source_tree::native().join("src/code_rule_dsl/typing/programs");
        for file in [programs.join("calls_cx.rerail"), lib.join("python/conditions.rerail")] {
            let text = crate::code_rule_dsl::logic::settings::program_text(&file).unwrap();
            let seed = crate::code_rule_dsl::logic::rr::text_seed(&text).unwrap().expect("includes");
            let kept = super::graph(&seed).unwrap();
            let derived = super::derive(std::slice::from_ref(&seed)).unwrap().of(&seed, 0);
            assert_eq!(kept.digest(), derived.digest(), "{}:\nkept {}\nderived {}", file.display(), kept.describe(), derived.describe());
        }
        // A name with no file is refused as before.
        let err = crate::code_rule_dsl::logic::rr::parse_version("#include \"no/such.rerail\"\n").expect_err("refused").to_string();
        assert!(err.contains("no such file in the library"), "{err}");
    }

    /// A seed's graph is the same whatever seeds shared its run of `bootstrap.rerail` (`Graph::of`). A module names
    /// `core.rerail` only as a file seed, and `p0_absent` names no includer: taken whole from a run that also had the
    /// library's modules as seeds (the test stores' template's), a text's graph held their implicit includes and
    /// every seed's absent files, and differed from its own run's.
    #[test]
    fn a_seeds_graph_is_its_own_whatever_run_derived_it() {
        let lib = dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).unwrap();
        let text = super::Seed::Text { t_shape: false, includes: vec![(0, "python/env.rerail".to_owned())], phases: Vec::new() };
        let alone = super::derive(std::slice::from_ref(&text)).unwrap().of(&text, 0);
        let seeds = vec![super::Seed::File(lib.join("python/env.rerail")), super::Seed::File(lib.join("python/imports.rerail")), text.clone()];
        let shared = super::derive(&seeds).unwrap();
        let of = shared.of(&text, 2);
        assert_eq!(of.digest(), alone.digest(), "shared:\n{}\nalone:\n{}", of.describe(), alone.describe());
        // Each file seed's graph too.
        for (k, seed) in seeds.iter().enumerate().take(2) {
            let alone = super::derive(std::slice::from_ref(seed)).unwrap().of(seed, 0);
            assert_eq!(shared.of(seed, k).digest(), alone.digest(), "{seed:?}");
        }
    }

    /// A text's or a program file's graph made of its modules' and phase programs' own (`composed`) is the one a run
    /// of `bootstrap.rerail` over it alone derives: its edges, its absent candidates (not those beside a module that a module names
    /// only as a file seed: its own `core.rerail`), and what its phases and its modules' phases reach. A phase that is no
    /// file is an absent candidate.
    #[test]
    fn a_texts_graph_made_of_its_modules_is_the_one_its_run_derives() {
        let seed = |includes: &[&str], phases: &[&str]| super::Seed::Text {
            t_shape: false,
            includes: includes.iter().enumerate().map(|(k, n)| (k as i64, (*n).to_owned())).collect(),
            phases: phases.iter().map(|p| (*p).to_owned()).collect(),
        };
        let seeds = [
            seed(&["core.rerail", "python/conditions.rerail"], &["sources"]),
            seed(&["core.rerail"], &["struct"]),
            seed(&["core.rerail"], &["values"]),
            seed(&["data-layer/boot-lazy-sql.rerail"], &[]),
            seed(&["python/env.rerail"], &["no_such_phase"]),
        ];
        for s in &seeds {
            let composed = super::composed_here(s).unwrap().expect("its names are the library's modules");
            let derived = super::derive(std::slice::from_ref(s)).unwrap().of(s, 0);
            assert!(super::same_graph(&composed, &derived), "{s:?}: {}", composed.differences(&derived));
        }
        // A name that is no module of the library: its run derives it (and reports it).
        assert!(super::composed_here(&seed(&["no/such.rerail"], &[])).unwrap().is_none());
        // A program file of the run's own (a test's, a policy): its includes placed by their first byte, its own
        // `core.rerail` (none in T's shape), each name's candidate beside it absent, and the phases it names.
        let dir = tempfile::tempdir().unwrap();
        let files = [
            ("two.rerail", "\n#include \"python/env.rerail\"\n// between\n#include \"core.rerail\"\n.decl x(s: symbol)\n"),
            ("t.rerail", "#include \"python/env.rerail\"\n.decl doc(d: number, p: symbol, m: symbol)\n"),
            ("phase.rerail", "#include \"core.rerail\"\n.decl values__x(s: symbol)\n.decl y(s: symbol)\ny(s) :- values__x(s).\n"),
            ("none.rerail", ".decl z(s: symbol)\n"),
        ];
        for (name, text) in files {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            let s = super::Seed::File(dunce::canonicalize(&path).unwrap());
            let composed = super::composed_here(&s).unwrap().expect("its names are the library's modules");
            let derived = super::derive(std::slice::from_ref(&s)).unwrap().of(&s, 0);
            assert!(super::same_graph(&composed, &derived), "{name}: {}", composed.differences(&derived));
        }
        // A name whose file stands beside the program file: its run resolves it.
        std::fs::write(dir.path().join("core.rerail"), ".decl w(s: symbol)\n").unwrap();
        let beside = super::Seed::File(dunce::canonicalize(dir.path().join("none.rerail")).unwrap());
        assert!(super::composed_here(&beside).unwrap().is_none());
        // A module of the library is the library's run's to derive (the template's, with every module).
        let lib = dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).unwrap();
        assert!(super::composed_here(&super::Seed::File(lib.join("python/env.rerail"))).unwrap().is_none());
    }

    /// A phase program's graph from a run shared with other file seeds is its own (`Graph::of`): its walk through the
    /// phase programs' includes names no other seed's `core.rerail` (a module that was a seed of the run names it as a
    /// file seed), which held `<the module's dir>/core.rerail` absent in every graph that reached it.
    #[test]
    fn a_phase_programs_graph_is_its_own_whatever_run_derived_it() {
        let lib = dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).unwrap();
        let phase = super::Seed::File(dunce::canonicalize(crate::source_tree::native().join("src/code_rule_dsl/typing/programs/struct.rerail")).unwrap());
        let alone = super::derive(std::slice::from_ref(&phase)).unwrap().of(&phase, 0);
        let mut seeds: Vec<super::Seed> = ["c/env.rerail", "c/preprocess.rerail", "python/env.rerail"].iter().map(|m| super::Seed::File(lib.join(m))).collect();
        seeds.push(phase.clone());
        let shared = super::derive(&seeds).unwrap().of(&phase, seeds.len() - 1);
        assert!(super::same_graph(&shared, &alone), "{}", shared.differences(&alone));
    }

    /// A call's graphs share their look at each file (`Looks`): two kept graphs of one module, the module edited, and
    /// the next call finds both stale and derives both again, each holding the module's new digest; the look one graph
    /// took answers the other.
    #[test]
    fn graphs_sharing_a_module_all_see_its_edit() {
        let dir = std::env::temp_dir().join(format!("rerail-p0-shared-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shared.rerail"), "t(1).\n").unwrap();
        for name in ["a", "b"] {
            std::fs::write(dir.join(format!("{name}.rerail")), format!("#include \"shared.rerail\"\n.decl {name}(x: number)\n{name}(1).\n.output {name}\n")).unwrap();
        }
        let seeds: Vec<super::Seed> = ["a", "b"].iter().map(|n| super::Seed::File(dunce::canonicalize(dir.join(format!("{n}.rerail"))).unwrap())).collect();
        let before: Vec<[u8; 32]> = super::graphs(&seeds).unwrap().iter().map(|g| g.digest()).collect();
        // A new size and content: the observer's stat sees it, whatever the clock's grain.
        std::fs::write(dir.join("shared.rerail"), "t(1).\nt(2).\n").unwrap();
        let after = super::graphs(&seeds).unwrap();
        for (k, g) in after.iter().enumerate() {
            assert_ne!(g.digest(), before[k], "{:?}'s graph still holds the old module:\n{}", seeds[k], g.describe());
            assert!(g.holds_now(), "{}", g.describe());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A rule's include beside it whose directory lies outside phase 0's roots (`rules/../packs/…`, a benchmark's
    /// copy of a pack's tables): the loader refused it, so the graph does not hold it absent, and the graph holds while
    /// the file is there. Held absent, the graph never held, and phase 0 ran again on every run.
    #[test]
    fn a_candidate_outside_the_roots_is_not_held_absent() {
        let dir = std::env::temp_dir().join(format!("rerail-p0-absent-{}", std::process::id()));
        let (rules, packs) = (dir.join("rules"), dir.join("packs"));
        std::fs::create_dir_all(&rules).unwrap();
        std::fs::create_dir_all(&packs).unwrap();
        std::fs::write(packs.join("rerail-p0-absent-table.rerail"), "t(1).\n").unwrap();
        std::fs::write(rules.join("r.rerail"), "#include \"../packs/rerail-p0-absent-table.rerail\"\n.decl x(a: number)\nx(1).\n.output x\n").unwrap();
        let rule = dunce::canonicalize(rules.join("r.rerail")).unwrap();
        let graph = super::graphs(&[super::Seed::File(rule.clone())]).unwrap().remove(0);
        let described = graph.describe();
        assert!(!described.contains("rules/../packs/rerail-p0-absent-table.rerail"), "{described}");
        assert!(graph.holds_now(), "{described}");
        let _ = std::fs::remove_dir_all(&dir);
    }
