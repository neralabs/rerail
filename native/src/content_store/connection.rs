//! Per-thread SQLite connections, connection pragmas, each write transaction's turn in the store's
//! writer line, the checkpoint a commit leaves for after its turn, and the busy retry a write runs
//! under for a writer of another process.

use super::writer_line::{Refusal, Turn, Wait, WriterLine};
use super::{ContentStore, StoreError, StoreResult};
use parking_lot::{ArcMutexGuard, Mutex as ParkingMutex, RawMutex};
use rusqlite::hooks::Wal;
use rusqlite::wal::CheckpointMode;
use rusqlite::{Connection, MAIN_DB, OpenFlags, Transaction, TransactionBehavior};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_int;
use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long the store writes of one operation wait on other writers in all before they report
/// `Busy`: their turns in the store's writer line (`take_turn`), which end at the release of the
/// writer ahead or at their owner's cancel (`waits_end_when`), and reach it only when the
/// writers ahead stop letting go (a disk that stalls), and SQLite's lock held by a writer of
/// another process, which each attempt waits for at most one busy timeout (`wait_for_writer`), so
/// a cancelled write notices between attempts.
const BUSY_DEADLINE: Duration = Duration::from_secs(10);
/// The first and the largest backoff between a write's attempts on a store another process holds.
const BUSY_BACKOFF_FIRST: Duration = Duration::from_millis(2);
const BUSY_BACKOFF_MAX: Duration = Duration::from_millis(200);
/// How long one attempt waits inside SQLite on a writer of another process before it reports
/// `Busy`.
const BUSY_TIMEOUT: Duration = Duration::from_millis(250);
/// SQLite's own delays between the checks of a busy lock (`sqliteDefaultBusyCallback`).
const BUSY_POLL_DELAYS_MS: [u64; 12] = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];

#[cfg(test)]
type AttemptHook = Box<dyn FnMut(Duration) -> Duration>;
#[cfg(test)]
type WaitHook = Box<dyn FnMut(usize)>;
#[cfg(test)]
thread_local! {
    /// Replaces `BUSY_DEADLINE` for writes on this thread, so a test reaches the deadline fast.
    pub(crate) static BUSY_DEADLINE_OVERRIDE: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
    /// Replaces the busy timeout (one attempt's wait) of writes on this thread, so a test's
    /// deadline spans many attempts instead of one or two: a scheduler stall
    /// then cannot decide how many retries it sees.
    pub(crate) static BUSY_TIMEOUT_OVERRIDE: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };

    /// Runs on each of this thread's write attempts that found the store busy, with how long the
    /// operation's writes have waited so far, and returns time to add to that wait without
    /// sleeping it: a test spends a deadline without waiting for it.
    pub(crate) static BUSY_ATTEMPT_HOOK: RefCell<Option<AttemptHook>> =
        const { RefCell::new(None) };
    /// Runs on each check of a busy lock inside SQLite on this thread, with the number of checks
    /// so far in that wait: a test releases a writer once another is waiting on it.
    pub(crate) static BUSY_WAIT_HOOK: RefCell<Option<WaitHook>> =
        const { RefCell::new(None) };
}

thread_local! {
    /// How long this thread has waited on the lock inside the current SQLite wait.
    static WAIT_SPENT: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    /// How long this thread has waited on other writers, in the writer line and inside SQLite,
    /// since its write attempt began.
    static ATTEMPT_WAITED: std::cell::Cell<Duration> =
        const { std::cell::Cell::new(Duration::ZERO) };
    /// How this thread's writes wait for other writers (`Waits`).
    static WAITS: Cell<Waits> = const { Cell::new(Waits::InLine) };
    /// Whether this thread's last commit left the store's write-ahead log at the size SQLite's
    /// own automatic checkpoint starts at (`checkpoint_after_turn`).
    static CHECKPOINT_DUE: Cell<bool> = const { Cell::new(false) };
}

/// How a thread's writes wait for other writers.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Waits {
    /// In the writer line for this process's writers, then inside SQLite for another process's.
    InLine,
    /// Only inside SQLite, for another process's writer: the line's turn is taken only while no
    /// writer of this process holds the store or waits for it (`WritesOnlyWhenFree`).
    OnlyWhenFree,
    /// Never: the write gives up while any writer holds the store (`NoBusyWait`).
    Never,
}

thread_local! {
    /// How long the writes under this thread's `OperationDeadline` have waited on other writers;
    /// `None` outside one, where each write has a deadline of its own.
    static OPERATION_WAITED: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
    /// Whether a write under this thread's `OperationDeadline` ended `Busy`.
    static OPERATION_BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The cancel of the work this thread runs (`waits_end_when`), only while that call runs.
    static WAIT_CANCEL: Cell<Option<NonNull<dyn Fn() -> bool>>> = const { Cell::new(None) };
}

/// Runs `body` with this thread's waits for a turn in the store's writer lines ending once
/// `cancelled` holds, and returns what it returns: the owner of the work (a publication whose
/// workspace closes, the maintenance thread whose handle stops it) ends a wait it no longer needs at
/// once, not at the turn or the write deadline. Whoever makes `cancelled` hold wakes the waits
/// (`end_waits`), and a woken wait reads it again. Such a write fails as interrupted, never as
/// `Busy`, so it is not retried, marks no holder stalled and opens no busy window. A call inside
/// another reads only its own `cancelled` while it runs.
pub(crate) fn waits_end_when<T>(cancelled: &dyn Fn() -> bool, body: impl FnOnce() -> T) -> T {
    /// Puts the outer call's cancel back when `body` returns or unwinds.
    struct Restore(Option<NonNull<dyn Fn() -> bool>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            WAIT_CANCEL.set(self.0);
        }
    }
    // SAFETY: only the lifetime is erased. The pointer is read by `waits_ended`, on this thread
    // only, while `body` runs, and `Restore` takes it out when `body` returns or unwinds, so it is
    // never read after `cancelled`'s borrow ends; no caller can keep it past this call.
    let erased = unsafe {
        std::mem::transmute::<NonNull<dyn Fn() -> bool + '_>, NonNull<dyn Fn() -> bool + 'static>>(
            NonNull::from(cancelled),
        )
    };
    let _restore = Restore(WAIT_CANCEL.replace(Some(erased)));
    body()
}

/// Sets `cancel` and wakes every write waiting in this process's writer lines, so a wait whose
/// cancel reads it (`waits_end_when`) ends.
pub(crate) fn end_waits(cancel: &AtomicBool) {
    cancel.store(true, Ordering::Release);
    super::writer_line::wake_all();
}

/// Whether the work this thread runs was cancelled (`waits_end_when`).
fn waits_ended() -> bool {
    // SAFETY: `waits_end_when` keeps the pointer valid while it is set (see there).
    WAIT_CANCEL
        .get()
        .is_some_and(|cancelled| unsafe { cancelled.as_ref()() })
}

/// Makes every store write this thread runs while it lives share one `BUSY_DEADLINE` instead of
/// one each: an apply publishing in several passes, promoting, renewing and removing waits on
/// other writers for one deadline in all, not one per write; a Git snapshot publication likewise
/// across its batches. The deadline is the time the operation's writes spent waiting: attempts
/// that found the store busy, the backoff after them, and the time any attempt, a successful one
/// included, spent waiting for the write lock inside SQLite. Work between writes never spends it.
/// Once it is spent, a write makes a single attempt, and reports `Busy` if that finds the store
/// held, without retrying; a free store is still written. Writes outside a deadline
/// (maintenance) keep a deadline of their own. Entering one inside another joins the outer one.
pub(crate) struct OperationDeadline {
    owned: bool,
}

impl OperationDeadline {
    pub(crate) fn enter() -> Self {
        let owned = OPERATION_WAITED.get().is_none();
        if owned {
            OPERATION_WAITED.set(Some(Duration::ZERO));
            OPERATION_BUSY.set(false);
        }
        Self { owned }
    }
}

impl Drop for OperationDeadline {
    fn drop(&mut self) {
        if self.owned {
            OPERATION_WAITED.set(None);
            OPERATION_BUSY.set(false);
        }
    }
}

/// Whether a write under this thread's `OperationDeadline` ended `Busy`: its operation then left
/// something unwritten, whatever the store's state is now.
pub(crate) fn operation_found_busy() -> bool {
    OPERATION_BUSY.get()
}

fn busy_deadline() -> Duration {
    #[cfg(test)]
    if let Some(deadline) = BUSY_DEADLINE_OVERRIDE.get() {
        return deadline;
    }
    BUSY_DEADLINE
}

/// What other writers cost a write: its attempts that found the store busy, and the time those
/// attempts, the backoff after them and a successful attempt's wait for its turn took.
#[derive(Clone, Copy, Debug, Default)]
pub struct BusyRetries {
    pub retries: usize,
    pub wait_nanoseconds: u64,
}

/// Runs `attempt` until it returns anything but `Busy`, re-running it whole after a jittered
/// exponential backoff; `Busy` once `BUSY_DEADLINE` of waiting has passed, counting what the
/// thread's other writes under the same `OperationDeadline` already waited. Every store write
/// that takes the write lock runs under it, so one writer's long transaction delays the others
/// instead of failing them. An attempt waits its turn in the writer line first (`take_turn`), so
/// it finds SQLite's lock held only by a writer of another process.
pub(super) fn retry_busy<T>(
    retries: &mut BusyRetries,
    mut attempt: impl FnMut() -> StoreResult<T>,
) -> StoreResult<T> {
    let deadline = busy_deadline();
    let mut waited = OPERATION_WAITED.get().unwrap_or_default();
    let shared = OPERATION_WAITED.get().is_some();
    let mut backoff = BUSY_BACKOFF_FIRST;
    loop {
        ATTEMPT_WAITED.set(Duration::ZERO);
        let attempt_started = Instant::now();
        match attempt() {
            Err(StoreError::Busy) => {
                let attempted = attempt_started.elapsed();
                waited = waited.saturating_add(attempted);
                #[cfg(test)]
                {
                    let spent = waited;
                    waited = waited.saturating_add(BUSY_ATTEMPT_HOOK.with(|hook| {
                        hook.borrow_mut()
                            .as_mut()
                            .map_or(Duration::ZERO, |hook| hook(spent))
                    }));
                }
                // Counted also when it ends the write: a wait that reached the deadline shows
                // in the profile's retries and wait time.
                retries.retries += 1;
                add_wait(retries, attempted);
                let Some(remaining) = deadline.checked_sub(waited) else {
                    if shared {
                        OPERATION_WAITED.set(Some(waited));
                        OPERATION_BUSY.set(true);
                    }
                    return Err(StoreError::Busy);
                };
                let delay = jittered(backoff).min(remaining);
                waited = waited.saturating_add(delay);
                if shared {
                    OPERATION_WAITED.set(Some(waited));
                }
                add_wait(retries, delay);
                std::thread::sleep(delay);
                backoff = backoff.saturating_mul(2).min(BUSY_BACKOFF_MAX);
            }
            result => {
                // An attempt that got the lock after waiting for its turn or inside SQLite still
                // spent that time.
                add_wait(retries, ATTEMPT_WAITED.get());
                if shared {
                    OPERATION_WAITED.set(Some(waited.saturating_add(ATTEMPT_WAITED.get())));
                }
                return result;
            }
        }
    }
}

/// Adds `waited` to the time `retries` spent on other writers.
fn add_wait(retries: &mut BusyRetries, waited: Duration) {
    retries.wait_nanoseconds = retries
        .wait_nanoseconds
        .saturating_add(u64::try_from(waited.as_nanos()).unwrap_or(u64::MAX));
}

/// SQLite's busy handler of every store connection: waits for the write lock the way SQLite's
/// own busy timeout does (`BUSY_TIMEOUT` in all, `BUSY_POLL_DELAYS_MS` between checks), and
/// records what it waited, which an attempt that then gets the lock would otherwise leave
/// uncounted. It runs on the thread that runs the statement. A write transaction asks SQLite for
/// the lock only in its turn in the writer line, so it waits here for a writer of another process.
fn wait_for_writer(checks: i32) -> bool {
    if WAITS.get() == Waits::Never {
        return false;
    }
    let spent = if checks <= 0 {
        Duration::ZERO
    } else {
        WAIT_SPENT.get()
    };
    let Some(remaining) = busy_timeout()
        .checked_sub(spent)
        .filter(|remaining| !remaining.is_zero())
    else {
        return false;
    };
    #[cfg(test)]
    BUSY_WAIT_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(usize::try_from(checks).unwrap_or_default());
        }
    });
    let index = usize::try_from(checks)
        .unwrap_or_default()
        .min(BUSY_POLL_DELAYS_MS.len() - 1);
    let delay = Duration::from_millis(BUSY_POLL_DELAYS_MS[index]).min(remaining);
    let started = Instant::now();
    std::thread::sleep(delay);
    WAIT_SPENT.set(spent.saturating_add(delay));
    ATTEMPT_WAITED.set(ATTEMPT_WAITED.get().saturating_add(started.elapsed()));
    true
}

fn busy_timeout() -> Duration {
    #[cfg(test)]
    if let Some(timeout) = BUSY_TIMEOUT_OVERRIDE.get() {
        return timeout;
    }
    BUSY_TIMEOUT
}

/// How long the writes under this thread's `OperationDeadline` have waited so far.
#[cfg(test)]
pub(crate) fn operation_waited() -> Option<Duration> {
    OPERATION_WAITED.get()
}

/// Counts `spent` as already waited by the writes under this thread's `OperationDeadline`, so a
/// test gives a write only part of the deadline without waiting for the rest.
#[cfg(test)]
pub(crate) fn spend_operation_wait(spent: Duration) {
    OPERATION_WAITED.set(Some(OPERATION_WAITED.get().unwrap_or_default() + spent));
}

/// `backoff` jittered to between half and all of it, so writers of several processes that found the
/// same writer holding the store do not retry in step.
fn jittered(backoff: Duration) -> Duration {
    let half = backoff / 2;
    let span = u64::try_from((backoff - half).as_nanos())
        .unwrap_or(u64::MAX)
        .max(1);
    // A fresh `RandomState` is randomly keyed per thread and differs per call.
    half + Duration::from_nanos(RandomState::new().hash_one(Instant::now()) % span)
}

type SharedConnection = Arc<ParkingMutex<Option<Connection>>>;

thread_local! {
    static THREAD_CONNECTIONS: RefCell<HashMap<PathBuf, SharedConnection>> =
        RefCell::new(HashMap::new());
}

/// Every thread's store connections by database, so an idle store closes them from whichever
/// thread releases it (Rayon workers and libuv threads both read the store).
type ConnectionRegistry = Vec<(PathBuf, std::sync::Weak<ParkingMutex<Option<Connection>>>)>;
static ALL_CONNECTIONS: std::sync::LazyLock<ParkingMutex<ConnectionRegistry>> =
    std::sync::LazyLock::new(|| ParkingMutex::new(Vec::new()));

pub(super) struct ThreadConnection {
    pub(super) database_path: PathBuf,
    /// The line every write transaction of this process on the file takes its turn in.
    pub(super) line: Arc<WriterLine>,
}

/// A locked thread connection, reopened if an idle release closed it.
pub(super) struct ConnectionGuard<'a> {
    connection: ArcMutexGuard<RawMutex, Option<Connection>>,
    line: &'a WriterLine,
}

impl std::ops::Deref for ConnectionGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("connection opened under the lock")
    }
}

// The guard reads through the connection: the store writes only through
// `transaction_with_behavior` and `execute_batch` below, each in a turn of the writer line. The
// connection it derefs to could still run a write outside a turn, so in debug builds every commit
// on a store connection checks that its thread holds a turn (`commits_only_in_a_turn`).
impl ConnectionGuard<'_> {
    /// Begins a transaction on the store, shadowing `Connection::transaction_with_behavior`. A
    /// write transaction (immediate or exclusive) first takes the write's turn in the store's
    /// writer line (`take_turn`) and holds it until it ends, so SQLite's write lock is then held,
    /// if at all, by a writer of another process. A deferred one only reads, and takes no turn.
    pub(super) fn transaction_with_behavior(
        &mut self,
        behavior: TransactionBehavior,
    ) -> rusqlite::Result<StoreTransaction<'_>> {
        let turn = match behavior {
            TransactionBehavior::Deferred => None,
            _ => Some(take_turn(self.line)?),
        };
        // `&mut self` keeps a second transaction off this connection while this one lives.
        let connection: &Connection = self;
        Ok(StoreTransaction {
            transaction: Transaction::new_unchecked(connection, behavior)?,
            connection,
            turn,
        })
    }

    /// The connection itself, for a test that instruments it (a progress handler); no store code
    /// reaches it this way.
    #[cfg(test)]
    pub(super) fn instrument(&mut self) -> &mut Connection {
        self.connection
            .as_mut()
            .expect("connection opened under the lock")
    }

    /// Runs `sql` outside a transaction in a turn of the writer line, shadowing
    /// `Connection::execute_batch`: such a batch may write (collection's checkpoint and vacuum).
    /// A checkpoint its writes leave due runs after the turn is handed on.
    pub(super) fn execute_batch(&self, sql: &str) -> rusqlite::Result<()> {
        let connection: &Connection = self;
        CHECKPOINT_DUE.set(false);
        let turn = take_turn(self.line)?;
        let ran = connection.execute_batch(sql);
        drop(turn);
        checkpoint_if_due(connection);
        ran
    }
}

/// A store transaction, holding its write's turn in the writer line until it commits or rolls
/// back: the next writer is handed the line at that moment.
pub(super) struct StoreTransaction<'a> {
    transaction: Transaction<'a>,
    connection: &'a Connection,
    turn: Option<Turn<'a>>,
}

impl<'a> std::ops::Deref for StoreTransaction<'a> {
    type Target = Transaction<'a>;
    fn deref(&self) -> &Transaction<'a> {
        &self.transaction
    }
}

impl StoreTransaction<'_> {
    /// Commits, hands the line to the next writer, then runs the checkpoint the commit left due
    /// (`checkpoint_after_turn`): SQLite releases its write lock before its automatic checkpoint,
    /// so the next writer is handed the line at that same point, not after the checkpoint.
    pub(super) fn commit(self) -> rusqlite::Result<()> {
        let Self {
            transaction,
            connection,
            turn,
        } = self;
        CHECKPOINT_DUE.set(false);
        transaction.commit()?;
        drop(turn);
        checkpoint_if_due(connection);
        Ok(())
    }
}

/// Puts SQLite's automatic checkpoint of `connection` after the write's turn. SQLite runs it inside
/// the commit, once the write-ahead log holds `wal_autocheckpoint` pages (1,000 by default), after
/// it has released its write lock, so a writer of another connection can write during it; the
/// commit returns, and the turn is handed on, only after it. So the store replaces it with SQLite's
/// own notice of the log's size at each commit (`sqlite3_wal_hook`, which turns the automatic
/// checkpoint off), which marks the checkpoint due at that same size, and the committing thread runs
/// it once its turn is handed on (`checkpoint_if_due`). Like SQLite's, it is passive and runs at
/// every commit that leaves the log at that size or more: a reader that holds the log back lets it
/// grow until that reader ends, as before, and `journal_size_limit` shortens the file once a
/// checkpoint has copied the whole log and the next writer starts it over.
fn checkpoint_after_turn(connection: &mut Connection) -> StoreResult<()> {
    let due_at: c_int =
        connection.pragma_query_value(None, "wal_autocheckpoint", |row| row.get(0))?;
    connection.wal_hook(Some(move |_: &mut Wal, pages: c_int| {
        if due_at > 0 && pages >= due_at {
            CHECKPOINT_DUE.set(true);
        }
        Ok(())
    }))?;
    Ok(())
}

/// Runs the passive checkpoint this thread's last commit left due (`checkpoint_after_turn`).
fn checkpoint_if_due(connection: &Connection) {
    if CHECKPOINT_DUE.replace(false) {
        #[cfg(test)]
        run_checkpoint_hook();
        // As SQLite's own automatic checkpoint: it copies what no reader still needs, never waits,
        // and its result is not the commit's.
        let _ = connection.wal_checkpoint_v2(Some(MAIN_DB), CheckpointMode::PASSIVE);
    }
}

#[cfg(test)]
thread_local! {
    /// Runs on this thread right before it runs a checkpoint its commit left due: a test lets
    /// the next writer write there, to show it was handed the line before the checkpoint.
    pub(crate) static CHECKPOINT_HOOK: RefCell<Option<Box<dyn FnMut()>>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
fn run_checkpoint_hook() {
    CHECKPOINT_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook();
        }
    });
}

/// Takes `line`'s turn for one write: at once when the line is free, else in arrival order at the
/// release of the writers ahead, for at most what is left of the write deadline (`busy_deadline`,
/// less what the thread's `OperationDeadline` already waited) and until the work's cancel holds
/// (`waits_end_when`); only when the line is free under `WritesOnlyWhenFree` and `NoBusyWait`. The
/// wait counts as time waited on another writer (`ATTEMPT_WAITED`). A turn not given in its bound
/// comes back as SQLite's own busy error, so every caller treats it as a busy store; a cancelled
/// wait as SQLite's interrupt, which no caller retries.
pub(super) fn take_turn(line: &WriterLine) -> rusqlite::Result<Turn<'_>> {
    if WAITS.get() != Waits::InLine {
        return line.try_take().ok_or_else(|| refused(&Refusal::Bound));
    }
    let deadline = busy_deadline();
    let wait = Wait {
        bound: deadline.saturating_sub(OPERATION_WAITED.get().unwrap_or_default()),
        stalls_after: deadline,
        cancelled: &waits_ended,
    };
    let (taken, waited) = line.take(&wait);
    ATTEMPT_WAITED.set(
        ATTEMPT_WAITED
            .get()
            .saturating_add(waited.unwrap_or_default()),
    );
    taken.map_err(|refusal| refused(&refusal))
}

/// The SQLite error a turn not given comes back as.
fn refused(refusal: &Refusal) -> rusqlite::Error {
    let (code, message) = match refusal {
        Refusal::Bound => (
            rusqlite::ffi::SQLITE_BUSY,
            "another write of this process held the content store past the write deadline",
        ),
        Refusal::Cancelled => (
            rusqlite::ffi::SQLITE_INTERRUPT,
            "the content store write was cancelled while it waited for its turn",
        ),
    };
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), Some(message.to_owned()))
}

impl ThreadConnection {
    pub(super) fn lock(&self) -> Result<ConnectionGuard<'_>, String> {
        let connection = THREAD_CONNECTIONS.with(|connections| {
            let mut connections = connections.borrow_mut();
            Arc::clone(
                connections
                    .entry(self.database_path.clone())
                    .or_insert_with(|| {
                        let connection = SharedConnection::default();
                        ALL_CONNECTIONS
                            .lock()
                            .push((self.database_path.clone(), Arc::downgrade(&connection)));
                        connection
                    }),
            )
        });
        let mut guard = connection.lock_arc();
        if guard.is_none() {
            *guard = Some(
                open_runtime_connection(&self.database_path).map_err(|error| error.to_string())?,
            );
        }
        Ok(ConnectionGuard {
            connection: guard,
            line: &self.line,
        })
    }

    /// Closes every thread's idle connection to this database; one in use stays open.
    pub(super) fn release_all(&self) {
        ALL_CONNECTIONS.lock().retain(|(path, connection)| {
            let Some(connection) = connection.upgrade() else {
                return false;
            };
            if *path == self.database_path
                && let Some(mut connection) = connection.try_lock()
            {
                connection.take();
            }
            true
        });
    }
}

pub(super) fn configure_connection(connection: &mut Connection) -> StoreResult<()> {
    connection.set_prepared_statement_cache_capacity(128);
    connection.busy_handler(Some(wait_for_writer))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    // Readers never block the writer, nor it them: every worker thread reads while one apply
    // publishes.
    let _: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    // A store may be opened by each of Rerail's worker threads. Keep their aggregate page-cache
    // residency bounded; active reads still use the decompressed cache above.
    connection.pragma_update(None, "cache_size", -1_024_i64)?;
    connection.pragma_update(None, "mmap_size", 0_i64)?;
    connection.pragma_update(None, "journal_size_limit", 16_777_216_i64)?;
    checkpoint_after_turn(connection)?;
    #[cfg(debug_assertions)]
    commits_only_in_a_turn(connection)?;
    Ok(())
}

/// Rolls back, in debug builds (every test), a write that commits on a store connection while its
/// thread holds no turn of a writer line: the store writes only in a turn, and the connection the
/// guard derefs to would let a new write skip it unseen.
#[cfg(debug_assertions)]
fn commits_only_in_a_turn(connection: &mut Connection) -> StoreResult<()> {
    connection.commit_hook(Some(|| {
        let outside = !super::writer_line::holds_a_turn();
        if outside {
            eprintln!(
                "rerail: a content store write committed outside its turn in the writer line"
            );
        }
        outside
    }))?;
    Ok(())
}

fn open_runtime_connection(path: &Path) -> StoreResult<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_FULL_MUTEX;
    let mut connection = Connection::open_with_flags(path, flags)?;
    configure_connection(&mut connection)?;
    Ok(connection)
}

/// Makes this thread's waits for another writer, in the writer line and inside SQLite, give up at
/// once while it lives.
struct NoBusyWait(Waits);

impl NoBusyWait {
    fn enter() -> Self {
        Self(WAITS.replace(Waits::Never))
    }
}

impl Drop for NoBusyWait {
    fn drop(&mut self) {
        WAITS.set(self.0);
    }
}

/// Makes this thread's writes take the store, while it lives, only when no writer of this process
/// holds it or waits for it: a background write that runs again later (a search-index batch) never
/// joins the line ahead of a write a person waits for, and gives up as `Busy` instead. A writer of
/// another process it still waits for one SQLite busy timeout, as such a write did before the line.
pub(crate) struct WritesOnlyWhenFree(Waits);

impl WritesOnlyWhenFree {
    pub(crate) fn enter() -> Self {
        Self(WAITS.replace(Waits::OnlyWhenFree))
    }
}

impl Drop for WritesOnlyWhenFree {
    fn drop(&mut self) {
        WAITS.set(self.0);
    }
}

impl ContentStore {
    /// Runs `operation` in one immediate (write-locking) transaction, retried under `retry_busy`.
    pub(super) fn immediate_transaction<T>(
        &self,
        mut operation: impl FnMut(&Transaction<'_>) -> StoreResult<T>,
    ) -> StoreResult<T> {
        retry_busy(&mut BusyRetries::default(), || {
            self.immediate_transaction_once(&mut operation)
        })
    }

    /// Runs `operation` in one immediate transaction, attempted once: `Busy` once the writers ahead
    /// in the line held it past the write deadline, or a writer of another process held the store
    /// for one SQLite busy timeout. For background writes that run again later.
    pub(super) fn immediate_transaction_once<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> StoreResult<T>,
    ) -> StoreResult<T> {
        let mut connection = self
            .connection
            .lock()
            .expect("content store connection unavailable");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = operation(&transaction)?;
        transaction.commit()?;
        Ok(value)
    }

    /// Runs `operation` in one immediate transaction without waiting: `Busy` if another writer
    /// holds the store or waits in its line now. For the write a caller that must never block
    /// (closing a workspace on the host's thread) tries once.
    pub(super) fn immediate_transaction_no_wait<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> StoreResult<T>,
    ) -> StoreResult<T> {
        // Opening the thread's connection (if an idle release closed it) is not a wait for
        // another writer, and must not fail as one.
        let mut connection = self.connection.lock().map_err(StoreError::Invalid)?;
        let _no_wait = NoBusyWait::enter();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = operation(&transaction)?;
        transaction.commit()?;
        Ok(value)
    }

    /// Whether a write of this process has held the store past another write's whole wait for
    /// it, and holds it still (`WriterLine::stalled`).
    pub(crate) fn writes_stalled(&self) -> bool {
        self.connection.line.stalled()
    }

    /// Returns once every write whose `LINE_WAIT_HOOK` ran waits for its turn (`WriterLine::sync`).
    #[cfg(test)]
    pub(crate) fn sync_writer_line(&self) {
        self.connection.line.sync();
    }

    /// Returns once `count` writes wait in this store's writer line.
    #[cfg(test)]
    pub(crate) fn wait_for_line_waiters(&self, count: usize) {
        self.connection.line.wait_for_waiters(count);
    }
}
