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

    /// A rule joining an input the run knows is empty goes, with what only it reads (decision 0563 A): `dem` feeds `a`
    /// alone, and `c`'s other rule stays. A negated empty relation keeps its rule; a relation only rules make but none
    /// of whose rules reach an empty one (`e`, read from `f`, which a phase binds) is never taken for empty.
    #[test]
    fn a_rule_joining_an_empty_input_goes_with_what_only_it_reads() {
        let text = ".decl b(x: number)\n.input b\n.decl gp(n: number, s: number)\n.input gp\n.decl f(x: number)\n\
                    .decl dem(x: number)\n.decl a(x: number)\n.decl c(x: number)\n.decl d(x: number)\n.decl e(x: number)\n\
                    .output c\n.output d\n.output e\n\
                    dem(x) :- b(x).\na(x) :- dem(x), gp(x, _).\nc(x) :- a(x).\nc(x) :- b(x), x > 3.\nd(x) :- b(x), !gp(x, _).\ne(x) :- f(x).\n";
        let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
        let mut p = Pipeline::new("test", v);
        let mut unfillable = super::Unfillable::new(["gp".to_owned()], []);
        p.program(&mut unfillable);
        let (out, _) = p.finish().unwrap();
        let heads: Vec<String> = out.rules().map(|r| out.show_rule(r)).collect();
        assert_eq!((unfillable.dropped, unfillable.unread), (2, 1), "a's rule and c's rule over a, then dem's: {heads:?}");
        assert_eq!(unfillable.found, BTreeSet::from(["a".to_owned(), "gp".to_owned()]));
        let of = |h: &str| out.rules().filter(|r| out.rel_name(r.head().rel()) == h).count();
        assert_eq!((of("a"), of("dem"), of("c"), of("d"), of("e")), (0, 0, 1, 1, 1), "{heads:?}");
    }
