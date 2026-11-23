//! Tests of this module that read what the engine registers as it loads (the languages, their `.rerail` libraries,
//! the ECMA crate's functions): the engine's facade of the module mounts them.

#[test]
fn native_metadata_reads_pin_inputs_and_targets_through_their_consumers() {
    use crate::code_rule_dsl::logic::{plan, rr};
    use std::collections::HashSet;
    let p = rr::parse_version(r#"
#include "core.rerail"
.decl demand(d:number)
.input demand
.decl target_seed(q:symbol)
.input target_seed
.decl targets(q:symbol)
targets(q) :- target_seed(q).
.decl reached(c:node,f:node,r:node,o:node)
reached(c,f,r,o) :- python_candidates(c,f,r,o,demand,targets,"async_function","open").
.output reached
"#).unwrap();
    let strata = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::program_image::Strata>(&p).unwrap();
    let sccs: Vec<std::collections::BTreeSet<String>> = strata.sccs.iter().map(|scc| scc.iter().map(|&r| p.rel_name(r).to_owned()).collect()).collect();
    let natives = p.native_map().unwrap();
    let operator = natives.keys().next().unwrap();
    let consumer = sccs.iter().position(|s| s.contains(operator)).unwrap();
    // The target can be a view. Its underlying input must also survive until the native reader.
    for views in [HashSet::new(), HashSet::from(["targets".into()])] {
        let vs = plan::body::Versions::of_placed(plan::body::Placed::whole(p.clone()));
        let by = plan::keys_by_stratum(vs.placed(0), &sccs);
        let first = plan::first_reads(vs.whole(0), &vs, &sccs, &by, &views);
        let last = plan::last_reads(vs.whole(0), &vs, &sccs, &by, &views);
        for rel in ["demand", "targets"] {
            assert_eq!(first.get(rel), Some(&consumer), "first read of {rel}");
            assert_eq!(last.get(rel), Some(&consumer), "last read of {rel}");
        }
        if views.contains("targets") {
            assert_eq!(first["target_seed"], consumer);
            assert_eq!(last["target_seed"], consumer);
        }
    }
}
