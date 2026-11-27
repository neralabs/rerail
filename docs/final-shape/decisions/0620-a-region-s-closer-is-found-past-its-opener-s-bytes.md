# 0620: A region's closer is found past its opener's bytes

Status: decided (2026-11-22, on the standing order to make the Kotlin parser's Rust read as the lean
parser's does). Follows 0598 and 0600.

## Intro

**The problem.** A region's fill runs from its opener to the first byte of its closer *after the opener's
first byte*. Kotlin's raw string opens at `"""` and closes at `"""`, and the lean lexer looks for the closer
from the opener's end (`string_end`: `i += 3`). The two differ where the opener's own quotes make a closer:
`""""x"""`, whose run of four quotes is the opener and one quote of content.

The grammar made up for it in the closer: `rclose` was the last three quotes of a run, but never one or two
bytes past a run's first quote. That excluded the closing runs of four and five quotes too (`"""a""""`),
which the lean lexer closes at the run's first quote, the closer taking the whole run. An escape refused
them, and one corpus file with them.

## The decision

- **`O bytes N` on a region kind's opener** (`Region::opens`, 1 to 3; more than 1: no `outside`, no
  `else`). The closer is found past the opener's `N` bytes.
- **The rounds.** `f_K = fill(o_K, C & ~prev(o_K) … & ~prev(o_K, N - 1))` (`Region::closers_past`), in
  the group, as Metal runs it. The masks lowering makes the same ops; the block verifier knows the fill by
  them (`BlockProgram::fill_of`).
- **The walks.** The interpreter's (`walk_group`) looks from `t + N`. The JIT's (`native_cpu_walk`) masks
  the closer from `t + N`, and an opener in a block's last bytes carries the places past the block in the
  state's skip, as the hidden places are carried. The Rust writer's walk gate and its readers look from
  `s + N`; `opener_body` is `N - 1` at least.
- **The readers.** A reader's closer is its region's closer past the opener (`closers_past(o_K)`), in the
  lowering and in the Rust writer's tier-2 masks: a reader looks from the byte after its opener, as
  before.
- **The text form**: `region … opens N`; the digest covers it.
- **Kotlin**: `rstr = dq3 bytes 3 .. rclose …`, `rclose = dq3 & ~next(dq, 3)`. The quotes of a closing run
  before its last three are a part of no leaf (`r_lead`, `'""'`, implied in `string_literal`), as the lean
  lexer's closer takes them. The escape goes.

## Why

The criterion is the lean lexer's code: it reads a raw string's closer from the opener's end. The region
said something else, and the grammar's closer stood in for it with a rule that held for the opener's runs
and not for the closer's.

## What changed

- `tools/bitlex/src/syntax.rs`: `Region::opens`, its syntax, `closers_past`, `Regions::body`.
- `bitlex_masks.rs`: the fill past the opener; the readers' closers.
- `block.rs`: `WalkKind::opens`, the walk, the verifier, the digest, the text form; `rir_asm.rs` reads it.
- `native_cpu_walk.rs`: the closer from `t + N`, and the skip into the next block.
- `bitlex_lex_rust.rs`: the walk gate, the readers, `opener_body`, the readers' tier-2 closer.
- `kotlin.bitlex`: as above. `tier1.rs`, `tier2.rs` and `kotlin.answers` regenerated.
- Tests: `regions_tests::a_closer_is_found_past_its_openers_bytes` (the walk, the rounds and the JIT agree,
  openers at a block's end; it fails with either of the JIT's two parts taken out);
  `raw_strings_are_the_lean_parsers` (the closing runs, from refused to the same; openers at a block's
  last bytes).

The corpus: 487 the same, 4 refused, 0 differ (one more file the same).
