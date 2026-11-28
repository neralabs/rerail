# 0629: An `as` type splits with names alone in its angles

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows the `as` split's rules (`as_open`, `as_split`).

## Intro

**The problem.** After `x as a.b.C<…>` at a line's end, tree-sitter may read the angles as comparisons, and
the lean parser's `as_split` says where. Its rule: a dotted name, then `<`, a name and dots, then `>` that
ends the line. The grammar's `as_lt` ran from the `<` over the type's whole run, `?` and further angles
included. So after `<Int?>` or `<Map<A>>`, `as_open` held on the next line where no split's rule takes it,
and the file was refused. The lean parser keeps the type whole there and reads on. Both are shapes of real
code, such as `x as java.util.List<Int?>` and then a call.

## The decision

- **`as_lt`** stops at a `?`, at a `<` (which starts it again) and at a second `>` in a row:
  `fill(as_dot & "<", ~as_dot | "?" | "<" | gt & prev(gt))`. After `<Int?>` the run stops before the `>`.
  After `<Map<A>>` the run from the inner `<` stops at the second `>`. Either way no `>` that ends the line
  is in it, and `as_open` does not hold.
- **The `as_split` marks** keep their one run of tokens. A token mark takes one run at most, so the marks
  can't say "one `<`". They end `with as_open`, and `as_open` now says it.
- **Tier 1 as Rust**: `within_fill` takes `fill(F & Y, ~F | Z)`, with `Z` of stage 1 and at no skipped
  byte. At a token's first byte the fill is `F & (Y | before & ~Z)`. Over a token's bytes after the first
  (a `->`'s `>`, a `..<`'s `<`), a `Z` there is tested, and a `Y` there, which comes after it, starts the
  fill again.

## Why

The criterion is the lean parser's `as_split`. Left as they were:

- the keyword trials. After a split, the lean parser reads the next line on trial with keywords as names:
  `for (i in l) {}` is a call, `while (c) g()` keeps the type. The grammar refuses those lines. Saying it
  needs a mark over a whole line's expression.
- `a.B<C>.D<E>` at a line's end. The lean parser keeps the type; the run from the second `<` splits it.

## What changed

- `kotlin.bitlex`: `as_lt`. `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- `bitlex_lex_rust.rs`: `within_fill`'s stop `Z`.
- Tests: `as_types_the_lean_parser_does_not_split_are_its`: `<Int?>`, `<Map<A>>`, `as?`, two arguments, a
  dotted argument, a nullable type, `is`, and a call or a lambda after a split.
