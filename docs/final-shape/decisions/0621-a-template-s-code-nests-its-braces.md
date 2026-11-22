# 0621: A template's code nests its braces

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0600 and 0620.

## Intro

**The problem.** A Kotlin template `${…}` ends at the `}` that closes it, not at the first `}` after it.
The lean lexer's `interpolation_end` counts the braces in the template's code: `"${x.f { it.name }} b"`
holds a lambda, and the string goes on after the second `}`. The regions had no count. A tail (the
string's rest after a template) opened at the first `}` in the template's code, so the grammar refused
every template with a brace in it, by an escape. Four corpus files were refused, two of them for this.

A count is not a mask of bytes about a position. It is state carried along the stream, as the regions'
own fills are, so it belongs to the regions, not to a mask the grammar writes.

## The decision

- **`regions H nests O .. C depth D else E { … }`** (`Regions::nest`, `RegionNest`; `D` from 1 to 15).
  Outside every region, `O` opens a level and `C` closes the innermost one. A string's template (a kind
  closed at its second closer, a `$`) opens a level whose kind is that string's; a tail (a kind with
  `after`) opens only at the closer of a level of its string's kind, with no level inside it.
- **The rounds** (`Regions::body`). A level `j` is a carry `H_lv{j}`, with one per string kind
  (`H_lv{j}_{K}`) for the kinds a tail names. `in_j` is "at level `j`, not deeper". A level opens at a
  template's `{` (`H_t`) or at a bracket `O` (`H_b`) where the level below it is innermost, and closes at
  `C` there: `next lv_j = fill(push_j, close_j)`. A tail's opener holds at a level's closer whose kind is
  its string's: `open &= OR_j(pop_j & prev(lv_{j,K}))`. `E` is a push past `D` levels. The levels read the
  hide expression, not the hidden carry, so every way from a carry to its next goes through a `prev`
  (strict causality, as eval asks).
- **The lowering.** A group with a nest lowers through the generic rounds, with no `Walk`: the walks know
  no count. With a nest, a resuming kind names exactly one kind, and no kind has `else`.
- **The Rust writer**, phase 1 and 2: a stack of `D` levels, each a string's kind or a bracket's
  `usize::MAX`, as `interpolation_end` keeps its count. The gate walk pushes at a bracket before the next
  opener and at a string's `$`, and pops at `C`; a tail's arm pops first. The token loop pushes and pops at
  a token's start, and a tail's close at its `$` pushes its string's next level. Past `D` levels, the gate
  walk sets `E`, and the token loop refuses.
- **Kotlin**: `regions hidden nests lbrace .. rbrace depth 4 else t_deep`. The escape for braces in a
  template goes; `escape "a template's code nested past 4 levels" when t_deep` refuses the rest, which the
  lean parser reads.

## Why

The criterion is the lean lexer's code: `interpolation_end` counts braces and recurses into strings. Only
carried state says that, and the regions are where the grammar's carried state is.

The depth is 4 and not 8: each level adds its carries to every proof the compiler asks of the group, and
at 8 the cold compile did not finish in 4 minutes. At 4, the warm compile (the shipped answers) takes what
it took before (17 s at O0 under load, against 17 s), and a cold one proves about as many answers. The
corpus has no template deeper than 2.

## What changed

- `tools/bitlex/src/syntax.rs`: `RegionNest`, its syntax and checks, the levels in `Regions::body`.
- `bitlex_masks.rs`: `nested_regions`, the rounds with no walk.
- `bitlex_lex_rust.rs`: the stack in the gate walk and the token loop; `E` refused in the loop.
- `kotlin.bitlex`: as above. `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- Tests: `braces_in_templates_are_the_lean_parsers` (lambdas, blocks, braces in strings and characters,
  line breaks, raw strings; five levels refused).

The corpus: 489 the same, 2 refused, 0 differ (two more files the same). The two left are templates with
templates in their code, which need the string's held record (0600) to become a stack.
