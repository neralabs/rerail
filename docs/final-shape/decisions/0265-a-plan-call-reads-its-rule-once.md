# 0265: A plan call reads its rule once

Status: built (2026-11-15). Takes up decision 0258's first open item: a plan call's fixed cost. `plan.rs`
(`Model::plan_key_at`, `Model::live_sizes_at`, `Model::delta_misses`, `line_key`), `plan/body.rs` (`Body::key`).

## Intro

Each time a rule line is planned, the planner first decides whether it can reuse a kept plan. That decision reads:

- the line's keys: `plan_key` for its kept plan (and a live replan's, `"live "`), `line_key` for its kept order
  (decision 0258), and the caller's `reused_key`;
- its sizes (`live_sizes`): each relation's rows, each column's distinct values and fan, each constant's count;
- the joins whose keys cannot meet the delta's (`delta_misses`), which both the key and the sizes include.

This is paid by every line, kept or new. Decision 0258 measured it at about 18 µs a line, against 6.5 µs for a search.

## The measurement was mostly the profiler

The 18 µs came from the profile's spans. With the profile on, every `profile::decision` formats its text and every
span formats its name, and the planner makes several of each per line. A sampled run with the profile off (xctrace,
1 ms samples) gives the real cost.

O125 replay, `--profile profiling`, a cold run at step 11 and sessions 12 and 13, about 9,000 lines planned:

| | in plan calls | search and pricing | outside them |
|---|---|---|---|
| ms over the run | 157 | 95 | 62 |

So the fixed part was about 7 µs a line, not 18. Of its 62 ms:

| what | ms |
|---|---|
| the rule's digest (`Body::digest`) | 32 |
| `live_sizes`, outside `delta_misses` | 10 |
| `delta_misses` | 14 |
| the kept plans' lookups | 9 |

## What changed

1. **The rule is digested once per body.** A plan call hashed the rule's whole text for each of its keys: the
   kept plan's, the live replan's, the line's, then the caller's. `Body::key` digests it once, at the first ask, and
   every key writes that digest. `observed_key` keeps its value: it is the same digest, folded to 64 bits.
2. **The joins that miss the delta are found once per call.** `plan_key` and `live_sizes` each computed
   `delta_misses`. A plan call now computes it once and passes it to both (`plan_key_at`, `live_sizes_at`).
3. **`delta_misses` reads only what it can use.** It made each delta column's value set before looking at the
   other atoms. Now a column's values are made only where another atom shares its variable. An atom is tested by its
   terms first, and only then by its relation's name (`changes_with_the_delta` tests the suffix before the
   component's set of names).
4. **`live_sizes` finds each relation once.** It looked each column's statistics up by the relation's name, once per
   map (`ndv`, `fan`, `dfreq`), each a lookup in the global name table under its lock. It now finds the relation's
   number once per atom and reads the columns by it, as pricing does (decision 0209 §3).

The keys' values change (a key now writes the rule's digest, not its text). Kept plans are stored per prepared
program and per engine build, so a new build already starts without an old build's plans.

## Measured

The same run, the same build settings, two runs after the change. A 1 ms sample is coarse at this size: the
unchanged search part read 66 and 88 ms in the two runs.

| outside the search, ms | before | after |
|---|---|---|
| all | 62 | 36, 42 |
| the rule's digest | 32 | 7 to 10 |
| `delta_misses` | 14 | 6 |
| `live_sizes` | 17 | 9, 10 |

The fixed part is about 4 to 5 µs a line now. The digest left is each body's first: a body is made again at each
site that reads a rule (`Versions::at`), under 1 µs a line.

Session 13's outputs are the full run's ("same as full: true").

## Open

- **The rule's digest belongs to its version.** A rule's digest is the same in every body made of it. Kept in the
  version's image with the rule, no body would make it. At under 1 µs a line it is not worth the image change now.
- **What is left is spread thin**: the kept plans' lookups (a read lock and a clone of the kept entry), the sizes'
  vector, `kind_frequency`'s key (a string clone per constant). None is over 2 ms of the run.
- **The profile's planning picture is inflated.** With the profile on, a line's decisions and span names cost more
  than the work they describe. A planning measurement reads samples with the profile off, as here.

## Tests

- Logic crate `plan::` (141, 1 ignored).
- Engine `corpus::session` and `logic::plan::tests`.
- O125 replay, steps 12 and 13: "same as full: true".
