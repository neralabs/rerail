//! The store's writer line. SQLite lets one connection write a store at a time, and a connection
//! that finds the write lock held can only look again later (`connection::wait_for_writer`): a
//! writer that commits batch after batch takes the lock back before the next look, so a write that
//! arrives during the batches waits for all of them. So every write this process makes to a store
//! file first takes a turn in that file's line, in arrival order, and is handed the line at the
//! release of the writer ahead of it, with no look and no pause. SQLite's own lock is then held,
//! if at all, by a writer of another process.
//!
//! A wait in the line ends at the handoff, at the cancel its waiter names (`Wait::cancelled`: the
//! waiter's owner sets it and wakes the lines, `wake_all`), or at one bound, what is left of the
//! write's deadline (`connection::take_turn`): a holder that never lets go (a disk that stalls)
//! costs a waiter that bound and no more. A waiter that waited the whole deadline itself marks the
//! holder stalled until that holder lets go (`stalled`), so an apply, whose results can wait for a
//! later one, leaves the store alone meanwhile instead of waiting out the bound again, and
//! publishes at the first apply after.

use parking_lot::{Condvar, Mutex, MutexGuard};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

#[cfg(test)]
type WaitHook = Box<dyn FnMut()>;
#[cfg(test)]
thread_local! {
    /// Runs on this thread when one of its writes has taken its place in a line behind another
    /// writer, before it waits: a test starts the next writer, or lets the holder go, once this
    /// one is known to wait.
    pub(crate) static LINE_WAIT_HOOK: std::cell::RefCell<Option<WaitHook>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Default)]
struct Line {
    /// The ticket the next writer to arrive takes.
    next: u64,
    /// The ticket whose turn it is: held by its writer, or handed to it and not taken yet.
    serving: u64,
    held: bool,
    /// The tickets of writers that stopped waiting, passed over when their turn comes.
    left: BTreeSet<u64>,
    /// The ticket of a holder a waiter gave up behind, until that holder lets go.
    stalled: Option<u64>,
    /// The thread holding the line: one that asks for the line it holds would wait for itself.
    #[cfg(debug_assertions)]
    holder: Option<std::thread::ThreadId>,
}

impl Line {
    fn arrive(&mut self) -> u64 {
        let ticket = self.next;
        self.next += 1;
        ticket
    }

    fn is_turn(&self, ticket: u64) -> bool {
        !self.held && self.serving == ticket
    }

    /// Passes the turn on from `serving`, over the writers that left.
    fn advance(&mut self) {
        self.serving += 1;
        while self.left.remove(&self.serving) {
            self.serving += 1;
        }
    }

    /// The writer of `ticket` stops waiting: a turn handed to it passes on, and a later one is
    /// passed over when it comes. Whether the turn moved.
    fn leave(&mut self, ticket: u64) -> bool {
        if self.serving == ticket {
            self.advance();
            return true;
        }
        self.left.insert(ticket);
        false
    }

    /// The holder of `ticket` lets go: its stall, if any, ends, and the turn passes on.
    fn release(&mut self, ticket: u64) {
        self.held = false;
        #[cfg(debug_assertions)]
        {
            self.holder = None;
        }
        if self.stalled == Some(ticket) {
            self.stalled = None;
        }
        self.advance();
    }

    /// A thread that asks for the line it holds would wait for itself.
    #[cfg(debug_assertions)]
    fn assert_not_held_here(&self) {
        assert!(
            !self.held || self.holder != Some(std::thread::current().id()),
            "a thread asked for the store's writer line it holds"
        );
    }

    /// The writers waiting for their turn: arrived, not passed over, and not holding.
    #[cfg(test)]
    fn waiting(&self) -> usize {
        let arrived = usize::try_from(self.next - self.serving).unwrap_or(usize::MAX);
        arrived - usize::from(self.held) - self.left.len()
    }
}

/// How a writer waits for its turn.
pub(super) struct Wait<'a> {
    /// The longest it waits.
    pub(super) bound: Duration,
    /// How long a wait that ends at its bound must have lasted to mark the holder stalled: the
    /// whole write deadline, so a writer whose operation already spent most of it marks nothing.
    pub(super) stalls_after: Duration,
    /// The fact its owner sets to end the wait, waking the lines (`wake_all`).
    pub(super) cancelled: &'a dyn Fn() -> bool,
}

/// Why a writer was not given its turn.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Refusal {
    /// It waited its whole bound.
    Bound,
    /// Its owner cancelled the wait.
    Cancelled,
}

/// One store file's line of writers in this process.
pub(super) struct WriterLine {
    line: Mutex<Line>,
    turn: Condvar,
}

static LINES: LazyLock<Mutex<HashMap<PathBuf, Weak<WriterLine>>>> = LazyLock::new(Mutex::default);

impl WriterLine {
    /// The line of the store file at `path`, shared by every `ContentStore` of this process on
    /// that file, however its folder is spelled.
    pub(super) fn of(path: &Path) -> Arc<Self> {
        let key = line_key(path);
        let mut lines = LINES.lock();
        if let Some(line) = lines.get(&key).and_then(Weak::upgrade) {
            return line;
        }
        lines.retain(|_, line| line.strong_count() > 0);
        let line = Arc::new(Self {
            line: Mutex::default(),
            turn: Condvar::new(),
        });
        lines.insert(key, Arc::downgrade(&line));
        line
    }

    /// Takes the next turn: at once when nobody holds the line or waits in it, else at the
    /// release of the writers ahead, in arrival order, until `wait` ends it. Returns the turn or
    /// why it was refused, and how long it waited when it found the line held.
    pub(super) fn take(&self, wait: &Wait<'_>) -> (Result<Turn<'_>, Refusal>, Option<Duration>) {
        let mut line = self.line.lock();
        #[cfg(debug_assertions)]
        line.assert_not_held_here();
        let ticket = line.arrive();
        if line.is_turn(ticket) {
            return (Ok(self.hold(&mut line, ticket)), None);
        }
        #[cfg(test)]
        {
            self.turn.notify_all();
            MutexGuard::unlocked(&mut line, run_wait_hook);
        }
        let started = Instant::now();
        let ended = self.wait_for_turn(&mut line, ticket, started + wait.bound, wait.cancelled);
        let waited = started.elapsed();
        let taken = match ended {
            None => Ok(self.hold(&mut line, ticket)),
            Some(refusal) => {
                let stalls = refusal == Refusal::Bound && waited >= wait.stalls_after;
                self.give_up(&mut line, ticket, stalls);
                Err(refusal)
            }
        };
        (taken, Some(waited))
    }

    /// Waits until `ticket`'s turn comes (`None`), `cancelled` holds, or `deadline` passes.
    fn wait_for_turn(
        &self,
        line: &mut MutexGuard<'_, Line>,
        ticket: u64,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Refusal> {
        while !line.is_turn(ticket) {
            if cancelled() {
                return Some(Refusal::Cancelled);
            }
            if self.turn.wait_until(line, deadline).timed_out() && !line.is_turn(ticket) {
                return Some(Refusal::Bound);
            }
        }
        None
    }

    /// The writer of `ticket` leaves the line; `stalls` marks the holder it gave up behind.
    fn give_up(&self, line: &mut Line, ticket: u64, stalls: bool) {
        if stalls && line.held {
            line.stalled = Some(line.serving);
        }
        if line.leave(ticket) {
            self.turn.notify_all();
        }
    }

    fn hold(&self, line: &mut Line, ticket: u64) -> Turn<'_> {
        line.held = true;
        #[cfg(debug_assertions)]
        {
            line.holder = Some(std::thread::current().id());
        }
        Turn { line: self, ticket }
    }

    /// The turn, only when nobody holds the line or waits in it: for a write that must not wait.
    pub(super) fn try_take(&self) -> Option<Turn<'_>> {
        let mut line = self.line.lock();
        if line.held || line.next != line.serving {
            return None;
        }
        let ticket = line.arrive();
        Some(self.hold(&mut line, ticket))
    }

    /// Whether a holder outlasted a waiter's whole deadline and holds the line still.
    pub(super) fn stalled(&self) -> bool {
        self.line.lock().stalled.is_some()
    }

    /// Returns once `count` writers wait in the line, so a test acts while they wait.
    #[cfg(test)]
    pub(super) fn wait_for_waiters(&self, count: usize) {
        let mut line = self.line.lock();
        let deadline = Instant::now() + Duration::from_secs(60);
        while line.waiting() < count {
            assert!(
                !self.turn.wait_until(&mut line, deadline).timed_out(),
                "{count} writers never waited in the line"
            );
        }
    }
}

/// Wakes every writer waiting in this process's lines, so each looks at its cancel again: called
/// by an owner that has just set one.
pub(super) fn wake_all() {
    let lines = LINES
        .lock()
        .values()
        .filter_map(Weak::upgrade)
        .collect::<Vec<_>>();
    for line in lines {
        // Taken, so a waiter between its look at the cancel and its wait does not miss the wake.
        drop(line.line.lock());
        line.turn.notify_all();
    }
}

/// The file at `path` by its resolved folder and its name. The folder exists once a store opens;
/// the file may not (on Windows SQLite creates it at the first open), so resolving the whole path
/// would key the first open and the later ones apart.
fn line_key(path: &Path) -> PathBuf {
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    match (std::fs::canonicalize(folder), path.file_name()) {
        (Ok(folder), Some(name)) => folder.join(name),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
fn run_wait_hook() {
    LINE_WAIT_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook();
        }
    });
}

/// One write's turn in its store's line: dropping it hands the line to the next writer.
pub(super) struct Turn<'a> {
    line: &'a WriterLine,
    ticket: u64,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.line.line.lock().release(self.ticket);
        self.line.turn.notify_all();
    }
}

#[cfg(test)]
mod line_tests {
    //! The line's order and stall marks, with no threads.

    use super::Line;

    /// A holder and a line that has served it.
    fn held() -> (Line, u64) {
        let mut line = Line::default();
        let holder = line.arrive();
        assert!(line.is_turn(holder));
        line.held = true;
        (line, holder)
    }

    /// Two tickets behind a holder: at its release the first one's turn has come, the second
    /// one's has not; at the first one's release the second one's comes.
    #[test]
    fn a_release_hands_the_turn_to_the_first_ticket_behind_the_holder_only() {
        let (mut line, holder) = held();
        let first = line.arrive();
        let second = line.arrive();
        assert!(!line.is_turn(first) && !line.is_turn(second));
        assert_eq!(line.waiting(), 2);
        line.release(holder);
        assert!(line.is_turn(first), "the first ticket's turn has come");
        assert!(!line.is_turn(second), "the second ticket's turn has not");
        line.held = true;
        line.release(first);
        assert!(line.is_turn(second));
    }

    /// A ticket that left is passed over when its turn comes.
    #[test]
    fn a_ticket_that_left_is_passed_over() {
        let (mut line, holder) = held();
        let left = line.arrive();
        let next = line.arrive();
        assert!(
            !line.leave(left),
            "a ticket behind the holder leaves without moving the turn"
        );
        assert_eq!(line.waiting(), 1);
        line.release(holder);
        assert!(line.is_turn(next));
    }

    /// A stall mark ends with the release of the holder it names, not of another.
    #[test]
    fn a_stall_ends_with_the_holder_it_names() {
        let (mut line, holder) = held();
        line.stalled = Some(holder);
        let next = line.arrive();
        line.release(holder);
        assert_eq!(line.stalled, None);
        line.held = true;
        line.stalled = Some(holder);
        line.release(next);
        assert_eq!(
            line.stalled,
            Some(holder),
            "another holder's release leaves it"
        );
    }
}
