# 0386. Device work belongs to its job, and is committed before any wait

Status: built (2026-11-17). Work queue Q177 (Q52 until the hub merge). Builds on 0100 §2 (one queue for the process), 0113 §3 (a level's
strata wait for one device wave), 0123 §6 (a component's launches handed back to the run's wave) and Q48 (a batch
run apart commits the thread's wave first). Agrees with the ledger's lesson that a shared resource is held only
across work that cannot wait on its other holders.

## Intro

The engine runs relational work on Metal through one command queue for the whole process. Metal lets a queue hold a
bounded number of command buffers that have not completed: 64 by default. A thread that asks for one more waits
until one completes. A command buffer that is never committed never completes.

The device joins kept their open wave (a command buffer and its encoder, shared by a level's strata) in a
thread-local. Its flush was deferred to the next `wave::flush` on the same thread. A component's job flushed it only
when one of its members waited on it. So a job whose strata were all run apart, or that started inside another job
on the same thread (a wait runs the pool's jobs), left an uncommitted command buffer on a pool thread that then went
idle. On the 300-file, 317-rule corpus with every thread, the 64 ran out: six threads waited in
`-[MTLCommandBuffer initWithQueue:…]` for buffers that only waiting threads could commit, and the run stopped after
4.6 minutes of CPU in 29 minutes of wall. With 4 threads it finished, and with a queue of 4,096 every step finished.

The same thread-local state had a second fault. A job nested on a thread whose wave was open encoded its strata
into that wave, and could not flush it (another job had opened it). Its members came back parked, were resumed as a
job on another thread, and found their tickets in the first thread's thread-locals: "a stratum's device joins were
not submitted". 20 strata of that run fell back to the CPU after their GPU work was encoded.

## Decision

1. **No open command buffer outlives the call that encoded it.** `setfix::device::submit` commits the wave's command
   buffer before it returns. The wave keeps its strata's committed buffers, and its flush still waits once for all
   of them. A thread holds an uncommitted buffer only while it encodes, one at a time, so the queue always drains:
   committed work completes without the host.

   ```text
   before:  submit A ─ encode ┐          submit B ─ encode ┐            flush: commit ─ wait
                              └── open across the job's end, its waits, the thread's idle time ──┘
   after:   submit A ─ encode ─ commit   submit B ─ encode ─ commit     flush: wait (once)
   ```

2. **Device work belongs to its job, not its thread.** The wave's per-thread state (launches queued, submissions
   deferred, and the open submission) is a scope that every pool job enters and leaves
   (`rerail_accel::exec::set_job_scope`, entered in `as_job` and in a stream's `State::run`; the scope is
   `lanes::wave::Scope`). Entering saves the outer job's state and starts empty. Leaving runs what the job deferred
   and did not flush, which finishes its open submission on the thread that encoded it, hands what it queued and
   did not take to the outer wave, and restores the outer's state. A job's strata never join a wave another job
   owns, and no submission is left on a thread that may go idle.

3. **A wait holds no uncommitted command buffer.** The accel crate counts the command buffers each thread opened and
   not committed (`command_opened`, `command_committed`, `uncommitted_here`; the device joins' `Batch` counts its
   own). The wait primitives (`help_until_as`, `Bell::wait_as`, `Stream::wait`) assert in debug builds that the
   waiting thread holds none.

## What stays

- One queue for the process (0100 §2), so work committed first runs first. Q48's order holds: a direct batch commits
  the wave first, which is now committed already.
- A level's strata still wait once for their wave (0113 §3). A wave is now several command buffers, one per
  stratum's encoding, instead of one, which is a few microseconds of commit each.
- Launches queued for the run's wave are still handed back by a job (0123 §6): the scope hands untaken ones to the
  outer wave instead of dropping them.

## Tests

- `lanes::wave_tests::a_jobs_wave_scope_keeps_the_outer_waves_apart`: entering and leaving a scope, without Metal.
- `setfix::device::tests::a_stratum_waiting_for_its_wave_leaves_no_command_buffer_open`.
- `setfix::device::tests::a_jobs_wave_is_its_own_and_ends_with_it`: a job on a thread with a wave open opens its
  own, its stratum resumes with nothing flushed, and the outer wave holds the outer stratum alone. It fails before
  the change: the job's stratum joined the outer wave.
- `rerail-accel` `exec::job_scope_tests`: the scope around every job, also on a panic; the per-thread count.
