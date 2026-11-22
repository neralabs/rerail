    use crate::code_rule_dsl::logic::program_image::Builder;
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    use std::collections::BTreeSet;

    /// A request rule is kept where a reader its record names is read, and where the reader is outside the version
    /// (another part's, decision 0345: a part prepared alone does not know whether it is read). A reader of the
    /// version that nothing reads drops it.
    #[test]
    fn a_request_read_by_another_part_keeps_its_ask() {
        let text = ".decl k(x: number)\n.input k\n.decl req(x: number)\n.decl dead(x: number)\n.decl o(x: number)\n.output o\n\
                    req(x) :- k(x).\no(x) :- req(x).\n";
        let pruned = |reader: &str| {
            let read = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
            let mut b = Builder::on(&read);
            b.fields().set_asked(&std::collections::BTreeMap::from([("req".to_owned(), vec![BTreeSet::from([reader.to_owned()])])]));
            let (v, _) = b.finish().unwrap();
            let mut p = Pipeline::new("test", v);
            let mut prune = super::Prune::default();
            p.program(&mut prune);
            let (out, _) = p.finish().unwrap();
            (prune.0, out.rules().any(|r| out.rel_name(r.head().rel()) == "req"))
        };
        assert_eq!(pruned("o"), (0, true), "its reader is read");
        assert_eq!(pruned("elsewhere"), (0, true), "its reader is another part's");
        assert_eq!(pruned("dead"), (1, false), "its reader is the version's, and unread");
    }
