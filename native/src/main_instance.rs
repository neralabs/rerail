//! The OS advisory lock behind Rerail's single main instance (`src/engine/native/main-instance.ts`)
//! and the locks a Rerail home's processes take one at a time (`file-lock.ts`, `home-daemon-lock.ts`).
//!
//! `std::fs::File::try_lock` is `flock(LOCK_EX | LOCK_NB)` on Unix and `LockFileEx` on Windows.
//! Both belong to the open handle, so the OS drops the lock when the holder closes it or exits for
//! any reason, `SIGKILL` included; no stale state survives a crash and nothing is reclaimed.
//!
//! A waiting take ([`lock_exclusive`]) asks the OS for the lock without the fail-at-once flag, so
//! the OS hands it over the moment its holder lets go, to the waiters it has queued. The wait runs
//! on a thread of its own, never on the JavaScript thread or on Node's thread pool, whose threads
//! also serve file reads; its bound is the caller's.

use napi::bindgen_prelude::Object;
use napi::{Env, Error, JsDeferred, Result, Status};
use napi_derive::napi;
use parking_lot::Mutex;
use std::fs::{File, OpenOptions, TryLockError};
use std::sync::mpsc;
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

/// Opens (creating) the lock file at `path`. On Windows the handle is opened for overlapped I/O,
/// so a waiting `LockFileEx` returns at once and its wait can be given a bound and cancelled.
fn open_lock_file(path: &str) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED);
    }
    options
        .open(path)
        .map_err(|error| lock_failure("open", path, error))
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
type Settle = Box<dyn FnOnce(Env) -> Result<Option<ExclusiveFileLock>> + Send>;

/// Takes an exclusive lock on `path`, creating the file when missing, waiting up to `wait_ms` while
/// another handle holds it: resolves with the lock the moment the OS hands it over, or with `null`
/// when the other handle still holds it at the bound. A wait given up never ends up holding the
/// lock. The file opens before this returns, so an open that fails throws here.
#[napi(ts_return_type = "Promise<ExclusiveFileLock | null>")]
pub fn lock_exclusive(env: &Env, path: String, wait_ms: u32) -> Result<Object<'_>> {
    let file = open_lock_file(&path)?;
    let wait = Duration::from_millis(wait_ms.into());
    // The promise is made only once its thread runs: a promise left unsettled would keep Node's
    // event loop, and so the process, alive for ever.
    let (hand, handed) = mpsc::sync_channel::<JsDeferred<Option<ExclusiveFileLock>, Settle>>(1);
    std::thread::Builder::new()
        .name("rerail-lock-wait".to_owned())
        .spawn(move || {
            if let Ok(deferred) = handed.recv() {
                settle(deferred, wait_and_name(file, wait, &path));
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
    Ok(promise)
}

fn wait_and_name(file: File, wait: Duration, path: &str) -> Result<Option<File>> {
    crate::panic_boundary::contain_panic("the wait for a lock", || {
        wait_for_lock(file, wait).map_err(|error| lock_failure("lock", path, error))
    })
}

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
    std::thread::Builder::new()
        .name("rerail-lock-flock".to_owned())
        .spawn(move || {
            let taken = file.lock().map(|()| file);
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

/// `LockFileEx` on the overlapped handle returns at once with the request pending; its completion
/// signals the event. At the bound `CancelIoEx` ends the request, and its result then says whether
/// the OS granted it first, so no request outlives the wait.
#[cfg(windows)]
fn wait_for_lock(file: File, wait: Duration) -> std::io::Result<Option<File>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LockFileEx};
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let handle = file.as_raw_handle();
    let event = Event::new()?;
    // SAFETY: an all-zero OVERLAPPED is valid; offset 0 and the whole length are the range
    // `File::try_lock` locks, so the two conflict.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event.0;
    // SAFETY: the handle, the event and the OVERLAPPED outlive the request: it completes, or is
    // cancelled and waited for, before any of them goes.
    let requested = unsafe {
        LockFileEx(
            handle,
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    } != 0;
    // Read right after the call: a request that did not complete at once says why in the last error.
    let granted = requested || granted_within(handle, &overlapped, &event, wait)?;
    Ok(granted.then_some(file))
}

/// Whether the lock request that `LockFileEx` just left pending is granted within `wait`: waits for
/// its event, cancels it at the bound, then waits for it to end either way.
#[cfg(windows)]
fn granted_within(
    handle: std::os::windows::io::RawHandle,
    overlapped: &windows_sys::Win32::System::IO::OVERLAPPED,
    event: &Event,
    wait: Duration,
) -> std::io::Result<bool> {
    use windows_sys::Win32::Foundation::{
        ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult};
    use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};

    let error = std::io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        return Err(error);
    }
    // INFINITE is u32::MAX: a longer bound is one millisecond short of it, never no bound.
    let millis = u32::try_from(wait.as_millis())
        .unwrap_or(INFINITE)
        .min(INFINITE - 1);
    // SAFETY: the event is valid. A wait that ends any other way than signalled cancels the
    // request, so the result below waits only for that request to end.
    if unsafe { WaitForSingleObject(event.0, millis) } != WAIT_OBJECT_0 {
        unsafe { CancelIoEx(handle, overlapped) };
    }
    let mut transferred = 0u32;
    // SAFETY: waits for this handle's one request to complete or end cancelled.
    if unsafe { GetOverlappedResult(handle, overlapped, &mut transferred, 1) } != 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED as i32) {
        return Ok(false);
    }
    Err(error)
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
    const NEVER: Duration = Duration::from_secs(60);

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

    #[test]
    fn a_waiter_is_handed_the_lock_at_its_holders_release() {
        let (directory, path) = lock_path("handover");
        let held = try_lock_exclusive(path.clone()).unwrap().unwrap();
        let (started, waiting) = mpsc::channel();
        let file = open_lock_file(&path).unwrap();
        let waiter = std::thread::spawn(move || {
            started.send(()).unwrap();
            wait_for_lock(file, NEVER)
        });
        waiting.recv().unwrap();
        held.release();

        let taken = waiter.join().unwrap().unwrap();
        assert!(
            taken.is_some(),
            "the waiter holds the lock once its holder let go"
        );
        assert!(
            try_lock_exclusive(path.clone()).unwrap().is_none(),
            "the lock the waiter was handed is held"
        );
        drop(taken);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_wait_given_up_at_its_bound_never_ends_up_holding_the_lock() {
        let (directory, path) = lock_path("given-up");
        let held = try_lock_exclusive(path.clone()).unwrap().unwrap();

        let given_up = wait_for_lock(open_lock_file(&path).unwrap(), Duration::from_millis(20));
        assert!(
            given_up.unwrap().is_none(),
            "a wait whose holder stays ends empty at its bound"
        );
        held.release();

        // Whatever the given-up wait's request does at the release, it lets go: the next waiter is
        // handed the lock, long before its own bound.
        let next = wait_for_lock(open_lock_file(&path).unwrap(), NEVER).unwrap();
        assert!(
            next.is_some(),
            "a wait given up must leave the lock free once its holder lets go"
        );
        drop(next);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_free_lock_is_taken_at_once_by_a_wait() {
        let (directory, path) = lock_path("free");
        let taken = wait_for_lock(open_lock_file(&path).unwrap(), Duration::ZERO).unwrap();
        assert!(taken.is_some(), "a free lock is taken whatever the bound");
        drop(taken);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
