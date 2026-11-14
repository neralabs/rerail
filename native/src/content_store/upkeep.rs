//! The store's upkeep jobs (decision 0190 §4): a SQLite file kept small and its statistics fresh ([`SqliteFile`]),
//! and the store's collection ([`Collect`]), each a `rerail_accel::upkeep::Job`.

use super::{CapacityState, ContentStore, CorpusPolicy};
use rerail_accel::upkeep::{Due, Job, Slice, Step};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// A write-ahead log past this is checkpointed and truncated.
const WAL_BYTES: u64 = 64 << 20;
/// Free pages past this share of the file (and past `FREE_BYTES`) are given back to the disk.
const FREE_SHARE: f64 = 0.10;
const FREE_BYTES: u64 = 16 << 20;
/// A file with no incremental vacuum is vacuumed whole only past this share free, only up to `VACUUM_BYTES`, and
/// only with this many times its size free on the disk (the copy VACUUM writes).
const VACUUM_SHARE: f64 = 0.20;
const VACUUM_BYTES: u64 = 1 << 30;
const VACUUM_DISK: u64 = 3;
/// The pages an incremental vacuum step gives back (16 MB of 4 KB pages): the write lock held briefly, as the
/// collector's own vacuum does (4,096 pages a pass).
const VACUUM_PAGES: u64 = 4096;
/// The rows `ANALYZE` reads of each index (SQLite's approximate analysis): seconds on a large table otherwise.
const ANALYSIS_LIMIT: u32 = 1000;
/// Tables are analyzed again this long after their last analysis.
const ANALYZE_EVERY: Duration = Duration::from_secs(24 * 3600);
/// How often the file's pages are looked at (an open and four pragmas; the WAL's size is read at every poll).
const PROBE_EVERY: Duration = Duration::from_secs(600);

/// What a file's probe found to do, in the order it is done.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Work {
    pub checkpoint: bool,
    /// Pages to give back with `incremental_vacuum`.
    pub free_pages: u64,
    /// A whole `VACUUM` (no incremental vacuum, much free): converts the file to incremental vacuum.
    pub vacuum: bool,
    /// Tables to analyze.
    pub analyze: Vec<String>,
}

impl Work {
    fn is_empty(&self) -> bool {
        *self == Work::default()
    }
}

/// One SQLite file's upkeep: its write-ahead log checkpointed and truncated, its free pages given back, and the
/// tables named analyzed (approximately: `analysis_limit`), never a table whose statistics `fact_stats` writes
/// (`stats::plans::write_stat1`: the planner's statistics are its own, never `ANALYZE`'s). Each step is one
/// action on a connection of its own that waits for no lock (a file in use is tried at a later poll).
pub struct SqliteFile {
    path: PathBuf,
    tables: Tables,
    work: Work,
    probed: Option<Instant>,
    analyzed: Option<Instant>,
}

/// The tables a file's upkeep analyzes: never SQLite's own (`sqlite_*`) or one `fact_stats` names.
#[derive(Clone, Debug)]
pub enum Tables {
    All,
    Only(Vec<String>),
}

impl SqliteFile {
    /// The upkeep of the file at `path`, analyzing `tables`.
    pub fn new(path: &Path, tables: Tables) -> Self {
        SqliteFile { path: path.to_path_buf(), tables, work: Work::default(), probed: None, analyzed: None }
    }

    fn wal_bytes(&self) -> u64 {
        let mut wal = self.path.as_os_str().to_owned();
        wal.push("-wal");
        std::fs::metadata(wal).map_or(0, |m| m.len())
    }

    fn connect(&self, memory: usize) -> rusqlite::Result<Connection> {
        let db = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        db.busy_timeout(Duration::from_millis(50))?;
        // The page cache within the job's memory (a negative size is KiB).
        db.pragma_update(None, "cache_size", -((memory / 2 / 1024).max(256) as i64))?;
        Ok(db)
    }

    /// What there is to do, read from the file.
    pub fn probe(&self) -> rusqlite::Result<Work> {
        let db = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        db.busy_timeout(Duration::from_millis(50))?;
        let get = |p: &str| db.query_row(&format!("PRAGMA {p}"), [], |r| r.get::<_, i64>(0)).map(|v| v.max(0) as u64);
        let (pages, free, page, auto) = (get("page_count")?, get("freelist_count")?, get("page_size")?, get("auto_vacuum")?);
        let bytes = pages * page;
        let much_free = free * page > FREE_BYTES;
        let mut work = Work { checkpoint: self.wal_bytes() > WAL_BYTES, ..Default::default() };
        if auto == 2 && much_free && free as f64 > FREE_SHARE * pages as f64 {
            work.free_pages = free;
        }
        if auto == 0 && much_free && free as f64 > VACUUM_SHARE * pages as f64 && bytes <= VACUUM_BYTES && disk_free(&self.path).is_some_and(|d| d > VACUUM_DISK * bytes) {
            work.vacuum = true;
        }
        if self.analyzed.is_none_or(|t| t.elapsed() >= ANALYZE_EVERY) {
            let has = |sql: &str, t: &str| db.query_row(sql, [t], |_| Ok(())).optional().map(|r| r.is_some());
            let stat1 = has("SELECT 1 FROM sqlite_schema WHERE name = ?1", "sqlite_stat1")?;
            let fact_stats = has("SELECT 1 FROM sqlite_schema WHERE name = ?1", "fact_stats")?;
            let all: Vec<String> = db
                .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let tables = match &self.tables {
                Tables::All => all,
                Tables::Only(only) => only.iter().filter(|t| all.contains(t)).cloned().collect(),
            };
            for t in &tables {
                if fact_stats && has("SELECT 1 FROM fact_stats WHERE tbl = ?1 LIMIT 1", t)? {
                    continue;
                }
                // Analyzed in this process's day, or before it (rows in `sqlite_stat1`): only after a day here.
                let analyzed = stat1 && has("SELECT 1 FROM sqlite_stat1 WHERE tbl = ?1 LIMIT 1", t)?;
                if !analyzed || self.analyzed.is_some() {
                    work.analyze.push(t.clone());
                }
            }
        }
        Ok(work)
    }
}

/// The bytes free on the disk holding `path`.
pub fn disk_free(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail as u64 * s.f_frsize as u64)
}

impl Job for SqliteFile {
    fn name(&self) -> String {
        format!("sqlite {}", self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned()))
    }

    fn due(&mut self) -> Due {
        // Upkeep is behind the run: its SQL is on its own connections, never a wave's (decision 0104).
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        if !self.path.exists() {
            return Due::No;
        }
        if self.wal_bytes() > WAL_BYTES {
            self.work.checkpoint = true;
        }
        if self.probed.is_none_or(|t| t.elapsed() >= PROBE_EVERY) {
            self.probed = Some(Instant::now());
            if let Ok(w) = self.probe() {
                self.work = Work { checkpoint: self.work.checkpoint || w.checkpoint, ..w };
            }
        }
        match &self.work {
            w if w.is_empty() => Due::No,
            // A whole vacuum holds the file's writers for its length: an idle machine's work only.
            w if w.vacuum && !w.checkpoint && w.free_pages == 0 && w.analyze.is_empty() => Due::Idle { starve: Duration::MAX },
            w if w.checkpoint || w.free_pages > 0 => Due::Idle { starve: Duration::from_secs(3600) },
            _ => Due::Idle { starve: Duration::from_secs(7 * 24 * 3600) },
        }
    }

    fn step(&mut self, slice: &Slice) -> Result<Step, String> {
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        let err = |e: rusqlite::Error| format!("{}: {e}", self.path.display());
        let db = self.connect(slice.memory).map_err(err)?;
        // Pages given back reach the disk at the checkpoint after (the file is truncated there, under WAL).
        if self.work.free_pages > 0 {
            let n = self.work.free_pages.min(VACUUM_PAGES);
            super::reclamation::incremental_vacuum(&db, n as u32).map_err(|e| format!("{}: {e}", self.path.display()))?;
            self.work.free_pages -= n;
            self.work.checkpoint = true;
        } else if self.work.vacuum {
            db.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM").map_err(err)?;
            self.work.vacuum = false;
            self.work.checkpoint = true;
        } else if self.work.checkpoint {
            // TRUNCATE waits for no reader (busy: it checkpoints what it can, and is tried again at a later poll).
            let busy: i64 = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0)).map_err(err)?;
            self.work.checkpoint = false;
            if busy != 0 {
                return Ok(Step::Done);
            }
        } else if let Some(t) = self.work.analyze.first().cloned() {
            db.pragma_update(None, "analysis_limit", ANALYSIS_LIMIT).map_err(err)?;
            db.execute_batch(&format!("ANALYZE main.\"{}\"", t.replace('"', "\"\""))).map_err(err)?;
            self.work.analyze.remove(0);
            if self.work.analyze.is_empty() {
                self.analyzed = Some(Instant::now());
            }
        }
        let _ = slice;
        Ok(if self.work.is_empty() { Step::Done } else { Step::More })
    }
}

/// The store's collection (decision 0053): corpus runs' passes when collection is due (idle work, run within
/// an hour of waiting), and at once past the store's soft limit (urgent work).
pub struct Collect {
    store: Weak<ContentStore>,
    name: String,
    policy: CorpusPolicy,
}

impl Collect {
    pub fn new(store: &Arc<ContentStore>) -> Self {
        let name = store.database_path().file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        Collect { store: Arc::downgrade(store), name: format!("collect {name}"), policy: CorpusPolicy::default() }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Job for Collect {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn due(&mut self) -> Due {
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        let Some(store) = self.store.upgrade() else { return Due::No };
        if store.capacity().is_ok_and(|c| !matches!(c.state, CapacityState::Normal)) {
            return Due::Urgent;
        }
        if store.corpus_pass_wanted(&self.policy, unix_now()).unwrap_or(false) {
            return Due::Idle { starve: Duration::from_secs(3600) };
        }
        Due::No
    }

    fn step(&mut self, slice: &Slice) -> Result<Step, String> {
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        let Some(store) = self.store.upgrade() else { return Ok(Step::Done) };
        let now = unix_now();
        while !slice.over() {
            match store.collect_corpus_pass(&self.policy, now) {
                Ok(pass) if pass.did_work() => {}
                Ok(_) => return Ok(Step::Done),
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(Step::More)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn file(dir: &Path, auto: &str) -> PathBuf {
        let p = dir.join(format!("upkeep-{auto}.sqlite"));
        let db = Connection::open(&p).unwrap();
        db.execute_batch(&format!(
            "PRAGMA auto_vacuum = {auto}; PRAGMA journal_mode = WAL;
             CREATE TABLE rows(k INTEGER PRIMARY KEY, v TEXT); CREATE INDEX rows_v ON rows(v);
             CREATE TABLE fact_stats(tbl TEXT, col TEXT, rows INTEGER, sq INTEGER);
             CREATE TABLE facts(a INTEGER); INSERT INTO fact_stats VALUES ('facts', 'a', 1, 1);"
        ))
        .unwrap();
        let tx = db.unchecked_transaction().unwrap();
        for i in 0..20_000 {
            tx.execute("INSERT INTO rows(v) VALUES (?1)", [format!("{i:0>1000}")]).unwrap();
        }
        tx.commit().unwrap();
        db.execute_batch("DELETE FROM rows WHERE k % 4 != 0; PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        p
    }

    fn run(job: &mut SqliteFile) -> u32 {
        let stop = AtomicBool::new(false);
        let mut steps = 0;
        while job.due() != Due::No && steps < 100 {
            steps += 1;
            job.step(&Slice::for_test(Duration::from_millis(200), 64 << 20, &stop)).unwrap();
        }
        steps
    }

    #[test]
    fn free_pages_go_back_and_own_tables_are_analyzed() {
        let dir = tempfile::tempdir().unwrap();
        for auto in ["INCREMENTAL", "NONE"] {
            let p = file(dir.path(), auto);
            let before = std::fs::metadata(&p).unwrap().len();
            let only = SqliteFile::new(&p, Tables::Only(vec!["facts".into(), "missing".into()])).probe().unwrap();
            assert!(only.analyze.is_empty(), "{auto}: the planner's table and a missing one are left out: {only:?}");
            let mut job = SqliteFile::new(&p, Tables::All);
            let work = job.probe().unwrap();
            assert_eq!(work.analyze, vec!["fact_stats".to_string(), "rows".to_string()], "{auto}");
            assert_eq!((work.free_pages > 0, work.vacuum), (auto == "INCREMENTAL", auto == "NONE"), "{auto}: {work:?}");
            assert!(run(&mut job) > 0);
            let after = std::fs::metadata(&p).unwrap().len();
            assert!(after * 2 < before, "{auto}: {before} -> {after}");
            let db = Connection::open(&p).unwrap();
            let tbls: Vec<String> = db.prepare("SELECT DISTINCT tbl FROM sqlite_stat1").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
            assert!(!tbls.contains(&"facts".to_string()), "{tbls:?}");
            assert!(tbls.contains(&"rows".to_string()), "{tbls:?}");
            assert_eq!(db.query_row("PRAGMA auto_vacuum", [], |r| r.get::<_, i64>(0)).unwrap(), 2, "{auto}: incremental after");
            assert_eq!(job.due(), Due::No);
        }
    }
}
