# Explore X2: compact records with one evicting interner (design b)

Branch `Moti-Velz/facts-engine-x2-compact` from `eee7d6ac3`. Runs: `~/rerail-bench/facts/explore/x2-compact/runs/<timestamp>-<label>/`. Specs: `~/rerail-bench/facts/explore/x2-compact/specs/` (S3's `w6` specs with the rule paths pointed at this worktree).

## Decision-table row

| Design | Memory: load / end of 300-event prefix / after 1,000 edits | Per-edit latency (single-file, 300-event prefix) | Exactness | What it breaks | Kill point | Commits |
|---|---|---|---|---|---|---|
| (b) Records as fixed-width `u32` columns, one evicting interner. Combined with nothing else | **Load: footprint 10.2 GB, RSS 13.0 GB.** 146.5M records at 55.1 B/record of columns. **Prefix end: footprint 10.7 GB, RSS 14.0 GB.** 171.9M records at 53.3 B/record. **1,000 edits: not run** (killed). [INFERENCE] S3's 252.8M records × 53 B ≈ 13.4 GB of columns, plus about 0.9 GB of interner and rows: about 15 GB. Baseline at the prefix: footprint 32.6 GB, about 201 B/record | p50 36 / p95 99 / p99 343 / max 496 ms (n = 298). Load average 34–44 on 18 cores, unlocked. Baseline: 62 ms p95 locked, 77 ms in P3a's unlocked profiled run | **0 differences** on the prefix:<br>- light, every stage (33 outputs): 151 checks, 165 worktree checks<br>- dead code, every stage (97 outputs): 7 checks, 9 worktree checks<br>Sweeps ran after every event and freed and reused 4,045 + 1,139,578 symbols. Dead-code parity **4,931 = 4,931**. 0 runtime refusals | Nothing in I3, I6, I8 or I9. Two caveats: the representation limits are asserts, not named refusals (2^30 symbols, 2^15 tags, 2^32 fixpoint rounds); and the sweep is exact only if every holder of a symbol is marked, which nothing type-checks. T0 (`cargo test -p facts-b`): 16/16 | **Killed at the 300-event prefix.** Resident is 10.7 GB footprint / 14.0 GB RSS, over the 10 GB line. Bytes per record pass: 53 by the column census, 60 by footprint, 80 by RSS (§ 15.2's method), against the 64 limit | `60b2c5cf0` (code); this report |

**X1 + X2 together (arithmetic only, nothing run).** X1 with all cuts holds 104.08M records at load and 122.83M at the prefix end (X1Prune's runs `…/x1-prune/runs/20261023-161101-all300`).

| | Load | Prefix end |
|---|---|---|
| Records × this design's bytes per record | 104.08M × 55.1 B = 5.7 GB | 122.83M × 53.3 B = 6.5 GB |
| \+ interner, row cache, store (0.92 GB) | **6.7 GB** | **7.5 GB** |
| If X1's column demand also narrows rows by its 21 % (201 → 158 B at load), on this design's 39 B row payload | | about 45 B/record: **6.4 GB** |

Either way the combination stays about 25–30× over 250 MB. The interner alone (630 MB of live strings) is 2.5× the target.

## 1. What was built (disposable)

- **Values** (`gen/val.rs`). `Val` is 32 bits: a 2-bit tag; integers in [−2^29, 2^29) inline; text a symbol; larger integers and reals indexed into a number table. Rows outside arrangements stay 24-byte `Row`s with 4 values inline.
- **Arrangements** (`gen/col.rs`, wired in `gen/lower.rs`). Every keyed arrangement, presence, group output and output capture uses a custom differential `Layout`. There is no per-record `Row`, `Vec` or `String`. A `consolidate` capture is a registered presence arrangement.

| Part | Representation |
|---|---|
| Keys | `RowCol`: one flat `Vec<Val>`, `width` words per key, no offsets. A column whose rows differ in width falls back to `u32` offsets (no value column did at the prefix) |
| Values | `TagRowCol`: a `u16` tag column (the refusal bit is bit 15) and a fixed-width row column |
| Presence values | `TagCol`: `u16` tags only |
| Times | `Times<Tm>`: `u32`, or `(outer, round)` packed into one `u64` instead of the 16-byte `Product` |
| Diffs | `Lean<i32>`. Differential's builder reserves one slot per update; its singleton optimisation fills few of them |
| Offsets | `Offs`: one bit per item while every step is 0 or 1, with a rank directory every 512 items; any other step switches to `OffsetList` |
| Key columns | `Splice`: columns a key reads bare (`Col` under `Group` or affinity-free `Eq`) are left out of the stored row and read back from the key. A record whose value differs from its canonical key (an integral REAL) is stored whole; the stored width tells the two cases apart |
| Exported rows | The row cache is flat per (export, path, content) (`col::Flat`) |

- **One evicting interner** (`store::Interner`).
  - Strings the store interns (paths, names) and everything interned before the membership model are pinned. Strings computed by the dataflow (`val::sym`) are not.
  - The sweep (`drive.rs` `gc`) marks every symbol and number held by the registered arrangements' batches, the exported-row cache and the outputs. It then frees the rest: symbols go on a free list and are reused, and the arena is compacted once half of it is dead.
  - Arrangements register a trace handle whose compaction frontiers are empty, so the handle never holds compaction back.
  - Policy: sweep when the symbols created since the last sweep exceed a quarter of the live ones. `FACTS_SWEEP_MIN`/`FACTS_SWEEP_RATIO` turn this into a sweep after every event, the stress setting.
- **Census** (`results.json` → `x2`). Exact column bytes per arrangement, split by component and by value-row width (`census_loaded`, `census_end`, `top`, `by_row_width`), plus the sweep log.

## 2. Numbers (all 5 plans, 40 worktrees, 300-event prefix, `--no-oracle`)

Each step is cumulative; v5 is the committed code. These are column bytes per arranged record, from the census.

| Step | Run | Load B/rec | Prefix B/rec | key | offsets | row | tag | times + diffs | Prefix footprint / RSS |
|---|---|---|---|---|---|---|---|---|---|
| v1: `u32` columns, 4-byte tags, `OffsetList`, `Vec` times | `20261023-160253-mem-all5-300` | 69.3 | 67.0 | 4.6 | 6.0 | 42.6 | 4.1 | 9.7 | 13.9 / 13.2 GB |
| v3: + bit offsets, `u16` tags, no reservation for times/diffs | `…-164820-mem-all5-300-v3` | 60.6 | 58.6 | 4.6 | 2.2 | 42.6 | 2.1 | 7.2 | 12.7 / 13.4 GB |
| v5: + key-column elision, packed iteration times | `…-171046-mem-all5-300-v5` | **55.1** | **53.3** | 4.6 | 2.2 | **39.1** | 2.1 | 5.4 | **10.7 / 14.0 GB** |

Where the 53.3 B go at the prefix end (v5, 171.9M records, 9.17 GB of columns):

- **The row payload is 39 B per record.**
  - The 118.9M keyed records store rows a mean **14.1 words** wide, even after elision.
  - Rows 20 or more words wide (42.8M records) hold **5.4 GB, 59 % of all column bytes**. The largest are `n1228:iter-join-l` (32 words, 0.96 GB), `n103`/`n1014:group-out` (31 words, 0.57 GB each, identical in two plans) and `n102:by67`/`n1013:by253` (28 words, 0.52 GB each, identical).
- **Presence: 53.0M records at 18.6 B each** (key, tag, offsets, time).
- **Times and diffs: 5.4 B per record.** The fixpoints' arrangements keep one update per iteration round, so differential's singleton optimisation does not apply to them.

Outside the columns:

| Item | Size | Note |
|---|---|---|
| Interner | 630 MB, 3.15M live strings | Unchanged by eviction (below) |
| Flat exported-row cache | 254 MB | Was 670 MB as `Rc<[Row]>` (P3a's baseline) |
| Number table | 0 entries | No integer outside ±2^29 and no real reached a row |

RSS exceeds footprint by about 3 GB in every run. [INFERENCE] These are pages the allocator has freed but macOS still counts as resident (merge outputs freed after a spine merge). The machine was in swap throughout: 30–31 of 32 GB swap used, and other slices held 30, 12 and 11 GB. Footprint is the comparable number; P3a's baseline reports both.

**Eviction.**

- At load the sweep freed 322K symbols after level 0 (strings computed by the membership model's weights) and 19K at the end of the load. 3.15M strings stay live, because records hold them.
- Over the prefix, the default policy (created > live / 4) never fired.
- In the stress runs it fired after every event: 208 sweeps freed 4,045 symbols in the light run, and the checks reused their ids with 0 differences.
- The sweep's pause is a mark pass over every arranged word: 2.0 s at load, 146.5M records. Under the default policy that pause is rare. In the dead-code stress run (about 150M records with every stage captured), the 300 sweeps paused 1.8 s on average and at most 3.0 s. They ran outside the timed window.
- Eviction caps the interner's growth: S3's full replay grew it 600 → 688 MB. It cannot shrink the live set: those 630 MB are the strings live records hold, averaging about 170 B. [INFERENCE] Most are dead code's computed qualified names and link keys.

## 3. Exactness

| Check | Result | Run |
|---|---|---|
| T0 `cargo test --release -p facts-b` | 16/16 | — |
| Same, which includes `norm::tests::generated_*`: dataflow ≡ reference over random tables with NULLs and duplicates, through the columnar layout and the splices | pass | — |
| Same, `col::tests::{offs_round_trip, rowcol_round_trip}` | pass | — |
| Dead-code parity (v4: elision and `u16` tags) | 4,931 = 4,931, 0 missing, 0 extra, rows identical to S3's run `20261023-085817-parity-dead-code-w6` | `20261023-170334-parity-dead-code-v4` |
| Light-plan parity | 0 = 0 for the 4 clauses, identical | `20261023-170334-parity-light-v4` |
| Light, incremental ≡ from scratch, every stage (`FACTS_STAGE_OUTPUTS=1`, `--check-every 2 --sweep 5`), sweep after every event (v4) | **0 differences**: 151 checks, 165 worktree checks, 0 runtime refusals, 208 sweeps, 4,045 symbols freed and reused | `20261023-170334-exactness-light-stages-v4` |
| Dead code, incremental ≡ from scratch, every stage (97 outputs incl. the linker views; `--check-every 50 --sweep 1`), sweep after every event (v5) | **0 differences**: 7 checks, 9 worktree checks, 0 runtime refusals, 300 sweeps, **1,139,578 symbols freed and reused**; footprint 9.6 GB at the end (dead code only, every stage captured) | `20261023-172619-exactness-dead-code-stages-v5` |

The from-scratch side is the same lowering, so it cannot catch a representation bug that both sides share (S3 § 15.1 makes the same point about mutant 6). The parity runs and the T0 reference vectors are the independent checks of the representation.

## 4. Why killed, and what the numbers say

- **Payload, not encoding, is what remains.** The encoding now costs about 14 B per record: key share 4.6, offsets 2.2, tag 2.1, times and diffs 5.4. The rest is 39 B of row payload at 4 B per value: the plans arrange rows 14 words wide on average, and up to 32.
  - The next byte has to come from fewer records (X1, X3, X4) or narrower rows (X1's column demand), not from encoding.
  - Per-column compression inside a batch (dictionary, frame of reference) would give up `&[Val]` reads and the fixed-width design; it was not tried.
- **Resident at the prefix: 10.7 GB footprint, over 10 GB.**
  - [INFERENCE] The remaining ~1–2 GB of headroom would not change the verdict: 250 MB at about 50 B/record allows about 5M arranged records with an empty interner, against 172M today and 123M with X1.
  - The interner alone (630 MB live) is over the target. Its strings would have to become ids (S3 § 15.2 item 4). That is a rule- or language-level change; I3 forbids the engine recognising which strings those are.
- **Latency held, but is not comparable.** p95 was 86–148 ms across v1–v5 at load averages 23–83 on 18 cores, all under the 500 ms target. The work is the baseline's: the same arranged records, with decoding and splicing added. No locked A/B was run.

## 5. Interface changes (disposable branch)

- `gen::col` (new): `RowCol`, `TagRowCol`, `TR(W, &[Val])`, `TagCol`, `TU(W)`, `Times`/`Packed`, `Lean`, `Offs`, `Flat`, the arrangement registry (`registering`, `register_rows`, `register_pres`, `census`).
- `lower`:
  - Arrangements are `col::RowSpine`/`col::PresSpine`.
  - `Source` returns `Rc<col::Flat>`.
  - Group logics take `&[Val]` rows.
  - `keyed` returns `(Arr, Splice)`.
  - `capture` arranges into a registered presence.
- `val`: `Val(u32)`; `pin_all`, `sweep`, `mark`, `syms_created`, `nums_footprint`.
- `store::Interner`: `intern` pins; `intern_computed`; `pin_all`; `sweep`.
- `greplay`:
  - `results.json` → `x2` (census, sweep log). Each `rss` sample adds `census`, `interned`, `interned_bytes`, `exported_row_bytes`.
  - Env: `FACTS_SWEEP=0`, `FACTS_SWEEP_MIN`, `FACTS_SWEEP_RATIO`.
