use rerail_logic::code_rule_dsl::logic::{plan::{Model, Stats, Inputs, Constant},front::ast::Program};
use std::collections::{HashMap,HashSet};
use std::sync::Arc;
#[test]
fn current_kind_postings_override_biased_priors_and_preserve_missing_vs_zero() {
    let p=Program::default();
    let mut stats=Stats::empty(&p);
    Arc::make_mut(&mut stats.kfreq).insert("1".into(),10_000);
    Arc::make_mut(&mut stats.kfreq).insert("2".into(),100);
    stats.dfreq.insert(("kind".into(),1),HashMap::from([(Constant::Int(1),10)]));
    let mut model=Model { shapes:Default::default(),analysis:Default::default(),input_set:Default::default(),stats,ndv:Default::default(),docof:Default::default(),view_src:Default::default(),inputs:Arc::new(Inputs::file()),docs_by_shift:false,derived:Arc::new(HashSet::from(["kind".into()])) };
    assert_eq!(model.kind_frequency(&Constant::Int(1)),Some(10),"exact live counts must override sampled priors");
    assert_eq!(model.kind_frequency(&Constant::Int(2)),Some(0),"missing from a complete live histogram means zero");
    model.stats.card.insert("property".into(),10_000.0);
    model.ndv.insert(("property".into(),1),100.0);
    model.stats.fan.insert(("property".into(),1),1000.0);
    assert_eq!(model.unknown_constant_rows("property",1),100.0,
        "a literal key is not drawn from the relation's rows; row-weighted fanout is a biased prior");
    model.stats.dfreq.clear();
    assert_eq!(model.kind_frequency(&Constant::Int(2)),Some(100),"unknown live counts may fall back to priors");
}
