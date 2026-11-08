use super::*;
use crate::code_rule_dsl::logic::rr;
use std::collections::BTreeSet;

#[test]
fn keys_propagate_through_projections_and_joins_without_assuming_injective_calls() {
    let p = rr::parse(".decl seed(k:number,v:number)\n.input seed\n.decl lattice(k:number,v:min<>) widen 3\nlattice(k,v) :- seed(k,v).\n.decl copy(k:number,v:number)\ncopy(k,v) :- lattice(k,v).\n.decl joined(k:number,v:number,w:number)\njoined(k,v,w) :- copy(k,v),lattice(k,w).\n.decl lossy(v:number)\nlossy(v) :- copy(_,v).\n.decl hidden(k:number,v:number)\nhidden(k,v) :- seed(k,_),lattice(k,v).\n.output joined,lossy,hidden\n").unwrap();
    let a = ProgramAnalysis::analyze(&p);
    assert_eq!(a.relation_keys["copy"], vec![vec![0]]);
    assert_eq!(a.relation_keys["joined"], vec![vec![0]]);
    assert!(
        a.rules
            .iter()
            .find(|r| r.head == "joined")
            .unwrap()
            .uniqueness
            .decision
            .proven
    );
    for name in ["lossy", "hidden"] {
        let proof = &a.rules.iter().find(|r| r.head == name).unwrap().uniqueness;
        assert!(!proof.decision.proven);
        assert!(!proof.unresolved.is_empty());
    }
}

#[test]
fn finite_instances_verify_join_witness_uniqueness() {
    let input = |name: &str| keys::Input {
        relation: name.into(),
        columns: vec![format!("{name}.k"), format!("{name}.v")],
        keys: vec![vec![0]],
    };
    for equality in [false, true] {
        let model = keys::Projection {
            inputs: vec![input("a"), input("b")],
            outputs: ["a.k", "a.v", "b.k", "b.v"].map(|s| Some(s.into())).into(),
            equalities: if equality {
                vec![("a.k".into(), "b.k".into())]
            } else {
                vec![]
            },
            ..Default::default()
        };
        for columns in 0u32..16 {
            let projection: Vec<usize> = (0..4).filter(|i| columns & (1 << i) != 0).collect();
            if !model.prove(&projection).decision.proven {
                continue;
            }
            for world in 0..81 {
                let values = [
                    (world / 27) % 3,
                    (world / 9) % 3,
                    (world / 3) % 3,
                    world % 3,
                ];
                let mut seen = BTreeSet::new();
                for a in 0..2 {
                    for b in 0..2 {
                        if equality && a != b {
                            continue;
                        }
                        let row = [a as i32, values[a], b as i32, values[2 + b]];
                        assert!(
                            seen.insert(projection.iter().map(|i| row[*i]).collect::<Vec<_>>())
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn scalar_compatibility_preserves_filters_keys_values_and_count_does_not_bridge_values() {
    use scalars::{Expression as E, Kind, Scalar};
    let scalar = |kind, value| Scalar {
        kind,
        value: E::Column(value),
        source: "events".into(),
        keys: vec![E::Column(0)],
        lookup: vec![E::Column(0)],
        predicates: vec![],
        dependent: false,
        non_null: true,
    };
    let inputs = vec![
        scalar(Kind::Count, 1),
        scalar(Kind::Min, 1),
        scalar(Kind::Max, 1),
        scalar(Kind::Sum, 1),
        scalar(Kind::Sum, 2),
    ];
    let a = scalars::analyze(inputs);
    assert_eq!(a.families[0].members, vec![0, 1, 2, 3]);
    assert_eq!(a.families[1].members, vec![4]);
    let original = scalar(Kind::Sum, 1);
    let mut changes = vec![];
    let mut s = original.clone();
    s.non_null = false;
    changes.push(s);
    let mut s = original.clone();
    s.dependent = true;
    changes.push(s);
    let mut s = original.clone();
    s.source = "other".into();
    changes.push(s);
    let mut s = original.clone();
    s.keys = vec![];
    changes.push(s);
    let mut s = original.clone();
    s.lookup = vec![E::Column(1)];
    changes.push(s);
    let mut s = original.clone();
    s.predicates = vec![E::Integer(1)];
    changes.push(s);
    let mut s = original.clone();
    s.kind = Kind::OrderedJoin;
    changes.push(s);
    let mut s = original.clone();
    s.value = E::Opaque("callback".into());
    changes.push(s);
    for changed in changes {
        let proof = original.compatible(&changed);
        assert!(!proof.proven, "{changed:?}");
        assert!(!proof.reason.is_empty());
    }
}

#[test]
fn logical_aggregates_share_only_the_same_bag_and_keep_dependent_lookups_separate() {
    let p = rr::parse(".decl event(id:number,g:number,v:number,w:number)\n.decl owner(g:number)\n.input event,owner\n.decl out(g:number,n:number,s:number,lo:number,hi:number,different:number)\nout(g,n,s,lo,hi,different) :- owner(g), n=count : {event(_,g,_,_)}, s=sum v : {event(_,g,v,_)}, lo=min x : {event(_,g,x,_)}, hi=max y : {event(_,g,y,_)}, different=sum w : {event(_,g,_,w)}.\n.output out\n").unwrap();
    let a = ProgramAnalysis::analyze(&p);
    assert_eq!(a.rules[0].scalars.families[0].members, vec![0, 1, 2, 3]);
    assert_eq!(a.rules[0].scalars.families[1].members, vec![4]);
    let p = rr::parse(".decl event(k:number,v:number)\n.decl owner(k:number)\n.input event,owner\n.decl out(k:number,n:number,s:number)\nout(k,n,s) :- owner(k), n=count : {event(k,_)}, s=sum v : {event(n,v)}.\n.output out\n").unwrap();
    let a = ProgramAnalysis::analyze(&p);
    assert!(a.rules[0].scalars.inputs[1].dependent);
    assert_eq!(a.rules[0].scalars.families.len(), 2);
}

#[test]
fn initial_summary_evaluation_requires_pure_outer_expressions_and_independent_sites() {
    use scalars::Expression as E;
    let p = rr::parse(".decl event(k:number,v:number)\n.decl owner(k:number)\n.input event,owner\n.decl out(k:number,n:number,s:number)\nout(k,n,s) :- owner(k), n=count : {event(k,_)}, s=sum v : {event(k,v)}.\n.output out\n").unwrap();
    let a = ProgramAnalysis::analyze(&p);
    let scalars = &a.rules[0].scalars;
    assert!(scalars.families[0].lookup.proven);
    assert!(scalars.initial_evaluation(&[E::Column(0)]).proven);
    for expression in [E::Opaque("callback".into()), E::Apply("Add".into(), vec![E::Column(0), E::Integer(1)])] {
        assert!(!scalars.initial_evaluation(&[expression]).proven);
    }
    let mut dependent = scalars.inputs.clone();
    dependent[0].dependent = true;
    let dependent = scalars::analyze(dependent);
    assert!(!dependent.families[0].lookup.proven);
    assert!(!dependent.initial_evaluation(&[]).proven);
}
