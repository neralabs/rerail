//! posix_spawn for `git_start`: the child gets its stdio as libuv gives it, and nothing of this process is copied.

use super::{GitExit, StartedGit};
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::ptr::null_mut;
use std::sync::mpsc;

/// libuv's buffer size for a child's stdio sockets (`uv__process_init_stdio`).
const STDIO_BUFFER: libc::c_int = 64 * 1024;

fn check(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

fn last(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn c_string(text: &str) -> io::Result<CString> {
    CString::new(text)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a string holds a NUL byte"))
}

/// A connected pair: this process's end first, the child's second, both close-on-exec, each with libuv's buffers.
fn socket_pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds: [RawFd; 2] = [-1; 2];
    // SAFETY: `fds` has room for the two descriptors socketpair writes.
    last(unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    })?;
    // SAFETY: socketpair succeeded, so both descriptors are open and owned here alone.
    let pair = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [pair.0.as_raw_fd(), pair.1.as_raw_fd()] {
        for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
            let size: *const libc::c_int = &STDIO_BUFFER;
            // SAFETY: `size` points at a c_int for the length given.
            last(unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, option, size.cast(), 4) })?;
        }
    }
    Ok(pair)
}

/// A close-on-exec pipe: its read end, then its write end.
fn exit_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds: [RawFd; 2] = [-1; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    last(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    // SAFETY: pipe2 succeeded, so both descriptors are open and owned here alone.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// What the child gets as its descriptors 0, 1 and 2, and how it starts. Dropped after the spawn.
struct Spawn {
    actions: libc::posix_spawn_file_actions_t,
    attributes: libc::posix_spawnattr_t,
}

impl Drop for Spawn {
    fn drop(&mut self) {
        // SAFETY: both were initialised in `Spawn::new`, and are destroyed once.
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.actions);
            libc::posix_spawnattr_destroy(&mut self.attributes);
        }
    }
}

impl Spawn {
    fn new() -> io::Result<Box<Self>> {
        // SAFETY: zeroed storage is what posix_spawn_*_init expect to initialise.
        let mut spawn: Box<Self> = Box::new(unsafe { std::mem::zeroed() });
        // SAFETY: initialises the storage above.
        check(unsafe { libc::posix_spawn_file_actions_init(&mut spawn.actions) })?;
        // SAFETY: as above; on failure the actions are destroyed with `spawn`.
        check(unsafe { libc::posix_spawnattr_init(&mut spawn.attributes) })?;
        spawn.signals()?;
        Ok(spawn)
    }

    /// Signals 1 to 31 at their default action and an empty mask, as libuv's child sets them before `exec`.
    fn signals(&mut self) -> io::Result<()> {
        // SAFETY: each sigset_t is initialised before use; the attributes were initialised in `new`.
        unsafe {
            let mut defaults: libc::sigset_t = std::mem::zeroed();
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut defaults);
            libc::sigemptyset(&mut mask);
            for signal in (1..32).filter(|&s| s != libc::SIGKILL && s != libc::SIGSTOP) {
                libc::sigaddset(&mut defaults, signal);
            }
            check(libc::posix_spawnattr_setsigdefault(
                &mut self.attributes,
                &defaults,
            ))?;
            check(libc::posix_spawnattr_setsigmask(
                &mut self.attributes,
                &mask,
            ))?;
            let flags = libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK;
            check(libc::posix_spawnattr_setflags(
                &mut self.attributes,
                flags as libc::c_short,
            ))
        }
    }

    /// The child's descriptor `target` is `fd`. `fd` is 3 or more (`above_stdio`), so the duplicate is a new
    /// descriptor without close-on-exec.
    fn give(&mut self, fd: &OwnedFd, target: RawFd) -> io::Result<()> {
        // SAFETY: the actions were initialised in `new`; `fd` stays open until after the spawn.
        check(unsafe {
            libc::posix_spawn_file_actions_adddup2(&mut self.actions, fd.as_raw_fd(), target)
        })
    }

    fn null_stderr(&mut self) -> io::Result<()> {
        let null_device = c_string("/dev/null")?;
        // SAFETY: the actions copy the path; the actions were initialised in `new`.
        check(unsafe {
            libc::posix_spawn_file_actions_addopen(
                &mut self.actions,
                2,
                null_device.as_ptr(),
                libc::O_RDWR,
                0,
            )
        })
    }
}

/// `fd` moved to a descriptor of 3 or more, so giving it to the child as 0, 1 or 2 is never a duplicate onto itself.
fn above_stdio(fd: OwnedFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() > 2 {
        return Ok(fd);
    }
    // SAFETY: duplicates an open descriptor; the copy is owned here alone.
    let moved = last(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) })?;
    // SAFETY: fcntl succeeded, so `moved` is open.
    Ok(unsafe { OwnedFd::from_raw_fd(moved) })
}

/// argv or envp: the strings, and a NULL-ended array of pointers into them.
fn c_array(items: &[CString]) -> Vec<*mut libc::c_char> {
    items
        .iter()
        .map(|item| item.as_ptr().cast_mut())
        .chain(Some(null_mut()))
        .collect()
}

/// One socket pair for the child's descriptor `target`: this process's end, and the child's, given to it as `target`.
fn stdio_pair(spawn: &mut Spawn, target: RawFd) -> io::Result<(OwnedFd, OwnedFd)> {
    let (mine, child) = socket_pair()?;
    let child = above_stdio(child)?;
    spawn.give(&child, target)?;
    Ok((mine, child))
}

/// The child's ends of its stdio, given to it as 0, 1 and 2; this process's ends, returned.
fn stdio(spawn: &mut Spawn, pipe_stderr: bool) -> io::Result<(Vec<OwnedFd>, Vec<OwnedFd>)> {
    let piped: RawFd = if pipe_stderr { 3 } else { 2 };
    let (ours, theirs) = (0..piped)
        .map(|target| stdio_pair(spawn, target))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .unzip();
    if !pipe_stderr {
        spawn.null_stderr()?;
    }
    Ok((ours, theirs))
}

/// Runs `f` on the exit watch's thread: a service of the engine's executor (it waits in the OS, so it is no job of the
/// pool's), named, on the stack `service` gives when it is asked for none: std's for any thread (2 MiB, or
/// `RUST_MIN_STACK`), of which it touches only the pages it uses. Its own work is small, but jemalloc's on the same
/// thread is not when jemalloc is built unoptimised: its malloc slow path takes 35 to 40 KiB of stack in one frame, and
/// setting up the thread's cache at its first allocation, which std makes as the thread starts, ran past a 64 KiB stack.
fn watch_thread<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> io::Result<std::thread::JoinHandle<T>> {
    rerail_accel::exec::service("rerail-git-exit", None, f)
}

/// A thread that waits for the pid it is sent to exit, leaves it unreaped, and then drops `write`, which closes
/// the exit pipe. Started before the child, so a thread that cannot start leaves no child behind. A wait that fails
/// closes the pipe too, while the child may still run: `reap` then says it runs, and the caller reaps it later.
fn exit_watch(write: OwnedFd) -> io::Result<mpsc::Sender<libc::pid_t>> {
    let (send, receive) = mpsc::channel::<libc::pid_t>();
    watch_thread(move || {
        if let Ok(pid) = receive.recv() {
            wait_unreaped(pid);
        }
        drop(write);
    })?;
    Ok(send)
}

fn wait_unreaped(pid: libc::pid_t) {
    // SAFETY: zeroed siginfo_t is a valid out-parameter for waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: waits on one child of this process without reaping it.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if result == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return;
        }
    }
}

/// `program` as a path, `program` and `args` as argv, and `env`: each as C strings, refused when one holds a NUL byte.
fn c_strings(
    program: &str,
    args: &[String],
    env: &[String],
) -> io::Result<(CString, Vec<CString>, Vec<CString>)> {
    let argv = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(c_string)
        .collect::<io::Result<_>>()?;
    let envp = env
        .iter()
        .map(|entry| c_string(entry))
        .collect::<io::Result<_>>()?;
    Ok((c_string(program)?, argv, envp))
}

/// `program` started with `args` and `env`, its stdin and stdout (and stderr when `pipe_stderr`) on socket pairs:
/// its pid and this process's ends, in that order. The child's ends are closed here once it has them.
fn spawn_child(
    program: &str,
    args: &[String],
    env: &[String],
    pipe_stderr: bool,
) -> io::Result<(libc::pid_t, Vec<OwnedFd>)> {
    let (path, argv, envp) = c_strings(program, args, env)?;
    let mut spawn = Spawn::new()?;
    let (ours, theirs) = stdio(&mut spawn, pipe_stderr)?;
    let mut pid: libc::pid_t = 0;
    // SAFETY: every pointer is valid for the call: the path, NULL-ended argv and envp over live CStrings, and the
    // initialised actions and attributes.
    let result = unsafe {
        libc::posix_spawn(
            &mut pid,
            path.as_ptr(),
            &spawn.actions,
            &spawn.attributes,
            c_array(&argv).as_ptr(),
            c_array(&envp).as_ptr(),
        )
    };
    drop(theirs);
    check(result)?;
    Ok((pid, ours))
}

/// `spawn_child`'s pid and ends, and the read end of a pipe whose write end only the exit watch holds: it closes once
/// the child has exited, whatever a process the child started holds, and the child stays unreaped until then.
pub(super) fn spawn_watched(
    program: &str,
    args: &[String],
    env: &[String],
    pipe_stderr: bool,
) -> io::Result<(libc::pid_t, Vec<OwnedFd>, OwnedFd)> {
    let (exited, write) = exit_pipe()?;
    let watch = exit_watch(write)?;
    let (pid, ours) = spawn_child(program, args, env, pipe_stderr)?;
    let _ = watch.send(pid);
    Ok((pid, ours, exited))
}

pub(super) fn start(
    program: &str,
    args: &[String],
    env: &[String],
    pipe_stderr: bool,
) -> io::Result<StartedGit> {
    let (pid, ours, exited) = spawn_watched(program, args, env, pipe_stderr)?;
    let mut fds = ours.into_iter().map(IntoRawFd::into_raw_fd);
    Ok(StartedGit {
        pid,
        stdin: fds.next().unwrap_or(-1),
        stdout: fds.next().unwrap_or(-1),
        stderr: fds.next().unwrap_or(-1),
        exited: exited.into_raw_fd(),
    })
}

/// Reaps `pid` if it has exited, without waiting: how it ended; or nothing, with `running` when it has not exited yet
/// (waitpid's 0) and without when it is no child of this process's to reap (ECHILD).
pub(super) fn reap(pid: i32) -> GitExit {
    let mut status: libc::c_int = 0;
    // SAFETY: reaps one child of this process, without blocking.
    let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if reaped != pid {
        return GitExit {
            code: None,
            signal: None,
            running: reaped == 0,
        };
    }
    if libc::WIFEXITED(status) {
        return GitExit {
            code: Some(libc::WEXITSTATUS(status)),
            signal: None,
            running: false,
        };
    }
    GitExit {
        code: None,
        signal: libc::WIFSIGNALED(status).then(|| libc::WTERMSIG(status)),
        running: false,
    }
}

#[cfg(test)]
mod tests {
    use super::start;
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn a_child_reads_its_input_writes_its_output_and_is_reaped_after_the_exit_pipe_closes() {
        let started = start(
            "/bin/sh",
            &["-c".into(), "read line; echo \"got $line\"; exit 3".into()],
            &["PATH=/usr/bin:/bin".into()],
            false,
        )
        .expect("start");
        // SAFETY: each descriptor start returned is open and ours to own.
        let (mut stdin, mut stdout, mut exited) = unsafe {
            (
                UnixStream::from_raw_fd(started.stdin),
                UnixStream::from_raw_fd(started.stdout),
                std::fs::File::from_raw_fd(started.exited),
            )
        };
        stdin.write_all(b"one\n").expect("write");
        drop(stdin);
        let mut out = String::new();
        stdout.read_to_string(&mut out).expect("read");
        let mut nothing = Vec::new();
        exited.read_to_end(&mut nothing).expect("exit pipe");
        let exit = super::reap(started.pid);
        assert_eq!(
            (out.as_str(), exit.code, exit.signal, started.stderr),
            ("got one\n", Some(3), None, -1)
        );
    }

    #[test]
    fn a_child_that_has_not_exited_is_left_unreaped_and_said_to_run() {
        let started = start(
            "/bin/sh",
            &["-c".into(), "read line; exit 3".into()],
            &["PATH=/usr/bin:/bin".into()],
            false,
        )
        .expect("start");
        let running = super::reap(started.pid);
        // SAFETY: each descriptor start returned is open and ours to own.
        let (stdin, stdout, mut exited) = unsafe {
            (
                UnixStream::from_raw_fd(started.stdin),
                UnixStream::from_raw_fd(started.stdout),
                std::fs::File::from_raw_fd(started.exited),
            )
        };
        drop((stdin, stdout));
        let mut nothing = Vec::new();
        exited.read_to_end(&mut nothing).expect("exit pipe");
        let (exit, gone) = (super::reap(started.pid), super::reap(started.pid));
        assert_eq!(
            [
                (running.running, running.code),
                (exit.running, exit.code),
                (gone.running, gone.code)
            ],
            [(true, None), (false, Some(3)), (false, None)]
        );
    }

    /// The size of the stack the calling thread runs on, as glibc reports it, without its guard page.
    fn own_stack_size() -> usize {
        // SAFETY: pthread_getattr_np initialises `attr` before it is read, and it is destroyed once.
        unsafe {
            let mut attr: libc::pthread_attr_t = std::mem::zeroed();
            let this = libc::pthread_self();
            assert_eq!(libc::pthread_getattr_np(this, &mut attr), 0);
            let mut size = 0;
            assert_eq!(libc::pthread_attr_getstacksize(&attr, &mut size), 0);
            libc::pthread_attr_destroy(&mut attr);
            size
        }
    }

    #[test]
    fn the_exit_watch_runs_on_the_stack_std_gives_any_thread() {
        let (watch, any) = (
            super::watch_thread(own_stack_size)
                .expect("the exit watch's thread")
                .join()
                .expect("its stack size"),
            std::thread::Builder::new()
                .spawn(own_stack_size)
                .expect("a thread")
                .join()
                .expect("its stack size"),
        );
        assert!(
            watch >= any,
            "the exit watch's stack is {watch} bytes, std's default {any}"
        );
    }
}
