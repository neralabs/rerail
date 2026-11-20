# 0321: Placement leans to the GPU

Status: built (2026-11-16). Replaces 0095's margin as 0304 kept it. `logic/plan/placement.rs`: `GPU_BIAS`,
`DEVICE_ROW_BACK`; `logic/profile/report.rs`. Work queue Q36.
Numbered 0315 on this branch until the number was found taken in another worktree.

## Intro

Placement chooses the CPU or the GPU for each stratum from a price on each. Until now the GPU had to be priced at
half the CPU's or less (`MARGIN`, 2), since the estimates were rough. They were 8 times off (0320).

The GPU is the wider of the two, with the more memory bandwidth. Work it takes leaves the CPU's cores to the strata
only they can run: those with a computed term, and the many small ones. So the choice now leans the other way.

## Decision

1. **The GPU is chosen while its price is under twice the CPU's** (`GPU_BIAS`, 2): `device_s < cpu_work_s × 2`,
   where it was `device_s × 2 < cpu_work_s`. The stratum's floor (planning, opening, closing) is still in neither side.
   A stratum that needs a form the device lacks stays on the CPU, as before.
2. **A join step's form**: the cheaper of the device's two forms (sort and probe, tile) is taken over the CPU's index
   while under twice its price. Before, a tile had to be half a probe's price as well.
3. **A row read back costs 105 ns** (`DEVICE_ROW_BACK`, was 75 ns): refitted from `kind`'s 63 runs on the GPU, 443
   ms of the host's CPU, 76 ms of it the submissions and uploads, for 3.52 M rows. The CPU's run of the same copy
   took 437 ms, and is priced at 457 ms. So a copy of an input is priced about level on either side, as measured.
   Decision 0277's test that a copy stays on the CPU now checks that its two prices are within 10%.
4. **The profile says so**: its prices line gives the constants and the bias. Its count of the strata "the GPU
   would be cheaper for" now counts those whose GPU price was within the bias.

### A variant measured and dropped

The bias can be applied only to the GPU's own work: the host's part of a device run (encoding, uploads, rows back)
is CPU either way, so it would be weighed at par. That moved about 118 runs to the GPU, against about 316 with
the bias over the whole price. The strata's CPU was 3.20 s, against 3.03 s for the whole-price bias and 3.30 s
before. Its wall time was the worst of the three, at load 70.

## Results

PostHog's `posthog/hogql`, the light run, cold, `RERAIL_SQL_STRICT=1`, profiled, with 0320's estimates.

The bias against the margin, with 0320's estimates in both, alternated at load 50–55: runs on the GPU went from
43–55 to 301–322 of about 2,200, and the strata's CPU fell from 2.96–3.38 s to 2.42 s. Against the build before
0320 and 0321, alternated at load 45–50, it was a wash: wall 1.84–2.70 s against 1.64–2.33 s, strata CPU 2.53–2.90 s
against 2.52–2.89 s, instructions 75.8–80.1 G against 79.5–80.2 G. Findings are 1,629 in every run.

The same build's plans under each binding show what the GPU's strata do. In one Metal run, 37 strata ran on the GPU
at least once. Against the same strata's CPU under the CPU binding, they saved 2,238 ms of host CPU and lost 115 ms:

| stratum | runs on the GPU | CPU binding | Metal | priced CPU / GPU |
|---|---|---|---|---|
| `pre_126` | 18 of 18 | 1,801 ms | 299 ms | 1,616 / 424 ms |
| `wstart` | 18 of 18 | 415 ms | 68 ms | 322 / 62 ms |
| `walk` | 18 of 18 | 380 ms | 64 ms | 142 / 59 ms |
| `attr_of` | 18 of 18 | 69 ms | 38 ms | 50 / 45 ms |
| `demand_tp_name` | 18 of 18 | 140 ms | 113 ms | 257 / 252 ms |
| `kind` (a copy) | 61 of 66 | 336 ms | 373 ms | 458 / 446 ms |
| `asg` | 17 of 18 | 4.7 ms | 9.2 ms | 16 / 17 ms |
| `export_query__sup0` | 1 of 1 | 4.5 ms | 9.5 ms | 10 / 10 ms |

The large gains are strata the margin placed there too. The strata the bias adds are about level: `attr_of` and
`demand_tp_name` gain, and small strata each lose about 0.3 ms a run on the GPU.

## What it does not fix

- **A small stratum's device run costs more than priced**: about 0.3 ms a run past its price (`asg`,
  `export_query__sup0`, `sym_of`). The submission's 100 µs (`DEVICE_SUBMISSION`) is likely low for a stratum run
  alone, outside a wave. A refit wants an idle machine (Q36).
- **The wall time** did not move measurably at load 45–70. The host CPU the GPU's strata save is the measure here.

## Tests

`placement::tests::a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks`: a stratum the GPU's
price puts at 1.5 times the CPU's is the GPU's, and at 2.5 times the CPU's; a copy is priced level on either side.
