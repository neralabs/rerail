# 0217: A relation held as kept is planned with the statistics the engine kept of its columns

Status: built (2026-11-14), `plan.rs` (`quiet_card`, used by `hold_quiet` and the quiet strata of decision 0101 §1).
It narrows decision 0101 §4: a kept relation's statistics are still made only when a plan first prices it.

## Intro

In a session, a stratum that an edit does not reach is quiet, and its relations are held as they were kept
(`hold_quiet`; decision 0101 §1 for a maintained stratum whose deltas are all empty). The planner is given each such
relation's row count. It looked for column statistics only under the relation's own name or its `__old` rows. A
quiet relation is bound under its own name with neither measured, so it kept the row count alone.

With no distinct counts, the planner takes each column as holding one value (`ndv_or_1`). A probe of any column then
reads as returning all the relation's rows. So every order that reached the relation by a bound column looked
expensive, and the search chose orders that scanned it first.

```text
aknown__sup4__del(t, m, h, a, w) :- aleaf(t, m), fl412_acls(m, a),
                                     builtin_ref__old(h, 10121), pv__minus(a, 3689, w), ahead(m, h).

aleaf: 12,779 rows, no column statistics   -> aleaf(t, m) by m priced at 12,779 rows a probe
pv__minus first: 3 rows, then fl412_acls (3), then aleaf by m: 3 x 12,779 = 38,337 ...
chosen: aleaf first (12,779), fl412_acls (2,475), builtin_ref__old with h unbound (1,739,925), pv__minus (0)
```

O125 step 14 (one file edited) spent 1.76 M rows of join work on this rule, against 1,862 for all of `aknown__sup4`
in the cold run. `rpvar__sup0__del` spent 0.83 M against 13 k. Together they were 62% of the step's cross-file work
(notes t39).

## What changed

The engine keeps statistics of a derived relation's rows as they come (`InRel::kept_stats`). A relation held as
kept, with no statistics of its own or of its old rows, is now given those, as lazy statistics
(`Stats::lazily`). The planner reads them when a plan first prices the relation, as it does for kept relations
under 0101 §4.

They are given only where the engine kept every column's statistics. The planner reads a relation's columns all or
none (`input_stats`): a relation with some columns missing (`tvar_of`) failed "no statistics for tvar_of.c0".

A relation still held with its row count alone is counted (`unmeasured_quiet`).

## Measured

O125 replay, `--profile profiling` build, unprofiled, all 20 steps. Every step ended "same as full".

| step | files | cross before (G) | cross after (G) |
|---|---|---|---|
| 14 | 1 | 7.07 | 3.73 (−47%) |
| 16 | 2 | 7.19 | 4.91 (−32%) |
| 15 | 1 | 4.77 | 4.43 (−7%) |
| all 20 | 84 | 121.5 | 116.9 (−3.8%) |

The other steps moved within noise. In step 14 (profiled), the join work fell from 4.24 M to 1.03 M rows. Rules that
read a delta after other literals now take 1% of it, from 61%.

## Tests

- `a_session_passes_on_a_quiet_stratum` asserts that no quiet relation is held with its row count alone. It fails
  with the kept statistics switched off.
- Engine: `logic::plan::tests`, `session`, `corpus`, `waves`.
