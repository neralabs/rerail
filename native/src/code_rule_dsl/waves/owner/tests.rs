use super::*;
use crate::code_rule_dsl::facts::vtab::syntax::{sources, tests::documents};
use std::path::PathBuf;

fn temp(what: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-owner-{what}-{}-{nonce}.sqlite", std::process::id()))
}
fn count(db: &Database, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// A lease's rows and vtab bindings are gone for the next lease; the owner and its schema stay.
#[test]
fn a_lease_returns_empty_backing_and_empty_vtabs() {
    let store = ContentStore::open(&temp("lease")).unwrap();
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let pool = scope.pool();
    {
        let db = pool.take().unwrap();
        db.storage.borrow_mut().declare(&db, "input", "CREATE TEMP TABLE input(value)".into(), true).unwrap();
        db.execute("INSERT INTO input VALUES(7)", []).unwrap();
        db.storage.borrow_mut().written("input", true);
        for (name, source) in sources(&store, documents().into_iter().take(1).collect()).unwrap() {
            db.bind(name, source).unwrap();
        }
        assert!(count(&db, "SELECT count(*) FROM node") > 0);
        assert_ne!(db.plan_signature.get(), 0);
    }
    let db = pool.take().unwrap();
    assert_eq!(count(&db, "SELECT count(*) FROM input"), 0);
    assert_eq!(count(&db, "SELECT count(*) FROM node"), 0);
    assert_eq!(db.plan_signature.get(), 0);
    // The owner is on the store: its tables are `main`.
    assert_eq!(count(&db, "SELECT count(*) FROM main.fact_stats"), 0);
}

#[test]
fn an_open_sql_cursor_prevents_switching_its_execution_binding() {
    let store = ContentStore::open(&temp("cursor")).unwrap();
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let pool = scope.pool();
    let first = pool.take().unwrap();
    first.execute_batch("CREATE TEMP TABLE frame(v);INSERT INTO frame VALUES(1),(2)").unwrap();
    let mut statement = first.prepare("SELECT v FROM frame").unwrap();
    let mut rows = statement.query([]).unwrap();
    assert!(rows.next().unwrap().is_some());
    assert!(pool.take().is_err());
    assert_eq!(rows.next().unwrap().unwrap().get::<_, i64>(0).unwrap(), 2);
    drop(rows);
    drop(statement);
    assert!(pool.take().is_ok());
}

#[test]
fn incomplete_or_failed_consumers_cannot_recycle_partial_rows() {
    let store = ContentStore::open(&temp("partial")).unwrap();
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let pool = scope.pool();
    {
        let db = pool.take().unwrap();
        let name = db.private("unfinished");
        db.storage.borrow_mut().declare(&db, &name, format!("CREATE TEMP TABLE {name}(value)"), true).unwrap();
        db.execute_batch(&format!("INSERT INTO {name} VALUES(1)")).unwrap();
        db.storage.borrow_mut().written(&name, true);
    }
    let db = pool.take().unwrap();
    assert!(db.prepare("SELECT * FROM __private_1_unfinished").is_err());
}

/// A clean owner is kept between executions on its store, schema and all; another store opens its
/// own, and an unhealthy one is never kept.
#[test]
fn a_clean_owner_is_recycled_for_its_store_only() {
    let (one, two) = (ContentStore::open(&temp("one")).unwrap(), ContentStore::open(&temp("two")).unwrap());
    let backing = Backing::default();
    {
        let scope = backing.enter(&one);
        let db = scope.pool().take().unwrap();
        db.storage.borrow_mut().declare(&db, "kept", "CREATE TEMP TABLE kept(v)".into(), true).unwrap();
    }
    let kept = |store: &ContentStore| {
        let scope = backing.enter(store);
        let db = scope.pool().take().unwrap();
        count(&db, "SELECT count(*) FROM temp.sqlite_schema WHERE name = 'kept'")
    };
    assert_eq!(kept(&one), 1);
    assert_eq!(kept(&two), 0);
    {
        let scope = backing.enter(&one);
        let pool = scope.pool();
        let db = pool.take().unwrap();
        db.storage.borrow_mut().declare(&db, "kept", "CREATE TEMP TABLE kept(v)".into(), true).unwrap();
        drop(db);
        pool.invalidate();
    }
    assert_eq!(kept(&one), 0);
}

/// Counts reported by writers reach `temp.wave_stats` at the boundary, keyed as the plans read them.
#[test]
fn statistics_boundary_returns_the_counted_rows() {
    let store = ContentStore::open(&temp("stats")).unwrap();
    let backing = Backing::default();
    let scope = backing.enter(&store);
    let pool = scope.pool();
    let db = pool.take().unwrap();
    db.storage.borrow_mut().declare(&db, "edge", "CREATE TEMP TABLE edge(a INTEGER, b INTEGER)".into(), true).unwrap();
    let added = db.execute("INSERT INTO edge VALUES (1, 2), (2, 3)", []).unwrap();
    db.storage.borrow_mut().grew("edge", added);
    let stats = pool.statistics_boundary().unwrap();
    assert_eq!(stats["rows.edge"], 2);
    assert_eq!(count(&db, "SELECT rows FROM temp.wave_stats WHERE tbl = 'edge'"), 2);
    assert!(db.resident_bytes().unwrap() > 0);
}
