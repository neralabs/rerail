# 0218: Demands keep what their arguments reach; a column's type through `=`; a witness's end point; export lists are references

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

2. **An rr helper column's type follows `x = y`** (`rr::lower::type_through`). A context or helper relation's
   column takes its type from the literals that bind it. When only an equality binds it (react-hooks-immutability's
   `rr_ctx120($F, …) :- …, $F = $n`), the type is the other variable's. It was `sym`, the fallback. An ordered
   comparison refuses a symbol column, and guest hiding bounds every node column with one (`$F < 2^62`). So the
   aggregate's support stratum had no native step, and the run aborted under `RERAIL_SQL_STRICT`.
3. **`end_point(n, p)`: the point where a node ends** (core.rr). p is an empty span at n's end offset, with its
   line and column. A finding can sit on it. The syntax vtab places one per node after a document's lines, only for
   a program that reads `end_point` (`Locs::ends`), so no other program's ids move. It is read off the node's own
   span, not a token, so a token the tape leaves out cannot move it.
   - The translator writes main's `witness.end` as `end_point(witness, at)`. Before, it was `@place(path,
     end_line)`, a place with no column.
4. **An export list's name is a reference** (`tsx/names.rr`). In `export { e }` with no `from`, `e` reads the top
   level's e, as main's scope makes it one reference of both spaces. Its `ref` space is `"dual"`: the lookup stops at
   a scope that declares e as a value or a type, and `ref_site` joins either. Readers of value references
   (`free_ref`, `env_global`) do not see it, so a type-only `export { T }` is no free global.

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
- react-hooks-immutability runs under `RERAIL_SQL_STRICT` on the 292 React fixture and probe files where it aborted
  before (rc 134), in 5.7 s CPU, with 3 findings.
- explicit-module-boundary-types, on its nine-line probe, flags lines 1, 4, 5, 7, 8 and 9, as typescript-eslint
  does. `export const d = () => 1` is at column 19, the end of `()`: main's `witness.end`. `const e = () => 1;
  export { e }` is now flagged at line 5.
- Of main's 31 rules only explicit-module-boundary-types translates differently; all pass the reader. The Python
  pack's goldens are unchanged. Main's four Vue rules match 130 of 142 fixture findings, as before; no-shadow finds
  the same over the 62 files.

