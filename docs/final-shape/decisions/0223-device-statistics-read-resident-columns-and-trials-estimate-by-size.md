# 0223: The device's statistics read resident columns in place, spread their reduction, and trials estimate by size

Status: built (2026-11-14). Decision 0221's three open items:

- the GPU batch reads a relation the device holds where it is (`SolverDevice::column_stats`,
  `setfix::device::column_stats`, `rerail_accel::gpu::join::column_stats_of`);
- `col_stats` is spread over a column's tiles;
- a stratified trial's parts are estimated by ratio to their rows' sizes (`Run::heavy_parts`).

## Intro

Decision 0221 made heavy values on the GPU, and split a trial's sample into a skewed key's heavy rows and the rest.
It left three things open:

- the batch copied every column from the host, even one the device joins already held;
- one threadgroup reduced a whole column;
- the trial's estimate of the line's work stayed under it: 0.68 of it.

## Decision

### 1. The batch goes through the device interface

The planner's batch (`table::heavy_batch`) called the accel crate directly. That bypassed the logic's device
interface (`logic::device`), which is what gives the device crate its relations. It now asks the registered device
(`SolverDevice::column_stats`). Each column is one of:

- `StatColumn::Values`: values on the host;
- `StatColumn::Rel(rel, c)`: column `c` of an engine relation, where the engine holds it (`engine::held`).

Without a device, the CPU's batch runs, as before.

### 2. A resident relation is read in place

The device keeps a thread's relations uploaded, column-major, one buffer each (`setfix::device`'s `RESIDENT`). A
requested relation found there, with its live rows unchanged, is read from that buffer: `StatCol::Device { buffer,
at, rows }`. Nothing is copied.

Its span is not known on the host. Three kernels find it on the device (`COL_SPAN_SOURCE`):

- `col_span`: each tile's least and greatest value;
- `col_base`: the column's least value (the base), and whether its span passes 32 bits;
- `col_narrow`: each value less the base, as a sort key.

The keys are sorted 32 bits wide, in four passes. A column whose span passes 32 bits is made again, in a second
command buffer, 64 bits wide. A relation not resident is gathered on the host and uploaded, as before.

### 3. The reduction is spread

`col_stats` runs one threadgroup per 1,024 runs and writes each group's sums of squared and cubed lengths. `col_sum`
adds the groups' sums in one threadgroup.

### 4. A trial's parts are estimated by size

Each row left has a size: the rows of its key, raised to the number of positive atoms holding the key, less one.
That is about the work per row of a self-join on the key. A row that reaches its key through a link atom takes
the sum over its keys.

A part's work is estimated as:

    the sample's work × (all the part's sizes / the sizes of the rows it ran)

The old estimate was the sample's mean work per row, times the part's rows. Where a sample has a few heavy rows
of unusual size, the ratio corrects it.

## Measured (2026-11-14, debug build, strict)

**Tests** (each crate's own; the planner's test target was not rebuilt, since this change does not touch it):

- accel `gpu::tests`:
  - two columns of one device buffer, a narrow span far from zero and one past 32 bits, read in place, are the
    host batch's statistics;
  - the 0221 test, with the spread reduction.
- device crate (`setfix::device::tests`):
  - a relation made resident is read in place (2 columns), and gives the gathered batch's statistics;
  - the GPU batch makes the CPU's heavy values; moved here from `plan::table`, where no device is registered.

**Smoke** (the self-join seed `call_ret` of decisions 0206 and 0207; 545 findings):

| Estimate | Sample (heavy keys / the rest) | The winner's work per row | Share of the line's 874 |
|---|---|---|---|
| At a stride (0216) | 9 rows, unsplit | 492 | 0.56 |
| Weighted by the parts' rows (0221) | 2 of 75 / 7 of 252 | 593 | 0.68 |
| By size (this decision) | 2 of 75 / 7 of 252 | 599 | 0.69 |

The sample was already representative by size here. Its two heavy rows hold 117 k of the part's 4.47 M of size:
2.6% of the size for 2.7% of the rows. So the ratio changes little on this seed. The winner and the line's work
(385 k) are as before.

## Open

- **O173** (closed by decision 0226). The trial estimate is still under the line's work on `call_ret`. The gap is per-row work that the
  key's size does not explain: the returns a function reaches, not its size, as 0216 §3 found for pricing. Pair
  statistics (0203 §3) are the measure that would see it.
- The in-place path runs only where the batch goes to the GPU: from 1 M rows (`gpu_pays`). The fixture's batch is
  smaller, and runs on the CPU.
