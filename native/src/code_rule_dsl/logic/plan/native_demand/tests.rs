use super::*;

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::code_rule_dsl::logic::native::component;

    #[test]
    fn same_relation_reuses_keys_and_lifetime_does_not_cross_runs() {
        let db = Connection::open_in_memory().unwrap();
        let source = component::columns_source(
            vec!["c0".into()],
            vec![Some(vec![1, 1, 2, (2_i64 << 32) | 1])],
        );
        let mut demands = Demands::default();
        let first = demands.read(&db, "keys", "c0", Some(source)).unwrap();
        assert_eq!(first.as_slice(), &[1, 2, (2_i64 << 32) | 1]);
        // There is no SQL table: a second consumer must reuse the same snapshot.
        let second = demands.read(&db, "keys", "c0", None).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let narrower = component::columns_source(vec!["c0".into()], vec![Some(vec![2])]);
        assert_eq!(
            demands
                .read(&db, "narrower", "c0", Some(narrower))
                .unwrap()
                .as_slice(),
            &[2]
        );
        demands.forget("keys");
        let empty = component::columns_source(vec!["c0".into()], vec![Some(vec![])]);
        assert!(
            demands
                .read(&db, "keys", "c0", Some(empty))
                .unwrap()
                .is_empty()
        );
        let mut next_run = Demands::default();
        let other = component::columns_source(vec!["c0".into()], vec![Some(vec![9])]);
        assert_eq!(
            next_run
                .read(&db, "keys", "c0", Some(other))
                .unwrap()
                .as_slice(),
            &[9]
        );
    }

    #[test]
    fn sql_demands_are_read_once_and_cached_without_requiring_native_inputs() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE keys(c0 INTEGER); INSERT INTO keys VALUES(7),(7),(3)")
            .unwrap();
        let mut demands = Demands::default();
        let first = demands.read(&db, "keys", "c0", None).unwrap();
        db.execute_batch("DROP TABLE keys").unwrap();
        let second = demands.read(&db, "keys", "c0", None).unwrap();
        assert_eq!(first.as_slice(), &[3, 7]);
        assert!(Arc::ptr_eq(&first, &second));
    }
}

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
