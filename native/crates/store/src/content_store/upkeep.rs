//! The store's upkeep jobs (decision 0190 §4): a SQLite file kept small and its statistics fresh ([`SqliteFile`]),
//! the store's collection ([`Collect`]), and the memo entries a keep decision left unreachable ([`MemoSweep`]), each a
//! `rerail_accel::upkeep::Job`.

#[allow(unused_imports)]
use crate::content_store::Cached as _;
use super::{CapacityState, ContentStore, CorpusPolicy};
use rerail_accel::upkeep::{Due, Job, Qos, Slice, Step};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

pub use rerail_store_base::content_store::upkeep::{SqliteFile, Tables, Work, disk_free, interrupted, leaves_to_whole_vacuum};

/// Sources and tapes one archive step takes of each (`ContentStore::archive_aged`).
const ARCHIVE_BATCH: usize = 256;

/// The store's collection (decision 0053): corpus runs' passes when collection is due (idle work, run within
/// an hour of waiting), and at once past the store's soft limit (urgent work); then the archive of what is aged
/// (decision 0450 step 4).
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
        if store.corpus_pass_wanted(&self.policy, unix_now()).unwrap_or(false) || store.archive_due().unwrap_or(false) {
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
                Ok(_) => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        // Then the archive (decision 0450 step 4): what collection left aged, compressed into rows.
        while !slice.over() {
            match store.archive_due().and_then(|due| if due { store.archive_aged(now, ARCHIVE_BATCH) } else { Ok(0) }) {
                Ok(0) => return Ok(Step::Done),
                Ok(_) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(Step::More)
    }
}

/// The memo entries the last keep decision left unreachable, collected (decision 0227): a utility job ([`Qos::Utility`]),
/// due once a decision is past the last sweep (`corpus_decided.swept`), so the sweep runs soon after the decision, behind the
/// run, and before the process exits (`upkeep::finish_soon`). A decision left unswept (a process stopped) is the
/// next process's to sweep.
pub struct MemoSweep {
    path: PathBuf,
    name: String,
}

impl MemoSweep {
    pub fn new(path: &Path) -> Self {
        let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        MemoSweep { path: path.to_path_buf(), name: format!("memo sweep {name}") }
    }

    /// The job for the store at `path`, registered once a process: the runs over that store share it (the
    /// registry finds it by its name and file).
    pub fn register_once(path: &Path) {
        rerail_accel::upkeep::register_for_process(Box::new(MemoSweep::new(path)));
    }
}

impl Job for MemoSweep {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn due(&mut self) -> Due {
        rerail_prof::code_rule_dsl::logic::profile::behind_the_run();
        // A point read on a connection of its own, closed after it: at most one a poll (10 s). Kept from poll to poll,
        // it held the store's file open between them: a store removed (a test's, with its directory) was held open
        // until the next poll found its path gone. A store forgotten (`ContentStore::forget`) is read by no check
        // under way as it is removed (a read connection makes the store's `-shm` file): its jobs are taken out under
        // the registry's lock, which a check runs under, and the steps' (`upkeep::forget_file`).
        if !self.path.exists() {
            return Due::No;
        }
        let wanted = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .is_ok_and(|c| super::corpus_runs::memo_sweep_wanted(&c).unwrap_or(false));
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
        // A store gone since the job became due has nothing to sweep, and opening it would make a new one. (One
        // forgotten has no job left to step.)
        if !self.path.exists() {
            return Ok(Step::Done);
        }
        let store = ContentStore::open(&self.path).map_err(|e| e.to_string())?;
        // Stopped by a run that began: rolled back, and still due.
        let Some((n, seconds)) = store.sweep_memo(slice.stopper()).map_err(|e| e.to_string())? else { return Ok(Step::More) };
        rerail_prof::code_rule_dsl::logic::profile::decision("unreachable memo entries collected (decision 0227)", || format!("{n} memo entries, {seconds:.3} s"));
        Ok(Step::Done)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn removal_waits_for_maintenance_with_an_active_wal_reader() {
        use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
        // A job that holds a WAL reader on the store, as a collector's poll does (the frame collector's was, before
        // kept facts retired it, 0456).
        struct ReadWal {
            path: PathBuf,
            reader: Option<Connection>,
            reading: Option<SyncSender<()>>,
            finish: Receiver<()>,
        }
        impl Job for ReadWal {
            fn file(&self) -> Option<PathBuf> { Some(self.path.clone()) }
            fn name(&self) -> String { "test WAL retirement".to_owned() }
            fn due(&mut self) -> Due {
                if self.reading.is_none() { return Due::No; }
                if self.reader.is_none() {
                    self.reader = Some(Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).unwrap());
                }
                Due::Urgent
            }
            fn step(&mut self, _: &Slice) -> Result<Step, String> {
                let db = self.reader.as_ref().expect("the job opened its reader");
                db.execute_batch("BEGIN").unwrap();
                let read = || db.query_row("SELECT generation FROM analysis_scope_generation WHERE scope = 'test'", [], |row| row.get::<_, i64>(0)).unwrap();
                assert_eq!(read(), 2);
                self.reading.take().unwrap().send(()).unwrap();
                self.finish.recv_timeout(Duration::from_secs(5)).unwrap();
                assert_eq!(read(), 2);
                db.execute_batch("ROLLBACK").unwrap();
                Ok(Step::Done)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store.sqlite");
        drop(ContentStore::open_images(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
            INSERT INTO analysis_scope_generation VALUES ('test', 1);
            PRAGMA wal_checkpoint(TRUNCATE);
            UPDATE analysis_scope_generation SET generation = 2 WHERE scope = 'test'").unwrap();
        assert!(std::fs::metadata(directory.path().join("store.sqlite-wal")).unwrap().len() > 32);
        let (entered, active) = sync_channel(0);
        let (release, finish) = sync_channel(0);
        let _job = rerail_accel::upkeep::register(Box::new(ReadWal { path: path.clone(), reader: None, reading: Some(entered), finish }));
        std::thread::scope(|threads| {
            threads.spawn(rerail_accel::upkeep::run_now);
            active.recv_timeout(Duration::from_secs(5)).unwrap();
            let (started, removing) = sync_channel(0);
            let (retired, removed) = sync_channel(0);
            let directory = directory.path();
            threads.spawn(move || {
                started.send(()).unwrap();
                rerail_accel::upkeep::retire_files_under(directory);
                retired.send(()).unwrap();
            });
            removing.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(removed.recv_timeout(Duration::from_millis(30)).is_err());
            release.send(()).unwrap();
            removed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        db.close().unwrap();
        directory.close().unwrap();
    }

    /// A store removed after its last keep decision (a test's, with its directory) is due no more, and its job makes
    /// no store at its path: its reader, opened before the removal, still read the old file's unswept decision.
    #[test]
    fn a_removed_stores_memo_sweep_is_not_due_and_no_store_is_made_again() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("test-store");
        let path = folder.join("store.sqlite");
        drop(ContentStore::open(&path).unwrap());
        let c = Connection::open(&path).unwrap();
        // A decision past the last sweep (the corpus runs' kept form, `corpus_runs::kept`).
        crate::content_store::corpus_runs::kept::write_at(&c, |k, w| {
            let mut state = k.state();
            state.decided_run = state.swept + 1;
            w.state(state);
            Ok(())
        })
        .unwrap();
        drop(c);
        let mut job = MemoSweep::new(&path);
        assert_eq!(job.due(), Due::Urgent, "a decision past the last sweep");
        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(job.due(), Due::No, "the store is gone");
        let stop = AtomicBool::new(false);
        assert_eq!(job.step(&Slice::for_test(Duration::from_millis(200), 64 << 20, &stop)), Ok(Step::Done));
        assert!(!folder.exists(), "no store is made where the removed one was");
    }
}
