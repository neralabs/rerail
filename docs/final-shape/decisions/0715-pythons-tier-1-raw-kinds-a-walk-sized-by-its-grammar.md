# 0715: Python's tier 1 in the lean form: raw strings as kinds, a walk sized by its grammar

Status: in progress (2026-11-26, at the merge of Parser-generators into Go-C-and-C-support). The user
chose the lean form for Python's tier 1 ("the token loop keeps the count's depth, so marks are asked
at gaps; product builds walk no regions") and option A for raw strings. Follows 0698, 0699 and 0703.

## Intro

Parser-generators brought Python's grammar (0698) and its Rust tiers. This branch's generator (0699,
0703) refuses a part mask the walk cannot write in every region, and runs stage 1 and the walk in
test builds only. Python's grammar did not compile under it:

- **Rawness was a fill.** `in_raw` ran a fill from a raw f-string's quote across its tails. The walk
  cannot write a fill in a tail, whose own token has no prefix. Every other place in the tree carries
  a property decided at an opener as part of the string's kind: Kotlin's `rstr` and `str2` (0699),
  rr's `t1` to `t3`, and the lean Python lexer's `Quote { raw, … }` stack entries.
- **The walk's state was packed in fixed fields.** A kind took 4 bits, and a frame was 4 bits of kind
  and 4 of level. Python with raw kinds has 26 kinds.
- **Several reads inside a region token had no form:** a shift back to the token's own opener, a
  closer byte read against a closer shorter than it, `escaped` inside a token, and `escaped` whose run
  reaches the opener (a tail's `}`).

## The decision

- **Raw strings are kinds** (`python.bitlex`): `rs1`, `rd1`, `rs3`, `rd3`; `rfs1` to `rfd3`; and the
  tails `rts1` to `rtd3`, each resuming its raw f-string. `in_raw` is the union of their `f_` masks.
- **The walk is sized by its grammar** (`block::WalkLayout`). The carried word holds:
  - the open kind, in bits(kinds);
  - 2 hidden bits;
  - a bit a kind for the last template;
  - the frames, each a template's number among the template kinds (bits(templates)) and its level
    (bits(max)), times the nest.

  Python's walk is 5 + 2 + 26 + 4 × (4 + 3) = 61 bits. The limit is the one 64-bit word, and a walk
  past it is refused with each part's width named. A kind's `after` is a `u32`, so at most 32 kinds.
- **The loop keeps a count's depth** where its regional terms are the loop's events (a template's
  close, an opener at a token's start). A gap mark's `C.frame(a)` is then read as the count before the
  token (`cb_C`): one case where a frame is open, one where none is.
- **Reads inside a region token:**
  - a shift back reaching the token's opener reads the opener's bytes, not another token's end;
  - a closer byte is read against the closers that have that byte, each over its own bytes, as
    `fits_closer` reads them (0699);
  - `escaped(a)` past the token's first byte is the run of `a`'s body value, where `a` holds at none of
    the opener's bytes;
  - where that value may hold at an opener byte (a tail's `}`), the run starts after the opener: at a
    fixed offset, a finite expression of `prev`s; elsewhere `escaped_from(b, opens)`, which only a read
    knowing the token's first byte makes (the field's windows, the token loop's reads of its bytes).
- **The escapes made of `code` after the loop:** a whole-file pass they read (`//~pass`) ends their
  pass before it and stands between the two. A test of `code` at a byte is written `bit(&code_l, q)`,
  and becomes the spans' read only where the passes fold into the loop.
- **Supertype classes in a writer that passes nodes on** (0434 with 0698): `Writer::keys_by(classes)`
  numbers each node passed to format 16 by its classes, from its slot and final kind; a node retagged
  later is numbered again. `map_code_keys` ends it.

- **A fused loop steps a count at its rows** (at the merge of e8c4cb277a): with `skip ws`, Python's rows pick the
  region a token opens (`picks.fused`), and no `open` exists before the pick. A count's step at a region opener
  (`scan_starts`) is written in each row's arm, its `open == K` folded against the regions that row opens
  (`fold_open`). The count before the token (`cb_C`) is still read before the pick.
- **A token of `pair empty` is one byte:** no byte after its first can be its (`row_tail_bytes`), so a gap mark's
  mask over the tokens of code is clear of Python's `bad` token.

## Not yet

The product build still walks the regions, for seven escapes tested over the file that read its
masks: `\N` in a bytes string, a raw string of backslashes only, an empty interpolation, a `}` alone,
a comment in an interpolation, an interpolation's `=`, and past 7 frames. Each is being made a check
of the region tokens that read it, or of the loop's own count, as Kotlin's are. (The number escapes
are gone: Parser-generators' da0f2e4188 made a number a run with a check at the token.)

## Tests

- `rerail-python-parser`: every `_are_the_lean_parsers` test, the refusals, the shipped answers and
  `rust_is_current`. Its tier 1 comparison asks the derived and spanned marks as Kotlin's does.
- `rerail-kotlin-parser`'s 74 lean comparisons, `rerail-rr-parser`'s quick tier (the chunked tests
  included), `rerail-json-aot` and `rerail-compiler`'s library tests, all on the new walk layout.
