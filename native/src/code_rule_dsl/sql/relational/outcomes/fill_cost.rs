//! A cache-fill delta belongs to one reached miss, never to a batch of inputs.
use super::bridge;

pub(in crate::code_rule_dsl::sql::relational::outcomes) fn bracket(
    query: &str,
    width: usize,
    guard: &str,
) -> Result<String, String> {
    // relation_row has 130 columns: retain that bound when adding the fill delta.
    if width == 0 || width >= 130 {
        return Err("cache-fill return exceeds 130 typed columns".into());
    }
    let columns = (0..width)
        .map(|index| format!("r.c{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let returned = (0..width)
        .map(|index| format!("v.c{index}"))
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        "WITH fill_before AS MATERIALIZED (SELECT relation_cache_fill_steps() AS start WHERE {guard}),\nfill_result AS MATERIALIZED (SELECT b.start,{columns} FROM fill_before b {})\nSELECT {returned},relation_cache_fill_delta(v.start) FROM fill_result v",
        bridge(query, width),
    ))
}

#[test]
fn fill_cost_return_preserves_the_existing_typed_width_bound() {
    assert!(bracket("SELECT 1", 129, "1").is_ok());
    assert!(bracket("SELECT 1", 130, "1").is_err());
    assert!(bracket("SELECT 1", 0, "1").is_err());
}

#[test]
fn fill_cost_bracket_is_per_reached_input_and_includes_nested_fills() {
    use rusqlite::functions::FunctionFlags;
    use std::sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    };
    let db = rusqlite::Connection::open_in_memory().unwrap();
    crate::code_rule_dsl::sql::relational::row::register(&db).unwrap();
    let counter = Arc::new(AtomicI64::new(0));
    let before = counter.clone();
    db.create_scalar_function(
        "relation_cache_fill_steps",
        0,
        FunctionFlags::SQLITE_UTF8,
        move |_| Ok(before.load(Ordering::SeqCst)),
    )
    .unwrap();
    let after = counter.clone();
    db.create_scalar_function(
        "relation_cache_fill_delta",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| Ok(after.load(Ordering::SeqCst) - ctx.get::<i64>(0)?),
    )
    .unwrap();
    let filling = counter.clone();
    db.create_scalar_function("fill", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let amount = ctx.get::<i64>(0)?;
        filling.fetch_add(amount, Ordering::SeqCst);
        Ok(amount)
    })
    .unwrap();
    let nested = bracket("SELECT 1,d.work-fill(d.amount)", 2, "d.reached=1").unwrap();
    // The outer bracket includes the same fill, without charging or executing it twice.
    let outer = bracket(
        &format!("SELECT r.c0,r.c1 FROM (SELECT 1) {}", bridge(&nested, 3)),
        2,
        "d.reached=1",
    )
    .unwrap();
    let query = format!(
        "SELECT d.file,r.c0,r.c1,r.c2 FROM (SELECT 0 AS file,100 AS work,3 AS amount,1 AS reached UNION ALL SELECT 1,100,7,1 UNION ALL SELECT 2,100,19,0) d {} ORDER BY d.file",
        bridge(&outer, 3)
    );
    let rows = db
        .prepare(&query)
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (0, Some(1), Some(97), Some(3)),
            (1, Some(1), Some(93), Some(7)),
            (2, None, None, None)
        ]
    );
    assert_eq!(counter.load(Ordering::SeqCst), 10);
}
