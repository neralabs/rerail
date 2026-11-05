# 0036: Rules are Datalog; tree axes are built-ins computed as tapes

Status: accepted (2026-11-04, the user; list step 30)

## Intro

The 60 data-layer rules are being turned into Datalog programs, with no regex, SQL or code. Most
of a rule is plain Datalog: joins, negation, name sets and tests on names. The open question was
the tree axes: `inside` (an ancestor of some kind), `has` (a descendant of some kind) and
`stopBy`. Each could be written in Datalog itself, as recursion over `child` or as span
containment, or provided as a built-in computed by the transducer, as `nearest` already is.

## Decision

- **Rules are Datalog.** That covers joins, negation, recursion through facts, name sets
  (constant relations), string tests (`@prefix`, `@suffix`, `@contains`, `@glob`), types and
  options.
- **Tree axes are built-ins, as `nearest` is.**
  - They are `inside($n, $a, kindset)` and `has($a, $n, …)`.
  - They run as tape arithmetic in the transducer, and the rules read them as relations.
- **The NFA is optimized** as in the Pulling-it-all-together workstream, and the built-ins lower
  to the appropriate tapes.
- **If needed, an optimizer** lowers Datalog rules that could be tapes into tapes, probably
  through transducers.

## Evidence

`measure_inside_three_ways` asks "calls inside a function" over nera-data-layer `2b1504ad`
(9,010 files, 28.2M nodes, 104 chunks). It is CPU, at load 43–87, and every way finds 999,607
rows.

| Way | CPU of the rule phase | The inside test alone (less the baseline) |
|---|---|---|
| baseline (every call, no test) | 11.87 s | — |
| tape (`nearest`, the transducer's preorder pass; CPU solver) | 13.29 s | 1.4 s |
| Datalog recursion (ancestor closure over `child`, native engine) | 18.17 s | 6.3 s (4.5×) |
| Datalog interval join (span containment) | 51.51 s | 39.6 s (28×) |

- **Interval join:** span containment uses no index, so each document joins every call with
  every function. It is the slowest way, not a middle way.
- **Recursion:** the native engine's recursion is far better than SQLite's recursive CTE (03 §7:
  4.5 s against 4 ms over 1.2M nodes). It is still 4.5× the tape.
- **Prior work agrees:**
  - The AST-index spike's design index (§7 and the tape's shape) has preorder plus subtree size,
    a nested interval: a descendant test is an O(1) range check on the tape, not a join.
  - The lessons ledger's row 22 says ordered sibling semantics need an NFA, not independent
    `cpos <` joins.
