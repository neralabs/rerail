//! Which process holds the server end of a Windows named pipe, and when it started.
//!
//! Any local user can create a pipe of any name on Windows, so a process that finds a pipe by its name cannot tell from
//! the name whose it is. Windows records the process that created each pipe instance, and a process keeps its id for
//! life, so that id and its start time tell one process from another that took the same id after it ended.

use napi_derive::napi;

/// What Windows says of the server end of a named pipe.
#[napi(object)]
pub struct NamedPipeServer {
    /// The process holding it (`GetNamedPipeServerProcessId`); absent when other clients kept every instance of the
    /// pipe taken until the wait ran out, so no instance could be opened to ask, or Windows gave `windows_error`.
    pub pid: Option<u32>,
    /// When it started, in milliseconds since the Unix epoch (`GetProcessTimes`); absent where this user may not ask.
    pub started_at: Option<f64>,
    /// The error Windows gave where a pipe of that name is there but refused the open or would not name its holder:
    /// `ERROR_ACCESS_DENIED` where its access list refuses this user the right to read its attributes. Absent otherwise.
    pub windows_error: Option<u32>,
}

/// [`pipe_server`], run on a thread of the host's worker pool: the caller's own thread runs on while the lookup waits
/// for a busy pipe. Not in a `headless` build, where `#[napi]` derives nothing (napi-derive `noop`), so
/// `NamedPipeServer` cannot be handed to a host.
#[cfg(not(feature = "headless"))]
#[napi]
pub fn named_pipe_server(
    path: String,
    timeout_ms: u32,
) -> napi::bindgen_prelude::AsyncTask<NamedPipeServerTask> {
    napi::bindgen_prelude::AsyncTask::new(NamedPipeServerTask { path, timeout_ms })
}

/// [`named_pipe_server`]'s work, with what it was asked.
#[cfg(not(feature = "headless"))]
pub struct NamedPipeServerTask {
    path: String,
    timeout_ms: u32,
}

#[cfg(not(feature = "headless"))]
impl napi::Task for NamedPipeServerTask {
    type Output = Option<NamedPipeServer>;
    type JsValue = Option<NamedPipeServer>;

    fn compute(&mut self) -> napi::Result<Option<NamedPipeServer>> {
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || {
            Ok(pipe_server(&self.path, self.timeout_ms))
        })
    }

    fn resolve(
        &mut self,
        _env: napi::Env,
        output: Option<NamedPipeServer>,
    ) -> napi::Result<Option<NamedPipeServer>> {
        Ok(output)
    }
}

/// The process holding the server end of the Windows named pipe `path`, and when it started. Opening the pipe takes
/// one of its instances as a client and sends nothing; while other clients hold every instance it waits for one to free
/// and opens again until `timeout_ms` has passed ([`open_within`]), and then says no pid. None where Windows says no
/// pipe of that name exists, and on every other system, which has no named pipes.
fn pipe_server(path: &str, timeout_ms: u32) -> Option<NamedPipeServer> {
    #[cfg(windows)]
    {
        windows::named_pipe_server(path, timeout_ms)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, timeout_ms);
        None
    }
}

/// What one try to open an instance of a named pipe as a client gave.
#[cfg(any(windows, test))]
#[cfg_attr(test, derive(Debug, PartialEq))]
enum PipeOpen<H> {
    Opened(H),
    /// Other clients hold every instance (`ERROR_PIPE_BUSY`).
    Busy,
    /// No pipe of that name exists (`ERROR_FILE_NOT_FOUND`).
    Missing,
    /// Windows refused the open with this error, `ERROR_ACCESS_DENIED` where this user may not open the pipe.
    Refused(u32),
}

/// Opens an instance of a named pipe (`open`), waiting while other clients hold every instance: `wait` returns once one
/// frees or the milliseconds it is given pass, and the pipe is opened again, since another waiting client can take the
/// instance that freed first; Node's own client loops the same way. `left` gives the milliseconds that remain, and a wait
/// is never given 0, which `WaitNamedPipeW` reads as the wait the pipe's server chose. Busy once none remain; a wait that
/// fails (the time passed, or the pipe is gone) gets one more open, whose answer stands.
#[cfg(any(windows, test))]
fn open_within<H>(
    mut open: impl FnMut() -> PipeOpen<H>,
    mut wait: impl FnMut(u32) -> bool,
    mut left: impl FnMut() -> u32,
) -> PipeOpen<H> {
    loop {
        let opened = open();
        if !matches!(opened, PipeOpen::Busy) {
            return opened;
        }
        let ms = left();
        if ms == 0 {
            return PipeOpen::Busy;
        }
        if !wait(ms) {
            return open();
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::{NamedPipeServer, PipeOpen, open_within};
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, FILETIME, GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_READ_ATTRIBUTES, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// Milliseconds from 1601-01-01 UTC, where a FILETIME counts from, to the Unix epoch.
    const FILETIME_EPOCH_MS: f64 = 11_644_473_600_000.0;

    pub(super) fn named_pipe_server(path: &str, timeout_ms: u32) -> Option<NamedPipeServer> {
        let pipe = match open_pipe(path, timeout_ms) {
            PipeOpen::Opened(pipe) => pipe,
            PipeOpen::Busy => return Some(unnamed(None)),
            PipeOpen::Missing => return None,
            PipeOpen::Refused(error) => return Some(unnamed(Some(error))),
        };
        let mut pid = 0u32;
        // SAFETY: `pipe` is an open handle to an instance of the pipe, and `pid` outlives the call.
        if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0 {
            // SAFETY: reads the calling thread's last-error value, which the call just set.
            return Some(unnamed(Some(unsafe { GetLastError() })));
        }
        Some(NamedPipeServer {
            pid: Some(pid),
            started_at: process_started_at(pid),
            windows_error: None,
        })
    }

    /// What the lookup says of a pipe that is there when Windows names no holder: the error Windows gave
    /// (`windows_error`), or none where other clients kept every instance taken.
    fn unnamed(windows_error: Option<u32>) -> NamedPipeServer {
        NamedPipeServer {
            pid: None,
            started_at: None,
            windows_error,
        }
    }

    /// A client handle to an instance of the pipe `path`, waiting for one to free until `timeout_ms` has passed.
    fn open_pipe(path: &str, timeout_ms: u32) -> PipeOpen<OwnedHandle> {
        let name: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
        let until = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        open_within(
            || open_pipe_once(&name),
            // SAFETY: `name` is NUL-terminated.
            |ms| unsafe { WaitNamedPipeW(name.as_ptr(), ms) } != 0,
            || ms_until(until),
        )
    }

    /// The whole milliseconds left until `until`, short of `u32::MAX`, which `WaitNamedPipeW` reads as no limit.
    fn ms_until(until: Instant) -> u32 {
        let left = until.saturating_duration_since(Instant::now()).as_millis();
        u32::try_from(left).unwrap_or(u32::MAX).min(u32::MAX - 1)
    }

    fn open_pipe_once(name: &[u16]) -> PipeOpen<OwnedHandle> {
        // Only the right to read the pipe's attributes, the least that opens it: the open neither reads nor writes,
        // `GetNamedPipeServerProcessId` needs no more, and every open Node's client makes asks this right too, so a
        // pipe Node could write to opens here. Asking to read as well refuses a pipe whose access list denies this
        // user the read right, or that its server made inbound only.
        // No quality-of-service flags (the 0 after OPEN_EXISTING): this open writes nothing, so Windows lets the
        // process serving the pipe act as no one, which the case below checks. `SECURITY_SQOS_PRESENT` with
        // `SECURITY_IDENTIFICATION` would hand that process this user's identity at the open.
        // SAFETY: `name` is NUL-terminated; no security attributes and no template file.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_READ_ATTRIBUTES,
                0,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            // SAFETY: a handle CreateFileW returned open is owned here alone and closed once, when dropped.
            return PipeOpen::Opened(unsafe { OwnedHandle::from_raw_handle(handle) });
        }
        // SAFETY: reads the calling thread's last-error value, which the open just set.
        match unsafe { GetLastError() } {
            ERROR_PIPE_BUSY => PipeOpen::Busy,
            ERROR_FILE_NOT_FOUND => PipeOpen::Missing,
            error => PipeOpen::Refused(error),
        }
    }

    /// When the process `pid` started, in milliseconds since the Unix epoch; None where this user may not ask.
    fn process_started_at(pid: u32) -> Option<f64> {
        // SAFETY: asks for the least access that reads a process's times; a null handle is checked below.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: the handle OpenProcess returned open is owned here alone and closed once, when dropped.
        let owned = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut times = [FILETIME::default(); 4];
        let [created, exited, kernel, user] = &mut times;
        let process = owned.as_raw_handle();
        // SAFETY: `process` is open with query access, and each FILETIME outlives the call.
        if unsafe { GetProcessTimes(process, created, exited, kernel, user) } == 0 {
            return None;
        }
        let ticks = (u64::from(times[0].dwHighDateTime) << 32) | u64::from(times[0].dwLowDateTime);
        Some((ticks / 10_000) as f64 - FILETIME_EPOCH_MS)
    }

    #[cfg(test)]
    mod tests {
        use super::{PipeOpen, open_pipe_once};
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::Foundation::{
            CloseHandle, ERROR_CANNOT_IMPERSONATE, GetLastError, INVALID_HANDLE_VALUE,
        };
        use windows_sys::Win32::Security::RevertToSelf;
        use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows_sys::Win32::System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, ImpersonateNamedPipeClient, PIPE_TYPE_BYTE,
            PIPE_WAIT,
        };

        /// The process serving a pipe the lookup opens to ask its pid may not act as the process that asks.
        #[test]
        fn the_process_serving_the_pipe_may_not_act_as_the_one_that_asks() {
            let path = format!(r"\\.\pipe\rerail-pipe-server-test-{}", std::process::id());
            let name: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `name` is NUL-terminated; no security attributes.
            let server = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_WAIT,
                    1,
                    512,
                    512,
                    0,
                    null(),
                )
            };
            assert_ne!(server, INVALID_HANDLE_VALUE);
            let client = open_pipe_once(&name);
            assert!(matches!(client, PipeOpen::Opened(_)));
            // SAFETY: `server` is the pipe's server end, open until closed below; the client end above is already
            // connected, so the call returns at once. The last-error value is read right after the call that set it.
            let (acted, error) = unsafe {
                ConnectNamedPipe(server, null_mut());
                (ImpersonateNamedPipeClient(server), GetLastError())
            };
            if acted != 0 {
                // SAFETY: ends the impersonation the call above began on this thread.
                unsafe { RevertToSelf() };
            }
            drop(client);
            // SAFETY: `server` is open and closed here alone.
            unsafe { CloseHandle(server) };
            assert_eq!((acted, error), (0, ERROR_CANNOT_IMPERSONATE));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PipeOpen, open_within};
    use std::collections::VecDeque;

    /// What [`open_within`] did against a stand-in pipe: its answer, the waits it asked for, and how often it opened.
    #[derive(Debug, PartialEq)]
    struct Run {
        answer: PipeOpen<&'static str>,
        waits: Vec<u32>,
        opens: usize,
    }

    /// Runs [`open_within`] where each open gives the next of `tries`, each look at the time gives the next of `lefts`,
    /// and each wait returns `waited`.
    fn run(tries: Vec<PipeOpen<&'static str>>, lefts: Vec<u32>, waited: bool) -> Run {
        let mut tries = VecDeque::from(tries);
        let mut lefts = VecDeque::from(lefts);
        let mut waits = Vec::new();
        let mut opens = 0;
        let answer = open_within(
            || {
                opens += 1;
                tries
                    .pop_front()
                    .expect("no more opens than the test gives")
            },
            |ms| {
                waits.push(ms);
                waited
            },
            || {
                lefts
                    .pop_front()
                    .expect("no more looks at the time than the test gives")
            },
        );
        Run {
            answer,
            waits,
            opens,
        }
    }

    #[test]
    fn opens_again_when_another_waiting_client_takes_the_instance_that_freed() {
        let tries = vec![PipeOpen::Busy, PipeOpen::Busy, PipeOpen::Opened("pipe")];
        let expected = Run {
            answer: PipeOpen::Opened("pipe"),
            waits: vec![30_000, 29_700],
            opens: 3,
        };
        assert_eq!(run(tries, vec![30_000, 29_700], true), expected);
    }

    #[test]
    fn is_busy_once_no_time_is_left_and_never_waits_for_none() {
        let expected = Run {
            answer: PipeOpen::Busy,
            waits: vec![5],
            opens: 2,
        };
        assert_eq!(
            run(vec![PipeOpen::Busy, PipeOpen::Busy], vec![5, 0], true),
            expected
        );
        let expected = Run {
            answer: PipeOpen::Busy,
            waits: vec![],
            opens: 1,
        };
        assert_eq!(run(vec![PipeOpen::Busy], vec![0], true), expected);
    }

    #[test]
    fn opens_once_more_after_a_wait_that_fails_and_takes_its_answer() {
        // The pipe's server closed its last instance during the wait.
        let expected = Run {
            answer: PipeOpen::Missing,
            waits: vec![30_000],
            opens: 2,
        };
        assert_eq!(
            run(vec![PipeOpen::Busy, PipeOpen::Missing], vec![30_000], false),
            expected
        );
    }

    #[test]
    fn gives_the_first_open_that_is_not_busy_without_waiting() {
        let expected = Run {
            answer: PipeOpen::Opened("pipe"),
            waits: vec![],
            opens: 1,
        };
        assert_eq!(run(vec![PipeOpen::Opened("pipe")], vec![], true), expected);
        let expected = Run {
            answer: PipeOpen::Refused(5),
            waits: vec![],
            opens: 1,
        };
        assert_eq!(run(vec![PipeOpen::Refused(5)], vec![], true), expected);
    }
}
