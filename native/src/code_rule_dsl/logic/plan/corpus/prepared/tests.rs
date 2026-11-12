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
        let path = dir.join("calls.rr");
        std::fs::write(&path, &rule).unwrap();
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        let run = || {
            let out = dir.join("out");
            let report = super::super::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
            let mut rows: Vec<String> = std::fs::read_to_string(out.join("calls.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
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

    /// A program written as facts reads back as the same program, and so does the document of those facts, read off
    /// its tape (decision 0074). A pack rule, prepared, both of its programs.
    #[test]
    fn a_prepared_program_reads_back_from_its_facts_and_its_tape() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python/ruff-up006-non-pep585-annotation.rr");
        let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap();
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("prepared-facts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "from typing import List\nx: List[int] = []\n").unwrap();
        super::super::run(&program, &dir, None, &dir.join("out"), "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let prepared = super::super::with_prepared(|kept| kept.values().flatten().find(|p| p.prog.rules.iter().any(|r| r.head.rel.contains("violation") || !r.body.is_empty())).cloned()).expect("a prepared program");
        let facts = super::facts_of(&prepared);
        let dialect = prepared.lowered.dialect;
        let back = super::prepared_of(&facts, dialect).unwrap();
        // Program has no PartialEq: its Debug form is the comparison, field by field.
        assert_eq!(format!("{:?}", back.prog), format!("{:?}", prepared.prog), "the front program");
        assert_eq!(format!("{:?}", back.lowered.program), format!("{:?}", prepared.lowered.program), "the lowered program");
        assert_eq!(back.lowered.domains, prepared.lowered.domains);
        let matches = |p: &super::Prepared| p.vocabulary.matches.iter().map(|re| re.as_str().to_owned()).collect::<Vec<_>>();
        assert_eq!((matches(&back), &back.vocabulary.markers), (matches(&prepared), &prepared.vocabulary.markers), "the match_all regexes and f-string markers");
        assert_eq!((back.sets, back.demanded, back.pushed, back.factorized, back.located, back.merged, back.interned), (prepared.sets.clone(), prepared.demanded.clone(), prepared.pushed.clone(), prepared.factorized.clone(), prepared.located.clone(), prepared.merged, prepared.interned.clone()));
        assert_eq!(back.vocabulary.specs, prepared.vocabulary.specs);
        assert_eq!(format!("{:?}", back.split.local), format!("{:?}", prepared.split.local), "the split's local program");
        assert_eq!(format!("{:?}", back.split.cross), format!("{:?}", prepared.split.cross), "the split's cross-file program");
        assert_eq!((&back.split.exports, &back.split.base, &back.split.locations, &back.split.same_doc), (&prepared.split.exports, &prepared.split.base, &prepared.split.locations, &prepared.split.same_doc));
        // Every prepared program the process holds splits back the same: which one is found above varies with the
        // tests that run beside this one.
        let all: Vec<std::sync::Arc<super::Prepared>> = super::super::with_prepared(|kept| kept.values().flatten().cloned().collect());
        for p in &all {
            let b = super::prepared_of(&super::facts_of(p), p.lowered.dialect).unwrap();
            assert_eq!((&b.split.exports, &b.split.base, &b.split.locations, &b.split.same_doc), (&p.split.exports, &p.split.base, &p.split.locations, &p.split.same_doc));
            assert_eq!(format!("{:?}", b.split.cross), format!("{:?}", p.split.cross));
        }
        // And one stored without its split (an earlier entry's facts) makes it again, the same.
        let mut unsplit = facts.clone();
        unsplit.remove("prepared_split");
        let again = super::prepared_of(&unsplit, dialect).unwrap();
        assert_eq!(format!("{:?}", again.split.cross), format!("{:?}", prepared.split.cross), "the split, made again");
        let document = crate::code_rule_dsl::logic::program_facts::document(&facts, &super::schema()).unwrap();
        let read = crate::code_rule_dsl::logic::program_facts::document_facts(&document).unwrap();
        assert_eq!(read, facts, "the document's tape gives the facts written");
        assert_eq!(crate::code_rule_dsl::logic::program_facts::decode(&crate::code_rule_dsl::logic::program_facts::encode(&facts)).unwrap(), facts);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A program's facts are stored in parts of at most `PART_ROWS` rows, and the parts joined in order are the facts.
    #[test]
    fn a_programs_facts_are_stored_in_bounded_parts_that_join_back() {
        use crate::code_rule_dsl::logic::{front::Value, program_facts::Facts};
        let mut facts = Facts::new();
        facts.insert("a".into(), (0..super::PART_ROWS as i64 + 7).map(|i| vec![Value::Int(i)]).collect());
        facts.insert("b".into(), (0..super::PART_ROWS as i64 * 2).map(|i| vec![Value::Str(format!("s{i}"))]).collect());
        let parts: Vec<Facts> = super::parts(facts.clone()).collect();
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().all(|p| p.values().map(Vec::len).sum::<usize>() <= super::PART_ROWS));
        let mut joined = Facts::new();
        for part in parts {
            for (rel, rows) in part {
                joined.entry(rel).or_default().extend(rows);
            }
        }
        assert_eq!(joined, facts);
    }

    /// A rule about rules reads a program's facts: a document `#include`d, its atoms asked by relation (decision 0074).
    #[test]
    fn a_rule_reads_a_program_as_facts() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rules-as-facts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.py"), "f()\n").unwrap();
        // The rule read as facts, with the core declarations it reads (`kind`, `text`).
        let rule = dir.join("rule.rr");
        std::fs::write(&rule, ".decl calls(c: node, x: symbol)\ncalls(c, x) :- kind(c, \"call\"), text(c, x).\n.decl seen(x: symbol)\nseen(x) :- calls(c, x), !kind(c, \"exec\").\n.output seen\n").unwrap();
        let program = crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap();
        std::fs::remove_file(&rule).unwrap();
        let mut facts = crate::code_rule_dsl::logic::program_facts::Facts::new();
        crate::code_rule_dsl::logic::program_facts::write(&mut facts, "rule", &program);
        let document = crate::code_rule_dsl::logic::program_facts::document(&facts, &Default::default()).unwrap();
        let query = format!(
            "{document}\n.decl negated(rel: symbol)\nnegated(rel) :- body(\"rule\", _, _, l), lit_atom(\"rule\", l, a), atom(\"rule\", a, rel, 1, _).\n.output negated\n"
        );
        // Run as any rule is, over a corpus of one file: the program's facts are the query's own.
        let path = dir.join("query.rr");
        std::fs::write(&path, &query).unwrap();
        let query = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap();
        let out = dir.join("out");
        super::super::run(&query, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let rows = std::fs::read_to_string(out.join("negated.csv")).unwrap();
        assert_eq!(rows.lines().collect::<Vec<_>>(), vec!["kind"], "the one negated relation of the rule");
        let _ = std::fs::remove_dir_all(&dir);
    }
