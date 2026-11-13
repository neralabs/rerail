# t19: Typing support, rounds 8–9: open opportunities

## Intro

The typing spike measured magic sets in rounds 8 and 9, which came after t9 was written. Those rounds added:

- Soufflé-style magic sets over the whole program;
- splitting `@cat` for demand;
- non-key adornments;
- gated negations;
- delta-first plans for plain SQL.

All of it has been copied to this branch (from `Typing-support@7ebcffe0`) and ported:

- `logic::magic` has `answers`, `gate_negations`, and `Options` with `nonkey` and `input_first` on and `gate` off;
- `logic::native::setfix` has the `Split`, `Count`, `Lt` and gate steps;
- `logic::plan` has delta-first `semi_naive_round` and the `solving` floor.

What was not carried over is the list of what those rounds found worth doing next. t9's "Stated open items"
stops at round 7, and the open-items ledger has no row for any of it. This note keeps that list.

The measurements are in the spike's `spikes/datalog-engines/rdl/RESULTS.md`, sections "Round 8" and "Round 9".
That file is on branch `jeremybarnes/Typing-support` (backup ref `jeremybarnes/Typing-support-spike-r9`), not on
this branch. All times are CPU seconds of execution, without construction, on a loaded machine (load 19–54).

## Where things stand

The programs and the reference outputs are T's `calls.rdl` (Python) and `calls_rust.rdl` (Rust). Every result
below matched them.

| Run | Native program mode | Plain SQL |
|---|---|---|
| Python, all files, as written | 1.84–1.87 s | 16.9 s |
| Python, all files, as written, magic sets | 3.08–3.26 s | 34–36 s |
| Python, all files, no hand demand, magic sets | 2.57–2.64 s | over 280 s |
| Rust, all files, as written | 1.21–1.25 s | 9.4 s |
| Rust, all files, as written, magic sets | 2.11 s | 13.8–14.3 s |
| Rust, no `wants`, magic sets | 2.67–2.74 s | over 280 s |
| One-file query, as written | 1.78–1.81 s | 16.8 s |
| One-file query, no hand demand, magic sets | 1.20–1.28 s | 9.1–11.0 s |

Without hand-written demand, magic sets now come within 1.4–2.2× of the hand-tuned programs on whole programs,
and beat them on a one-file query. What follows is what is left between those numbers and better ones.

## Opportunities

### 1. Demand that covers the whole domain

Some magic relations are seeded from inputs alone and end up as large as the relation they guard. `tgt` is the
case seen here: every binding target is demanded. That relation and its guarded copies are pure overhead.

- **Change:** estimate a magic relation's size at transformation time, from the seeds' statistics. Compute the
  relation in full (no adornment) when the estimate reaches its domain.
- **Where:** `logic::magic::magic_adorn_once`, which would read `logic::plan::Stats`.
- **Validated by:** `tgt` and its dependents unguarded in the transformed query, with outputs unchanged and fewer
  tuples than 1.45M.

### 2. Magic-set overhead on programs that already have demand

On the as-written programs, magic sets add 0.9–1.4 s natively (1.85 → 3.1 s for Python, 1.2 → 2.1 s for Rust)
and double the time in SQL. The tuple counts barely move. The magic relations and guarded copies are paid for
nothing. Hand demand also defeats the transformation: `need_attr($c, $a)` is asked with both arguments bound, its
rule reaches `$c` only through `pv`'s value column, and `pv` goes full.

- **Change:** decide per relation whether to adorn, using the same estimate as item 1. Leave a relation unguarded
  when its demand is not expected to be much smaller than the relation. Or drop the hand-written demand rules
  from the ported programs, since magic sets now replace them (round 8: 2.53M tuples against the hand-tuned
  2.51M).
- **Where:** `logic::magic`. The programs are in `typing/programs/*.rr`.
- **Validated by:** magic sets costing no more than no magic on the as-written programs.

### 3. Cheaper gated negations (`Options::gate`)

Gating is exact and computes no relation in full for negation. The query falls to 0.43M tuples from 1.45M, but
takes 3.2–3.4 s against 1.20–1.28 s. Three things cost the time:

- about 200 waves, because each gate waits until its whole quiet set is still;
- 581 probe replans across those waves;
- item 1's `tgt` demand, which gating makes larger.

On whole programs gating only adds time (Python without demand: 2.7 → 5.2 s; Rust: 2.9 → 3.4 s), so it stays off.

- **Change:**
  - replan only the probes whose input relations changed since the last plan;
  - open a gate as soon as the relations its own negated relation reads are quiet, instead of the whole level's
    quiet set;
  - then item 1.
- **Where:** `logic::native::setfix`, the gate promotion in `run` and `reprobe`; `logic::magic::gate_negations`
  for the quiet sets.
- **Validated by:** a gated query under the ungated 1.2 s. Then try gating on by default for queries.

### 4. Non-key adornments the `answers` check refuses

`answers` grants a non-key adornment only when every rule can reach its derived atoms by key. `sbinds` by
statement qualifies. `base` by base class does not: answering it reads `pv` by value ("which nodes hold this
value"), which spread demand to every scope. Without the check the query took 13.8 s.

- **Change:** a name-keyed index (value → nodes) as an input relation, maintained at ingestion, so the reverse
  lookup is a probe instead of a derived spread. The same applies to `ibind` and `home`, which are reached by name.
- **Where:** fact production (stats/ingest), and `logic::magic::answers` treating such indexes as keys.
- **Validated by:** `base` adorned by base class in the transformed query, with fewer full relations.

### 5. Plain SQL plans from first-round counts

The SQL fallback plans each delta variant once, from the first round's counts. Without hand demand that fails.
In Python, `need_attr`'s delta reads `pv($o, "inst", $c)` by value before the `magic_ext_attr_b($n)` guard that
would restrict it. Whole programs without demand take over 280 s. Replanning when a relation doubles, with fresh
distinct counts and fanouts (T's `RDL_SQL_REPLAN=1`, not ported), brings Python to 117 s. But it slows the
as-written programs from 17 s to 71 s, and Rust stays over 280 s.

- **Change:** replan from observed cardinalities, as the native engine does after every chunk, but price a
  replan's new plan against the old one before switching. The 71 s came from replans that were worse. O69 is the
  same gap for waves over derived relations.
- **Where:** `logic::plan`, `semi_naive_round` and `refresh`.
- **Validated by:** no-demand programs with magic sets in SQL within a small multiple of the native times, and
  the as-written programs not slower.

### 6. Small engine gaps found on the way

- **`lt`/`le` over symbols:** these native steps take only int and node operands. A comparison over `sym` makes
  the rule fall back to SQL. Ordering interned symbols needs their text order, not their ids (`logic::native::setfix`,
  step parsing at `"lt" | "le"`).
- **Aggregates force a full relation under demand:** a guarded count would join its demand's recursive component,
  so `logic::magic` computes every aggregated relation in full (`nopaque` counting `opaque_dec`). The native `count`
  step could count per demanded key instead.
- **Determinism of the fallback:** the negative-cycle fallback lands on 8 full relations for Python without
  demand. Some other stratification orders needed only 4. Choosing the order to minimise the fallback's closure
  is open. A one-step lookahead cost 1 s and picked a 26-relation closure, so it needs a cheaper estimate.

## Tried and rejected (do not retry without new evidence)

- **Joined sideways order** (prefer the atom that binds more arguments): 2.16M tuples, worse than the
  input-first order (`Options::input_first`).
- **Adorning only key columns:** 38 s, since `need_attr` needs `$a` bound.
- **Moving the delta in every native replan:** 12 s. A chunk runs several waves on one plan, and later deltas
  are smaller. Each probe instead keeps a second plan for the whole-relation wave.
- **SQL replans by default:** see item 5.
