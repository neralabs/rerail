# Tier 2 as a table: a spike

## Intro

Tier 2 turns tier 1's tokens and masks into the node table. Today it is generated code: the
compiler lowers each of the program's words (`TOKEN`, `SWITCH`, `LEAF`, `PUSH`, …) to its own
block of Cranelift code, and one word goes on to the next by a direct jump. For `.rr` that is
10,063 words and 151,050 machine instructions, about three times the CPU's level 1 instruction
cache.

The other shape is a table: a handler per kind of word, the words kept as data, and a loop
that runs each word's handler in turn. Its code is small. The cost is a dispatch per word:
an indirect jump on the word's place.

This spike measures that trade on the `.rr` corpus (1,362 files, 1.53M tokens), on the CPU,
before choosing a shape.

## What was built

Behind `RERAIL_TIER2_DISPATCH` (spike branch only), the same lowering in a dispatched form:

- One handler per distinct word: its operation with its constants (a kind, a field, a mask,
  a target). `.rr`'s 10,063 words have 2,022 distinct values.
- The word's place, `pc`, a variable. A word that goes on to the next (`fall`) or the one
  after (`skip`) dispatches on `pc` through one jump table. A constant target sets `pc` and
  jumps straight to its handler.
- Nothing else lives across the dispatch: the run's state (`start`, `end`, `nodes`, `open`,
  `ret`, …) is in the frame, and each handler loads the pointers it uses from `ctx`.
- The stream verifier still refuses a cycle of words without a `TOKEN` or `RETURN`; each
  handler is a named head for the shape check, which still sees inside every handler.

It parses as the interpreter does on every ladder step (the compiler's 60 tests), and as the
reference and the engine on the `.rr` ladder; the corpus and JSON tests pass.

## Measured

Tier 2's CPU a pass over the corpus, debug build, nice 19; the machine's load was 28 to 60,
so the ratios matter more than the times. The variants:

- **Static:** today's code.
- **State in memory:** today's code with the run's state in the frame.
- **Dispatched:** one handler per distinct word.
- **Direct:** dispatched, but a `fall` or `skip` jumps straight on where every place sharing
  the handler goes on to the same handler.

| Tier 2 | Instructions | Stack loads/stores | CPU a pass |
|---|---|---|---|
| Static | 151,050 | 62,188 | 47.9–51.8 ms |
| State in memory | 122,462 | 42,749 | 47.4 ms |
| Dispatched | 53,807 | 12,208 | 189–196 ms |
| Direct | 53,510 | 12,061 | 177 ms |

The words run: 12.07 a token, about 11.8M dispatches a pass.

Getting the dispatched form small took three steps:

1. With the state in SSA variables, every one of the jump table's 10,063 edges carried 14
   values. Cranelift split each edge into a block of moves: 263,839 instructions.
2. With the state in memory, the pointers loaded once at entry were still reloaded on every
   edge (161K).
3. With per-handler pointer loads, Cranelift kept the unused loads. It drops only loads
   marked movable, and those LICM hoists back out of the loop. A small pass removes them.

## What it says

- **The dispatch is the whole cost.** Moving the state to memory costs nothing in the static
  shape, so the 3.8× slowdown is the indirect jump per word, about 12 ns each at this load.
  Jumping straight on where the next handler is known saves only 7%.
- **Code size is not tier 2's limit on the CPU.** A third of the code, with no instruction
  cache pressure to speak of, runs four times slower.
- **Keeping the state in memory is a free 19% cut of the static code** (and a third of its
  stack traffic). Worth taking into the static shape.
- **A table that dispatches per word does not pay on the CPU.** Grouping per arm (one dispatch
  a token, not twelve) would be the static code's shared arms again.

Not measured here:

- **Metal**, where the trade differs. A kernel the size of today's is costly to build and to
  cache. Divergence counts distinct handlers: 2,022 is far too many, so it would need handlers
  that take their operands from the table (about 40 kinds).
- **A grammar much larger than `.rr`.**
