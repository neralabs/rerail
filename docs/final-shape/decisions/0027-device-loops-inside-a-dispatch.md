# 0027: device-decided loops run inside one dispatch

Status: accepted (2026-11-02, list step 17)

## Context

Phase 4 (e) asks for device-side enqueue of in-file loops whose bound is not fixed (E9), "from G
indirect dispatch to indirect command buffers"; (f) asks for file-local lattice FIX waves (I10, R0).

What Metal offers:
- A kernel cannot launch a kernel.
- An indirect command buffer (ICB) can be encoded by the GPU, but only into commands the host
  allocated before the command buffer ran. It cannot add rounds it did not reserve. A loop with
  no fixed bound therefore needs either a host round trip per round or a reserved maximum, which
  is a fixed bound with early exit, the shape G's indirect dispatch already has.
- A threadgroup can loop on its own for as long as it likes inside one dispatch, with
  `threadgroup_barrier` between rounds, provided its loop's data stays inside the group.

The evidence:
- ast-index-spike: GPU work is absent (DI §19, "Nothing was designed or measured for GPU"), so
  this is new ground. The spike's structural acceptance rule still applies: a bound must follow
  from the work, not be a fixed budget (lessons ledger row 26).
- Tempo-metal's host-driven generations stopped at 595 dispatches (00-review), and one task row
  per tuple needed 1.99 GB (ledger row 1).
- Our label groups already run this way: `Runner::group` fuses a recursive label group into one
  dispatch whose threadgroup iterates to its fixpoint (S7: 8 groups fused, 0 unfused).

## Decision

On Metal, a loop the device decides is a threadgroup's own loop inside one dispatch. Its data is
partitioned so no edge leaves a group. The bound comes from the structure: a lattice's height
times its cells, or a label group's nodes. There are no ICBs and no host-planned rounds for these
loops.

- **Label groups:** `Runner::group`, as built in steps 13–15.
- **File-local lattice FIX** (`metal_lattice::solve`, kernel `lattice_flat`):
  - The cells split into components by union-find over the copy and transfer edges, one
    threadgroup each.
  - Each wave is Jacobi. It evaluates the dirty cells from the last values, then applies the
    changes and marks their dependents dirty, until a wave changes nothing.
  - The flat lattice has height 3, so `2·cells + 2` waves bound it. Reaching the bound is an
    error, not a truncation.
  - `logic::native::lattice_fixpoint` binds it when the binding is Metal, the lattice is flat and
    there is no widening. The CPU fixpoint is the reference: a monotone system over a finite
    lattice has one least fixpoint, whatever the evaluation order.
- **`R[tid]`** (g) needs no loop. A walk's text slot is read from the term index
  (`prefilter::terms`).

## Not decided here

- The unfused rounds path stays host-planned rounds with device early exit and a rerun. These
  are recursive groups holding interpreter tasks or pattern kernels, which a threadgroup loop
  cannot hold. G's grid executor's frontier waves also stay as they are (O33). See O34.
- Widened and non-flat lattices stay on the CPU.
