//! The OS advisory lock behind Rerail's single main instance (`src/engine/native/main-instance.ts`)
//! and the locks a Rerail home's processes take one at a time (`file-lock.ts`, `home-daemon-lock.ts`).
//!
//! `std::fs::File::try_lock` is `flock(LOCK_EX | LOCK_NB)` on Unix and `LockFileEx` on Windows.
//! Both belong to the open handle, so the OS drops the lock when the holder closes it or exits for
//! any reason, `SIGKILL` included; no stale state survives a crash and nothing is reclaimed.
//!
//! A waiting take ([`lock_exclusive`]) tries first, on the calling thread, so a free lock costs what
//! a try costs. While another handle holds the lock, it asks the OS without the fail-at-once flag,
//! so the OS hands the lock over the moment its holder lets go, to the waiters it has queued. The
//! wait runs on a thread of its own, never on the JavaScript thread or on Node's thread pool, whose
//! threads also serve file reads; its bound is the caller's. On Windows the calling thread queues
//! the request, which returns at once, and hands it to that thread; `flock` blocks the thread that
//! asks, so on Unix the waiting thread asks.

use napi::bindgen_prelude::PromiseRaw;
use napi::{Env, Error, JsDeferred, JsValue, Result, Status};
use napi_derive::napi;
use parking_lot::Mutex;
use std::fs::{File, OpenOptions, TryLockError};
use std::sync::mpsc;
// Only the waits of Unix and Windows count time; wasm32-wasip1 has none.
#[cfg(any(unix, windows))]
use std::time::Duration;

/// An exclusive lock on one file, held until `release` or until the owning process exits.
#[napi]
pub struct ExclusiveFileLock {
    file: Mutex<Option<File>>,
}

impl ExclusiveFileLock {
    fn holding(file: File) -> Self {
        Self {
            file: Mutex::new(Some(file)),
        }
    }
}

#[napi]
impl ExclusiveFileLock {
    /// Unlocks and closes the file now rather than at process exit; later calls do nothing.
    #[napi]
    pub fn release(&self) {
        if let Some(file) = self.file.lock().take() {
            // Unlock before the close so the lock is free the moment this returns, even where a
            // duplicated handle (a forked child) would keep the open file description alive. An
            // unlock failure leaves the close, which releases the lock anyway.
            let _ = file.unlock();
        }
    }
}

fn lock_failure(action: &str, path: &str, error: std::io::Error) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("cannot {action} lock file {path}: {error}"),
    )
}

/// Opens (creating) the lock file at `path`, as the main-instance election always has.
fn open_lock_file(path: &str) -> Result<File> {
    lock_file_options()
        .open(path)
        .map_err(|error| lock_failure("open", path, error))
}

/// Opens (creating) the lock file at `path` for a wait. On Windows the handle is opened for
/// overlapped I/O, so a waiting `LockFileEx` returns at once and its wait can be given a bound and
/// cancelled; a try keeps its synchronous handle.
fn open_waiting_lock_file(path: &str) -> Result<File> {
    let mut options = lock_file_options();
    overlapped_on_windows(&mut options);
    options
        .open(path)
        .map_err(|error| lock_failure("open", path, error))
}

#[cfg(windows)]
fn overlapped_on_windows(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED);
}

/// Unix opens a waiting take's file as any other: `flock` blocks on any descriptor.
#[cfg(not(windows))]
fn overlapped_on_windows(_options: &mut OpenOptions) {}

fn lock_file_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    options
}

/// Takes an exclusive lock on `path`, creating the file when missing. `None` when another handle
/// holds it, whether in another process or in this one.
#[napi]
pub fn try_lock_exclusive(path: String) -> Result<Option<ExclusiveFileLock>> {
    let file = open_lock_file(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(ExclusiveFileLock::holding(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(lock_failure("lock", &path, error)),
    }
}

/// What the wait's promise settles with, made on the JavaScript thread.
// Only the thread's wait of `lock_exclusive` uses it, which only JavaScript calls and
// wasm32-wasip1 has not.
#[cfg_attr(any(test, not(any(unix, windows))), allow(dead_code))]
type Settle = Box<dyn FnOnce(Env) -> Result<Option<ExclusiveFileLock>> + Send>;

/// The promise `lock_exclusive` returns.
// Only `lock_exclusive` uses it, which only JavaScript calls.
#[cfg_attr(test, allow(dead_code))]
type Taking<'env> = PromiseRaw<'env, Option<ExclusiveFileLock>>;

/// Takes an exclusive lock on `path`, creating the file when missing, waiting up to `wait_ms` while
/// another handle holds it; `wait_ms` 0 tries once. A free lock is taken on the calling thread, and
/// its promise is already settled when this returns: no thread starts and the event loop need not
/// turn. A held one is waited for on a thread of its own: the promise resolves with the lock the
/// moment the OS hands it over, or with `null` when the other handle still holds it at the bound.
/// A wait given up never ends up holding the lock. The open and the first try happen before this
/// returns, so a failure of either throws here.
#[napi(ts_return_type = "Promise<ExclusiveFileLock | null>")]
// Only JavaScript calls it; tests make the calls its wait makes (`wait_for_lock` on Unix,
// `request_lock` then `PendingLock::granted_within` on Windows).
#[cfg_attr(test, allow(dead_code))]
pub fn lock_exclusive(env: &Env, path: String, wait_ms: u32) -> Result<Taking<'_>> {
    let file = open_for_take(&path, wait_ms)?;
    match file.try_lock() {
        Ok(()) => PromiseRaw::resolve(env, Some(ExclusiveFileLock::holding(file))),
        Err(TryLockError::WouldBlock) if wait_ms == 0 => PromiseRaw::resolve(env, None),
        Err(TryLockError::WouldBlock) => wait_for_release(env, file, wait_ms, path),
        Err(TryLockError::Error(error)) => Err(lock_failure("lock", &path, error)),
    }
}

/// A take that never waits keeps the handle a try has always had; one that may wait opens its file
/// for the wait.
#[cfg_attr(test, allow(dead_code))]
fn open_for_take(path: &str, wait_ms: u32) -> Result<File> {
    if wait_ms == 0 {
        open_lock_file(path)
    } else {
        open_waiting_lock_file(path)
    }
}

/// Unix: the waiting thread asks the OS for the lock another handle holds (`wait_for_lock`), up to
/// `wait_ms`, since `flock` blocks the thread that asks.
#[cfg(unix)]
#[cfg_attr(test, allow(dead_code))]
fn wait_for_release(env: &Env, file: File, wait_ms: u32, path: String) -> Result<Taking<'_>> {
    let wait = Duration::from_millis(wait_ms.into());
    wait_on_its_thread(env, path, move || wait_for_lock(file, wait))
}

/// Windows: this thread queues the request in the kernel (`request_lock` returns at once on the
/// overlapped handle) and hands it to the waiting thread, which waits for the OS to grant it up to
/// `wait_ms` (`PendingLock::granted_within`). A lock its holder let go of since the try is granted
/// to the request at once, and taken here. The OS ends a thread's requests as the thread ends; the
/// calling thread is Node's, which lives as long as the process.
#[cfg(windows)]
#[cfg_attr(test, allow(dead_code))]
fn wait_for_release(env: &Env, file: File, wait_ms: u32, path: String) -> Result<Taking<'_>> {
    let wait = Duration::from_millis(wait_ms.into());
    match request_lock(file).map_err(|error| lock_failure("lock", &path, error))? {
        Requested::Granted(file) => {
            PromiseRaw::resolve(env, Some(ExclusiveFileLock::holding(file)))
        }
        Requested::Pending(pending) => {
            wait_on_its_thread(env, path, move || pending.granted_within(wait))
        }
    }
}

/// wasm32-wasip1, and any target that is neither: no thread can wait for the holder there, so a
/// held lock settles at once with `null`, as a take that never waits, and the thread's helpers
/// below go unused.
#[cfg(not(any(unix, windows)))]
fn wait_for_release(env: &Env, _file: File, _wait_ms: u32, _path: String) -> Result<Taking<'_>> {
    PromiseRaw::resolve(env, None)
}

/// Runs `wait`, a wait for the lock another handle holds, on a thread of its own, and settles the
/// promise it returns with what that wait ends with.
#[cfg_attr(any(test, not(any(unix, windows))), allow(dead_code))]
fn wait_on_its_thread<Wait>(env: &Env, path: String, wait: Wait) -> Result<Taking<'_>>
where
    Wait: FnOnce() -> std::io::Result<Option<File>> + Send + 'static,
{
    // The promise is made only once its thread runs: a promise left unsettled would keep Node's
    // event loop, and so the process, alive for ever.
    let (hand, handed) = mpsc::sync_channel::<JsDeferred<Option<ExclusiveFileLock>, Settle>>(1);
    // A named service thread of the process's one executor: it waits on the OS, so it holds none
    // of the pool's threads.
    rerail_accel::exec::service("rerail-lock-wait", None, move || {
        if let Ok(deferred) = handed.recv() {
            settle(deferred, wait_and_name(wait, &path));
        }
    })
    .map_err(|error| {
        Error::new(
            Status::GenericFailure,
            format!("cannot start a lock wait: {error}"),
        )
    })?;
    let (deferred, promise) = env.create_deferred()?;
    // The thread waits in `recv` until this arrives, so the send cannot fail.
    let _ = hand.send(deferred);
    Ok(PromiseRaw::new(env.raw(), promise.raw()))
}

#[cfg_attr(any(test, not(any(unix, windows))), allow(dead_code))]
fn wait_and_name(
    wait: impl FnOnce() -> std::io::Result<Option<File>>,
    path: &str,
) -> Result<Option<File>> {
    crate::panic_boundary::contain_panic("the wait for a lock", || {
        wait().map_err(|error| lock_failure("lock", path, error))
    })
}

#[cfg_attr(any(test, not(any(unix, windows))), allow(dead_code))]
fn settle(deferred: JsDeferred<Option<ExclusiveFileLock>, Settle>, waited: Result<Option<File>>) {
    match waited {
        Ok(file) => deferred.resolve(Box::new(move |_| Ok(file.map(ExclusiveFileLock::holding)))),
        Err(error) => deferred.reject(error),
    }
}

/// `flock` takes no bound, so a helper thread blocks in it and this thread waits for the helper up
/// to `wait`. A wait given up leaves the helper blocked until the holder lets go; the helper then
/// drops the lock it is handed at once. Only the holder's release or exit ends the helper.
#[cfg(unix)]
fn wait_for_lock(file: File, wait: Duration) -> std::io::Result<Option<File>> {
    use std::sync::Arc;
    match file.try_lock() {
        Ok(()) => return Ok(Some(file)),
        Err(TryLockError::Error(error)) => return Err(error),
        Err(TryLockError::WouldBlock) => {}
    }
    let (sender, receiver) = mpsc::channel();
    // `None` once this side has given up: the helper sends while it holds the slot, so a lock it
    // took before the bound is in the channel, and one it takes after is dropped.
    let slot = Arc::new(Mutex::new(Some(sender)));
    let helper_slot = Arc::clone(&slot);
    // A named service thread of the process's one executor, as the wait above: `flock` blocks it.
    rerail_accel::exec::service("rerail-lock-flock", None, move || {
        let taken = lock_blocking(&file).map(|()| file);
        if let Some(sender) = helper_slot.lock().take() {
            let _ = sender.send(taken);
        }
    })?;
    if let Ok(taken) = receiver.recv_timeout(wait) {
        return taken.map(Some);
    }
    slot.lock().take();
    receiver
        .try_recv()
        .map_or(Ok(None), |taken| taken.map(Some))
}

/// `flock(LOCK_EX)`, asked again when a signal interrupts it: a handler installed without
/// `SA_RESTART` would otherwise end the wait with an error while the holder still holds the lock.
#[cfg(unix)]
fn lock_blocking(file: &File) -> std::io::Result<()> {
    loop {
        match file.lock() {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            done => return done,
        }
    }
}

/// A manual-reset event, closed when dropped.
#[cfg(windows)]
struct Event(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Event {
    fn new() -> std::io::Result<Self> {
        // SAFETY: a manual-reset event with no name or attributes, closed by `drop`.
        let event = unsafe {
            windows_sys::Win32::System::Threading::CreateEventW(
                std::ptr::null(),
                1,
                0,
                std::ptr::null(),
            )
        };
        if event.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self(event))
    }
}

#[cfg(windows)]
impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the event is this value's own; the request that signals it has ended first.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// What `LockFileEx` did with a request for the lock on an overlapped handle.
#[cfg(windows)]
enum Requested {
    /// The lock was free: the request completed at once.
    Granted(File),
    /// Another handle holds the lock: the OS queued the request.
    Pending(PendingLock),
}

/// A lock request the OS queued (`LockFileEx` answered `ERROR_IO_PENDING`); it completes when the OS hands the lock
/// over. It owns the handle, the event and the OVERLAPPED the request uses, and one dropped before its request ended
/// cancels it and waits for it to end, so no request outlives them.
#[cfg(windows)]
struct PendingLock {
    handle: std::os::windows::io::RawHandle,
    file: Option<File>,
    event: Event,
    // Boxed: the OS may write to it until the request ends, so it must not move.
    overlapped: Box<windows_sys::Win32::System::IO::OVERLAPPED>,
    ended: bool,
}

// SAFETY: the handles are kernel handles any thread of the process may wait on, cancel and close,
// and only the one thread that owns the request uses them; the OVERLAPPED is boxed, so moving the
// request moves no memory the OS writes to.
#[cfg(windows)]
unsafe impl Send for PendingLock {}

/// Asks the OS for the lock on `file`'s overlapped handle without the fail-at-once flag. A held lock leaves the request
/// queued: `LockFileEx` returns at once, and the request completes at the hand-over.
#[cfg(windows)]
fn request_lock(file: File) -> std::io::Result<Requested> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_IO_PENDING;
    use windows_sys::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LockFileEx};

    let handle = file.as_raw_handle();
    let event = Event::new()?;
    // SAFETY: an all-zero OVERLAPPED is valid; offset 0 and the whole length are the range
    // `File::try_lock` locks, so the two conflict.
    let mut overlapped: Box<windows_sys::Win32::System::IO::OVERLAPPED> =
        Box::new(unsafe { std::mem::zeroed() });
    overlapped.hEvent = event.0;
    // SAFETY: the handle, the event and the OVERLAPPED outlive the request: a queued one is
    // `PendingLock`'s, which ends it before any of them goes.
    let granted = unsafe {
        LockFileEx(
            handle,
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            u32::MAX,
            u32::MAX,
            &mut *overlapped,
        )
    } != 0;
    if granted {
        return Ok(Requested::Granted(file));
    }
    // Read right after the call: a request that did not complete at once says why in the last error.
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        return Err(error);
    }
    Ok(Requested::Pending(PendingLock {
        handle,
        file: Some(file),
        event,
        overlapped,
        ended: false,
    }))
}

#[cfg(windows)]
impl PendingLock {
    /// Whether the OS grants the request within `wait`: waits for its event, cancels it at the bound, then waits for it
    /// to end either way, so its result says whether the OS granted it before the cancel.
    fn granted_within(mut self, wait: Duration) -> std::io::Result<Option<File>> {
        use windows_sys::Win32::Foundation::{ERROR_OPERATION_ABORTED, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};

        // INFINITE is u32::MAX: a longer bound is one millisecond short of it, never no bound.
        let millis = u32::try_from(wait.as_millis())
            .unwrap_or(INFINITE)
            .min(INFINITE - 1);
        // SAFETY: the event is this request's own and valid.
        if unsafe { WaitForSingleObject(self.event.0, millis) } != WAIT_OBJECT_0 {
            self.cancel();
        }
        match self.end() {
            Ok(()) => Ok(self.file.take()),
            Err(error) if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn cancel(&self) {
        // SAFETY: cancels this handle's one request if it has not ended; `end` reads its result.
        unsafe { windows_sys::Win32::System::IO::CancelIoEx(self.handle, &*self.overlapped) };
    }

    /// Waits for the request to complete or end cancelled: `Ok` when the OS granted it.
    fn end(&mut self) -> std::io::Result<()> {
        let mut transferred = 0u32;
        // SAFETY: waits for this handle's one request; the OVERLAPPED is valid until then.
        let completed = unsafe {
            windows_sys::Win32::System::IO::GetOverlappedResult(
                self.handle,
                &*self.overlapped,
                &mut transferred,
                1,
            )
        } != 0;
        let ended = if completed {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        };
        self.ended = true;
        ended
    }
}

#[cfg(windows)]
impl Drop for PendingLock {
    fn drop(&mut self) {
        if !self.ended {
            self.cancel();
            let _ = self.end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_path(name: &str) -> (std::path::PathBuf, String) {
        let directory = std::env::temp_dir().join(format!(
            "rerail-main-instance-lock-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory
            .join("main-instance.lock")
            .to_string_lossy()
            .into_owned();
        (directory, path)
    }

    /// Bounds a wait the test expects to end at a release, so a regression fails instead of hanging.
    #[cfg(any(target_os = "linux", windows))]
    const NEVER: Duration = Duration::from_secs(60);
    /// Bounds a step the test expects at once (a request queued in the kernel, a handler run, a thread
    /// ended, a wait given up), so one that never comes fails in seconds.
    #[cfg(any(target_os = "linux", windows))]
    const SOON: Duration = Duration::from_secs(10);

    /// Spins until `done`, failing when it is not so within [`SOON`]. Only Linux's tests wait on a
    /// thread's state this way; Windows sees its request queued in the kernel directly.
    #[cfg(target_os = "linux")]
    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + SOON;
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what}: not within {SOON:?}"
            );
            std::thread::yield_now();
        }
    }

    /// The thread blocked asking the OS for the lock on `fd` without the fail-at-once flag,
    /// `flock(fd, LOCK_EX)`, once one is: Linux names the system call each thread is blocked in, then
    /// its arguments (`/proc/self/task/<tid>/syscall`). Fails when `ended` says the wait ended first.
    #[cfg(target_os = "linux")]
    fn thread_blocked_in_flock(fd: std::os::fd::RawFd, ended: impl Fn() -> bool) -> String {
        let blocked = format!("{} {:#x} {:#x} ", libc::SYS_flock, fd, libc::LOCK_EX);
        let mut found = None;
        until("a thread asks the OS for the lock and waits", || {
            assert!(
                !ended(),
                "the wait ended before it asked the OS for the lock"
            );
            found = task_in_syscall(&blocked);
            found.is_some()
        });
        found.unwrap_or_default()
    }

    /// The first of this process's threads whose system call line starts with `blocked`.
    #[cfg(target_os = "linux")]
    fn task_in_syscall(blocked: &str) -> Option<String> {
        std::fs::read_dir("/proc/self/task")
            .ok()?
            .flatten()
            .find_map(|task| {
                let syscall = std::fs::read_to_string(task.path().join("syscall")).ok()?;
                syscall
                    .starts_with(blocked)
                    .then(|| task.file_name().to_string_lossy().into_owned())
            })
    }

    /// The request a held lock leaves queued in the kernel (`ERROR_IO_PENDING`).
    #[cfg(windows)]
    fn queued(file: File) -> PendingLock {
        match request_lock(file).unwrap() {
            Requested::Pending(pending) => pending,
            Requested::Granted(_) => {
                panic!("a request for a held lock must be queued in the kernel")
            }
        }
    }

    #[cfg(any(target_os = "linux", windows))]
    fn assert_handed(taken: std::io::Result<Option<File>>, path: &str) {
        let taken = taken.unwrap();
        assert!(
            taken.is_some(),
            "the waiter holds the lock once its holder let go"
        );
        assert!(
            try_lock_exclusive(path.to_owned()).unwrap().is_none(),
            "the lock the waiter was handed is held"
        );
    }

    #[test]
    fn a_held_lock_refuses_every_other_handle_until_released() {
        let (directory, path) = lock_path("refuses");

        let held = try_lock_exclusive(path.clone())
            .unwrap()
            .expect("the first handle takes the lock");
        assert!(
            try_lock_exclusive(path.clone()).unwrap().is_none(),
            "a second handle must be refused while the first holds the lock"
        );
        held.release();
        held.release();
        let again = try_lock_exclusive(path.clone()).unwrap();
        assert!(again.is_some(), "a released lock is free again");
        drop(again);

        std::fs::remove_dir_all(directory).unwrap();
    }

    // The wait's tests run where a test can see the request queued in the kernel before the holder lets
    // go, so the OS, not a later try, hands the lock over: Linux and Windows, where CI runs them.

    #[cfg(target_os = "linux")]
    #[test]
    fn a_waiter_is_handed_the_lock_at_its_holders_release() {
        use std::os::fd::AsRawFd;
        let (directory, path) = lock_path("handover");
        let held = try_lock_exclusive(path.clone()).unwrap().unwrap();
        let file = open_waiting_lock_file(&path).unwrap();
        let fd = file.as_raw_fd();
        let waiter = std::thread::spawn(move || wait_for_lock(file, NEVER));
        thread_blocked_in_flock(fd, || waiter.is_finished());
        held.release();

        assert_handed(waiter.join().unwrap(), &path);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_waiter_is_handed_the_lock_at_its_holders_release() {
        let (directory, path) = lock_path("handover");
        let held = try_lock_exclusive(path.clone()).unwrap().unwrap();
        // As `wait_for_release` does: this thread queues the request, a thread of its own waits for it.
        let pending = queued(open_waiting_lock_file(&path).unwrap());
        let waiter = std::thread::spawn(move || pending.granted_within(NEVER));
        held.release();
        // The OS grants the queued request as the holder lets go, so no handle that only tries gets
        // the lock in between.
        assert!(
            try_lock_exclusive(path.clone()).unwrap().is_none(),
            "the lock is the queued request's from its holder's release on"
        );

        assert_handed(waiter.join().unwrap(), &path);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_wait_given_up_at_its_bound_never_ends_up_holding_the_lock() {
        let (directory, path) = lock_path("given-up");
        let held = try_lock_exclusive(path.clone()).unwrap().unwrap();
        let file = open_waiting_lock_file(&path).unwrap();
        #[cfg(target_os = "linux")]
        let fd = std::os::fd::AsRawFd::as_raw_fd(&file);
        // On a thread of its own, so a wait whose cancel is lost fails here in seconds instead of hanging.
        // As `wait_for_release` waits: on Windows this thread queues the request first.
        #[cfg(target_os = "linux")]
        let wait = move || wait_for_lock(file, Duration::from_millis(20));
        #[cfg(windows)]
        let wait = {
            let pending = queued(file);
            move || pending.granted_within(Duration::from_millis(20))
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || sender.send(wait()));
        let given_up = receiver
            .recv_timeout(SOON)
            .expect("a wait ends at its bound");
        assert!(
            given_up.unwrap().is_none(),
            "a wait whose holder stays ends empty at its bound"
        );
        // Unix: the given-up wait's helper is still blocked in flock, and is handed the lock at the
        // release. Windows ended the request before the wait returned.
        #[cfg(target_os = "linux")]
        let helper = thread_blocked_in_flock(fd, || false);
        held.release();
        #[cfg(target_os = "linux")]
        until("the given-up wait's helper ends", || {
            !std::path::Path::new(&format!("/proc/self/task/{helper}")).exists()
        });

        assert!(
            try_lock_exclusive(path.clone()).unwrap().is_some(),
            "a wait given up must leave the lock free once its holder lets go"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// A signal whose handler does not restart system calls lands on the thread blocked in `flock`.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_signal_during_the_wait_does_not_end_it() {
        use std::os::fd::AsRawFd;
        use std::sync::atomic::{AtomicBool, Ordering};
        static HANDLED: AtomicBool = AtomicBool::new(false);
        extern "C" fn on_signal(_: libc::c_int) {
            HANDLED.store(true, Ordering::SeqCst);
        }
        // SAFETY: a handler that only stores to an atomic, installed without SA_RESTART.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            let handler: extern "C" fn(libc::c_int) = on_signal;
            action.sa_sigaction = handler as libc::sighandler_t;
            libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut());
        }
        let (directory, path) = lock_path("signal");
        let held = try_lock_exclusive(path.clone()).unwrap().unwrap();
        let file = open_waiting_lock_file(&path).unwrap();
        let fd = file.as_raw_fd();
        let (ids, waiter_ids) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            // SAFETY: the calling thread's own id.
            ids.send(unsafe { libc::pthread_self() }).unwrap();
            lock_blocking(&file)
        });
        let thread = waiter_ids.recv().unwrap();
        thread_blocked_in_flock(fd, || waiter.is_finished());
        // SAFETY: the thread is alive until it is joined below.
        unsafe { libc::pthread_kill(thread, libc::SIGUSR1) };
        // The handler runs as the interrupted flock returns, so the holder lets go only after that.
        until("the signal's handler runs", || {
            HANDLED.load(Ordering::SeqCst)
        });
        held.release();

        let locked = waiter.join().unwrap();
        assert!(
            locked.is_ok(),
            "an interrupted flock must be asked again: {locked:?}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_free_lock_is_taken_at_once_by_a_wait() {
        let (directory, path) = lock_path("free");
        let file = open_waiting_lock_file(&path).unwrap();
        #[cfg(unix)]
        let taken = wait_for_lock(file, Duration::ZERO).unwrap();
        // Windows grants a request for a free lock as it is made, with nothing queued.
        #[cfg(windows)]
        let taken = match request_lock(file).unwrap() {
            Requested::Granted(file) => Some(file),
            Requested::Pending(_) => None,
        };
        assert!(taken.is_some(), "a free lock is taken whatever the bound");
        drop(taken);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
