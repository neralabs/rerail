//! Tests of this module that read what the engine registers as it loads (the languages, their `.rr` libraries,
//! the ECMA crate's functions): the engine's facade of the module mounts them.

#[test]
fn native_metadata_reads_pin_inputs_and_targets_through_their_consumers() {
    use crate::code_rule_dsl::logic::{front, plan, rr};
    use std::collections::HashSet;
    let p = rr::parse(r#"
#include "core.rr"
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
    let (sccs, _) = front::strata(&p).unwrap();
    let operator = p.natives.keys().next().unwrap();
    let consumer = sccs.iter().position(|s| s.contains(operator)).unwrap();
    // The target can be a view. Its underlying input must also survive until the native reader.
    for views in [HashSet::new(), HashSet::from(["targets".into()])] {
        let first = plan::first_reads(&p, &sccs, &views);
        let last = plan::last_reads(&p, &sccs, &views);
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
