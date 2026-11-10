use super::scaling::profile;
use super::*;
use rusqlite::functions::{Aggregate, Context, FunctionFlags};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Bits;

#[test]
fn registered_join_rotations_and_composite_keys_match_full_reduction() {
    let (_d, mut e) = executor();
    register_bits(&mut e);
    e.bind_relation(json!({"name":"event","columns":["id","g","h","v"],"rows":[]}).to_string())
        .unwrap();
    let p = ".lattice summary_bits<number> { Lub -> @summary_bits }\n.decl event(id:number,g:number,h:number,v:number)\n.input event\n.decl mask(g:number,h:number,v:summary_bits<>) widen 3\nmask(g,h,v) :- event(_,g,h,v).\n.output mask\n";
    e.run(p.into(), "python".into(), None).unwrap();
    let mut present = BTreeSet::<i64>::new();
    let mut seed = 17u64;
    for iteration in 0..240 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = if iteration < 80 {
            iteration
        } else {
            ((seed >> 32) % 100) as i64
        };
        let row = json!([id, id % 3, id % 5, (id / 2) * 17]);
        let patch = if present.remove(&id) {
            json!([{"name":"event","delete":[row]}])
        } else {
            present.insert(id);
            json!([{"name":"event","insert":[row]}])
        };
        e.apply_changes(patch.to_string(), None).unwrap();
        assert_eq!(
            rows(&e, "SELECT * FROM mask ORDER BY c0,c1"),
            rows(
                &e,
                "SELECT g,h,test_join_bits(v) FROM event GROUP BY g,h ORDER BY g,h"
            ),
            "iteration {iteration}"
        );
        assert_eq!(
            index_rows(
                &e,
                "SELECT count(*) FROM __sql_state_mask_join_nodes n LEFT JOIN __sql_state_mask_join_nodes l ON l.c0=n.c0 AND l.c1=n.c1 AND l.value=n.left_value LEFT JOIN __sql_state_mask_join_nodes r ON r.c0=n.c0 AND r.c1=n.c1 AND r.value=n.right_value WHERE abs(coalesce(l.height,0)-coalesce(r.height,0))>1 OR n.height<>1+max(coalesce(l.height,0),coalesce(r.height,0))"
            ),
            json!([[0]])
        );
    }
    let deletion = rows(&e, "SELECT * FROM event");
    e.apply_changes(
        json!([{"name":"event","delete":deletion}]).to_string(),
        None,
    )
    .unwrap();
    assert_eq!(
        index_rows(&e, "SELECT count(*) FROM __sql_state_mask_join_nodes"),
        json!([[0]])
    );
    assert_eq!(
        index_rows(&e, "SELECT count(*) FROM __sql_state_mask_join_roots"),
        json!([[0]])
    );
}
// Inspect the active cursor explicitly; closing frames can retain older caches.
fn index_rows(e: &CodeRuleSqlExecutor, sql: &str) -> Value {
    let trace=e.executions[0].temporal[&0].explain();
    let frame=trace["frames"].as_array().unwrap().iter().find(|f|
        f["operators"].as_array().unwrap().iter().any(|op|op["relation"]=="mask" && op["active"]==true)).unwrap();
    let op=frame["operators"].as_array().unwrap().iter().find(|op|op["relation"]=="mask" && op["active"]==true).unwrap();
    let index=&op["analysis"][0]["lattice_support_index"];
    let nodes=index["nodes"].as_str().unwrap();
    let roots=index["roots"].as_str().unwrap();
    let _scope=e.versions.activate(frame["native_scope"].as_u64().unwrap() as usize).unwrap();
    assert!(!crate::code_rule_dsl::facts::vtab::bound_source(e.connection(),nodes).unwrap().writable());
    let query=sql.replace("__sql_state_mask_join_nodes",nodes).replace("__sql_state_mask_join_roots",roots);
    assert_eq!(rows(e,"SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '__sql_state_*' OR name GLOB '__rr_update_*'"),json!([[0]]));
    rows(e,&query)
}

impl Aggregate<i64, i64> for Bits {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<i64> {
        Ok(0)
    }
    fn step(&self, c: &mut Context<'_>, v: &mut i64) -> rusqlite::Result<()> {
        *v |= c.get::<i64>(0)?;
        Ok(())
    }
    fn finalize(&self, _: &mut Context<'_>, v: Option<i64>) -> rusqlite::Result<i64> {
        Ok(v.unwrap_or(0))
    }
}

#[test]
fn registered_joins_update_logarithmic_summary_paths_and_restore_after_errors() {
    let (_dir, mut e) = executor();
    let calls = register_bits(&mut e);
    let top = 2147483647;
    e.bind_relation(json!({"name":"event","columns":["id","g","v"],"rows":(0..100000).map(|i|vec![i,1,i]).collect::<Vec<_>>()} ).to_string()).unwrap();
    e.run(".lattice summary_bits<number> { Lub -> @summary_bits }\n.decl event(id:number,g:number,v:number)\n.input event\n.decl mask(g:number,v:summary_bits<>) widen 3\nmask(g,v) :- event(_,g,v).\n.output mask\n".into(),"python".into(),None).unwrap();
    for patch in [
        json!([{"name":"event","delete":[[50000,1,50000]]}]),
        json!([{"name":"event","insert":[[100000,1,top]]}]),
        json!([{"name":"event","delete":[[100000,1,top]],"insert":[[100001,1,524288]]}]),
        json!([{"name":"event","insert":[[100002,1,524288]],"delete":[[100001,1,524288]]}]),
        json!([{"name":"event","delete":[[100002,1,524288]]}]),
    ] {
        calls.store(0, Ordering::Relaxed);
        e.set_limits(json!({"max_steps":50000}).to_string())
            .unwrap();
        let (result, work) = profile(&mut e, |e| e.apply_changes(patch.to_string(), None));
        let report: Value =
            serde_json::from_str(&result.unwrap_or_else(|err| panic!("{err}: {work:?}"))).unwrap();
        assert_eq!(report["programs"][0]["refreshed_strata"], json!([]));
        assert!(
            calls.load(Ordering::Relaxed) < 300,
            "{} merge calls",
            calls.load(Ordering::Relaxed)
        );
        assert!(work.data_scans.get() < 1000, "{work:?}");
        eprintln!(
            "registered tree: {} merges, {work:?}",
            calls.load(Ordering::Relaxed)
        );
        e.set_limits("{}".into()).unwrap();
        assert_eq!(
            rows(&e, "SELECT * FROM mask"),
            rows(&e, "SELECT g,test_join_bits(v) FROM event GROUP BY g")
        );
        assert!(
            index_rows(&e, "SELECT max(height) FROM __sql_state_mask_join_nodes")[0][0]
                .as_i64()
                .unwrap()
                < 25
        );
    }
    let before = rows(&e, "SELECT * FROM mask");
    e.connection().execute_batch("CREATE TEMP TRIGGER fail_summary BEFORE INSERT ON mask BEGIN SELECT RAISE(ABORT,'stop summary'); END").unwrap();
    assert!(
        e.apply_changes(
            json!([{"name":"event","insert":[[200000,1,1048576]]}]).to_string(),
            None
        )
        .unwrap_err()
        .reason
        .contains("stop summary")
    );
    assert_eq!(rows(&e, "SELECT * FROM mask"), before);
    e.connection()
        .execute_batch("DROP TRIGGER fail_summary")
        .unwrap();
    e.apply_changes(
        json!([{"name":"event","insert":[[200000,1,1048576]]}]).to_string(),
        None,
    )
    .unwrap();
    assert_eq!(
        rows(&e, "SELECT * FROM mask"),
        rows(&e, "SELECT g,test_join_bits(v) FROM event GROUP BY g")
    );
    e.reset().unwrap();
    assert_eq!(
        rows(
            &e,
            "SELECT count(*) FROM temp.sqlite_schema WHERE name GLOB '__sql_state_mask_join_*'"
        ),
        json!([[0]])
    );
}

pub(super) fn register_bits(e: &mut CodeRuleSqlExecutor) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    e.connection()
        .create_aggregate_function(c"test_join_bits", 1, flags, Bits)
        .unwrap();
    e.connection()
        .create_scalar_function(c"test_join_merge", 2, flags, move |c| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(c.get::<i64>(0)? | c.get::<i64>(1)?)
        })
        .unwrap();
    let top = 2147483647;
    e.register_lattice("summary_bits".into(),json!({"aggregate":"test_join_bits","merge":"test_join_merge","widen":"test_join_merge","top":top}).to_string()).unwrap();
    calls
}
