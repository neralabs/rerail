use super::*;
const PROGRAM: &str = ".decl event(id:number,g:number,v:number,w:number)\n.decl owner(id:number,g:number,h:number)\n.decl watch(id:number,g:number)\n.input event,owner,watch\n.decl b(id:number,g:number,n:number,s:number)\nb(id,g,n,s) :- owner(id,_,g), n=count : {event(_,g,_,_)}, s=sum v : {event(_,g,v,_)}.\n.decl a(id:number,g:number,n:number,s:number,lo:number,hi:number)\na(id,g,n,s,lo,hi) :- owner(id,g,_), n=count : {event(_,g,_,_)}, s=sum v : {event(_,g,v,_)}, lo=min x : {event(_,g,x,_)}, hi=max y : {event(_,g,y,_)}.\na(id,g,n,s,lo,hi) :- watch(id,g), n=count : {event(_,g,_,_)}, s=sum v : {event(_,g,v,_)}, lo=min x : {event(_,g,x,_)}, hi=max y : {event(_,g,y,_)}.\n.decl other(id:number,g:number,s:number)\nother(id,g,s) :- owner(id,g,_), s=sum w : {event(_,g,_,w)}.\n.output a,b,other\n";
fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], values: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":values}).to_string())
        .unwrap();
}
fn compare(e: &mut CodeRuleSqlExecutor) {
    let (_d, mut oracle) = executor();
    oracle.cold_oracle = true;
    for (name, columns) in [
        ("event", &["id", "g", "v", "w"][..]),
        ("owner", &["id", "g", "h"][..]),
        ("watch", &["id", "g"][..]),
    ] {
        bind(
            &mut oracle,
            name,
            columns,
            rows(e, &format!("SELECT * FROM {name}")),
        );
    }
    oracle.run(PROGRAM.into(), "python".into(), None).unwrap();
    for output in ["a", "b", "other"] {
        let query = format!("SELECT * FROM {output} ORDER BY c0,c1");
        assert_eq!(rows(e, &query), rows(&mut oracle, &query));
    }
}
#[test]
fn different_rules_and_lookup_mappings_share_one_bag_and_keep_separate_values() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v", "w"],
        json!([[0, 1, 7, 9], [1, 1, 7, 9], [2, 1, 3, 4], [3, 2, -2, 8]]),
    );
    bind(
        &mut e,
        "owner",
        &["id", "g", "h"],
        json!([[0, 1, 2], [1, 2, 1], [2, 3, 3]]),
    );
    bind(&mut e, "watch", &["id", "g"], json!([[0, 1]]));
    e.run(PROGRAM.into(), "python".into(), None).unwrap();
    compare(&mut e);
    let explain: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    let pools = &explain["executions"][0]["scalar_storage"][0];
    assert_eq!(
        pools["analysis"]["bags"]["families"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let retained = explain["executions"][0]["retained"].as_array().unwrap();
    let group = |output: &str| {
        retained.iter().find(|s| s["relation"] == output).unwrap()["rules"][0]["summary_storage"][0]
            ["group"]
            .clone()
    };
    assert_eq!(group("a"), group("b"));
    assert_ne!(group("a"), group("other"));
    assert_eq!(
        rows(&mut e, "SELECT * FROM b WHERE c0=2"),
        json!([[2, 3, 0, 0]])
    );
    for patch in [
        json!([{"name":"event","delete":[[0,1,7,9]]}]),
        json!([{"name":"event","delete":[[1,1,7,9]]}]),
        json!([{"name":"watch","delete":[[0,1]]}]),
        json!([{"name":"owner","delete":[[0,1,2]],"insert":[[0,2,1]]},{"name":"event","insert":[[4,1,12,-4]]}]),
        json!([{"name":"event","delete":[[2,1,3,4],[4,1,12,-4]]}]),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        compare(&mut e);
    }
    // A later consumer fails after the shared bag and earlier output have changed.
    e.connection().execute_batch("CREATE TEMP TRIGGER stop_b BEFORE INSERT ON b BEGIN SELECT RAISE(ABORT,'stop shared consumer'); END").unwrap();
    let before = rows(&mut e, "SELECT * FROM event ORDER BY id");
    let patch = json!([{"name":"event","insert":[[5,1,6,2]]}]);
    assert!(
        e.apply_changes(patch.to_string(), None)
            .unwrap_err()
            .reason
            .contains("stop shared consumer")
    );
    assert_eq!(rows(&mut e, "SELECT * FROM event ORDER BY id"), before);
    e.connection().execute_batch("DROP TRIGGER stop_b").unwrap();
    e.apply_changes(patch.to_string(), None).unwrap();
    compare(&mut e);
}

#[test]
fn recursive_projection_proofs_remove_duplicate_stores_and_keep_computed_results() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "edge",
        &["a", "b", "w"],
        json!([[0, 1, 2], [1, 2, 3], [2, 1, 1], [4, 5, 1]]),
    );
    bind(&mut e, "root", &["n"], json!([[0], [4]]));
    let program = ".decl edge(a:number,b:number,w:number)\n.decl root(n:number)\n.input edge,root\n.decl distance(n:number,d:min<>) widen 4\ndistance(n,0) :- root(n).\ndistance(b,d+w) :- distance(a,d), edge(a,b,w).\n.output distance\n";
    e.run(program.into(), "python".into(), None).unwrap();
    let explain: Value = serde_json::from_str(&e.explain().unwrap()).unwrap();
    let mut reconstructed = false;
    let mut computed = false;
    let names = e
        .generated
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<BTreeSet<_>>();
    for trace in explain["executions"][0]["recursive"].as_array().unwrap() {
        for frame in trace["frames"].as_array().unwrap() {
            for op in frame["operators"].as_array().unwrap() {
                for state in op["analysis"].as_array().unwrap() {
                    for rule in state["rules"].as_array().unwrap() {
                        if rule["contributions_reconstructed"] == true {
                            reconstructed = true;
                            assert!(rule["contribution_store"].is_null());
                            assert!(names.contains(rule["context_store"].as_str().unwrap()));
                        } else if rule["contribution_storage"]["decision"]["proven"] == false {
                            computed = true;
                            assert!(
                                !rule["contribution_storage"]["decision"]["reason"]
                                    .as_str()
                                    .unwrap()
                                    .is_empty()
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(reconstructed && computed);
    for patch in [
        json!([{"name":"edge","delete":[[0,1,2]]}]),
        json!([{"name":"root","delete":[[4]]}]),
        json!([{"name":"edge","insert":[[0,2,1]]}]),
        json!([{"name":"root","delete":[[0]]}]),
        json!([{"name":"root","insert":[[0],[4]]}]),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        let (_d, mut oracle) = executor();
        oracle.cold_oracle = true;
        bind(
            &mut oracle,
            "edge",
            &["a", "b", "w"],
            rows(&mut e, "SELECT * FROM edge"),
        );
        bind(&mut oracle, "root", &["n"], rows(&mut e, "SELECT * FROM root"));
        oracle.run(program.into(), "python".into(), None).unwrap();
        assert_eq!(
            rows(&mut e, "SELECT * FROM distance ORDER BY c0"),
            rows(&mut oracle, "SELECT * FROM distance ORDER BY c0")
        );
    }
}

#[test]
fn walk_lowering_preserves_pools_shared_with_consumers_initialized_after_cold_execution() {
    let (_d, mut e) = executor();
    let files=json!({"files":[{"path":"a.py","language":"python","source":"def first():\n    work()\n"}]}).to_string();
    e.bind_snapshot(files.clone()).unwrap();
    bind(&mut e, "event", &["g", "v"], json!([[1, 3], [1, 7]]));
    bind(&mut e, "owner", &["g"], json!([[1]]));
    let program = ".decl event(g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl a(g:number,n:number,s:number)\na(g,n,t) :- owner(g), n=count : {event(g,_)}, s=sum v : {event(g,v)}, t=s+1.\n.decl b(g:number,n:number,s:number,lo:number,hi:number)\nb(g,n,s,lo,hi) :- owner(g), n=count : {event(g,_)}, s=sum v : {event(g,v)}, lo=min x : {event(g,x)}, hi=max y : {event(g,y)}.\n.decl node(d:number,n:node,k:symbol,s:number,e:number)\n.decl tape(n:node,p:node,s:number,d:number)\n.decl field(p:node,f:symbol,c:node)\n.decl name(n:node,s:symbol)\n.input node,tape,field,name\n.decl scope(k:symbol)\nscope(\"function_definition\").\n.decl walked(n:node,o:symbol)\nwalked(n,o) :- node(_,n,\"call\",_,_), nearest(n,a,scope), field(a,\"name\",f), name(f,o).\n.output a,b,walked\n";
    e.run(program.into(), "python".into(), None).unwrap();
    assert!(
        e.executions[0]
            .emitted
            .queries
            .iter()
            .flatten()
            .all(|q| q.ctes.is_empty())
    );
    for patch in [
        json!([{"name":"event","insert":[[1,11]]}]),
        json!([{"name":"event","delete":[[1,7]]}]),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        let (_d, mut oracle) = executor();
        oracle.cold_oracle = true;
        oracle.bind_snapshot(files.clone()).unwrap();
        bind(
            &mut oracle,
            "event",
            &["g", "v"],
            rows(&mut e, "SELECT * FROM event"),
        );
        bind(
            &mut oracle,
            "owner",
            &["g"],
            rows(&mut e, "SELECT * FROM owner"),
        );
        oracle.run(program.into(), "python".into(), None).unwrap();
        for output in ["a", "b", "walked"] {
            let query = format!("SELECT * FROM {output} ORDER BY c0");
            assert_eq!(rows(&mut e, &query), rows(&mut oracle, &query));
        }
    }
}

#[test]
fn dependent_consumers_share_bag_storage_with_independent_dependency_indexes() {
    let (_d, mut e) = executor();
    bind(&mut e,"event",&["g","v"],json!([[1,3],[1,5],[2,7]]));
    bind(&mut e,"owner",&["g"],json!([[1],[2]]));
    let p = ".decl event(g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl result(g:number,n:number,s:number,m:number)\nresult(g,n,s,m) :- owner(g), n=count : {event(g,_)}, s=sum v : {event(n,v)}, m=max v : {event(n,v)}.\n.output result\n";
    e.run(p.into(),"python".into(),None).unwrap();
    let explain:Value=serde_json::from_str(&e.explain().unwrap()).unwrap();
    let rules=&explain["executions"][0]["retained"][0]["rules"][0];
    assert_eq!(rules["summary_storage"][1]["group"],rules["summary_storage"][2]["group"],"{explain}");
    for patch in [json!([{"name":"event","insert":[[1,9]]}]),json!([{"name":"event","delete":[[1,3],[2,7]],"insert":[[3,11]]}])] {
        e.apply_changes(patch.to_string(),None).unwrap();
        let (_d,mut cold)=executor(); cold.cold_oracle=true;
        bind(&mut cold,"event",&["g","v"],rows(&mut e,"SELECT * FROM event"));
        bind(&mut cold,"owner",&["g"],rows(&mut e,"SELECT * FROM owner"));
        cold.run(p.into(),"python".into(),None).unwrap();
        assert_eq!(rows(&mut e,"SELECT * FROM result ORDER BY c0"),rows(&mut cold,"SELECT * FROM result ORDER BY c0"));
    }
}
