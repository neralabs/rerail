//! Starting Git from a large process without copying it (Linux).
//!
//! Node's `child_process` starts every child through libuv, which on Linux calls `fork()`: the kernel copies the
//! caller's page tables, marks every written page copy-on-write, and the child tears that copy down again when it
//! runs `exec`. libuv waits for the `exec` on the calling thread, so in the daemon each Git start holds its event loop
//! for a time that grows with its resident memory (about 30 ms per GB measured), and every page the daemon writes
//! afterwards takes a fault. glibc's `posix_spawn` starts the child with `clone(CLONE_VM | CLONE_VFORK)`: the child
//! runs in the caller's memory until `exec`, so nothing is copied, whatever the caller's size.
//!
//! `start_git` gives the child what libuv gives it: Unix socket pairs of 64 KiB buffers for its stdio (blocking on the
//! child's side), the null device for an ignored stderr, signals 1 to 31 at their default action and an empty signal
//! mask, the caller's working folder and no other descriptor (every descriptor this process opens is close-on-exec).
//! A thread waits for the child to exit without reaping it (`WNOWAIT`) and then closes the `exited` pipe; the caller
//! reaps it with `reap_git` once it has read that close, on its own thread. Until then the child stays a zombie, so its
//! pid names no other process and a kill sent before the reap reaches this child or nothing.

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

/// Starts `program` (an absolute path) with `args` (argv from its second entry) and `env` (`NAME=value` entries),
/// stdin and stdout on socket pairs, and stderr on one when `pipe_stderr`, else on the null device.
#[napi]
pub fn start_git(program: String, args: Vec<String>, env: Vec<String>, pipe_stderr: bool) -> Result<StartedGit> {
    #[cfg(target_os = "linux")]
    {
        linux::start(&program, &args, &env, pipe_stderr).map_err(|error| failed("startGit", &error))
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
        linux::reap(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        GitExit { code: None, signal: None }
    }
}

#[cfg(target_os = "linux")]
fn failed(call: &str, error: &std::io::Error) -> Error {
    Error::new(Status::GenericFailure, format!("{call}: {error}"))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{GitExit, StartedGit};
    use std::ffi::CString;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
    use std::ptr::{null, null_mut};
    use std::sync::mpsc;

    /// libuv's buffer size for a child's stdio sockets (`uv__process_init_stdio`).
    const STDIO_BUFFER: libc::c_int = 64 * 1024;

    fn check(result: libc::c_int) -> io::Result<()> {
        if result == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(result)) }
    }

    fn last(result: libc::c_int) -> io::Result<libc::c_int> {
        if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
    }

    fn c_string(text: &str) -> io::Result<CString> {
        CString::new(text).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a string holds a NUL byte"))
    }

    /// A connected pair: this process's end first, the child's second, both close-on-exec, each with libuv's buffers.
    fn socket_pair() -> io::Result<(OwnedFd, OwnedFd)> {
        let mut fds: [RawFd; 2] = [-1; 2];
        // SAFETY: `fds` has room for the two descriptors socketpair writes.
        last(unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) })?;
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
                check(libc::posix_spawnattr_setsigdefault(&mut self.attributes, &defaults))?;
                check(libc::posix_spawnattr_setsigmask(&mut self.attributes, &mask))?;
                let flags = libc::POSIX_SPAWN_SETSIGDEF | libc::POSIX_SPAWN_SETSIGMASK;
                check(libc::posix_spawnattr_setflags(&mut self.attributes, flags as libc::c_short))
            }
        }

        /// The child's descriptor `target` is `fd`. `fd` is 3 or more (`above_stdio`), so the duplicate is a new
        /// descriptor without close-on-exec.
        fn give(&mut self, fd: &OwnedFd, target: RawFd) -> io::Result<()> {
            // SAFETY: the actions were initialised in `new`; `fd` stays open until after the spawn.
            check(unsafe { libc::posix_spawn_file_actions_adddup2(&mut self.actions, fd.as_raw_fd(), target) })
        }

        fn null_stderr(&mut self) -> io::Result<()> {
            let null_device = c_string("/dev/null")?;
            // SAFETY: the actions copy the path; the actions were initialised in `new`.
            check(unsafe { libc::posix_spawn_file_actions_addopen(&mut self.actions, 2, null_device.as_ptr(), libc::O_RDWR, 0) })
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
        items.iter().map(|item| item.as_ptr().cast_mut()).chain(Some(null_mut())).collect()
    }

    /// The child's ends of its stdio, given to it as 0, 1 and 2; this process's ends, returned.
    fn stdio(spawn: &mut Spawn, pipe_stderr: bool) -> io::Result<(Vec<OwnedFd>, Vec<OwnedFd>)> {
        let mut ours = Vec::new();
        let mut theirs = Vec::new();
        for target in 0..if pipe_stderr { 3 } else { 2 } {
            let (mine, child) = socket_pair()?;
            let child = above_stdio(child)?;
            spawn.give(&child, target)?;
            ours.push(mine);
            theirs.push(child);
        }
        if !pipe_stderr {
            spawn.null_stderr()?;
        }
        Ok((ours, theirs))
    }

    /// A thread that waits for the pid it is sent to exit, leaves it unreaped, and then drops `write`, which closes
    /// the exit pipe. Started before the child, so a thread that cannot start leaves no child behind.
    fn exit_watch(write: OwnedFd) -> io::Result<mpsc::Sender<libc::pid_t>> {
        let (send, receive) = mpsc::channel::<libc::pid_t>();
        std::thread::Builder::new().name("rerail-git-exit".into()).stack_size(64 * 1024).spawn(move || {
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
            let result = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOWAIT) };
            if result == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return;
            }
        }
    }

    pub(super) fn start(program: &str, args: &[String], env: &[String], pipe_stderr: bool) -> io::Result<StartedGit> {
        let argv: Vec<CString> = std::iter::once(program).chain(args.iter().map(String::as_str)).map(c_string).collect::<io::Result<_>>()?;
        let envp: Vec<CString> = env.iter().map(|entry| c_string(entry)).collect::<io::Result<_>>()?;
        let path = c_string(program)?;
        let mut spawn = Spawn::new()?;
        let (ours, theirs) = stdio(&mut spawn, pipe_stderr)?;
        let (exited, write) = exit_pipe()?;
        let watch = exit_watch(write)?;
        let mut pid: libc::pid_t = 0;
        // SAFETY: every pointer is valid for the call: the path, NULL-ended argv and envp over live CStrings, and the
        // initialised actions and attributes.
        let result = unsafe {
            libc::posix_spawn(&mut pid, path.as_ptr(), &spawn.actions, &spawn.attributes, c_array(&argv).as_ptr(), c_array(&envp).as_ptr())
        };
        drop(theirs);
        check(result)?;
        let _ = watch.send(pid);
        let mut fds = ours.into_iter().map(IntoRawFd::into_raw_fd);
        Ok(StartedGit {
            pid,
            stdin: fds.next().unwrap_or(-1),
            stdout: fds.next().unwrap_or(-1),
            stderr: fds.next().unwrap_or(-1),
            exited: exited.into_raw_fd(),
        })
    }

    pub(super) fn reap(pid: i32) -> GitExit {
        let mut status: libc::c_int = 0;
        // SAFETY: reaps one child of this process, without blocking.
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if reaped != pid {
            return GitExit { code: None, signal: None };
        }
        if libc::WIFEXITED(status) {
            return GitExit { code: Some(libc::WEXITSTATUS(status)), signal: None };
        }
        GitExit { code: None, signal: libc::WIFSIGNALED(status).then(|| libc::WTERMSIG(status)) }
    }

    #[cfg(test)]
    mod tests {
        use super::start;
        use std::io::{Read, Write};
        use std::os::fd::FromRawFd;
        use std::os::unix::net::UnixStream;

        #[test]
        fn a_child_reads_its_input_writes_its_output_and_is_reaped_after_the_exit_pipe_closes() {
            let started = start("/bin/sh", &["-c".into(), "read line; echo \"got $line\"; exit 3".into()], &["PATH=/usr/bin:/bin".into()], false).expect("start");
            // SAFETY: each descriptor start returned is open and ours to own.
            let (mut stdin, mut stdout, mut exited) = unsafe {
                (UnixStream::from_raw_fd(started.stdin), UnixStream::from_raw_fd(started.stdout), std::fs::File::from_raw_fd(started.exited))
            };
            stdin.write_all(b"one\n").expect("write");
            drop(stdin);
            let mut out = String::new();
            stdout.read_to_string(&mut out).expect("read");
            let mut nothing = Vec::new();
            exited.read_to_end(&mut nothing).expect("exit pipe");
            let exit = super::reap(started.pid);
            assert_eq!((out.as_str(), exit.code, exit.signal, started.stderr), ("got one\n", Some(3), None, -1));
        }
    }
}
