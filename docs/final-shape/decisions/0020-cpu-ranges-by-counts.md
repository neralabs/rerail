# 0020: CPU execution ranges packed by u32 record counts; program groups from one batch; text rows shared across runs

Status: accepted (2026-10-31)

## Context

After 0019, the S7 Metal wave (250 files) had the following host costs (`RERAIL_PROF=1`):

- **Executions.** `demand.execute` took 2.6–3.1 s over 247 executions, one for nearly every file.
  - 0018 packed files to a 64 MB workspace budget. But per-file sizes summed to 1.9 TB, about
    7.7 GB of virtual size per file, so no two files fit together.
  - The CPU JIT's workspaces are demand-zero (`DemandZeroRegion`): pages exist only as used. What
    binds a CPU execution is that each arena's record count fits a `u32`, not the bytes.
- **Program groups.** Four files cannot be sized at all. For them, `program_groups`' binary search
  rebuilt the file's batch for each fit check: 1.0–1.2 s.
- **Text rows.** They were evaluated again by each of the wave's 9 GPU runs: 58 evaluations.

## Decision

1. **CPU ranges by counts.** On CPU backends, `execution_partitions` packs files while each summed
   record count (`PackedBatch::workspace_counts`) stays within `u32::MAX / 2`. Metal backends keep
   the 64 MB byte budget, since their workspaces are allocated.
   - A range that overflows anyway runs file by file. None did on S7.
2. **Program groups from one batch.** `PackedBatch::program_counts` sums the per-task capacities
   (`task_capacities`, factored out of the assignment) per program, from one batch of the file.
   Groups are packed from those sums.
   - A group that overflows when it runs falls back to the fits search
     (`program_groups_searched`). None did on S7.
3. **Text rows across runs.** The rows are kept in a `TextRows` cache that `apart` owns and passes
   to each run through `Plan::rows`.
4. **Candidates:** rows map to tapes through a hash, and each clause's nodes are sorted in
   parallel instead of built in a `BTreeSet`.

## Tried and dropped

- **Parallel executions on the CPU:** 1 s less wall, but 21 s of summed roots time and 2.4 GB RSS
  (as in 0017).
- **Parallel per-program sizing:** 0.5 s less wall, but ~6 s of CPU.

## Evidence

- **Gates:** S1, S2 (87 of 87), S4–S6, the walk differential and vtab pass.
- **S7 against W:** 710 keys against W's 710 on both backends, and both mutations fail the gate.

| stage | before | after |
|---|---|---|
| executions | 247 | 32–45 |
| `demand.groups` | 1.0–1.2 s | 0.09 s |
| `demand.execute` | 2.6–3.1 s | 1.6–2.2 s |
| text evaluations | 58 | 47 |

S7, 250 files, load ~40:

| backend | wall | CPU (user+sys) |
|---|---|---|
| CPU | 19.3 s | 20.1 s |
| Metal | 5.2 s | 11.1 s |

## Open

Metal's remaining host stages:

| stage | time | note |
|---|---|---|
| prefilter SQL | 1.35 s | 1.15 M candidate rows; anchor-index work, in the `lab.rs` loop |
| roots on the CPU JIT | 1.3 s | sequential |
| CPU sweeps of recursive groups | 0.5 s | |
| push | 0.5–0.8 s | |
| label views | 0.3 s | |
| GPU label runs | 0.7 s | |

Peak RSS is 1.1–1.3 GB (O30).
