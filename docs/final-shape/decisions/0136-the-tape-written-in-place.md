# 0136: Tier 2 writes the tape directly

**Corrected by 0140 (2026-11-13).** What this decision built is not the tape. Tier 2 writes a
table of node rows (`NodeRow`), a format of its own; the engine's tape is the v9np image's
bytes. The rows are written in place, as below, but a tape from them still needs the engine's
passes. The "lean parser too" part below made the lean parser write the same rows, which was
wrong: it now writes the image's bytes (0140).

Status: built (2026-11-12). Tier 2, JIT: 330.4M instructions to 248.3M, 58.4M cycles to 41.0M.
Ahead of time: 307.0M to 224.8M, 54.3M to 36.4M.

## Intro

**The problem.** The compiled parser's tier 2 wrote its node rows as it parsed,
then made the tape at `ACCEPT` in passes over the whole table:

- a wrap (an operator found after its left operand, `a - b`) was written at the table's end
  and linked into a chain; at the end, every row was copied into a second buffer in preorder;
- a dropped node stayed as a dead row until that copy left it out;
- a last pass wrote each row's parent and extra flag.

The lean parser writes its tape directly, each node in its place as it goes.
Avoiding a second packing pass was the point of writing a tape.

**The decision.** Tier 2 keeps the table as it stands at the end, as the lean parser does:

- **A row's parent and extra flag are written with the row.**
  The parent is the innermost open node; the flag comes from the kind.
- **A wrap goes in place**, before the node it wraps.
  That node is the last made at its level, so its rows are the table's tail
  (with the pending extras after it). The tail moves up a row, and parents in it follow.
  A wrapped leaf is one row moved, inline; a larger tail is an outlined function.
- **Pending extras** (comments deferred to the next node) move out to the parent
  at each close they outlive.
- **A dropped node is taken out** by the word that drops it, over the tail that word walks.
- `ACCEPT` makes no reorder and no table pass. The second row buffer is unused.

```
a - b - c        made:   [x a]  →  [x W1 a]  →  [x W2 W1 a]
                 each wrap inserted before the node it wraps, in place
```

**A reread meeting a drop.** A reread (`RETAG m=3`) reads a node and the nodes inside it.
The interpreter kept a dropped row in place until `ACCEPT`,
so a reread over it met a node no reading names, and refused; the reference refuses there too.
In place the row is gone, so a row carries a mark, "a dropped row was taken out
from among my children", moved with the rows.
A reread that reads on into a marked row refuses.

## How

- `rust_stream::node_words`, `accept`, `reread`; `marks` (whether rows carry marks:
  a wrap's inner field, or a reread).
- `native_cpu_stream`: the node words; `TableFns`, the outlined `tempo_stream_shift`,
  `tempo_stream_unpend` and `tempo_stream_drop`. Each is made only where the program needs it.
  The reorder, the table pass and their frame and chain state are gone.
- The interpreter (`stream::run`) keeps the deferred form: it is the test oracle.

Tests: every tape over the corpus is the engine's; the `.rr` ladder against the reference and the
engine; JSON's compiled path makes no call; wraps in chains, over large operands, among comments,
and a drop then a reread (`the_table_is_written_in_place`), on the JIT and ahead of time.

**The lean parser too.** It wrote the engine's nodes (`tape::Node`: no size, an optional
parent), so a tape needed a pass after it (sizes from the last node back).
It now writes the tape's rows (`tape::Row`), the compiled parser's 24 bytes:
parent and extra flag at each push, size at each close, the root's at the end.
`rr_parser` checks the two layouts are one and compares the tapes byte for byte.
Its benchmark: 356.5M instructions to 362.4M, 68.0M cycles to 69.3M, for the sizes
it did not write before.

Open: `ACCEPT` still reads each row's field once, to refuse a provisional field left
(`.rr` has one). A count kept at the field writes would remove that read.
