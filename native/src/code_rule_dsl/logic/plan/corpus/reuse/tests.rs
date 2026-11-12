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
        let _no_session = super::super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let program = |name: &str| {
            let text = format!("#include \"core.rr\"\n.decl {name}(s: symbol)\n{name}(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output {name}\n");
            crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(&text).unwrap()).unwrap()
        };
        let options = serde_json::json!({ "reuse": true });
        let run = |text: &str| super::super::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let kept = |r: &super::super::Report| r.phases.iter().any(|p| p.0.starts_with("unchanged"));
        let defs = || std::fs::read_to_string(out.join("defs.csv")).unwrap();
        let text = program("defs");
        assert!(!kept(&run(&text)), "the first run derives");
        assert_eq!(defs(), "f\n");
        std::fs::remove_file(out.join("defs.csv")).unwrap();
        let again = run(&text);
        assert!(kept(&again), "unchanged: kept");
        assert_eq!((defs().as_str(), again.rows.get("defs").copied()), ("f\n", Some(1)), "the kept outputs written again");
        // A document's edit runs it again, and that run is kept in turn.
        std::fs::write(src.join("a.py"), "def g():\n    return 1\n").unwrap();
        assert!(!kept(&run(&text)), "an edited document");
        assert_eq!(defs(), "g\n");
        assert!(kept(&run(&text)));
        std::fs::write(src.join("b.py"), "def h():\n    pass\n").unwrap();
        assert!(!kept(&run(&text)), "a new document");
        assert_eq!(defs(), "g\nh\n");
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
        let rules = super::rule_files(&["#include \"core.rr\"\n.decl x(s: symbol)\n".to_owned()]).unwrap();
        assert!(rules.iter().any(|r| r.1.ends_with("core.rr") && !r.2.is_empty()) && rules.iter().any(|r| r.1.ends_with("components.rr")), "{rules:?}");
        // A policy file the program reads (`.input … filename=`) is consulted: its edit runs it again.
        std::fs::write(src.join("allow.txt"), "a\n").unwrap();
        let reads = "#include \"core.rr\"\n.decl entry(n: number, e: symbol)\n.input entry(IO=lines, filename=\"allow.txt\")\n.decl listed(e: symbol)\nlisted(e) :- entry(_, e).\n.output listed\n";
        let reads = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reads).unwrap()).unwrap();
        run(&reads);
        assert!(kept(&run(&reads)));
        std::fs::write(src.join("allow.txt"), "a\nb\n").unwrap();
        assert!(!kept(&run(&reads)), "an edited input file");
        assert_eq!(std::fs::read_to_string(out.join("listed.csv")).unwrap(), "a\nb\n");
        assert!(kept(&run(&reads)));
        // Off: derived every time.
        let off = serde_json::json!({});
        let r = super::super::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &off).unwrap();
        assert!(!kept(&r), "not asked");
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unchanged_cross_file_outputs_skip_derivation_and_changed_members_invalidate() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("cross-reuse");
        let src = dir.join("src"); std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"),"x = 1\n").unwrap();
        std::fs::write(src.join("b.py"),"y = 2\n").unwrap();
        let _guard = super::super::SESSION_TESTS.lock().unwrap_or_else(|e|e.into_inner());
        super::super::TEST_TAPE_CACHE.with(|p|*p.borrow_mut()=Some(dir.join("store.sqlite")));
        let p = "#include \"core.rr\"\n.decl pairs(a:symbol,b:symbol)\npairs(a,b) :- doc(d,a,_),doc(e,b,_),d!=e.\n.output pairs\n";
        let out=dir.join("out");
        let run=||super::super::run_with_options(&[p],&src,None,&out,"program",Binding::Cpu,1<<18,&Default::default(),&serde_json::json!({"reuse":true})).unwrap();
        let first=run();
        let original=std::fs::read(out.join("pairs.csv")).unwrap();
        assert_eq!(first.rows["pairs"],2);
        std::fs::remove_file(out.join("pairs.csv")).unwrap();
        let kept=run();
        assert!(kept.phases.iter().any(|p|p.0.starts_with("unchanged")),"{:?}",kept.phases);
        assert_eq!(std::fs::read(out.join("pairs.csv")).unwrap(),original);
        assert_eq!(kept.cross,(0,0)); assert_eq!(kept.chunks,0);
        std::fs::write(src.join("c.py"),"z = 3\n").unwrap();
        std::fs::write(out.join("stale.csv"),"old\n").unwrap();
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
        let files = vec![("a.csv".to_owned(), b"1\t2\n".to_vec()), ("b.csv".to_owned(), Vec::new())];
        assert_eq!(super::decode_outputs(&super::encode_outputs(&files)).unwrap(), files);
        let c = vec![("doc".to_owned(), "a.py".to_owned(), "ab".to_owned()), ("rule".to_owned(), "/x/core.rr".to_owned(), "cd".to_owned())];
        assert_eq!(super::decode_consulted(&super::encode_consulted(&c)), c);
    }
