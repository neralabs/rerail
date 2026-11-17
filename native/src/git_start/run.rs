//! A blocking Git run for `git_start`, as Node's `spawnSync` runs one: no input, stdout and stderr read to their ends,
//! `SIGTERM` at the time limit or once the output passes its limit, and the exit waited for.

use super::GitRun;
use super::spawn::spawn_child;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

/// One of the child's outputs: its descriptor while open, and what it printed.
struct Output {
    fd: Option<OwnedFd>,
    bytes: Vec<u8>,
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
    let (pid, mut ours) = spawn_child(program, args, env, true)?;
    // No input: the child reads the end of it at once, as with spawnSync and no `input`.
    let stderr = ours.pop();
    let stdout = ours.pop();
    drop(ours);
    let mut outputs = [
        Output {
            fd: stdout,
            bytes: Vec::new(),
        },
        Output {
            fd: stderr,
            bytes: Vec::new(),
        },
    ];
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(u64::from(ms)));
    let ended = collect(pid, &mut outputs, deadline, max_buffer as usize);
    let (code, signal) = wait(pid);
    let [stdout, stderr] = outputs;
    Ok(GitRun {
        status: code,
        signal,
        stdout: stdout.bytes.into(),
        stderr: stderr.bytes.into(),
        timed_out: ended.timed_out,
        past_max_buffer: ended.past_max_buffer,
    })
}

/// Reads both outputs to their ends. Past `deadline`, or once they hold more than `max_buffer` bytes together, the
/// child gets `SIGTERM` once and the reads go on to the end, as spawnSync's do.
fn collect(
    pid: libc::pid_t,
    outputs: &mut [Output; 2],
    deadline: Option<Instant>,
    max_buffer: usize,
) -> Ended {
    let mut ended = Ended::default();
    while outputs.iter().any(|output| output.fd.is_some()) {
        let wait = if ended.killed() { None } else { deadline };
        if !read_ready(outputs, wait) {
            ended.timed_out = true;
            terminate(pid);
            continue;
        }
        if !ended.killed()
            && outputs
                .iter()
                .map(|output| output.bytes.len())
                .sum::<usize>()
                > max_buffer
        {
            ended.past_max_buffer = true;
            terminate(pid);
        }
    }
    ended
}

fn terminate(pid: libc::pid_t) {
    // SAFETY: signals this process's own child, which stays unreaped until `wait`, so the pid is still its.
    unsafe { libc::kill(pid, libc::SIGTERM) };
}

/// Waits for either output to be readable until `deadline` (for ever without one), then reads what each has. False
/// once the deadline passed with nothing to read.
fn read_ready(outputs: &mut [Output; 2], deadline: Option<Instant>) -> bool {
    let mut polled: Vec<libc::pollfd> = outputs
        .iter()
        .map(|output| libc::pollfd {
            fd: output.fd.as_ref().map_or(-1, AsRawFd::as_raw_fd),
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let ready = loop {
        // SAFETY: `polled` holds one pollfd per output for its whole length.
        let ready = unsafe {
            libc::poll(
                polled.as_mut_ptr(),
                polled.len() as libc::nfds_t,
                remaining(deadline),
            )
        };
        if ready >= 0 || io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break ready;
        }
    };
    if ready == 0 {
        return false;
    }
    for (output, poll) in outputs.iter_mut().zip(&polled) {
        if poll.revents != 0 {
            read_once(output);
        }
    }
    true
}

/// Milliseconds to `deadline` for poll, -1 for none.
fn remaining(deadline: Option<Instant>) -> libc::c_int {
    deadline.map_or(-1, |at| {
        at.saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as libc::c_int
    })
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
        let ran = sh(
            "head -c 200000 /dev/zero; echo warned >&2; read nothing || exit 4",
            Some(10_000),
            1 << 20,
        );
        assert_eq!(
            (
                ran.stdout.len(),
                &ran.stderr[..],
                ran.status,
                ran.signal,
                ran.timed_out
            ),
            (200_000, &b"warned\n"[..], Some(4), None, false)
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
}
