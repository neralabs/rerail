# JSON as a Bitlex program in simdjson's structure: the map

Status: 2026-11-18, a map for the user to check before any program or compiler change.

## Intro

**What is wanted.** A Bitlex program for JSON that states simdjson's structure. Compiled by our
compiler, it should give simdjson's code for stage 1 and stage 2, and simdjson's speed end to end.
Our tape is written where simdjson writes its own.

**What this page is.** Each piece of simdjson is set beside the Bitlex that states it, or marked
missing. The simdjson source is the vendored one (`native/vendor/simdjson-5.0.5/simdjson.cpp`),
cited by function.

**Why this page exists.** Decisions 0417 to 0428 tuned the compiler on `14-json.bitlex` and on
`stage1.rir`. Neither is a Bitlex program in simdjson's structure:
- `stage1.rir` is simdjson's stage 1 hand-written in the compiler's IR;
- `14-json.bitlex` splits the work between the tiers differently from simdjson.

Tuning how the compiler emits code can't turn `14-json` into simdjson's code, because the
tiers' split of the work stays the same. The split has to change, and it's the compiler's to
make (below).

## What is not done, and what differs today

Nothing on this page is built. Today's program, `14-json.bitlex`, differs from simdjson in
these ways:

| | simdjson | `14-json` today |
|---|---|---|
| Stage 1 writes | one `u32` position per structural | a start, a last byte and a code per token |
| A string, in stage 1 | one index, its opening quote | `"`, one token per content run, one per escape, `"` |
| A string's end | found by stage 2 (`parse_string`) | a token in stage 1 |
| A token's kind | stage 2 switches on the byte at the index | a code from stage 1 |
| A number's end | found by stage 2's parse | stage 1's `lasts(word)` |

Counted on `twitter.json`:

| | Count |
|---|---:|
| simdjson's structural indexes (`fixtures/simdjson-stage1/stage1_indexes.cpp`) | 55,263 |
| our tokens (`json_tier2_by_word`, the interpreter's `TOKEN` runs) | 93,565 |

On the 17 corpus files that fit tier 2's widths, simdjson finds 142,800 indexes.
Not measured: our token count on those 17 files.

## Stage 1, piece by piece

simdjson's stage 1 is `json_scanner::next` and `json_block` (`simdjson.cpp`, the `stage1`
namespace), with `bit_indexer::write` and `utf8_checker`.

| simdjson | What it is | Bitlex | Status |
|---|---|---|---|
| `json_character_block::classify` | the `op` and `whitespace` masks | `class op`, `class ws` | have |
| `json_escape_scanner` | the bytes a backslash escapes | `escaped(backslash)` | have |
| `json_string_scanner`: `in_string` | prefix XOR of the unescaped quotes | `toggle(q)` | have |
| `string_tail()` | `in_string ^ quote`: body and closing quote | `inside ^ q` | have |
| `scalar()` | `~(op \| whitespace)` | `~(op \| ws)` | have |
| `follows_potential_nonquote_scalar` | after a scalar that is not a quote | `prev(scalar & ~quote)` | have |
| `structural_start()` | `(op \| scalar & ~follows) & ~string_tail` | one mask | have, not used today |
| `non_quote_inside_string(in <= 0x1f)` | a control byte in a string | `error … when inside & control` | have |
| `utf8_checker` | UTF-8 by lookup | the UTF-8 rules (0404, 0405) | have |
| `bit_indexer::write` | one position per structural bit | one token per structural bit | **the program's tokens** |

Stated in Bitlex, stage 1 is:

```
# simdjson's json_block, word for word.
mask escaped    = escaped(backslash)
mask q          = quote & ~escaped
mask inside     = toggle(q)                  # opening quote in, closing quote out
mask tail       = inside ^ q                 # string_tail: the body and the closing quote
mask scalar     = ~(op | ws)
mask follows    = prev(scalar & ~quote)      # _follows_potential_nonquote_scalar
mask structural = (op | (scalar & ~follows)) & ~tail
error "a control byte in a string" when inside & control   # non_quote_inside_string
```

Every one of these is a mask Bitlex already has. What has to change is the tokens: every token
starts at a bit of `structural`, and stage 1 writes nothing else.

## Stage 2, piece by piece

simdjson's stage 2 is `json_iterator::walk_document` with `tape_builder` as its visitor.

| simdjson | What it is | Bitlex | Status |
|---|---|---|---|
| `walk_document`'s states (`object_begin`, `object_field`, `object_continue`, `array_begin`, `array_value`, `array_continue`, `scope_end`) | a state machine over the indexes, with a depth stack | `tree json { … }`, compiled to tier 2's token loop | have |
| `switch (*value)` | the kind from the byte at the index | `keywords` over the first byte | have; the compiler reads a code today |
| `visit_primitive`, a number | `parse_number` from the index; it finds the end | `token number = … run word` (the end by rule), `check number` | have, not used for JSON |
| `visit_primitive`, `true`/`false`/`null` | an atom check at the index | `keywords number { "true" … }` | have |
| `visit_primitive`, a string: `parse_string` | from the opening quote: 16 or 32 bytes at a time, find backslash and quote (`backslash_and_quote::copy_and_find`), handle an escape, stop at the quote | `read str parts { escape_sequence = esc bytes 2 } else string_content` | **the language has it, but its parts are stage 1 token rows** |
| `handle_unicode_codepoint` | `\u` hex digits and surrogate pairs | token checks on the escape (0399, 0428) | have, in tier 2 |
| `tape_builder` | writes the tape | tier 2's tape writer (page 37) | have |

## The gap: the compiler splits tokens (the user, 2026-11-18)

The user, on this map's first version: "It should be possible to split tokens so that only
their positions/types are recorded in phase 1 and they are actually processed in phase 2.
That's a basic hoisting that compilers can do all the time."

So the gap is the compiler's, not the language's. The program states the tokens, as
`14-json` does. The compiler decides what each tier does with them:
- **Tier 1 records** a token's position and type, and only for the tokens tier 2 can't find
  from the ones recorded.
- **Tier 2 processes** the rest from the bytes: a token's end, and the tokens inside a region it
  has the start of.

This is the same motion 0399 and 0428 made for checks, applied to the token rows themselves.

What the split gives for JSON's tokens:

| `14-json` token | Tier 1 records | Tier 2 finds, from the bytes | simdjson |
|---|---|---|---|
| punctuation | position; the type is its byte | its end: one byte | `bit_indexer::write`, `switch (*value)` |
| `number` (a word) | position | its end: the run of word bytes; `check`, `keywords` | `parse_number`, the atom checks |
| `"` opening | position | the string's end: the first unescaped quote | `parse_string` |
| `string_content`, `escape_sequence`, `"` closing | nothing | made by tier 2's read of the string from its opening quote | `parse_string`'s scan for backslash and quote |

What the compiler has to establish before it moves a token:
- **What tier 2 can recompute.** A row can move when its start and end are a forward
  computation over the bytes from a recorded token. The carries it reads restart there:
  - in a string, `escaped` restarts at the opening quote, and `inside` is 1 until the first
    unescaped quote;
  - a word's run needs no carry.
- **What stays in tier 1.** The carries that decide where the recorded tokens are: `inside`,
  `escaped` and `follows`. Here those are simdjson's `structural_start()`. The same goes for the
  errors and the UTF-8 check that tier 1 already does by block.
- **That the tiers agree.** Tier 2's recomputed tokens must equal tier 1's rows. The
  interpreter has both, so the test compares them.

Today, tier 1 records every row with a start, a last and a code. 0231's columns cover all the
rows, and 0177's `run`/`pair` rules end a token at its start, but in the CPU token loop of
tier 1. Not checked here: which of these the split can reuse.

## Step 1, done: tier 1 records simdjson's structurals

**The program:** `native/crates/json_parser/json-simdjson.bitlex`. It has `json.bitlex`'s tokens,
told by their end rules (`pair`, `run`, `read … parts`), and no `skip`, so tier 1 is the block of
masks, not a token loop.

**The compiler's split** (`bitlex_masks`):
- where tier 1 is the block of masks, a region read in parts is split by tier 2
  (`Split::Tier2`);
- its parts' rows leave tier 1's starts, which are then simdjson's `structural_start()`;
- `records::lasts` ends a region's token at its closer (`read_last`), so tier 2 reads the parts
  from the opener's record, as 0323's lowering B does.

**Tests:**
- `simdjson_forms_starts_are_simdjsons_indexes`: tier 1's starts are simdjson's indexes, count
  and FNV hash, on the files of `fixtures/simdjson-stage1/expected.txt` under 200 KB.
- `simdjson_forms_trees_are_14_jsons`: the interpreter's verdict and Cranelift's nodes are
  `14-json`'s on 58 documents (strings with escapes at a block's edges, invalid documents) and
  on the 17 corpus files.

**Measured** (retired instructions per pass, the 17 corpus files, 1.36 MB, 200 passes, the
load-only run subtracted, load about 5):

| | Tier 1 | Whole parse | Per byte | Against simdjson |
|---|---:|---:|---:|---:|
| simdjson | 3.73M | 9.88M | 7.3 | 1× |
| `14-json` | 20.4M | 47.5M | 35.0 | 4.8× |
| `json-simdjson` | 15.3M | **60.9M** | 44.9 | **6.2×** |

On `[1,1,…]`, the whole parse is 284M against `14-json`'s 207M and simdjson's 54.5M.

**Slower end to end.** The record pass wasn't removed; it moved:
- tier 1 still makes each structural token's last (by its rule) and its code (kind, word,
  marks);
- tier 2's string reader (0323's B) makes each part's code with the same pass (`plain_code`),
  reading tier 1's mask words for the closer and the escapes.

simdjson has no record pass: stage 2 switches on the byte at the index, and `parse_string`
scans the string's bytes.

Not measured: time.

## Step 2, done: no record pass; each `SWITCH` edge makes its token's record

**What changed in the code, beside simdjson's:**

| simdjson | Before | Now |
|---|---|---|
| Stage 1: `bit_indexer::write`, a position per structural | the starts, then a record pass: each token's last by its rule and its code (kind table, word hash) | the starts alone; the CPU's tier 1 has no record pass (`Records::fetched_in_tier2`) |
| Stage 2: `advance()`, one load of the next index | `TOKEN` loads start, last and code | `TOKEN` loads the start alone, where a `SWITCH` on its first byte takes it next |
| `switch (*value)` | a switch on the code's kind | a switch on the byte at the start, as simdjson's |
| The arm does its kind's work (`visit_primitive`, `parse_string`) | made for every token by the record pass | each switch edge makes its own bytes' record: a one-byte token's code by one table load; a word byte that begins no keyword, the kind as a constant and the run's scan; `t`, `f`, `n`, the run and the word hash; `"`, the closer's scan, then the string reader |

A token whose record is made already, a string's part or the stream's end, takes the switch on
its kind, as before. A `TOKEN` that no switch on the first byte follows makes the whole record
itself (`fetch_last`, by its first byte's rule).

**Tests:**
- `simdjson_forms_starts_are_simdjsons_indexes` now checks the CPU's tier 1: no record pass, no
  lasts, no codes, and its starts are simdjson's indexes.
- `simdjson_forms_trees_are_14_jsons`: Cranelift's trees equal `14-json`'s on the 75 documents.
- The ladder (`every_step_parses_as_the_reference`, `tier2_tests`), the parser tests' JSON and
  rr_parser's JSON pass.

**What still differs from simdjson's stage 2:**
- **A string's parts go through the grammar's state machine, a token each.** Each part is a
  `TOKEN` and a `SWITCH`, and its code is the word hash. simdjson's `parse_string` is one tight
  loop: it finds the next backslash or quote and handles the escape by its byte
  (`escape_map[escape_char]`), with no return to `walk_document` until the closing quote.
- **The state lives in a stack frame.** Each token stores and loads its start, limit, code and
  index there (`stack_store` and `stack_load` in `TOKEN`). simdjson keeps `next_structural` in a
  register.
- **The tape writer's words** (`OPEN`, `CLOSE`, about 50 IR instructions each) against
  `tape_builder`'s appends.

**Counts, as information** (retired instructions per pass, 200 passes, the load-only run
subtracted, load about 35):

| | Tier 1 | Whole parse | Against simdjson |
|---|---:|---:|---:|
| 17 corpus files: simdjson | 3.73M | 9.88M | 1× |
| 17 corpus files: `14-json` | 20.4M | 47.5M | 4.8× |
| 17 corpus files: step 1 | 15.3M | 60.9M | 6.2× |
| 17 corpus files: now | **5.96M** | 56.3M | 5.7× |
| `[1,1,…]`: simdjson | 8.5M | 54.5M | 1× |
| `[1,1,…]`: now | **9.2M** | 219.5M | 4.0× |
| short strings: simdjson | 3.1M | 9.19M | 1× |
| short strings: now | 4.9M | 74.7M | 8.1× |

Not measured: time.

## Step 3, done: atoms as simdjson tests them; the token loop's state as SSA values

**Atoms.** A switch edge whose bytes begin words, its run's output a byte class, tests each
word as simdjson's `visit_primitive` does `true`:

| simdjson (`parse_true_atom`) | Ours, before | Ours, now |
|---|---|---|
| `str4ncmp(src, "true")`: one 4-byte load, one compare | 64 bytes classified (NEON nibble lookups, 4 `addp`), the run's end by `ctz`, then the word table's hash | one 8-byte load, masked to the word, one compare |
| `is_not_structural_or_whitespace(src[4])`: one table load | — | one table load of the byte after (`run_class`) |
| on to the value | the switch again on the word (`by_word`) | the word's target at once |

Another token of those bytes takes the rule's run and the word table, as before.

**The state.** One-function tier 2 (`Cut::one`) holds the token loop's fields as SSA values
(`HELD_IN_ONE`: the next index, the token's start, limit and code, the nodes made and open,
a split region's cursor: seven, with `ctx` within the ten registers a call keeps, as the
loop's cold calls clobber the rest), and each word loads the `ctx`
fields it reads where it runs (`Cut::reload`). The frame's loads and stores in the IR fell
from 587 and 836 to 161 and 248.

**What the machine code shows still differs** (`json_tier2_by_word` with `RERAIL_DUMP_TIER2`
now writes the code and each word's ranges):
- The function is about 19K instructions: each `SWITCH` is 250 to 800, as each carries its
  own copy of every edge's record code. simdjson's `walk_document` states share
  `visit_primitive` and `parse_string`.
- The allocator still spills: `ctx` and the next index live in the frame across most blocks
  (`ldr x?, [sp, #0x108]` and `#0x118` in `TOKEN`). simdjson keeps `next_structural` and the
  tape pointer in registers.
- A number's end is still found by classifying 64 bytes; simdjson's `parse_number` finds it
  by parsing, and the number check is a second pass over the same bytes.
- A string's parts, as in step 2.

**A string's content takes no word table.** A part's code was its bytes' word looked up in the
word table (a hash, about 30 instructions), though only the escapes' words are read after.
The compiler now finds which kinds' words the words after a `TOKEN` read, up to the next
`TOKEN` (`words_read`: a switch's named words, by kind; `WORD`, a mark or a keyword's leaf
read every token's). A part of another kind takes its first byte's kind alone: two loads and
a compare (`ldrb`, `ldrb`, `cmp #2`, `b.ne`). An escape still takes the table.

**A number's end from one window.** A run's end (`EndRule::Run`) whose output tier 2 can
make from the bytes is read first from the 16 bytes after the token's first, one vector
(`short_word`: a load, the nibble lookups, a compare, a narrowing to a mask, `ctz`): about 30
instructions, as a lexer reads a short token's bytes and no more. Only a longer run takes the
64-byte scan. simdjson's `parse_number` finds the end by parsing the digits; the number check
here is still a pass of its own over the same bytes.

Tests: the trees equal `14-json`'s on the 75 documents; the ladder, `tier2_tests`, the parser
tests' JSON and block tests, rr_parser's JSON and CPU tests pass.

Counts, as information (retired instructions a pass, load-only run subtracted, load about
15): the 17 files' whole parse 53.1M (step 2: 56.3M), `[1,1,…]` 206.8M (219.5M), short
strings 70.5M (74.7M).

## Step 4, done: a string read as `parse_string` reads it

| simdjson | Before | Now |
|---|---|---|
| Stage 1 stores no mask | tier 1 stored a row of the escape part's start (an escape carry's output) for tier 2, and rows of the quote and the run's masks | none of the three (`read_at_tokens` where tier 2 makes the records): 63 stores in tier 1's code to 56 |
| `"` opens the string; no closer scan first | the `"` edge scanned for the closer (the quote's mask made from the bytes, with the escape carry) | the record is the opener alone |
| `copy_and_find`: 16 bytes, `cmeq` against `"` and `\`, the masks, the first bit | each part: the part's row word, the nearest bit, before the closer found already | 16 bytes, `cmeq` against `"` and `\`, two masks, `orr`, `cbnz`, the first bit (`split_next_seq`) |
| an escape by its byte: `escape_map[escape_char]` | the word table's hash | one load of the part's code by its second byte (`Seq::codes2`) |
| content copied, no lookup | the word table's hash | its first byte's kind (step 3's `words_read`) |

**Why it is sound** (`native_cpu_stream::sequential`). The case:
- the closer is `X &~ escaped(E)`, its one part `(E & R) &~ escaped(E)` of two bytes, with `X`
  and `E` disjoint byte classes;
- `R` is the region's inside, `toggle(T) &~ (T &~ prev(toggle(T), 1))`, where `T` is the
  closer and a mask that holds at every byte of the stream;
- the opener is the closer's output, and a region open at the file's end is refused.

From a clean place (after the opener, or after a part), no `E` lies before the first byte of `X`
or `E`, so that byte is not escaped. There, an `X` is the closer and an `E` is a part. A part's
two bytes end a run of `E` of even length, so the place after it is clean again.

The test of tier 1's starts asserts tier 1 stores no row of the run's mask, the quote's or the
escape part's. The escape part has no mask tier 2 can make from the bytes, so this also locks
the sequential reading. A part of one byte in place of two fails the trees test.

Counts, as information (load about 15): the 17 files' whole parse 44.5M (step 3: 53.1M),
`[1,1,…]` 190.7M (206.8M), short strings 51.1M (70.5M). Against simdjson's whole parse: 4.5×,
3.5×, 5.6×.

## Step 5, done: no room check a token; the start read as `*(next_structural++)`

| simdjson | Before | Now |
|---|---|---|
| The tape's capacity from the input, once (`allocate`); no check a token | three room checks a token (nodes, supertypes, returns): three loads from `ctx`, three compares, three branches | none: where the bound holds for every run (no returns chain) and a token's path writes at most 4 nodes (`Bounds::unchecked`), the run's tape is the bound's from the start. The capacity is reserved, not written |
| `buf + *(next_structural++)`: a 4-byte index array | the record's stride loaded from `ctx`, a multiply, a load | `ldr w, [starts, next, lsl #2]`: tier 1 writes the starts alone, a column (`call` asserts the stride) |

JSON's bound is 2 nodes, 1 supertype and 1 return a token. A `TOKEN` at a value is now 13
instructions before its switch, from 24. Of those, two checks remain that simdjson does not make
a token: the stream's end (`AtEnd`, and the count's compare). simdjson has a sentinel in place of
both: the index after the last points at the padding, which no switch accepts. The starts test
asserts there is no room check.

**The end in one compare.** The end's token now sets `Next` one past the count, so "at the
end" is `Next > nstarts` and the `AtEnd` flag is gone from the state. A token tests
`Next >= nstarts` once (`b.hs` to a cold block that tells the end's token from past it), in
place of a flag loaded from the frame, its branch, and the count's compare. `SWITCH`, `MARK` and
`ACCEPT` read the end the same way. A value's `TOKEN` before its switch: 13 instructions to 11.

## Order

1. Done: step 1, above.
2. The split, as a decision record with its requirements, and a test for each before the code:
   - **Tier 1 records only the moved tokens' anchors.** For `14-json` that's simdjson's
     structural positions, the same as `stage1_indexes.cpp` file by file.
   - **Tier 2's tokens equal the interpreter's rows** on the corpus and the escape texts.
   - **A grammar whose rows can't move keeps them in tier 1**, as `.rr`'s ladder shows.
3. Then the records: a moved token's record is its position alone, and tier 2 reads the type
   from the byte, as `walk_document` does.
4. At each step, the report shows three things side by side:
   - the program;
   - our generated code for the piece;
   - simdjson's code for the piece.

   Then whole-parse instructions per byte and time against simdjson, on `numbers.json` (dense
   in tokens) and `twitter.json`, with what was not measured.
