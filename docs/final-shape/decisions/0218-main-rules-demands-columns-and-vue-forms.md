# 0218: A demand keeps only what its call's arguments reach

Status: decided (2026-11-14, in the work on what decisions 0201 and 0202 left open: "please merge in the hub
again / Then work through all of these").

## Intro

The translator builds magic sets by hand. For a helper called with some of its columns bound, it writes a demand
relation, the rows the callers ask for. Each rule of the demand is the call's argument and the literals around the
call. A demand may be wider than the calls need. The helper's rows are joined back into the caller, so a wider
demand only costs rows. A narrower one would lose findings.

Before, a demand took every plain literal of the call's rule. Some share no variable with the call's arguments. A
literal like that joins in as a cross product. no-shadow's `vardecl` demand carried main's "is set" tests on the
metavariables it never binds:

```
// main: {"compare": {"left": "MV", "op": "eq", "right": "MV"}}, one of two alternatives
eslint_demand_vardecl(n) :- eslint_demand_alt_80(n, N), bind_name(n, b31),
    (text(MV, t1), text(MV, t1) ; text(VAL, t3), text(VAL, t3)),   // every text node, twice
    ( text(HK, t5), text(HK, t5) ; … ).                             // times every text node
```

Over 62 small React and TypeScript files, that one relation did 1.0 G rows of work and took 34 of the run's 39 s.

## Decision

1. **A demand rule keeps the literals connected to its arguments** (`json_rules::demand::connected`). The demand
   starts from the call's arguments. It keeps each literal that shares a variable with one already reached, until
   nothing more joins. This holds for both demand passes: `demand` (metavariable columns) and `node_demand` (a
   recursion over descendants or ancestors).
   - A literal's variables are its identifiers. Relation names, `@` functions and aggregate words are not
     variables. In a pattern (`` p ~ `$F2` ``), a metavariable names the variable it binds (`F2`).
   - The grounding check still runs after the cut. A demand it refuses is not made, as before.
   - A cut can make a demand possible where an unconnected, ungrounded literal blocked one before. AIR311's
     `quoted_name_param` and `seen_airflow` now have demands.

## Consequences

- no-shadow over 62 React and TypeScript files: 3.4 s CPU, against 39 s, with the same findings. Over 10 files:
  3.5 s against 4.2 s. What is left is mostly the program's compile.
- Of main's 31 rules, two translate differently (no-shadow and react-hooks-refs), and all 31 pass the reader.
  react-hooks-refs finds the same on the 62 files (nothing either way).
- Four Python rules change: S615 (bandit B615), AIR301, AIR311 and PYI051. All 537 pass the reader. On the 502
  Ruff fixture files each finds what it did (nothing). On a probe written to trigger them, each finds the same
  1 to 9 findings. AIR311 runs in 0.7 s against 0.8 s.
- The Ruff fixture comparison is as at 0201: F601 13 missing; F632, F722 and PIE796 find nothing; three codes have
  no generated rule; the other 10 are equal.
