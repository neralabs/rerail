# 0016: Work that outgrows a workspace is split along an axis it is independent on; S7's gate lists known differences by name

Status: accepted
Date: 2026-10-31

## Question
06 S7 ran the slice on real files: the pack's fixtures plus 100 files of CPython's standard
library, 250 files and 1.5 MB in all. Two things failed on files no fixture resembles, and the
gate needed a rule for a divergence it found.
- **A 100 KB module failed every wave** with `BackendCapacity`. A demand partition is a range of
  files, and a range that does not fit is one file. That file's tasks, over all 87 root programs,
  exceed a packed workspace's `u32` totals, although each program alone fits.
- **Metal's labels failed on the whole chunk.** All 250 files are one chunk, so a label level over
  all of them outgrew a workspace.
- **One clause differs from W:** SIM300 reports 3 keys that W does not (O29).

## Decision
- **A file that does not fit runs its programs in groups** (`PackedDemand::program_groups` in
  `demand/vtab.rs`):
  - each group is the longest run of programs from where the last one ended that fits, found by
    bisection over `packed_workspace_fits`;
  - an execution carries its first program, and the rows it produces are offset by it;
  - a range that fits runs as one execution, as before.
  - Root programs are independent per (program, file), so groups are exact.
- **A chunk whose labels outgrow Metal is labelled in halves** (`labelled_metal` in
  `waves/matches.rs`), recursively. A file's labels read only that file.
- **The gate requires equality for every (rule, code) except those named in `KNOWN`.**
  - Each `KNOWN` entry names its open item.
  - Each has an ignored test of its own (`a_recursive_label_settles_at_every_depth`), which fails
    until it is fixed.
  - The mutation checks require a difference beyond `KNOWN`.
- **The sample's real repository is read in place**, never vendored: `RERAIL_S7_REPO`, by default
  `python3`'s standard library. The private repositories on this machine are not used.

## Prior work consulted
- **Hist-index 2.18, "measure at deployment size" (DH L3630-3635):** toy slices hid scaling.
  *Agrees.* The fixtures, all under 20 KB, never reached either capacity; 100 KB modules did.
- **Hist-index 2.17, "known residual mismatches tracked explicitly" (DH L8524-8527; L8771-8775):**
  *Agrees.* `KNOWN` names the one divergence, and the gate still fails on any other.
- **Hist-index 2.17, mutation-checked tests (DH L3415-3420):** shift a span by one byte.
  *Agrees, with a contract caveat.* A key is a line (the loose findings contract), so a one-byte
  shift changes a key only across a line boundary. The check therefore moves the anchor span of a
  finding at its line's first byte.
- **Design-index 12 (D S145 L1087-1117):** partitioning pays its fixed cost once per part.
  *Agrees.* Only a range that does not fit is split, so the fixtures' batches are unchanged: one
  execution per range.

## Consequences
- S7's gate passes on the CPU in 55 s: 713 keys, W 710, the 3 extra being O29's. Both mutations
  fail it.
- Metal agrees on a third of the sample: 75 files, 361 keys, W 361. On the whole sample its labels
  take minutes and 3.9 GB, so its arm is opt-in (`RERAIL_S7_BACKENDS`; O28).
- Peak RSS on the CPU is 972 MB, against a 500 MB budget, and labels take 24 s of a 32 s cold
  audit (O30, for S8).
