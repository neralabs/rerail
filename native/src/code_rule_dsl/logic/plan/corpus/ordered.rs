//! A batch's documents built, and its chunks projected, on several threads (`threads::document_builders`); the
//! documents taken in order, so chunks are cut as one thread cuts them.
//!
//! ```text
//! helpers:  build 0 ─ build 2 ─ project chunk 0 ─ build 5 …   (a posted job first, else the next document)
//!           build 1 ─ build 3 ─ build 4 ─ project chunk 1 …
//! worker:   take 0, 1, 2 … cut chunk 0, post it; take … post chunk 1; hand on chunk 0 once projected, then 1 …
//! ```
//!
//! A helper claims a document only within `ahead` of the next one taken, so at most `ahead` built documents wait.
//! Helpers end when every document is claimed and the worker says no job will come (`finish`), or at `stop` (the
//! worker's error, or its channel closed). A helper's panic is handed to whoever takes that document or job's
//! result, which resumes it.

use std::collections::{BTreeMap, VecDeque};

use parking_lot::{Condvar, Mutex};

type Made<T> = std::thread::Result<T>;

pub(super) struct Ordered<R, J, O> {
    len: usize,
    ahead: usize,
    state: Mutex<State<R, J, O>>,
    /// A document built or a job done (for the worker).
    ready: Condvar,
    /// A document taken, a job posted, the end of jobs, or a stop (for a waiting helper).
    room: Condvar,
}

struct State<R, J, O> {
    /// The next document to claim, and the next to be taken.
    next: usize,
    taken: usize,
    built: BTreeMap<usize, Made<R>>,
    jobs: VecDeque<(usize, J)>,
    posted: usize,
    done: BTreeMap<usize, Made<O>>,
    finished: bool,
    stopped: bool,
}

impl<R: Send, J: Send, O: Send> Ordered<R, J, O> {
    pub(super) fn new(len: usize, ahead: usize) -> Self {
        Self {
            len,
            ahead: ahead.max(1),
            state: Mutex::new(State { next: 0, taken: 0, built: BTreeMap::new(), jobs: VecDeque::new(), posted: 0, done: BTreeMap::new(), finished: false, stopped: false }),
            ready: Condvar::new(),
            room: Condvar::new(),
        }
    }

    /// A helper's loop: a posted job first, else the next document while it is within `ahead` of the next taken.
    pub(super) fn help(&self, build: impl Fn(usize) -> R, run: impl Fn(J) -> O) {
        enum Work<J> {
            Doc(usize),
            Job(usize, J),
        }
        loop {
            let work = {
                let mut s = self.state.lock();
                loop {
                    if s.stopped {
                        return;
                    }
                    if let Some((id, job)) = s.jobs.pop_front() {
                        break Work::Job(id, job);
                    }
                    if s.next < self.len && s.next < s.taken + self.ahead {
                        s.next += 1;
                        break Work::Doc(s.next - 1);
                    }
                    if s.next >= self.len && s.finished {
                        return;
                    }
                    self.room.wait(&mut s);
                }
            };
            match work {
                Work::Doc(i) => {
                    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(i)));
                    self.state.lock().built.insert(i, made);
                }
                Work::Job(id, job) => {
                    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job)));
                    self.state.lock().done.insert(id, made);
                }
            }
            self.ready.notify_all();
        }
    }

    /// The `i`-th document, once built; documents are taken in order, from 0.
    pub(super) fn take(&self, i: usize) -> R {
        let made = {
            let mut s = self.state.lock();
            loop {
                if let Some(made) = s.built.remove(&i) {
                    s.taken = i + 1;
                    break made;
                }
                self.ready.wait(&mut s);
            }
        };
        self.room.notify_all();
        made.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    /// Posts a job for the helpers: its id, for `result`.
    pub(super) fn post(&self, job: J) -> usize {
        let id = {
            let mut s = self.state.lock();
            let id = s.posted;
            s.posted += 1;
            s.jobs.push_back((id, job));
            id
        };
        self.room.notify_all();
        id
    }

    /// Job `id`'s result: once done when `wait`, else if it is done.
    pub(super) fn result(&self, id: usize, wait: bool) -> Option<O> {
        let made = {
            let mut s = self.state.lock();
            loop {
                if let Some(made) = s.done.remove(&id) {
                    break made;
                }
                if !wait {
                    return None;
                }
                self.ready.wait(&mut s);
            }
        };
        Some(made.unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
    }

    /// No job will be posted: helpers end once every document is claimed and the jobs are done.
    pub(super) fn finish(&self) {
        self.state.lock().finished = true;
        self.room.notify_all();
    }

    /// Ends the helpers: each finishes what it is doing and takes nothing else.
    pub(super) fn stop(&self) {
        self.state.lock().stopped = true;
        self.room.notify_all();
    }

    /// Stops the helpers when dropped: held by the worker for the scope, so a panic it resumes (or any early end)
    /// leaves no helper waiting and the scope joins.
    pub(super) fn stopper(&self) -> Stopper<'_, R, J, O> {
        Stopper(self)
    }
}

pub(super) struct Stopper<'a, R: Send, J: Send, O: Send>(&'a Ordered<R, J, O>);

impl<R: Send, J: Send, O: Send> Drop for Stopper<'_, R, J, O> {
    fn drop(&mut self) {
        self.0.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::Ordered;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    /// Built on three threads with slower early items, taken in order; at most `ahead` built and waiting; jobs
    /// posted meanwhile are run and their results read.
    #[test]
    fn items_built_on_several_threads_are_taken_in_order() {
        let n = 200;
        let ordered: Ordered<usize, usize, usize> = Ordered::new(n, 8);
        let (waiting, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let (taken, jobs): (Vec<usize>, Vec<usize>) = std::thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    ordered.help(
                        |i| {
                            std::thread::sleep(std::time::Duration::from_micros(((n - i) % 7) as u64 * 50));
                            most.fetch_max(waiting.fetch_add(1, SeqCst) + 1, SeqCst);
                            i * 2
                        },
                        |j| j + 1000,
                    )
                });
            }
            let mut ids = Vec::new();
            let taken = (0..n)
                .map(|i| {
                    let v = ordered.take(i);
                    waiting.fetch_sub(1, SeqCst);
                    if i % 50 == 0 {
                        ids.push(ordered.post(i));
                    }
                    v
                })
                .collect();
            ordered.finish();
            (taken, ids.into_iter().map(|id| ordered.result(id, true).unwrap()).collect())
        });
        assert_eq!(taken, (0..n).map(|i| i * 2).collect::<Vec<_>>());
        assert_eq!(jobs, vec![1000, 1050, 1100, 1150]);
        assert!(most.load(SeqCst) <= 8 + 3, "at most `ahead` built ahead, and one a helper");
    }

    /// Stopped after two taken, the helpers end and the scope joins.
    #[test]
    fn stopped_helpers_end() {
        let ordered: Ordered<usize, (), ()> = Ordered::new(1000, 4);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| ordered.help(|i| i, |()| ()));
            }
            assert_eq!((ordered.take(0), ordered.take(1)), (0, 1));
            ordered.stop();
        });
    }

    /// A helper's panic reaches the taker of its document.
    #[test]
    fn a_helpers_panic_reaches_the_taker() {
        let ordered: Ordered<usize, (), ()> = Ordered::new(3, 4);
        let taken = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            std::thread::scope(|scope| {
                scope.spawn(|| ordered.help(|i| if i == 1 { panic!("item 1") } else { i }, |()| ()));
                let _stop = ordered.stopper();
                assert_eq!(ordered.take(0), 0);
                ordered.take(1)
            })
        }));
        assert!(taken.is_err());
    }
}
