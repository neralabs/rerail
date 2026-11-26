# 0231: The compiler chooses the records' layout

Status: built (audited 2026-11-23). Decided (2026-11-14); landed on the CPU (rows for rr) and on Metal (columns). Part
of 0177's token loop.

## Intro

**The problem.** Tier 1 writes one record a token: its first byte, its last
byte and its code. Tier 2 reads them back, a token at a time. Until now the
records were always three columns: a column of first bytes, one of last
bytes, one of codes.

On the CPU that costs registers. The token loop holds three column bases,
and in the generated loop they were among the values spilled to the stack
each token (about 12 spills a token in all). The lean lexer has one base:
its `Tok` is a row, the three fields side by side.

On a GPU the opposite holds. A group of threads writes a token each, and
their stores coalesce only when each field is a column.

**The decision.** The layout is the compiler's choice for each program, by
who writes the records. No backend assumes one.

- **Rows** (12 bytes a token, start, last, code) where one thread's token
  loop writes all three fields of each token and they are the program's only
  extracts, so nothing else shares their room.
- **Columns** otherwise, and wherever threads write a token each (Metal's
  block passes).

## How it is described

`records::RecordLayout` is the one description, with `choose(writer, whole)`
taking who writes (`RecordWriter::TokenLoop` or `RecordWriter::Lanes`).
Every writer and reader asks it where a field is:

```rust
// Token k's field (FIELD_START, FIELD_LAST, FIELD_CODE), in u32s from
// the file's first position. `columns` are the three columns' numbers.
layout.index(field, k, columns, capacity)
// The bytes from one token's field to the next token's: 4 or 12.
layout.stride()
```

The room is the same in both: rows take the three columns' place.

## Where each side reads it

| Part | What it does |
|---|---|
| CPU tier 1 (`native_cpu_block`) | `record_layout` chooses; the loop's stores go through `FileCols::field`, one base for rows |
| CPU batch results | `Batch::results` reads extracts and codes through `index` |
| Tier 2, Cranelift | `Ctx::record_stride`; a token's record at `next * stride` |
| Tier 2, AOT Rust | the same field, `rs` in the generated `S` |
| Metal | `Resident::layout`, from `choose(Lanes, …)`: columns; `tier2_on` reads through `index` |

## Checks

- `rrs_records_are_rows_on_the_cpu`: rr's program gets rows, and its tape
  is the lean tape.
- The tier 1 and tier 2 parity tests (the interpreter's records, the
  engine's tapes, the AOT's tapes) run on rows unchanged.
- `tier1_on_metal_equals_cranelift` runs Metal's columns against the CPU.

## The cursor

Rows indexed as `base + k * 12` gave little: a `mov #12; madd` a token, and
the record's address, made once, spilled across the kind's dispatch, as the
last and code stores come after it. The loop now carries a cursor
(`FileCols::cursor`, `at`, `step`): in rows the record's address, bumped by
12 a token, as the lean lexer pushes a `Tok`; in columns the index. It is an
index again only where it leaves the loop: a part's state and the count.

## Measured (once, reported, not a gate)

`jit_against_aot` over the corpus, 2026-11-14, instructions a node (the
machine's load was 80 to 90, so its cycles are not compared):

| | Columns | Rows, `k * 12` | Rows, a cursor |
|---|---|---|---|
| Tier 1 JIT | 285 | 280 | 259 |
| Tier 1 AOT | 193 | 190 | 186 |
| Tier 1 + 2 AOT | 435 | 437 | 433 |

## Why the spills stay

The profile of the generated loop (f0) shows the hot stack operations went
from 27 to 19, but the heaviest remain, and the cursor is now spilled
itself. The registers two bases freed were spent elsewhere. The cause is
what the loop carries:

| | Live around a token |
|---|---|
| Lean lexer's run loop | about 10: the run, the pairs, the window's word mask and bytes, the base, the output and its length, the cursor, the end, the table |
| Tier 1's loop | 10 carried (the cursor, the end before, the refusal, the pending marks, where the gap marks start, the word before, the block's starts, three block words) and about 8 invariants and 2 vector constants |

The marks' and checks' state (the refusal, the pending marks, the gap's
start, the word before, the hits) goes round every token. The lean lexer
has none of it on its common path: comments and checks are its rare
tokens'. The heaviest samples are also not spills but the first byte's
table load and the kind's dispatch after it, a chain of compares.

Tier 2 multiplies by a stride it loads (its program is compiled apart from
tier 1's). Baking the stride in where the two are compiled together is
open.
