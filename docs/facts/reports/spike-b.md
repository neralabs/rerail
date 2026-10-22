# Spike B report: base + delta dataflow for cross-file rules

Status: **in progress** (draft; numbers below are from dev runs and are replaced by the protocol runs as they land).

Branch `Moti-Velz/facts-spike-b`, crate `spikes/facts-b`, raw runs under `~/rerail-bench/facts/spike-b/runs/`.

## Done so far

- Extractor (tree-sitter, Rerail's vendored patched Python grammar), `.xfacts` format, 7,091 distinct contents extracted.
- Base (strict majority over 01-40) and per-worktree deltas; divergence table.
- One timely/differential dataflow, tagged single-arrangement base + delta form, R1-R3.
- Seeded edit script (1,000 edits + 5 bulk + 5 new worktrees x 10 edits + 51 real commits).
- From-scratch oracle; release exactness run `20261022-120135-exactness`: 1,576 worktree checks, 0 differences.
- Variants: two-level cluster bases (`--clusters`), naive worktree-keyed (`--naive`), R1 only (`--only-r1`).

## Next

Protocol runs (bench-lock, 3x), per-worktree state (`--track`), report tables.
