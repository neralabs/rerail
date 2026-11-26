    use super::*;

    #[test]
    fn a_demand_prefix_keeps_only_reads_not_already_proved() {
        let program = crate::code_rule_dsl::logic::rr::parse_version(
            "
            .decl relation(k: number, value: number, tag: number)
            .decl veto(value: number)
            .decl result(k: number, value: number)
            .input relation, veto
            .output result
            result(k, value) :- relation(k, _, _), relation(k, value, _), relation(k, _, 1), !veto(value).
            ",
        )
        .unwrap();
        let base = Placed::whole(program);
        let split = Split { pre: "pre_0".into(), vars: vec!["k".into(), "value".into()], cut: BTreeSet::from([3]) };
        let decls = [("pre_0".into(), vec![("k".into(), "int".into()), ("value".into(), "int".into())])];
        let actual = overlaid(&base, &BTreeSet::from(["veto"]), &BTreeMap::from([(0, split)]), &decls, &[]).unwrap();
        let pre = actual.v.rules().find(|r| actual.v.rel_name(r.head().rel()) == "pre_0").map(|r| actual.v.show_rule(r)).unwrap();
        assert_eq!(pre, "pre_0($k, $value) :- relation($k, $value, _), relation($k, _, 1)");

        let body = super::super::body::Body::of(&base.v, base.v.rule(base.id(0)).unwrap());
        assert!(!body.implied_read(0, &[0, 3]), "a cut read cannot prove a kept read");
    }
