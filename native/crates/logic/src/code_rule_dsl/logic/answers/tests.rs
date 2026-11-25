    use rerail_blocks::vector::Vector;
    use super::*;

    /// `!seen(x)` inside a component that loads waits for its key's gate; the rule asking about it is its prefix.
    #[test]
    fn a_negation_inside_a_loading_component_is_gated_per_key() {
        let prog = super::super::rr::parse_version(
            ".decl want(x: symbol)\n.decl req(x: symbol)\n.decl got(x: symbol, y: symbol)\n.decl seen(x: symbol)\n.decl fresh(x: symbol)\n\
             want(\"a\").\nwant(y) :- got(_, y).\nseen(y) :- got(_, y).\nfresh(x) :- want(x), !seen(x).\nreq(x) :- fresh(x).\n",
        )
        .unwrap();
        let answers: BTreeMap<String, Vec<String>> = [("req".to_owned(), vec!["got".to_owned()])].into();
        let mut ungated = Builder::on(&prog);
        ungated.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Answers(&answers));
        let (ungated, _) = ungated.finish().unwrap();
        assert!(Placed::whole(ungated).components().is_err(), "ungated, the negation is inside the component");
        let gated = gate(&Placed::whole(prog.clone()), &answers).unwrap();
        let sccs = Placed::whole(gated.clone()).components().unwrap();
        let loading = sccs.iter().find(|s| s.contains("req")).unwrap();
        for rel in ["got", "want", "fresh", "seen", "ask_seen_b", "done_seen_b"] {
            assert!(loading.contains(rel), "{rel} in {loading:?}");
        }
        let gates = crate::code_rule_dsl::logic::program_image::fields::ProgramFields::gates(&*gated);
        let (done, g) = &gates[0];
        assert_eq!((done.as_str(), g.src.as_deref(), g.rel.as_str()), ("done_seen_b", Some("ask_seen_b"), "seen"));
        assert!(g.quiet.contains(&"req".to_owned()), "a request unanswered keeps the gate shut: {:?}", g.quiet);
        let shown = super::super::rr::parse_version(".decl fresh(x: symbol)\n.decl shown(x: symbol)\nshown(x) :- fresh(x).\n").unwrap();
        let mut b = Builder::on(&gated);
        b.copy(&shown, shown.rules().next().unwrap());
        b.fields().declare("shown", &[("x", "sym")]);
        let (with_shown, _) = b.finish().unwrap();
        let parts = split(&Placed::whole(with_shown.clone())).unwrap();
        let (load, rest) = parts.parts(&with_shown).unwrap();
        assert!(load.rules().all(|r| load.rel_name(r.head().rel()) != "shown") && rest.rules().all(|r| rest.rel_name(r.head().rel()) == "shown"));
        assert_eq!(parts.reads.iter().collect::<Vec<_>>(), ["fresh"]);
        let rest = Placed::whole(rest);
        let f = Fields::of(&rest).unwrap();
        assert!(f.is_input("fresh") && f.gates().is_empty() && f.answers().is_empty());
    }

    /// An aggregate inside a component that loads waits for its group's gate, as a negation does (Q409: `plain_str`'s
    /// `join` over a parsed string's nodes): its key is the columns the literals before it bind, and an earlier
    /// aggregate of the component in the same rule is written gated into the later one's `ask`.
    #[test]
    fn an_aggregate_inside_a_loading_component_is_gated_per_group() {
        let prog = super::super::rr::parse_version(
            ".decl want(x: symbol)\n.decl req(x: symbol)\n.decl got(x: symbol, y: symbol)\n.decl few(x: symbol)\n\
             want(\"a\").\nwant(y) :- got(_, y).\n\
             few(x) :- want(x), m = count : { got(x, _) }, m < 3, n = count : { got(x, y), want(y) }, n < 2.\n\
             req(x) :- few(x).\n",
        )
        .unwrap();
        let answers: BTreeMap<String, Vec<String>> = [("req".to_owned(), vec!["got".to_owned()])].into();
        let mut ungated = Builder::on(&prog);
        ungated.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Answers(&answers));
        let (ungated, _) = ungated.finish().unwrap();
        assert!(Placed::whole(ungated).components().is_err(), "ungated, the aggregates are inside the component");
        let gated = gate(&Placed::whole(prog.clone()), &answers).unwrap();
        let sccs = Placed::whole(gated.clone()).components().unwrap();
        let loading = sccs.iter().find(|s| s.contains("req")).unwrap();
        assert!(["got", "want", "few"].iter().all(|r| loading.contains(r)), "{loading:?}");
        let gates = crate::code_rule_dsl::logic::program_image::fields::ProgramFields::gates(&*gated);
        assert_eq!(gates.len(), 2, "a gate for each aggregate: {:?}", gates.iter().map(|(d, _)| d).collect::<Vec<_>>());
        for (done, g) in &gates {
            assert!(done.starts_with("done_") && g.src.as_deref().is_some_and(|a| a.starts_with("ask_")), "keyed by its group: {done}");
            assert!(g.quiet.contains(&"req".to_owned()), "a request unanswered keeps {done} shut: {:?}", g.quiet);
        }
        // Each gated aggregate is read gated, and the rule's `done`s come before them.
        let few = gated.rules().find(|r| gated.rel_name(r.head().rel()) == "few").unwrap();
        let lits: Vec<LitRef<'_>> = few.body().collect();
        let aggs: Vector<usize> = (0..lits.len()).filter(|&i| matches!(lits[i].view(), Lit::Agg { .. })).collect();
        assert_eq!(aggs.len(), 2);
        for &i in &aggs {
            assert!(lits[i].atom().unwrap().gated(), "the aggregate at {i} is gated");
            assert!(lits[..i].iter().any(|l| l.atom().is_some_and(|a| is_gate(gated.rel_name(a.rel())))), "its gate before it");
        }
        // The second aggregate's `ask` reads the first gated.
        let asks: Vec<RuleRef<'_>> = gated.rules().filter(|r| gated.rel_name(r.head().rel()).starts_with("ask_")).collect();
        assert!(asks.iter().any(|r| r.body().any(|l| matches!(l.view(), Lit::Agg { .. }) && l.atom().unwrap().gated())), "an ask reads the earlier aggregate gated");
        assert!(asks.iter().all(|r| r.body().all(|l| !matches!(l.view(), Lit::Agg { .. }) || l.atom().unwrap().gated())), "no ask reads an aggregate raw");
    }

    /// A rule with two negations inside a loading component: the second's ask reads the first gated, as the rule does,
    /// never bare: a bare negation of the component in an ask made the program unstratifiable (PYI's `type_root`, past
    /// `!typing_sym(h, "Literal")`, asking `annotated_meta`: "ask_annotated_meta_bb depends negatively on typing_sym
    /// in its own component").
    #[test]
    fn an_ask_past_a_gated_negation_reads_it_gated() {
        let prog = super::super::rr::parse_version(
            ".decl want(x: symbol)\n.decl req(x: symbol)\n.decl got(x: symbol, y: symbol)\n.decl seen(x: symbol)\n.decl kept(x: symbol)\n.decl fresh(x: symbol)\n\
             want(\"a\").\nwant(y) :- got(_, y).\nseen(y) :- got(_, y).\nkept(y) :- got(y, _).\n\
             fresh(x) :- want(x), !seen(x), !kept(x).\nreq(x) :- fresh(x).\n",
        )
        .unwrap();
        let answers: BTreeMap<String, Vec<String>> = [("req".to_owned(), vec!["got".to_owned()])].into();
        let gated = gate(&Placed::whole(prog.clone()), &answers).unwrap();
        let sccs = Placed::whole(gated.clone()).components().unwrap();
        let loading = sccs.iter().find(|s| s.contains("req")).unwrap();
        for rel in ["ask_seen_b", "done_seen_b", "ask_kept_b", "done_kept_b"] {
            assert!(loading.contains(rel), "{rel} in {loading:?}");
        }
        let ask = gated.rules().find(|r| gated.rel_name(r.head().rel()) == "ask_kept_b").expect("kept's ask");
        let negated: Vec<(&str, bool)> = ask.body().filter_map(|l| l.atom()).filter(|a| a.neg()).map(|a| (gated.rel_name(a.rel()), a.gated())).collect();
        assert_eq!(negated, [("seen", true)], "the ask reads the earlier negation gated, never bare");
        assert!(ask.body().filter_map(|l| l.atom()).any(|a| gated.rel_name(a.rel()) == "done_seen_b"), "behind its gate");
    }
