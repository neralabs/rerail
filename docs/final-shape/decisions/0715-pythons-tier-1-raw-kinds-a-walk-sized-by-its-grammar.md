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

## At the merge of Parser-generators 3ca03306c4 (2026-11-27)

- **Raw strings: the reader's flags, not kinds of their own.** Python's strings are a reader (`read string { … }`,
  0698), which Bitlex lowers to plain kinds (`s1` … `fd3`, the tails, `spec`). Rawness is a flag the reader keeps
  on its template state in the loop, as the lean lexer's `Quote { raw, … }`. That keeps this decision's invariant (a
  tail's rawness is its template's, never a fill a tail reads) another way, and the raw kinds above are gone. The
  walk's layout sized by its grammar (`WalkLayout`, 32 kinds) and `O bytes N` stay.
- **A count kept by the loop at region tokens: one stepper.** A grammar with a reader has its count stepped by the
  reader's arms (`scan_regional`: down at a tail's start, up at an interpolation's `{`). A grammar without one keeps
  this decision's form (`scan_starts`, stepped at each row that opens a region). They never both step a count.
  A gap mark reads a count as `C.eq(k)` (0698) or by its frame (`cb_C`, above).
- **The seven escapes below are the reader's refusals**, each tested where its place is (`refuse … at …`). Python's
  product tier 1 has no whole-file pass: stage 1 and the walk are a test build's.

## At the merge of Parser-generators-final-shape 7f4f5f7115 (2026-11-27)

Python's tier 1 has no stage 1 (0698 step 88): each block's masks are made where the loop reads them. With this
branch's generator the tier is 1931 lines (Parser-generators' own: 2269).

- **`escaped(B)` and `fill(S, E)` at a byte:** in a partial window (`partial_windows`, Kotlin's, 0703) they are read
  back to the run's nearest start (`esc_back`, `fill_back`); everywhere else by Parser-generators' walk back over the
  bytes. One read each, chosen by the window.
- **The bytes form's gate:** stage 1's finals may be its errors alone (`finals_read`), the counts' scans stay where the
  loop reads a count (`keep_scans`), beside this branch's region ends from inside (`enter_fns`, `chunked_lean`) and
  Kotlin's row starts (`row_starts`, `row_firsts`).
- **A count's read at a run's end and at the found close** stays, with Parser-generators' error naming the mask.
- **Python's records in the tests:** the marks tier 2 asks of the bytes (`UNRECORDED`) and the derived and spanned marks
  are both left out of the comparison: the mask is `unrecorded`, apart from the closure adding the derived marks.
- Kotlin's checks of a token's escapes now search `A & fill` by `A`'s bits (`bfirst_in`): its tier regenerated.

## A lexer meant to run from the middle has no whole-file pass (2026-11-27)

- `bitlex_lex_rust::middle::whole_file_passes` reads a generated tier 1's product build: it drops every item and
  statement under `#[cfg(feature = "test-hooks")]`, takes the functions `lex` reaches, and names each whole-file
  pass in them (`fill`, `escaped`, `any`, a whole-file `stage1_best`, a loop over every word or byte).
- `the_lexers_meant_to_run_from_the_middle_have_no_whole_file_pass` holds it for rr, JSON, Kotlin and Python.
  `a_whole_file_pass_is_found` checks it finds each form, and none under `test-hooks` or in an unreached function.
- Its first finding: Kotlin's product build made the file's word bits (`lv`, over every word) that only its test
  build reads. The generator's gate for `lv` now runs last in the chain (`interleave::lv_for_tests`) and asks the
  product build's code through the same stripping (`middle::product_after`).

## Kotlin's token loop runs from the middle: three passes (2026-11-27)

`bitlex_lex_rust::chunked` makes, from a tier 1 whose token loop keeps a nest (`nest`, `depth`, `template`, `held`,
`opened`), the loop's chunked form, appended to the tier. Kotlin's is made, and Python's (below); JSON's is not yet.

- **Pass 1, one chunk each, writing nothing (`chunk_ends`).** A chunk's summary is its lanes: for each place the
  chunk can be entered, and each class of nest below the entry that the lexing reads, where it leaves and what it
  counted. A lane is a typed bracket sequence: the levels it closes (`kc`, innermost first, of a kind or of any), the
  levels it needs below them and never reaches (`need`), whether it read the bottom (`exact`), and the levels it leaves
  open (`p`), with their strings' records, its own or kept from a level it closed (`INHERIT`).
  - **The places:**
    - the code entry, past the blanks before the chunk;
    - where a token of code the chunk starts in ends: lexed from the first byte of the run of bytes before the chunk,
      and from each byte of the chunk's first run (blanks part code tokens);
    - the end of each region the chunk can start inside: the region's row reads it as a token forced at the chunk's
      first byte (`Force`, the table `REGIONS`, made from the loop's open and row matches), one entry a level for the
      block comment's nesting.
  - **The classes:** a lane that reads the bottom gets one more level below it, of no kind (`OPEN`), until a read of
    `template` or `opened` asks the kind. Then it splits by kind. The splits go breadth first across all the places,
    so where a chunk passes `L` (64) lanes, the deepest go.
- **Pass 3, the prefix sum (`entries`): a single pass with decoupled look-back.**
  - A chunk applies its predecessor's summary to its predecessor's entry when that entry is known. Otherwise it
    composes the summaries back to the last known entry.
  - Where a composition passes `L` lanes (`over`), it waits for its predecessor's entry instead, so a lost lane never
    gives a wrong entry.
  - `compose` is associative as a function of the entry. An entry at or past a range's end goes through it as it came.
  - The Hillis–Steele scan over summaries was dropped: its long general ranges passed `L` and lost the right lane.
- **Pass 2 (`lex_chunked`)** lexes each chunk from its entry, writing the records, and counts the chunks whose entry
  was not the one the chunk before left.

### One summary for every loop's chunked form (the fold)

The three passes' summary for nesting is one definition, `bitlex_lex_rust::chunked`'s core: `Lane`, `Summary`,
`then`, `compose`, `apply` and pass 3's look-back (`entries`). Each token loop's form gives its `Entry` and makes its
chunks' summaries:

- `chunked_nest` (Kotlin's loop, which keeps a nest): `MAXD` 4, `L` 64, the lanes above.
- `chunked_lean` (rr's loop, which reads its blocks' bytes; moved into `chunked`): no nest (`MAXD` 0), lanes by place
  alone from its opener-byte walk, `L` 32.
  - An entry in a range's head with no lane at its place goes on in the code lane (`CODE_ANYWHERE`): rr's tokens from
    any byte of code come to the same place. That never holds on a summary that dropped lanes.
  - `fill_far`, the suffix scan that gave a region running through a chunk its far end, is gone. An entry at or past a
    range's end goes through it, and `compose(a, b)` keeps `b`'s lanes at or past `a`'s end.
- A lane that meets an entry is its own on a summary that went past `L`: lanes are disjoint classes. Pass 3 waits only
  where no lane meets the entry.

rr's corpus (1953 files): the serial lexer's records, 0 chunks entered again at 1, 2, 16 and 256 blocks, as before
the fold. Pass 3 waits for 2095, 965, 48 and 0 chunks (32 in flight). rr's speed report was not rerun after the fold.

### Numbers (Kotlin)

- **The test files** (1133 files at 1, 2, 3 and 16 blocks, 7780 chunks): the serial lexer's records, 0 chunks entered
  again.
- **The corpus** (495 files, 3.6 MB): the serial lexer's records at every size.

| Chunk size (blocks) | Chunks | Entered again | Chunks past `L` | Pass 3's waits (32 in flight) |
|---|---|---|---|---|
| 1 | 57,029 | 0 | 725 | 91 |
| 2 | 28,642 | 0 | 819 | 49 |
| 16 | 3,815 | 0 | 381 | 2 |
| 256 | 558 | 0 | 13 | 0 |

Tests (`rerail-kotlin-parser`, `chunked_tests`):

- `chunked_is_the_serial_lexer`;
- `nested_templates_are_entered_where_the_serial_lexer_is`;
- `summaries_compose_in_any_order` (the look-back against a left fold, and `(A·B)·C = A·(B·C)` on entries);
- `corpus_chunked_is_the_serial_lexer` (ignored: `RERAIL_KOTLIN_CORPUS`);
- `rerail-bitlex`'s `kotlins_loop_has_a_chunked_form`.

### Python's token loop from the middle: its nest is its string reader's (2026-11-27)

`chunked_reader` makes Python's chunked form, on the template above. Python's loop has no stage 1 and keeps no
`nest`: its nest is its string reader's state (`rd_on`, `rd_q`, `rd_fl`, `rd_level`, `rd_total`, `rd_around`,
`rd_depth`) and the code's bracket count `c_depth`. The generator passes the reader's facts (`ReaderNest`: its quotes,
its interpolation's flag, the flags a string may hold besides it, the opener's byte) from `reader_code` to
`with_chunks`.

- **The levels (`READER_NEST`).** A level is a bracket (255) or an interpolation, of its string's kind. Whether a
  bracket is the code's or an interpolation's code's is its place: below every interpolation, or above one.
  `Entry::levels` and `set_levels` map the reader's state to levels and back.
- **The reads, made where the loop makes them (`instrument_reader`):**
  - each step of `c_depth` a level pushed or popped;
  - `rd_pick`'s read of the top (`rd_top`);
  - a tail's reads of its interpolation's kind, bit by bit: its quote (`match qi`) and each flag it tests
    (`fl & 0x01`, `0x02`, `0x80`). The format and template flags are the same for every interpolation, so they are
    not reads;
  - `g_depth == 0`, the count at the bottom.

  Counts and bounds (`rd_level`, `rd_total`, `rd_depth`) are not reads: a lane's are relative, so it never refuses
  where the serial lexer does not, and pass 2 refuses.
- **A kind as far as it was read.** An interpolation's kind is partial: which of its bits (its quote, its free
  flags) a lane read, and their values. That gives 80 kinds and 252 (none known); the 16 whole kinds are
  `RD_KINDS`' indexes. A read of an unknown bit splits the level in two (`split_kinds`, by `Reads::bit`). Splitting
  each tail's level into its 16 kinds at once put 102 lanes at one place.
- **A kept value read by class (`Lane::cond`).** `rd_code`, where the outermost interpolation's code began, is a value
  a lane keeps from its entry (keep bit 2). A lane reads only whether it is none. Such a lane runs again as two (none,
  and one before the lane), and `cond` names the class of entries each holds for. Two lanes equal but for `cond` are
  one, for every entry.
- **A tail that opens the next interpolation** pushes the level it popped (`Reads::popped`), its kind as far as read.
  Pushing it from the lane's own copy (`rd_kind(rd_q, rd_fl)`) made a whole kind of guessed bits: its tail then never
  read the quote, walked to the other quote and refused.
- **Places in a region:** each string kind's tail forced at the chunk's first byte (`RD_PLACES`); a spec, a
  conversion and a comment. A forced token has no `}`, `:` or `!` of its own, so the checks of the token that opens
  the row (an interpolation's `=`, an empty interpolation, a comment, an empty spec) are skipped (`forced`). A chunk
  starting right after `value="` had refused there, at the `=`, and lost its place.
- `L` is 128: at 64, 4 of the test files' chunks were entered again.

The core (the template's and the summary's, for every form) changed with it:

- **A wildcard in `p`** (an entered level the lane left as it was, its kind not whole) is the entry's level.
  `apply` takes the entry's; `then` refines `a`'s class where `b` reads more of it, as the meet of `b`'s kind and
  `a`'s class at that level (`meet`), and fails where they are disjoint. Meeting `b`'s kind with the wildcard alone
  replaced what `a`'s class knew, and gave refused lanes to classes no lane held.
- **The kinds' operations** (`meet`, `wild`, `split_kinds`) are the nest's (`KIND_OPS` for one kind a level).
- **A refused lane ends the file:** `then` returns it as it is, and it keeps nothing past its class (its exit
  START's, no levels, no records), so the lanes that refuse alike are one.
- **An exit's kept values** take one form (START's), since they are not read.
- **The crossed token's places:** lexed from the first byte of the run before the chunk in each class of nest
  (`lanes_from`, stopped at the chunk's first block). The per-byte fallback, for a run that starts at a region's
  closer, stops at its first end. Every byte of the chunk's first run had been a place: an f-string's code
  (`{record_type.key}:{obj.pk}:{panel.key}`) has no blanks, so its chunk passed `L`.
- **A summary's lane runs are at most `4 L`;** past them it is `over`, as past `L` lanes. The bound did not fire in
  the tests.

### Numbers (Python)

- **The test files** (1614 files at 1, 2, 3 and 16 blocks, 11,407 chunks): the serial lexer's records, 0 chunks
  entered again, 0 waits, 66 chunks past `L`.
- **The corpus** (posthog's first 250 files in path order, 2.9 MB; `RERAIL_PYTHON_CORPUS_FILES`), before the last
  three core fixes (the refinement, the crossed token's places, the tail's popped kind): the serial lexer's records
  at 1, 2, 16 and 256 blocks, with 12,272, 5,307, 403 and 2 chunks entered again. After them, the miss report at 1
  and 2 blocks (`report_the_corpus_entries_pass_3_misses`): 28 chunks with no lane, each after a chunk past `L`, and
  no wrong entry. The four-size corpus run was not repeated after the fixes.

Tests (`rerail-python-parser`, `chunked_tests`): `chunked_is_the_serial_lexer`, `summaries_compose_in_any_order`,
`report_the_entries_pass_3_misses`, and, ignored (`RERAIL_PYTHON_CORPUS`), `corpus_chunked_is_the_serial_lexer` and
`report_the_corpus_entries_pass_3_misses`; `rerail-bitlex`'s `pythons_loop_has_a_chunked_form`.

## Not yet (superseded at the merge above)

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
