# 0422: The Bitlex ladders keep their last step only

Status: decided (the user, 2026-11-18: "get rid of all of the old grammars"). Follows 0416.

## Intro

A ladder builds a grammar a step at a time: each step is a whole Bitlex file, the grammar so far
plus one construct. The tree kept every step, so it held many near-copies of each grammar:
Kotlin's 36 steps were 1.4 MB, and since 0416 its steps 1 to 35 did not even parse. A step that
is not the grammar is history, and git keeps history.

## The decision

Each ladder keeps its last step, the grammar its parser compiles. The steps before it are
removed from the tree; git's history has them, and the decisions that made them (0332's sections
for Kotlin, docs/parser-generators/24 for `.rr`) say what each added.

| Ladder | Kept | Removed |
|---|---|---|
| Kotlin (`kotlin_parser/bitlex_ladder`) | `36-no-back-fills.bitlex` | steps 1 to 35 |
| `.rr` (`rr_syntax/bitlex_ladder`) | `22-test-blocks.bitlex` | steps 1 to 21 |
| C directives (`c_directives/bitlex_ladder`) | `06-raw.bitlex` | steps 1 to 5 |
| The compiler's (`compiler/bitlex_ladder`) | `13-commentaires-bloc.bitlex`, `14-json.bitlex`, `15-jsonc.bitlex` | steps 1 to 12 |

The compiler's ladder keeps three: step 13 is its made-up test language complete, and 14 and 15
are two languages (JSON and JSONC), not one's versions; the JSON oracle tests use both, and the
engine's JSON parser compiles 15.

What a removed step tested, the last step tests: the ladders' checks (`check_ladder`, the `.rr`
steps against the engine) run on the steps that remain. The `.rr` tests that opened step 10
(`refuse M in L`) and step 12 (`reread R`) open step 22, which has both.

The last removed commit with every step: `d3070e67c`.

Since roll-up 20 (2026-11-20, 0484): Kotlin keeps no ladder. Its last step is the grammar itself,
`kotlin_parser/kotlin.bitlex`, with no step number.
