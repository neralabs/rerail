//! Where the engine opens a Git repository: on a thread of its own, under the time limit Rerail
//! gives a Git process (10 s), so a disk that stops answering costs the open, never the JS thread
//! or a libuv thread for good. Past the limit the caller gets an error while the open's thread
//! finishes on its own, since a thread cannot be ended from outside. Until every open of that root
//! has ended, a later open of it fails at once instead of stranding another thread on the same
//! disk; opens of other roots are not held up. A root is the path as the caller names it, so a hung
//! disk holds one thread per root path. A panic in an open becomes an error naming it.
//!
//! Opens are rare (one per checkout and reader), so a thread each costs nothing that matters, and
//! no fixed set of threads can be used up by hung ones.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// The limit `src/engine/git.ts` gives a Git process (`DEFAULT_TIMEOUT_MS`).
pub const GIT_READ_TIME_LIMIT: Duration = Duration::from_secs(10);

/// Why an open gave no answer: `timed_out` when it, or an earlier open of the same root, outran
/// its limit, so a later try may answer; otherwise it failed (a panic, no thread).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitReadError {
    pub timed_out: bool,
    pub message: String,
}

impl std::fmt::Display for GitReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// The opens of one root that have not ended, and whether one of them outran its limit.
#[derive(Default)]
struct Root {
    running: usize,
    stuck: bool,
}

static ROOTS: Mutex<Option<HashMap<PathBuf, Root>>> = Mutex::new(None);

fn roots<R>(change: impl FnOnce(&mut HashMap<PathBuf, Root>) -> R) -> R {
    change(
        ROOTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashMap::new),
    )
}

/// One open of a root: it counts in the root's `running` until it ends, and `ended` says, under
/// the same lock, whether it has.
struct Running {
    root: PathBuf,
    ended: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        roots(|roots| {
            self.ended.store(true, Ordering::Relaxed);
            if let Some(root) = roots.get_mut(&self.root) {
                root.running -= 1;
                if root.running == 0 {
                    roots.remove(&self.root);
                }
            }
        });
    }
}

/// Runs `read`, an open of `root`, on a thread of its own and returns its answer, or an error
/// naming `what` when it panics, has not answered within `limit`, or an open of `root` that outran
/// its limit has not ended yet.
pub fn run<T: Send + 'static>(
    what: &str,
    root: &Path,
    limit: Duration,
    read: impl FnOnce() -> T + Send + 'static,
) -> Result<T, GitReadError> {
    let admitted = roots(|roots| {
        let entry = roots.entry(root.to_path_buf()).or_default();
        if entry.stuck {
            return false;
        }
        entry.running += 1;
        true
    });
    if !admitted {
        return Err(GitReadError {
            timed_out: true,
            message: format!(
                "{what} for {}: an earlier one has given no answer for over {} s and has not ended",
                root.display(),
                limit.as_secs_f64()
            ),
        });
    }
    let ended = Arc::new(AtomicBool::new(false));
    let running = Running {
        root: root.to_path_buf(),
        ended: Arc::clone(&ended),
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("rerail-git-open".to_owned())
        .spawn(move || {
            // Unwind-safe in effect: a read owns what it works on, and the caller only sees the error.
            let answer = catch_unwind(AssertUnwindSafe(read));
            drop(running);
            let _ = sender.send(answer);
        })
        .map_err(|error| GitReadError {
            timed_out: false,
            message: format!("{what}: cannot start a thread: {error}"),
        })?;
    match receiver.recv_timeout(limit) {
        Ok(Ok(answer)) => Ok(answer),
        Ok(Err(payload)) => Err(GitReadError {
            timed_out: false,
            message: format!("{what} panicked: {}", panic_message(&*payload)),
        }),
        Err(RecvTimeoutError::Timeout) => {
            // Marked only while this open still runs, so its root's entry is the one it counts in;
            // an open that ended just now leaves a newer open of the root alone.
            roots(|roots| {
                if !ended.load(Ordering::Relaxed)
                    && let Some(entry) = roots.get_mut(root)
                {
                    entry.stuck = true;
                }
            });
            Err(GitReadError {
                timed_out: true,
                message: format!("{what} gave no answer within {} s", limit.as_secs_f64()),
            })
        }
        Err(RecvTimeoutError::Disconnected) => Err(GitReadError {
            timed_out: false,
            message: format!("{what} ended without an answer"),
        }),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_owned())
}
