//! The store's upkeep jobs (decision 0190 §4): a SQLite file kept small and its statistics fresh ([`SqliteFile`]),
//! the store's collection ([`Collect`]), and the frames a keep decision left unreachable ([`Frames`]), each a
//! `rerail_accel::upkeep::Job`.

#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use super::{CapacityState, ContentStore, CorpusPolicy};
use rerail_accel::upkeep::{Due, Job, Qos, Slice, Step};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
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
        let mut work = Work { checkpoint: self.wal_bytes() > WAL_BYTES, ..Default::default() };
        (work.free_pages, work.vacuum) = give_back(pages, free, page, auto, || disk_free(&self.path));
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
pub(super) fn leaves_to_whole_vacuum(connection: &Connection, path: &Path) -> rusqlite::Result<bool> {
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

    fn file(&self) -> Option<PathBuf> {
        self.store.upgrade().map(|s| s.database_path().to_path_buf())
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

/// Whether `e` is SQLite's interrupt: a step's progress handler stopped the statement, for a run that began
/// (`Slice::stopper`). Its transaction is rolled back.
pub fn interrupted(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted)
}

/// The frames the last keep decision left unreachable, collected (decision 0227): a utility job ([`Qos::Utility`]),
/// due once a decision is past the last sweep (`corpus_decided.swept`), so the sweep runs soon after the decision, behind the
/// run, and before the process exits (`upkeep::finish_soon`). A decision left unswept (a process stopped) is the
/// next process's to sweep.
pub struct Frames {
    path: PathBuf,
    name: String,
    /// A read connection for `due` (a point read at each poll), and the identity of the file it opened.
    reader: Option<(Connection, Option<FileIdentity>)>,
}

impl Frames {
    pub fn new(path: &Path) -> Self {
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        Frames { path: path.to_path_buf(), name: format!("frames {name}"), reader: None }
    }

    /// The job for the store at `path`, registered once a process: the runs over that store share it (the
    /// registry finds it by its name and file).
    pub fn register_once(path: &Path) {
        rerail_accel::upkeep::register_for_process(Box::new(Frames::new(path)));
    }
}

impl Job for Frames {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn due(&mut self) -> Due {
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        // The cached connection reads the file it opened: one deleted or replaced since (a test's store gone with
        // its directory, a store the user removed) would say a sweep is wanted for ever. So it is dropped once the
        // path names another file, or none.
        let now = identity(&self.path);
        if self.reader.as_ref().is_some_and(|(_, was)| now.is_none() || *was != now) {
            self.reader = None;
        }
        if self.reader.is_none() && now.is_some() {
            self.reader = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok().map(|c| (c, now));
        }
        let wanted = self.reader.as_ref().is_some_and(|(c, _)| super::corpus_runs::frames_sweep_wanted(c).unwrap_or(false));
        if wanted { Due::Urgent } else { Due::No }
    }

    fn qos(&self) -> Qos {
        Qos::Utility
    }

    fn file(&self) -> Option<PathBuf> {
        Some(self.path.clone())
    }

    fn step(&mut self, slice: &Slice) -> Result<Step, String> {
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        if slice.over() {
            return Ok(Step::More);
        }
        // A store gone since the job became due has nothing to sweep, and opening it would make a new one.
        if identity(&self.path).is_none() {
            self.reader = None;
            return Ok(Step::Done);
        }
        let store = ContentStore::open(&self.path).map_err(|e| e.to_string())?;
        // Stopped by a run that began: rolled back, and still due.
        let Some((u, seconds)) = store.sweep_frames(slice.stopper()).map_err(|e| e.to_string())? else { return Ok(Step::More) };
        rerail_prof::code_rule_dsl::logic::profile::decision("unreachable frames collected (decision 0227)", || {
            format!("{} memo entries, {} maintained, {} programs' rows ({} of programs no manifest names), {:.3} s", u.memo, u.maintained, u.programs, u.unnamed, seconds)
        });
        Ok(Step::Done)
    }
}

/// Which file a path names: its device and inode where the system has them, else its creation time.
type FileIdentity = (u64, u64);

fn identity(path: &Path) -> Option<FileIdentity> {
    let m = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        Some((m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let t = m.created().or_else(|_| m.modified()).ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some((t.as_secs(), u64::from(t.subsec_nanos())))
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
            tx.execute_cached("INSERT INTO rows(v) VALUES (?1)", [format!("{i:0>1000}")]).unwrap();
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
            assert!(only.analyze.is_empty(), "{auto}: the planner's table and a missing one are left out: {only:?}");
            let mut job = SqliteFile::new(&p, Tables::All);
            let work = job.probe().unwrap();
            assert_eq!(work.analyze, vec!["fact_stats".to_string(), "rows".to_string()], "{auto}");
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
