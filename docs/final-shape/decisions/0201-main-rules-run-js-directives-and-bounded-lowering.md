# 0201: Main's other rules run: `.js` read as TypeScript, ESLint's directives widen globals, lowering stays bounded

Status: decided (2026-11-14, in the work to run main's rules directly: "Fix these", on what decision 0194 left
open; "decision: keep this the same as main for now", on how `.js` is read).

## Intro

Decision 0194 brought 30 of main's 31 JavaScript, TypeScript, React, Vue, Rust and SQL `binds` rules through the
reader. Reading is not running. Several of the 30 were refused at run time, read no file, or did not fit in memory:

```
eslint-no-eval        resolves: {global: ["eval"], directives: "eslint", scriptDeclarations: true}   // refused
any .js rule          .language typescript, .scope "**/*.js"                                         // 0 files
clippy-many-single-char-names   check-rr past 9 GB                                                   // 101,423 rules
typescript-eslint-explicit-module-boundary-types   line 304: t5_2_8_10_16 ungrounded                 // reader
```

This decision fixes those, and the faults found on the way.

## Decision

1. **`.language typescript` reads `.js` too, as main's does.**
   - The registry's TypeScript language runs on `typescript`, `tsx` and `javascript`.
   - The `logic` command now runs a program once per dialect of its language, as `run_program` already did
     (`tools::dialect_runs`, `corpus::run_dialects_in`).
   - Each further dialect's findings join the first run's `violation.csv`.
2. **ESLint's comment directives are library terms in `tsx/names.rr`.**
   - `directive_named(doc, "global" | "exported", x)` reads the `eslint-directive` guest.
   - `directives: "eslint"` widens `builtin` and `exported` (`site_builtin_directives`,
     `site_exported_directives`), as main's does. It is no longer a conjunct of its own.
   - `resolves.global` with `directives` and `scriptDeclarations` is `global_ref_with(r, x, directives, script)`:
     - a `/* global x */` comment adds x, and `x: off` takes an environment global away;
     - in a script, a reference to a top-level declaration is the global itself.
   - `resolves.this: "global"` is `global_this(t)` with `sloppyFunctions`, and `global_this_top(t)` without
     (`tsx/eslint-a-globals.rr`).
   - `es_global`'s rows are `tsx/es-globals.rr`, generated from the engine's own ECMAScript table
     (`scope/data/es_globals.generated.json`, `ecmaVersion: "latest"`). A test holds the two equal.
3. **A literal tree's parent runs over its children on the tape** (`literal::format_tape`). Main's directive tree
   puts `b:off`'s item over `b` and its setting over `off`, past the item. The tape refused the file ("invalid tape
   parent or span"). Main's tree is unchanged.
4. **The translator:**
   - An `ast` that is itself `{$option, cases, default}` (no-eval's `allowIndirect`) is one `ast` whose rule is
     the `$option`. Before, it matched every node.
   - `stored` beside a `regex` is main's note, not a term.
   - A util whose name core.rr declares is `<name>_util`. A util `field` or `value` had taken the rule's reads of
     the core `field(p, f, c)` and `value(n, x)`. Seven Python rules read them again.
   - A metavariable a callee reads that its call site never binds is unset there, as an unbound ast-grep
     metavariable is empty. That also covers a call that passes `_`. So explicit-module-boundary-types' `$LATE`,
     bound only under `late-checked`, has a demand through the recursive group, and its `early-enough` holds
     when it is unset.
   - A Message's `$N` column is `n_mv` when `n` is the rule's node.
   - The unset fallback applies only to a metavariable that nothing in the call's scope mentions. One bound by a
     literal the demand drops (a literal that reaches the demanded group) is bound at the call, so the helper has
     no demand. Before, no-shadow's `isval` was asked only for unset `$MV`, `$TI` and `$VAL`, and the rule flagged
     nothing. react-hooks-immutability's `same-ref` had been asked for an unset `$SITE` that
     `mutating-fn-name` binds.
   - `redefines` on a node's `binds` ties the binding's name to that node: `text(n, x)`, or `ident_pg(n, x)` in
     SQL. A `let (a, b)` site binds both names, and before, a redefinition of `b` counted against `a`. Python keeps
     no tie, because `import a.b` binds `a` at a node spelled `a.b`.
   - In SQL, `compare` with `text: "name"` reads `ident_pg`, the name PostgreSQL folds an identifier to. SQL
     identifiers have no `value`, so `FROM t AS a JOIN t AS b` read as a self-join of two different tables.
5. **rr lowering stays bounded:**
   - **Disjunctions become helper relations.** When a rule's alternatives would multiply past 16 (`OUTLINE_PAST`),
     a disjunction's alternatives become a helper relation `rr_or<k>`. Each alternative that grounds the variables
     it shares becomes one rule of the helper; the helper is one atom in the rule. An alternative that leaves one
     of them unbound stays in the rule.
   - **`within_stop`.** It is two alternatives: n has no stop above it, or its nearest stop is not strictly below
     a. Before, it was three. Past the bound, over a kind set, it is the shared relation `rr_wstop_<S>(n, a)`. That
     relation has at most `within`'s rows.
   - **Aggregate demand.** It no longer walks the dependency graph for every literal of every consumer: a
     target's reachers are found once.
   - Below the bound, a program lowers as it did. The 79 data-layer policies outline nothing.
6. **The reader refuses a read of the wrong arity.** `front::rule_declared` checks each atom's column count
   against its declaration. Before, the planner panicked at run time.

## Consequences

- All 31 rules pass the reader, and all 537 of Python's still do. The Python pack changes in 38 files:
  - column renames (`n_mv`);
  - seven `_util` renames that restore core reads;
  - demands that carry an unset metavariable.
- The Ruff fixture comparison has the same per-code results as at 0194: F601 13 missing; F632, F722 and PIE796
  find nothing; three codes have no generated rule; the other 10 are equal. (0194 recorded these as "11 equal";
  the list it describes has 10.)
- The ten Python rules whose translation changed in substance find the same as before on the one-file corpus.
- Probes:
  - **no-shadow** flags the inner `const y` under a top-level `y`, and the inner `let x` under a parameter `x`.
  - **clippy-many-single-char-names** with the name tie flags lines 2–6 of `let a; let (a, b); c; d; e`, and
    the three-function probe as before. no-redeclare finds the same on its JS and TS probe as without the tie.
  - **no-eval** flags `eval(…)`, `var x = eval` and, with `/* global window */`, `window.eval(…)`; a file with
    `/* global eval: off */` now runs.
  - **clippy-many-single-char-names** lowers to 1,458 rules, not 101,423. It runs in 11 s CPU and 1 GB on a probe
    of three functions, and flags only the function with five fresh single-letter names.
  - **explicit-module-boundary-types** runs, and flags lines 1, 7, 8 and 9 of a nine-line probe, as
    typescript-eslint does.

Open:

- explicit-module-boundary-types misses `export const d = () => 1` and `const e = …; export { e }`. Its program is
  46,904 components, 2.3 GB on 109 nodes: the translator inlines the reference utils into each other (variable
  names nested eight deep), and the outlining bound does not reach them. The translator should keep a util used
  more than once as its relation.
- clippy-many-single-char-names is still 11 s on 106 nodes. Its compile, not its rows, is the cost.
- `OUTLINE_PAST` is a bound, not a cost model. An outlined alternative reads all of its rows, where the rule's own
  literals narrowed it. 19 of the first 200 Python rules outline at 16.
- Still open from 0194: no-shadow's type parameter; the Vue view.
- AL05 on sqlfluff's 47 default-config fixtures: 5 of the 7 expected findings, and 5 more. The self-join pass
  (`test_pass_repeat_referenced_table`) is now right. The 5 extra are quoted aliases (`"A"` against `a`, a
  sqlite alias in backticks). `ident_pg` folds as PostgreSQL does, and ANSI and these dialects fold otherwise.
  The 2 missed are `AS 'a'` read as `a`, and `TO_JSON_STRING(t)`, a table alias passed as a value.
- react-hooks-immutability does not run under `RERAIL_SQL_STRICT`: its aggregate's support stratum compares
  `$F < TOP` (`rr_agg119__sup0`), a comparison the native engine has no step for. This is so with and without
  the demand change above.
- eslint-no-unused-vars (a candidate rule) is refused at run time: `resolves: {write: "last"}` has no lowering.
