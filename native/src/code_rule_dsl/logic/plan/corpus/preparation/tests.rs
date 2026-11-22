    use super::*;

    /// `p` with the readers of each of `q`'s rules, in order (its `asked` rows).
    fn asked(p: &Arc<Version>, readers: Vec<std::collections::BTreeSet<String>>) -> Arc<Version> {
        use crate::code_rule_dsl::logic::program_facts::Field;
        use crate::code_rule_dsl::logic::program_image::{Builder, fields::FieldsMut};
        let mut b = Builder::on(p);
        b.fields().set_field(Field::Asked(&std::collections::BTreeMap::from([("q".to_owned(), readers)])));
        b.finish().unwrap().0
    }

    /// An input no rule reads leaves the inputs (an output stays one); the rules reordered keep each rule; the
    /// program that comes out is a base, which the store keeps.
    #[test]
    fn the_passes_after_the_image_prune_the_inputs_and_reorder_the_rules() {
        let v = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl a(x: number)\n.decl b(x: number)\n.decl c(x: number)\n.input a, b, c\n.decl p(x: number)\n.decl q(x: number)\n\
             p(x) :- a(x).\nq(x) :- p(x).\n.output p, q, c\n",
        )
        .unwrap();
        let (out, _, _) = run(v.clone(), true, None, |_| Ok(())).unwrap();
        assert!(out.base_bytes().is_some(), "a base");
        let inputs: Vec<&str> = out.inputs().map(|a| out.text(a)).collect();
        assert_eq!(inputs, ["a", "c"], "b is read by no rule; c is an output");
        let mut pl = Pipeline::new("test", v);
        pl.program(&mut RuleOrder("reverse".into()));
        let (rev, _) = pl.finish().unwrap();
        let heads: Vec<&str> = rev.rules().map(|r| rev.rel_name(r.head().rel())).collect();
        assert_eq!(heads, ["q", "p"]);
    }

    /// The request record (decision 0331) follows the steps' rules, the derivations' too (by each rule's source), and
    /// holds one entry a rule after the pipeline; readers merged are named by their class.
    #[test]
    fn the_request_record_follows_the_preparation_steps() {
        let p = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl k(x: number)\n.input k\n.decl q(x: number)\n.decl a(x: number)\n.decl b(x: number)\n\
             q(x) :- k(x), x < 3.\nq(x) :- k(x), x > 5.\na(x) :- q(x).\nb(x) :- q(x), x != 1.\n.output a, b\n",
        )
        .unwrap();
        let by = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<std::collections::BTreeSet<String>>();
        let v = asked(&p, vec![by(&["a"]), by(&["b"])]);
        let (out, _, _) = run(v, true, None, |_| Ok(())).unwrap();
        assert_eq!(out.asked_against_rules(), [("q".to_owned(), 2, 2)]);
        assert_eq!(out.asked()["q"], [by(&["a"]), by(&["b"])]);
        // Two equal readers merged: the record names the class (`MergeIdentical`'s rename).
        let p = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl k(x: number)\n.input k\n.decl q(x: number)\n.decl a(x: number)\n.decl a2(x: number)\n.decl o(x: number)\n\
             q(x) :- k(x), x < 3.\nq(x) :- k(x), x > 5.\na(x) :- q(x).\na2(x) :- q(x).\no(x) :- a(x), a2(x).\n.output o\n",
        )
        .unwrap();
        let v = asked(&p, vec![by(&["a"]), by(&["a2"])]);
        let (out, _, _) = run(v, true, None, |_| Ok(())).unwrap();
        assert!(out.rel("a2").is_none_or(|r| out.heads(r).next().is_none()), "a2 merged into a");
        assert_eq!(out.asked()["q"], [by(&["a"]), by(&["a"])]);
    }
