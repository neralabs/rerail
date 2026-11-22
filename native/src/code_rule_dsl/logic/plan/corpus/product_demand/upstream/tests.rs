    use super::*;
    use super::super::tests::parse;

    #[test]
    fn bound_columns_keep_wildcards_and_scope_the_point_walk() {
        let v = parse(
            "
            .decl kind(n: node, k: symbol)
            .decl child(n: node, c: node, i: number)
            .decl chosen(x: node)
            .input kind, child, chosen
            .decl peel(x: node, y: node)
            peel(x, x) :- kind(x, k), k != \"wrap\".
            peel(x, y) :- kind(x, \"wrap\"), child(x, c, _), peel(c, y).
            .decl missing(x: node)
            .output missing
            missing(x) :- chosen(x), !peel(x, _).
        ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert!(restrict(&mut p, "peel", &BTreeSet::new(), false, false).unwrap());
        let specs = p.shown(Some("rr_reader_demand_peel"));
        assert!(specs.iter().any(|r| r.contains("chosen($x)")));
        assert!(specs.iter().any(|r| r.starts_with("rr_reader_demand_peel($c)")
            && r.contains("rr_reader_demand_peel($x)")
            && r.contains("child($x, $c")));
    }

    #[test]
    fn a_negated_reader_supplies_its_filter_to_the_tuple_demand() {
        let v = parse(
            "
            .decl attrs(n: node, r: node, s: symbol)
            .decl a(n: node)
            .decl b(n: node)
            .input attrs, a, b
            .decl product(r: node, n: node)
            product(r, n) :- a(r), b(n).
            .decl reader(r: node, n: node)
            reader(r, n) :- attrs(n, r, _), product(r, n).
            .decl out(n: node)
            .output out
            out(n) :- attrs(n, r, s), prefix(\"_\", s), !reader(r, n).
            ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        let rewritten = super::super::rewrite(&mut p, true).unwrap();
        assert!(rewritten.products.contains("product"));
        assert!(rewritten.readers.contains("reader"));
        let proof = p.shown(Some("rr_reader_demand_reader")).remove(0);
        assert!(proof.contains("@prefix"), "{proof}");
        assert!(p.shown(Some("rr_product_demand_product")).iter().any(|r| r.contains("rr_reader_demand_reader(")));
    }

    #[test]
    fn a_node_maximum_retains_its_group_witness() {
        let v = parse(
            "
            .decl node_order(n: node, k: number)
            .decl site(n: node, b: node)
            .decl asked(n: node)
            .input node_order, site, asked
            .decl rr_ordered0(n: node, k: number)
            rr_ordered0(n, k) :- site(n, b), node_order(b, k).
            .decl selected(n: node, b: node)
            .output selected
            selected(n, b) :- asked(n), k = max r : { rr_ordered0(n, r) }, node_order(b, k).
            ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert_eq!(
            restore_order_witnesses(&mut p, &BTreeSet::from(["selected".to_owned()])).unwrap(),
            1
        );
        let selected = p.shown(Some("selected")).remove(0);
        assert!(selected.ends_with("site($n, $b)"), "{selected}");
    }

    #[test]
    fn an_open_reader_preserves_the_whole_relation() {
        let v = parse(
            "
            .decl edge(x: node, y: node)
            .input edge
            .decl all(x: node, y: node)
            all(x, y) :- edge(x, y).
            .decl out(x: node, y: node)
            .output out
            out(x, y) :- all(x, y).
        ",
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert!(!restrict(&mut p, "all", &BTreeSet::new(), true, false).unwrap());
        assert!(p.columns("rr_reader_demand_all").is_none());
    }
