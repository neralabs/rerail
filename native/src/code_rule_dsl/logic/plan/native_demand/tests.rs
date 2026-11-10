use super::*;

#[cfg(test)]
mod tests {
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

#[cfg(test)]
mod integration_tests {
    use crate::code_rule_dsl::{
        facts::{
            core,
            vtab::{Modules, syntax},
        },
        logic::{
            native,
            plan::{self, Error, Stats, lower},
            rr,
        },
        model::FileInput,
    };
    use crate::content_store::ContentStore;
    use std::{collections::BTreeSet, sync::Arc};

    #[test]
    fn retained_semantic_results_support_sql_outputs_and_isolate_targets_and_runs() {
        let p = rr::parse(
            r#"
#include "core.rr"
.decl py_call_site(d:number,report:node,call:node,ref:node)
.input py_call_site
.decl wanted(n:node)
wanted(n) :- py_call_site(_,_,_,n), n >= 0.
.decl get_target(q:symbol)
get_target("requests.get").
.decl post_target(q:symbol)
post_target("requests.post").
.decl gets(n:node,q:symbol)
.decl posts(n:node,q:symbol)
gets(n,q) :- python_qualified(n,q,wanted,get_target).
posts(n,q) :- python_qualified(n,q,wanted,post_target).
.output gets, posts
"#,
        )
        .unwrap();
        let path = crate::code_rule_dsl::prefilter::tests::temp();
        let store = ContentStore::open(&path).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        native::register(&db).unwrap();
        native::register_symbols(&db).unwrap();
        let modules = Modules::default();
        let lowered = lower::lower(
            &p,
            crate::code_rule_dsl::language::dialect_by_label("python").unwrap(),
            &mut |s| {
                store
                    .intern_atoms(s)
                    .map_err(|e| Error::Plan(e.to_string()))
            },
        )
        .unwrap();
        for (modes,composed) in [("program",false), ("",false), ("program",true)] {
            let _plans=composed.then(||plan::KeepPlans::start(None));
            for (source, expected_gets) in [
                ("import requests as r\nr.get('x')\nr.post('x')\n", 1),
                ("import requests as r\nr.post('x')\n", 0),
            ] {
                native::component::forget_derived(&db);
                native::forget_inputs(&db);
                let file = FileInput::new(
                    "test.py".into(),
                    "python".into(),
                    Arc::new(source.into()),
                    None,
                    None,
                    "core",
                )
                .unwrap();
                let core::Built::Facts(facts) = core::build(&file).unwrap() else {
                    panic!("parse failed")
                };
                let facts = Arc::new(facts);
                let docs = vec![syntax::Document {
                    key: 7,
                    core: "test#core".into(),
                    digest: core::digest(&facts),
                    path: "test.py".into(),
                    facts,
                    guest: None,
                    module: None,
                }];
                let mut stats = Stats::empty(&lowered.program);
                for (name, source) in syntax::with_nearest(
                    &store,
                    docs,
                    &[],
                    &Default::default(),
                    crate::code_rule_dsl::typing::file_local::Binding::Cpu,
                    &syntax::Projection::core(),
                )
                .unwrap()
                {
                    let measured = syntax::statistics(source.as_ref());
                    stats.measured(
                        &name,
                        measured
                            .into_iter()
                            .map(|(c, n, d, q, qd)| (c, [n, d, q, qd])),
                        std::iter::empty(),
                    );
                    modules.bind(&db, &name, source).unwrap();
                }
                let got = plan::run_atoms_native_over(
                    &lowered,
                    &db,
                    "main",
                    stats,
                    BTreeSet::new(),
                    modes,
                )
                .unwrap();
                assert_eq!(got.outputs["gets"].len(), expected_gets);
                assert_eq!(got.outputs["posts"].len(), 1);
                assert!(
                    got.outputs["gets"]
                        .iter()
                        .all(|r| r.ends_with("\trequests.get"))
                );
                assert!(
                    got.outputs["posts"]
                        .iter()
                        .all(|r| r.ends_with("\trequests.post"))
                );
            }
        }
    }

    #[test]
    fn retained_fact_sets_materialize_all_unique_rows_and_do_not_adopt_next_run_inputs() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        native::register(&db).unwrap();
        native::component::retain(
            &db,
            "facts",
            vec![vec![7, 9], vec![7, 9], vec![8, 10]],
            vec![b'i'; 2],
        )
        .unwrap();
        assert_eq!(native::component::derived(&db, "facts").unwrap().len(), 2);
        assert!(native::component::adopted(&db).is_empty());
        db.execute_batch(
            "CREATE TABLE facts(c0 INTEGER,c1 INTEGER,PRIMARY KEY(c0,c1)) WITHOUT ROWID",
        )
        .unwrap();
        assert_eq!(native::component::materialize(&db, "facts").unwrap(), 2);
        let count: i64 = db
            .query_row("SELECT count(*) FROM facts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        native::component::release(&db, &["facts".into()]);
        assert!(native::component::derived(&db, "facts").is_none());
    }
}
