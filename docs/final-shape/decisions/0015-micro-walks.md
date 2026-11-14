# 0015: Short label programs run as micro walks, one thread per (pass, start); workspaces are untouched by the host

Status: accepted
Date: 2026-10-31

## Question
O28: the wave's labels on Metal were barely faster than on the CPU, and Metal's process peaked at
2.1 GB of resident memory. Profiling the S6 bench (48 fixtures, `RERAIL_PROF` stage timers) gave
these causes:
- **The work shape.** A cold audit ran 2.51 M interpreter tasks, about 160 per node. 97% came from
  label passes over every node (sub-rules with no kind), and only 35% produced a result. Each
  task carries a scheduler entry, quotas and arena blocks sized for unbounded backtracking.
- **The memory layout.** A 1,024-lane workspace is about 930 MB. `MappedBuffer::zeroed` cleared it
  on the host, which committed every page and took 144 ms per allocation. The pool held one
  workspace, so a run whose levels varied in size replaced it on every call.
- **More lanes were slower.** At 16,384 × 256 the flush took 828 ms, against 260 ms at 1,024 ×
  4,096. One interpreter thread costs about 100 times a CPU core. Placing the lanes' blocks on the
  host instead of by CAS changed nothing.

The programs are mostly short walks. Weighted by roots: a label test 13%, a text test 12%, a child
label test 7–13%, kind tests behind a fork 5%.

## Decision
- **A micro walk** (`compile/micro.rs`, `label_micro` in `metal_label_passes.metal`) is a label
  program whose every operation is a node test, a tree move (child, descend, next, following,
  parent), a saved or restored cursor, a capture, a non-greedy fork, a goto or an emit.
  - It runs from one start with a fixed stack of 128 frames, 2 cursors, 2 captures and 2^16
    steps. A frame is a forked path or a move's remaining candidates, so the stack grows with the
    program's nesting, not with a node's fan-out.
  - It has no task list, arenas or scheduler. On Metal there is one thread per (pass, start), and
    the starts are the level's root lists as global nodes.
  - 380 of the 560 label programs qualify.
- **Its outputs are the interpreter's, as a label reads them.** A roots label is the starts that
  emit, and the walk stops at the first. A targets label is every node captured on a path that
  emits.
- **A walk that outgrows its frames, captures or steps fails.**
  - On Metal it sets `flags[0] & 2`, and the run encodes again with every program on the
    interpreter (`Outcome::interpreted`).
  - On the CPU sweep it falls back to `execute_roots` for that (pass, node).
  - The fixtures need 128 frames; at 32 they overflowed 12 of 2.39 M starts.
- **Programs that do not qualify** stay interpreter tasks: text and regex, size, assertions,
  first and last, absent fields, repeats, greedy forks, directives and output programs. So do
  sweeping groups on Metal.
- **Workspace buffers are allocated untouched** (`MappedBuffer::untouched`). Metal zero-fills a new
  buffer, the GPU writes every arena record before it reads it, and the pool already reuses
  leases by generation stamp. Pages are committed only as the GPU touches them.
- **A new workspace covers what the pool held as well** (`WorkspaceTotals::widest`), so a run whose
  levels differ is served from one workspace instead of replacing it on each call.

## Prior work consulted
- **Hist-index 2.23, "Generic interpretation is about 12x slower than specialized code"
  (S250, DH L7715-7728):** a GPU kernel interpreting patterns pays the same per-row generality
  cost unless the patterns are compiled. *Agrees.* The micro walk is a specialized executor for
  the subset of programs that are walks. The general interpreter stays for the rest.
- **Design-index 19 (D S152 L6687-6721):** row machinery, not decode, dominates a touch, and a
  batch executor that avoids per-row machinery attacks the main cost. *Agrees.* A walk drops the
  interpreter's per-task scheduler entry, quotas and arena blocks. They were the per-row
  machinery here.
- **Hist-index 2.23 (Part 0 items 3, 8, 9) and design-index 12 (S149):** plan-level selectivity
  dominates, and what a batch could share it should not. *Partly.* The domains (kinds, labels,
  every node) are unchanged here. Each pass's starts are listed per level, not shared, and the
  walk's threads share nothing but read-only tapes.

## Consequences
- **Measured on the S6 bench** (48 fixtures, CPU time user+sys, load 55–90):

  | | CPU backend | Metal before | Metal after |
  |---|---|---|---|
  | Cold audit | 1.79 s | 1.23 s | 0.92 s |
  | One-file edit | 0.40 s | 0.26 s | 0.22 s |
  | Labels, cold (wall) | 1.09 s | 1.09 s | 0.43 s |
  | Peak resident memory | 209 MB | 2.1 GB | 284 MB |

  The CPU sweeps use the walk too: 267 ms became 110 ms cold.
- **Checked by:**
  - `micro_walks_are_the_interpreters_label_outputs`: every qualifying program at every node of
    4 fixtures equals `execute_roots`, 2.39 M starts with 0 overflowing;
  - S1 and S2 parity (87 of 87 clauses, 60 fixtures, Metal equal);
  - S4's wave gates, S5's findings and S6's slice gate, with 0 differing.
- **Left (O28):**
  - the remaining interpreter tasks: text tests are 12% of roots;
  - sizing and the GPU flush, about 270 ms cold;
  - the CPU sweeps between segments, about 85 ms warm.
