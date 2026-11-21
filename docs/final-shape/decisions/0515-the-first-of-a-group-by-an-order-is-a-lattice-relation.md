# 0515. The first of a group by an order is a lattice relation, not a self-join

Status: accepted (2026-11-20), built. t50 step 2; follows 0511.

## Intro

The rules pick "the first of a group" the way Soufflé programs do: a row is kept when no other row of its group comes
before it.

```
rf_method_first(c, f, r) :- rf_method_at(c, f, r), !rf_method_later(c, f, r).
rf_method_later(c, f, r) :- rf_method_at(c, f, r), rf_method_at(c, f, s), @strlen(s) < @strlen(r).
rf_method_later(c, f, r) :- rf_method_at(c, f, r), rf_method_at(c, f, s), @strlen(s) = @strlen(r), s < r.
```

W does it with a loop over the order that returns at the first hit (`Graph::lookup` over the MRO). The engine made a
row of `rf_method_later` for every pair of a group's rows, so its work grew with the square of the group.

The engine already has the operator: a lattice relation keeps one value a key and joins each new one in
(`min<>`, `max<>`, `shortlex_min<>`, decision 0108). `rf_order` uses `shortlex_min` for the least route.

## Decision

1. **The shape** (`rr::lower::first`), recognized in a module before its declarations are read:
   - `F(k…, v) :- X(k…, v), !L(k…, v), <literals that do not read v>`, F's only rule;
   - every rule of L is `L(k…, v) :- X(k…, v), X(k…, w), <comparisons of w and v>`. X may have other columns, read as
     `_` in the same places in every use;
   - L's rules together state one order:
     - `w < v` gives min, `w > v` max, on a number column;
     - `@strlen(w) < @strlen(v)` together with `@strlen(w) = @strlen(v), w < v` gives shortlex min, on a symbol column.

   Symbols under `<` alone are text order, which no lattice keeps, so that shape is left as written.
2. **The rewrite.** F's value column becomes the order's lattice, and F's rule drops `!L`. L loses its declaration
   and rules when nothing else reads it. Nothing changes in `.rerail`. `RERAIL_FIRST=off` leaves the shape as written,
   for measuring.
   On rr-align a statement is its tape's node, read in place (decision 0276, stage 6 A), so the rewrite is what the
   lowering reads past (`rr::lower::first::Firsts`): L's statements are left out, F's value column is read as the
   lattice, and F's `!L` literal is not read. "Nothing else reads it" is the module's: no other statement names L
   (an `.output`, an aggregate's body, a relation given as an argument), and L's rules do not read L.
3. **Unbounded lattices need `widen k` only where they recurse.** A key's value changes once a batch when no rule of
   the relation reads it back, so nothing widens (`front::finish_checks_given`). The check before refused every
   unbounded lattice relation.

## Found on the way

- **`ShortlexMin` joined a stored atom to the top.** It looked a symbol up only among the atoms this process holds in
  memory. The set engine's symbols read the store as well (`Interner::known`). With 10⁴ routes, ids past those held
  joined to TOP. It now reads the store too (`atoms.value`). The coverage probes made a few texts each, so all were
  held, and the gap did not show.
- **The device folded a bounded aggregate (0511) as a plain min or max.** 0511 said the device refused it, but its
  compile matched the step with `..` and never read the bound. The coverage gate's device column showed the wrong
  rows (it ran by mistake: see Measured). The compile now refuses a bounded step. That has not been run on a device,
  since Metal work waits on the 2026-11-20 crash.

## Measured (`all`, 3,089 files, same host, 2026-11-20)

Every stage's rows are as before the step. The 29 `look`, 11 `ref` and 1 `open_name` differences t50 lists stay, with
no edge or open definition changed.

| Strata (`RERAIL_STAGE_TOP`) | before s | after s |
|---|---|---|
| `rf_method_first`, `rf_method_later` | 0.202 + 0.256 | 0.252 |
| `rf_super_first`, `rf_super_later` | 0.157 + 0.220 | 0.182 |
| `rf_field_first`, `rf_field_later` | 0.026 + 0.046 | 0.040 |
| **together** | **0.907** | **0.474** |

The stage totals moved with the host's load (8 on 16 cores) more than with the step.

The coverage gate was run once without `RERAIL_COVERAGE_NO_DEVICE=1`, so its probes ran on a farm Mac's Metal device,
against the order to run nothing on Metal. That run showed the bounded-aggregate bug above.

## Tests

`plan::first_tests`:

- the shapes recognized, with their order: shortlex, max with a literal on the keys, min written the other way round,
  and X with a wildcard column;
- the shapes left alone: a literal reading the value, another relation, half the shortlex order, text order,
  wildcards in different places. A later-relation something else reads is kept;
- the rows on a group of 10⁴ routes and 300 small ones: each group's shortlex least, as W's loop finds it, natively
  and on SQLite. The later-relation is not in the program.

The reference stage cases, projects and fixtures, and the findings replay sample, pass.
