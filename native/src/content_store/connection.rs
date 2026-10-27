//! Per-thread SQLite connections, connection pragmas, and the busy retry every store write runs
//! under.

use super::{ContentStore, StoreError, StoreResult};
use parking_lot::{ArcMutexGuard, Mutex as ParkingMutex, RawMutex};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the store writes of one operation wait out other writers before they report `Busy`.
/// Each attempt waits at most one busy timeout (`wait_for_writer`), so a cancelled write notices
/// between attempts.
const BUSY_DEADLINE: Duration = Duration::from_secs(10);
/// The first and the largest backoff between a write's attempts.
const BUSY_BACKOFF_FIRST: Duration = Duration::from_millis(2);
const BUSY_BACKOFF_MAX: Duration = Duration::from_millis(200);
/// How long one attempt waits on another writer inside SQLite before it reports `Busy`.
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
    /// How long this thread has waited on the lock inside SQLite since its write attempt began.
    static ATTEMPT_WAITED: std::cell::Cell<Duration> =
        const { std::cell::Cell::new(Duration::ZERO) };
    /// Whether this thread's SQLite waits give up at once (`immediate_transaction_no_wait`).
    static NO_BUSY_WAIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// How long the writes under this thread's `OperationDeadline` have waited on other writers;
    /// `None` outside one, where each write has a deadline of its own.
    static OPERATION_WAITED: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
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
        }
        Self { owned }
    }
}

impl Drop for OperationDeadline {
    fn drop(&mut self) {
        if self.owned {
            OPERATION_WAITED.set(None);
        }
    }
}

fn busy_deadline() -> Duration {
    #[cfg(test)]
    if let Some(deadline) = BUSY_DEADLINE_OVERRIDE.get() {
        return deadline;
    }
    BUSY_DEADLINE
}

/// What other writers cost a write: its attempts that found the store busy, and the time those
/// attempts and the backoff after them took.
#[derive(Clone, Copy, Debug, Default)]
pub struct BusyRetries {
    pub retries: usize,
    pub wait_nanoseconds: u64,
}

/// Runs `attempt` until it returns anything but `Busy`, re-running it whole after a jittered
/// exponential backoff; `Busy` once `BUSY_DEADLINE` of waiting has passed, counting what the
/// thread's other writes under the same `OperationDeadline` already waited. Every store write
/// that takes the write lock runs under it, so one writer's long transaction delays the others
/// instead of failing them.
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
                let Some(remaining) = deadline.checked_sub(waited) else {
                    if shared {
                        OPERATION_WAITED.set(Some(waited));
                    }
                    return Err(StoreError::Busy);
                };
                let delay = jittered(backoff).min(remaining);
                waited = waited.saturating_add(delay);
                if shared {
                    OPERATION_WAITED.set(Some(waited));
                }
                retries.retries += 1;
                retries.wait_nanoseconds = retries.wait_nanoseconds.saturating_add(
                    u64::try_from(attempted.saturating_add(delay).as_nanos()).unwrap_or(u64::MAX),
                );
                std::thread::sleep(delay);
                backoff = backoff.saturating_mul(2).min(BUSY_BACKOFF_MAX);
            }
            result => {
                // An attempt that got the lock after waiting for it inside SQLite still spent
                // that time.
                if shared {
                    OPERATION_WAITED.set(Some(waited.saturating_add(ATTEMPT_WAITED.get())));
                }
                return result;
            }
        }
    }
}

/// SQLite's busy handler of every store connection: waits for the write lock the way SQLite's
/// own busy timeout does (`BUSY_TIMEOUT` in all, `BUSY_POLL_DELAYS_MS` between checks), and
/// records what it waited, which an attempt that then gets the lock would otherwise leave
/// uncounted. It runs on the thread that runs the statement.
fn wait_for_writer(checks: i32) -> bool {
    if NO_BUSY_WAIT.get() {
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

/// `backoff` jittered to between half and all of it, so writers that found the same writer
/// holding the store do not retry in step.
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
}

/// A locked thread connection, reopened if an idle release closed it.
pub(super) struct ConnectionGuard(ArcMutexGuard<RawMutex, Option<Connection>>);

impl std::ops::Deref for ConnectionGuard {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0.as_ref().expect("connection opened under the lock")
    }
}

impl std::ops::DerefMut for ConnectionGuard {
    fn deref_mut(&mut self) -> &mut Connection {
        self.0.as_mut().expect("connection opened under the lock")
    }
}

impl ThreadConnection {
    pub(super) fn lock(&self) -> Result<ConnectionGuard, String> {
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
        Ok(ConnectionGuard(guard))
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
    super::postings::install(connection)?;
    super::vocabulary::install(connection)?;
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

/// Makes this thread's SQLite waits for another writer give up at once while it lives.
struct NoBusyWait;

impl NoBusyWait {
    fn enter() -> Self {
        NO_BUSY_WAIT.set(true);
        Self
    }
}

impl Drop for NoBusyWait {
    fn drop(&mut self) {
        NO_BUSY_WAIT.set(false);
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

    /// Runs `operation` in one immediate transaction, attempted once: `Busy` once another writer
    /// held the store for one SQLite busy timeout. For background writes that run again later.
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
    /// holds the store now. For the write a caller that must never block (closing a workspace on
    /// the host's thread) tries once.
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
}
