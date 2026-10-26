//! Read-only SQL replay. It shares SQLite and the backup boundary with the engine.
#[path = "../../../../../native/src/code_rule_dsl/profile/sql/backup.rs"]
mod backup;
#[path = "../../../../../native/src/code_rule_dsl/profile/clock.rs"]
mod clock;
#[cfg(test)]
#[path = "../../../../../native/src/code_rule_dsl/profile/memory.rs"]
mod memory;
#[path = "../../../../../native/src/code_rule_dsl/relational/functions.rs"]
mod sql_functions;
use rusqlite::{Connection, OpenFlags, Result, StatementStatus, types::ValueRef};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

fn restore(db: &Connection, path: &Path, schema: &std::ffi::CStr) -> Result<()> {
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    backup::copy(&source, c"main", db, schema)
}
fn database(manifest: &Path, data: &serde_json::Value) -> Result<(Connection, Arc<AtomicUsize>)> {
    let db = Connection::open_in_memory()?;
    db.execute_batch("PRAGMA temp_store=FILE;PRAGMA cache_spill=ON;PRAGMA cache_size=-1024;PRAGMA temp.cache_size=-512;CREATE TEMP TABLE __snapshot_init(x);DROP TABLE __snapshot_init;")?;
    let directory = manifest.parent().unwrap();
    restore(
        &db,
        &directory.join(data["main"].as_str().expect("main snapshot")),
        c"main",
    )?;
    if let Some(temp) = data["temp"].as_str() {
        restore(&db, &directory.join(temp), c"temp")?;
    }
    let work = Arc::new(AtomicUsize::new(usize::MAX));
    let binding=sql_functions::binding();
    sql_functions::rebind(&binding,1024*1024,Arc::clone(&work));
    sql_functions::install_bound(&db,binding)?;
    Ok((db, work))
}
fn evaluate(db: &Connection, sql: &str) -> Result<serde_json::Value> {
    let cpu = clock::thread_cpu_ns();
    let start = Instant::now();
    let mut q = db.prepare(sql)?;
    if !q.readonly() || q.parameter_count() != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let columns = q.column_count();
    let mut count = 0u64;
    let mut bytes = 0u64;
    let mut digest = 0u64;
    {
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let mut hash = DefaultHasher::new();
            for col in 0..columns {
                match row.get_ref(col)? {
                    ValueRef::Null => 0u8.hash(&mut hash),
                    ValueRef::Integer(n) => {
                        1u8.hash(&mut hash);
                        n.hash(&mut hash);
                        bytes += 8;
                    }
                    ValueRef::Real(n) => {
                        2u8.hash(&mut hash);
                        n.to_bits().hash(&mut hash);
                        bytes += 8;
                    }
                    ValueRef::Text(value) => {
                        3u8.hash(&mut hash);
                        value.hash(&mut hash);
                        bytes += value.len() as u64;
                    }
                    ValueRef::Blob(value) => {
                        4u8.hash(&mut hash);
                        value.hash(&mut hash);
                        bytes += value.len() as u64;
                    }
                }
            }
            count += 1;
            digest = digest.wrapping_add(hash.finish());
        }
    }
    Ok(
        serde_json::json!({"rows":count,"valueBytes":bytes,"bagDigest":format!("{digest:016x}"),
        "cpuNs":cpu.zip(clock::thread_cpu_ns()).map(|(a,b)|b-a),"wallNs":start.elapsed().as_nanos(),
        "vmSteps":q.get_status(StatementStatus::VmStep),"fullscanSteps":q.get_status(StatementStatus::FullscanStep),
        "sorts":q.get_status(StatementStatus::Sort),"autoIndexRows":q.get_status(StatementStatus::AutoIndex)}),
    )
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(
        (1..=3).contains(&args.len()),
        "usage: failure_replay MANIFEST.json [QUERY.sql [SETUP.sql]]"
    );
    let manifest = Path::new(&args[0]);
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest)?)?;
    assert_eq!(data["format"], "rerail.sql-failure-snapshot/v1");
    let sql = if args.len() >= 2 {
        std::fs::read_to_string(&args[1])?
    } else {
        data["context"]["query"]
            .as_str()
            .expect("failed SELECT query")
            .into()
    };
    let setup = Instant::now();
    let (db, work) = database(manifest, &data)?;
    // Experiments mutate only this restored, in-memory copy, never the snapshots.
    if let Some(setup) = args.get(2) {
        db.execute_batch(&std::fs::read_to_string(setup)?)?;
    }
    let setup_ns = setup.elapsed().as_nanos();
    let result = evaluate(&db, &sql)?;
    println!(
        "{}",
        serde_json::json!({"format":"rerail.sql-failure-replay/v1","sqliteVersion":rusqlite::version(),
        "setupWallNs":setup_ns,"result":result,"regexWorkUnits":usize::MAX-work.load(Ordering::Relaxed),
        "note":"Includes result hashing. Uses production regex functions and SQLite. No AST ingestion or original rule allowance. Missing native producers remain errors. Compare bag digests with this same replay binary."})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replays_the_production_regex_functions_and_keeps_invalid_groups_as_errors() -> Result<()> {
        let directory = tempfile::tempdir().unwrap();
        let source = Connection::open(directory.path().join("main.sqlite"))?;
        source.execute_batch(
            "CREATE TABLE input(value);INSERT INTO input VALUES('é_async'),(NULL);",
        )?;
        let data = serde_json::json!({"main":"main.sqlite"});
        let (db, work) = database(&directory.path().join("manifest.json"), &data)?;
        let result = evaluate(
            &db,
            "SELECT rx_replace(value,'_async$',''),rx_extract(value,'(é)',1),rx_matches(value,'^é') FROM input",
        )?;
        assert_eq!(result["rows"], 2);
        let expected = evaluate(&db, "SELECT 'é','é',1 UNION ALL SELECT '',NULL,0")?;
        assert_eq!(result["bagDigest"], expected["bagDigest"]);
        assert_eq!(usize::MAX - work.load(Ordering::Relaxed), 6);
        assert!(evaluate(&db, "SELECT rx_extract('x','(x)',2)").is_err());
        Ok(())
    }

    #[test]
    fn restores_both_owners_and_compares_typed_bags_without_running_writes() -> Result<()> {
        let directory = tempfile::tempdir().unwrap();
        let main = Connection::open(directory.path().join("main.sqlite"))?;
        main.execute_batch("CREATE TABLE input(value);INSERT INTO input VALUES(1),(NULL),('é');")?;
        let temp = Connection::open(directory.path().join("temp.sqlite"))?;
        temp.execute_batch("CREATE TABLE selected(value);INSERT INTO selected VALUES(1);")?;
        let data = serde_json::json!({"main":"main.sqlite","temp":"temp.sqlite"});
        let (db, _) = database(&directory.path().join("manifest.json"), &data)?;
        let a = evaluate(&db, "SELECT value FROM input")?;
        let b = evaluate(&db, "SELECT value FROM input ORDER BY value DESC")?;
        assert_eq!(a["rows"], 3);
        assert_eq!(a["bagDigest"], b["bagDigest"]);
        assert_eq!(
            evaluate(
                &db,
                "SELECT i.value FROM input i JOIN selected s USING(value)"
            )?["rows"],
            1
        );
        assert!(evaluate(&db, "DELETE FROM input").is_err());
        Ok(())
    }
}
