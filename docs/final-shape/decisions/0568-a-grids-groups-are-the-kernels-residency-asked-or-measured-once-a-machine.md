# 0568. A grid's groups are its kernel's residency, asked or measured once a machine

Status: accepted (2026-11-21, the user: "do A (but cached per machine, and ideally calling APIs) and C"). Changes how
0468 §4 finds G; the grid, its barrier and its join are unchanged.

## Intro

The preprocessor's setup is one persistent grid (0468 §4): G groups, with a bounded device-wide barrier
(`ukl/grid.h`) between its phases. A barrier holds only if all G groups run at once. A group that is never scheduled
never arrives, the others reach the barrier's bound, and the unit is refused (`barrier`).

0468 took G from the device: its core count times the groups a core holds at the kernel's threadgroup memory
(`Device::resident_groups`). That is an upper bound. A core holds fewer groups of a kernel that uses many registers,
and Metal neither promises co-residency nor says how many groups of a pipeline fit. The join (`ukl_grid_join`) is the
floor: a group that comes late does not join, and the grid runs on the groups that did. But a dispatch past
residency is still a dispatch that may wait.

## Decision

1. **G is the kernel's residency, from the first source that knows it** (`rerail_device::grid::residency`):
   - **the backend's answer** (`Known::Api`), where it has one:
     - CUDA: `cuOccupancyMaxActiveBlocksPerMultiprocessor` for the function at its block size and dynamic shared
       memory, times the device's multiprocessors (`CudaComputeFunction::resident_groups`);
     - the CPU: its own rule (`compute_kernel_cpu::resident_groups`);
   - **else the measure kept for this machine** (`Known::Measured`): the kernel store's `grid/<key>.txt`, keyed by the
     device's name and core count, the kernel (its source and name) and the group's threads. A new kernel source is
     a new key, so it is measured again;
   - **else the device's bound** (`Known::Bound`), 8 groups where the device gives none. That is the first count
     tried on Metal and OpenCL, which have no occupancy call.

   The preprocessor dispatches `min(residency, GROUPS_MOST)` groups (1 on the CPU, as before). The machine's answer
   is found once a process and held by a machine object (`pp_arena::Groups`, decision 0442). The device crate keeps
   nothing in memory.
2. **A failed barrier halves the residency, and the half is kept** (`grid::halved`, `pp_arena::halved`).
   - `on_grid` sees the unit's `barrier` flag and halves the residency, keeping `max(G/2, 1)` for the machine. Then
     it returns `BackendFailure`.
   - `pp_eval::at_resident` runs the evaluation again at the new residency, while a failure lowered it below the
     groups just run.
   - Every grid entry point goes through it: `evaluate`, `definitions_from`, `expand_sized` and `lookups_sized`.
   - Not halved: one group (nothing smaller helps), or a backend's answer (its count is resident, so its failed
     barrier is the kernel's failure).
   - A measure only falls. To measure again, remove its `grid/` entry.
3. **A dispatch past the residency is refused** before it is encoded (`pp_arena::refuse_past`, `BackendCapacity`).
   `stage_run` and `at_resident` check it. The join stays the floor below it.
4. **A device's output is checked before the host follows it.** `Expansion::read` reads the output's pieces with each
   word bounded by its store: the pieces must add to the output's count, and name only nodes, gaps and definitions
   inside their stores. Otherwise the run is `BackendFailure`, not a read past a buffer. A wrong device output used
   to become a SIGSEGV in the host.

## Tests

- `grid::tests::a_measure_is_kept_and_a_halving_keeps_the_half` (the device crate): a measure kept is read back, a
  halving keeps the half, never below one.
- `pp_arena::tests::the_grid_is_the_devices_resident_count`: the residency is between 1 and the device's bound, and
  `groups()` is it, capped at `GROUPS_MOST`.
- `eval::tests::an_evaluation_is_a_fixed_handful_of_launches`: a dispatch past the residency (4,096 groups) is refused
  with `BackendCapacity`. A dispatch at the residency gives the default grid's output, at the same fixed launch count
  for the smallest and largest units.

Measured (2026-11-21):
- the m3, CPU backend: the eval, `pp_arena` and grid tests 43 of 43;
- Linux: the device crate 8 of 8, its CUDA path compiled;
- Metal (the m3): `the_grid_is_the_devices_resident_count` passes.

## The monolithic kernel's failure on Metal (recorded, not chased)

On the m3's Metal, `an_evaluation_is_a_fixed_handful_of_launches` fails at its large shape (`eval/tests.rs:831`, "a
device error"), with or without this decision.

- **Before this decision** (3d24d7e030's tree): 3 of 3 runs, at the device's bound.
- **With it**: at 4 groups, a measure kept after earlier failed barriers. Both grid runs reported no failed barrier,
  but the output's pieces did not add to its count, and the host's read of them crashed (SIGSEGV). Item 4 makes that
  a `BackendFailure`.
- **The same test, the same shapes, on UKL's CPU backend**: passes.

So the failure is the device's code at the large shape, not the grid's count. The user said not to chase the one
kernel's miscompile (2026-11-21, "don't chase"). The fix being built is to compile pp's Metal kernel as per-function
dynamic libraries (the next decision). This test on Metal is its check.

Halvings caused by that failure may also have lowered the m3's kept measure: a measure only falls (2). Once the
kernel is per-function, its source is new, so its key is new, and it is measured again.
