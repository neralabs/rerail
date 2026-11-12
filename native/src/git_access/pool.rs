//! The threads every engine Git read runs on: four of their own, apart from libuv's pool and the
//! rule workers, so a slow disk under one checkout cannot hold up rule evaluation. A read has the
//! time limit Rerail gives a Git process (10 s); past it the caller gets an error while the read's
//! thread finishes on its own, since a thread cannot be ended from outside. A panic in a read
//! becomes an error naming the read, and the thread stays in the pool.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::LazyLock;
use std::time::Duration;

pub(crate) const GIT_READ_THREADS: usize = 4;
/// The limit `src/engine/git.ts` gives a Git process (`DEFAULT_TIMEOUT_MS`).
pub(crate) const GIT_READ_TIME_LIMIT: Duration = Duration::from_secs(10);

static GIT_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(GIT_READ_THREADS)
        .thread_name(|index| format!("rerail-git-{index}"))
        .build()
        .expect("build Rerail's Git read pool")
});

/// Runs `read` on the Git read pool and returns its answer, or an error naming `what` when it
/// panics or has not answered within `limit` of being asked (time spent waiting for a thread
/// counts too).
pub(crate) fn run<T: Send + 'static>(
    what: &str,
    limit: Duration,
    read: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    GIT_POOL.spawn(move || {
        // Unwind-safe in effect: a read owns what it works on, and the caller only sees the error.
        let _ = sender.send(catch_unwind(AssertUnwindSafe(read)));
    });
    match receiver.recv_timeout(limit) {
        Ok(Ok(answer)) => Ok(answer),
        Ok(Err(payload)) => Err(format!("{what} panicked: {}", panic_message(&*payload))),
        Err(_) => Err(format!(
            "{what} gave no answer within {} s",
            limit.as_secs_f64()
        )),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_owned())
}
