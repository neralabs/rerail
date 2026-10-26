//! Same production FTS/value schema; transaction and statistics maintenance prototypes.
use super::*;
const COLUMNS: &str = "fid,nodes,path,digest,lang,srcenc,src,rec,spans,checkpoints,parb,sizb,flags";

#[derive(Default, serde::Serialize)]
struct Executions {
    profile_events: usize,
    commented_profile_events: usize,
    trigger_entries: usize,
}
// SAFETY: the pointer is a live stack-owned counter until tracing is detached below.
unsafe extern "C" fn trace(
    mask: u32,
    context: *mut std::ffi::c_void,
    stmt: *mut std::ffi::c_void,
    sql: *mut std::ffi::c_void,
) -> i32 {
    let out = unsafe { &mut *context.cast::<Executions>() };
    if mask == rusqlite::ffi::SQLITE_TRACE_PROFILE {
        out.profile_events += 1;
        let text = unsafe { rusqlite::ffi::sqlite3_sql(stmt.cast()) };
        if !text.is_null()
            && unsafe { std::ffi::CStr::from_ptr(text) }
                .to_bytes()
                .starts_with(b"--")
        {
            out.commented_profile_events += 1;
        }
    } else if mask == rusqlite::ffi::SQLITE_TRACE_STMT
        && !sql.is_null()
        && unsafe { std::ffi::CStr::from_ptr(sql.cast()) }
            .to_bytes()
            .starts_with(b"-- TRIGGER")
    {
        out.trigger_entries += 1;
    }
    0
}

struct Pack;
impl rusqlite::functions::Aggregate<Vec<u32>, Vec<u8>> for Pack {
    fn init(&self, _: &mut rusqlite::functions::Context<'_>) -> rusqlite::Result<Vec<u32>> {
        Ok(Vec::new())
    }
    fn step(
        &self,
        ctx: &mut rusqlite::functions::Context<'_>,
        values: &mut Vec<u32>,
    ) -> rusqlite::Result<()> {
        values.push(ctx.get(0)?);
        Ok(())
    }
    fn finalize(
        &self,
        _: &mut rusqlite::functions::Context<'_>,
        values: Option<Vec<u32>>,
    ) -> rusqlite::Result<Vec<u8>> {
        let mut values = values.unwrap_or_default();
        values.sort();
        let mut bytes = Vec::new();
        let mut previous = 0;
        for pre in values {
            crate::postings::append_delta(&mut bytes, pre - previous);
            previous = pre;
        }
        Ok(bytes)
    }
}

fn cache_counters(db: &Connection) -> Value {
    let mut out = json!({});
    for (name, kind) in [
        ("bytes", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED),
        ("hits", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_HIT),
        ("misses", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS),
        ("writes", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE),
        ("spills", rusqlite::ffi::SQLITE_DBSTATUS_CACHE_SPILL),
    ] {
        let (mut current, mut high) = (0i64, 0i64);
        // SAFETY: live connection, scalar outputs, SQLite's documented status selector.
        let status = unsafe {
            rusqlite::ffi::sqlite3_db_status64(db.handle(), kind, &mut current, &mut high, 0)
        };
        assert_eq!(status, rusqlite::ffi::SQLITE_OK);
        out[name] = current.into();
    }
    out
}

fn publish(
    db: &Connection,
    lo: usize,
    hi: usize,
    set_stats: bool,
    m: &mut Meter,
) -> rusqlite::Result<()> {
    m.statement(db, "BEGIN IMMEDIATE")?;
    m.statement(db,&format!("INSERT INTO file({COLUMNS}) SELECT {COLUMNS} FROM pending_file WHERE fid>={lo} AND fid<{hi}"))?;
    m.statement(db,&format!("INSERT INTO ftsk(rowid,k) SELECT fid,tokens FROM pending_fts WHERE fid>={lo} AND fid<{hi}"))?;
    m.statement(db,&format!("INSERT INTO file_kind_counts SELECT fid,code,n,postings FROM pending_kind WHERE fid>={lo} AND fid<{hi}"))?;
    m.statement(db,&format!("INSERT INTO kfreq SELECT 1,code,sum(n) FROM pending_kind WHERE fid>={lo} AND fid<{hi} GROUP BY code ON CONFLICT(lang,code) DO UPDATE SET n=n+excluded.n"))?;
    m.statement(db,&format!("INSERT INTO vidx SELECT lang,term,fid,kind,postings,n FROM pending_value WHERE fid>={lo} AND fid<{hi}"))?;
    if set_stats {
        m.statement(db,&format!("INSERT INTO vlex SELECT lang,term,count(*) FROM pending_value WHERE fid>={lo} AND fid<{hi} GROUP BY lang,term ON CONFLICT(lang,term) DO UPDATE SET n=n+excluded.n"))?;
        m.statement(db,&format!("INSERT INTO value_frequency SELECT lang,term,kind,sum(n) FROM pending_value WHERE fid>={lo} AND fid<{hi} GROUP BY lang,term,kind ON CONFLICT(lang,term,kind) DO UPDATE SET n=n+excluded.n"))?;
        m.statement(db,&format!("INSERT INTO value_kind_frequency SELECT lang,kind,sum(n) FROM pending_value WHERE fid>={lo} AND fid<{hi} GROUP BY lang,kind ON CONFLICT(lang,kind) DO UPDATE SET n=n+excluded.n"))?;
    }
    m.statement(db, "COMMIT")?;
    Ok(())
}
fn state(db: &Connection) -> rusqlite::Result<Vec<Vec<i64>>> {
    let mut m = Meter::default();
    // Full committed statistic rows, not a lossy checksum. Term identities become stable
    // lexical IDs only for this integer-row measurement host.
    m.statement(db,"WITH terms AS (SELECT term,row_number() OVER(ORDER BY term) AS id FROM (SELECT DISTINCT term FROM vidx)) SELECT 1,lang,id,0,n FROM vlex JOIN terms USING(term) UNION ALL SELECT 2,lang,id,kind,n FROM value_frequency JOIN terms USING(term) UNION ALL SELECT 3,lang,kind,0,n FROM value_kind_frequency UNION ALL SELECT 4,lang,code,0,n FROM kfreq ORDER BY 1,2,3,4,5")
}
fn assert_state(db: &Connection) -> rusqlite::Result<()> {
    for (actual, expected) in [
        (
            "SELECT lang,term,n FROM vlex",
            "SELECT lang,term,count(*) FROM vidx GROUP BY lang,term",
        ),
        (
            "SELECT lang,term,kind,n FROM value_frequency",
            "SELECT lang,term,kind,sum(n) FROM vidx GROUP BY lang,term,kind",
        ),
        (
            "SELECT lang,kind,n FROM value_kind_frequency",
            "SELECT lang,kind,sum(n) FROM vidx GROUP BY lang,kind",
        ),
        (
            "SELECT lang,code,n FROM kfreq",
            "SELECT f.lang,k.code,sum(k.n) FROM file_kind_counts k JOIN file f USING(fid) GROUP BY f.lang,k.code",
        ),
    ] {
        let bad:i64=db.query_row(&format!("SELECT count(*) FROM (SELECT * FROM ({actual} EXCEPT {expected}) UNION ALL SELECT * FROM ({expected} EXCEPT {actual}))"),[],|r|r.get(0))?;
        assert_eq!(bad, 0, "committed statistics differ");
    }
    let bad:i64=db.query_row("SELECT count(*) FROM (SELECT * FROM (SELECT lang,term,fid,kind,hex(postings),n FROM vidx EXCEPT SELECT lang,term,fid,kind,hex(postings),n FROM pending_value) UNION ALL SELECT * FROM (SELECT lang,term,fid,kind,hex(postings),n FROM pending_value EXCEPT SELECT lang,term,fid,kind,hex(postings),n FROM vidx))",[],|r|r.get(0))?;
    assert_eq!(bad, 0, "value posting contents differ");
    let fts: i64 = db.query_row("SELECT count(*) FROM ftsk_inst", [], |r| r.get(0))?;
    let nodes: i64 = db.query_row("SELECT sum(nodes) FROM file", [], |r| r.get(0))?;
    assert_eq!(fts, nodes, "positional FTS coverage");
    let mismatch:i64=db.query_row("SELECT count(*) FROM (SELECT term,offset,count(*) AS n FROM ftsk_inst GROUP BY term,offset HAVING n<>(SELECT count(*) FROM file))",[],|r|r.get(0))?;
    assert_eq!(mismatch, 0, "every copied source has identical positions");
    Ok(())
}
pub(super) fn run(capture: &Path, variant: &str, scale: usize) -> Result<Value> {
    if scale == 0 {
        return Err("publication requires positive scale".into());
    }
    let process = cpu_ms();
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("index.sqlite");
    let db = Connection::open(&path)?;
    crate::session::configure(&db)?;
    crate::postings::install(&db)?;
    db.create_aggregate_function(
        c"lab_pack",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        Pack,
    )?;
    db.execute_batch(include_str!(
        "../../../../../native/src/code_rule_dsl/matcher/anchors/sql/schema.sql"
    ))?;
    db.execute(
        "ATTACH ?1 AS cap",
        [capture.to_str().ok_or("path encoding")?],
    )?;
    let (packet, set_stats, cache) = match variant {
        "file" => (1, false, 1024),
        "file-eight-cache" => (1, false, 8192),
        "packet-triggers" => (32, false, 1024),
        "packet-set" => (32, true, 8192),
        "packet-small-cache" => (32, true, 1024),
        "packet-large-cache" => (32, true, 32768),
        _ => return Err("publication variant".into()),
    };
    db.execute_batch(&format!(
        "PRAGMA main.cache_size=-{cache}; PRAGMA temp.cache_size=-{cache}"
    ))?;
    let common = cpu_ms();
    // Use the complete tiny real file, replicated under independent file IDs. This
    // isolates publication from parsing. Packed tape and posting bytes remain exact.
    db.execute_batch(&format!("CREATE TEMP TABLE pending_file AS WITH RECURSIVE ids(id) AS (VALUES(1000) UNION ALL SELECT id+1 FROM ids WHERE id<{}) SELECT ids.id AS fid,nodes,path,digest,lang,srcenc,src,rec,spans,checkpoints,parb,sizb,flags FROM ids CROSS JOIN cap.file WHERE cap.file.fid=2",1000+scale-1))?;
    db.execute_batch("CREATE TEMP TABLE source_value AS SELECT v.lang,v.term,v.kind,v.postings,(SELECT count(*) FROM nera_postings(v.postings)) AS n FROM cap.vidx v WHERE v.fid=2; CREATE TEMP TABLE source_kind AS SELECT kind AS code,count(*) AS n,lab_pack(pre) AS postings FROM cap.decoded_properties WHERE fid=2 GROUP BY kind; CREATE TEMP TABLE pending_value AS SELECT v.lang,v.term,f.fid,v.kind,v.postings,v.n FROM pending_file f CROSS JOIN source_value v; CREATE INDEX temp.pending_value_fid ON pending_value(fid); CREATE TEMP TABLE pending_kind AS SELECT f.fid,c.code,c.n,c.postings FROM pending_file f CROSS JOIN source_kind c; CREATE INDEX temp.pending_kind_fid ON pending_kind(fid); CREATE TEMP TABLE pending_fts AS SELECT fid,(SELECT group_concat(term,' ') FROM (SELECT term FROM cap.ftsk_inst WHERE doc=2 ORDER BY offset)) AS tokens FROM pending_file")?;
    // This capture's vocabulary ID is copied, not assumed by production. The prototype
    // currently uses one-language inputs; reject rather than silently mixing IDs.
    let lang: i64 = db.query_row("SELECT DISTINCT lang FROM pending_file", [], |r| r.get(0))?;
    assert_eq!(lang, 1);
    if set_stats {
        db.execute_batch("DROP TRIGGER value_insert")?;
    }
    let common_cpu = cpu_ms() - common;
    let cache_before = cache_counters(&db);
    let mut m = Meter::default();
    let started = cpu_ms();
    for lo in (1000..1000 + scale).step_by(packet) {
        publish(&db, lo, (lo + packet).min(1000 + scale), set_stats, &mut m)?;
    }
    let kernel = cpu_ms() - started;
    let cache_after = cache_counters(&db);
    assert_state(&db)?;
    let before = state(&db)?;
    // Fault isolation: all trigger writes, kind statistics and FTS updates roll back.
    db.execute_batch("BEGIN; DELETE FROM file WHERE fid=1000; ROLLBACK")?;
    assert_state(&db)?;
    assert_eq!(before, state(&db)?);
    // A source replacement must remove old kind counts before deleting its posting rows.
    let pinned = Connection::open(&path)?;
    pinned.execute_batch("BEGIN")?;
    assert_eq!(
        pinned.query_row("SELECT count(*) FROM file", [], |r| r.get::<_, i64>(0))?,
        scale as i64
    );
    db.execute_batch("BEGIN; UPDATE kfreq SET n=n-coalesce((SELECT n FROM file_kind_counts WHERE fid=1000 AND code=kfreq.code),0); DELETE FROM kfreq WHERE n=0; DELETE FROM file WHERE fid=1000; COMMIT")?;
    assert_eq!(
        pinned.query_row("SELECT count(*) FROM file WHERE fid=1000", [], |r| r
            .get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM file WHERE fid=1000", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    publish(&db, 1000, 1001, set_stats, &mut Meter::default())?;
    assert_state(&db)?;
    assert_eq!(before, state(&db)?);
    pinned.execute_batch("ROLLBACK")?;
    // Count SQLite internals in a separate replay. Instrumentation never contributes
    // to the publication CPU above. An INSERT SELECT is still a host batch when FTS
    // performs per-document writes internally.
    let diagnostic = cpu_ms();
    let diagnostic_files = scale.min(32);
    let last = 1000 + diagnostic_files;
    db.execute_batch(&format!("BEGIN; UPDATE kfreq SET n=n-coalesce((SELECT sum(n) FROM file_kind_counts WHERE fid>=1000 AND fid<{last} AND code=kfreq.code),0); DELETE FROM kfreq WHERE n=0; DELETE FROM file WHERE fid>=1000 AND fid<{last}; COMMIT"))?;
    let mut counts = Executions::default();
    let mut diagnostic_meter = Meter::default();
    // SAFETY: counter address remains stable during synchronous SQLite work, and
    // the callback is detached even if publication returns an error.
    unsafe {
        rusqlite::ffi::sqlite3_trace_v2(
            db.handle(),
            rusqlite::ffi::SQLITE_TRACE_STMT | rusqlite::ffi::SQLITE_TRACE_PROFILE,
            Some(trace),
            (&mut counts as *mut Executions).cast(),
        );
    }
    let replay = (|| {
        for lo in (1000..last).step_by(packet) {
            publish(
                &db,
                lo,
                (lo + packet).min(last),
                set_stats,
                &mut diagnostic_meter,
            )?;
        }
        Ok::<_, rusqlite::Error>(())
    })();
    unsafe {
        rusqlite::ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut());
    }
    replay?;
    let diagnostic_cpu = cpu_ms() - diagnostic;
    // Every host statement is submitted through this meter during the traced replay.
    // SQLite's additional PROFILE completions are internal work, not new submissions.
    let diagnostic_host = serde_json::to_value(&diagnostic_meter)?["statements"]
        .as_u64()
        .unwrap() as usize;
    assert!(counts.profile_events >= diagnostic_host);
    let internal_executions = counts.profile_events - diagnostic_host;
    assert_state(&db)?;
    assert_eq!(before, state(&db)?);
    let mut cache_used = 0;
    let mut high = 0;
    // SAFETY: live connection and valid SQLite output pointers.
    unsafe {
        rusqlite::ffi::sqlite3_db_status(
            db.handle(),
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED,
            &mut cache_used,
            &mut high,
            0,
        );
    }
    Ok(
        json!({"format":"rerail.cold-cpu-spike/v1","experiment":"publication","variant":variant,"scale":scale,"cpuMs":common_cpu+kernel,"kernelCpuMs":kernel,"commonCpuMs":common_cpu,"processCpuMs":cpu_ms()-process,"peakRssBytes":peak_bytes(),"sqliteCacheBytes":cache_used,"cacheBefore":cache_before,"cacheAfter":cache_after,"cacheKiB":cache,"packetFiles":packet,"transactions":scale.div_ceil(packet),"sql":m,"rows":before,"rollback":true,"replacement":true,"pinnedSqlSnapshot":true,"diagnosticExecutions":counts,"diagnosticSqliteInternalExecutions":internal_executions,"diagnosticHost":diagnostic_meter,"diagnosticFiles":diagnostic_files,"diagnosticCpuMs":diagnostic_cpu,"sqliteVersion":rusqlite::version(),"scope":"Exact production FTS/value schema and real file-2 tape/posting bytes, replicated under separate file IDs. Includes image staging, SQL statistics, writes and commits. Verification and traced replay are separate. Parsing, source image encoding, engine version ownership and scheduling are not modeled. A pinned SQLite reader is verified separately."}),
    )
}
