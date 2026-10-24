use super::super::{declaration, describe, register};
use super::*;
use rusqlite::{Connection, Result};

#[test]
fn sqlite_affinity_survives_cache_hits_and_different_input_types_do_not_share() -> Result<()> {
    let mut cache = Cache::default();
    let query = "SELECT amount,CAST(amount AS REAL) AS real_value,CAST(amount AS TEXT) AS text_value,amount+1 AS expression,NULL AS missing FROM input";
    for (kind, hit) in [
        ("TEXT", false),
        ("REAL", false),
        ("", false),
        ("TEXT", true),
    ] {
        let db = Connection::open_in_memory()?;
        let input = register(&db, "input", format!("CREATE TABLE input(amount {kind})"))?;
        let schemas = Registry::from([("input".to_owned(), input)]);
        let key = Key::new(query, &schemas, schemas.keys(), std::iter::empty());
        assert_eq!(cache.get(&key).is_some(), hit);
        // Independently ask SQLite for the expected CTAS schema in every iteration.
        db.execute_batch(&format!(
            "CREATE TEMP TABLE expected AS SELECT * FROM ({query}) LIMIT 0"
        ))?;
        let expected = describe(&db, "expected")?;
        let columns = if let Some(columns) = cache.get(&key) {
            columns.clone()
        } else {
            cache.insert(key, expected.clone());
            expected.clone()
        };
        let actual = register(&db, "actual", declaration("actual", &columns))?;
        assert_eq!(actual, expected);
        let query = "SELECT amount,real_value,text_value,expression,missing FROM actual";
        let source = Registry::from([("actual".to_owned(), actual)]);
        let downstream = Key::new(query, &source, source.keys(), std::iter::empty());
        assert_eq!(
            cache.get(&downstream).is_some(),
            hit,
            "downstream keys include inherited affinity"
        );
        cache.insert(downstream, expected);
    }
    Ok(())
}

#[test]
fn complete_source_column_order_and_deferred_columns_participate_in_identity() {
    let columns = vec![
        super::super::Column {
            name: "a".into(),
            declared_type: "TEXT".into(),
        },
        super::super::Column {
            name: "b".into(),
            declared_type: "REAL".into(),
        },
    ];
    let mut schemas = Registry::from([("input".to_owned(), columns)]);
    let key = Key::new(
        "SELECT * FROM input",
        &schemas,
        schemas.keys(),
        std::iter::empty(),
    );
    let extra = ["later".to_owned()];
    let deferred = Key::new(
        "SELECT * FROM input",
        &schemas,
        schemas.keys(),
        extra.iter(),
    );
    assert!(key != deferred);
    schemas.get_mut("input").unwrap().reverse();
    assert!(
        key != Key::new(
            "SELECT * FROM input",
            &schemas,
            schemas.keys(),
            std::iter::empty()
        )
    );
}

#[test]
fn metadata_retention_has_fixed_entry_and_byte_bounds() {
    let schemas = Registry::new();
    let mut cache = Cache::default();
    for n in 0..(MAX_ENTRIES + 4) {
        let key = Key::new(
            &format!("SELECT {n}"),
            &schemas,
            schemas.keys(),
            std::iter::empty(),
        );
        cache.insert(key, vec![]);
        assert!(cache.entries.len() <= MAX_ENTRIES);
        assert!(cache.bytes <= MAX_BYTES);
    }
    assert!(
        cache
            .get(&Key::new(
                "SELECT 0",
                &schemas,
                schemas.keys(),
                std::iter::empty()
            ))
            .is_none()
    );
    assert!(
        cache
            .get(&Key::new(
                "SELECT 131",
                &schemas,
                schemas.keys(),
                std::iter::empty()
            ))
            .is_some()
    );
    let before = (cache.entries.len(), cache.bytes);
    cache.insert(
        Key::new(
            &"x".repeat(MAX_BYTES),
            &schemas,
            schemas.keys(),
            std::iter::empty(),
        ),
        vec![],
    );
    assert_eq!(
        (cache.entries.len(), cache.bytes),
        before,
        "oversized metadata never evicts a useful cache"
    );
}
