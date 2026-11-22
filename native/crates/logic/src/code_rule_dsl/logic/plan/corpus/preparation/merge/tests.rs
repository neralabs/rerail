    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    use crate::code_rule_dsl::logic::program_image::{Builder, T, Version};
    use std::sync::Arc;

    fn read(text: &str) -> Arc<Version> {
        crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()
    }

    fn heads(v: &Version) -> Vec<String> {
        v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect()
    }

    /// Two programs' copies of a relation go to the shared namespace (`s_q`) and their readers read it; an output
    /// merged keeps its name as a copy of its class; relations of equal facts merge; a repeated rule is dropped
    /// (`DropRepeated` around `MergeIdentical`, as the preparation runs them).
    #[test]
    fn the_merges_on_a_version() {
        let text = ".decl e(x: number, y: number)\n.input e\n\
                    .decl p0_q(x: number)\n.decl p1_q(x: number)\np0_q(x) :- e(x, _).\np1_q(x) :- e(x, _).\np1_q(x) :- e(x, _).\n\
                    .decl p0_r(x: number)\n.decl p1_r(x: number)\np0_r(x) :- p0_q(x), e(_, x).\np1_r(x) :- p1_q(x), e(_, x).\n\
                    .decl f1(k: symbol)\n.decl f2(k: symbol)\nf1(\"a\").\nf2(\"a\").\n\
                    .decl uses(x: number, k: symbol)\nuses(x, k) :- p0_r(x), f1(k).\nuses(x, k) :- p1_r(x), f2(k).\n\
                    .output uses, p0_r\n";
        let mut p = Pipeline::new("test", read(text));
        let (mut merge, mut repeats) = (super::MergeIdentical::default(), super::DropRepeated::default());
        p.program(&mut repeats).program(&mut merge).program(&mut repeats);
        let (out, _) = p.finish().unwrap();
        assert!(merge.merged > 0 && merge.shared > 0 && repeats.0 > 0, "the program merges, shares and repeats: {} {} {}", merge.merged, merge.shared, repeats.0);
        let heads = heads(&out);
        assert!(heads.iter().any(|h| h == "s_q"), "{}", out.show());
        assert!(heads.iter().any(|h| h == "p0_r"), "the output keeps its name:\n{}", out.show());
        assert_eq!(heads.iter().filter(|h| *h == "uses").count(), 1, "uses' two rules are one once their reads merge:\n{}", out.show());
    }

    /// A request relation (decision 0174, `rr_node_text_req`) keeps its own: equal rules do not merge it with another
    /// relation (`fixed`). PR #508's c80b57f39 (t47): the version's merge took it as any relation, merged 1 where the
    /// tree's merged none, and a replay sample failed the debug check "the version's merges are the tree's". The
    /// tree's answer, stated: nothing merged or shared, both relations keep their rules.
    #[test]
    fn a_request_relation_keeps_its_own_on_a_version_as_on_the_tree() {
        let text = ".decl e(x: number, y: number)\n.input e\n\
                    .decl rr_node_text_req(n: number)\nrr_node_text_req(n) :- e(n, _).\n\
                    .decl other(n: number)\nother(n) :- e(n, _).\n\
                    .decl uses(n: number)\nuses(n) :- other(n), rr_node_text_req(n).\n.output uses\n";
        let mut p = Pipeline::new("test", read(text));
        let (mut merge, mut repeats) = (super::MergeIdentical::default(), super::DropRepeated::default());
        p.program(&mut repeats).program(&mut merge).program(&mut repeats);
        let (out, _) = p.finish().unwrap();
        assert_eq!((merge.merged, merge.shared, repeats.0), (0, 0, 0), "{}", out.show());
        for rel in ["rr_node_text_req", "other", "uses"] {
            assert_eq!(out.rel(rel).map(|id| out.heads(id).count()), Some(1), "{rel}: {}", out.show());
        }
    }

    /// A request relation's repeated rule dropped: the rule kept serves both copies' readers, stated by rule
    /// (decision 0331). Left by place over the rules dropped, the merge's
    /// renamed readers after it fell on other rules (the combine's tail on corpus0 x B-now).
    #[test]
    fn a_repeated_request_rule_dropped_keeps_both_readers_by_rule() {
        use std::collections::BTreeSet;
        let text = ".decl a(x: number)\n.input a\n.decl b(x: number)\n.input b\n.decl q(x: number)\n.output q\n\
                    q(x) :- a(x).\nq(x) :- b(x).\n";
        let by = |rs: &[&str]| rs.iter().map(|r| (*r).to_owned()).collect::<BTreeSet<String>>();
        // `text`'s version with record `asked` for `q`, and `q`'s first rule repeated after its others when `repeat`,
        // as one base image: a text's repeated rule is read as one (the version lowering, 0276 step 6), and a repeat
        // reaches the pass from a merge of two programs' rules.
        let image = |repeat: bool, asked: Vec<BTreeSet<String>>| {
            let v = read(text);
            let mut b = Builder::on(&v);
            if repeat {
                b.rule().head("q", &[T::Var("x")]).atom("a", &[T::Var("x")]).finish().unwrap();
            }
            b.fields().set_asked(&std::collections::BTreeMap::from([("q".to_owned(), asked)]));
            let (o, _) = b.finish().unwrap();
            Version::open(o.fold().unwrap().base_bytes().unwrap()).unwrap()
        };
        let v = image(true, vec![by(&["r1"]), by(&["r2"]), by(&["r3"])]);
        assert_eq!(v.rel("q").map(|q| v.heads(q).count()), Some(3));
        let mut p = Pipeline::new("test", v);
        let mut repeats = super::DropRepeated::default();
        p.program(&mut repeats);
        let out = p.current().unwrap().clone();
        assert_eq!((repeats.0, out.rules().count()), (1, 2));
        let want = image(false, vec![by(&["r1", "r3"]), by(&["r2"])]);
        assert_eq!(out.asked().get("q"), Some(&vec![BTreeSet::from(["r1".to_owned(), "r3".to_owned()]), BTreeSet::from(["r2".to_owned()])]), "the step's own record, by rule");
        assert!(crate::code_rule_dsl::logic::program_image::same_program(&want, &out), "{}", crate::code_rule_dsl::logic::program_image::differences(&want, &out));
    }

    /// A library's facts may go undeclared (`param_slot("datetime.datetime", "tzinfo", 7).`): such relations merge by
    /// their facts, and the merge does not ask for a declaration they lack.
    #[test]
    fn undeclared_fact_relations_merge() {
        let text = ".decl e(x: number)\n.input e\nf1(\"a\", 1).\nf2(\"a\", 1).\n\
                    .decl uses(x: number, k: symbol)\nuses(x, k) :- e(x), f1(k, _).\nuses(x, k) :- e(x), f2(k, _).\n\
                    .output uses\n";
        let mut p = Pipeline::new("test", read(text));
        let mut merge = super::MergeIdentical::default();
        p.program(&mut merge);
        p.finish().unwrap();
        assert_eq!(merge.merged, 1);
    }

    /// A request and its widened copy left with the request's own rules merge into the request (decision 0174's copy,
    /// Q3): the request's rules serve the copy's readers too (decision 0331). Another relation equal to the request
    /// stays apart.
    #[test]
    fn a_request_merges_with_its_own_copy_only() {
        let text = ".decl k(x: number)\n.input k\n.decl rr_token_req(x: number)\n.decl rr_wide_node_text_rr_token_req(x: number)\n\
                    .decl other(x: number)\n.decl a(x: number)\n.decl b(x: number)\n\
                    rr_token_req(x) :- k(x), x < 3.\nrr_wide_node_text_rr_token_req(x) :- k(x), x < 3.\nother(x) :- k(x), x < 3.\n\
                    a(x) :- rr_token_req(x).\nb(x) :- rr_wide_node_text_rr_token_req(x), other(x).\n.output a, b\n";
        let by = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<std::collections::BTreeSet<String>>();
        let read = read(text);
        let mut b = Builder::on(&read);
        let asked = std::collections::BTreeMap::from([("rr_token_req".to_owned(), vec![by(&["a"])]), ("rr_wide_node_text_rr_token_req".to_owned(), vec![by(&["b"])])]);
        b.fields().set_asked(&asked);
        let (v, _) = b.finish().unwrap();
        let mut p = Pipeline::new("test", v);
        let mut merge = super::MergeIdentical::default();
        p.program(&mut merge);
        let (out, _) = p.finish().unwrap();
        assert_eq!(merge.merged, 1, "the copy merges into the request, `other` stays apart");
        assert!(heads(&out).iter().any(|h| h == "other"), "{}", out.show());
        assert_eq!(out.asked(), std::collections::BTreeMap::from([("rr_token_req".to_owned(), vec![by(&["a", "b"])])]));
    }

    /// A rule's branded atoms (decision 0270, a lowered program's names) compare as `canonical` writes them, the atom
    /// itself: `a1` and `a2` read the same branded atom and merge, `b` reads another and stays apart, though the three
    /// rules differ in nothing else.
    #[test]
    fn rules_merge_only_on_the_same_branded_atom() {
        use crate::code_rule_dsl::logic::front::Branded;
        let mut b = Builder::base();
        b.fields().declare("e", &[("x", "number"), ("k", "symbol")]);
        for rel in ["a1", "a2", "b", "uses"] {
            b.fields().declare(rel, &[("x", "number")]);
        }
        b.fields().set_inputs(&["e"]);
        b.fields().set_outputs(&["uses"]);
        for (rel, atom) in [("a1", 1), ("a2", 1), ("b", 2)] {
            b.rule().head(rel, &[T::Var("x")]).atom("e", &[T::Var("x"), T::Branded(Branded::name(atom))]).finish().unwrap();
        }
        b.rule().head("uses", &[T::Var("x")]).atom("a1", &[T::Var("x")]).atom("a2", &[T::Var("x")]).atom("b", &[T::Var("x")]).finish().unwrap();
        let (v, _) = b.finish().unwrap();
        assert!(v.show().matches("e($x, ").count() == 3 && !v.show().contains("\"p\""), "branded: {}", v.show());
        let mut p = Pipeline::new("test", v);
        let mut merge = super::MergeIdentical::default();
        p.program(&mut merge);
        let (out, _) = p.finish().unwrap();
        assert_eq!(merge.merged, 1, "a1 and a2 alone: {}", out.show());
        let heads = heads(&out);
        assert!(heads.iter().any(|h| h == "b") && !(heads.iter().any(|h| h == "a1") && heads.iter().any(|h| h == "a2")), "{}", out.show());
    }
