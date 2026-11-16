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
        let err = crate::code_rule_dsl::logic::rr::parse("#include \"no/such.rr\"\n").expect_err("refused").to_string();
        assert!(err.contains("no such file in the library"), "{err}");
    }

    /// An include's file is named as its module is loaded (`module_path`), so the module's own includes are found
    /// under it: Windows' canonical paths are `\`-separated where `bootstrap.rr` writes `cat(lib, "/", name)`, and
    /// an included module's includes were lost (`python/modules.rr`'s `python/env.rr`: "py_stdlib … not declared").
    #[test]
    fn an_include_is_named_as_its_module_is_loaded() {
        // A Windows candidate, as `cat` writes it, against the canonical path a load names it by.
        assert_eq!(super::in_separators(r"D:\a\rerail\docs\rule-syntax\lib/python/env.rr", '\\'), r"D:\a\rerail\docs\rule-syntax\lib\python\env.rr");
        assert_eq!(super::in_separators("/srv/lib/python/env.rr", '/'), "/srv/lib/python/env.rr");
        let lib = dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).unwrap();
        let candidate = format!("{}/python/env.rr", lib.display());
        assert_eq!(super::module_path(&candidate), dunce::canonicalize(lib.join("python").join("env.rr")).unwrap());
        // Every module an include reaches that includes files itself is an includer under the same name.
        let seed = crate::code_rule_dsl::logic::rr::text_seed("#include \"python/modules.rr\"\n.decl x(s: symbol)\n").unwrap().expect("includes");
        let graph = super::derive(std::slice::from_ref(&seed)).unwrap().of(&seed, 0);
        let env = dunce::canonicalize(lib.join("python").join("env.rr")).unwrap();
        assert!(graph.has_module(&env), "python/modules.rr's include of python/env.rr is in its graph:\n{}", graph.describe());
        let modules = dunce::canonicalize(lib.join("python").join("modules.rr")).unwrap();
        assert!(graph.includes_of(&modules.to_string_lossy()).contains(&env), "{}", graph.describe());
    }
