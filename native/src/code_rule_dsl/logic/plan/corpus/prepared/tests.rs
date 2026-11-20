    /// A program one process prepared is read from the store by the next (decision 0073). With this process's
    /// prepared programs dropped, as a new process starts without them, the second run reads it from the store and
    /// gives the first run's rows. A third run finds it in memory.
    #[test]
    fn a_prepared_program_is_read_from_the_store() {
        // A text no other run prepared, so the first run prepares it: the marker is a constant of the rule, since
        // printing the rule drops its comments.
        let marker = format!("{:?}", std::time::SystemTime::now());
        let rule = format!(
            ".decl calls(c: node, x: symbol)\ncalls(c, x) :- kind(c, \"call\"), text(c, x), x != \"g()\", x != {marker:?}.\n.output calls\n"
        );
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("prepared");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "f(1)\ng()\nh(f(2))\n").unwrap();
        // The rule with the core declarations it reads (`kind`, `text`), as `rerail-engine` reads a rule file.
        let path = dir.join("calls.rerail");
        std::fs::write(&path, &rule).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        let run = || {
            let out = dir.join("out");
            let report = super::super::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            let mut rows: Vec<Vec<String>> = crate::code_rule_dsl::logic::plan::output::shown(&out, "calls").unwrap_or_default();
            rows.sort();
            (report.prepared, rows)
        };
        let (from, first) = run();
        assert_eq!(from, "prepared");
        assert_eq!(first.len(), 3, "{first:?}");
        super::super::with_prepared(|kept| kept.clear());
        let (from, second) = run();
        assert_eq!(from, "store", "the program is read from the store");
        assert_eq!(second, first);
        let (from, third) = run();
        // From memory, unless a test running beside this one filled the process's cache past `PREPARED_KEPT` and it
        // was cleared: then from the store again.
        assert!(from == "memory" || from == "store", "{from}");
        assert_eq!(third, first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A prepared program another store made (the engine's bundle, decision 0264) is read with its name atoms moved to
    /// this store's (decision 0270): its version's tree decoded once, the names moved, imaged again; the pointer's
    /// atom rows name this store's atoms, and read so, the program is this store's (nothing to move).
    #[test]
    fn a_prepared_program_of_another_store_moves_to_this_stores_atoms() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python/ruff-up006-non-pep585-annotation.rerail");
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap();
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("prepared-moved");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "from typing import List\nx: List[int] = []\n").unwrap();
        super::super::run(&program, &dir, None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let prepared = super::super::with_prepared(|kept| kept.values().flatten().find(|p| p.source.rules().any(|r| p.source.rel_name(r.head().rel()).contains("violation"))).cloned()).expect("a prepared program");
        let mut facts = super::facts_of(&prepared);
        // Another store, its atoms numbered past strings this one interned first: the program's names are not its.
        let store = crate::content_store::ContentStore::open(&dir.join("other.sqlite")).unwrap();
        let fillers: Vec<String> = (0..64).map(|i| format!("a string interned first {i}")).collect();
        store.intern_atoms(&fillers.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
        let moved = super::relocate(&store, &mut facts, &prepared.lowered.version).expect("every name has an atom here").expect("the names moved");
        assert_ne!(moved.digest(), prepared.lowered.version.digest(), "the moved image names other atoms");
        // The pointer's rows name this store's atoms, so the program read again is the store's.
        let strings: Vec<String> = facts["prepared_interned_atom"].iter().map(|r| match &r[2] { crate::code_rule_dsl::logic::front::Value::Str(s) => s.to_string(), other => panic!("{other:?}") }).collect();
        let here: Vec<i64> = store.intern_atoms(&strings.iter().map(String::as_str).collect::<Vec<_>>()).unwrap().into_iter().map(i64::from).collect();
        let kept: Vec<i64> = facts["prepared_interned_atom"].iter().map(|r| r[3].integer().unwrap()).collect();
        assert_eq!(kept, here);
        assert!(super::relocate(&store, &mut facts, &moved).expect("read").is_none(), "already this store's");
        super::prepared_of(&facts, std::sync::Arc::clone(&prepared.source), moved, prepared.lowered.dialect).expect("the moved program reads as prepared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A prepared program reads back from what is kept of it: its two programs from their images, the rest from its
    /// facts (decision 0155). A pack rule, prepared.
    #[test]
    fn a_prepared_program_reads_back_from_its_text_and_facts() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python/ruff-up006-non-pep585-annotation.rerail");
        let program = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap();
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("prepared-facts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "from typing import List\nx: List[int] = []\n").unwrap();
        super::super::run(&program, &dir, None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let prepared = super::super::with_prepared(|kept| kept.values().flatten().find(|p| p.source.rules().any(|r| p.source.rel_name(r.head().rel()).contains("violation") || r.body().next().is_some())).cloned()).expect("a prepared program");
        let facts = super::facts_of(&prepared);
        let dialect = prepared.lowered.dialect;
        // The lowered program's version, read back from its image's bytes, as the loader opens it.
        let lowered_image = |p: &super::Prepared| crate::code_rule_dsl::logic::program_image::Version::open(p.lowered.version.base_bytes().expect("a prepared program's version is a base")).unwrap();
        // The source's version, read back the same way.
        let source_image = |p: &super::Prepared| crate::code_rule_dsl::logic::program_image::Version::open(p.source.base_bytes().expect("the source is a base")).unwrap();
        let digest = |l: &crate::code_rule_dsl::logic::plan::lower::Lowered| l.placed().unwrap().digest();
        let back = super::prepared_of(&facts, source_image(&prepared), lowered_image(&prepared), dialect).unwrap();
        // Program has no PartialEq: its Debug form is the comparison, field by field; versions compare by digest.
        assert_eq!((back.source.size(), back.source.digest()), (prepared.source.size(), prepared.source.digest()), "the source");
        assert_eq!(digest(&back.lowered), digest(&prepared.lowered), "the lowered program");
        assert_eq!(back.lowered.domains, prepared.lowered.domains);
        let matches = |p: &super::Prepared| p.vocabulary.matches.iter().map(|re| re.as_str().to_owned()).collect::<Vec<_>>();
        assert_eq!((matches(&back), &back.vocabulary.markers), (matches(&prepared), &prepared.vocabulary.markers), "the match_all regexes and f-string markers");
        assert_eq!((back.sets, back.demanded, back.pushed, back.factorized, back.located, back.merged, back.interned), (prepared.sets.clone(), prepared.demanded.clone(), prepared.pushed.clone(), prepared.factorized.clone(), prepared.located.clone(), prepared.merged, prepared.interned.clone()));
        assert_eq!(back.vocabulary.specs, prepared.vocabulary.specs);
        // What preparing decided (decision 0269), made though this process does not profile, and read back.
        assert!(!prepared.decided.is_empty(), "preparing decided nothing");
        assert_eq!(back.decided, prepared.decided);
        assert_eq!(back.split.versions.local.digest(), prepared.split.versions.local.digest(), "the split's local program");
        assert_eq!(back.split.versions.cross.digest(), prepared.split.versions.cross.digest(), "the split's cross-file program");
        assert_eq!((&back.split.exports, &back.split.base, &back.split.locations, &back.split.same_doc), (&prepared.split.exports, &prepared.split.base, &prepared.split.locations, &prepared.split.same_doc));
        // Every prepared program the process holds splits back the same: which one is found above varies with the
        // tests that run beside this one.
        let all: Vec<std::sync::Arc<super::Prepared>> = super::super::with_prepared(|kept| kept.values().flatten().cloned().collect());
        for p in &all {
            let b = super::prepared_of(&super::facts_of(p), source_image(p), lowered_image(p), p.lowered.dialect).unwrap();
            assert_eq!((&b.split.exports, &b.split.base, &b.split.locations, &b.split.same_doc), (&p.split.exports, &p.split.base, &p.split.locations, &p.split.same_doc));
            assert_eq!(b.split.versions.cross.digest(), p.split.versions.cross.digest());
        }
        // And one stored without its split (an earlier entry's facts) makes it again, the same.
        let mut unsplit = facts.clone();
        unsplit.remove("prepared_split");
        let again = super::prepared_of(&unsplit, source_image(&prepared), lowered_image(&prepared), dialect).unwrap();
        assert_eq!(again.split.versions.cross.digest(), prepared.split.versions.cross.digest(), "the split, made again");
        // No program is in the facts: no row per node (decision 0155).
        assert!(["program", "rule", "atom", "term"].iter().all(|r| !facts.contains_key(*r)), "{:?}", facts.keys().collect::<Vec<_>>());
        assert_eq!(crate::code_rule_dsl::logic::program_facts::decode(&crate::code_rule_dsl::logic::program_facts::encode(&facts)).unwrap(), facts);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A rule about rules reads a program's facts: a document `#include`d, its atoms asked by relation (decision 0074).
    #[test]
    fn a_rule_reads_a_program_as_facts() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rules-as-facts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "f()\n").unwrap();
        // The rule read as facts, with the core declarations it reads (`kind`, `text`).
        let rule = dir.join("rule.rerail");
        std::fs::write(&rule, ".decl calls(c: node, x: symbol)\ncalls(c, x) :- kind(c, \"call\"), text(c, x).\n.decl seen(x: symbol)\nseen(x) :- calls(c, x), !kind(c, \"exec\").\n.output seen\n").unwrap();
        let program = crate::code_rule_dsl::logic::rr::parse_file_version(&rule).unwrap();
        std::fs::remove_file(&rule).unwrap();
        let mut facts = crate::code_rule_dsl::logic::program_facts::Facts::new();
        crate::code_rule_dsl::logic::program_facts::write_version(&mut facts, "rule", &program);
        let document = crate::code_rule_dsl::logic::program_text::document(&facts, &[]).unwrap();
        let query = format!(
            "{document}\n.decl negated(rel: symbol)\nnegated(rel) :- body(\"rule\", _, _, l), lit_atom(\"rule\", l, a), atom(\"rule\", a, rel, 1, _).\n.output negated\n"
        );
        // Run as any rule is, over a corpus of one file: the program's facts are the query's own.
        let path = dir.join("query.rerail");
        std::fs::write(&path, &query).unwrap();
        let query = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_file_version(&path).unwrap()).unwrap();
        let out = dir.join("out");
        super::super::run(&query, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows = crate::code_rule_dsl::logic::plan::output::shown(&out, "negated").unwrap();
        assert_eq!(rows, [["kind"]], "the one negated relation of the rule");
        let _ = std::fs::remove_dir_all(&dir);
    }
