//! A blocking Git run for `git_start`, as Node's `spawnSync` runs one: no input, stdout and stderr read until the child
//! has exited and both have ended, and at the time limit or once the output passes its limit, `SIGTERM` and both
//! outputs closed at once, then the exit waited for.

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

/// Why the run sent its child `SIGTERM`, if it did.
#[derive(Default)]
struct Ended {
    timed_out: bool,
    past_max_buffer: bool,
}

impl Ended {
    fn killed(&self) -> bool {
        self.timed_out || self.past_max_buffer
    }
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
    })
}

/// Reads stdout and stderr (the first two of `outputs`) until both have ended and the exit pipe (the third) has
/// closed. Past `deadline`, or once the two hold more than `max_buffer` bytes together, the child gets `SIGTERM` and
/// both outputs close at once, as spawnSync's kill closes its pipes: a process the child started can hold them for as
/// long as it lives, and a child that goes on writing then gets `EPIPE` in place of a full buffer. The caller then
/// waits for the exit alone. A child that has exited but is not reaped yet takes the signal as a zombie, which no other
/// process can be.
fn collect(
    pid: libc::pid_t,
    outputs: &mut [Output; 3],
    deadline: Option<Instant>,
    max_buffer: usize,
) -> Ended {
    let mut ended = Ended::default();
    while !ended.killed() && outputs.iter().any(|output| output.fd.is_some()) {
        if read_ready(outputs, deadline) {
            ended.past_max_buffer = printed(outputs) > max_buffer;
        } else {
            ended.timed_out = true;
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

/// Waits for any of `outputs` to be readable until `deadline` (for ever without one), then reads what each has. False
/// once the deadline has passed, whatever is still to read, as spawnSync's timer ends a run that goes on printing.
fn read_ready(outputs: &mut [Output; 3], deadline: Option<Instant>) -> bool {
    let mut polled: Vec<libc::pollfd> = outputs
        .iter()
        .map(|output| libc::pollfd {
            fd: output.fd.as_ref().map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    if !poll_until(&mut polled, deadline) {
        return false;
    }
    for (output, poll) in outputs.iter_mut().zip(&polled) {
        if poll.revents != 0 {
            read_once(output);
        }
    }
    true
}

/// Polls `polled` until one is ready, or an error other than an interruption (true), or `deadline` has passed
/// (false).
fn poll_until(polled: &mut [libc::pollfd], deadline: Option<Instant>) -> bool {
    loop {
        let Some(timeout) = remaining(deadline) else {
            return false;
        };
        // SAFETY: `polled` holds one pollfd per output for its whole length.
        let ready =
            unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, timeout) };
        if ready != -1 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return ready != 0;
        }
    }
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

    fn sh(script: &str, timeout_ms: Option<u32>, max_buffer: u32) -> super::GitRun {
        run(
            "/bin/sh",
            &["-c".into(), script.into()],
            &["PATH=/usr/bin:/bin".into()],
            timeout_ms,
            max_buffer,
        )
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
