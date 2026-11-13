# 0018: execution ranges packed to a byte budget; count programs run as walks

Status: accepted (2026-10-31)

## Context

These are the stages of the S7 Metal wave on 250 files (`RERAIL_PROF=1`, load 40–60):

- **`demand.partitions`: 2.4 s.**
  - `stable_partition_ranges` halved the chunk until each range fit.
  - It rebuilt the packed batch of the whole range at every level.
  - Fitting only checked the `u32` workspace totals, so a range could still hold gigabytes.
- **`labels.gpu`: 1.9–2.2 s,** of which `gpu.size` is ~1.1 s and `gpu.flush` ~0.7 s.
  - `size.tasks`: 4.7 M interpreter tasks per run.
  - `refused.other`: 4.6 M starts. Most come from the count programs, because `Micro::of` was given only modes 0 and 1.

## Decision

1. **Budgeted ranges.** Each file's workspace is sized once, on its own and in parallel, by the new
   `packed_workspace_bytes`. Consecutive files are then packed into ranges of at most 64 MB
   (`RANGE_BYTES`). A file over the budget runs alone. A file whose own tasks overflow the totals
   still splits by program groups (0016).
2. **Count walks.** The micro walk takes an `Emit` mode, which follows `label_sink`'s routes:
   - start once;
   - start at every path, which is the tally by root;
   - captured nodes, for targets and the tally by capture.

   On Metal, `label_micro` tallies into `counts`, bound at buffer 10, with the Shape at 11. The
   CPU sweep uses the same `Emit`.

## Evidence

- **Micro differential:** `micro_walks_are_the_interpreters_label_outputs` compares walks with the
  interpreter, with multiplicity for counts. It passes on the fixtures (3.5 M starts) and on the
  100 KB module (14 M starts). 557 of 579 label programs are walks, 19 of them counts.
- **Gates:** S2 Metal parity is 87 of 87; S1, S4–S6 and vtab pass.
- **S7 against W:** 710 keys against W's 710 on both backends, and both mutations fail the gate.

Stage times on Metal, before → after:

| stage | before | after |
|---|---|---|
| `demand.partitions` | 2.4 s | 0.05 s |
| `size.tasks` | 4.7 M | 67 k |
| `labels.gpu` | 2.2 s | 1.1 s |
| `gpu.flush` | 0.7 s | 0.25 s |

S7, 250 files, load 40–60:

| backend | wall | CPU (user+sys) |
|---|---|---|
| CPU | 28.3 s | 28.7 s |
| Metal | 8.4 s | 16.7 s |

## Open

- **Peak RSS is unchanged, at 1.25–1.4 GB (O30).**
  - `prof::peak` records it by stage.
  - Its readings so far are noisy, because the process is shared with W's run.
- **The CPU backend's labels are still single-threaded interpretation (O30).**
