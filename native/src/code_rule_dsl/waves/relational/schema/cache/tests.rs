use super::super::{bind, declaration, describe, register};
use super::*;
use rusqlite::{Connection, Result};
use std::sync::Arc;

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
        let schemas = Registry::from([("input".to_owned(), Arc::new(input))]);
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
            cache.insert(key, Arc::new(expected.clone()));
            Arc::new(expected.clone())
        };
        bind(&db, "actual", declaration("actual", &columns))?;
        let actual = describe(&db, "actual")?;
        assert_eq!(actual, expected);
        for (name, source) in [("from_cached", "actual"), ("from_native", "expected")] {
            db.execute_batch(&format!("CREATE TABLE {name} AS SELECT amount,real_value,text_value,expression,missing FROM {source} LIMIT 0"))?;
        }
        assert_eq!(
            describe(&db, "from_cached")?,
            describe(&db, "from_native")?,
            "downstream SQLite CTAS retains the same affinity across the opaque boundary"
        );
        let query = "SELECT amount,real_value,text_value,expression,missing FROM actual";
        let source = Registry::from([("actual".to_owned(), Arc::new(actual))]);
        let downstream = Key::new(query, &source, source.keys(), std::iter::empty());
        assert_eq!(
            cache.get(&downstream).is_some(),
            hit,
            "downstream keys include inherited affinity"
        );
        cache.insert(downstream, Arc::new(expected));
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
    let mut schemas = Registry::from([("input".to_owned(), Arc::new(columns))]);
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
    Arc::make_mut(schemas.get_mut("input").unwrap()).reverse();
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
        cache.insert(key, Arc::new(vec![]));
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
        Arc::new(vec![]),
    );
    assert_eq!(
        (cache.entries.len(), cache.bytes),
        before,
        "oversized metadata never evicts a useful cache"
    );
}

#[test]
fn simultaneous_compilations_admit_one_schema_inference() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    let start = Arc::new(Barrier::new(8));
    let inferences = Arc::new(AtomicUsize::new(0));
    let threads = (0..8)
        .map(|_| {
            let start = Arc::clone(&start);
            let inferences = Arc::clone(&inferences);
            std::thread::spawn(move || -> Result<SharedColumns> {
                let query = "SELECT CAST(1 AS REAL) AS value /* concurrent-schema-inference */";
                let schemas = Registry::new();
                let key = Key::new(query, &schemas, schemas.keys(), std::iter::empty());
                start.wait();
                let mut cache = shared();
                if let Some(columns) = cache.get(&key) {
                    return Ok(columns.clone());
                }
                let db = Connection::open_in_memory()?;
                db.execute_batch(&format!(
                    "CREATE TABLE inferred AS SELECT * FROM ({query}) LIMIT 0"
                ))?;
                let columns = Arc::new(describe(&db, "inferred")?);
                inferences.fetch_add(1, Ordering::Relaxed);
                cache.insert(key, columns.clone());
                Ok(columns)
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        let columns = thread.join().unwrap().unwrap();
        assert_eq!(columns[0].declared_type, "REAL");
    }
    assert_eq!(inferences.load(Ordering::Relaxed), 1);
}
