//! Per-thread SQLite connections, connection pragmas, and retrying immediate transactions.

use super::{ContentStore, StoreError, StoreResult};
use parking_lot::{ArcMutexGuard, Mutex as ParkingMutex, RawMutex};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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
    connection.busy_timeout(Duration::from_millis(250))?;
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

impl ContentStore {
    pub(super) fn immediate_transaction<T>(
        &self,
        mut operation: impl FnMut(&Transaction<'_>) -> StoreResult<T>,
    ) -> StoreResult<T> {
        let mut retries = 0_u32;
        loop {
            let result = {
                let mut connection = self
                    .connection
                    .lock()
                    .expect("content store connection unavailable");
                (|| {
                    let transaction =
                        connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let value = operation(&transaction)?;
                    transaction.commit()?;
                    Ok(value)
                })()
            };
            match result {
                Err(StoreError::Busy) if retries < 8 => {
                    retries += 1;
                    std::thread::sleep(Duration::from_millis(1_u64 << retries.min(6)));
                }
                result => return result,
            }
        }
    }
}
