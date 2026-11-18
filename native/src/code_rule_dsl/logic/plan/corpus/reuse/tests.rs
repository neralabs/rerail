    use crate::code_rule_dsl::typing::file_local::Binding;

    /// An unchanged run is kept; a document's edit, a new document, the root's settings (read by `env`) and the text
    /// each run it again.
    #[test]
    fn an_unchanged_run_writes_its_kept_outputs() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("reuse");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"), "def f():\n    return 1\n").unwrap();
        let out = dir.join("out");
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let program = |name: &str| {
            let text = format!("#include \"core.rerail\"\n.decl {name}(s: symbol)\n{name}(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output {name}\n");
            crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(&text).unwrap()).unwrap()
        };
        let options = serde_json::json!({ "reuse": true });
        let run = |text: &str| super::super::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let kept = |r: &super::super::Report| r.phases.iter().any(|p| p.0.starts_with("unchanged"));
        let defs = || crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap();
        let text = program("defs");
        assert!(!kept(&run(&text)), "the first run derives");
        assert_eq!(defs(), [["f"]]);
        std::fs::remove_file(out.join(crate::code_rule_dsl::logic::plan::output::FILE)).unwrap();
        let again = run(&text);
        assert!(kept(&again), "unchanged: kept");
        assert_eq!((defs(), again.rows.get("defs").copied()), (vec![vec!["f".to_owned()]], Some(1)), "the kept outputs written again");
        // A document's edit runs it again, and that run is kept in turn.
        std::fs::write(src.join("a.py"), "def g():\n    return 1\n").unwrap();
        assert!(!kept(&run(&text)), "an edited document");
        assert_eq!(defs(), [["g"]]);
        assert!(kept(&run(&text)));
        std::fs::write(src.join("b.py"), "def h():\n    pass\n").unwrap();
        assert!(!kept(&run(&text)), "a new document");
        assert_eq!(defs(), [["g"], ["h"]]);
        std::fs::write(src.join("rerail.env"), "mode=strict\n").unwrap();
        run(&text);
        assert!(kept(&run(&text)), "settings reach only a program that declares `env`");
        let with_env = |name: &str| program(name).replace(".output", ".decl env(k: symbol, v: symbol)\n.output");
        assert!(!kept(&run(&with_env("defs"))));
        assert!(kept(&run(&with_env("defs"))));
        std::fs::write(src.join("rerail.env"), "mode=lax\n").unwrap();
        assert!(!kept(&run(&with_env("defs"))), "the root's settings, for a program that declares `env`");
        assert!(!kept(&run(&program("names"))), "another text");
        // Each library file a text includes is in the record, with its digest (a flattened text includes none).
        let rules = super::rule_files(&super::super::Texts::of(&["#include \"core.rerail\"\n.decl x(s: symbol)\n"]).unwrap(), &serde_json::Value::Null).unwrap();
        assert!(rules.iter().any(|r| r.1.ends_with("core.rerail") && !r.2.is_empty()) && rules.iter().any(|r| r.1.ends_with("components.rerail")), "{rules:?}");
        // A policy file the program reads (`.input … filename=`) is consulted: its edit runs it again.
        std::fs::write(src.join("allow.txt"), "a\n").unwrap();
        let reads = "#include \"core.rerail\"\n.decl entry(n: number, e: symbol)\n.input entry(IO=lines, filename=\"allow.txt\")\n.decl listed(e: symbol)\nlisted(e) :- entry(_, e).\n.output listed\n";
        let reads = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reads).unwrap()).unwrap();
        run(&reads);
        assert!(kept(&run(&reads)));
        std::fs::write(src.join("allow.txt"), "a\nb\n").unwrap();
        assert!(!kept(&run(&reads)), "an edited input file");
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "listed").unwrap(), [["a"], ["b"]]);
        assert!(kept(&run(&reads)));
        // Off: derived every time.
        let off = serde_json::json!({});
        let r = super::super::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &off).unwrap();
        assert!(!kept(&r), "not asked");
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A phase program the record found among the engine's own is checked where it was looked for first, as a file that must
    /// still be missing: one written into the run's `phase_dir` is read instead, so the record's rule files differ
    /// (decision 0275: the check compares the record's include graphs, not the rule files listed again).
    #[test]
    fn a_phase_program_appearing_where_it_was_looked_for_first_differs() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("reuse-phase");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let options = serde_json::json!({ "phase_dir": dir.to_string_lossy() });
        let text = "#include \"core.rerail\"\n.decl values__x(s: symbol)\n";
        let rules = || super::rule_files(&super::super::Texts::of(&[text]).unwrap(), &options).unwrap();
        let shadow = dir.join("values.rerail");
        let entry = |rules: &[(String, String, String)]| rules.iter().find(|r| r.1 == shadow.to_string_lossy()).cloned();
        let before = rules();
        assert_eq!(entry(&before).map(|r| r.2), Some(String::new()), "looked for there first, and missing: {before:?}");
        assert!(before.iter().any(|r| r.1.ends_with("typing/programs/values.rerail") && !r.2.is_empty()), "the engine's own, read: {before:?}");
        std::fs::write(&shadow, "#include \"core.rerail\"\n.decl y(s: symbol)\n").unwrap();
        // The check compares each `rule` line's digest with the file's now: an empty one with a file there differs.
        let now = rules();
        assert!(entry(&now).is_some_and(|r| !r.2.is_empty()), "the file there is read now: {now:?}");
        assert!(!now.iter().any(|r| r.1.ends_with("typing/programs/values.rerail")), "and the engine's own is not: {now:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read off phase 0's graph (no `phase_dir`), a phase program a text names and the engine does not have is listed
    /// where it was looked for, with no digest, as `listed` lists it (decision 0275): one written there differs.
    #[test]
    fn a_phase_program_the_graph_did_not_find_is_listed_as_missing() {
        let text = "#include \"core.rerail\"\n.decl nosuchphase__x(s: symbol)\n";
        let texts = [text];
        let given = super::super::Texts::of(&texts).unwrap();
        assert!(given.graphs.iter().all(Option::is_some), "the text has its graph");
        let rules = super::rule_files(&given, &serde_json::json!({})).unwrap();
        let missing: Vec<_> = rules.iter().filter(|r| r.1.ends_with("typing/programs/nosuchphase.rerail")).collect();
        assert_eq!(missing.len(), 1, "{rules:?}");
        assert!(missing[0].2.is_empty(), "no digest: {rules:?}");
    }

    #[test]
    fn unchanged_cross_file_outputs_skip_derivation_and_changed_members_invalidate() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("cross-reuse");
        let src = dir.join("src"); std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"),"x = 1\n").unwrap();
        std::fs::write(src.join("b.py"),"y = 2\n").unwrap();
        super::super::TEST_TAPE_CACHE.with(|p|*p.borrow_mut()=Some(dir.join("store.sqlite")));
        let p = "#include \"core.rerail\"\n.decl pairs(a:symbol,b:symbol)\npairs(a,b) :- doc(d,a,_),doc(e,b,_),d!=e.\n.output pairs\n";
        let out=dir.join("out");
        let run=||super::super::run_with_options(&[p],&src,None,&out,"program",Binding::Cpu,1<<18,&Default::default(),&serde_json::json!({"reuse":true})).unwrap();
        let first=run();
        let original=crate::code_rule_dsl::logic::plan::output::shown(&out,"pairs").unwrap();
        assert_eq!(first.rows["pairs"],2);
        std::fs::remove_file(out.join(crate::code_rule_dsl::logic::plan::output::FILE)).unwrap();
        let kept=run();
        assert!(kept.phases.iter().any(|p|p.0.starts_with("unchanged")),"{:?}",kept.phases);
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out,"pairs").unwrap(),original);
        assert_eq!(kept.cross,(0,0)); assert_eq!(kept.chunks,0);
        std::fs::write(src.join("c.py"),"z = 3\n").unwrap();
        {
            // An older program's output, written over the stack: an image of it alone, over the one there.
            use crate::code_rule_dsl::logic::plan::output;
            let outputs=output::Outputs::open(&out).unwrap();
            let mut image=output::Builder::new(outputs.images().last().map_or(0,output::Image::flags),true);
            let mut stale=output::Rows::new(vec![output::col::TEXT]);
            stale.push(&[output::Cell::Text("old")]);
            image.put("stale",&[&stale],&[]);
            let bytes=image.finish();
            drop(outputs);
            output::write(&out,&bytes).unwrap();
        }
        assert_eq!(run().rows["pairs"],6);
        let kept=run();
        assert!(!kept.rows.contains_key("stale"),"an older program's output is not part of this run");
        std::fs::remove_file(src.join("b.py")).unwrap();
        assert_eq!(run().rows["pairs"],2);
        super::super::TEST_TAPE_CACHE.with(|p|*p.borrow_mut()=None);
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn outputs_and_records_round_trip() {
        let files = vec![("a.rro".to_owned(), b"1\t2\n".to_vec()), ("b.rro".to_owned(), Vec::new())];
        assert_eq!(super::decode_outputs(&super::encode_outputs(&files)).unwrap(), files);
        let c = vec![("doc".to_owned(), "a.py".to_owned(), "ab".to_owned()), ("rule".to_owned(), "/x/core.rerail".to_owned(), "cd".to_owned())];
        assert_eq!(super::decode_consulted(&super::encode_consulted(&c)), Some(c.clone()));
        let bytes = super::encode_consulted(&c);
        assert_eq!(super::decode_consulted(&bytes[..bytes.len() - 1]), None, "a cut record is none");
    }
