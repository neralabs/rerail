# Code-rule audit latency benchmark

This benchmark measures Rerail's code-rule runtime against the complete
bundled `nera-data-layer` rule corpus. It measures two separate operations:

- **Full scan**: construct a fresh code-rule workspace and scan every eligible
  source file in a repository. This includes code-rule workspace construction and
  uses the production 32-file/1-MiB batching shape.
- **Incremental evaluation**: load the same source into one resident workspace
  backed by a fresh or warm SQLite content store, then apply one changed source
  file at a time. The reported latency starts when the change request is
  encoded for the code-rule workspace call and ends when that call returns. The
  call evaluates the change and persists it to the content store before it
  returns, so SQLite persistence is inside the measurement; with
  `RERAIL_POLICY_PROFILE=1` set, the profile's `storageRead`, `storageManifest`
  and `storageWrite` fields attribute it. It does not include host-hook
  transport, reading the changed file from disk, or delivery rendering.

Every content store is a temporary file the run creates and removes: the cold
full scan and the cold incremental run each start from an empty store, and the
warm runs reuse the store the cold full scan populated. A run never reads or
writes Rerail's machine-wide store.

The separation is intentional. A repository scan is throughput work and an
individual edit is an interactive latency path. Do not compare their values.

## Run

The bundled rules scope their files as `nera-data-layer/...`, relative to the
root of the landing repository. Every `--repository` must therefore be a
nera-landing checkout with the `nera-data-layer` submodule initialised; pointing
it at the submodule directory itself puts no file in scope, and the runner
stops with an error. From the landing checkout root, initialise the pinned
data-layer revision and run against that checkout (`npm run` executes in
`rerail/`, so `..` is the landing root):

```sh
git submodule update --init nera-data-layer
cd rerail
npm ci
npm run build:native
npm run benchmark:audit-latency -- --repository ..
```

Pass one landing checkout for each repository in the campaign, each with its
submodule initialised. The runner reports one result per checkout, rather than
merging different repository sizes into an unusable average.

```sh
npm run benchmark:audit-latency -- \
  --repository .. \
  --repository /work/nera-landing-2 \
  --iterations 5 \
  --incremental-iterations 100
```

`sourceFiles` counts every file Rerail reads in the landing checkout;
`eligibleSourceFiles` counts the data-layer files the rules scope, and only
those are scanned and mutated.

The JSON result contains source-file count, eligible-file count, LOC, all raw
samples, wall-clock and aggregate process-CPU p50/p95/p99 values, and process
RSS before and after the run. Save
that JSON with the machine model, OS, Node version, Rerail commit, data-layer
commit, and the command line used. For rule-class attribution, add
`--rule-kind ast`, `--rule-kind regex`, or `--rule-kind select-when`; `all` is
the default and is the acceptance run.

## Acceptance targets and resource envelope

The runner fails when an incremental result exceeds either latency target or
its 500-ms aggregate CPU budget:

| Operation        | Wall-clock p50 | Wall-clock p95 | Aggregate CPU p95 |
| ---------------- | -------------: | -------------: | ----------------: |
| One changed file |         100 ms |         500 ms |       500 core-ms |

The CPU budget is **0.5 core-seconds**, not a requirement to restrict Rerail to
one core. Rerail may use bounded parallelism to reduce latency, provided the
aggregate CPU used by one incremental evaluation stays within 500 core-ms.
Run the campaign on a 16-GB machine and record peak RSS with the host's process
monitor. The runner fails when its final RSS exceeds 1 GB. `rssBytes` is not a
portable peak-memory measurement, so retain the host measurement too.

## Scale campaign

Use real, pinned checkout snapshots. Do not create synthetic copies of files:
rule scopes, parse trees, fact density, and violation rates must stay realistic.
Prepare a manifest of 1, 2, 5, 10, and 20 data-layer-sized repositories, with
up to 2M total LOC, each laid out as a landing checkout with its sources under
`nera-data-layer/` so the rule scopes apply. Run the command once for every
manifest prefix. The point
where p95 rises materially between adjacent prefixes, or where RSS reaches 1
GB, is the operational limit.

The benchmark deliberately does not invent a user-facing limit. Set it only
from the recorded campaign: choose the largest repository count and LOC range
that meet the incremental targets and resource envelope. If a checkout exceeds
that measured range, show this message:

> Rerail could not run all policy checks within this workspace's supported
> scale. Reduce the enabled rules or scan a smaller repository scope.

Record the exact limit and the result JSON in this document or the release
validation record when the campaign is complete.

## September 2026 profiling baseline

A deterministic stratified subset of the pinned data-layer checkout was used
for a small profile: 483 Python files, 161,371 LOC, and 100 top-level source
groups. It retains the original relative paths, so code-rule scopes still apply.
The checkout was `fce4dc7208c0b892c11a549c8be43fc285580d6b`.

| Enabled rules        | Full scan p50 | Incremental p50 | Finding          |
| -------------------- | ------------: | --------------: | ---------------- |
| All 104              |      2,201 ms |         45.4 ms | Baseline         |
| 57 AST rules         |      1,593 ms |         13.6 ms | Primary CPU cost |
| 46 regex rules       |       71.6 ms |          5.3 ms | Small cost       |
| 1 `select_when` rule |       22.5 ms |         0.01 ms | Not a bottleneck |

An unstripped, optimized native addon was built with:

```sh
CARGO_PROFILE_RELEASE_STRIP=none cargo rustc --release --target aarch64-apple-darwin -- -C debuginfo=2
```

A five-second macOS `sample` capture resolves the hot stack to
`evaluate_file` → `evaluate_ast_file` → `AstMatcher::match_node` /
`match_node_with_env`. The counters confirm the result for the 483-file scan:
306 ms Tree-sitter parsing, 1,712 ms AST matching, 50 ms regex work, 30 ms
final serialization, and 2.74M AST matcher calls.

For 20 individual edits, AST work was only 0.75 ms parse plus 1.16 ms matching.
Final result serialization consumed 594 ms total (29.7 ms/edit), which explains
most of the 48.8-ms incremental p50. Aggregate incremental CPU p95 was 51.1
core-ms, within the 500-core-ms budget. The first implementation work should
therefore return only changed-file violations on incremental evaluation, rather
than serializing violations for every resident file. The second should reduce AST
matcher calls through stronger root-kind/token prefilters and matcher sharing.

### Resident-workspace measurement

A fresh-process `benchmark:workspace-size` run loaded all 104 rules and the
full pinned data-layer source corpus: 5,736 files and 95.8 MB source. After
source input was released, process RSS was 4.10 GB. The workspace load added
3.78 GB above the already-loaded source, or **39.5x source size**; baseline to
resident RSS was 3.89 GB, or **40.6x source size**. This is process RSS rather
than an allocator-independent object measurement, but it is a one-workspace
run rather than the earlier repeated-scan high-water mark. It decisively misses
the 1-GB envelope.
