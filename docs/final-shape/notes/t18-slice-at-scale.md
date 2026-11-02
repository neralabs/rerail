# T18 notes: the first slice at scale

This note runs the slice (06 S6, `cr/slice/napi.rs` `CodeRuleSlice`) and W's native workspace over the same real
`nera-data-layer` worktrees with the Python pack, then compares their findings key for key. S7 and S8 measured the
slice on 250 files. T13 (on the facts-crossfile branch) found that the warm edit grows with the scope, and that S8 on
2,650 files failed with `Err(Busy)`.

Branch `jeremybarnes/final-rerail-shape-slice-harness`, off his `35c010235`. The engine is unchanged; the only engine-side
commit is an ignored reproducer test (PROPOSAL).

## Conventions

- **M** means measured in a cited run, **D** means derived from M numbers, and **A** means inferred.
- `SH:` is `~/rerail-bench/final-shape/slice-harness/runs/`. Each run directory holds `provenance.txt` (commit, addon
  sha256, settings), `load.txt` (the 1, 5 and 15-minute load at each process's start and end), per engine and worktree
  `<engine>.json`, `stderr.txt` and `time.txt` (`/usr/bin/time -l`), the findings, `compare.txt` and `summary.md`.
- "Quiet" runs waited for a 1-minute load below 3 before each process (`run.sh`). Loaded runs are marked as such, and
  their times are not used.
- Machine: Apple M5 Max, 18 cores, shared with three other workstreams.
- CPU means user + sys for the whole process. Footprint is `time -l`'s peak memory footprint for the whole node process.
  That process is about 220–310 MB before any workspace runs (node, tsx, the addon and the compiled rules).
- Addons:
  - `tip.node` (sha256 `c91b3e96…`) is `35c010235` built with `npx napi build --release`.
  - `o70-patched.node` (`5fd393c4…`) is the same commit plus `o70-put-tape-retry.patch`. The patch runs `put_tape`
    under `immediate_transaction`, as the store's other writes are. It was built to measure past O70. **It is not
    committed.**

## Setup

- **Worktrees.** The memory47 `nera-data-layer` worktrees `~/rerail-bench/memory47/workspaces/01..40`, at their pinned
  commits. Each has 5,494–5,725 `.py` files, 88–91 MB.
- **Rules.** `packs/python/rules` plus `packs/python/candidates/rules`: 234 policies, as the slice's tests load them.
  The slice lowers 87 clauses of 63 policies, each with one Python matcher. Native runs all 234 policies.
- **Harness.** `node --import tsx benchmark/workspaces/workspaces.ts run --engine native|slice …`, one process per
  worktree and engine.
  - **native** is facts-engine's harness (COPY `1fc5da3bb`, adapted to this branch). It does the enforcer's audit walk
    without a store, then `--edits` one-file edits. Each edit appends a comment to the Python file at the minimum, p50,
    p95 and maximum size in turn.
  - **slice** (`slice.ts`) runs one `CodeRuleSlice` over a fresh store and backend `cpu`. It does:
    1. a cold `audit` of the directory snapshot;
    2. the unchanged `apply`, which must evaluate nothing;
    3. the same edits as native, as editor buffers over the directory, keeping the earlier edits' buffers so that
       each apply changes one document.
  - `--sample p` keeps native's path-hash sample. For the slice it copies the sampled files to a directory of their
    own.
- **Exactness.** `compare-findings` compares sets of `[rule, code, path, line]`. Native's violations are restricted to
  the clauses the slice lowers.
  - `slice.json`'s `lowered` names those clauses. The harness probes them by lowering each policy alone, and each
    `forbid` clause alone with the policy's `define`s.
  - Both engines count lines from 0. The comparison is done after the cold audit and again after the edits.
  - The slice's `Unknown` coverage is listed where native has no failure for that (rule, path).

## Exactness per worktree

| Run | Worktrees | Files each | Keys each | Only native | Only slice | Verdict (scan, after edits) |
|---|---|---|---|---:|---:|---|
| `SH:20261102-163113-quiet-o70patched-full-5wt` (quiet) | 01, 08, 21, 34, 40 | whole (5,494–5,725 `.py`) | 94,260–97,484 | 0 | 0 | **equal, equal** on all 5 |
| `SH:20261102-154134-o70patched-full-5wt-loaded` | same | whole | same | 0 | 0 | equal, equal on all 5 |
| `SH:20261102-154151-tip-s5-5wt-loaded` (tip addon) | same | 5 % (256–268) | 4,829–5,031 | 0 | 0 | equal, equal on all 5 |
| `SH:20261102-163113-quiet-tip-s{5,10,20}-40` (tip) | 40 | 256, 571, 1,135 | 4,829, 11,199, 21,920 | 0 | 0 | equal, equal |

(M) The slice finds exactly W's keys on every worktree measured, at every size, before and after edits. Native reports 2
more violations than keys per whole worktree: two violations share a key.

There is one coverage difference (O71). On every whole worktree the slice leaves 80 (rule, path) pairs `Unknown`
(`source-incomplete`). They cover two MySQL `.sql` files (`docker-entrypoint-initdb.d/01-setup-users.sql` and
`data_history/full_gh2429_remaining/postgres_expression_index_eval.sql`) and the lowered rules whose scope is `**/*`.
Native neither fails nor reports anything there, because a Python clause does not read an SQL file. The slice reports the
coverage of its Python clauses on a non-Python document whose parse is incomplete. No key differs because of it.

## Failures found

### O70: `Err(Busy)` on every whole worktree

- **M, at full scale.** With the tip addon, the cold audit fails `Busy` on all 40 whole worktrees after about 5–8 s
  (`SH:20261102-152414-busy-full-40wt`, loaded). Quiet, it fails the same way on 40 and 01
  (`SH:20261102-163113-quiet-tip-full-busy`: 7.4 s and 8.6 s wall in all).
- **M, on samples.** Quiet, 5 %, 10 % and 20 % samples (up to 1,135 files, 20 MB) complete. Under load (1-minute load
  ~100), even 20 % failed (`SH:20261102-152029-smoke-s20-40`). The failure depends on timing, not only on size.
- **M, the cause.** A profiling-build S8 with 2,500 files of worktree 40 and a backtrace at `StoreError::Busy`'s
  conversion (`SH:20261102-151858-diag-busy-s8-2500`) shows:
  - the error is `database is locked` (`SQLITE_BUSY`);
  - it is raised at `content_store/objects.rs:453` (`put_tape`'s `BEGIN IMMEDIATE`);
  - the call path is `facts/families.rs:347` (`encode`), then `Families::push`, then `slice.rs:153` inside
    `Overlay::scoped`.
- **Why.** Store connections are per thread (`ThreadConnection`).
  1. The overlay's background writer writes layers (`write_derivation_layer`, `put_prefilters_bulk`) on its own
     thread's connection, under `retry_busy`, which waits up to 10 s.
  2. `put_tape`, on the push's thread, begins its transaction once and waits only the 250 ms busy timeout.
  3. Once a layer write holds the lock longer than that, the push fails. Larger scopes and a loaded machine both make
     layer writes longer.
- **Reproducer.** `content_store::tests::a_tape_write_waits_out_another_writer_as_the_store_writes_do` (ignored; PROPOSAL
  commit) holds the lock from another connection for 200 ms with a 20 ms busy timeout. `write_derivation_layer` waits
  it out, and `put_tape` returns `Err(Busy)`. With the patch, the test passes.
- **M, with the patch.** S8 at 2,500 files completes and absorbs 69 busy waits
  (`SH:20261102-153227-diag-patched-s8-2500`, loaded). Every whole-worktree run with `o70-patched.node` completes.
- **No other failure was seen.**

## Costs against W and against file count

Quiet runs on worktree 40 (tip addon for the samples, patched addon for the whole worktree):

| Files (`.py`) | MB | Slice cold CPU s (wall) | Slice warm edit CPU p50 / p95 ms | Slice unchanged apply CPU ms | Native scan CPU s (234 rules) | Native edit CPU p50 / p95 ms | Slice / native peak footprint MB | Slice store MB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 256 | 4.6 | 11.5 (5.7) | 771 / 2,287 | 736 | 15.3 | 32 / 1,582 | 886 / 1,522 | 26 |
| 571 | 10.4 | 25.1 (12.2) | 1,854 / 3,391 | 1,831 | 35.3 | 35 / 1,589 | 1,188 / 1,535 | 48 |
| 1,135 | 20.0 | 49.9 (24.8) | 4,811 / 6,338 | 4,969 | 80.0 | 53 / 1,636 | 1,494 / 1,593 | 82 |
| 5,494 | 88.2 | 231.7 (124.7) | 20,148 / 21,639 | 20,071 | 343.8 | 253 / 1,673 | 2,705 / 2,744 | 313 |

The quiet whole-worktree runs on 01, 08, 21 and 34 agree with this:

- slice cold: 233–243 s CPU, 126–131 s wall;
- slice warm edit: 20.2–21.6 s CPU (p50);
- slice footprint: 2,729–2,812 MB;
- native scan: 342–361 s CPU, with edits at 219–276 ms CPU (p50).

What these numbers show:

- **The warm edit is O(scope) (D).** The unchanged apply evaluates 0 documents and costs the same as a one-file edit at
  every size (736 against 771 ms, up to 20.07 against 20.15 s). Nothing about the edit itself is visible in the cost.
  - It costs 0.17, 0.18, 0.24 and 0.23 s of CPU per MB of scope source, slightly superlinear.
  - On a whole worktree, a one-file edit costs 20–22 s of CPU, against native's 0.22–0.28 s with all 234 rules: 80–90×.
  - This is T13 row 12's O(scope) work, at full scale (O72).
- **Native on the slice's own 87 clauses (M, `SH:20261102-172945-quiet-native87-*`, quiet).** `lowered-rules.py`
  cuts the pack to the 63 policies and 87 clauses the slice lowers. Native then finds the same violation counts as the
  restricted whole-pack comparison above (97,486 on worktree 01, and so on).

  | Scope | Native scan CPU | Native edit CPU p50 / p95 | Native peak footprint | Slice cold CPU | Slice edit CPU p50 | Slice peak footprint |
  |---|---:|---:|---:|---:|---:|---:|
  | 40 at 5% (315 files) | 0.6 s | 8 / 74 ms | 329 MB | 11.5 s | 0.77 s | 886 MB |
  | 40 at 10% (704) | 1.6 s | 16 / 82 ms | 390 MB | 25.1 s | 1.85 s | 1,188 MB |
  | 40 at 20% (1,379) | 3.5 s | 37 / 97 ms | 495 MB | 49.9 s | 4.81 s | 1,494 MB |
  | whole 01, 08, 21, 34, 40 | 36.7–45.4 s | 145–161 / 208–250 ms | 1.60–1.67 GB | 231.7–242.7 s | 20.1–21.6 s | 2.70–2.81 GB |

  Like for like, on whole worktrees (D):
  - the slice's cold audit costs 5.3–6.6× native's CPU;
  - its warm edit costs about 130× native's;
  - its footprint is about 1.7× native's.

  The whole-pack native numbers in the first table carry 147 more policies.
- **Memory against the 500 MB budget (M).**
  - The slice's process peaks at 886 MB on 4.6 MB of source and at 2.7–2.8 GB on a whole worktree. That is about
    580 MB and 2.4 GB above the process right after lowering (~305 MB RSS).
  - The edits raise the slice's RSS from about 1.6 GB after the cold audit to about 2.4 GB. On a whole worktree the
    edit phase's peak (2.72–2.82 GB sampled RSS) is above the cold audit's (2.47–2.54 GB).
  - Native with the whole pack also peaks at 2.7–2.8 GB.
  - Both are about 5× the budget at worktree scale (O73).
  - The slice's store on disk is 313–323 MB per whole worktree.

## Scripts to rerun

```
npx napi build --platform --release --manifest-path native/Cargo.toml --output-dir target-addon/addon
# Native then the slice, per worktree, each process at load < 3, then compare-findings and the tables:
benchmark/final-shape/slice-at-scale/run.sh <run dir> 40 01 08 21 34      # env: EDITS SAMPLE BACKEND OBSERVER ADDON ENGINES QUIET NATIVE_RULES
benchmark/final-shape/slice-at-scale/summarise.py <run dir>
# Native on the slice's 87 clauses alone:
benchmark/final-shape/slice-at-scale/lowered-rules.py <run>/slice/40/slice.json <out dir> packs/python/rules packs/python/candidates/rules
# The O70 reproducer:
cargo test -p rerail-engine --profile profiling --features metal-tests --lib -- --ignored --exact \
  content_store::tests::a_tape_write_waits_out_another_writer_as_the_store_writes_do
```

## Not done

- **All 40 worktrees, completed runs.** The tip addon fails on all 40 within seconds. With the O70 patch each worktree
  takes about 10 min (two processes, 8 edits at about 21 s each). A one-file edit costing 20 s is not a reasonable
  editor latency, so the runs stopped at 5 worktrees, as the task allows.
- **Metal and the `head` observer.** The harness supports both (`--backend metal`, `--observer head`). Neither was
  measured.
