    use std::path::PathBuf;

    /// Each library module's text, included by a program: the files phase 0's graph gives, in its order, are the
    /// builtin reader's (the library first, then beside the includer; `core.rr` first; includes first).
    #[test]
    fn bootstrap_resolves_as_the_builtin_reader_does() {
        let lib = crate::code_rule_dsl::logic::rr::library();
        let mut names: Vec<String> = Vec::new();
        for dir in ["python", "rust", "data-layer", "complexity", "php"] {
            for e in std::fs::read_dir(lib.join(dir)).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".rr") {
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
            super::IN_PHASE_0.with(|c| c.set(true));
            let builtin = order(&text);
            super::IN_PHASE_0.with(|c| c.set(false));
            assert_eq!(graph, builtin, "{name}");
        }
        // Two names, and a text in T's shape (no `core.rr`).
        for text in ["#include \"python/env.rr\"\n#include \"python/imports.rr\"\n", "#include \"python/env.rr\"\n.decl doc(d: number, p: symbol, m: symbol)\n"] {
            let graph = order(text);
            super::IN_PHASE_0.with(|c| c.set(true));
            let builtin = order(text);
            super::IN_PHASE_0.with(|c| c.set(false));
            assert_eq!(graph, builtin, "{text}");
        }
        // A text made from a program file keeps the graph a run of `bootstrap.rr` would give it (`keep_text`).
        let programs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/typing/programs");
        for file in [programs.join("calls_cx.rr"), lib.join("python/conditions.rr")] {
            let text = crate::code_rule_dsl::logic::settings::program_text(&file).unwrap();
            let seed = crate::code_rule_dsl::logic::rr::text_seed(&text).unwrap().expect("includes");
            let kept = super::graph(&seed).unwrap();
            let derived = super::derive(std::slice::from_ref(&seed)).unwrap().of(&seed, 0);
            assert_eq!(kept.digest(), derived.digest(), "{}:\nkept {}\nderived {}", file.display(), kept.describe(), derived.describe());
        }
        // A name with no file is refused as before.
        let err = crate::code_rule_dsl::logic::rr::parse("#include \"no/such.rr\"\n").err().expect("refused").to_string();
        assert!(err.contains("no such file in the library"), "{err}");
    }
