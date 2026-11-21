# 0554: The generated parser's cells are of constant widths

Status: done by the implementer. It is item 1 of the user's "go ahead with those two" (2026-11-20: constant widths in
the generated format 16 writer), asked again in the form 0551 writes (2026-11-21). How it is done is the
implementer's choice. Follows 0553. Page 40, step 57.

## What was there

The writer's ops (`writer16`: an open, a leaf, a close) read the language's widths from the writer's state at each
cell: each column's offset (`at`), each cell's width (`w`) and escape (`esc`), and wrote a cell as a word loaded,
masked and stored. The generated parser (0552) called them so.

## The decision

1. **The cell ops are generic over where their widths come from** (`writer16::CellWidths`):
   - `Run` reads the state's, as before. Every existing caller (the methods, the JIT's ops) is `Run`, so their code
     is unchanged.
   - `Fixed<S, G, L>` makes the widths, the columns' offsets and the escapes constants, and a cell of 1, 2, 4 or 8
     bytes is one store of its width. Other widths keep the masked word. The bytes are the same. A debug build
     checks the state's widths and offsets are `Fixed`'s.
   - `w_open_with`, `w_leaf_with` and `w_close_with` are the ops at `X`'s widths.
2. **The generated parser owns its language's widths.** It writes `pub const WIDTHS` (one byte each for JSON, the
   implementer's choice: a value past a width goes to its side list, as in every format 16 tape) and calls the ops
   at `Fixed` of them. The writer's language is to be made with `WIDTHS`; a debug build checks it.

## Checks

- `f16::tests::the_ops_at_constant_widths_write_the_state_widths_bytes`: three trees of up to 40 levels over
  300,000 bytes, with gaps and lengths past every width and codes past the cut, at widths `[1,1,1]`, `[2,1,4]`,
  `[1,2,2]`, `[4,4,8]` and `[3,1,1]` (the masked word's fallback). Format 16 and 16z are the methods' bytes, byte for
  byte.
- `json_tests::json_rusts_tapes_are_the_jits`: 196 tapes the JIT's, 106 refused by both (debug build: the widths'
  checks hold). JSON's tests: 24. `rerail-compiler`'s tests: 151. `rerail-tape-format`'s tests: 39 here, without the
  Metal test (it does not run here).

## Not checked

- Speed: nothing was measured, so whether the stores are faster than the masked words is not known.
- The commit, the block's settling and the side lists still read the state's widths.
