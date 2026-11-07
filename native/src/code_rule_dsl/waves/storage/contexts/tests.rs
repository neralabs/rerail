use super::super::*;

#[test]
fn imported_span_lookup_does_not_scan_every_row_in_the_scope() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut s = Storage::default();
    let slot = "__context_input_span";
    s.bind_scope(&db, 1, BTreeMap::from([(slot.into(), 1)]))?;
    s.declare_input(
        &db,
        slot,
        format!("CREATE TABLE {slot}(__path TEXT,__start INTEGER,__end INTEGER,value INTEGER)"),
    )?;
    let (table, _) = s.write_target(slot);
    db.execute_batch(&format!("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) INSERT INTO {table} SELECT 1,'target.py',x,x+1,x FROM n;INSERT INTO {table} SELECT 2,__path,__start,__end,value FROM {table} WHERE __execution_key=1"))?;
    let mut stmt = db.prepare(&format!(
        "SELECT sum(value) FROM {slot} WHERE __path='target.py' AND __start>=998 AND __end<=1000"
    ))?;
    assert_eq!(stmt.query_row([], |r| r.get::<_, i64>(0))?, 1997);
    let work = stmt.get_status(rusqlite::StatementStatus::VmStep);
    assert!(work < 1000, "selective span lookup used {work} VM steps");
    Ok(())
}

/// Only the relations whose counts changed are written again, and nothing is scanned: a sentinel
/// count left in `wave_stats` survives a boundary that did not touch its relation.
#[test]
fn statistics_boundary_writes_only_changed_counts() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut s = Storage::default();
    for (id, slot) in [(1, "__context_unchanged"), (2, "__context_changed")] {
        s.bind_scope(&db, id, BTreeMap::from([(slot.into(), id)]))?;
        s.declare(&db, slot, format!("CREATE TABLE {slot}(v INTEGER)"), true)?;
        let (table, _) = s.write_target(slot);
        db.execute(&format!("INSERT INTO {table} VALUES(?1,1)"), [id as i64])?;
        s.populated(slot, 1);
    }
    let first = s.statistics_boundary(&db)?;
    assert_eq!(first["rows.__context_changed"], 1);
    db.execute("UPDATE temp.wave_stats SET rows = 12345 WHERE tbl = '__context_unchanged'", [])?;
    s.grew("__context_changed", 2);
    let second = s.statistics_boundary(&db)?;
    assert_eq!(second["rows.__context_changed"], 3);
    let stored = |tbl: &str| db.query_row("SELECT rows FROM temp.wave_stats WHERE tbl = ?1", [tbl], |r| r.get::<_, i64>(0));
    assert_eq!(stored("__context_unchanged")?, 12345);
    assert_eq!(stored("__context_changed")?, 3);
    assert!(db.query_row("SELECT count(*) FROM temp.sqlite_schema WHERE name LIKE 'sqlite_stat%'", [], |r| r.get::<_, i64>(0))? == 0);
    Ok(())
}

#[test]
fn scope_selection_is_an_outer_gate_for_recursive_readers() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut s = Storage::default();
    let slot = "__context_recursive_gate";
    s.bind_scope(&db, 1, BTreeMap::from([(slot.into(), 1)]))?;
    s.declare_input(
        &db,
        slot,
        format!("CREATE TABLE {slot}(__path TEXT,__start INTEGER,__end INTEGER,id INTEGER,parent INTEGER)"),
    )?;
    let (table, _) = s.write_target(slot);
    db.execute_batch(&format!("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) INSERT INTO {table} SELECT 1,'a.py',x,x+1,x,x-1 FROM n;INSERT INTO {table} VALUES(2,'a.py',2,3,2,0)"))?;
    // Plan shape is the contract: the one-row scope gate must be outside the
    // backing lookup, rather than a scalar subquery attached to every arm.
    let sql: String = db.query_row(
        "SELECT sql FROM temp.sqlite_schema WHERE name=?1",
        [slot],
        |r| r.get(0),
    )?;
    assert!(
        sql.contains("CROSS JOIN"),
        "scope selection was not an outer gate: {sql}"
    );
    assert_eq!(db.query_row(&format!("WITH RECURSIVE reaching(id) AS (VALUES(0) UNION SELECT v.id FROM reaching r JOIN {slot} v ON v.parent=r.id) SELECT count(*) FROM reaching"), [], |r| r.get::<_, i64>(0))?, 1001);
    Ok(())
}

#[test]
fn scopes_share_a_schema_without_sharing_rows_or_rebuilding_indexes() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut storage = Storage::default();
    let slot = "__context_1_values";
    let definition = format!("CREATE TEMP TABLE {slot}(value TEXT,optional BLOB)");
    storage.bind_scope(&db, 1, BTreeMap::from([(slot.into(), 11)]))?;
    storage.declare(&db, slot, definition.clone(), true)?;
    let (table, binding) = storage.write_target(slot);
    db.execute(
        &format!("INSERT INTO {table} VALUES(?1,'first',NULL),(?1,'first',NULL)"),
        [binding.unwrap()],
    )?;
    storage.written(slot, true);
    storage.index(
        &db,
        slot,
        "__context_1_index",
        format!("CREATE INDEX __context_1_index ON {slot}(value)"),
    )?;
    let schema: i64 = db.query_row("PRAGMA temp.schema_version", [], |r| r.get(0))?;
    storage.bind_scope(&db, 2, BTreeMap::from([(slot.into(), 22)]))?;
    storage.declare(&db, slot, definition, true)?;
    let (table, binding) = storage.write_target(slot);
    db.execute(
        &format!("INSERT INTO {table} VALUES(?1,'second',x'01')"),
        [binding.unwrap()],
    )?;
    storage.written(slot, true);
    storage.index(
        &db,
        slot,
        "__context_1_index",
        format!("CREATE INDEX __context_1_index ON {slot}(value)"),
    )?;
    assert_eq!(
        db.query_row("PRAGMA temp.schema_version", [], |r| r.get::<_, i64>(0))?,
        schema
    );
    assert_eq!(
        db.query_row(&format!("SELECT value FROM {slot}"), [], |r| r
            .get::<_, String>(0))?,
        "second"
    );
    storage.bind_scope(&db, 1, BTreeMap::from([(slot.into(), 11)]))?;
    assert_eq!(
        db.query_row(
            &format!("SELECT count(*) FROM {slot} WHERE value='first' AND optional IS NULL"),
            [],
            |r| r.get::<_, i64>(0)
        )?,
        2
    );
    storage.clear_owner(&db, 2)?;
    assert_eq!(
        db.query_row(&format!("SELECT count(*) FROM {slot}"), [], |r| r
            .get::<_, i64>(0))?,
        2
    );
    storage.clear_owner(&db, 1)?;
    assert_eq!(
        db.query_row(&format!("SELECT count(*) FROM {slot}"), [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[test]
fn completed_scope_cache_is_bounded_and_does_not_keep_every_historical_consumer() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut storage = Storage::default();
    let slot = "__context_cache";
    for scope in 1..=200 {
        storage.bind_scope(&db, scope, BTreeMap::from([(slot.into(), scope)]))?;
        storage.declare(
            &db,
            slot,
            format!("CREATE TEMP TABLE {slot}(v INTEGER)"),
            true,
        )?;
        let (table, key) = storage.write_target(slot);
        db.execute(
            &format!("INSERT INTO {table} VALUES(?1,?1)"),
            [key.unwrap()],
        )?;
        storage.written(slot, true);
        storage.publish(slot);
        storage.clear_owner(&db, scope)?;
        storage.evict(&db, 4 * 1024 * 1024)?;
    }
    let (table, _) = storage.write_target(slot);
    let retained: i64 = db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
    assert!(retained <= 128, "retained {retained} historical scope rows");
    Ok(())
}

#[test]
fn eviction_cannot_remove_pinned_complete_or_empty_bindings() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut storage = Storage::default();
    let slot = "__context_pins";
    let bindings = BTreeMap::from([(slot.into(), 7)]);
    storage.bind_scope(&db, 1, bindings.clone())?;
    storage.declare(&db, slot, format!("CREATE TABLE {slot}(v)"), true)?;
    let (table, key) = storage.write_target(slot);
    db.execute(
        &format!("INSERT INTO {table} VALUES(?1,99)"),
        [key.unwrap()],
    )?;
    storage.written(slot, true);
    storage.publish(slot);
    storage.bind_scope(&db, 2, bindings)?;
    storage.clear_owner(&db, 1)?;
    storage.evict(&db, 0)?;
    assert!(storage.ready(slot));
    assert_eq!(
        db.query_row(&format!("SELECT v FROM {slot}"), [], |r| r.get::<_, i64>(0))?,
        99
    );
    storage.clear_owner(&db, 2)?;
    storage.evict(&db, 0)?;
    assert!(!storage.ready(slot));
    storage.bind_scope(&db, 3, BTreeMap::from([(slot.into(), 8)]))?;
    storage.declare(&db, slot, format!("CREATE TABLE {slot}(v)"), true)?;
    storage.written(slot, false);
    storage.publish(slot);
    storage.evict(&db, 0)?;
    assert!(
        storage.ready(slot),
        "empty complete output is a pinned result"
    );
    storage.clear_owner(&db, 3)?;
    storage.evict(&db, 0)?;
    assert!(!storage.ready(slot));
    Ok(())
}

/// A new binding's rows replace the old binding's in the counts, and a boundary after the first
/// writes no schema.
#[test]
fn publication_boundary_counts_the_bound_rows_without_ddl() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut storage = Storage::default();
    let slot = "__context_stats";
    let definition = format!("CREATE TABLE {slot}(v TEXT)");
    storage.bind_scope(&db, 1, BTreeMap::from([(slot.into(), 1)]))?;
    storage.declare(&db, slot, definition.clone(), true)?;
    let (table, _) = storage.write_target(slot);
    db.execute_batch(&format!("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100) INSERT INTO {table} SELECT 1,'same' FROM n"))?;
    storage.populated(slot, 100);
    assert_eq!(storage.statistics_boundary(&db)?["rows.__context_stats"], 100);
    storage.clear_owner(&db, 1)?;
    storage.bind_scope(&db, 2, BTreeMap::from([(slot.into(), 2)]))?;
    storage.declare(&db, slot, definition, true)?;
    db.execute_batch(&format!("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<40) INSERT INTO {table} SELECT 2,printf('v%d',x) FROM n"))?;
    storage.populated(slot, 40);
    assert_eq!(storage.statistics_boundary(&db)?["rows.__context_stats"], 40);
    let schema: i64 = db.query_row("PRAGMA temp.schema_version", [], |r| r.get(0))?;
    storage.statistics_boundary(&db)?;
    assert_eq!(db.query_row("PRAGMA temp.schema_version", [], |r| r.get::<_, i64>(0))?, schema);
    Ok(())
}

#[test]
fn final_reader_releases_only_its_binding_before_consumer_completion() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut s = Storage::default();
    let slot = "__context_last_read";
    s.bind_scope(&db, 1, BTreeMap::from([(slot.into(), 1)]))?;
    s.declare(&db, slot, format!("CREATE TABLE {slot}(v)"), true)?;
    let (target, _) = s.write_target(slot);
    db.execute(&format!("INSERT INTO {target} VALUES(1,'one')"), [])?;
    s.written(slot, true);
    s.bind_scope(&db, 2, BTreeMap::from([(slot.into(), 2)]))?;
    s.declare(&db, slot, format!("CREATE TABLE {slot}(v)"), true)?;
    db.execute(&format!("INSERT INTO {target} VALUES(2,'two')"), [])?;
    s.written(slot, true);
    s.release_binding(&db, 1, slot, 1)?;
    assert_eq!(
        db.query_row(&format!("SELECT v FROM {slot}"), [], |r| r
            .get::<_, String>(0))?,
        "two"
    );
    assert_eq!(
        db.query_row(&format!("SELECT count(*) FROM {target}"), [], |r| r
            .get::<_, i64>(0))?,
        1
    );
    s.clear_owner(&db, 1)?;
    s.clear_owner(&db, 2)?;
    assert_eq!(
        db.query_row(&format!("SELECT count(*) FROM {target}"), [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[test]
fn public_column_names_cannot_collide_with_the_private_scope_key() -> Result<()> {
    let db = Connection::open_in_memory()?;
    let mut s = Storage::default();
    let slot = "__context_collision";
    s.bind_scope(&db, 1, BTreeMap::from([(slot.into(), 42)]))?;
    s.declare(
        &db,
        slot,
        format!("CREATE TABLE {slot}(\"__EXECUTION_KEY\" INTEGER,value TEXT)"),
        true,
    )?;
    let (table, key) = s.write_target(slot);
    db.execute(
        &format!("INSERT INTO {table} VALUES(?1,17,'first')"),
        [key.unwrap()],
    )?;
    s.written(slot, true);
    let value: (i64, String) = db.query_row(&format!("SELECT * FROM {slot}"), [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    assert_eq!(value, (17, "first".into()));
    Ok(())
}
