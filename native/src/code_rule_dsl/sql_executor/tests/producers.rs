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
    assert!(super::super::super::facts::vtab::bound_source(&e.db, "reach").is_none());
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
    assert!(super::super::super::facts::vtab::bound_source(&e.db, "reach").is_none());
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
