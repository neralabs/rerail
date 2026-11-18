//! The writer line through the store's own writes. A write is handed the store at the release of
//! the writer ahead, in arrival order; behind a writer that commits batch after batch it waits for
//! the batch in progress, not the run; a wait ends at the cancel its owner sets; a waiter that
//! waited its whole deadline marks the holder stalled until that holder lets go. No test waits on
//! a clock: holders let go by channel, a waiter says it waits through `LINE_WAIT_HOOK`, and a
//! bound passes at once under a deadline of a nanosecond or with the operation's deadline spent.

use super::test_support::{TemporaryDatabase, TemporaryFolder};
use super::{
    BUSY_DEADLINE_OVERRIDE, BUSY_WAIT_HOOK, ContentStore, LINE_WAIT_HOOK, OperationDeadline,
    StoreError, StoreResult, WaitsEndOn, end_waits, operation_found_busy, spend_operation_wait,
};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A deadline no test waits out: a write gets only what its operation has not spent of it.
const HOUR: Duration = Duration::from_secs(3_600);

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
/// batch writer, so the write waited for the whole run (here up to 400 batches). The batches are
/// counted from the write's arrival (its place in the line, or its first look at SQLite's lock
/// without one) to its own transaction, so the time its thread takes to get there counts for
/// nothing.
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
    limit.store(committed.load(Ordering::Acquire) + 400, Ordering::Release);
    let arrived = Rc::new(Cell::new(None));
    let at_arrival = {
        let (arrived, committed) = (Rc::clone(&arrived), Arc::clone(&committed));
        move || {
            if arrived.get().is_none() {
                arrived.set(Some(committed.load(Ordering::Acquire)));
            }
        }
    };
    LINE_WAIT_HOOK.set(Some(Box::new(at_arrival.clone())));
    BUSY_WAIT_HOOK.set(Some(Box::new(move |_| at_arrival())));
    let at_turn = store.immediate_transaction(|_| Ok(committed.load(Ordering::Acquire)));
    LINE_WAIT_HOOK.set(None);
    BUSY_WAIT_HOOK.set(None);
    done.store(true, Ordering::Release);
    writer.join().unwrap();
    let at_turn = at_turn.unwrap();
    let waited_for = arrived.get().map_or(0, |arrived| at_turn - arrived);
    assert!(
        waited_for <= 2,
        "the write waited for {waited_for} of the writer's batches, not the one in progress"
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

/// A link at `link` to the folder `folder`: a symbolic link, or on Windows a junction, which needs
/// no privilege.
fn link_folder(folder: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(folder, link).unwrap();
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(folder)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
    }
}

/// Two stores of one process on one file, however its folder is reached, share one line: a write
/// on one waits in the line for a write on the other, not in SQLite's busy handler. The first
/// store opens the file through a link to its folder before the file exists (on Windows SQLite
/// creates it then), the second by the folder itself after, so on every system the line knows
/// the file only by its resolved folder.
#[test]
fn two_stores_on_one_file_share_its_line() {
    let path = TemporaryDatabase::new("rerail-writer-line-shared-line");
    let links = TemporaryFolder::new("rerail-writer-line-shared-line-link");
    let linked = links.join("store");
    link_folder(path.parent().unwrap(), &linked);
    let first = Arc::new(ContentStore::open(&linked.join(path.file_name().unwrap())).unwrap());
    let second = Arc::new(ContentStore::open(&path).unwrap());
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
    assert!(
        !store.writes_stalled(),
        "the stall ends when its holder lets go"
    );
    batch(&store, 1).unwrap();
}

/// A writer that panics while it holds the store hands the line on as it unwinds: the next write
/// is handed the store without waiting.
#[test]
fn a_writer_that_panics_while_holding_the_store_hands_it_on() {
    let (_path, store) = store("panicking-holder");
    let panicking = Arc::clone(&store);
    let ended = thread::spawn(move || {
        panicking.immediate_transaction(|_| -> StoreResult<()> { panic!("the holder fails") })
    })
    .join();
    assert!(ended.is_err(), "the holder panicked");
    LINE_WAIT_HOOK.set(Some(Box::new(|| {
        panic!("the next write waited for a holder that is gone")
    })));
    let written = batch(&store, 1);
    LINE_WAIT_HOOK.set(None);
    written.unwrap();
}

/// A write behind a holder after its operation spent `spent` of an hour's deadline: whether the
/// holder was then marked stalled.
fn stalls_after_spending(label: &str, spent: Duration) -> bool {
    let (_path, store) = store(label);
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    let operation = OperationDeadline::enter();
    spend_operation_wait(spent);
    BUSY_DEADLINE_OVERRIDE.set(Some(HOUR));
    let given_up = batch(&store, 1);
    BUSY_DEADLINE_OVERRIDE.set(None);
    drop(operation);
    assert!(matches!(given_up, Err(StoreError::Busy)), "{given_up:?}");
    let stalled = store.writes_stalled();
    holder.let_go();
    stalled
}

/// A write with no time left in its operation's deadline gives up at once, and says nothing about
/// the holder: the holder is not marked stalled.
#[test]
fn a_write_with_no_time_left_does_not_mark_the_holder_stalled() {
    assert!(!stalls_after_spending("no-time-left", HOUR));
}

/// A write whose operation already spent most of the deadline waits for the rest and gives up,
/// but it did not itself wait the whole deadline, so it marks nothing: only such a waiter shows a
/// holder stalled.
#[test]
fn a_write_with_part_of_the_deadline_left_does_not_mark_the_holder_stalled() {
    assert!(!stalls_after_spending(
        "part-left",
        HOUR - Duration::from_nanos(1)
    ));
}

/// A wait in the line ends when its owner cancels it (`end_waits`), with the holder still holding:
/// the write fails as interrupted, not as busy, and marks no stall. The cancel lands while the
/// writer is inside its wait, so it must wake the writer: a cancel that did not would leave the
/// wait to its 10 s deadline, which ends as busy and marks the holder stalled.
#[test]
fn a_wait_in_the_line_ends_when_its_owner_cancels_it() {
    let (_path, store) = store("cancelled");
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    let cancel = Arc::new(AtomicBool::new(false));
    let (waiting, waits) = mpsc::channel();
    let waiter = {
        let (store, cancel) = (Arc::clone(&store), Arc::clone(&cancel));
        thread::spawn(move || {
            let _ends = WaitsEndOn::enter(&cancel);
            LINE_WAIT_HOOK.set(Some(Box::new(move || waiting.send(()).unwrap())));
            let written = batch(&store, 1);
            LINE_WAIT_HOOK.set(None);
            written
        })
    };
    waits.recv().unwrap();
    // The waiter is inside its wait, so only the cancel's wake can end it before its deadline.
    store.sync_writer_line();
    end_waits(&cancel);
    let written = waiter.join().unwrap();
    assert!(
        matches!(&written, Err(StoreError::Database(error)) if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted)),
        "{written:?}"
    );
    assert!(!store.writes_stalled(), "a cancelled wait marks no stall");
    holder.let_go();
    assert_eq!(*order.lock().unwrap(), ["holder"]);
    batch(&store, 1).unwrap();
}

/// A write that must not wait (a workspace closing on the host's thread) gives up at once while
/// another write holds the store, and is written once the store is free.
#[test]
fn a_write_that_must_not_wait_gives_up_while_another_holds_the_store() {
    let (_path, store) = store("no-wait");
    let order = Arc::new(Mutex::new(Vec::new()));
    let holder = Holder::hold(&store, &order);
    LINE_WAIT_HOOK.set(Some(Box::new(|| {
        panic!("a write that must not wait took a place in the line")
    })));
    let refused = store.immediate_transaction_no_wait(|_| Ok(()));
    LINE_WAIT_HOOK.set(None);
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
