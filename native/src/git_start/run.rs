//! A blocking Git run for `git_start`, as Node's `spawnSync` runs one: no input, stdout and stderr read until the child
//! has exited and both have ended, and at the time limit, once the output passes its limit, or when the wait on the
//! outputs fails, `SIGTERM` and both outputs closed at once, then the exit waited for.

use super::GitRun;
use super::spawn::spawn_watched;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

/// One of the child's outputs, or its exit pipe: the descriptor while open, and what was read from it.
struct Output {
    fd: Option<OwnedFd>,
    bytes: Vec<u8>,
}

impl Output {
    fn of(fd: Option<OwnedFd>) -> Self {
        Output {
            fd,
            bytes: Vec::new(),
        }
    }
}

/// Why the run sent its child `SIGTERM`, if it did: one of its limits, or the errno of a wait that failed.
#[derive(Default)]
struct Ended {
    timed_out: bool,
    past_max_buffer: bool,
    failed: Option<i32>,
}

impl Ended {
    fn killed(&self) -> bool {
        self.timed_out || self.past_max_buffer || self.failed.is_some()
    }
}

/// What a wait on the outputs saw: something to read, the deadline passed, or an error other than an interruption.
enum Look {
    Ready,
    Late,
    Failed(io::Error),
}

pub(super) fn run(
    program: &str,
    args: &[String],
    env: &[String],
    timeout_ms: Option<u32>,
    max_buffer: u32,
) -> io::Result<GitRun> {
    let (pid, mut ours, exited) = spawn_watched(program, args, env, true)?;
    // No input: the child reads the end of it at once, as with spawnSync and no `input`.
    let stderr = ours.pop();
    let stdout = ours.pop();
    drop(ours);
    let mut outputs = [
        Output::of(stdout),
        Output::of(stderr),
        Output::of(Some(exited)),
    ];
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(u64::from(ms)));
    let ended = collect(pid, &mut outputs, deadline, max_buffer as usize);
    let (code, signal) = wait(pid);
    let [stdout, stderr, _] = outputs;
    Ok(GitRun {
        status: code,
        signal,
        stdout: stdout.bytes.into(),
        stderr: stderr.bytes.into(),
        timed_out: ended.timed_out,
        past_max_buffer: ended.past_max_buffer,
        error: ended.failed,
    })
}

/// Reads stdout and stderr (the first two of `outputs`) until both have ended and the exit pipe (the third) has
/// closed. Past `deadline`, once the two hold more than `max_buffer` bytes together, or when the wait on them fails
/// (poll's EINVAL once the process's open-file limit is below the three descriptors), the child gets `SIGTERM` and
/// both outputs close at once, as spawnSync's kill closes its pipes, also when one of its reads fails: a process the
/// child started can hold them for as long as it lives, and a child that goes on writing then gets `EPIPE` in place of
/// a full buffer. The caller then waits for the exit alone. A child that has exited but is not reaped yet takes the
/// signal as a zombie, which no other process can be.
fn collect(
    pid: libc::pid_t,
    outputs: &mut [Output; 3],
    deadline: Option<Instant>,
    max_buffer: usize,
) -> Ended {
    let mut ended = Ended::default();
    while !ended.killed() && outputs.iter().any(|output| output.fd.is_some()) {
        match read_ready(outputs, deadline) {
            Look::Ready => ended.past_max_buffer = printed(outputs) > max_buffer,
            Look::Late => ended.timed_out = true,
            Look::Failed(error) => ended.failed = Some(error.raw_os_error().unwrap_or(libc::EIO)),
        }
    }
    if ended.killed() {
        end(pid, outputs);
    }
    ended
}

/// The bytes stdout and stderr have printed so far.
fn printed(outputs: &[Output; 3]) -> usize {
    outputs[..2].iter().map(|output| output.bytes.len()).sum()
}

/// `SIGTERM` to the child, and its stdout and stderr closed here, as spawnSync's kill does.
fn end(pid: libc::pid_t, outputs: &mut [Output; 3]) {
    // SAFETY: signals this process's own child, which stays unreaped until `wait`, so the pid is still its.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    for output in &mut outputs[..2] {
        output.fd = None;
    }
}

/// Waits for any of `outputs` to be readable until `deadline` (for ever without one), then reads what each has. Late
/// once the deadline has passed, whatever is still to read, as spawnSync's timer ends a run that goes on printing.
fn read_ready(outputs: &mut [Output; 3], deadline: Option<Instant>) -> Look {
    let mut polled: Vec<libc::pollfd> = outputs
        .iter()
        .map(|output| libc::pollfd {
            fd: output.fd.as_ref().map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let seen = poll_until(&mut polled, deadline);
    if matches!(seen, Look::Ready) {
        for (output, poll) in outputs.iter_mut().zip(&polled) {
            if poll.revents != 0 {
                read_once(output);
            }
        }
    }
    seen
}

/// Polls `polled` until one is ready, `deadline` has passed, or the poll fails with an error other than an
/// interruption.
fn poll_until(polled: &mut [libc::pollfd], deadline: Option<Instant>) -> Look {
    loop {
        let Some(timeout) = remaining(deadline) else {
            return Look::Late;
        };
        // SAFETY: `polled` holds one pollfd per output for its whole length.
        let ready =
            unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, timeout) };
        if let Some(seen) = look(ready) {
            return seen;
        }
    }
}

/// What poll's answer `ready` says, read at once after it; none for an interruption, to poll again.
fn look(ready: libc::c_int) -> Option<Look> {
    if ready > 0 {
        return Some(Look::Ready);
    }
    if ready == 0 {
        return Some(Look::Late);
    }
    let error = io::Error::last_os_error();
    (error.kind() != io::ErrorKind::Interrupted).then_some(Look::Failed(error))
}

/// Milliseconds to `deadline` for poll, -1 for none, and none once it has passed.
fn remaining(deadline: Option<Instant>) -> Option<libc::c_int> {
    let Some(at) = deadline else { return Some(-1) };
    let left = at.checked_duration_since(Instant::now())?;
    Some(left.as_millis().min(i32::MAX as u128) as libc::c_int)
}

/// One read of what `output` has; its end, or an error other than an interruption, closes it.
fn read_once(output: &mut Output) {
    let Some(fd) = output.fd.as_ref() else { return };
    let mut buffer = [0u8; 64 * 1024];
    // SAFETY: reads at most the buffer's length into it, from an open descriptor.
    let read = unsafe { libc::read(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
    if read > 0 {
        output.bytes.extend_from_slice(&buffer[..read as usize]);
    } else if read == 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
        output.fd = None;
    }
}

/// Reaps the child, waiting for it to exit: its exit code, or the signal that ended it.
fn wait(pid: libc::pid_t) -> (Option<i32>, Option<i32>) {
    let mut status: libc::c_int = 0;
    loop {
        // SAFETY: waits for and reaps one child of this process.
        let reaped = unsafe { libc::waitpid(pid, &mut status, 0) };
        if reaped == pid {
            break;
        }
        if reaped < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return (None, None);
        }
    }
    if libc::WIFEXITED(status) {
        return (Some(libc::WEXITSTATUS(status)), None);
    }
    (
        None,
        libc::WIFSIGNALED(status).then(|| libc::WTERMSIG(status)),
    )
}

#[cfg(test)]
mod tests {
    use super::run;
    use std::sync::mpsc;
    use std::time::Duration;

    /// What a run answered, in fields a test thread can be sent.
    struct Ran {
        status: Option<i32>,
        signal: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        timed_out: bool,
        past_max_buffer: bool,
        error: Option<i32>,
    }

    /// `script` run under /bin/sh on a thread of its own. A run that never returns fails the test after 20 s, many
    /// times the limits the cases give, where the call would otherwise hang the test binary.
    fn sh(script: &str, timeout_ms: Option<u32>, max_buffer: u32) -> Ran {
        let args = ["-c".to_owned(), script.to_owned()];
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let ran = run(
                "/bin/sh",
                &args,
                &["PATH=/usr/bin:/bin".into()],
                timeout_ms,
                max_buffer,
            );
            let _ = send.send(ran.map(|ran| Ran {
                status: ran.status,
                signal: ran.signal,
                stdout: ran.stdout.to_vec(),
                stderr: ran.stderr.to_vec(),
                timed_out: ran.timed_out,
                past_max_buffer: ran.past_max_buffer,
                error: ran.error,
            }));
        });
        receive
            .recv_timeout(Duration::from_secs(20))
            .expect("the run returned within 20 s")
            .expect("run")
    }

    #[test]
    fn a_run_reads_both_outputs_to_their_ends_and_its_exit_code() {
        // 200,007 bytes in all: exactly the output limit, which a run may reach and not pass.
        let ran = sh(
            "head -c 200000 /dev/zero; echo warned >&2; read nothing || exit 4",
            Some(10_000),
            200_007,
        );
        assert_eq!(
            (
                ran.stdout.len(),
                &ran.stderr[..],
                ran.status,
                ran.signal,
                ran.timed_out,
                ran.past_max_buffer
            ),
            (200_000, &b"warned\n"[..], Some(4), None, false, false)
        );
    }

    #[test]
    fn a_run_past_its_time_limit_or_its_output_limit_ends_with_sigterm() {
        let late = sh("exec sleep 5", Some(100), 1 << 20);
        let long = sh(
            "head -c 3000000 /dev/zero; exec sleep 5",
            Some(10_000),
            1 << 20,
        );
        assert_eq!(
            (
                late.timed_out,
                late.past_max_buffer,
                late.status,
                late.signal
            ),
            (true, false, None, Some(libc::SIGTERM))
        );
        assert_eq!(
            (
                long.timed_out,
                long.past_max_buffer,
                long.status,
                long.signal
            ),
            (false, true, None, Some(libc::SIGTERM))
        );
    }

    /// Runs `script` after starting `sleep 10` in the background, which holds both outputs: how the run ended, and
    /// whether that sleep still ran once the run returned. The sleep is ended either way.
    fn held(
        name: &str,
        script: &str,
        timeout_ms: Option<u32>,
    ) -> (Option<i32>, Option<i32>, bool, bool, bool) {
        let file = format!("rerail-git-run-{name}-{}", std::process::id());
        let file = std::env::temp_dir().join(file);
        let ran = sh(
            &format!("sleep 10 & echo $! > '{}'; {script}", file.display()),
            timeout_ms,
            1 << 20,
        );
        let holder: libc::pid_t = std::fs::read_to_string(&file)
            .expect("the holder's pid")
            .trim()
            .parse()
            .expect("a pid");
        let _ = std::fs::remove_file(&file);
        // SAFETY: signal 0 only asks whether the process is there; SIGKILL then ends this test's own sleep.
        let held = unsafe { libc::kill(holder, 0) } == 0;
        // SAFETY: as above.
        unsafe { libc::kill(holder, libc::SIGKILL) };
        (
            ran.status,
            ran.signal,
            ran.timed_out,
            ran.past_max_buffer,
            held,
        )
    }

    #[test]
    fn a_run_past_its_limits_returns_while_a_process_its_child_started_holds_both_outputs() {
        let term = Some(libc::SIGTERM);
        assert_eq!(
            held("late", "exec sleep 10", Some(100)),
            (None, term, true, false, true)
        );
        assert_eq!(
            held("exited", "exit 0", Some(100)),
            (Some(0), None, true, false, true)
        );
        assert_eq!(
            held(
                "long",
                "head -c 3000000 /dev/zero; exec sleep 10",
                Some(10_000)
            ),
            (None, term, false, true, true)
        );
        let quiet = sh("exec > /dev/null 2>&1; exec sleep 10", Some(100), 1 << 20);
        assert_eq!(
            (quiet.status, quiet.signal, quiet.timed_out),
            (None, term, true)
        );
    }

    /// Set in the process of its own that the case below starts: only there may a run lower the process's open-file
    /// limit, which every other test of this binary shares.
    const OWN_PROCESS: &str = "RERAIL_GIT_RUN_OWN_PROCESS";

    #[test]
    fn a_run_whose_poll_fails_ends_its_child_and_says_why() {
        let case = "git_start::run::tests::a_run_in_a_process_whose_open_file_limit_drops";
        let this_binary = std::env::current_exe().expect("this test binary");
        let ran = std::process::Command::new(this_binary)
            .args([
                case,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(OWN_PROCESS, "1")
            .output()
            .expect("the case in a process of its own");
        let printed = String::from_utf8_lossy(&ran.stdout);
        assert!(
            ran.status.success() && printed.contains("1 passed"),
            "{printed}{}",
            String::from_utf8_lossy(&ran.stderr)
        );
    }

    /// Run by the case above in a process of its own: the child lowers this process's open-file limit to 2, below the
    /// three descriptors the run polls, so each poll after that fails with EINVAL. The child then sleeps until that
    /// failed poll ends it, whenever this process polls again within the run's 3 s limit, which only bounds a run whose
    /// poll does not fail. It exits 97 when prlimit is not on its PATH, and 98 when prlimit fails.
    #[test]
    #[ignore = "run in a process of its own by a_run_whose_poll_fails_ends_its_child_and_says_why"]
    fn a_run_in_a_process_whose_open_file_limit_drops() {
        if std::env::var_os(OWN_PROCESS).is_none() {
            return;
        }
        let ran = sh(
            "command -v prlimit > /dev/null || exit 97; prlimit --nofile=2: --pid \"$PPID\" || exit 98; echo lowered; \
             exec sleep 10",
            Some(3_000),
            1 << 20,
        );
        assert_ne!(
            ran.status,
            Some(97),
            "this case lowers the open-file limit with prlimit (util-linux), which is not in /usr/bin or /bin"
        );
        assert_eq!(
            (ran.status, ran.signal, ran.timed_out, ran.error),
            (None, Some(libc::SIGTERM), false, Some(libc::EINVAL)),
            "{}",
            String::from_utf8_lossy(&ran.stderr)
        );
    }

    #[test]
    fn a_run_ends_on_time_while_its_child_prints_and_closes_its_outputs_at_sigterm() {
        // `cat` prints for as long as it runs, and in 10 ms far less than the 64 MiB output limit: the time limit
        // ends the run.
        let printing = sh("exec cat /dev/zero", Some(10), 64 << 20);
        assert_eq!(
            (
                printing.timed_out,
                printing.past_max_buffer,
                printing.signal
            ),
            (true, false, Some(libc::SIGTERM))
        );
        // The child takes its SIGTERM and then writes 1 MB: with its outputs closed, the write ends at once on EPIPE
        // (exit status 141); with them open, it would fill the buffers and wait, here until `timeout` ends it (124).
        let file = format!("rerail-git-run-writer-{}", std::process::id());
        let file = std::env::temp_dir().join(file);
        let script = format!(
            "sleep 10 & s=$!; trap 'timeout 5 head -c 1000000 /dev/zero; echo $? > \"{}\"; kill $s; exit 0' TERM; wait",
            file.display()
        );
        let writer = sh(&script, Some(100), 1 << 20);
        let wrote = std::fs::read_to_string(&file).unwrap_or_default();
        let _ = std::fs::remove_file(&file);
        assert_eq!(
            (writer.status, writer.timed_out, wrote.trim()),
            (Some(0), true, "141")
        );
    }
}
