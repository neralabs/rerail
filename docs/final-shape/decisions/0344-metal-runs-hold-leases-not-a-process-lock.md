# 0344. Metal runs hold leases, not a process-wide lock; a GPU program runs beside the one before

Status: accepted (2026-11-17)

## Intro

A Metal run (the matcher wave's label passes, a packed program batch, an anchor scan) took a process-wide mutex,
`metal_execution_guard`, for its whole run: encoding, commit, the wait for the GPU, and reading back. It came from
Tempo with the launch budget. So no run could encode while another's command buffers ran, and the GPU lane could not
overlap one wave's host part with another's device part (work queue Q50).

The user asked: "do we really need a process-wide Metal execution lock? Shouldn't we take leases on the resources
instead?" We do not need it.

## What the lock protected

- **The launch budget** (`launch::LaunchScope`) is thread-local. A nested run shares the outer one's budget on the
  same thread. It needs no lock.
- **The device memory a run writes** is already its own:
  - the label passes' kept buffers are taken out of their slot for the run and put back at its end;
  - the packed program backend's workspaces come from a mutex-guarded pool, taken and returned;
  - every other buffer is made by the run.
- **What runs share** (compiled pipelines, libraries, the JIT cache) is behind its own mutex.
- **The anchor scan** wrote its own hit buffers through `&self`. It now takes `&mut self`, so a run holds the scan
  alone.
- Most Metal paths (rows, sort, nearest, set steps, block programs, the buffer pool) never took the lock.

## Decision

1. `metal_execution_guard` is gone. `rerail_metal::evaluation_scope()` is the evaluation's launch scope alone, and
   resources are leases: a run holds the memory it writes, taken from its owner and given back.
2. The GPU backend's `start` runs a batch of programs on a pool of its own and returns at once
   (`exec::lane_spawn`). The lane's worker, two submissions deep (0100 §11), then begins the next submission. So
   one wave's sizing, encoding and reading back run beside another's command buffers.
3. A spawned submission's end, a wait for its result, runs on the worker's own thread, never on a pool thread
   (`Pending::finish_on_pool`). On the lanes' pool, a thread waiting in one wave's parallel join stole the worker's
   wait for the other wave, and neither moved: a deadlock, found on s1000 by a stack sample.
4. Each running submission has a slot: one pool per submission in flight, two slots. On one shared pool, a thread
   waiting in a join could steal the other submission and run it nested on the same thread. The nested run would
   share the outer one's thread-local launch budget and profile spans, and the outer run would wait for all of it.
5. A submission that ran to its end as it began is handed back at once, not after the next one (24a3e5d09).

## Measured

s1000 internal_dead_code, the wave pinned to the GPU:
- 19 wave batches began, 8 of them while another ran, with the same 0 findings.
- The machine's load was 54–91 during these runs, too high to time the overlap. The timing is still open.

The overlap's timing (2026-11-17, 0392's engine). `RERAIL_GPU_OVERLAP=off` runs each batch of programs as it
begins, for this comparison. The runs used an M3 Max with the wave pinned to the GPU, at s1000, with on and off in
turn, 4 runs each. A quiet machine was not to be had: the M3's load was 67–72 on 16 cores, and this M5 Max's 64–71,
where run walls swung from 3.5 to 13.4 s.

| M3 Max | Overlap on | Overlap off |
|---|---|---|
| Run wall | 5.35–6.61 s (mean 5.91) | 5.59–6.45 s (mean 6.03) |
| Matcher waves, the executor's wall | 1.32–1.92 s (mean 1.70) | 1.74–2.21 s (mean 1.90) |
| Jobs' wait in the GPU lane's queue | 0.28–0.47 s | 0.60–0.85 s |
| Batches begun beside another | 1–11 of 16–18 | 0 |

The findings and the peak memory (934–978 MB) are the same. The overlap halves the GPU lane's queueing and takes
about a tenth off the waves' wall. That is 0.2 s of a 6 s run, inside the run-to-run noise at this load.

## Consequences

- Two waves' buffers can be live at once. The peak was 925 MB against 906–956 MB before, within the 1 GB line.
- The CPU a spawned submission spends on its slot's thread is added to its ticket (`Ticket::add_cpu`), so the
  lane's job CPU stays whole.
- `GPU lane programs: N batches begun, M of them while another ran` in the profile's lane section.
