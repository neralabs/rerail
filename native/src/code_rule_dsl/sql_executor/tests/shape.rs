use super::*;

#[test]
fn isolated_scalar_paths_keep_private_state_out_of_sql_schema_and_recover_after_failure() {
    let prefix=".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n";
    let cases=[
        (".decl out(g:number,n:number,m:number)\nout(g,n,m) :- owner(g), n=count : {event(_,g,_)}, m=count : {event(_,n,_)}.\n", "SELECT * FROM out ORDER BY c0"),
        (".decl out(g:number,s:symbol)\nout(g,s) :- owner(g), s=join v by id sep \"|\" : {event(id,g,v)}.\n", "SELECT c0,sym_text(c1) FROM out ORDER BY c0"),
        (".decl out(g:number,n:number)\nout(g,n) :- owner(g), k=g+1, n=count : {event(_,k,_)}.\n", "SELECT * FROM out ORDER BY c0"),
        (".decl out(g:number,v:number)\nout(g,v/n) :- owner(g), v=min x : {event(_,g,x)}, n=count : {event(_,g,_)}.\n", "SELECT * FROM out ORDER BY c0"),
    ];
    for (rule,query) in cases {
        let (_dir,mut e)=executor();
        e.bind_relation(json!({"name":"event","columns":["id","g","v"],"rows":[[1,1,3],[2,1,7],[3,2,9]]}).to_string()).unwrap();
        e.bind_relation(json!({"name":"owner","columns":["g"],"rows":[[1],[2],[3]]}).to_string()).unwrap();
        let p=format!("{prefix}{rule}.output out\n");
        e.run(p.clone(),"python".into(),None).unwrap();
        assert!(e.executions[0].temporal.contains_key(&0));
        let before=rows(&mut e,query);
        e.set_limits(json!({"max_steps":1}).to_string()).unwrap();
        assert!(e.apply_changes(json!([{"name":"event","insert":[[99,1,99]]}]).to_string(),None).is_err());
        e.set_limits("{}".into()).unwrap();
        assert_eq!(rows(&mut e,query),before);
        assert_eq!(rows(&mut e,"SELECT count(*) FROM event WHERE id=99"),json!([[0]]));
        for patch in [
            json!([{"name":"event","delete":[[1,1,3]],"insert":[[4,1,11]]}]),
            json!([{"name":"event","delete":[[2,1,7],[3,2,9],[4,1,11]]}]),
            json!([{"name":"event","insert":[[5,3,13]]},{"name":"owner","delete":[[3]]}]),
            json!([{"name":"owner","insert":[[3]]}]),
        ] {
            e.apply_changes(patch.to_string(),None).unwrap();
            assert_eq!(rows(&mut e,"SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '__sql_state_*' OR name GLOB '__rr_update_*'"),json!([[0]]),"{rule}");
            let (_dir,mut oracle)=executor();
            oracle.cold_oracle=true;
            for (name,columns) in [("event",json!(["id","g","v"])),("owner",json!(["g"]))] {
                oracle.bind_relation(json!({"name":name,"columns":columns,"rows":rows(&mut e,&format!("SELECT * FROM {name}"))}).to_string()).unwrap();
            }
            oracle.run(p.clone(),"python".into(),None).unwrap();
            assert_eq!(rows(&mut e,query),rows(&mut oracle,query),"{rule}");
        }
    }
}

#[test]
fn independent_summary_transitions_consumers_and_wave_folds_share_one_phase() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["id","g","v"],"rows":[[1,1,3],[2,1,3],[3,2,7]]}).to_string()).unwrap();
    e.bind_relation(json!({"name":"owner","columns":["g"],"rows":[[1],[2],[3]]}).to_string()).unwrap();
    e.bind_relation(json!({"name":"blocked","columns":["g"],"rows":[]}).to_string()).unwrap();
    let p = ".decl seed(id:number,g:number,v:number)\n.decl owner(g:number)\n.decl blocked(g:number)\n.input seed,owner,blocked\n.decl totals(g:number,n:number,s:number,lo:number,hi:number)\ntotals(g,n,s,lo,hi) :- owner(g), n=count : {seed(_,g,_)}, s=sum v : {seed(_,g,v)}, lo=min v : {seed(_,g,v)}, hi=max v : {seed(_,g,v)}.\n.decl allowed(g:number)\nallowed(g) :- owner(g), !blocked(g).\n.output totals,allowed\n";
    e.run(p.into(),"python".into(),None).unwrap();
    for patch in [
        json!([{"name":"seed","delete":[[1,1,3]],"insert":[[4,1,11]]},{"name":"blocked","insert":[[2]]}]),
        json!([{"name":"seed","delete":[[2,1,3],[4,1,11],[3,2,7]]},{"name":"blocked","delete":[[2]]}]),
        json!([{"name":"seed","insert":[[5,3,13],[6,3,13]]},{"name":"owner","insert":[[4]]}]),
    ] {
        e.apply_changes(patch.to_string(),None).unwrap();
        let trace = e.executions[0].temporal[&0].explain();
        let phases = trace["frames"].as_array().unwrap().iter().map(|f|f["phase_batches"].as_u64().unwrap()).sum::<u64>();
        assert_eq!(phases,1,"Independent summaries, consumers, and folds need one common phase: {trace}");
        super::temporal::check(&mut e,p,&[("seed",&["id","g","v"]),("owner",&["g"]),("blocked",&["g"])],&["totals","allowed"]);
    }
}

#[test]
fn standalone_scalar_jobs_use_native_retained_support_without_private_ddl() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["v"],"rows":[[1],[2],[3]]}).to_string()).unwrap();
    e.run(".decl seed(v:number)\n.input seed\n.decl total(n:number)\ntotal(n) :- n=count : {seed(_)}.\n.output total\n".into(),"python".into(),None).unwrap();
    assert!(e.executions[0].temporal.contains_key(&0));
    assert_eq!(rows(&mut e,"SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '__sql_state_*' AND type='table'"),json!([[0]]));
    for (patch, expected) in [(json!([{"name":"seed","delete":[[1],[2],[3]]}]),json!([[0]])),
        (json!([{"name":"seed","insert":[[5],[7]]}]),json!([[2]]))] {
        e.apply_changes(patch.to_string(),None).unwrap();
        assert_eq!(rows(&mut e,"SELECT * FROM total"),expected);
    }
}

#[test]
fn builtin_lattice_scalar_supports_keep_counts_retractions_and_readonly_storage() {
    for lattice in ["flat","min","max"] {
        let (_dir, mut e) = executor();
        e.bind_relation(json!({"name":"seed","columns":["id","g","v"],"rows":[[1,1,3],[2,1,3],[3,2,7]]}).to_string()).unwrap();
        e.bind_relation(json!({"name":"blocked","columns":["g"],"rows":[]}).to_string()).unwrap();
        let p=format!(".decl seed(id:number,g:number,v:number)\n.decl blocked(g:number)\n.input seed,blocked\n.decl r(g:number,v:{lattice}<>) widen 4\nr(g,v) :- seed(_,g,v), !blocked(g).\nr(g,v) :- r(g,v).\n.output r\n");
        e.run(p.clone(),"python".into(),None).unwrap();
        assert_eq!(rows(&mut e,"SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '__sql_state_*' AND type='table'"),json!([[0]]));
        for patch in [
            json!([{"name":"blocked","insert":[[1]]}]),
            json!([{"name":"seed","delete":[[1,1,3]]}]),
            json!([{"name":"blocked","delete":[[1]]}]),
            json!([{"name":"seed","delete":[[2,1,3],[3,2,7]]}]),
            json!([{"name":"seed","insert":[[4,1,9],[5,1,9],[6,2,11]]}]),
        ] {
            e.apply_changes(patch.to_string(),None).unwrap();
            super::temporal::check(&mut e,&p,&[("seed",&["id","g","v"]),("blocked",&["g"])],&["r"]);
        }
    }
}

#[test]
fn staged_predecessors_keep_secondary_probes_for_sparse_consumers() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["id","g"],"rows":(0..10000).map(|i|vec![i,i%100]).collect::<Vec<_>>()} ).to_string()).unwrap();
    e.bind_relation(json!({"name":"wanted","columns":["g"],"rows":[[7]]}).to_string()).unwrap();
    let p = ".decl seed(id:number,g:number)\n.decl wanted(g:number)\n.input seed,wanted\n.decl a(id:number,g:number)\na(id,g) :- seed(id,g).\n.decl b(id:number,g:number)\nb(id,g) :- wanted(g),a(id,g).\n.decl c(id:number)\nc(id) :- b(id,_).\n.output c\n";
    e.run(p.into(),"python".into(),None).unwrap();
    let trace = e.executions[0].temporal[&0].explain();
    let a = trace["frames"][0]["operators"].as_array().unwrap().iter().find(|op|op["relation"]=="a").unwrap();
    let root = a["result_root"].as_str().unwrap();
    let source = crate::code_rule_dsl::facts::vtab::bound_source(e.connection(),root).unwrap();
    assert!(source.indexed(1),"Staged values must retain the declared probe on the group column");
    assert_eq!(rows(&mut e,"SELECT count(*) FROM c"),json!([[100]]));
    e.apply_changes(json!([{"name":"wanted","insert":[[8]]}]).to_string(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT count(*) FROM c"),json!([[200]]));
    assert_eq!(rows(&mut e,"SELECT count(*) FROM c WHERE c0%100 NOT IN(7,8)"),json!([[0]]));
}

#[test]
fn isolated_checked_phases_preserve_overflow_rollback_and_retry() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["n","v"],"rows":[[1,3]]}).to_string()).unwrap();
    let p = ".decl seed(n:number,v:number)\n.input seed\n.decl out(n:number,v:flat<>)\nout(n,@flat_bin(\"*\",v,2)) :- seed(n,v).\nout(n,v) :- out(n,v).\n.output out\n";
    e.run(p.into(),"python".into(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT * FROM out"),json!([[1,6]]));
    assert!(e.explain_value().to_string().contains("isolated_checked_support_phase"));
    let patch = json!([{"name":"seed","delete":[[1,3]],"insert":[[1,i64::MIN]]}]);
    let error = e.apply_changes(patch.to_string(),None).unwrap_err();
    assert!(error.reason.contains("overflow"),"{error}");
    assert_eq!(rows(&mut e,"SELECT * FROM seed"),json!([[1,3]]));
    assert_eq!(rows(&mut e,"SELECT * FROM out"),json!([[1,6]]));
    e.apply_changes(json!([{"name":"seed","delete":[[1,3]],"insert":[[1,4]]}]).to_string(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT * FROM out"),json!([[1,8]]));
}

#[test]
fn acyclic_pipeline_preserves_deltas_across_sql_batch_boundaries() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["v"],"rows":[[1],[2]]}).to_string()).unwrap();
    let mut p = String::from(".decl seed(v:number)\n.input seed\n");
    for id in 0..90 {
        let parent = if id == 0 { "seed".to_owned() } else { format!("r{}", id-1) };
        p.push_str(&format!(".decl r{id}(v:number)\nr{id}(v) :- {parent}(v).\n"));
    }
    p.push_str(".output r0,r44,r89\n");
    e.run(p.clone(), "python".into(), None).unwrap();
    assert!(e.executions[0].temporal[&0].frames() <= 2);
    for patch in [json!([{"name":"seed","delete":[[1]]}]),json!([{"name":"seed","delete":[[2]]}]),json!([{"name":"seed","insert":[[7],[9]]}])] {
        e.apply_changes(patch.to_string(),None).unwrap();
        for rel in ["r0","r44","r89"] {
            assert_eq!(rows(&mut e,&format!("SELECT * FROM {rel} ORDER BY c0")), rows(&mut e,"SELECT * FROM seed ORDER BY v"));
        }
    }
    let explained = e.executions[0].temporal[&0].explain();
    assert!(explained["dependency_components"]["recursive"].as_array().unwrap().is_empty());
    assert!(explained["frames"].as_array().unwrap().iter().any(|f|f["phase_batches"].as_u64().unwrap()>1));
}

#[test]
fn collapsed_acyclic_chains_keep_sparse_edits_bounded() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["v"],"rows":(0..10000).map(|i|vec![i]).collect::<Vec<_>>()} ).to_string()).unwrap();
    let p = ".decl seed(v:number)\n.input seed\n.decl a(v:number)\na(v) :- seed(v).\n.decl b(v:number)\nb(v) :- a(v).\n.decl c(v:number)\nc(v) :- b(v).\n.output a,b,c\n";
    e.run(p.into(),"python".into(),None).unwrap();
    assert!(e.executions[0].temporal[&0].frames() <= 2);
    assert_eq!(rows(&mut e, "SELECT name FROM temp.sqlite_schema WHERE name GLOB '__it*' AND type='table'"), json!([]));
    e.set_limits(json!({"max_steps":30000}).to_string()).unwrap();
    for patch in [json!([{"name":"seed","delete":[[9999]]}]), json!([{"name":"seed","insert":[[10001]]}])] {
        let (result, work) = super::scaling::profile(&mut e, |e| e.apply_changes(patch.to_string(),None));
        result.unwrap_or_else(|err| panic!("Sparse stage update exceeded its SQL work bound: {err}; VM steps {}",work.vm.get()));
        assert!(work.data_scans.get() < 1000, "Sparse stage update scanned {} rows",work.data_scans.get());
        eprintln!("Sparse acyclic edit: {} VM steps, {} scanned rows",work.vm.get(),work.data_scans.get());
    }
    e.set_limits(json!({}).to_string()).unwrap();
    assert_eq!(rows(&mut e,"SELECT count(*),max(c0) FROM c"),json!([[10000,10001]]));
}

#[test]
fn acyclic_stages_share_sql_dependencies_and_preserve_incremental_set_semantics() {
    let (_dir, mut e) = executor();
    for (name, data) in [("seed",json!([[1,7],[2,7],[3,9]])), ("blocked",json!([]))] {
        e.bind_relation(json!({"name":name,"columns":if name=="seed" {vec!["id","v"]} else {vec!["v"]},"rows":data}).to_string()).unwrap();
    }
    let p = ".decl seed(id:number,v:number)\n.decl blocked(v:number)\n.input seed,blocked\n.decl a(v:number)\na(v) :- seed(_,v).\n.decl b(v:number)\nb(v) :- a(v).\n.decl c(v:number)\nc(v) :- b(v).\n.decl d(v:number)\nd(v) :- c(v), !blocked(v).\n.decl total(n:number)\ntotal(n) :- n=count : {d(_)}.\n.output a,b,c,d,total\n";
    e.run(p.into(), "python".into(), None).unwrap();
    let trace = &e.executions[0].temporal[&0];
    assert!(trace.frames() <= 4, "Acyclic dependency depth must not become wave depth: {}", trace.explain());
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[2]]));
    for patch in [
        json!([{"name":"seed","delete":[[1,7]]}]),
        json!([{"name":"blocked","insert":[[7]]}]),
        json!([{"name":"seed","delete":[[2,7],[3,9]]}]),
        json!([{"name":"seed","insert":[[4,7],[5,11]]}]),
        json!([{"name":"blocked","delete":[[7]]}]),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        super::temporal::check(&mut e, p, &[("seed", &["id","v"]), ("blocked", &["v"])], &["a","b","c","d","total"]);
    }
    assert!(e.executions[0].temporal[&0].explain()["acyclic_stages"]["c"]["proven"].as_bool().unwrap());
}

#[test]
fn suspended_contexts_keep_external_scalar_journals_until_reactivation() {
    let (_dir,mut e)=executor();
    e.bind_relation(json!({"name":"owner","columns":["g"],"rows":[[1]]}).to_string()).unwrap();
    e.bind_relation(json!({"name":"event","columns":["g","v"],"rows":[[1,7]]}).to_string()).unwrap();
    let p=".decl owner(g:number)\n.decl event(g:number,v:number)\n.input owner,event\n.decl live(g:number)\nlive(g) :- owner(g).\nlive(g) :- live(g).\n.decl amount(g:number,n:number)\namount(g,n) :- live(g), n=count : {event(g,_)}.\n.output live,amount\n";
    e.run(p.into(),"python".into(),None).unwrap();
    for (patch,expected) in [
        (json!([{"name":"owner","delete":[[1]]},{"name":"event","insert":[[1,9]]}]),json!([])),
        (json!([{"name":"owner","insert":[[1]]}]),json!([[1,2]])),
        (json!([{"name":"event","delete":[[1,7]]}]),json!([[1,1]])),
        (json!([{"name":"owner","delete":[[1]]},{"name":"event","delete":[[1,9]]}]),json!([])),
        (json!([{"name":"owner","insert":[[1]]}]),json!([[1,0]])),
    ] {
        e.apply_changes(patch.to_string(),None).unwrap();
        assert_eq!(rows(&mut e,"SELECT * FROM amount ORDER BY c0"),expected);
    }
}


#[test]
fn positive_scalar_sources_restore_identical_inputs_and_later_deltas() {
    let (_dir,mut e)=executor();
    let seed=json!([[1,7],[1,9],[2,11]]);
    e.bind_relation(json!({"name":"seed","columns":["g","v"],"rows":seed}).to_string()).unwrap();
    let p=".decl seed(g:number,v:number)\n.input seed\n.decl data(g:number,v:number)\ndata(g,v) :- seed(g,v).\ndata(g,v) :- data(g,v).\n.decl amount(g:number,n:number)\namount(g,n) :- data(g,_), n=count : {data(g,_)}.\n.decl single(g:number,v:number)\nsingle(g,v) :- amount(g,1), data(g,v).\n.output data,amount,single\n";
    e.run(p.into(),"python".into(),None).unwrap();
    assert!(e.executions[0].temporal.contains_key(&0));
    for (patch,amount,single) in [
        (json!([{"name":"seed","delete":seed}]),json!([]),json!([])),
        (json!([{"name":"seed","insert":seed}]),json!([[1,2],[2,1]]),json!([[2,11]])),
        (json!([{"name":"seed","insert":[[1,13]]}]),json!([[1,3],[2,1]]),json!([[2,11]])),
    ] {
        e.apply_changes(patch.to_string(),None).unwrap();
        assert_eq!(rows(&mut e,"SELECT * FROM amount ORDER BY c0"),amount);
        assert_eq!(rows(&mut e,"SELECT * FROM single ORDER BY c0,c1"),single);
    }
}


#[test]
fn empty_native_inputs_clear_nonscalar_caches_without_losing_scalar_cursors() {
    let (_dir,mut e)=executor();
    e.bind_snapshot(json!({"files":(0..1000).map(|i|json!({"path":format!("f{i}.py"),"language":"python","source":format!("def f{i}(x):\n    return x\n")})).collect::<Vec<_>>()} ).to_string()).unwrap();
    let p=".decl name(n:node,s:symbol)\n.input name\n.decl labels(s:symbol)\nlabels(@cat(s,\"!\")) :- name(_,s).\nlabels(s) :- labels(s).\n.decl total(n:number)\ntotal(n) :- n=count : {name(_,_)}.\n.output labels,total\n";
    e.run(p.into(),"python".into(),None).unwrap();
    let (result,work)=super::scaling::profile(&mut e,|e|e.apply_file_changes(json!({"delete":(0..1000).map(|i|format!("f{i}.py")).collect::<Vec<_>>()} ).to_string(),None));
    result.unwrap();
    assert_eq!(rows(&mut e,"SELECT count(*) FROM labels"),json!([[0]]));
    assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[0]]));
    assert!(!work.sql.borrow().keys().any(|sql|sql.starts_with("DELETE FROM __sql_state")&&sql.contains("_r0_contexts WHERE (c0,c1)")),"exactly empty native inputs must not delete nonscalar contexts one key at a time");
    let trace=e.executions[0].temporal[&0].explain();
    assert!(trace["frames"].as_array().unwrap().iter().any(|f|f["operators"].as_array().unwrap().iter().any(|o|o["empty_positive_body"]==true)));
    e.apply_file_changes(json!({"upsert":[{"path":"restored.py","language":"python","source":"def restored(x):\n    return x\n"}]}).to_string(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT sym_text(c0) FROM labels ORDER BY 1"),json!([["restored.!"],["x.!"]]));
    assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[3]]));
}


#[test]
fn large_waves_batch_frozen_inputs_and_roll_back_later_failures() {
    let (_dir, mut e) = executor();
    e.bind_relation(json!({"name":"seed","columns":["v"],"rows":[[1],[2]]}).to_string()).unwrap();
    let mut program = String::from(".decl seed(v:number)\n.input seed\n");
    for i in 0..90 {
        program.push_str(&format!(".decl r{i}(v:number)\nr{i}(v) :- seed(v).\n"));
    }
    // The recursive scalar makes this a retained whole-program wave schedule.
    program.push_str(".decl total(n:number)\ntotal(n) :- n=count : {seed(_)}.\ntotal(n) :- total(n), n=count : {seed(_)}.\n");
    program.push_str(&format!(".output {},total\n",(0..90).map(|i|format!("r{i}")).collect::<Vec<_>>().join(",")));
    e.run(program.clone(), "python".into(), None).unwrap();
    let trace = &e.executions[0].temporal[&0];
    assert!(trace.explain()["frames"].as_array().unwrap().iter().any(|f|f["phase_batches"].as_u64().unwrap()>1));
    assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[2]]));
    // This dependent program fails only after every earlier cache batch publishes.
    e.run(".decl total(n:number)\n.input total\n.decl guard(v:number)\nguard((n-2)+9223372036854775807) :- total(n).\n.output guard\n".into(),"python".into(),None).unwrap();
    assert!(e.apply_changes(json!([{"name":"seed","insert":[[3]]}]).to_string(),None).is_err());
    assert_eq!(rows(&mut e,"SELECT * FROM r89 ORDER BY c0"),json!([[1],[2]]));
    assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[2]]));
    e.apply_changes(json!([{"name":"seed","delete":[[1],[2]]}]).to_string(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[0]]));
    assert_eq!(rows(&mut e,"SELECT * FROM r0"),json!([]));
    e.apply_changes(json!([{"name":"seed","insert":[[7]]}]).to_string(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT * FROM r89"),json!([[7]]));
    assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[1]]));
}


#[test]
fn overlapping_replacements_use_set_differences_without_quadratic_old_row_scans() {
    let (_dir,mut e)=executor();
    for name in ["lefts","rights"] {
        e.bind_relation(json!({"name":name,"columns":["id","g"],"rows":(0..2048).map(|i|vec![i,i]).collect::<Vec<_>>()} ).to_string()).unwrap();
    }
    let program=".decl lefts(id:number,g:number)\n.decl rights(id:number,g:number)\n.input lefts,rights\n.decl r(g:number)\nr(g) :- lefts(_,g).\nr(g) :- rights(_,g).\nr(g) :- r(g), n=count : {rights(_,g)}, n>0.\n.output r\n";
    e.run(program.into(),"python".into(),None).unwrap();
    e.connection().execute_batch("CREATE TEMP TRIGGER unchanged_r_delete BEFORE DELETE ON r BEGIN SELECT RAISE(ABORT,'unchanged r'); END; CREATE TEMP TRIGGER unchanged_r_insert BEFORE INSERT ON r BEGIN SELECT RAISE(ABORT,'unchanged r'); END;").unwrap();
    let (result,work)=super::scaling::profile(&mut e, |e|e.apply_changes(json!([
        {"name":"lefts","delete":(0..1024).map(|i|vec![i,i]).collect::<Vec<_>>()},
        {"name":"rights","delete":(1024..2048).map(|i|vec![i,i]).collect::<Vec<_>>()}
    ]).to_string(),None));
    result.unwrap();
    assert!(work.data_scans.get()<100_000,"dense key reconciliation must avoid per-key scans: {work:?}");
    assert_eq!(rows(&mut e,"SELECT count(*) FROM r"),json!([[2048]]));
    assert!(e.explain_value().to_string().contains("materialized key difference"));
    e.connection().execute_batch("DROP TRIGGER unchanged_r_delete; DROP TRIGGER unchanged_r_insert").unwrap();
    e.apply_changes(json!([
        {"name":"lefts","delete":(1024..2048).map(|i|vec![i,i]).collect::<Vec<_>>()},
        {"name":"rights","delete":(0..1024).map(|i|vec![i,i]).collect::<Vec<_>>()}
    ]).to_string(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT count(*) FROM r"),json!([[0]]));
}

#[test]
fn intermediate_facades_keep_indexes_cross_program_deltas_and_failure_rollback() {
    let (_dir,mut e)=executor();
    let seed=json!([[1,7],[2,7],[3,9]]);
    e.bind_relation(json!({"name":"seed","columns":["id","v"],"rows":seed}).to_string()).unwrap();
    let program=".decl seed(id:number,v:number)\n.input seed\n.decl mid(v:number,id:number)\nmid(v,id) :- seed(id,v).\n.decl out(v:number)\nout(v) :- mid(v,_).\n.output out\n";
    e.run(program.into(),"python".into(),None).unwrap();
    assert!(e.versions.owns("mid"));
    assert_eq!(rows(&mut e,"SELECT count(*) FROM temp.sqlite_schema WHERE name='mid'"),json!([[0]]));
    e.create_index("mid".into(),vec![1]).unwrap();
    assert_eq!(rows(&mut e,"SELECT * FROM mid WHERE c1=2"),json!([[7,2]]));
    e.run(".decl mid(v:number,id:number)\n.input mid\n.decl total(n:number)\ntotal(n) :- n=count : {mid(_,_)}.\n.output total\n".into(),"python".into(),None).unwrap();
    e.run(".decl total(n:number)\n.input total\n.decl guard(v:number)\nguard((n-3)+9223372036854775807) :- total(n).\n.output guard\n".into(),"python".into(),None).unwrap();
    assert!(e.apply_changes(json!([{"name":"seed","insert":[[4,11]]}]).to_string(),None).is_err());
    assert_eq!(rows(&mut e,"SELECT * FROM mid ORDER BY c1"),json!([[7,1],[7,2],[9,3]]));
    assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[3]]));
    for (patch,mid,out,total) in [
        (json!([{"name":"seed","delete":[[1,7]]}]),json!([[7,2],[9,3]]),json!([[7],[9]]),2),
        (json!([{"name":"seed","delete":[[2,7],[3,9]]}]),json!([]),json!([]),0),
        (json!([{"name":"seed","insert":[[5,13]]}]),json!([[13,5]]),json!([[13]]),1),
    ] {
        e.apply_changes(patch.to_string(),None).unwrap();
        assert_eq!(rows(&mut e,"SELECT * FROM mid ORDER BY c1"),mid);
        assert_eq!(rows(&mut e,"SELECT * FROM out ORDER BY c0"),out);
        assert_eq!(rows(&mut e,"SELECT * FROM total"),json!([[total]]));
    }
    assert_eq!(rows(&mut e,"SELECT c0 FROM mid WHERE c1=5"),json!([[13]]));
    e.run(program.into(),"python".into(),None).unwrap();
    assert_eq!(rows(&mut e,"SELECT * FROM mid"),json!([[13,5]]));
}

#[test]
fn projections_and_mixed_scalars_use_one_retained_graph_with_exact_supports() {
    for recursive in [false, true] {
        let (_dir, mut e) = executor();
        e.bind_relation(
            json!({"name":"seed","columns":["id","v"],"rows":[[0,7],[1,7],[2,9]]}).to_string(),
        )
        .unwrap();
        let p = format!(
            ".decl seed(id:number,v:number)\n.input seed\n.decl names(v:number)\nnames(v) :- seed(_,v).\n{}\n.decl projected(v:number,id:number)\nprojected(v,id) :- seed(id,v).\n.decl total(n:number)\ntotal(n) :- n=count : {{seed(_,_)}}.\n.output names,projected,total\n",
            if recursive {
                "names(v) :- names(v)."
            } else {
                ""
            }
        );
        let run: Value =
            serde_json::from_str(&e.run(p.clone(), "python".into(), None).unwrap()).unwrap();
        assert_eq!(run["preparation"]["initial_execution"], "retained", "{run}");
        assert!(e.executions[0].temporal.contains_key(&0));
        for name in ["names", "projected", "total"] {
            assert!(
                !e.generated
                    .iter()
                    .any(|(n, _)| n == &format!("new_{name}") || n == &format!("delta_{name}"))
            );
        }
        assert_eq!(
            rows(
                &mut e,
                "SELECT count(*) FROM temp.sqlite_schema WHERE name IN('names','projected','total')"
            ),
            json!([[3]])
        );
        e.create_index("projected".into(), vec![1]).unwrap();
        let public = rows(
            &mut e,
            "SELECT rootpage FROM temp.sqlite_schema WHERE name='projected'",
        );
        e.run(".decl total(n:number)\n.input total\n.decl guarded(v:number)\nguarded((n-3)+9223372036854775807) :- total(n).\n.output guarded\n".into(),"python".into(),None).unwrap();
        let old_guard = rows(&mut e, "SELECT * FROM guarded");
        // The second program fails after the first program has published its new public roots.
        assert!(
            e.apply_changes(json!([{"name":"seed","insert":[[3,11]]}]).to_string(), None)
                .is_err()
        );
        assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[3]]));
        assert_eq!(
            rows(&mut e, "SELECT * FROM names ORDER BY c0"),
            json!([[7], [9]])
        );
        assert_eq!(rows(&mut e, "SELECT * FROM guarded"), old_guard);
        if recursive {
            assert!(
                e.apply_changes(
                    json!([{"name":"seed","insert":[[3,11]]}]).to_string(),
                    Some(1)
                )
                .is_err()
            );
            assert_eq!(rows(&mut e, "SELECT count(*) FROM seed"), json!([[3]]));
            assert_eq!(
                rows(&mut e, "SELECT * FROM names ORDER BY c0"),
                json!([[7], [9]])
            );
        }
        for (patch, expected) in [
            (json!([{"name":"seed","delete":[[0,7]]}]), json!([[7], [9]])),
            (
                json!([{"name":"seed","delete":[[1,7]],"insert":[[1,9]]}]),
                json!([[9]]),
            ),
            (json!([{"name":"seed","delete":[[1,9],[2,9]]}]), json!([])),
            (json!([{"name":"seed","insert":[[3,11]]}]), json!([[11]])),
        ] {
            e.apply_changes(patch.to_string(), None).unwrap();
            assert_eq!(rows(&mut e, "SELECT * FROM names ORDER BY c0"), expected);
            assert_eq!(
                rows(
                    &mut e,
                    "SELECT rootpage FROM temp.sqlite_schema WHERE name='projected'"
                ),
                public
            );
            assert_eq!(
                rows(
                    &mut e,
                    "SELECT count(*) FROM temp.sqlite_schema WHERE name='__sql_index_projected_1'"
                ),
                json!([[1]])
            );
            let (_cold_dir, mut cold) = executor();
            cold.cold_oracle = true;
            cold.bind_relation(
                json!({"name":"seed","columns":["id","v"],"rows":rows(&mut e,"SELECT * FROM seed")})
                    .to_string(),
            )
            .unwrap();
            cold.run(p.clone(), "python".into(), None).unwrap();
            for sql in [
                "SELECT * FROM names ORDER BY c0",
                "SELECT * FROM projected ORDER BY c0,c1",
                "SELECT * FROM total",
            ] {
                assert_eq!(rows(&mut e, sql), rows(&mut cold, sql), "{sql}");
            }
        }
    }
}
