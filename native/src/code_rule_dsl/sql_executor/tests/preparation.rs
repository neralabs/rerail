use super::scaling::profile;
use super::*;

#[test]
fn first_edit_uses_prepared_inputs_projections_and_reducers_with_bounded_work() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"event","columns":["id","g","v"],"rows":(0..100000).map(|i|vec![i,1,i]).collect::<Vec<_>>()} ).to_string()).unwrap();
    e.bind_relation(json!({"name":"owner","columns":["g"],"rows":[[1]]}).to_string())
        .unwrap();
    let source = e.inputs["event"].clone();
    assert!(e.live_inputs.contains_key("event"));
    let program = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl projected(id:number,g:number,v:number)\nprojected(i,g,v) :- event(i,g,v).\n.decl totals(g:number,n:number,lo:number)\ntotals(g,n,lo) :- owner(g), n=count : {event(_,g,_)}, lo=min v : {event(_,g,v)}.\n.output projected,totals\n";
    let initial: Value =
        serde_json::from_str(&e.run(program.into(), "python".into(), None).unwrap()).unwrap();
    assert_eq!(initial["preparation"]["initial_execution"],json!("retained"));
    assert_eq!(initial["preparation"]["projection_relations"],1);
    let projected = rows(&e,"SELECT rootpage FROM temp.sqlite_schema WHERE name='projected'");
    assert_eq!(projected.as_array().unwrap().len(),1);
    e.set_limits(json!({"max_steps":30000}).to_string())
        .unwrap();
    let (result, work) = profile(&mut e, |e| {
        e.apply_changes(
            json!([{"name":"event","insert":[[100000,1,23]]}]).to_string(),
            None,
        )
    });
    let report: Value =
        serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
    assert_eq!(report["initializations"], json!([]));
    assert_eq!(report["migrated_inputs"], json!([]));
    assert_eq!(report["programs"][0]["migrated_views"], 0);
    assert!(Arc::ptr_eq(&source, &e.inputs["event"]));
    assert!(work.data_scans.get() < 1000, "{work:?}");
    assert_eq!(rows(&e, "SELECT * FROM totals"), json!([[1, 100001, 0]]));
    assert_eq!(rows(&e,"SELECT rootpage FROM temp.sqlite_schema WHERE name='projected'"),projected);
    eprintln!("first edit: {work:?}");
}

fn scratch_names(e: &CodeRuleSqlExecutor) -> Vec<String> {
    rows(e,"SELECT name FROM temp.sqlite_schema WHERE type='table' AND name GLOB '__rr_update_*' ORDER BY name").as_array().unwrap().iter().map(|r|r[0].as_str().unwrap().to_owned()).collect()
}

#[test]
fn warm_edits_reuse_scratch_schema_and_failures_preserve_empty_pooled_tables() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3]]));
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    for insert in [true, false, true, false] {
        let patch = if insert {
            json!([{"name":"edge","insert":[[10,11]]}])
        } else {
            json!([{"name":"edge","delete":[[10,11]]}])
        };
        e.apply_changes(patch.to_string(), None).unwrap();
    }
    let names = scratch_names(&e);
    assert!(!names.is_empty());
    let schema = rows(&e, "PRAGMA temp.schema_version");
    for insert in [true, false, true, false] {
        let patch = if insert {
            json!([{"name":"edge","insert":[[10,11]]}])
        } else {
            json!([{"name":"edge","delete":[[10,11]]}])
        };
        let report: Value =
            serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap();
        assert_eq!(report["scratch_tables_created"], 0, "{report}");
        assert_eq!(scratch_names(&e), names);
        assert_eq!(rows(&e, "PRAGMA temp.schema_version"), schema);
        for name in &names {
            assert_eq!(
                rows(&e, &format!("SELECT count(*) FROM {name}")),
                json!([[0]])
            );
        }
    }
    let old = rows(&e, "SELECT * FROM reach ORDER BY c0,c1");
    e.set_limits(json!({"max_steps":50}).to_string()).unwrap();
    assert!(
        e.apply_changes(json!([{"name":"edge","delete":[[1,2]]}]).to_string(), None)
            .is_err()
    );
    e.set_limits("{}".into()).unwrap();
    assert_eq!(rows(&e, "SELECT * FROM reach ORDER BY c0,c1"), old);
    assert_eq!(scratch_names(&e), names);
    for name in &names {
        assert_eq!(
            rows(&e, &format!("SELECT count(*) FROM {name}")),
            json!([[0]])
        );
    }
    e.apply_changes(json!([{"name":"edge","delete":[[1,2]]}]).to_string(), None)
        .unwrap();
    e.reset().unwrap();
    assert!(scratch_names(&e).is_empty());
}
