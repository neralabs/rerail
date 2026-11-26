use rerail_logic::code_rule_dsl::logic::{plan::{Model, Stats, Inputs, Constant, body::Placed},rr};
use std::collections::HashMap;
use std::sync::Arc;
#[test]
// The model is the planner's, held by one thread: its `Arc`s are as the planner makes them.
#[allow(clippy::arc_with_non_send_sync)]
fn current_kind_postings_override_biased_priors_and_preserve_missing_vs_zero() {
    let p=Placed::whole(rr::parse_version("").unwrap());
    let mut stats=Stats::empty_of(&p.fields().unwrap());
    stats.tree=rerail_logic::code_rule_dsl::logic::plan::tree_stats::TreeStats::of([("1".to_owned(),10_000,None),("2".to_owned(),100,None)],[]);
    stats.dfreq.insert(("kind",1),Arc::new(HashMap::from([(Constant::Int(1),10)]).into_iter().collect()));
    let mut model=Model { shapes:Default::default(),analysis:Default::default(),stats,ndv:Default::default(),docof:Default::default(),view_src:Default::default(),inputs:Arc::new(Inputs::file()),docs_by_shift:false,derived:["kind"].into_iter().collect() };
    assert_eq!(model.kind_frequency(&Constant::Int(1)),Some(10),"exact live counts must override sampled priors");
    assert_eq!(model.kind_frequency(&Constant::Int(2)),Some(0),"missing from a complete live histogram means zero");
    model.stats.card.insert("property",10_000.0);
    model.ndv.insert(("property",1),100.0);
    model.stats.fan.insert(("property",1),1000.0);
    assert_eq!(model.unknown_constant_rows("property",1),100.0,
        "a literal key is not drawn from the relation's rows; row-weighted fanout is a biased prior");
    model.stats.dfreq.clear();
    assert_eq!(model.kind_frequency(&Constant::Int(2)),Some(100),"unknown live counts may fall back to priors");
}
