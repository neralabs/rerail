use super::*;

// Count every executed statement, including schema operations and batched helpers.
fn preparation_work(e: &mut CodeRuleSqlExecutor, program: &str) -> (Result<String>, u64) {
    if let Some(before) = e.sql_profile_totals() {
        let result = e.lower(program.into(), "python".into());
        return (result, e.sql_profile_totals().unwrap()[0] - before[0]);
    }
    use rusqlite::ffi;
    unsafe extern "C" fn trace(
        _: u32,
        context: *mut std::ffi::c_void,
        _statement: *mut std::ffi::c_void,
        _elapsed: *mut std::ffi::c_void,
    ) -> i32 {
        let count = unsafe { &*context.cast::<std::cell::Cell<u64>>() };
        count.set(count.get() + 1);
        0
    }
    struct Guard(*mut ffi::sqlite3);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                ffi::sqlite3_trace_v2(self.0, 0, None, std::ptr::null_mut());
            }
        }
    }
    let mut count = Box::new(std::cell::Cell::new(0u64));
    let db = unsafe { e.db.handle() };
    let code = unsafe {
        ffi::sqlite3_trace_v2(
            db,
            ffi::SQLITE_TRACE_PROFILE as u32,
            Some(trace),
            (&mut *count as *mut std::cell::Cell<u64>).cast(),
        )
    };
    assert_eq!(code, ffi::SQLITE_OK);
    let _guard = Guard(db);
    let result = e.lower(program.into(), "python".into());
    (result, count.get())
}

#[test]
fn producer_preparation_avoids_schema_submissions_and_cleans_failed_queries() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3], [3, 4]]));
    let mut program = CLOSURE.to_owned();
    for i in 0..40 {
        program.push_str(&format!(
            ".decl extra{i}(v:number)\nextra{i}(x) :- reach(x,_).\n.output extra{i}\n"
        ));
    }
    program.push_str(".decl total(n:number)\n.input total(IO=sql,query=\"SELECT count(*) FROM rr_rel_reach\")\n.output total\n");
    let (result, submissions) = preparation_work(&mut e, &program);
    let lowered: Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(
        submissions <= 2,
        "Preparing 43 relations must not submit per-relation schema helpers: {submissions}"
    );
    assert!(
        lowered["program"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["output"] == "total" && s["reads"] == json!(["reach"]))
    );
    assert!(e.db.prepare("SELECT * FROM reach").is_err());
    assert!(e.db.prepare("SELECT * FROM rr_rel_reach").is_err());
    assert!(super::super::super::facts::vtab::bound_source(&*e.db, "reach").is_none());
    let invalid = program.replace(
        "count(*) FROM rr_rel_reach",
        "missing_column FROM rr_rel_reach",
    );
    let (result, submissions) = preparation_work(&mut e, &invalid);
    assert!(result.unwrap_err().reason.contains("missing_column"));
    assert!(
        submissions <= 2,
        "Failed preparation must not execute schema cleanup: {submissions}"
    );
    assert!(e.db.prepare("SELECT * FROM reach").is_err());
    assert!(super::super::super::facts::vtab::bound_source(&*e.db, "reach").is_none());
    e.run(program.clone(), "python".into(), None).unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[6]]));
    e.apply_changes(json!([{"name":"edge","insert":[[4,5]]}]).to_string(), None)
        .unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[10]]));
    // Preparation leaves live roots and typed catalog views intact, even on failure.
    let before = rows(&mut e, "SELECT src,dst FROM rr_rel_reach ORDER BY src,dst");
    assert!(preparation_work(&mut e, &invalid).0.is_err());
    assert_eq!(
        rows(&mut e, "SELECT src,dst FROM rr_rel_reach ORDER BY src,dst"),
        before
    );
    e.lower(program, "python".into()).unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[10]]));
}

#[test]
fn producer_preparation_uses_current_declared_names_without_replacing_catalog_views() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2]]));
    e.run(CLOSURE.into(), "python".into(), None).unwrap();
    assert_eq!(
        rows(&mut e, "SELECT src,dst FROM rr_rel_reach"),
        json!([[1, 2]])
    );
    let program = CLOSURE.replace(
        "reach(src: number, dst: number)",
        "reach(first: number, last: number)",
    ) + ".decl copied(v:number)\n.input copied(IO=sql,query=\"SELECT last FROM rr_rel_reach\")\n.output copied\n";
    let lowered: Value = serde_json::from_str(&e.lower(program, "python".into()).unwrap()).unwrap();
    assert!(
        lowered["program"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["output"] == "copied" && s["reads"] == json!(["reach"]))
    );
    assert_eq!(
        rows(&mut e, "SELECT src,dst FROM rr_rel_reach"),
        json!([[1, 2]])
    );
}

#[test]
fn producers_keep_checked_regions_retained_through_chains_and_external_edits() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3], [3, 4]]));
    e.bind_relation(json!({"name":"limits","columns":["n"],"rows":[[2]]}).to_string())
        .unwrap();
    let program = CLOSURE.to_owned()
        + r#"
.decl fanout(src:number,n:number)
.input fanout(IO=sql,query="SELECT src,count(*) FROM rr_rel_reach GROUP BY src")
.decl cap(n:number)
.input cap(IO=sql,query="SELECT max(n) FROM limits")
.decl selected(src:number)
selected(s) :- fanout(s,n),cap(c),n>=c.
.decl summary(n:number)
.input summary(IO=sql,query="SELECT count(*) FROM selected")
.decl absent(src:number)
absent(s) :- fanout(s,_),!selected(s).
.output fanout,cap,selected,summary,absent
"#;
    let result: Value =
        serde_json::from_str(&e.run(program.clone(), "python".into(), None).unwrap()).unwrap();
    assert_eq!(result["preparation"]["initial_execution"], "retained");
    assert!(e.executions[0].temporal.contains_key(&0));
    let trace = e.executions[0].temporal[&0].explain();
    assert_eq!(trace["acyclic_stages"]["fanout"]["proven"], false);
    for patch in [
        json!([{"name":"edge","insert":[[4,5]]}]),
        json!([{"name":"limits","insert":[[3]],"delete":[[2]]}]),
        json!([{"name":"edge","delete":[[1,2],[2,3],[3,4],[4,5]]}]),
        json!([{"name":"edge","insert":[[1,2],[2,3],[3,4]]}]),
        json!([{"name":"limits","insert":[[2]],"delete":[[3]]}]),
    ] {
        let report: Value =
            serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap();
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        let (_dir, mut oracle) = executor();
        oracle.cold_oracle = true;
        edge(&mut oracle, rows(&mut e, "SELECT * FROM edge"));
        oracle
            .bind_relation(
                json!({"name":"limits","columns":["n"],"rows":rows(&mut e,"SELECT * FROM limits")})
                    .to_string(),
            )
            .unwrap();
        oracle.run(program.clone(), "python".into(), None).unwrap();
        for (r, width) in &e.executions[0].logical_relations.as_ref().clone() {
            let order = (1..=*width)
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let query = format!("SELECT * FROM {} ORDER BY {order}", quote(r));
            assert_eq!(rows(&mut e, &query), rows(&mut oracle, &query), "{r}");
        }
    }
}

#[test]
fn producer_boundaries_publish_inputs_once_and_rollback_early_publication() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3]]));
    let fail = Arc::new(AtomicBool::new(false));
    let failed = fail.clone();
    e.connection()
        .create_scalar_function(
            "producer_guard",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |ctx| {
                if failed.load(Ordering::Relaxed) {
                    Err(rusqlite::Error::UserFunctionError(
                        "producer failure".into(),
                    ))
                } else {
                    ctx.get::<i64>(0)
                }
            },
        )
        .unwrap();
    let program = CLOSURE.to_owned()
        + ".decl total(n:number)\n.input total(IO=sql,query=\"SELECT producer_guard(count(*)) FROM temp.reach\")\n.output total\n";
    e.run(program, "python".into(), None).unwrap();
    assert!(e.executions[0].temporal.contains_key(&0));
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[3]]));
    e.connection().execute_batch("CREATE TEMP TABLE audit(kind TEXT,a INTEGER,b INTEGER); CREATE TEMP TRIGGER reach_insert AFTER INSERT ON reach BEGIN INSERT INTO audit VALUES('insert',new.c0,new.c1); END; CREATE TEMP TRIGGER reach_delete AFTER DELETE ON reach BEGIN INSERT INTO audit VALUES('delete',old.c0,old.c1); END;").unwrap();
    let before = rows(&mut e, "SELECT * FROM reach ORDER BY c0,c1");
    let patch = json!([{"name":"edge","insert":[[3,4]]}]).to_string();
    fail.store(true, Ordering::Relaxed);
    assert!(
        e.apply_changes(patch.clone(), None)
            .unwrap_err()
            .reason
            .contains("producer failure")
    );
    assert_eq!(rows(&mut e, "SELECT * FROM reach ORDER BY c0,c1"), before);
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[3]]));
    assert_eq!(rows(&mut e, "SELECT count(*) FROM audit"), json!([[0]]));
    assert_eq!(
        rows(&mut e, "SELECT count(*) FROM edge WHERE src=3"),
        json!([[0]])
    );
    fail.store(false, Ordering::Relaxed);
    e.apply_changes(patch, None).unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[6]]));
    assert_eq!(
        rows(&mut e, "SELECT kind,count(*) FROM audit GROUP BY kind"),
        json!([["insert", 3]])
    );
    e.connection().execute_batch("DELETE FROM audit").unwrap();
    e.apply_changes(json!([{"name":"edge","delete":[[3,4]]}]).to_string(), None)
        .unwrap();
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[3]]));
    assert_eq!(
        rows(&mut e, "SELECT kind,count(*) FROM audit GROUP BY kind"),
        json!([["delete", 3]])
    );
}

#[test]
fn producer_reads_of_projection_views_keep_the_projection_gate() {
    let (_dir, mut e) = executor();
    edge(&mut e, json!([[1, 2], [2, 3], [3, 4]]));
    let program = ".decl edge(src:number,dst:number)\n.input edge\n.decl projected(src:number)\nprojected(s) :- edge(s,_).\n.decl total(n:number)\n.input total(IO=sql,query=\"SELECT count(*) FROM rr_rel_projected\")\n.output total\n";
    e.run(program.into(), "python".into(), None).unwrap();
    assert!(e.executions[0].temporal.contains_key(&0));
    assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[3]]));
    for (patch, count) in [
        (json!([{"name":"edge","insert":[[1,9]]}]), 3),
        (json!([{"name":"edge","insert":[[4,5]]}]), 4),
        (
            json!([{"name":"edge","delete":[[1,2],[1,9],[2,3],[3,4],[4,5]]}]),
            0,
        ),
        (json!([{"name":"edge","insert":[[7,8]]}]), 1),
    ] {
        e.apply_changes(patch.to_string(), None).unwrap();
        assert_eq!(rows(&mut e, "SELECT * FROM total"), json!([[count]]));
    }
}
