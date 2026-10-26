use super::*;

#[test]
fn scopes_share_a_schema_without_sharing_rows_or_rebuilding_indexes() -> Result<()> {
    let db=Connection::open_in_memory()?;
    let mut storage=Storage::default();
    let slot="__context_1_values";
    let definition=format!("CREATE TEMP TABLE {slot}(value TEXT,optional BLOB)");
    storage.bind_scope(&db,1,BTreeMap::from([(slot.into(),11)]))?;
    storage.declare(&db,slot,definition.clone(),true)?;
    let (table,binding)=storage.write_target(slot);
    db.execute(&format!("INSERT INTO {table} VALUES(?1,'first',NULL),(?1,'first',NULL)"),[binding.unwrap()])?;
    storage.written(slot,true);
    storage.index(&db,slot,"__context_1_index",format!("CREATE INDEX __context_1_index ON {slot}(value)"))?;
    let schema:i64=db.query_row("PRAGMA temp.schema_version",[],|r|r.get(0))?;
    storage.bind_scope(&db,2,BTreeMap::from([(slot.into(),22)]))?;
    storage.declare(&db,slot,definition,true)?;
    let (table,binding)=storage.write_target(slot);
    db.execute(&format!("INSERT INTO {table} VALUES(?1,'second',x'01')"),[binding.unwrap()])?;
    storage.written(slot,true);
    storage.index(&db,slot,"__context_1_index",format!("CREATE INDEX __context_1_index ON {slot}(value)"))?;
    assert_eq!(db.query_row("PRAGMA temp.schema_version",[],|r|r.get::<_,i64>(0))?,schema);
    assert_eq!(db.query_row(&format!("SELECT value FROM {slot}"),[],|r|r.get::<_,String>(0))?,"second");
    storage.bind_scope(&db,1,BTreeMap::from([(slot.into(),11)]))?;
    assert_eq!(db.query_row(&format!("SELECT count(*) FROM {slot} WHERE value='first' AND optional IS NULL"),[],|r|r.get::<_,i64>(0))?,2);
    storage.clear_owner(&db,2)?;
    assert_eq!(db.query_row(&format!("SELECT count(*) FROM {slot}"),[],|r|r.get::<_,i64>(0))?,2);
    storage.clear_owner(&db,1)?;
    assert_eq!(db.query_row(&format!("SELECT count(*) FROM {slot}"),[],|r|r.get::<_,i64>(0))?,0);
    Ok(())
}

#[test]
fn completed_scope_cache_is_bounded_and_does_not_keep_every_historical_consumer() -> Result<()> {
    let db=Connection::open_in_memory()?;let mut storage=Storage::default();
    let slot="__context_cache";
    for scope in 1..=200 {
        storage.bind_scope(&db,scope,BTreeMap::from([(slot.into(),scope)]))?;
        storage.declare(&db,slot,format!("CREATE TEMP TABLE {slot}(v INTEGER)"),true)?;
        let (table,key)=storage.write_target(slot);
        db.execute(&format!("INSERT INTO {table} VALUES(?1,?1)"),[key.unwrap()])?;
        storage.written(slot,true);storage.publish(slot);storage.clear_owner(&db,scope)?;
        storage.evict(&db,4*1024*1024)?;
    }
    let (table,_)=storage.write_target(slot);
    let retained:i64=db.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get(0))?;
    assert!(retained<=128,"retained {retained} historical scope rows");
    Ok(())
}
