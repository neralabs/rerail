    use super::*;
    #[test]
    fn only_affected_active_plans_reconsider_their_signatures() {
        let made = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl a(x:number,y:number)\n.decl b(x:number)\n.decl c(x:number)\n.decl p(x:number)\n.decl q(x:number)\np(x) :- a(x,_), b(x).\nq(x) :- c(x).\n").unwrap();
        let rels = BTreeMap::from([("a".into(),0),("b".into(),1),("c".into(),2)]);
        let probes = vec![(super::super::body::nth(&made,0),0),(super::super::body::nth(&made,1),0)];
        let mut w = Watch::new(&rels,&probes);
        assert!(!w.take(0), "empty deltas defer planning, including the first wave");
        w.delta(0,5); w.delta(2,2);
        assert!(w.take(0)); assert!(w.take(1));
        assert!(!w.take(0)); assert!(!w.take(1));
        w.changed(0,2); // Unused wildcard column.
        assert!(!w.take(0));
        w.changed(1,1); // A changed histogram matters, even with unchanged counts.
        assert!(w.take(0)); assert!(!w.take(1));
        assert_eq!(w.versions[1][1].value,1);
        w.delta(0,0); w.changed(0,0);
        assert!(!w.take(0));
        w.delta(0,1); assert!(w.take(0), "retain invalidation until the delta can execute");
        // A step's fanout observed for the first time, then moved past the drift threshold, replans the probe;
        // a small move does not.
        w.observed(0, vec![None, Some(0.01)]); assert!(w.take(0));
        w.observed(0, vec![None, Some(0.015)]); assert!(!w.take(0));
        w.observed(0, vec![None, Some(20.8)]); assert!(w.take(0));
    }
