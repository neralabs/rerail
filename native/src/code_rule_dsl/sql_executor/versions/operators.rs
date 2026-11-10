//! Retained rule state can live in versioned native tables while SQL evaluates its deltas.
use super::*;
#[test]
fn versioned_reducer_state_matches_fresh_sql_across_sparse_edits_and_forks() {
    let (_d, mut engine) = super::super::tests::executor();
    engine.legacy_initial=true;
    engine.bind_relation(json!({"name":"event","columns":["id","g","v"],"rows":(0..2000).map(|g|vec![g,g,g+1]).collect::<Vec<_>>()} ).to_string()).unwrap();
    engine.bind_relation(json!({"name":"owner","columns":["g"],"rows":(0..2000).map(|g|vec![g]).collect::<Vec<_>>()} ).to_string()).unwrap();
    let program = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl result(g:number,n:number,s:number,lo:number,hi:number)\nresult(g,n,s,lo,hi) :- owner(g), k=g+1, n=count : {event(_,k,_)}, s=sum v : {event(_,k,v)}, lo=min v : {event(_,k,v)}, hi=max v : {event(_,k,v)}.\n.output result\n";
    engine.run(program.into(), "python".into(), None).unwrap();
    let mut run = engine.executions[0].clone();
    let native = Store::default();
    let tx = native.transaction(&engine.db).unwrap();
    let support = run
        .objects
        .iter()
        .filter(|(n, _)| n.starts_with("__sql_state_"))
        .cloned()
        .collect::<Vec<_>>();
    for (name, table) in support {
        if engine.versions.owns(&name) {
            engine.versions.remove_all(&engine.db,&name).unwrap();
            continue;
        }
        engine
            .db
            .execute_batch(&format!(
                "DROP {} {}",
                if table { "TABLE" } else { "VIEW" },
                quote(&name)
            ))
            .unwrap();
    }
    run.objects.retain(|(n, _)| !n.starts_with("__sql_state_"));
    run.reductions = None;
    let mut work = maintenance::Work::new(&engine.db, &scratch::Pool::default());
    let mut potential = run.inputs.clone();
    reductions::prepare(&mut work, &mut run, &mut potential, Some(&native)).unwrap();
    for name in ["result", "new_result", "delta_result"] {
        let rows = super::tests::data(&engine.db, name);
        engine
            .db
            .execute_batch(&format!("DROP TABLE {}", quote(name)))
            .unwrap();
        native
            .create(&engine.db, name, Schema::integers(5, 5))
            .unwrap();
        let mut insert = engine
            .db
            .prepare(&format!(
                "INSERT INTO {} VALUES(?1,?2,?3,?4,?5)",
                quote(name)
            ))
            .unwrap();
        for row in rows {
            insert.execute(rusqlite::params_from_iter(row)).unwrap();
        }
    }
    let mut forked = super::fork::execution(
        &engine.db,
        &native,
        &run,
        "result",
        "fork_result",
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        native
            .version("__sql_state_result_heads")
            .full
            .as_ref()
            .unwrap(),
        native
            .version("__sql_state_fork_result_heads")
            .full
            .as_ref()
            .unwrap()
    ));
    assert!(
        run.objects
            .iter()
            .filter(|(n, _)| n.starts_with("__sql_state_"))
            .all(|(n, _)| native.contains(n))
    );
    let heads = run
        .objects
        .iter()
        .find(|(n, _)| n == "__sql_state_result_heads")
        .unwrap()
        .0
        .clone();
    native.fork(&engine.db, &heads, "saved_heads").unwrap();
    let saved = native.version("saved_heads");
    let source = engine.live_inputs["event"].clone();
    for (plus, minus) in [
        (vec![vec![2001, 17, 100]], vec![]),
        (vec![], vec![vec![17, 17, 18]]),
        (vec![], vec![vec![2001, 17, 100]]),
    ] {
        source.apply(&plus, &minus);
        let p = work.rows(3, &plus).unwrap();
        let m = work.rows(3, &minus).unwrap();
        let mut changes = BTreeMap::from([(
            "event".into(),
            maintenance::Delta {
                plus: p,
                minus: m,
                inserted: plus.len() as u64,
                deleted: minus.len() as u64,
            },
        )]);
        let mark = work.mark();
        let inputs = changes.clone();
        let fork_before = native.version("fork_result");
        let report = maintenance::maintain(&mut work, &mut run, &mut changes, 1024).unwrap();
        assert_eq!(report["refreshed_strata"], json!([]));
        assert!(report["context_rows"].as_u64().unwrap() < 3);
        let actual = super::super::tests::rows(&engine, "SELECT * FROM result WHERE c0=16");
        let expected = super::super::tests::rows(
            &engine,
            "SELECT 16,count(*),sum(v),min(v),max(v) FROM event WHERE g=17 HAVING count(*)>0",
        );
        assert_eq!(actual, expected);
        assert!(saved.same(&native.version("saved_heads")));
        assert!(fork_before.same(&native.version("fork_result")));
        work.rewind(mark).unwrap();
        let mark = work.mark();
        let mut inputs = inputs;
        maintenance::maintain(&mut work, &mut forked, &mut inputs, 1024).unwrap();
        assert!(
            native
                .version("result")
                .same(&native.version("fork_result"))
        );
        work.rewind(mark).unwrap();
    }
    assert!(!saved.same(&native.version(&heads)));
    work.cleanup().unwrap();
    tx.commit();
    native.close(&engine.db).unwrap();
}
