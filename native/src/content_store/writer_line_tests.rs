//! The writer line through the store's own writes. A write is handed the store at the release of
//! the writer ahead, in arrival order; behind a writer that commits batch after batch it waits for
//! the batch in progress, not the run; a waiter that gives up at its bound marks the holder stalled
//! until that holder lets go. No test waits on a clock: holders let go by channel, a waiter says it
//! waits through `LINE_WAIT_HOOK`, and a bound passes at once under a deadline of a nanosecond.

use super::test_support::TemporaryDatabase;
use super::{
    BUSY_DEADLINE_OVERRIDE, ContentStore, LINE_WAIT_HOOK, OperationDeadline, StoreError,
    StoreResult, operation_found_busy,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn store(label: &str) -> (TemporaryDatabase, Arc<ContentStore>) {
    let path = TemporaryDatabase::new(&format!("rerail-writer-line-{label}"));
    let store = Arc::new(ContentStore::open(&path).unwrap());
    (path, store)
}

/// One write transaction that inserts `rows` rows: a batch of a background writer.
fn batch(store: &ContentStore, rows: i64) -> StoreResult<()> {
    store.immediate_transaction(|transaction| {
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS filler(value INTEGER)")?;
        transaction.execute(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < ?1) \
             INSERT INTO filler SELECT x FROM n",
            [rows],
        )?;
        Ok(())
    })
}

/// A writer on its own thread that holds the store from inside its write transaction until
/// `release` is called. Returns once it holds it.
struct Holder {
    release: mpsc::Sender<()>,
    thread: thread::JoinHandle<StoreResult<()>>,
}

impl Holder {
    fn hold(store: &Arc<ContentStore>, order: &Arc<Mutex<Vec<&'static str>>>) -> Self {
        let (holding, held) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let (store, order) = (Arc::clone(store), Arc::clone(order));
        let thread = thread::spawn(move || {
            store.immediate_transaction(|_| {
                holding.send(()).unwrap();
                released.recv().unwrap();
                order.lock().unwrap().push("holder");
                Ok(())
            })
        });
        held.recv().unwrap();
        Self { release, thread }
    }

    fn let_go(self) {
        self.release.send(()).unwrap();
        self.thread.join().unwrap().unwrap();
    }
}

/// Starts a write named `name` on its own thread and returns once it waits in the store's line.
fn waiting_writer(
    store: &Arc<ContentStore>,
    order: &Arc<Mutex<Vec<&'static str>>>,
    name: &'static str,
) -> thread::JoinHandle<StoreResult<()>> {
    let (waiting, waits) = mpsc::channel();
    let (store, order) = (Arc::clone(store), Arc::clone(order));
    let handle = thread::spawn(move || {
        LINE_WAIT_HOOK.set(Some(Box::new(move || waiting.send(()).unwrap())));
        let written = store.immediate_transaction(|_| {
            order.lock().unwrap().push(name);
            Ok(())
        });
        LINE_WAIT_HOOK.set(None);
        written
    });
    waits.recv().unwrap();
    handle
}

/// Behind a writer that commits batch after batch, as a Git snapshot publication or a search-index
/// build does, a write waits for the batch in progress. SQLite's lock alone gave every gap to the
/// batch writer, so the write waited for the whole run (here 400 batches).
#[test]
fn a_write_during_a_run_of_batches_waits_for_the_batch_in_progress_not_the_run() {
    let (_path, store) = store("batch-run");
    let committed = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let limit = Arc::new(AtomicUsize::new(5_000));
    let (started, starts) = mpsc::channel();
    let writer = {
        let (store, committed) = (Arc::clone(&store), Arc::clone(&committed));
        let (done, limit) = (Arc::clone(&done), Arc::clone(&limit));
        thread::spawn(move || {
            while !done.load(Ordering::Acquire)
                && committed.load(Ordering::Acquire) < limit.load(Ordering::Acquire)
            {
                batch(&store, 2_000).unwrap();
                if committed.fetch_add(1, Ordering::AcqRel) == 0 {
                    started.send(()).unwrap();
                }
            }
        })
    };
    starts.recv().unwrap();
    let before = committed.load(Ordering::Acquire);
    limit.store(before + 400, Ordering::Release);
    batch(&store, 1).unwrap();
    let after = committed.load(Ordering::Acquire);
    done.store(true, Ordering::Release);
    writer.join().unwrap();
    assert!(
        after - before < 20,
        "the write waited for {} of the writer's batches, not the one in progress",
        after - before
    );
}

/// The writer holding the store hands it, at its commit, to the writers waiting for it in the
/// order they arrived.
#[test]
fn the_writer_ahead_hands_the_store_to_the_waiting_writers_in_arrival_order() {
    let (_path, store) = store("arrival-order");
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    let first = waiting_writer(&store, &order, "first");
    let second = waiting_writer(&store, &order, "second");
    holder.let_go();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    assert_eq!(*order.lock().unwrap(), ["holder", "first", "second"]);
}

/// Two stores of one process on one file, however its path is spelled, share one line: a write
/// on one waits in the line for a write on the other, not in SQLite's busy handler.
#[test]
fn two_stores_on_one_file_share_its_line() {
    let (path, first) = store("shared-line");
    let spelled = path.parent().unwrap().join(".").join(path.file_name().unwrap());
    let second = Arc::new(ContentStore::open(&spelled).unwrap());
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&first, &order);
    let waiter = waiting_writer(&second, &order, "second store");
    holder.let_go();
    waiter.join().unwrap().unwrap();
    assert_eq!(*order.lock().unwrap(), ["holder", "second store"]);
}

/// A waiter that gives up at its bound marks the holder stalled, so an apply can leave the store
/// alone instead of waiting out the bound again; the stall ends when that holder lets go, and the
/// next write is handed the store.
#[test]
fn a_waiter_that_gives_up_marks_the_holder_stalled_until_it_lets_go() {
    let (_path, store) = store("stalled");
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    assert!(!store.writes_stalled(), "a holder is not stalled by itself");
    BUSY_DEADLINE_OVERRIDE.set(Some(Duration::from_nanos(1)));
    let given_up = batch(&store, 1);
    BUSY_DEADLINE_OVERRIDE.set(None);
    assert!(matches!(given_up, Err(StoreError::Busy)), "{given_up:?}");
    assert!(store.writes_stalled());
    holder.let_go();
    assert!(!store.writes_stalled(), "the stall ends when its holder lets go");
    batch(&store, 1).unwrap();
}

/// A write with no time left in its operation's deadline gives up at once, and says nothing about
/// the holder: the holder is not marked stalled.
#[test]
fn a_write_with_no_time_left_does_not_mark_the_holder_stalled() {
    let (_path, store) = store("no-time-left");
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    BUSY_DEADLINE_OVERRIDE.set(Some(Duration::ZERO));
    let given_up = batch(&store, 1);
    BUSY_DEADLINE_OVERRIDE.set(None);
    assert!(matches!(given_up, Err(StoreError::Busy)), "{given_up:?}");
    assert!(!store.writes_stalled());
    holder.let_go();
}

/// A write that must not wait (a workspace closing on the host's thread) gives up at once while
/// another write holds the store, and is written once the store is free.
#[test]
fn a_write_that_must_not_wait_gives_up_while_another_holds_the_store() {
    let (_path, store) = store("no-wait");
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    let refused = store.immediate_transaction_no_wait(|_| Ok(()));
    assert!(matches!(refused, Err(StoreError::Busy)), "{refused:?}");
    holder.let_go();
    store.immediate_transaction_no_wait(|_| Ok(())).unwrap();
}

/// An operation remembers that one of its writes ended busy, whatever the store's state after it,
/// and a new operation starts without it.
#[test]
fn an_operation_remembers_a_write_that_ended_busy() {
    let (_path, store) = store("operation-busy");
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    {
        let _operation = OperationDeadline::enter();
        BUSY_DEADLINE_OVERRIDE.set(Some(Duration::from_nanos(1)));
        assert!(matches!(batch(&store, 1), Err(StoreError::Busy)));
        BUSY_DEADLINE_OVERRIDE.set(None);
        holder.let_go();
        assert!(operation_found_busy(), "the operation's write ended busy");
        batch(&store, 1).unwrap();
        assert!(operation_found_busy(), "a later write does not undo it");
    }
    let _next = OperationDeadline::enter();
    assert!(!operation_found_busy());
}
