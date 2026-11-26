//! A SQLite file's upkeep (decision 0190 §4): kept small and its statistics fresh ([`SqliteFile`]), a
//! `rerail_accel::upkeep::Job`. Any SQLite file's, the store's and the cost statistics' alike: the store's own jobs
//! (`Collect`, `MemoSweep`) are in rerail-store.

#[allow(unused_imports)]
use crate::content_store::Cached as _;
use rerail_accel::upkeep::{Due, Job, Slice, Step};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A write-ahead log past this is checkpointed and truncated.
const WAL_BYTES: u64 = 64 << 20;
/// Free pages past this share of the file (and past `FREE_BYTES`) are given back to the disk.
const FREE_SHARE: f64 = 0.10;
const FREE_BYTES: u64 = 16 << 20;
/// A file is vacuumed whole past this share free (either vacuum mode: [`give_back`]), only while its live pages are
/// within `VACUUM_BYTES`, and only with this many times their size free on the disk (the copy VACUUM writes).
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
/// tables named analyzed (approximately: `analysis_limit`). Each step is one
/// action on a connection of its own that waits for no lock (a file in use is tried at a later poll).
pub struct SqliteFile {
    path: PathBuf,
    tables: Tables,
    work: Work,
    probed: Option<Instant>,
    analyzed: Option<Instant>,
}

/// The tables a file's upkeep analyzes: never SQLite's own (`sqlite_*`).
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
        let mut work = Work { checkpoint: self.wal_bytes() > WAL_BYTES, ..Default::default() };
        (work.free_pages, work.vacuum) = give_back(pages, free, page, auto, || disk_free(&self.path));
        if self.analyzed.is_none_or(|t| t.elapsed() >= ANALYZE_EVERY) {
            let has = |sql: &str, t: &str| db.query_row(sql, [t], |_| Ok(())).optional().map(|r| r.is_some());
            let stat1 = has("SELECT 1 FROM sqlite_schema WHERE name = ?1", "sqlite_stat1")?;
            let all: Vec<String> = db
                .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let tables = match &self.tables {
                Tables::All => all,
                Tables::Only(only) => only.iter().filter(|t| all.contains(t)).cloned().collect(),
            };
            for t in &tables {
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

/// How a file's free pages go back to the disk: (pages an incremental vacuum gives back, whether a whole `VACUUM`
/// does instead). Past `VACUUM_SHARE` free a whole `VACUUM` is chosen in either mode, when the live pages are within
/// `VACUUM_BYTES` and the disk has room for their copy: its cost is the live pages', where an incremental vacuum's
/// grows with the free list (each page it moves walks the list's trunks: 8.1 s for 4,096 pages with 1.8M free, where
/// a whole `VACUUM` of the same file took 10 s).
fn give_back(pages: u64, free: u64, page: u64, auto: u64, disk: impl FnOnce() -> Option<u64>) -> (u64, bool) {
    if free * page <= FREE_BYTES || (auto != 0 && auto != 2) {
        return (0, false);
    }
    let live = (pages - free.min(pages)) * page;
    if free as f64 > VACUUM_SHARE * pages as f64 && live <= VACUUM_BYTES && disk().is_some_and(|d| d > VACUUM_DISK * live) {
        return (0, true);
    }
    (if auto == 2 && free as f64 > FREE_SHARE * pages as f64 { free } else { 0 }, false)
}

/// Whether the file `connection` has open is free enough that a whole `VACUUM` gives its pages back ([`give_back`]):
/// the collector's own incremental vacuum after a pass leaves it to the upkeep job (ep2: 8.1 s a pass, behind every run).
pub fn leaves_to_whole_vacuum(connection: &Connection, path: &Path) -> rusqlite::Result<bool> {
    let get = |p: &str| connection.query_row(&format!("PRAGMA {p}"), [], |r| r.get::<_, i64>(0)).map(|v| v.max(0) as u64);
    let (pages, free, page, auto) = (get("page_count")?, get("freelist_count")?, get("page_size")?, get("auto_vacuum")?);
    Ok(give_back(pages, free, page, auto, || disk_free(path)).1)
}

/// The bytes free on the disk holding `path` (unknown where the system has no `statvfs`: WASI).
pub fn disk_free(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: statvfs writes the C-layout struct it is given; the path is NUL-terminated.
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        (unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail as u64 * s.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
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

    fn file(&self) -> Option<PathBuf> {
        Some(self.path.clone())
    }

    fn step(&mut self, slice: &Slice) -> Result<Step, String> {
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        let err = |e: rusqlite::Error| format!("{}: {e}", self.path.display());
        let mut db = self.connect(slice.memory).map_err(err)?;
        // A run that begins interrupts the action (a `VACUUM` holds the file's writers for its length): rolled back,
        // and the work left as it was, still due.
        db.progress_handler(1000, Some(slice.stopper())).map_err(err)?;
        let done = (|| -> rusqlite::Result<Option<Step>> {
            // Pages given back reach the disk at the checkpoint after (the file is truncated there, under WAL).
            if self.work.free_pages > 0 {
                let n = self.work.free_pages.min(VACUUM_PAGES);
                let mut statement = db.prepare(&format!("PRAGMA incremental_vacuum({n})"))?;
                let mut rows = statement.query([])?;
                while rows.next()?.is_some() {}
                self.work.free_pages -= n;
                self.work.checkpoint = true;
            } else if self.work.vacuum {
                db.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM")?;
                self.work.vacuum = false;
                self.work.checkpoint = true;
            } else if self.work.checkpoint {
                // TRUNCATE waits for no reader (busy: it checkpoints what it can, and is tried again at a later poll).
                let busy: i64 = db.query_row_cached("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
                self.work.checkpoint = false;
                if busy != 0 {
                    return Ok(Some(Step::Done));
                }
            } else if let Some(t) = self.work.analyze.first().cloned() {
                db.pragma_update(None, "analysis_limit", ANALYSIS_LIMIT)?;
                db.execute_batch(&format!("ANALYZE main.\"{}\"", t.replace('"', "\"\"")))?;
                self.work.analyze.remove(0);
                if self.work.analyze.is_empty() {
                    self.analyzed = Some(Instant::now());
                }
            }
            Ok(None)
        })();
        match done {
            Ok(Some(step)) => Ok(step),
            Ok(None) => Ok(if self.work.is_empty() { Step::Done } else { Step::More }),
            Err(e) if interrupted(&e) => Ok(Step::More),
            Err(e) => Err(err(e)),
        }
    }
}

/// Whether `e` is SQLite's interrupt: a step's progress handler stopped the statement, for a run that began
/// (`Slice::stopper`). Its transaction is rolled back.
pub fn interrupted(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn file(dir: &Path, auto: &str) -> PathBuf {
        let p = dir.join(format!("upkeep-{auto}.sqlite"));
        let db = Connection::open(&p).unwrap();
        db.execute_batch(&format!(
            "PRAGMA auto_vacuum = {auto}; PRAGMA journal_mode = WAL; PRAGMA synchronous = OFF;
             CREATE TABLE rows(k INTEGER PRIMARY KEY, v TEXT, w INTEGER); CREATE INDEX rows_w ON rows(w);
             CREATE TABLE facts(a INTEGER);"
        ))
        .unwrap();
        let tx = db.unchecked_transaction().unwrap();
        // More than `FREE_BYTES` free once three quarters are deleted (18 MB of 24): the rows' text, its index a small
        // column's (an index of the text doubled what the fixture wrote), unsynced (the fixture is no subject).
        for i in 0..24_000i64 {
            tx.execute_cached("INSERT INTO rows(v, w) VALUES (?1, ?2)", rusqlite::params![format!("{i:0>1000}"), i % 64]).unwrap();
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
    fn a_file_mostly_free_is_vacuumed_whole_in_either_mode() {
        const GB: u64 = 1 << 30;
        let disk = || Some(100 * GB);
        // ep2's store after its frames backlog went: 1,818,329 of 1,993,536 pages free (91%), 718 MB live.
        assert_eq!(give_back(1_993_536, 1_818_329, 4096, 2, disk), (0, true));
        assert_eq!(give_back(1_993_536, 1_818_329, 4096, 0, disk), (0, true));
        // A share between `FREE_SHARE` and `VACUUM_SHARE`: incrementally, and not at all without incremental mode.
        assert_eq!(give_back(1_000_000, 150_000, 4096, 2, disk), (150_000, false));
        assert_eq!(give_back(1_000_000, 150_000, 4096, 0, disk), (0, false));
        // Live pages past `VACUUM_BYTES`, or no room for their copy: incrementally where the file can.
        assert_eq!(give_back(1_000_000, 500_000, 4096, 2, disk), (500_000, false));
        assert_eq!(give_back(1_993_536, 1_818_329, 4096, 2, || Some(GB)), (1_818_329, false));
        assert_eq!(give_back(1_993_536, 1_818_329, 4096, 0, || None), (0, false));
        // Few free bytes, or full vacuum mode: nothing.
        assert_eq!(give_back(10_000, 3_000, 4096, 2, disk), (0, false));
        assert_eq!(give_back(1_993_536, 1_818_329, 4096, 1, disk), (0, false));
    }

    #[test]
    fn free_pages_go_back_and_own_tables_are_analyzed() {
        let dir = tempfile::tempdir().unwrap();
        for auto in ["INCREMENTAL", "NONE"] {
            let p = file(dir.path(), auto);
            let before = std::fs::metadata(&p).unwrap().len();
            let only = SqliteFile::new(&p, Tables::Only(vec!["facts".into(), "missing".into()])).probe().unwrap();
            assert_eq!(only.analyze, vec!["facts".to_string()], "{auto}: a missing table is left out: {only:?}");
            let mut job = SqliteFile::new(&p, Tables::All);
            let work = job.probe().unwrap();
            assert_eq!(work.analyze, vec!["facts".to_string(), "rows".to_string()], "{auto}");
            // Three quarters free: a whole vacuum in either mode.
            assert_eq!((work.free_pages, work.vacuum), (0, true), "{auto}: {work:?}");
            assert!(run(&mut job) > 0);
            let after = std::fs::metadata(&p).unwrap().len();
            assert!(after * 2 < before, "{auto}: {before} -> {after}");
            let db = Connection::open(&p).unwrap();
            let tbls: Vec<String> = db.prepare("SELECT DISTINCT tbl FROM sqlite_stat1").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
            assert!(!tbls.contains(&"facts".to_string()), "{tbls:?}");
            assert!(tbls.contains(&"rows".to_string()), "{tbls:?}");
            assert_eq!(db.query_row_cached("PRAGMA auto_vacuum", [], |r| r.get::<_, i64>(0)).unwrap(), 2, "{auto}: incremental after");
            assert_eq!(job.due(), Due::No);
        }
    }
}
