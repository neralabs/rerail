//! Diagnostic snapshots separate live SQL objects, reserved pages, and allocator residency.
use rusqlite::{Connection, Result, ffi};
use serde_json::{Value, json};

pub fn snapshot(db: &Connection, live: usize) -> Result<Value> {
    let mut query = db.prepare("SELECT 'main',s.type,d.name,sum(d.pgsize),sum(d.payload) FROM dbstat('main') d JOIN main.sqlite_schema s ON s.name=d.name GROUP BY s.type,d.name UNION ALL SELECT 'temp',s.type,d.name,sum(d.pgsize),sum(d.payload) FROM dbstat('temp') d JOIN temp.sqlite_schema s ON s.name=d.name GROUP BY s.type,d.name ORDER BY 4 DESC")?;
    let mut objects = query.query_map([], |r| Ok(json!({"schema":r.get::<_,String>(0)?,"type":r.get::<_,String>(1)?,"name":r.get::<_,String>(2)?,"bytes":r.get::<_,i64>(3)?,"payloadBytes":r.get::<_,i64>(4)?})))?.collect::<Result<Vec<_>>>()?;
    drop(query);
    let mut totals =
        json!({"mainTableBytes":0,"mainIndexBytes":0,"tempTableBytes":0,"tempIndexBytes":0});
    for object in &objects {
        let key = match (object["schema"].as_str(), object["type"].as_str()) {
            (Some("main"), Some("index")) => "mainIndexBytes",
            (Some("main"), _) => "mainTableBytes",
            (Some("temp"), Some("index")) => "tempIndexBytes",
            _ => "tempTableBytes",
        };
        totals[key] = (totals[key].as_i64().unwrap() + object["bytes"].as_i64().unwrap()).into();
    }
    let temp_free: i64 = db.pragma_query_value(Some("temp"), "freelist_count", |r| r.get(0))?;
    let pages = db.query_row("SELECT (SELECT page_count FROM pragma_page_count),(SELECT freelist_count FROM pragma_freelist_count),(SELECT page_size FROM pragma_page_size),(SELECT page_count FROM pragma_page_count('temp')),?1,(SELECT page_size FROM pragma_page_size('temp'))",[temp_free],|r|Ok([r.get::<_,i64>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?]))?;
    let mut resident = json!({});
    for (name, kind) in [
        ("cacheBytes", ffi::SQLITE_DBSTATUS_CACHE_USED),
        ("schemaBytes", ffi::SQLITE_DBSTATUS_SCHEMA_USED),
        ("statementBytes", ffi::SQLITE_DBSTATUS_STMT_USED),
    ] {
        let (mut current, mut high) = (0i64, 0i64);
        // SAFETY: this connection is live and only its evaluating thread accesses it.
        let code =
            unsafe { ffi::sqlite3_db_status64(db.handle(), kind, &mut current, &mut high, 0) };
        if code != ffi::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None));
        }
        resident[name] = current.into();
    }
    let object_count = objects.len();
    objects.truncate(20);
    Ok(
        json!({"liveRelations":live,"objectCount":object_count,"objects":objects,"objectBytes":totals,"mainReservedBytes":pages[0]*pages[2],"mainFreeBytes":pages[1]*pages[2],"tempReservedBytes":pages[3]*pages[5],"tempFreeBytes":pages[4]*pages[5],"sqlite":resident,"process":crate::code_rule_dsl::profile::memory::sample("anchor-working-set")}),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn distinguishes_live_objects_from_freed_reserved_pages() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TEMP TABLE held AS WITH RECURSIVE n(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM n WHERE id<1000) SELECT id,zeroblob(100) AS bytes FROM n; CREATE INDEX temp.held_keys ON held(id)").unwrap();
        let live = super::snapshot(&db, 1).unwrap();
        assert!(live["objectBytes"]["tempTableBytes"].as_i64().unwrap() > 100000);
        assert!(live["objectBytes"]["tempIndexBytes"].as_i64().unwrap() > 0);
        db.execute_batch("DROP TABLE held").unwrap();
        let released = super::snapshot(&db, 0).unwrap();
        assert!(released["tempFreeBytes"].as_i64().unwrap() > 0);
        assert!(
            released["objects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|o| o["name"] != "held" && o["name"] != "held_keys")
        );
        assert!(released["sqlite"]["cacheBytes"].is_number());
    }
}
