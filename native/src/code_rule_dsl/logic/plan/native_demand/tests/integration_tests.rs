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
    };
    use crate::content_store::ContentStore;
    use std::{collections::BTreeSet, sync::Arc};

    // O-split-logic-tests: reads a crate above rerail-logic; compiled only once the split hosts it in the engine.

    #[cfg(any())]

    #[test]
    fn retained_semantic_results_support_sql_outputs_and_isolate_targets_and_runs() {
        let p = rr::parse(
            r#"
#include "core.rr"
.decl py_call_site(d:number,report:node,call:node,ref:node)
.input py_call_site
.decl wanted(n:node)
wanted(n) :- py_call_site(_,_,_,n).
.decl get_target(q:symbol)
.decl target_pool(q:symbol)
target_pool("requests.get").
target_pool("requests.post").
get_target(q) :- target_pool(q), q = "requests.get".
.decl post_target(q:symbol)
post_target(q) :- target_pool(q), q = "requests.post".
.decl gets(n:node,q:symbol)
.decl posts(n:node,q:symbol)
gets(n,q) :- python_qualified(n,q,wanted,get_target).
posts(n,q) :- python_qualified(n,q,wanted,post_target).
.output gets, posts
"#,
        )
        .unwrap();
        let path = crate::code_rule_dsl::fixtures::temp();
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
                let file = crate::code_rule_dsl::model::FileInput::new(
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
                    tape: None,
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
                if modes == "program" && !composed {
                    assert!(got.native.contains("wanted"), "a native demand projection must stay resident");
                    let published: i64 = db.query_row(
                        "SELECT count(*) FROM temp.sqlite_master WHERE name IN ('wanted', 'get_target', 'post_target')",
                        [], |r| r.get(0),
                    ).unwrap();
                    assert_eq!(published, 0, "resident native demands and targets must not be published to SQL");
                }
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
