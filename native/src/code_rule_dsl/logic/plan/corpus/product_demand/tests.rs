    use super::*;
    /// `text`'s version.
    pub(super) fn parse(text: &str) -> Result<Arc<Version>, crate::code_rule_dsl::logic::front::Error> {
        crate::code_rule_dsl::logic::rr::parse_version(text)
    }

    const INPUTS: &str = "
        .decl a(n: node)
        .decl b(n: node)
        .decl edge(from: node, to: node)
        .input a, b, edge
        .decl product(x: node, y: node)
        product(x, y) :- a(x), b(y).
    ";

    #[test]
    fn a_product_gets_only_the_pairs_its_readers_bind() {
        let v = parse(&format!(
            "{INPUTS}
            .decl relay(x: node, y: node)
            relay(x, y) :- product(x, y), a(x), b(y).
            .decl present(x: node, y: node)
            .decl missing(x: node, y: node)
            .output present, missing
            present(x, y) :- edge(x, y), relay(x, y).
            missing(x, y) :- edge(x, y), !relay(x, y).
        "
        ))
        .unwrap();
        let mut program = Program::of(&v);
        assert_eq!(
            rewrite(&mut program, false).unwrap().products,
            BTreeSet::from(["product".to_owned(), "relay".to_owned()])
        );
        let rules: BTreeSet<String> = program.shown(None).into_iter().collect();
        assert!(rules.contains("product($x, $y) :- rr_product_demand_product($x, $y), a($x), b($y)"));
        assert!(rules.contains("rr_product_demand_relay($x, $y) :- edge($x, $y)"));
        assert!(rules.contains("present($x, $y) :- edge($x, $y), relay($x, $y)"));
        assert!(rules.contains("missing($x, $y) :- edge($x, $y), !relay($x, $y)"));
    }

    /// A node bound only by a lookup on constants (`qv("builtins.object", "cls", c)`, in `calls.rerail`) is fixed, as
    /// `c = constant` is: the product is one side times that lookup's few rows, and an output may hold it. A lookup
    /// with a wildcard in its key reads every row: still a product, refused as an output.
    #[test]
    fn a_node_bound_by_a_lookup_on_constants_is_fixed() {
        const LOOKUP: &str = "
            .decl a(n: node)
            .decl named(k: symbol, t: symbol, n: node)
            .input a, named
            .decl pair(x: node, y: node)
            .output pair
        ";
        let v = parse(&format!("{LOOKUP}\npair(x, y) :- a(x), named(\"builtins.object\", \"cls\", y).\n")).unwrap();
        assert!(rewrite(&mut Program::of(&v), true).unwrap().products.is_empty());
        let v = parse(&format!("{LOOKUP}\npair(x, y) :- a(x), named(_, \"cls\", y).\n")).unwrap();
        let error = rewrite(&mut Program::of(&v), true).err().unwrap();
        assert!(error.contains("unbounded node Cartesian product in `pair`"), "{error}");
        assert!(error.contains("the products: pair("), "{error}");
    }

    /// Q249: a node reached from constants through declared few-row fan-outs (a lookup on a fixed variable, not on
    /// constants alone) is no side of a product; without the fan-outs the same rule is one, and its refusal names it.
    #[test]
    fn a_node_a_few_row_fanout_reaches_from_constants_makes_no_product() {
        let text = |layout: &str| {
            format!(
                "
                .decl a(n: node)
                .decl b(q: symbol, m: symbol)
                .decl named(q: symbol, k: symbol, v: node)
                .input a, b, named
                {layout}
                .decl out(x: node, y: node)
                .output out
                out(x, y) :- a(x), b(\"k\", m), named(m, \"cls\", y).
            "
            )
        };
        let v = parse(&text(".layout b fanout(q -> m, few)\n.layout named fanout(q, k -> v, few)")).unwrap();
        let mut program = Program::of(&v);
        assert!(rewrite(&mut program, true).unwrap().products.is_empty());
        assert_eq!(program.v().show(), v.show());
        let v = parse(&text("")).unwrap();
        let error = rewrite(&mut Program::of(&v), true).err().unwrap();
        assert!(error.contains("unbounded node Cartesian product in `out`") && error.contains("the products: out("), "{error}");
    }

    #[test]
    fn an_output_that_requires_every_pair_is_refused() {
        let v = parse(&format!("{INPUTS}\n.output product\n")).unwrap();
        let mut program = Program::of(&v);
        let error = rewrite(&mut program, true).err().unwrap();
        assert!(
            error.contains("unbounded node Cartesian product in `product`"),
            "{error}"
        );
        assert!(!program.changed());
    }

    #[test]
    fn preparation_by_parts_propagates_an_unbounded_product_refusal() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use crate::content_store::ContentStore;

        let version = crate::code_rule_dsl::logic::rr::parse_version(&format!("{INPUTS}\n.output product\n")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("facts.sqlite")).unwrap();

        let error = super::super::preparation::parts::front(version, GrammarDialect::Python, &store, false)
            .err()
            .expect("the preparation must refuse an output that requires all pairs");
        assert!(error.contains("unbounded node Cartesian product in `product`"), "{error}");
    }

    #[test]
    fn connected_joins_and_program_constants_stay_as_written() {
        let v = parse(
            "
            .decl edge(from: node, to: node)
            .input edge
            .decl constant(n: node)
            constant(1).
            .decl out(x: node, y: node)
            .output out
            out(x, y) :- edge(x, z), edge(z, y).
            out(x, y) :- edge(x, _), constant(y).
            out(x, y) :- edge(x, _), k = max z : { edge(x, z) }, edge(y, k).
        ",
        )
        .unwrap();
        let mut program = Program::of(&v);
        assert!(rewrite(&mut program, false).unwrap().products.is_empty());
        assert!(!program.changed());
    }
