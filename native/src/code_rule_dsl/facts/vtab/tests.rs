use super::*;

fn relation(columns: &[&str], rows: &[&[i64]]) -> Arc<Relation> {
    let mut r = Relation::new(columns);
    rows.iter().for_each(|row| r.push(row));
    Arc::new(r)
}
fn ints(db: &Connection, sql: &str) -> Result<Vec<i64>> {
    db.prepare(sql)?.query_map([], |r| r.get(0))?.collect()
}

#[test]
fn rebinding_preserves_open_cursor_schema_and_releases_old_payloads() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let modules = Modules::default();
    let first = relation(&["value"], &[&[1], &[2]]);
    let weak = Arc::downgrade(&first);
    modules.bind(&db, "__context_native", first.clone())?;
    drop(first);
    let mut statement = db.prepare_cached("SELECT value FROM __context_native")?;
    let mut rows = statement.query([])?;
    assert_eq!(rows.next()?.unwrap().get::<_, i64>(0)?, 1);
    modules.bind(&db, "__context_native", relation(&["value"], &[&[8]]))?;
    assert_eq!(rows.next()?.unwrap().get::<_, i64>(0)?, 2);
    assert!(rows.next()?.is_none());
    drop(rows);
    drop(statement);
    let mut statement = db.prepare_cached("SELECT value FROM __context_native")?;
    assert_eq!(statement.query_row([], |r| r.get::<_, i64>(0))?, 8);
    assert!(weak.upgrade().is_none());
    assert_eq!(statement.get_status(rusqlite::StatementStatus::RePrepare), 0);
    drop(statement);
    modules.get("__context_native").unwrap().clear();
    assert_eq!(db.query_row("SELECT count(*) FROM __context_native", [], |r| r.get::<_, i64>(0))?, 0);
    assert!(modules.bind(&db, "__context_native", relation(&["other"], &[])).is_err());
    modules.unregister(&db, "__context_native")?;
    assert!(db.prepare("SELECT * FROM __context_native").is_err());
    Ok(())
}

#[test]
fn plan_signatures_distinguish_empty_and_populated_sources() {
    let sources = |rows: &[&[i64]]| BTreeMap::from([("native".into(), relation(&["value"], rows) as Arc<dyn Source>)]);
    let empty = sources(&[]);
    assert_ne!(plan_signature(&empty), plan_signature(&sources(&[&[1], &[2], &[3], &[4]])));
    assert_eq!(plan_signature(&sources(&[&[1], &[2]])), plan_signature(&sources(&[&[5], &[6]])));
}

/// `EQ`, `IN` and ranges are answered from the column index, one filter per `IN` list, and agree with
/// SQLite's own comparison for constants of every type (SQLite rechecks; the index may only narrow).
#[test]
fn lookups_use_the_index_and_agree_with_sqlite() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let modules = Modules::default();
    let rows: Vec<[i64; 2]> = (0..200).map(|i| [i % 50, i]).collect();
    let mut r = Relation::new(&["k", "n"]);
    rows.iter().for_each(|row| r.push(row));
    modules.bind(&db, "rel", Arc::new(r))?;
    db.execute_batch("CREATE TABLE plain(k INTEGER, n INTEGER)")?;
    for [k, n] in &rows {
        db.execute("INSERT INTO plain VALUES (?1, ?2)", [k, n])?;
    }
    let counters = modules.get("rel").unwrap();
    for filter in [
        "k = 7",
        "k = 7.0",
        "k = '7'",
        "k = 7.5",
        "k = 'x'",
        "k = NULL",
        "k IN (3, 3, 9, '11', 12.5)",
        "n > 190",
        "n >= 10.5 AND n < 13",
        "n BETWEEN 5 AND 7",
        "n < 'a'",
        "n > 9223372036854775807",
    ] {
        let before = counters.counters().candidates.load(Ordering::Relaxed);
        let got = ints(&db, &format!("SELECT n FROM rel WHERE {filter} ORDER BY n"))?;
        let expected = ints(&db, &format!("SELECT n FROM plain WHERE {filter} ORDER BY n"))?;
        assert_eq!(got, expected, "{filter}");
        let read = counters.counters().candidates.load(Ordering::Relaxed) - before;
        if !filter.contains("'a'") {
            assert!(read < 200, "{filter} read {read} candidates");
        }
    }
    let batches = counters.counters().batches.load(Ordering::Relaxed);
    ints(&db, "SELECT n FROM rel WHERE k IN (1, 2, 3)")?;
    assert_eq!(counters.counters().batches.load(Ordering::Relaxed), batches + 1);
    let plan: Vec<String> =
        db.prepare("EXPLAIN QUERY PLAN SELECT n FROM rel WHERE k = 4")?.query_map([], |r| r.get(3))?.collect::<Result<_>>()?;
    assert!(plan.iter().any(|p| p.contains("VIRTUAL TABLE INDEX 2:")), "{plan:?}");
    Ok(())
}

/// A join probes the inner table once per outer row by key, never scanning it.
#[test]
fn joins_probe_by_key() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let modules = Modules::default();
    modules.bind(&db, "parent", relation(&["p", "c"], &[&[1, 2], &[1, 3], &[2, 4], &[3, 5]]))?;
    modules.bind(&db, "named", relation(&["n", "s"], &[&[1, 10], &[2, 20], &[3, 30], &[4, 40], &[5, 50]]))?;
    let got = ints(&db, "SELECT named.s FROM parent CROSS JOIN named ON named.n = parent.c WHERE parent.p = 1 ORDER BY 1")?;
    assert_eq!(got, [20, 30]);
    let named = modules.get("named").unwrap();
    assert_eq!(named.counters().candidates.load(Ordering::Relaxed), 2);
    Ok(())
}

/// Several `=` are one filter: the source's composite order where it keeps one, else a probe of the most
/// selective column with the rest checked before SQLite sees a row. Both agree with a plain table.
#[test]
fn several_equalities_probe_once_and_agree_with_sqlite() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let modules = Modules::default();
    // `a` has 4 values, `b` 50, `c` 2000: no single column but `c` is selective; `a` and `b` together are.
    let rows: Vec<[i64; 3]> = (0..2000).map(|i| [i % 4, (i / 4) % 50, i]).collect();
    let mut r = Relation::new(&["a", "b", "c"]);
    rows.iter().for_each(|row| r.push(row));
    modules.bind(&db, "probe", Arc::new(r))?;
    let columns = ["a", "b", "c"].map(str::to_owned).to_vec();
    let values = (0..3).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect();
    modules.bind(&db, "ordered", crate::code_rule_dsl::logic::native::component::columns_source(columns, values))?;
    db.execute_batch("CREATE TABLE plain(a INTEGER, b INTEGER, c INTEGER)")?;
    for [a, b, c] in &rows {
        db.execute("INSERT INTO plain VALUES (?1, ?2, ?3)", [a, b, c])?;
    }
    for table in ["probe", "ordered"] {
        let counters = modules.get(table).unwrap();
        for filter in ["a = 1 AND b = 7", "b = 7 AND a = 1", "a = 1 AND b = 7 AND c = 29", "a = 1 AND b = '7'", "a = 1 AND b = 7.5",
            "a IN (1, 2) AND b = 7", "a = 3 AND b = 49 AND c > 1000"] {
            let before = counters.counters().candidates.load(Ordering::Relaxed);
            let got = ints(&db, &format!("SELECT c FROM {table} WHERE {filter} ORDER BY c"))?;
            let expected = ints(&db, &format!("SELECT c FROM plain WHERE {filter} ORDER BY c"))?;
            assert_eq!(got, expected, "{table}: {filter}");
            let read = counters.counters().candidates.load(Ordering::Relaxed) - before;
            // An `IN` list is checked by SQLite, not here: `b` = 7's 40 rows reach it.
            let most = if filter.contains(" IN ") { 40 } else { 20 };
            assert!(read <= most, "{table}: {filter} read {read} candidates");
        }
    }
    // The bound column of a join is probed, not a constant shared by many rows (`b` = 7 has 40).
    let plan: Vec<String> = db
        .prepare("EXPLAIN QUERY PLAN SELECT p.c FROM plain p CROSS JOIN ordered o WHERE o.b = 7 AND o.c = p.c")?
        .query_map([], |r| r.get(3))?
        .collect::<Result<_>>()?;
    assert!(plan.iter().any(|p| p.contains("VIRTUAL TABLE INDEX 4194304:c")), "{plan:?}");
    Ok(())
}
