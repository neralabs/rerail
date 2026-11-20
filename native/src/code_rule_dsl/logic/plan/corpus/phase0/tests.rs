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
        for name in &names {
            let text = format!("#include \"{name}\"\n.decl x(s: symbol)\n");
            let graph = order(&text);
            let phase_0 = super::in_phase_0();
            let builtin = order(&text);
            drop(phase_0);
            assert_eq!(graph, builtin, "{name}");
        }
        // Two names, and a text in T's shape (no `core.rerail`).
        for text in ["#include \"python/env.rerail\"\n#include \"python/imports.rerail\"\n", "#include \"python/env.rerail\"\n.decl doc(d: number, p: symbol, m: symbol)\n"] {
            let graph = order(text);
            let phase_0 = super::in_phase_0();
            let builtin = order(text);
            drop(phase_0);
            assert_eq!(graph, builtin, "{text}");
        }
        // A text made from a program file keeps the graph a run of `bootstrap.rerail` would give it (`keep_text`).
        let programs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/typing/programs");
        for file in [programs.join("calls_cx.rerail"), lib.join("python/conditions.rerail")] {
            let text = crate::code_rule_dsl::logic::settings::program_text(&file).unwrap();
            let seed = crate::code_rule_dsl::logic::rr::text_seed(&text).unwrap().expect("includes");
            let kept = super::graph(&seed).unwrap();
            let derived = super::derive(std::slice::from_ref(&seed)).unwrap().of(&seed, 0);
            assert_eq!(kept.digest(), derived.digest(), "{}:\nkept {}\nderived {}", file.display(), kept.describe(), derived.describe());
        }
        // A name with no file is refused as before.
        let err = crate::code_rule_dsl::logic::rr::parse("#include \"no/such.rerail\"\n").err().expect("refused").to_string();
        assert!(err.contains("no such file in the library"), "{err}");
    }

    /// A seed's graph is the same whatever seeds shared its run of `bootstrap.rerail` (`Graph::of`). A module names
    /// `core.rerail` only as a file seed, and `p0_absent` names no includer: taken whole from a run that also had the
    /// library's modules as seeds (the test stores' template's), a text's graph held their implicit includes and
    /// every seed's absent files, and differed from its own run's.
    #[test]
    fn a_seeds_graph_is_its_own_whatever_run_derived_it() {
        let lib = std::fs::canonicalize(crate::code_rule_dsl::logic::rr::library()).unwrap();
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
        let rule = std::fs::canonicalize(rules.join("r.rerail")).unwrap();
        let graph = super::graphs(&[super::Seed::File(rule.clone())]).unwrap().remove(0);
        let described = graph.describe();
        assert!(!described.contains("rules/../packs/rerail-p0-absent-table.rerail"), "{described}");
        assert!(graph.holds_now(), "{described}");
        let _ = std::fs::remove_dir_all(&dir);
    }
