use super::*;
use crate::code_rule_dsl::logic::rr;
use std::collections::BTreeSet;

#[test]
fn existence_evaluation_proof_does_not_require_summary_combination() {
    use scalars::{Expression as E,Kind,Scalar};
    let mut exists = Scalar { kind:Kind::Exists,source:"events".into(),keys:vec![E::Column(0)],
        predicates:vec![],lookup:vec![E::Column(1)],value:E::Integer(1),dependent:false,non_null:true };
    assert!(!exists.compatible(&exists).proven);
    assert!(exists.independent_evaluation().proven);
    assert!(scalars::analyze(vec![exists.clone()]).evaluation_order(&[E::Column(0)]).proven);
    exists.dependent = true;
    assert!(!exists.independent_evaluation().proven);
    exists.dependent = false;
    exists.predicates.push(E::Opaque("callback".into()));
    assert!(!exists.independent_evaluation().proven);
}

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

#[test]
fn bag_storage_ignores_outer_mapping_but_preserves_value_and_effect_barriers() {
    use scalars::{Expression as E,Kind};
    let p=rr::parse(".decl event(k:number,v:number,w:number)\n.decl owner(a:number,b:number)\n.input event,owner\n.decl x(a:number,n:number,s:number)\nx(a,n,s) :- owner(a,_), n=count : {event(a,_,_)}, s=sum v : {event(a,v,_)}.\n.decl y(b:number,m:number)\ny(b,m) :- owner(_,b), m=max v : {event(b,v,_)}.\n.output x,y\n").unwrap();
    let a=ProgramAnalysis::analyze(&p);
    assert_eq!(a.scalar_storage.bags.families.len(),1);
    let mut other=a.scalar_storage.bags.inputs[0].clone();
    other.lookup=vec![E::Column(999)];
    assert!(other.same_bag(&a.scalar_storage.bags.inputs[1]).proven);
    assert!(!other.compatible(&a.scalar_storage.bags.inputs[1]).proven);
    other.dependent=true;assert!(other.same_bag(&other).proven);assert!(!other.compatible(&other).proven);
    let mut sum=a.scalar_storage.bags.inputs[1].clone();sum.kind=Kind::Sum;
    let mut other_value=sum.clone();other_value.value=E::Column(2);
    let analysis=scalars::analyze_bags(vec![sum, a.scalar_storage.bags.inputs[0].clone(),other_value]);
    assert_eq!(analysis.families.len(),2);
    let simple=storage::projection(2,vec![E::Column(1),E::Integer(0)],vec![]);
    assert!(simple.decision.proven);
    assert!(!storage::projection(2,vec![E::Apply("Add".into(),vec![E::Column(1),E::Integer(1)])],vec![]).decision.proven);
    assert!(!storage::projection(2,vec![E::Column(0)],vec![E::Opaque("checked predicate".into())]).decision.proven);
}

#[test]
fn cross_rule_storage_proofs_and_explain_grow_linearly_for_many_sites() {
    let p=rr::parse(".decl event(g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl out(g:number,n:number)\nout(g,n) :- owner(g), n=count : {event(g,_)}.\n.output out\n").unwrap();
    let scalar=ProgramAnalysis::analyze(&p).rules[0].scalars.inputs[0].clone();
    let analysis=bags::analyze((0..10000).map(|rule| {
        let mut s=scalar.clone();s.lookup=vec![scalars::Expression::Column(rule)];
        (bags::Site {rule,head:format!("out{rule}"),scalar:0},s)
    }));
    assert_eq!(analysis.bags.families.len(),1);
    assert!(analysis.bags.checks.len()<=20000);
    assert!(serde_json::to_vec(&analysis).unwrap().len()<12_000_000);
}

#[test]
fn cross_rule_counts_keep_local_value_lookups_when_stores_are_shared() {
    let p=rr::parse(".decl event(g:number,v:number,w:number)\n.decl owner(g:number)\n.input event,owner\n.decl first(g:number,n:number,s:number)\nfirst(g,n,s) :- owner(g), n=count : {event(g,_,_)}, s=sum v : {event(g,v,_)}.\n.decl second(g:number,n:number,s:number)\nsecond(g,n,s) :- owner(g), n=count : {event(g,_,_)}, s=sum w : {event(g,_,w)}.\n.output first,second\n").unwrap();
    let a=ProgramAnalysis::analyze(&p);
    assert_eq!(a.scalar_storage.bags.families[0].members,vec![0,1]);
    assert_eq!(a.scalar_storage.bags.families[1].members,vec![2,3]);
}

#[test]
fn disjoint_literal_tags_prove_unique_union_support_and_overlap_keeps_counts() {
    let program = rr::parse(".decl a(x:number)\n.input a\n.decl tagged(tag:number,x:number)\ntagged(1,x) :- a(x).\ntagged(2,x) :- a(x).\n.output tagged\n").unwrap();
    let analysis=ProgramAnalysis::analyze(&program);
    assert!(analysis.support_uniqueness["tagged"].proven);
    let overlap=rr::parse(".decl a(x:number)\n.input a\n.decl tagged(tag:number,x:number)\ntagged(1,x) :- a(x).\ntagged(1,x) :- a(x).\n.output tagged\n").unwrap();
    assert!(!ProgramAnalysis::analyze(&overlap).support_uniqueness["tagged"].proven);
    assert!(!scalars::maintenance_equivalent(true,false).proven);
    assert!(!scalars::maintenance_equivalent(false,true).proven);
    assert!(scalars::maintenance_equivalent(true,true).proven);
}

#[test]
fn dependency_lookup_effects_do_not_escape_into_shared_bag_proofs() {
    use scalars::{Scalar,Kind,Expression as E};
    let scalar=Scalar {kind:Kind::Sum,source:"events".into(),keys:vec![E::Column(0)],predicates:vec![],lookup:vec![E::Opaque("dependent checked lookup".into())],value:E::Column(1),dependent:true,non_null:true};
    assert!(scalar.same_bag(&scalar).proven);
    assert!(!scalar.compatible(&scalar).proven);
    let mut nullable=scalar.clone(); nullable.non_null=false;
    assert!(!nullable.same_bag(&nullable).proven);
    let mut opaque=scalar.clone(); opaque.predicates.push(E::Opaque("callback".into()));
    assert!(!opaque.same_bag(&opaque).proven);
}

#[test]
fn total_boolean_values_can_share_and_checked_arithmetic_keeps_its_contract() {
    use scalars::{Scalar,Kind,Expression as E};
    let scalar=Scalar {kind:Kind::Sum,source:"events".into(),keys:vec![E::Column(0)],predicates:vec![],lookup:vec![E::Column(0)],value:E::Apply("Greater".into(),vec![E::Column(1),E::Integer(0)]),dependent:false,non_null:true};
    assert!(scalar.value.non_null());
    assert!(scalar.compatible(&scalar).proven);
    let mut checked=scalar;checked.value=E::Apply("Add".into(),vec![E::Column(1),E::Integer(1)]);
    assert!(!checked.value.non_null());
    assert!(!checked.compatible(&checked).proven);
}
