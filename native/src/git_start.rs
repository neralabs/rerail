//! Starting Git from a large process without copying it (Linux).
//!
//! Node's `child_process` starts every child through libuv, which on Linux calls `fork()`: the kernel copies the
//! caller's page tables, marks every written page copy-on-write, and the child tears that copy down again when it
//! runs `exec`. libuv waits for the `exec` on the calling thread, so in the daemon each Git start holds its event loop
//! for a time that grows with its resident memory (about 30 ms per GB measured), and every page the daemon writes
//! afterwards takes a fault. glibc's `posix_spawn` starts the child with `clone(CLONE_VM | CLONE_VFORK)`: the child
//! runs in the caller's memory until `exec`, so nothing is copied, whatever the caller's size.
//!
//! `start_git` starts a child for a reader that does not block, and `run_git` runs one to its end as `spawnSync` does.
//! `start_git` gives the child what libuv gives it: Unix socket pairs of 64 KiB buffers for its stdio (blocking on the
//! child's side), the null device for an ignored stderr, signals 1 to 31 at their default action and an empty signal
//! mask, the caller's working folder and no other descriptor (every descriptor this process opens is close-on-exec).
//! A thread waits for the child to exit without reaping it (`WNOWAIT`) and then closes the `exited` pipe; the caller
//! reaps it with `reap_git` once it has read that close, on its own thread. Until then the child stays a zombie, so its
//! pid names no other process and a kill sent before the reap reaches this child or nothing.

use napi::bindgen_prelude::Buffer;
use napi::{Error, Result, Status};
use napi_derive::napi;

/// The descriptors and pid of a Git `start_git` started. Each descriptor is this process's to close.
#[napi(object)]
pub struct StartedGit {
    pub pid: i32,
    pub stdin: i32,
    pub stdout: i32,
    /// -1 when the child's stderr is the null device.
    pub stderr: i32,
    /// The read end of a pipe whose write end closes once the child has exited.
    pub exited: i32,
}

/// How a child `reap_git` reaped ended: its exit code, or the signal that ended it. Both absent when it was not this
/// process's child to reap.
#[napi(object)]
pub struct GitExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// How a blocking run (`run_git`) ended: as `GitExit`, with what the child printed, and whether the run sent it
/// `SIGTERM` for its time limit or for printing more than its output limit.
#[napi(object)]
pub struct GitRun {
    pub status: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: Buffer,
    pub stderr: Buffer,
    pub timed_out: bool,
    pub past_max_buffer: bool,
}

/// Runs `program` as `start_git` starts it, with no input, and blocks until it has exited and both outputs have
/// ended, as Node's `spawnSync` does: `SIGTERM` once `timeout_ms` has passed, or once stdout and stderr hold more than
/// `max_buffer` bytes together.
#[napi]
pub fn run_git(program: String, args: Vec<String>, env: Vec<String>, timeout_ms: Option<u32>, max_buffer: u32) -> Result<GitRun> {
    #[cfg(target_os = "linux")]
    {
        run::run(&program, &args, &env, timeout_ms, max_buffer).map_err(|error| failed("runGit", &error))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (program, args, env, timeout_ms, max_buffer);
        Err(Error::new(Status::GenericFailure, "runGit is only needed on Linux; child_process runs Git elsewhere"))
    }
}

/// Starts `program` (an absolute path) with `args` (argv from its second entry) and `env` (`NAME=value` entries),
/// stdin and stdout on socket pairs, and stderr on one when `pipe_stderr`, else on the null device.
#[napi]
pub fn start_git(program: String, args: Vec<String>, env: Vec<String>, pipe_stderr: bool) -> Result<StartedGit> {
    #[cfg(target_os = "linux")]
    {
        spawn::start(&program, &args, &env, pipe_stderr).map_err(|error| failed("startGit", &error))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (program, args, env, pipe_stderr);
        Err(Error::new(Status::GenericFailure, "startGit is only needed on Linux; child_process starts Git elsewhere"))
    }
}

/// Reaps `pid`, a child `start_git` started whose `exited` pipe has closed, and says how it ended.
#[napi]
pub fn reap_git(pid: i32) -> GitExit {
    #[cfg(target_os = "linux")]
    {
        spawn::reap(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        GitExit { code: None, signal: None }
    }
}

#[cfg(target_os = "linux")]
mod run;
#[cfg(target_os = "linux")]
mod spawn;

#[cfg(target_os = "linux")]
fn failed(call: &str, error: &std::io::Error) -> Error {
    Error::new(Status::GenericFailure, format!("{call}: {error}"))
}
