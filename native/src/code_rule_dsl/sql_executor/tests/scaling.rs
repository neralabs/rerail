use super::*;

#[test]
fn walks_with_negative_and_aggregate_seed_and_output_match_fresh_runs() {
    let (_d, mut e) = executor();
    let files = json!({"files":[{"path":"a.py","language":"python","source":"def first():\n    work()\ndef second():\n    other()\n"}]}).to_string();
    e.bind_snapshot(files.clone()).unwrap();
    let p = ".decl node(d:number,n:node,k:symbol,s:number,e:number)\n.decl tape(n:node,p:node,s:number,d:number)\n.decl field(p:node,f:symbol,c:node)\n.decl name(n:node,s:symbol)\n.decl doc(d:number,p:symbol,m:symbol)\n.decl blocked(n:node)\n.decl marks(n:node,v:number)\n.input node,tape,field,name,doc,blocked,marks\n.decl scope(k:symbol)\nscope(\"function_definition\").\n.decl owns(n:node,o:symbol,c:number)\nowns(n,o,c) :- node(_,n,\"call\",_,_), !blocked(n), unused = min v : {marks(n,v)}, denominator = count : {marks(n,_)}, quotient = 1 / denominator, seed = count : {field(n,_,_)}, seed > 0, nearest(n,a,scope), field(a,\"name\",f), name(f,o), !blocked(a), c = count : {field(a,_,_)}.\n.output owns\n";
    bind(&mut e, "blocked", &["n"], json!([]));
    bind(&mut e, "marks", &["n", "v"], json!([]));
    e.run(p.into(), "python".into(), None).unwrap();
    assert!(
        e.executions[0]
            .emitted
            .queries
            .iter()
            .flatten()
            .all(|q| q.ctes.is_empty())
    );
    let calls = rows(
        &mut e,
        "SELECT n FROM node WHERE kind_name('python',kind)='call' ORDER BY n",
    );
    let functions = rows(
        &mut e,
        "SELECT n FROM node WHERE kind_name('python',kind)='function_definition' ORDER BY n",
    );
    let call = calls[0][0].clone();
    let function = functions[1][0].clone();
    for values in [
        json!([[call]]),
        json!([[call], [function]]),
        json!([[function]]),
        json!([]),
    ] {
        let old = rows(&mut e, "SELECT * FROM blocked");
        let patch = json!([{"name":"blocked","delete":old,"insert":values}]);
        let report = change(&mut e, patch);
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        let (_d, mut fresh) = executor();
        fresh.bind_snapshot(files.clone()).unwrap();
        bind(&mut fresh, "blocked", &["n"], values);
        bind(&mut fresh, "marks", &["n", "v"], json!([]));
        fresh.run(p.into(), "python".into(), None).unwrap();
        let query = "SELECT c0,sym_text(c1),c2 FROM owns ORDER BY c0";
        assert_eq!(rows(&mut e, query), rows(&mut fresh, query));
    }
    let before = rows(&mut e, "SELECT c0,sym_text(c1),c2 FROM owns ORDER BY c0");
    assert_eq!(before.as_array().unwrap().len(), 2);
    for patch in [
        json!([{"name":"marks","insert":[[call,7]]}]),
        json!([{"name":"marks","delete":[[call,7]]}]),
    ] {
        let report = change(&mut e, patch);
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        assert_eq!(
            rows(&mut e, "SELECT c0,sym_text(c1),c2 FROM owns ORDER BY c0"),
            before
        );
    }
    e.connection().execute_batch("CREATE TEMP TRIGGER stop_walk BEFORE INSERT ON owns BEGIN SELECT RAISE(ABORT,'stop walk'); END").unwrap();
    let patch=json!({"upsert":[{"path":"a.py","language":"python","source":"def changed():\n    work()\n"}]}).to_string();
    assert!(
        e.apply_file_changes(patch.clone(), None)
            .unwrap_err()
            .reason
            .contains("stop walk")
    );
    assert_eq!(
        rows(&mut e, "SELECT c0,sym_text(c1),c2 FROM owns ORDER BY c0"),
        before
    );
    e.connection()
        .execute_batch("DROP TRIGGER stop_walk")
        .unwrap();
    let report: Value = serde_json::from_str(&e.apply_file_changes(patch, None).unwrap()).unwrap();
    assert_eq!(
        report["programs"][0]["refreshed_strata"],
        json!([]),
        "{report}"
    );
    assert_eq!(
        rows(&mut e, "SELECT sym_text(c1) FROM owns"),
        json!([["changed"]])
    );
}

#[derive(Default, Debug)]
pub(super) struct SqlWork {
    pub(super) vm: std::cell::Cell<u64>,
    scans: std::cell::Cell<u64>,
    pub(super) data_scans: std::cell::Cell<u64>,
    pub(super) sql: std::cell::RefCell<BTreeMap<String, (u64, u64)>>,
}
pub(super) fn profile<T>(
    e: &mut CodeRuleSqlExecutor,
    run: impl FnOnce(&mut CodeRuleSqlExecutor) -> T,
) -> (T, Box<SqlWork>) {
    if let Some(before) = e.sql_profile_totals() {
        let result = run(e);
        let after = e.sql_profile_totals().unwrap();
        let work = Box::<SqlWork>::default();
        work.vm.set(after[2] - before[2]);
        work.scans.set(after[3] - before[3]);
        work.data_scans.set(after[3] - before[3]);
        return (result, work);
    }
    use rusqlite::ffi;
    unsafe extern "C" fn trace(
        _: u32,
        context: *mut std::ffi::c_void,
        statement: *mut std::ffi::c_void,
        _elapsed: *mut std::ffi::c_void,
    ) -> i32 {
        // SQLite invokes the callback synchronously while the boxed counters remain registered.
        let work = unsafe { &*context.cast::<SqlWork>() };
        let stmt = statement.cast::<ffi::sqlite3_stmt>();
        let vm =
            unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 1) } as u64;
        let scans =
            unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP, 1) }
                as u64;
        work.vm.set(work.vm.get() + vm);
        work.scans.set(work.scans.get() + scans);
        let sql = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_sql(stmt)) }.to_string_lossy();
        if !sql.starts_with("DROP ") && !sql.starts_with("CREATE ") {
            work.data_scans.set(work.data_scans.get() + scans);
        }
        if (scans > 100 || vm > 10000) && !sql.starts_with("DROP ") && !sql.starts_with("CREATE ") {
            let mut rows = work.sql.borrow_mut();
            let row = rows.entry(sql.into_owned()).or_default();
            row.0 += vm;
            row.1 += scans;
        }
        0
    }
    struct Trace(*mut ffi::sqlite3);
    impl Drop for Trace {
        fn drop(&mut self) {
            unsafe {
                ffi::sqlite3_trace_v2(self.0, 0, None, std::ptr::null_mut());
            }
        }
    }
    let work = Box::<SqlWork>::default();
    let handle = unsafe { e.connection().handle() };
    unsafe {
        // Cached statements can retain counters from preparation before tracing starts.
        let mut stmt = ffi::sqlite3_next_stmt(handle, std::ptr::null_mut());
        while !stmt.is_null() {
            ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 1);
            ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP, 1);
            stmt = ffi::sqlite3_next_stmt(handle, stmt);
        }
        ffi::sqlite3_trace_v2(
            handle,
            ffi::SQLITE_TRACE_PROFILE as u32,
            Some(trace),
            std::ptr::from_ref(&*work).cast_mut().cast(),
        );
    }
    let guard = Trace(handle);
    let result = run(e);
    drop(guard);
    (result, work)
}

fn change(e: &mut CodeRuleSqlExecutor, patch: Value) -> Value {
    serde_json::from_str(&e.apply_changes(patch.to_string(), None).unwrap()).unwrap()
}
fn bind(e: &mut CodeRuleSqlExecutor, name: &str, columns: &[&str], values: Value) {
    e.bind_relation(json!({"name":name,"columns":columns,"rows":values}).to_string())
        .unwrap();
}
fn ordered(e: &mut CodeRuleSqlExecutor, table: &str) -> Value {
    let width = e
        .connection()
        .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
        .unwrap()
        .column_count();
    rows(
        e,
        &format!(
            "SELECT * FROM {table} ORDER BY {}",
            (1..=width)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
    )
}

#[test]
fn wide_old_sql_joins_use_indexed_deletion_rows_and_rollback_restores_them() {
    let (_d, mut e) = executor();
    e.legacy_initial=true;
    bind(
        &mut e,
        "item",
        &["x"],
        json!((0..20000).map(|i| vec![i]).collect::<Vec<_>>()),
    );
    let names = (0..14).map(|i| format!("kept{i}")).collect::<Vec<_>>();
    let rules = names
        .iter()
        .map(|r| format!(".decl {r}(x:number)\n{r}(x) :- item(x).\n"))
        .collect::<String>();
    let producer = format!(
        ".decl item(x:number)\n.input item\n{rules}.output {}\n",
        names.join(",")
    );
    e.run(producer, "python".into(), None).unwrap();
    let decls = names
        .iter()
        .map(|r| format!(".decl {r}(x:number)\n"))
        .collect::<String>();
    let body = names
        .iter()
        .map(|r| format!("{r}(x)"))
        .collect::<Vec<_>>()
        .join(",");
    let consumer = format!(
        "{decls}.input {}\n.decl joined(x:number)\njoined(x) :- {body}.\n.output joined\n",
        names.join(",")
    );
    e.run(consumer, "python".into(), None).unwrap();
    // First edit migrates projection ownership. The following sparse edits reuse the tables.
    change(&mut e, json!([{"name":"item","insert":[[20001]]}]));
    e.set_limits(json!({"max_steps":500000}).to_string())
        .unwrap();
    let (report, work) = profile(&mut e, |e| {
        e.apply_changes(
            json!([{"name":"item","delete":[[20001]]}]).to_string(),
            None,
        )
    });
    let report: Value =
        serde_json::from_str(&report.unwrap_or_else(|e| panic!("{e}: {work:?}"))).unwrap();
    eprintln!("wide join work: {work:?}");
    assert!(
        work.vm.get() < 100000 && work.data_scans.get() < 1000,
        "{work:?}"
    );
    assert_eq!(report["programs"][1]["outputs"]["joined"]["deleted"], 1);
    assert_eq!(rows(&mut e, "SELECT count(*) FROM kept0"), json!([[20000]]));
    assert_eq!(rows(&mut e, "SELECT count(*) FROM joined"), json!([[20000]]));
    // Fail halfway through restoring old SQL inputs. Earlier restored tables must roll back too.
    e.connection().execute_batch("CREATE TEMP TRIGGER reject_old_sql BEFORE INSERT ON kept7 WHEN NEW.c0=17 BEGIN SELECT RAISE(ABORT,'stop old SQL restoration'); END").unwrap();
    let err = e
        .apply_changes(json!([{"name":"item","delete":[[17]]}]).to_string(), None)
        .unwrap_err();
    assert!(err.reason.contains("stop old SQL restoration"), "{err}");
    for r in &names {
        assert_eq!(
            rows(&mut e, &format!("SELECT c0 FROM {r} WHERE c0=17")),
            json!([[17]])
        );
    }
    assert_eq!(rows(&mut e, "SELECT c0 FROM joined WHERE c0=17"), json!([[17]]));
    e.connection()
        .execute_batch("DROP TRIGGER reject_old_sql")
        .unwrap();
    change(
        &mut e,
        json!([{"name":"item","delete":[[17]],"insert":[[20002]]}]),
    );
    assert_eq!(
        rows(&mut e, "SELECT c0 FROM kept0 EXCEPT SELECT c0 FROM joined"),
        json!([])
    );
    assert_eq!(
        rows(&mut e, "SELECT c0 FROM joined EXCEPT SELECT c0 FROM kept0"),
        json!([])
    );
}

#[test]
fn simultaneous_lattice_replacements_restore_old_and_current_unique_keys() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "k", "v"],
        json!([[1, 10, 1], [2, 10, 2], [3, 20, 1]]),
    );
    e.run(".decl event(id:number,k:number,v:number)\n.input event\n.decl lefts(k:number,v:min<>) widen 100\nlefts(k,v) :- event(_,k,v).\n.decl rights(k:number,v:min<>) widen 100\nrights(k,v) :- event(_,k,v).\n.output lefts,rights\n".into(),"python".into(),None).unwrap();
    e.run(".decl lefts(k:number,v:number)\n.decl rights(k:number,v:number)\n.input lefts,rights\n.decl paired(k:number)\npaired(k) :- lefts(k,1),rights(k,1).\n.output paired\n".into(),"python".into(),None).unwrap();
    e.run(".decl lefts(k:number,v:number)\n.input lefts\n.decl current(k:number,v:number)\ncurrent(k,v) :- lefts(k,v).\n.output current\n".into(),"python".into(),None).unwrap();
    for patch in [
        json!([{"name":"event","delete":[[1,10,1]]}]),
        json!([{"name":"event","insert":[[4,10,1]],"delete":[[3,20,1]]}]),
        json!([{"name":"event","delete":[[2,10,2],[4,10,1]],"insert":[[5,30,1]]}]),
    ] {
        change(&mut e, patch);
        let expected = rows(&mut e, "SELECT k,min(v) FROM event GROUP BY k ORDER BY k");
        for r in ["lefts", "rights", "current"] {
            assert_eq!(ordered(&mut e, r), expected, "{r}");
        }
        assert_eq!(
            ordered(&mut e, "paired"),
            rows(
                &mut e,
                "SELECT k FROM event GROUP BY k HAVING min(v)=1 ORDER BY k"
            )
        );
    }
}

#[test]
fn computed_correlations_retain_indexes_and_match_fresh_execution() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 2, 5], [2, 2, 9], [3, 3, 4]]),
    );
    bind(&mut e, "owner", &["g"], json!([[0], [1], [2]]));
    let program = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl result(g:number,n:number)\nresult(g,n) :- owner(g), k=g+1, n=count : {event(_,k,_)}.\n.output result\n";
    e.run(program.into(), "python".into(), None).unwrap();
    for patch in [
        json!([{"name":"event","insert":[[4,1,7]]}]),
        json!([{"name":"event","delete":[[1,2,5],[2,2,9]],"insert":[[5,3,8]]}]),
        json!([{"name":"owner","delete":[[0]],"insert":[[3]]},{"name":"event","insert":[[6,4,1]]}]),
    ] {
        let report = change(&mut e, patch);
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        let (_fresh_dir, mut fresh) = executor();
        bind(
            &mut fresh,
            "event",
            &["id", "g", "v"],
            rows(&mut e, "SELECT * FROM event"),
        );
        bind(&mut fresh, "owner", &["g"], rows(&mut e, "SELECT * FROM owner"));
        fresh.run(program.into(), "python".into(), None).unwrap();
        assert_eq!(ordered(&mut e, "result"), ordered(&mut fresh, "result"));
    }
}

#[test]
fn computed_context_keys_probe_an_expression_index() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 2, 5], [2, 2, 9]]),
    );
    bind(
        &mut e,
        "owner",
        &["g"],
        json!((0..100000).map(|i| vec![i]).collect::<Vec<_>>()),
    );
    let program = ".decl event(id:number,g:number,v:number)\n.decl owner(g:number)\n.input event,owner\n.decl result(g:number,n:number)\nresult(g,n) :- owner(g), k=g+1, n=count : {event(_,k,_)}.\n.output result\n";
    e.run(program.into(), "python".into(), None).unwrap();
    change(&mut e, json!([{"name":"event","insert":[[3,3,1]]}]));
    e.set_limits(json!({"max_steps":20000}).to_string())
        .unwrap();
    let (result, work) = profile(&mut e, |e| {
        e.apply_changes(
            json!([{"name":"event","delete":[[1,2,5]]}]).to_string(),
            None,
        )
    });
    let report: Value =
        serde_json::from_str(&result.unwrap_or_else(|e| panic!("{e}: {work:?}"))).unwrap();
    assert_eq!(report["programs"][0]["context_rows"], 1);
    assert_eq!(report["programs"][0]["refreshed_strata"], json!([]));
    assert!(work.data_scans.get() < 1000, "{work:?}");
    eprintln!("computed key work: {work:?}");
    assert_eq!(rows(&mut e, "SELECT * FROM result WHERE c0=1"), json!([[1, 1]]));
}

#[test]
fn lattice_candidates_retract_extrema_top_and_duplicate_support_without_refresh() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 10, 3], [2, 10, 3], [3, 10, 9], [4, 20, 7]]),
    );
    let program = ".decl event(id:number,g:number,v:number)\n.input event\n.decl low(g:number,v:min<>) widen 100\nlow(g,v) :- event(_,g,v).\n.decl high(g:number,v:max<>) widen 100\nhigh(g,v) :- event(_,g,v).\n.decl flat(g:number,v:flat<>)\nflat(g,v) :- event(_,g,v).\n.decl global(v:min<>) widen 100\nglobal(v) :- event(_,_,v).\n.output low,high,flat,global\n";
    e.run(program.into(), "python".into(), None).unwrap();
    for patch in [
        json!([{"name":"event","delete":[[1,10,3]]}]),
        json!([{"name":"event","delete":[[2,10,3]],"insert":[[5,30,2]]}]),
        json!([{"name":"event","delete":[[3,10,9],[4,20,7],[5,30,2]]}]),
        json!([{"name":"event","insert":[[6,20,4],[7,20,5]]}]),
    ] {
        let report = change(&mut e, patch);
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        let (_fresh_dir, mut fresh) = executor();
        bind(
            &mut fresh,
            "event",
            &["id", "g", "v"],
            rows(&mut e, "SELECT * FROM event"),
        );
        fresh.run(program.into(), "python".into(), None).unwrap();
        for r in ["low", "high", "flat", "global"] {
            assert_eq!(ordered(&mut e, r), ordered(&mut fresh, r), "{r}");
        }
    }
}

#[test]
fn tree_walk_file_edits_retain_the_walk_and_match_fresh_execution() {
    let (_d, mut e) = executor();
    let mut files = (0..1000)
        .map(|i| (format!("f{i}.py"), format!("def owner{i}():\n    work()\n")))
        .collect::<BTreeMap<_, _>>();
    let snapshot = |files: &BTreeMap<String, String>| {
        json!({"files":files.iter().map(|(p,s)|json!({"path":p,"language":"python","source":s})).collect::<Vec<_>>()}).to_string()
    };
    let program = r#"
.decl node(d:number,n:node,k:symbol,s:number,e:number)
.decl tape(n:node,p:node,s:number,d:number)
.decl field(p:node,f:symbol,c:node)
.decl name(n:node,s:symbol)
.decl doc(d:number,p:symbol,m:symbol)
.input node,tape,field,name,doc
.decl scope(k:symbol)
scope("function_definition").
.decl owns(p:symbol,o:symbol)
owns(p,o) :- node(d,n,"call",_,_), doc(d,p,_), nearest(n,a,scope), field(a,"name",f), name(f,o).
.output owns
"#;
    e.bind_snapshot(snapshot(&files)).unwrap();
    e.run(program.into(), "python".into(), None).unwrap();
    assert!(
        e.executions[0]
            .emitted
            .queries
            .iter()
            .flatten()
            .all(|q| q.ctes.is_empty()),
        "walks are prepared during run"
    );
    for (edit, text) in [
        Some("def changed():\n    work()\n"),
        None,
        Some("def restored():\n    work()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        let patch = match text {
            Some(text) => {
                files.insert("f0.py".into(), text.into());
                json!({"upsert":[{"path":"f0.py","language":"python","source":text}]})
            }
            None => {
                files.remove("f0.py");
                json!({"delete":["f0.py"]})
            }
        };
        let (result, work) = profile(&mut e, |e| e.apply_file_changes(patch.to_string(), None));
        let report: Value =
            serde_json::from_str(&result.unwrap_or_else(|e| panic!("{e}: {work:?}"))).unwrap();
        if edit > 0 {
            eprintln!("warm walk work: {work:?}");
            assert!(work.data_scans.get() < 3000, "{work:?}");
        }
        assert_eq!(
            report["programs"][0]["refreshed_strata"],
            json!([]),
            "{report}"
        );
        let walks=e.executions[0].objects.iter().filter(|(name,_)|name.starts_with("__sql_walk_")).map(|(name,_)|name).collect::<Vec<_>>();
        assert!(!walks.is_empty());
        assert!(walks.iter().all(|name|e.versions.owns(name)));
        assert_eq!(rows(&mut e,"SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '__sql_walk_*' OR name GLOB '__sql_state_*' OR name GLOB '__rr_update_*'"),json!([[0]]));
        let (_fresh_dir, mut fresh) = executor();
        fresh.bind_snapshot(snapshot(&files)).unwrap();
        fresh.run(program.into(), "python".into(), None).unwrap();
        let query = "SELECT sym_text(c0),sym_text(c1) FROM owns ORDER BY 1,2";
        assert_eq!(rows(&mut e, query), rows(&mut fresh, query));
    }
    e.reset().unwrap();
    assert_eq!(
        rows(
            &mut e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '__sql_walk_*'"
        ),
        json!([[0]])
    );
}

#[test]
fn warm_lattice_extrema_use_key_indexes_over_a_large_group() {
    let (_d, mut e) = executor();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!((0..100000).map(|i| vec![i, 1, i]).collect::<Vec<_>>()),
    );
    let program = ".decl event(id:number,g:number,v:number)\n.input event\n.decl low(g:number,v:min<>) widen 100\nlow(g,v) :- event(_,g,v).\n.decl high(g:number,v:max<>) widen 100\nhigh(g,v) :- event(_,g,v).\n.decl flat(g:number,v:flat<>)\nflat(g,v) :- event(_,g,v).\n.output low,high,flat\n";
    e.run(program.into(), "python".into(), None).unwrap();
    change(
        &mut e,
        json!([{"name":"event","insert":[[100000,1,100000]]}]),
    );
    e.set_limits(json!({"max_steps":60000}).to_string())
        .unwrap();
    let (result, work) = profile(&mut e, |e| {
        e.apply_changes(
            json!([{"name":"event","delete":[[0,1,0],[100000,1,100000]]}]).to_string(),
            None,
        )
    });
    let report: Value =
        serde_json::from_str(&result.unwrap_or_else(|e| panic!("{e}: {work:?}"))).unwrap();
    assert_eq!(report["programs"][0]["refreshed_strata"], json!([]));
    assert!(work.data_scans.get() < 1000, "{work:?}");
    eprintln!("lattice work: {work:?}");
    assert_eq!(rows(&mut e, "SELECT * FROM low"), json!([[1, 1]]));
    assert_eq!(rows(&mut e, "SELECT * FROM high"), json!([[1, 99999]]));
    // An error after support mutation must restore the support tables and the folded result.
    e.connection().execute_batch("CREATE TEMP TRIGGER reject_lattice BEFORE INSERT ON low BEGIN SELECT RAISE(ABORT,'stop lattice fold'); END").unwrap();
    assert!(
        e.apply_changes(
            json!([{"name":"event","delete":[[1,1,1]]}]).to_string(),
            None
        )
        .unwrap_err()
        .reason
        .contains("stop lattice fold")
    );
    assert_eq!(rows(&mut e, "SELECT * FROM low"), json!([[1, 1]]));
    e.connection()
        .execute_batch("DROP TRIGGER reject_lattice")
        .unwrap();
    change(&mut e, json!([{"name":"event","delete":[[1,1,1]]}]));
    assert_eq!(rows(&mut e, "SELECT * FROM low"), json!([[1, 2]]));
}

#[test]
fn registered_lattice_joins_retract_candidates_without_refresh() {
    use rusqlite::functions::{Aggregate, Context, FunctionFlags};
    struct Peak;
    impl Aggregate<Option<i64>, Option<i64>> for Peak {
        fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Option<i64>> {
            Ok(None)
        }
        fn step(&self, ctx: &mut Context<'_>, value: &mut Option<i64>) -> rusqlite::Result<()> {
            let next = ctx.get::<i64>(0)?;
            *value = Some(value.map_or(next, |old| old.max(next)));
            Ok(())
        }
        fn finalize(
            &self,
            _: &mut Context<'_>,
            value: Option<Option<i64>>,
        ) -> rusqlite::Result<Option<i64>> {
            Ok(value.flatten())
        }
    }
    let (_d, mut e) = executor();
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    e.connection()
        .create_aggregate_function(c"custom_peak_reduce", 1, flags, Peak)
        .unwrap();
    e.connection()
        .create_scalar_function(c"custom_peak_join", 2, flags, |ctx| {
            Ok(ctx.get::<i64>(0)?.max(ctx.get::<i64>(1)?))
        })
        .unwrap();
    let top = 2147483647;
    e.register_lattice(
        "custom_peak".into(),
        json!({"aggregate":"custom_peak_reduce","merge":"custom_peak_join","widen":"custom_peak_join","top":top}).to_string(),
    )
    .unwrap();
    bind(
        &mut e,
        "event",
        &["id", "g", "v"],
        json!([[1, 1, 7], [2, 1, 9], [3, 1, top], [4, 2, 4]]),
    );
    let program = ".lattice custom_peak<number> { Lub -> @custom_peak }\n.decl event(id:number,g:number,v:number)\n.input event\n.decl peak(g:number,v:custom_peak<>) widen 3\npeak(g,v) :- event(_,g,v).\n.output peak\n";
    e.run(program.into(), "python".into(), None).unwrap();
    for patch in [
        json!([{"name":"event","delete":[[3,1,top]]}]),
        json!([{"name":"event","delete":[[2,1,9]],"insert":[[5,1,8]]}]),
        json!([{"name":"event","delete":[[1,1,7],[5,1,8],[4,2,4]]}]),
    ] {
        let report = change(&mut e, patch);
        assert_eq!(report["programs"][0]["refreshed_strata"], json!([]));
        assert_eq!(
            ordered(&mut e, "peak"),
            rows(&mut e, "SELECT g,max(v) FROM event GROUP BY g ORDER BY g")
        );
    }
}
