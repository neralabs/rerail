use serde_json::json;
    use super::*;
    #[test]
    fn equal_versions_and_cursors_reuse_frames_without_sql_phases() {
        let (_dir, mut e) = super::super::tests::executor();
        e.bind_relation(json!({"name":"seed","columns":["v"],"rows":[[1],[2]]}).to_string()).unwrap();
        e.run(".decl seed(v:number)\n.input seed\n.decl r(v:number)\nr(v) :- seed(v).\nr(v) :- r(v), n=count : {seed(_)}, n>0.\n.output r\n".into(),"python".into(),None).unwrap();
        let pool = e.scratch.clone();
        let mut work = Work::new(&e.db, &pool);
        work.versions = Some(&e.versions);
        let trace = e.executions[0].temporal.get_mut(&0).unwrap();
        assert!(trace.frames.iter().any(|f| !f.observation["recursive_components"].as_array().unwrap().is_empty()));
        let before = trace.frames.last().unwrap().versions(&e.versions);
        trace.replay(&mut work, &e.versions, &BTreeMap::new(), 100).unwrap();
        assert!(trace.frames.iter().all(|f| f.observation["reused"] == true), "{}", trace.explain());
        assert!(trace.frames.iter().all(|f| f.phase_batches == 0));
        let after = trace.frames.last().unwrap().versions(&e.versions);
        for (r, version) in before.published { assert!(version.same(&after.published[&r])); }
        work.cleanup().unwrap();
    }
