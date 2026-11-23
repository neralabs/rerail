    use super::*;
    use super::super::super::program_image::{Lit, RuleRef};
    use super::super::super::rr::parse_version;
    use super::super::body::Body;

    /// `calibrated` of rule `r` of `v` (the program's, or a copy's), under `base`'s fields.
    fn calibrated_rule(base: &Placed, model: &Model, v: &Version, r: RuleRef<'_>) -> Option<f64> {
        calibrated(base.fields().unwrap(), model, &Body::of(v, r))
    }

    const HEADER: &str = ".decl a(x: node)\n.decl b(x: node, y: node)\n.decl c(x: node, y: node)\n.decl r(x: node)\n.decl o(x: node)\n.input a\n.input b\n.output o\n";

    /// The program of `rules` under the tests' declarations.
    fn rules(rules: &[&str]) -> Arc<Version> {
        parse_version(&format!("{HEADER}{}", rules.join("\n"))).unwrap()
    }

    /// `v`'s rules for `rel`, in order.
    fn of<'v>(v: &'v Version, rel: &str) -> Vec<RuleRef<'v>> {
        v.rules().filter(|r| v.rel_name(r.head().rel()) == rel).collect()
    }

    /// The copies of the reader's (`o`'s) read `at` unfolded through `def`'s rules (`r`'s, or `c`'s), shown.
    fn unfold(reader: &str, at: usize, defs: &[&str], fresh: usize) -> Option<Vec<String>> {
        let v = rules(&std::iter::once(reader).chain(defs.iter().copied()).collect::<Vec<_>>());
        let read = defs.first().map(|d| d.split('(').next().unwrap().trim()).unwrap_or("r");
        inline::unfolded_shown(&v, of(&v, "o")[0], at, &of(&v, read), fresh)
    }

    /// The relation literal `i` of `v`'s rule `r` reads; `""` for one not an atom.
    fn read_at<'v>(v: &'v Version, r: RuleRef<'v>, i: usize) -> &'v str {
        match r.body().nth(i).expect("a literal").view() {
            Lit::Atom(a) => v.rel_name(a.rel()),
            _ => "",
        }
    }

    /// A positive read is one copy per rule; a negated one of a single atom with its own variable is that atom
    /// negated with `_`; a negated one of literals over the head is their disjunction; a join is refused.
    #[test]
    fn reads_unfold_into_their_readers() {
        assert_eq!(unfold("o(x) :- a(x), r(x).", 1, &["r(z) :- b(z, w), a(w)."], 1).unwrap(), ["o($x) :- a($x), b($x, $w_i1_1), a($w_i1_1)"]);
        assert_eq!(unfold("o(x) :- a(x), !r(x).", 1, &["r(z) :- b(z, w)."], 1).unwrap(), ["o($x) :- a($x), !b($x, _)"]);
        assert_eq!(unfold("o(x) :- a(x), !r(x).", 1, &["r(z) :- a(z), z != 3."], 1).unwrap(), ["o($x) :- a($x), !a($x)", "o($x) :- a($x), $x = 3"]);
        assert!(unfold("o(x) :- a(x), !r(x).", 1, &["r(z) :- b(z, w), a(w)."], 1).is_none());
        // A head variable named twice equates the read's terms, the reader's own variables as they are, though one of
        // them has the name of the rule's.
        assert_eq!(unfold("o(n) :- a(t), c(t, n).", 1, &["c(n, n) :- a(n)."], 1).unwrap(), ["o($n) :- a($t), a($t), $t = $n"]);
    }

    /// The copies a read unfolds into are counted without making them (`copies`): what `unfold` makes, or `None`
    /// where it refuses. Each was made, every rule cloned and its variables renamed, up to three times a reader only
    /// to be counted (13% of `keeps::choose`).
    #[test]
    fn a_reads_copies_are_counted_as_unfold_makes_them() {
        let cases = [
            ("o(x) :- a(x), r(x).", vec!["r(z) :- b(z, w), a(w).", "r(z) :- a(z)."]),
            ("o(x) :- a(x), !r(x).", vec!["r(z) :- b(z, w)."]),
            ("o(x) :- a(x), !r(x).", vec!["r(z) :- a(z), z != 3.", "r(z) :- b(z, z)."]),
            ("o(x) :- a(x), !r(x).", vec!["r(z) :- b(z, w), a(w)."]),
            ("o(x) :- a(x), !r(x).", vec!["r(z) :- b(z, w), b(w, w)."]),
            ("o(x) :- a(x), !r(x).", vec!["r(z) :- a(z), z != 1, z != 2, z != 3, z != 4, z != 5, z != 6.", "r(z) :- a(z), z != 7, z != 8, z != 9, z != 10, z != 11, z != 12."]),
            ("o(n) :- a(t), c(t, n).", vec!["c(n, n) :- a(n)."]),
        ];
        for (reader, defs) in cases {
            let v = rules(&std::iter::once(reader).chain(defs.iter().copied()).collect::<Vec<_>>());
            let o = of(&v, "o")[0];
            let read = defs[0].split('(').next().unwrap();
            let at = o.body().count() - 1;
            assert_eq!(inline::count(&v, o, at, &of(&v, read)), inline::unfolded(&v, o, at, &of(&v, read), 0).map(|c| c.1.len()), "{reader}");
        }
        // Not an atom: no copies.
        let v = rules(&["o(x) :- a(x), x != 3."]);
        assert_eq!(inline::count(&v, of(&v, "o")[0], 1, &[]), None);
    }

    /// A reader in a recursion is priced by its delta plans: inlined into `reach`'s recursive rule, `edge`'s join
    /// is made again for each new reach row, though priced whole the copy joins `imp` and `md` first and looks as
    /// cheap as reading `edge` (`rr_cx_edge`, priced 7.8 M work cheaper inlined; 277 s of calls_cx when it was).
    #[test]
    fn a_relation_read_in_a_recursion_is_priced_by_its_delta_plans() {
        let prog = parse_version(
            ".decl imp(d: number, q: number)\n.decl md(e: number, q: number)\n.decl own(d: number)\n.input imp\n.input md\n.input own\n\
             .decl edge(d: number, e: number)\nedge(d, e) :- imp(d, q), md(e, q).\n\
             .decl reach(d: number, e: number)\nreach(d, d) :- own(d).\nreach(d, f) :- reach(d, e), edge(e, f).\n.output reach\n",
        )
        .unwrap();
        let mut model = model_with(&prog, &[
            ("imp", 40_000.0, &[4_000.0, 4_000.0]),
            ("md", 4_000.0, &[4_000.0, 4_000.0]),
            ("own", 4_000.0, &[4_000.0]),
            ("edge", 40_000.0, &[4_000.0, 4_000.0]),
            ("reach", 100_000.0, &[4_000.0, 4_000.0]),
        ]);
        model.stats.nodes = 450_000.0;
        let chosen = choose(&Placed::whole(prog), &mut model).unwrap();
        assert!(chosen.is_none(), "edge inlined into reach's recursion");
    }

    /// A head that names a variable twice equates the read's terms there, the read's own: a reader variable named as
    /// one of the definition's (`n` in both) stays the reader's. (`expr_name(n, n, x) :- ast_name(n, x)` read as
    /// `expr_name(t, n, x)` once came out `t = t`, leaving the reader's `n` free: every stored name of the batch.)
    #[test]
    fn a_head_variable_named_twice_equates_the_readers_terms() {
        let v = parse_version(".decl a(x: node)\n.decl b(x: node, y: node)\n.decl r(x: node)\n.decl o(x: node)\n.input a\n.input r\n.output o\no(t) :- a(t), b(t, n), r(n).\nb(n, n) :- a(n).\n").unwrap();
        let copies = inline::unfolded_shown(&v, of(&v, "o")[0], 1, &of(&v, "b"), 1).unwrap();
        assert_eq!(copies[0], "o($t) :- a($t), a($t), $t = $n, r($n)");
    }

    /// A body that meets only at a variable the head projects away splits there; a join through a head variable does
    /// not.
    #[test]
    fn products_split_at_their_key() {
        let v = rules(&["o(x) :- a(x), b(x, y), b(y, z), a(z), y != 3."]);
        assert!(factors_of(&v, of(&v, "o")[0]).is_none(), "the head names only x");
        let v = parse_version(
            ".decl a(x: node)\n.decl b(x: node, y: node)\n.decl o(x: node, z: node)\n.input a\n.input b\n.output o\no(x, z) :- b(x, p), p != 3, b(p, z), a(z).\n",
        )
        .unwrap();
        let r = of(&v, "o")[0];
        let (key, parts) = factors_of(&v, r).unwrap();
        assert_eq!(key, "p");
        let lits: Vec<_> = r.body().collect();
        let shown: Vec<String> = parts.iter().map(|p| p.iter().map(|&i| v.show_lit(r, lits[i])).collect::<Vec<_>>().join(", ")).collect();
        assert_eq!(shown, ["b($x, $p), $p != 3", "b($p, $z), a($z)"]);
    }

    /// The two-chunk regression's shape (0099, "The regression"): `ctx` is small, read by one reader that binds its
    /// columns. What the steps really do (below: each probe of `look` by name and scope keeps 4 rows a binding,
    /// `binding` 3, the comparison every row) makes the copy about 66 k rows of work against about 18 k stored, so
    /// `ctx` should stay stored. Before the measured key domains (2ab61d7b4, O148) the estimate priced the copy's
    /// probe of `look` as keeping a binding in a hundred and inlined it, and only the observed side took that back.
    /// The estimate then bounded the bindings by their constant filters and kept `ctx` stored from the start. Priced
    /// as the executor counts (decision 0297: a step costs the rows it passes on, not its rows in as well), the
    /// estimates inline it again: the build scans `load`. Once the stored side has run (its build made 2 k rows),
    /// the copy is priced by those selectivities, and `ctx` stays stored. (`the_side_that_ran_is_believed` pins `weigh`'s taking an estimate back.)
    #[test]
    fn observed_selectivities_price_a_copy_that_never_ran() {
        use super::super::{Inputs, Stats, make_plan};
        const DECLS: &str = ".decl load(n: number, x: number)\n.decl look(n: number, x: number, s: number)\n.decl binding(s: number, x: number, b: number)\n\
             .decl ctx(n: number, s: number, x: number)\n.decl agg(n: number, s: number, x: number)\n.output agg\n";
        const CTX: &str = "ctx(n, s, x) :- load(n, x), look(n, x, s), x != 7.\n";
        let prog = parse_version(&format!("{DECLS}{CTX}agg(n, s, x) :- ctx(n, s, x), binding(s, x, b), b < n.\n")).unwrap();
        let base = Placed::whole(Arc::clone(&prog));
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let mut stats = Stats::empty_of(&base.fields().unwrap());
        for (rel, n) in [("load", 200_000.0), ("look", 500_000.0), ("binding", 30_000.0)] {
            stats.card.insert(rel, n);
        }
        let mut inputs = Inputs::file();
        inputs.facts.clear();
        inputs.given = Some(stats);
        let mut model = make_plan(&base, &db, inputs).unwrap();
        for (rel, n) in [("load", 200_000.0), ("look", 500_000.0), ("binding", 30_000.0)] {
            model.stats.card.insert(rel, n);
        }
        for (rel, ndvs) in [("load", vec![200_000.0, 500.0]), ("look", vec![5_000.0, 400.0, 800.0]), ("binding", vec![1_000.0, 700.0, 3_000.0])] {
            for (c, d) in ndvs.into_iter().enumerate() {
                model.ndv.insert((rel, c), d);
            }
        }
        let decided = |model: &mut Model| choose(&base, model).unwrap().map(|(done, _)| done).unwrap_or_default();
        let (reader, def) = (of(&prog, "agg")[0], of(&prog, "ctx")[0]);
        let (copies, ids) = inline::unfolded(&prog, reader, 0, &[def], 0).unwrap();
        let copy = copies.rule(ids[0]).unwrap();
        // Estimates alone: stored, as the steps' real selectivities say.
        observing(true);
        let def_v = Placed::whole(parse_version(&format!("{DECLS}{CTX}")).unwrap());
        let priced = |model: &mut Model| {
            let f = base.fields().unwrap();
            let strata = super::super::super::program_image::analyses::strata(&base.v, f.about).unwrap();
            let sccs: Vec<BTreeSet<String>> = strata.sccs.iter().map(|s| s.iter().map(|&r| base.v.rel_name(r).to_owned()).collect()).collect();
            let ids = BTreeMap::from([("ctx", vec![0])]);
            let (e, ins) = estimate(base.fields().unwrap(), model, (&def_v, &ids), &sccs, &BTreeSet::from(["ctx"]), &BTreeSet::from(["ctx"]));
            let got = format!(
                "cards {:?} ndv {:?}; ctx {:?}; reader {:?}, copy {:?}, build {:?}",
                ["load", "look", "binding"].map(|r| model.stats.card.get(r).copied()),
                model.ndv.get(&("look".to_owned(), 0)),
                e.get("ctx"),
                calibrated_rule(&base, model, &prog, reader),
                calibrated_rule(&base, model, &copies, copy),
                calibrated_rule(&base, model, &prog, def)
            );
            unestimate(model, ins);
            got
        };
        let shown = priced(&mut model);
        // Priced as the executor counts (decision 0297), the build scans `load`'s 200 k rows and the copy, starting
        // from `binding`, does 65 k: the estimates inline `ctx`.
        assert_eq!(decided(&mut model), vec!["ctx".to_owned()], "estimated, the copy is priced under the stored side: {shown}");
        // The stored side ran: the reader and the build, with the rows each step made.
        let nodes = model.stats.nodes;
        // Rule `r` of `v` run from 2,000 bindings, each step keeping `factor` of its rows (by the relation it reads),
        // observed under `key`; the counts.
        let ran = |model: &Model, v: &Version, r: RuleRef<'_>, key: u64, factor: &dyn Fn(&str) -> f64| -> Vec<(u64, u64)> {
            let b = Body::of(v, r);
            let (order, _, _) = model.plan_rule(base.fields().unwrap(), &b, None).unwrap();
            let mut rows = 2_000.0;
            let mut counts = vec![(1u64, 2_000u64)];
            for s in &order[1..] {
                let out = rows * factor(read_at(v, r, s.at));
                counts.push((rows as u64, out as u64));
                rows = out;
            }
            observe_steps(key, super::super::Order::new(&b, &order), &counts.iter().map(|&(a, b)| (a as i64, b as i64)).collect::<Vec<_>>());
            counts
        };
        // What the copy's steps did in other rules: `look` by name and scope 4 rows a binding, `x != 7` all of them.
        ran(&model, &copies, copy, 0, &|rel| match rel {
            "look" => 4.0,
            "binding" => 3.0,
            _ => 1.0,
        });
        let (reader_key, def_key) = (Body::of(&prog, reader).observed_key(), Body::of(&prog, def).observed_key());
        let own = ran(&model, &prog, reader, reader_key, &|rel| if rel == "binding" { 3.0 } else { 0.75 });
        let build = ran(&model, &prog, def, def_key, &|_| 1.0);
        observe(&[(reader_key, &own), (def_key, &build)], &[Seen { rel: "ctx", rows: 2_000.0, cols: vec![(Some(2_000.0), Some(1.0)), (Some(300.0), Some(7.0)), (Some(250.0), Some(8.0))] }], nodes);
        let shown = priced(&mut model);
        assert!(decided(&mut model).is_empty(), "observed, the copy is priced by what its steps did: ctx stays stored: {shown}");
        observing(false);
    }

    /// `weigh`: the side that ran is believed, the other doubted by the error the estimates were seen to make.
    #[test]
    fn the_side_that_ran_is_believed() {
        // Stored ran: the reader estimated 10 k and did 40 k, the build as estimated (50 k): the estimates missed by
        // 1.5 times. The copies' 70 k would save work against 40 k + 50 k + 4 x 1,000 rows; doubted, 105 k does not.
        let v = weigh(&[(10_000.0, Some(40_000.0))], &[(35_000.0, None), (35_000.0, None)], (50_000.0, Some(50_000.0)), (1_000.0, Some(1_000.0)));
        assert!(!v.inline && v.how.starts_with("stored observed"), "{v:?}");
        // Undoubted, the same copies are inlined.
        let v = weigh(&[(40_000.0, None)], &[(35_000.0, None), (35_000.0, None)], (50_000.0, None), (1_000.0, None));
        assert!(v.inline, "{v:?}");
        // Nothing ran: estimates against estimates.
        let v = weigh(&[(17_000.0, None)], &[(8_500.0, None), (8_500.0, None)], (55_000.0, None), (1_433.0, None));
        assert!(v.inline && v.how == "both sides estimated", "{v:?}");
        // Both ran, inlined did worse than stored: taken back.
        let v = weigh(&[(17_000.0, Some(30_000.0))], &[(8_500.0, Some(2_000_000.0))], (55_000.0, Some(55_000.0)), (1_433.0, Some(1_433.0)));
        assert!(!v.inline && v.how == "both sides observed", "{v:?}");
    }

    /// `.decl r(…) inline` and its kin are read as Soufflé spells them; two that disagree are an error.
    #[test]
    fn decl_qualifiers_are_read() {
        let base = Placed::whole(parse_version(".decl r(x: node) inline\n.decl s(x: node) no_magic\n.decl t(x: node) no_inline magic\n").unwrap());
        let f = base.fields().unwrap();
        assert!(f.qualified("r", "inline") && f.qualified("s", "no_magic") && f.qualified("t", "no_inline") && f.qualified("t", "magic"));
        assert!(!f.qualified("r", "magic"));
        assert!(parse_version(".decl r(x: node) inline no_inline\n").is_err());
    }

    /// A reader of one inlined relation twice has the second read unfolded a pass after the first, its copy's
    /// variables named apart from the first copy's: the same names would join the two reads' rows as one.
    #[test]
    fn two_reads_of_one_relation_unfold_apart() {
        let base = rules(&["o(x) :- a(y), r(x), r(y).", "r(z) :- b(z, w), a(w)."]);
        let got = super::inline::inline(&base, &base.rules().map(|r| r.id()).collect::<Vec<_>>(), &BTreeSet::from(["r".to_owned()])).unwrap();
        let first = got.rules.iter().map(|e| got.v.rule(e.0).unwrap()).find(|r| got.v.rel_name(r.head().rel()) == "o").unwrap();
        assert_eq!(got.v.show_rule(first), "o($x) :- a($y), b($x, $w_i0_1), a($w_i0_1), b($y, $w_i1_1), a($w_i1_1)");
    }

    /// A rule's copies over every pass stay within `RULE_COPIES`: each read of a two-rule inlined relation doubles
    /// them, and six reads would make 64. Past the bound the rule keeps its read, so the relation keeps its rules.
    #[test]
    fn unfolding_stops_at_a_rules_copy_bound() {
        let base = rules(&["o(x) :- r(x), r(y1), r(y2), r(y3), r(y4), r(y5).", "r(z) :- a(z).", "r(z) :- b(z, z)."]);
        let got = super::inline::inline(&base, &base.rules().map(|r| r.id()).collect::<Vec<_>>(), &BTreeSet::from(["r".to_owned()])).unwrap();
        let v = &got.v;
        let copies: Vec<RuleRef<'_>> = got.rules.iter().map(|e| v.rule(e.0).unwrap()).filter(|r| v.rel_name(r.head().rel()) == "o").collect();
        assert_eq!(copies.len(), RULE_COPIES);
        assert!(copies.iter().all(|r| r.body().filter(|l| matches!(l.view(), Lit::Atom(a) if v.rel_name(a.rel()) == "r")).count() == 1));
        assert!(got.unfinished.contains("r"), "the relation keeps its rules");
    }

    /// Each candidate is priced with every other stored. Two chosen where one reads the other would compose (the
    /// reader's copies holding the read relation's body), which nobody priced: the reader is stored. (RUF's and PYI's
    /// diagnostics' document key over an inlined diagnostic: 197 M rows on 30 files.)
    #[test]
    fn a_relation_reading_an_inlined_one_is_stored() {
        let prog = parse_version(
            ".decl a(x: number)\n.decl b(x: number, y: number)\n.decl c(x: number)\n.input a\n.input b\n.input c\n\
             .decl s(x: number)\ns(x) :- b(x, y).\n\
             .decl r(x: number)\nr(x) :- s(x), c(x).\n\
             .decl o(x: number)\no(x) :- a(x), r(x).\n.output o\n",
        )
        .unwrap();
        let mut model = model_with(&prog, &[("a", 5.0, &[5.0]), ("b", 1_000_000.0, &[100_000.0, 1_000.0]), ("c", 1_000_000.0, &[1_000_000.0])]);
        let done = choose(&Placed::whole(prog), &mut model).unwrap().map(|(done, _)| done).unwrap_or_default();
        assert!(!(done.iter().any(|r| r == "s") && done.iter().any(|r| r == "r")), "both inlined: {done:?}");
        assert!(!done.is_empty(), "nothing inlined: the case does not choose");
    }

    /// An aggregate's demand, read first by the aggregate it restricts (`Body::demand_first`), is stored: inlined,
    /// the aggregate would enumerate the demand's own join in place of its keys. A relation of the same shape read
    /// otherwise stays a candidate.
    #[test]
    fn an_aggregates_demand_is_stored() {
        let prog = super::super::super::rr::parse_version(
            ".decl b(x: number, y: number)\n.decl c(x: number)\n.input b\n.input c\n\
             .decl rr_demand_agg_g(x: number)\nrr_demand_agg_g(x) :- b(x, _), c(x).\n\
             .decl g(x: number, y: number)\ng(x, y) :- rr_demand_agg_g(x), b(x, y).\n\
             .decl s(x: number)\ns(x) :- b(x, _), c(x).\n\
             .decl o(x: number, y: number)\no(x, y) :- s(x), g(x, y).\n.output o\n",
        )
        .unwrap();
        let placed = Placed::whole(prog);
        let f = Fields::of(&placed).unwrap();
        let mut rules_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for k in 0..placed.len() {
            rules_of.entry(placed.v.rel_name(placed.rule(k).head().rel())).or_default().push(k);
        }
        let why = refusals(&f, &placed, placed.len(), &rules_of, &[]);
        assert_eq!(why.get("rr_demand_agg_g").copied(), Some("an aggregate's demand"), "{why:?}");
        assert!(!why.contains_key("s"), "{why:?}");
    }

    /// A relation a filter reads is stored (decision 0611): where every row passes, the filter is read as it, so it
    /// must have rows. Inlined, the filter is its rules under the bound, derived with nothing beside it. A relation of
    /// the same shape read by a join stays a candidate.
    #[test]
    fn a_filters_source_is_stored() {
        let prog = super::super::super::rr::parse_version(
            ".decl b(x: number, y: number)\n.decl c(x: number)\n.input b\n.input c\n\
             .decl s(x: number, y: number)\ns(x, y) :- b(x, y), c(x).\n\
             .decl t(x: number, y: number)\nt(x, y) :- s(x, y), x < 10.\n\
             .decl u(x: number, y: number)\nu(x, y) :- b(x, y), c(y).\n\
             .decl o(x: number, y: number)\no(x, y) :- t(x, y), u(y, x).\n.output o\n",
        )
        .unwrap();
        let placed = Placed::whole(prog);
        let f = Fields::of(&placed).unwrap();
        let mut rules_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for k in 0..placed.len() {
            rules_of.entry(placed.v.rel_name(placed.rule(k).head().rel())).or_default().push(k);
        }
        let why = refusals(&f, &placed, placed.len(), &rules_of, &[]);
        assert_eq!(why.get("s").copied(), Some("a filter reads it (decision 0611)"), "{why:?}");
        assert!(!why.contains_key("u"), "{why:?}");
    }

    /// A model of `prog` whose inputs have the rows and distinct values by column given.
    fn model_with(prog: &Arc<Version>, inputs: &[(&str, f64, &[f64])]) -> Model {
        use super::super::{Inputs, Stats};
        use std::collections::{HashMap, HashSet};
        let given = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
        let derived: HashSet<String> = prog.rules().map(|r| prog.rel_name(r.head().rel()).to_owned()).collect();
        let stats = Stats::empty_of(&Placed::whole(Arc::clone(prog)).fields().unwrap());
        let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), stats, ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: given.into(), derived: derived.into() };
        model.stats.nodes = 1_000_000.0;
        for &(rel, rows, ndv) in inputs {
            model.stats.card.insert(rel, rows);
            for (c, &n) in ndv.iter().enumerate() {
                model.ndv.insert((rel.to_owned(), c), n);
                model.stats.fan.insert((rel.to_owned(), c), rows / n);
            }
        }
        model
    }

    /// A relation's rows are bounded by its head variables' distinct values (each the fewest of a column binding it),
    /// a constant counting one, and by its atoms' rows; its columns by the same; a recursion, and a head variable only
    /// a comparison binds, are unbounded.
    #[test]
    fn a_relation_is_bounded_by_its_inputs_counts() {
        // w's n is each node an `ast` match of document d finds: many a document, though a has 5 rows.
        let prog = parse_version(
            ".decl a(x: number)\n.decl b(x: number, y: number)\n.decl c(y: number, z: number)\n.input a\n.input b\n.input c\n\
             .decl r(x: number)\nr(x) :- b(x, y), c(y, z).\n\
             .decl w(n: number)\nw(n) :- a(d), ast(d, n, {\"language\": \"python\", \"rule\": {\"kind\": \"call\"}}).\n\
             .decl s(x: number, k: number)\ns(x, 3) :- b(x, y), a(x).\ns(x, k) :- r(x), a(k).\n\
             .decl p(x: number)\np(x) :- a(x).\np(x) :- p(y), b(y, x).\n\
             .decl t(x: number)\nt(x) :- a(y), x = y + 1.\n\
             .decl u(x: number)\nu(x) :- b(x, x), c(x, 7).\n\
             .decl o(x: number)\no(x) :- r(x), s(x, _), p(x), t(x), u(x).\n.output o, w\n",
        )
        .unwrap();
        let model = model_with(&prog, &[("a", 5.0, &[5.0]), ("b", 1_000_000.0, &[10.0, 1_000.0]), ("c", 1_000_000.0, &[1_000.0, 50.0])]);
        let base = Placed::whole(prog);
        let f = Fields::of(&base).unwrap();
        let mut ids_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for k in 0..base.len() {
            ids_of.entry(base.v.rel_name(base.rule(k).head().rel())).or_default().push(k);
        }
        let strata = super::super::super::program_image::analyses::strata(&base.v, f.about).unwrap();
        let sccs: Vec<BTreeSet<String>> = strata.sccs.iter().map(|s| s.iter().map(|&r| base.v.rel_name(r).to_owned()).collect()).collect();
        let b = bounds(&f, &base, base.len(), &model, &ids_of, &sccs);
        // r: x at most b's 10 distinct firsts.
        assert_eq!(b["r"], (10.0, vec![10.0]));
        // s: x in a (5), a constant (1); then r's 10 by a's 5. Columns: 5 + 10, and 1 + 5.
        assert_eq!(b["s"], (55.0, vec![15.0, 6.0]));
        // u: x in b's first column (10) and c's first (1,000): 10.
        assert_eq!(b["u"].0, 10.0);
        assert!(!b.contains_key("p"), "a recursion is unbounded");
        // t: x is one value for each row of a (a comparison binds it): a's 5 rows bound it, its column not.
        assert_eq!(b["t"], (5.0, vec![5.0]));
        assert!(b["w"].0.is_infinite(), "an `ast` match makes many rows a row of a: {:?}", b["w"]);
        assert_eq!(b["o"].0, 5.0, "p is unbounded, but x is bounded by t (5), r and u (10)");
    }

    /// A candidate its inputs bound under `SMALL` rows is small, though the planner prices its join large: `r` keeps
    /// a first column of `b` that has 10 values, while its join with `c` is planned at many rows, and inlined into
    /// `o`, whose `a` binds its key. Before the bound, `estimate` planned each such candidate and everything upstream
    /// of it (on the data-layer batch, three quarters of the programs' rules, over 9 in 10 of them small).
    #[test]
    fn a_candidate_bounded_small_is_stored_unplanned() {
        let prog = parse_version(
            ".decl a(x: number)\n.decl b(x: number, y: number)\n.decl c(y: number, z: number)\n.input a\n.input b\n.input c\n\
             .decl r(x: number)\nr(x) :- b(x, y), c(y, z).\n\
             .decl o(x: number)\no(x) :- a(x), r(x).\n.output o\n",
        )
        .unwrap();
        let mut model = model_with(&prog, &[("a", 5.0, &[5.0]), ("b", 1_000_000.0, &[10.0, 1_000.0]), ("c", 1_000_000.0, &[1_000.0, 50.0])]);
        let base = Placed::whole(Arc::clone(&prog));
        // The planner's own estimate of r is over SMALL: priced, it would be weighed, not stored small.
        let (_, planned) = calibrated_rows(base.fields().unwrap(), &model, &Body::of(&prog, of(&prog, "r")[0])).unwrap();
        assert!(planned >= SMALL, "r planned at {planned} rows");
        let chosen = choose(&base, &mut model).unwrap();
        assert!(chosen.is_none(), "r inlined: {:?}", chosen.map(|c| c.0));
    }

    /// The estimate of a relation's column is its rules' values together: `n`'s `x` is `a`'s 5 and `b`'s 1,000 (the
    /// minimum took `a`'s 5 for both), a constant adds one, and `p`'s recursive rule passes `x` on from `p` itself, so
    /// `p`'s first column has its base case's 5. Joins on a column priced at one rule's few values came out as
    /// products: `assign_target`'s first column at 1 value (its chain's base case), and a data-layer write at 985 M rows
    /// where 1,727 were made.
    #[test]
    fn a_columns_values_are_its_rules_together() {
        let prog = super::super::super::rr::parse_version(
            ".decl a(x: number)\n.decl b(x: number, y: number)\n.input a\n.input b\n\
             .decl n(x: number)\nn(x) :- a(x).\nn(x) :- b(x, _).\nn(7) :- a(_).\n\
             .decl p(x: number, y: number)\np(x, x) :- a(x).\np(x, y) :- p(x, z), b(z, y).\n\
             .decl o(x: number)\no(x) :- n(x), p(x, _).\n.output o\n",
        )
        .unwrap();
        let mut model = model_with(&prog, &[("a", 5.0, &[5.0]), ("b", 100_000.0, &[1_000.0, 2_000.0])]);
        let base = Placed::whole(prog.clone());
        let f = Fields::of(&base).unwrap();
        let mut ids_of: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for k in 0..base.len() {
            ids_of.entry(base.v.rel_name(base.rule(k).head().rel())).or_default().push(k);
        }
        let strata = super::super::super::program_image::analyses::strata(&base.v, f.about).unwrap();
        let sccs: Vec<BTreeSet<String>> = strata.sccs.iter().map(|s| s.iter().map(|&r| base.v.rel_name(r).to_owned()).collect()).collect();
        let need: BTreeSet<&str> = ["n", "p", "o"].into_iter().collect();
        let _ = estimate(f, &mut model, (&base, &ids_of), &sccs, &need, &BTreeSet::new());
        let n_rows = model.stats.card["n"];
        assert_eq!(model.ndv.at("n", 0).copied(), Some(n_rows.min(1_006.0)), "n's x: a's 5, b's 1,000 and the constant");
        assert_eq!(model.ndv.at("p", 0).copied(), Some(5.0), "p's x: its base case's, passed on by the recursion");
    }

    /// A rule's rows are its steps' under this model, as its work is: a plan taken as prepared (decision 0361 step 5)
    /// carries the facts-only model's rows at its nominal scale, here none, where the steps make thousands. Its rows
    /// read as 0, a relation built on it had a column of one value, and every join on it was priced as a product.
    #[test]
    fn a_rules_rows_are_its_steps_not_a_prepared_plans() {
        let prog = super::super::super::rr::parse_version(".decl b(x: number, y: number)\n.input b\n.decl r(x: number)\nr(x) :- b(x, _).\n.output r\n").unwrap();
        let mut model = model_with(&prog, &[("b", 100_000.0, &[1_000.0, 2_000.0])]);
        let body = super::super::body::nth(&prog, 0);
        let f = Placed::whole(prog.clone());
        let f = f.fields().unwrap();
        let (_, planned) = calibrated_rows(f, &model, &body).unwrap();
        let ((order, _, _), _) = model.plan_rule_step_rows(f, &body, None).unwrap();
        let mut prepared = super::super::prepared::Prepared::default();
        prepared.0.insert(super::super::line_key(&body, None), super::super::prepared::PreparedPlan { order, work_per_node: 1e-6, rows_per_node: 0.0, assumed: Vec::new() });
        model.stats.prepared = Arc::new(prepared);
        let (_, taken) = calibrated_rows(f, &model, &body).unwrap();
        assert!(planned >= 1_000.0, "r planned at {planned} rows");
        assert_eq!(taken, planned, "a prepared plan's rows (0) taken for its steps' ({planned})");
    }
