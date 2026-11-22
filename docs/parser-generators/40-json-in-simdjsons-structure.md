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

## Step 6, done: `ctx` in a register of its own; a leaf's start not read back

| simdjson | Before | Now |
|---|---|---|
| `this` (the parser and its tape) in a register through `walk_document` | `ctx` one value through the function, which the allocator split around the cold calls: 486 reloads of it from the stack | Cranelift's pinned register (x21), as the clusters' already: set where the function is entered, the host's given back where it leaves, read at each use (`pin_one`). Stack loads in the function: 665 to 276 |
| `tape.append(value, type)` | a leaf's tail read the start back from the row it had just written | the start passed on (`close_cells`'s `start`) |

A leaf's other stores are the engine's tape: a row (kind, start, end) and the offset, size and
tail columns, against simdjson's one 8-byte word a scalar. That is the output's format, not a
step of the parse.

Counts, as information (load about 60): the 17 files' whole parse 40.4M (step 4: 44.5M),
`[1,1,…]` 174.5M (190.7M), short strings 45.4M (51.1M). Against simdjson's: 4.1×, 3.2×, 4.9×.

## `walk_document` next to tier 2's words

From the dump (`json_tier2_by_word` with `RERAIL_DUMP_TIER2`: each word, each switch's targets
by byte). Word ids: 12 `{`, 15 `}`, 16 `[`, 17 `]`, 13 `:`, 14 `,`, 9 to 11 the atoms.

| simdjson | Tier 2 | Same shape? |
|---|---|---|
| root `switch (*value)`: `{`, `[`, else `visit_root_primitive` | 1–2: `"`, digit or `-`, atoms, `{`, `[` | yes |
| `object_begin`: `*key != '"'` | 32–33: `"` to the key, `}` (the empty object) | yes; the empty case in the switch, not a `peek` |
| `object_field`: `*advance() != ':'`, a value switch | 87–88: `:`; 114–115: the value switch | yes, since step 7 |
| `object_continue`: `,` or `}` | 150–151 | yes, since step 7 |
| `array_begin`, `array_value`: a value switch | 46–47 (a value or `]`), 97–98 (after `,`) | yes |
| `array_continue`: `,` or `]` | 94–95 | yes, since step 7 |
| `scope_end`: `is_array[depth]` picks the continue state | `PUSH` of the place to go back to (43, 94 or 150), `RETURN` there | yes: a stack of where to continue |
| `parse_string`: one call reads the string | each part (content, an escape, the closing `"`) goes from the scan's outcome to its place, then a direct `GOTO` back | yes, since step 11 |
| `parse_string`'s loop only where a string is read | the part scan only in the four string loops (7, 38, 51, 126) | yes, since step 8 |
| `parse_number`: validates while reading, then the byte after | the edge reads 16 bytes once: the end and the check from them | yes, since step 12 |
| `parse_true_atom` and the others: the atom's bytes alone | the atom's word test; the number's check decided by the word as tier 2 compiles | yes, since step 12 |

## Step 7, done: a continue state takes its punctuation by byte, straight to its target

Before, at `array_continue` the bytes `t`, `f`, `n` and the digits went by word with `,` and
`]` (one record kind). Their edge made the record, tested the atoms, found the switch names no
atom, and refused. And `,` and `]` were taken by byte, their code loaded by the byte, then
switched on again by word. Now:
- a byte only a named word can take (its kind's own target a refusal) is refused at the byte
  where no named word begins with it, and the atom tests run only for words the switch names;
- a one-byte token whose byte is its word (`{`, `,`) gets its word's code as a constant and goes
  straight to the word's target.

So `array_continue` is the byte, `]` or `,`, and the target, as simdjson's
`switch (*advance())`. The trees test gained documents that fail at the continue states
(`[1 true]`, `{"a":1,}`, `{"a":[1}`, …). Taking the kind's target in place of the word's fails it.

## Step 8, done: the part scan only where a string is read

Every `TOKEN` held the scan for a string's next part (16 bytes, `cmeq` against `"` and `\\`),
behind a test of `PartAt`, at `array_continue` and `object_continue` too. `parts_pending`
finds where a part may be pending: over (place, a part pending), from the first word with none.
- An opener's byte at a switch makes one pending.
- Under one, the closer's kind ends it, and every other kind keeps it.

Only those `TOKEN`s hold the scan. For JSON these are the four string loops: a root string, a key,
an array's string and a pair's value. The starts test asserts it: the scans are at exactly the
`TOKEN`s whose switch takes the escapes. With no part ever pending, the trees test fails; with
one always pending, the starts test fails.

## Step 9, done: a string's content taken as `parse_string` takes it

The scan's three outcomes, against `parse_string`'s:

| `parse_string` | Before | Now |
|---|---|---|
| quote first: `return` | the closer's kind: its byte loaded, its kind loaded | a constant (every byte of `X` has the one kind) |
| backslash: `escape_map[src[bs_dist + 1]]` | one load by the second byte (step 4) | the same |
| the bytes before it: copied | the first byte's kind, then a test of it for the escape's kind, and under it the word table's hash | the first byte's kind alone: a content token's first byte is neither `X` nor `E`, and no other byte has a kind a word after reads |

`split_token` takes the bytes a token's first byte can be (`firsts`: `X` for the closer and
the opener, neither `X` nor `E` for content). It reads only the kinds one of them has, and a
kind every one of them shares is a constant. Content given the closer's bytes fails the trees
test.

## Step 10, done: the loop's state in registers

simdjson's stage 2 keeps `src` and `next_structural` in registers through the loop. Ours kept
`PartAt` and the token's end in the frame: loaded and stored at every string part. The cause
was in the compiler:
- the loop's cold calls (a wide tape entry, a refusal) took the platform's convention, which
  keeps 10 registers (x19 to x28) across a call and clobbers the rest;
- a few values made at the function's entry (the constant zero every word uses, `ctx`'s
  fields) were live through all of it, in registers the state then lacked;
- every read of a `ctx` field moved the pinned register to another one first.

The fixes:
- **A calling convention for the cold helpers.** The vendored Cranelift's `preserve_most`
  (`PATCHES.md`), as LLVM's: the callee saves every general register it writes but the
  returns' and the scratch ones.
- **Values made where they are used.** Constants and `ctx` field reads used in another block
  are made again in that block, from the pinned register (`remat_far`).
- **The pinned register as an address's base.** A load from `get_pinned_reg` reads x21 in
  place (aarch64; r15 on x86-64), as `sp`.
- **No stores nobody reads.** A store to a stack slot no load of the slot reads is dropped
  (`drop_dead_slot_stores`).
- **The token's end held too.** `LastEnd` joins the state held as SSA values (`HELD_IN_ONE`).

The string loop now matches `parse_string`'s: its cursor, the token's start and the next
structural's index stay in registers. The starts test asserts it on aarch64: no word of the
string loops or the continue states reads or writes the frame. With the helpers on the
platform's convention, it fails at the root string loop.

The frame is still read at a scope's close (the stack of open scopes, as simdjson's
`is_array[depth]`) and in the numbers' and atoms' words.

## Step 11, done: a string's part branches once

Before, the scan knew a part's outcome: the closer, an escape or content. But the part went
back through the `TOKEN`'s common path, and the `SWITCH` after dispatched on its kind again.
The content's arm was also shared by the four string loops (`shared_runs`). So each part
stored `Sel` and `Shift` and went back into its loop through a dispatch on `Sel`.
`parse_string`'s loop branches once a part and turns with a direct branch. Now:
- **Each outcome goes to its place** (`Onward`). The closer and the content go to the
  switch's target for their kinds. An escape, whose words the switch names, goes to the
  switch on its word: `escape_map`'s load, then one range test.
- **The loop's turn is a direct jump.** A run that goes back into a string loop (a place where
  a part may be pending) is a body of its own, not shared. A body with one target writes no
  `Sel`, and its own first place writes no `Shift`.
- **Two compiler passes.** One drops dead block parameters: a part's kind, which no word
  after reads, kept its load alive through one. The other drops unused pure instructions.
- **`Pending` is the constant 0** in a program with no extras, and `BlockBase` is held with
  the loop's state. So a leaf reads and writes no frame on its hot path.

The starts test asserts that every string loop's parts go straight on (`part_onward`).
Sending the closer to the content's place fails the trees test.

## Step 12, done: a number read once

`parse_number` reads the digits and validates them as it goes. Where they stop, it tests the
byte after (`is_not_structural_or_whitespace`). Ours read a number's bytes twice:
- the switch's edge found the end: the run of `wordbyte` scanned 16 bytes from `s + 1`;
- the check (`check number = …`, a token check, decision 0398) loaded the 16 bytes from `s`
  again and classified them again.

The atoms are keywords of the number's row, so `true` ran the number's check too. Now:
- **The edge reads the bytes once** (`fused_check`), where the target's words are the
  token's `SPAN`s and then its check. The target's check is over the token's bytes alone:
  none back, no gate.
  - 16 bytes are loaded at `s`. The check's byte sets and the run's end set (the bytes not in
    the run's class) are classified together: one nibble split, the tables once, one tree of
    adds.
  - The end is the first byte past `s` in the end set. The check runs over the bytes before it.
  - Passed, the edge goes on past the check's word. Inside a shared run it goes to the body's
    word, with the place's `Sel` and `Shift`.
  - A token whose end is past the 16 bytes takes the path as before.
- **An atom's check is decided as tier 2 compiles** (`check_at`, with `token_value`). A check
  of no delay reads only the token's bytes, and the edge knows the word. So `true` goes past
  the number's check, as simdjson tests an atom by its bytes and nothing else.
- **`token_classes` splits the bytes into nibbles once** for all its tables.

The starts test asserts it: every check over a token's bytes is made where its token is found,
in the edge (`fused_checks`) or as it compiles (`static_checks`). The trees test gained
numbers that fail the check, at the file's end and past 16 bytes. Two mutations fail it: the
fused check never refusing, and the atoms' decision inverted.

## Correction: steps 4 to 12 were judged on the wrong code (the user, 2026-11-19)

The user asked for the comparison to be made on the generated Rust: the ahead-of-time Rust that
`rust_stream` writes for a Bitlex program. Steps 4 to 12 were instead chosen and judged on
Cranelift's IR and its machine code. They were also reported as if they had been judged on the
Rust. `rust_stream` does not generate this program at all. It refuses it at "a region tier 2
splits (decision 0323): not yet ahead of time". The steps above went into the Cranelift backend
(`native_cpu_stream`) alone.

What follows, in order:
1. **`rust_stream` generates this program.** It gains what the form uses:
   - records made in tier 2: the switch by first byte, each edge making its token's record;
   - atoms tested as simdjson tests them;
   - supertype classes in the tape's codes;
   - the split string read as `parse_string`;
   - the number's check, made in the pass that finds its end.
2. **A JSON harness, like `rr_aot`.** It builds the generated Rust and runs the trees test
   against the interpreter.
3. **The comparison:** the generated Rust's structure, next to `walk_document`, `parse_string`
   and `parse_number`. The changes are chosen from that.

Later, not now: the CLIF-to-Rust translator (`rerail_clif::rust`, `clif_rust`) was never meant
to exist, and is to be deleted.

## Step 13, done: `rust_stream` generates the simdjson form

`rust_stream` now writes this program's tier 2 as Rust. What it gained:
- **Tier 2 makes the records.** `TOKEN` takes the next start, tier 1's structural index. The
  switch on the token's first byte makes the token's record in each arm (`fetched_arms`):
  - a token of one byte gets its code and its place at once;
  - the opening quote is a token of its own, with the string's parts pending after it;
  - a number's arm tests each atom as simdjson does, `str4ncmp`, then checks that the byte
    after is not in the run's class. Otherwise it finds the run's end, 16 bytes a vector
    (`run_R`).
  - A token switched on again (after a call returns, `again`) takes the switch on its kind, as
    before.
- **A string's parts are read as `parse_string`** (`part_plan`). It finds the next quote or
  backslash, 16 bytes a vector (`part_stop`, `copy_and_find`'s compares), then branches once:
  the content before it, the closer, or an escape. An escape's code comes from its second
  byte (`CODES2`, `escape_map`). The `TOKEN` and its switch are one piece of code.
- **Token checks** (`token_check`): over the token's bytes, 64 bytes a step, word-parallel.
  Byte sets are tier 1's nibble classes, a `prev` carries its last word, a `fill` its carry.
- **Supertype classes** in the tape's codes, as `native_cpu_tape::commit` and `finish` write
  them.

`simdjson_forms_rust_trees_are_the_jits` builds the source with rustc into a library, loads it
in place of the JIT's tier 2, and gives the JIT's trees on every file of the trees test: 108
with the corpus (81 parsed, 27 refused). A mutation fails it: the content's next part one byte
late.

## Step 14, done: a number read once, in the generated Rust

The number's arm read its bytes twice: `run_0` found the end, then `check_0` loaded them again,
64 bytes a window. Now the arm calls `run_0_check_0` (`run_check`):
- one load of 16 bytes at the number's start, the bytes past the file zeroed;
- the check's byte sets and the run's end set (the bytes not of its class) from one set of
  classes;
- the end: the first lane past the start in the end set, or past the file;
- the check's ops over those 16 lanes, then on past the check's word.

A token whose end is past the 16 bytes takes the path as before.

An atom's check is decided as tier 2 compiles (`token_value` over the word's bytes), so `true`
goes straight to its leaf.

The trees test with the Rust asserts each number's arm makes its end and check from one read.
Two mutations fail it: the fused check never refusing, and the atoms' decision inverted.

## Step 15, done: one dispatch a value, in the generated Rust

The array's first value has a switch of its own, since it also takes `]`, the empty array. Both
it and the switch after each `,` went to every value's place, so the loop had an entry for
each. Rust has no goto: the structure (`clif_rust::Shape`) put the place in a variable, `d`,
and the loop's head took the arms apart again with `match d`. Each value was dispatched twice.

- **A switch whose arms are another's and some of its own** (`delegate_of`, by each byte's arm
  as `byte_sig` gives it) keeps its own arms, and sends every other byte to the other switch,
  as `walk_document` peeks for `]` and then goes to `array_begin`. The array's loop now has one
  entry, its value switch.
- **A switch reached only from the `TOKEN` before it**, which takes a token (`fresh`), has no
  path on the kind: its token is not made yet, or the stream has ended. The path on the kind is
  left at the three switches a call returns to (`again`).

The trees test with the Rust asserts both: no `match d`, and the path on the kind only at the
switches a call returns to. Sending the empty array's `]` on to the value switch fails it.

## Step 16, done: the `\u` checks only after a `u`, in the generated Rust

Every escape ran both `\u` checks: their masks are tier 1's (`unicode`, and `high`, which holds
at a high surrogate's `u`). The escape's word is known in its arm, and so are its bytes. So when
tier 2 compiles, an escape word goes past each check whose mask can't hold at its bytes
(`past_void_checks`):
- `may_bytes` gives the bytes a block's value can hold at, a superset: a value of its own byte
  at its byte, an `and` where both sides can, a `prev` by `k` at the byte `k` back;
- a mask of delay `d` at a place is its value `d` places on.

`\u` goes to its two checks. The other eight escapes are one range (`1..=8`), straight to the
leaf, as simdjson does the `\u` work only after a `u`. The closer's arm no longer tests its
span's length, since it is one byte by construction (`past_span`).

The trees test gained bad `\u` escapes, and a file of every other escape beside a surrogate
pair. The Rust trees test asserts that the checks are called in the `\u` arm alone. Voiding
every check fails it, at `"\u12g4"`.

## Step 17, done: no room check a token, in the generated Rust

The harness sizes this program's tape from the input once (`Bounds::unchecked`), for the JIT and
for the Rust alike, as simdjson sizes its tape. The Rust still tested the tape's room at every
token and every string part. Where the bound holds, `rust_stream` now writes no room check. The
Rust trees test asserts that the source has none.

## Step 59: the generated Rust beside the lean parser, piece by piece

The generated parser is `rerail-json-aot`: `tier1.rs` (`bitlex_lex_rust`) and `tier2.rs` (`bitlex_rust::tier2_loop`).
The reference is `rerail-json-lean`: simdjson's arm64 stage 1 (`stage1.rs`) and the kernel's stage 2 (`stage2.rs`),
translated line for line. Both are Rust and both go through rustc, so each difference below is a difference in the
source the compiler writes. The order is the order of the fixes: what one unblocks comes first.

### Tier 2 against `stage2.rs`'s `walk`, before the fixes

| | `walk` | `tier2.rs` |
|---|---|---|
| A token known by the test before it | tested once | tested again: `0x22 => match peek { 0x22 => {} … }`, `0x2c => match peek { 0x2c => … }`; `0x7b` then `peek(i) == 0x7b && peek(i + 1) == 0x7d`; an atom's word decided, then its kind's bytes and its word again |
| A value | `primitive` and `string`, one method each, called at each place | the value's code written out at each of its five places |
| The walk | each container's cycle a loop of its own (`object_key` to `object_continue`, `array_value` to `array_continue`); only an open and a close pass the `match label` | one `'run: loop { match st }`: every place after a call an arm, every transition `st = N; continue` |
| Where a close returns | `object[d]`, a flag by depth: `scope_end` goes to the object's or the array's continue | a `Vec<u8>` of return places, `push` and `pop` |
| A pair's close | in `object_continue`, after the value | a place of its own (`5`, `10`), and `9` only goes on to `8` |
| A token | `take!()`: `*next`, the index ending in three entries (`len`, `len`, 0) | `peek`, `at`, `starts[i]`: each bounds-checked, 256 or the file's length past the end |
| An atom | a `switch` on its first byte, then `str4ncmp` and the byte after | the run of `wordbyte` from tier 1's mask (`run_last`), then `match` on the bytes as a slice |
| A number | `number_end`: one read of its bytes | `run_last` over the mask, then `check_number`, which tests each byte twice (`if matches!(x) { if !matches!(x) { return false } }`) |
| A string | `backslash_quote`: the bytes 32 at a time from the quote, the common case (no `\` before the closer) two nodes written at once | the closer from tier 1's mask (`first_bit(&m.q)`), each escape from another (`first_bit(&m.esc)`) |
| A node | `open`, `leaf`, `close`: the columns' stores | the writer's ops, then `commit` after every node |

### Tier 1 against `stage1.rs`'s `index`, before the fixes

| | `index` | `tier1.rs` |
|---|---|---|
| A string | `prefix_xor` of the unescaped quotes, carried to the next block: in the pass | a scalar walk after the pass (`first_bit` from each opener to its closer), writing `o_str`, `f_str` and `hidden` as the file's words |
| The masks made of the strings | in the pass: `structural_start`, `non_quote_inside_string` | about fifteen more passes over the file's words (`body`, `esc`, `code`, `word`, `t4` to `t11`) |
| The structurals | written in the pass, a block behind (`bit_indexer::write`: the bits reversed, four indexes, then two at a time, then one) | a loop over the file's words after the passes, one `push` a bit |
| What it writes | the indexes | the starts, and a code a token (`KINDS[src[s]]`, the same in each of three branches), which tier 2 does not read |
| What it keeps for stage 2 | nothing | four masks of the file (`q`, `esc`, `rd0_open`, `run_number`) |
| A block | loaded where it is; the last one copied once, padded | copied into a `[u8; 64]`; each of three stages works out its in-file bits and the byte order mark again |
| UTF-8 | `utf8_checker`: simdjson's lookup tables on the bytes' nibbles, a block of ASCII skipped | 26 classes as bitmasks, then the grammar's expression over them with `prev`, and a second stage two blocks behind for `next` |
| `\u`'s digits and surrogates | checked in stage 2 (`escaped_string`) | checked in tier 1, over the file's words |

### The fixes, in order

1. Tier 2 tests no token twice. Done (decision 0581).
2. Tier 2 writes each rule used at several places once, as a method, as `primitive` and `string`. Done (decision 0582).
3. Tier 2's walk: a loop a container's cycle; a close returns by the container's kind at its depth. Done (decision 0583).
4. Tier 2 reads its tokens as `take!()`, and writes its nodes without a commit each. Its writes done (decision 0584): the writer's nodes final as written. Its reads done (decision 0585): the source padded, the index ending in three entries.
5. Tier 1 makes the strings and the masks made of them in its pass, and writes the structurals there. Done (decision 0585).
6. Tier 1 writes no codes tier 2 does not read (done in 0585), and keeps no mask that tier 2 can read from the bytes, as `walk`
   reads a string and a number. Done (decision 0586).
7. Tier 1's UTF-8 check in simdjson's form. Done (decision 0591).

Steps 5 and 6 are also 0556's second step (`bitlex_lex_rust` is Kotlin's and `.rerail`'s tier 1 too); steps 1 to 4
are `tier2_loop`'s, which only JSON's grammar reaches.

### After the seven fixes (2026-11-21)

The same reading, after 0591. These are the differences left, by source shape.

Tier 2 against `walk`:

| | `walk` | `tier2.rs` |
|---|---|---|
| A container's cycle | an inner `loop`: pair (or element), then `,` back to the top or `}` out; only a container's begin and end go through `match label` | each place an arm of the one `match st`: between two pairs of an object, the pair (3), its close (5) and its `,` or `}` (4) are three trips through the match; an array's element and its `,` two |
| A container's begin | `ObjectBegin` takes `{` and tests the key's quote in the same arm, then enters the loop | its own place (2, 6) that takes the opener, then the next place (3, 7) |
| A pair's close | inline after the value, or in `ScopeEnd` for a container value | a place of its own (5), which every pair passes through |
| A value's byte | one `match *v`: `{`, `[`, else `primitive`, which switches on the byte once | the place's `match` on a range of bytes that holds `"`, then `value`'s `match` on `"` against the rest, then a `match` on the first byte for `true`, `false`, `null`, a number |
| A string | `backslash_quote` 32 bytes at a time; with no escape before the closer, the string and its content written at once | `marks` 64 bytes at a time; each piece a leaf |
| A node's write | the columns' stores, through pointers in `Walk`'s locals | `w_open_with`, `w_leaf_with`, `w_close_with` on the writer's state (`d`), final as written, no commit |
| A close's return | `object[d]`, then the container's kind's continue | `ret[d]`, the place number (the same shape) |
| Reads of a token, an atom, a number, the depth, the root's closer | | the same as `walk` (0581–0586) |

Tier 1 against `index`:

| | `index` | `tier1.rs` |
|---|---|---|
| `\u`'s digits and surrogates | checked in stage 2, when it reads an escape | checked in tier 1 at every block: five classes (`hex`, `u`, `dee`, `high_2`, `low_2`) and a second stage a block behind for their `next`s |
| A mask's word | a local of the block | written into a ring (`r_X[q % RING]`) and read back from it, a mask with no `next` reader too |
| The file's length and the byte order mark | the length's bits once, the byte order mark skipped before the loop | `lv` and the byte order mark's test in each of the two stages, at every block |
| A string open at the file's end | the carry tested once after the loop | `f_str & last_byte` at every block, `last_byte` a mask of its own |
| The classes | one pair of nibble tables (`whitespace`, `op`), compares for `"` and `\` | two pairs: ws, control, op and the `\u` classes; compares for `"`, `\`, `u` |
| UTF-8 | `utf8_checker` | `utf8_checker`, derived from the grammar (0591); a block's end tested by three byte compares, not a vector compare |
| The strings, the escapes, the records, the padding | | the same as `index` (0585, 0586) |

### What was left, in order: done (decision 0599)

8. Tier 2's cycles as loops: a list's head and cycle one inner loop inside the place of its level, the opener taken in it,
   and a pair's close written where its value ends. Only a level's call and its end go through `match st`. Done.
9. A value decided once: `value`'s alternatives as the arms of the place's own `match`, so the byte is matched once.
   Done: one `match src[s]` at each value place, `{`, `[`, `"`, the atoms and the number its arms; no `value` method.
10. `\u`'s digits and surrogates checked where tier 2 reads an escape, with tier 1's five classes and its second stage
    gone. This is a grammar question too: the checks are the grammar's `error` statements over masks. Done
    (`bitlex_piece_checks`).
11. Tier 1's masks as locals of the block, except those a `next` reads. The length's bits and the byte order mark worked
    out once; the end's errors tested once after the loop. Done.
12. A string with no escape written as its node and its content at once. Done (the writer's `leaf_in`).

### After the twelve fixes (2026-11-21)

The same reading, after 0599. These are the differences left, by source shape.

Tier 2 against `walk`:

| | `walk` | `tier2.rs` |
|---|---|---|
| A container's cycle | an inner `loop`; `ObjectBegin \| ObjectKey` one arm | the same: `'l2: loop` and `'l5: loop`, `1 \| 2` and `4 \| 5` |
| A pair's close | after the value | the same |
| A value's byte | `match *v` for `{` and `[`, then `primitive` tests it again | one `match src[s]`, every alternative its arm |
| A string with no escape | its node and its content at once, from 32 bytes | the same, from 64 bytes (`marks`) |
| `\u`'s digits and surrogates | in `escaped_string`: the four digits read as a number, the surrogates tested on it | at a `\u` in `string`: each digit's class, `D`, and the third byte's class; a low one's high one from a word of the pieces before |
| A node's write | the columns' stores, through pointers in `Walk`'s locals | the writer's ops on its state (`d`): `open`, `leaf`, `leaf_in`, `close` |

Tier 1 against `index`:

| | `index` | `tier1.rs` |
|---|---|---|
| A mask's word, the file's bits, the byte order mark, the end | locals; once; once; after the loop | the same |
| The classes | one pair of nibble tables, compares for `"` and `\` | the same |
| The masks made of the strings | `scalar`, `follows`, `in_string`: those the structurals need | the grammar's names: `o_str`, `hidden`, `body`, `code`, `wordbyte`, `word`, each a local |
| A control byte in a string | ORed into the error word, tested after the loop | the same (`errors \|= v_t2`) |
| UTF-8 | `utf8_checker` | `utf8_checker`, derived from the grammar (0591); a block's end tested by three byte compares, not a vector compare |

### The six differences after the twelve fixes: done (decision 0602, 2026-11-22)

The same reading, after 0602.

Tier 2 against `walk`:

| | `walk` | `tier2.rs` |
|---|---|---|
| A string's read | `backslash_quote`, 32 bytes a step | the same: `S::marks32`, `x += 32`, the backend chosen once (`run_best`) |
| A primitive | `primitive`, called from each place's `_` | the same: `value_primitive`, written once, each place's `_` (the byte matched at the place, then in it, as `walk`'s) |
| `\u`'s digits and surrogates | the four digits a number; the surrogates its ranges; a high one's low one checked and marked (`low_checked`) | the four digits a number (`cp`, `HEX_VALUE`); `high` and `low` ranges of it; a high one's pair from its next escape's two digits, a number too; a low one's high one one position (`after2`) |
| A node's write | the columns' stores, through pointers in `Walk`'s locals | the cells' stores through `Cursor`, `run`'s local; the state's op only at a block's first node, a close of a block before, or a bound that fails |

Tier 1 against `index`:

| | `index` | `tier1.rs` |
|---|---|---|
| The masks made of the strings | `scalar`, `follows`, `in_string`, `string_tail` | the same names and expressions; `body` too, for the escapes |
| The structurals | `(op \| scalar & ~follows) & ~string_tail` | `((q \| op) \| nonquote_scalar & ~follows) & ~string_tail`: the quote outside `~follows`, as the grammar refuses `1"a"` in tier 2's tree, not in a number's end |
| UTF-8's end | `is_incomplete`: one compare against `max_value` | the same (`gt_bits`, `MAX_VALUE`) |

### The generated Rust in the lean parser's shape: done (decision 0604, 2026-11-22)

The same reading, after 0604. Each row was a difference after 0602.

Tier 2 against `walk`:

| | `walk` | `tier2.rs` |
|---|---|---|
| The `\u` pair | a high one's low one read as four digits; that escape's checks skipped (`low_checked`) | the same: `n7` four digits, a range of it; `if after2 != y { … }`; a low one reached otherwise lone (`if m3`) |
| A node's bounds | none: the walk's order is its contract | the same: `Cursor::begin`'s contract, checked in a debug build |
| A run of closers | `ScopeEnd`'s loop, `object[d]` deciding | the places returned to one arm, `'ret: loop { match st { … } }`, a closer `continue 'ret` |
| A separator, an opener, a `:` | `take!()` | `i += 1` alone; a closer's end kept, its node's |

Tier 1 against `index`:

| | `index` | `tier1.rs` |
|---|---|---|
| The records | `structural_start = (op \| scalar & ~follows) & ~string_tail` | the same expression (`v_m_structural_start`); a quote after a scalar refused by the grammar's error in tier 1, where simdjson refuses it in stage 2 |
| The file's bits | none: the last block padded with spaces | none (no record or error holds at a space, proved) |
| The byte order mark | the blocks start after it | the same (`off`) |
| The string mask's name | `in_string` | the same |
| A string open at the end | `prev_in_string != 0` (`finish`) | `c_m_in_string != 0` |
| The records' write | `bit_indexer::write`, the block before's at each block, the last after the loop | the same (`write_indexes`, `prev_structurals`) |

What still differs is format 16's: tier 2 writes blocks of nodes, with the state's op at a block's first node and at
a close of a node in a block before; `walk` writes simdjson's columns.

## Step 58, done: the benchmark on the generated Rust, `rust_stream`'s tests gone (decision 0555)

`simdjson_form_against_simdjson` now times `rerail-json-aot` (tier 1 and tier 2 from the grammar) against simdjson.
`simdjson_forms_rust_trees_are_the_jits` and `aot_tests`, the tests of `rust_stream`'s output named in the steps
above, are removed; `json_rusts_tapes_are_the_jits` compares with the JIT in their place. On the M3: `twitter.json`
77.9 instructions a byte (tier 1 36.3, tier 2 41.2) against simdjson's 5.8, 24.8 times its time; `numbers.json` 69.7
against 14.3, 5.5 times. The benchmark found that tier 1 refused UTF-8: an error read a name (`high`) the grammar
defines again after it.

## Step 57, done: the cells at constant widths (decision 0554)

The generated parser names its language's widths (`WIDTHS`) and calls the writer's open, leaf and close at them as
constants (`Fixed`): each column's offset and each cell's width known where the code is compiled, a cell one store.
The writer's other callers keep the state's widths, with the same code.

## Step 56, done: the empty container and the root's closer, in the loop (decision 0553)

Steps 49 and 53 again, from the grammar: a level whose rule is a one-byte opener, what may be empty and a one-byte
closer peeks at its call for the closer and writes one leaf (`empty!`); where its call returns to the root's end,
the file's last record is tested first (`last != '}'`).

## Step 55, done: the loop's writer reserved once (decision 0552)

The generated tier 2 reserves its run's room once, as the JIT does, and calls the writer's ops on its state: an
open, a leaf, a close and a commit cannot fail, and the writer's failure is read once after the walk. On the way,
`reserve_final` had reserved no laid entry for a run's blocks (only for the blocks made before it).

## Step 54, done: JSON's Rust from the grammar, in one loop (decision 0551)

Steps 41 to 53 wrote tier 2's Rust from its native IR (`rust_stream`), which 0550 deprecates. The Bitlex compiler
now writes JSON's parser from the grammar, as it writes Kotlin's: tier 1 from the lexer's statements, as simdjson's
stage 1 (each token's first byte; no `skip`), and tier 2 from the tree rules as one loop, simdjson's stage 2's
shape:

| | simdjson's stage 2 | tier 2 as one loop (`rerail-json-aot`) |
|---|---|---|
| The walk | `goto` between `object_begin`, `object_field`, `object_continue`, `array_begin`, `array_value`, `array_continue`, `scope_end` | `'run: loop { match st { … } }`: each nested rule's first place, and each place after a level's call |
| A level | `depth++`, its kind on the stack | its node opened by the caller, its return place pushed, `DEPTH` checked |
| A value | `switch` on the token's byte | `match` on the next record's first byte; `true`, `false`, `null` by the word |
| A string | `parse_string` at the quote | read at its token: content and escape leaves, each escape one of its kind's words |
| A number | `parse_number` | `check number`'s function of its bytes |

Its tapes are the JIT's, byte for byte, in format 16 and 16z, on the tree test's files; the refusals are the JIT's.
Not yet: a writer with its room reserved once and constant widths, and steps 45 to 53's work in this form.

## Step 53, done: the document's closer checked before the walk (decision 0579)

| | the lean parser | before | now |
|---|---|---|---|
| The outer container | `last != '}'` before the walk, then `next == index + count` after it | the count after the walk alone | `if s.n == 0 \|\| s.byte(starts[(s.n - 1) * rs]) != 0x7d { refuse }` in the `{` arm (`0x5d` in `[`), then the count |

## Step 52, done: a return two ways (decision 0578)

| | the lean parser | before | now |
|---|---|---|---|
| A container's return | `scope_end`: `d == 0` ends the walk, else `object[d]` | `match ret { 0, 1, 2, _ => refuse }`, and `keep_ret(0)` at the document's calls | `if s.depth == 0 { … } else if ret == 1 { … } else { … }`, no number kept for the document |

## Step 51, done: a close that cannot fail (decision 0577)

| | the lean parser | before | now |
|---|---|---|---|
| A node closed | `close`: two stores, no failure | `s.close::<…>(…)?`, the writer testing for an open node, the bounds and the room | `s.close::<…>(…);`, the writer asserting them |

## Step 50, done: the end the stream's count, tested once (decision 0546)

| | the lean parser | before | now |
|---|---|---|---|
| The end | `next == index + count`, once, after the outer close | a token taken past the count, `t.start == s.len`, and `ACCEPT`'s `t.start == self.len` | no token taken, `if (t.next as usize == s.n)`, `ACCEPT` tests the open nodes alone |

## Step 49, done: an empty container one leaf (decision 0545)

| | the lean parser | before | now |
|---|---|---|---|
| `{}`, `[]` | `empty!`: the next token is the closer, so one leaf | `open16`, a call, the call's switch on `}`, `close`, the return | `if s.byte(starts[t.next]) == 0x7d { … s.leaf16(7, st, t.rec_end); … }`, else the open and the call |

## Step 48, done: a node's open and leaf cannot fail (decision 0544)

| | the lean parser | before | now |
|---|---|---|---|
| A node written | `open`, `leaf`: the columns hold `bound` nodes, reserved once | `s.open16(…)?`, `s.leaf16(…)?`, each a refusal where the writer failed, each `pending = 0` | `s.open16(…);`, `s.leaf16(…);`: the room reserved once, the nodes' count checked once (`fits`), no `pending` |

## Step 47, done: a token one load and one add (decision 0543)

| | the lean parser | before | now |
|---|---|---|---|
| A token taken | `take!()`: one load, one add; the index ends in sentinel entries | the index clamped to the count, the end selected, `last_end` copied | `t.start = *starts.add(t.next * rs); t.next += 1;`, the stream ended in two entries of the file's length |
| The last token's end | the `end` it holds | `t.last_end = t.rec_end` at every token | read from `rec_end` where no arm made it again (`,`, `:`); kept by the `}` and `]` arms before they make theirs |

## Step 46, done: the token's state the rule's local (decision 0542)

| | the lean parser | before | now |
|---|---|---|---|
| The token's state | `walk`'s locals `next`, `v`, `end` | fields of `S`: `s.start`, `s.rec_end`, `s.next`, `s.last_end`, `s.code` | `T`, the rule's local (`let t = &mut T::NEW;`): `t.start`, `t.rec_end`, … |
| A node's start | `open(…, start)`, `leaf(…, start, end)` | read from `self.start` | given: `s.open16(7, t.start)`, `s.leaf16(13, t.start, t.rec_end)` |

## Step 45, done: one dispatch, its values named by the grammar (decision 0541)

| | the lean parser | before | now |
|---|---|---|---|
| Dispatches | one `match label`, six labels | three nested `match d` loops, arms that only pass a value on | one `match d`, four values; the object's and the array's loops each entered once |
| A value's name | `ObjectBegin`, `ArrayValue`, `ScopeEnd` | its place (`L::Token32`, `L::Span40`) | what the place does, from the grammar's names: `L::ObjectBegin`, `L::ArrayBegin`, `L::ArrayValue`, `L::End` |

## Step 44, done: a string's place its argument, a dispatch's value named (decision 0526)

| | the lean parser | before | now |
|---|---|---|---|
| A string's place | `self.string(sink, v, field)`: an argument | `s.part_at`, set by the `"` arm, read by `string` | `s.string(s.start + 1)?`, `fn string(&mut self, mut at: u32)`; no field |
| Where a dispatch goes on to | `Label::ObjectBegin`, … | `d = 3;`, `3 => {` | `d = L::Close41;`, `L::Close41 => {`: one enum, each named for its place |

Left, set beside the lean parser's `walk` (0526, "Left"): three dispatch loops and arms that only pass a value on
where it has one `match label`; names of places, not of the grammar's meaning; the token's state in `S`'s fields
where it keeps locals; `token_all`'s clamp and end test per token where `take!()` is one load; a writer call per node
that can fail; empty containers through the general path; the end tested at each token.

## Step 43, done: a token's end made only where a word reads it (decision 0538)

| | the lean parser | before | now |
|---|---|---|---|
| A punctuation's end | none kept; an end computed where it is written | `s.rec_end = s.start + 1;` in every one-byte arm | only where a word reads it (`end_read`): `}` and `]`, not `,`, `:`, `{`, `[`, `"` |
| The last token's end in a string | none | copied at the loop's head and after each content | only where a word after the closer reads it: in JSON, nowhere |

Left: `s.part_at`, a field where the lean parser passes an argument (0538, "Left").

## Step 42, done: each word once, one room check, coded refusals, the source laid out (decision 0537)

These close the five gaps found when the generated JSON Rust was set beside simdjson's and the lean parser's:

| | simdjson / the lean parser | before | now |
|---|---|---|---|
| A string, an atom, a number | `parse_string`, `is_valid_*_atom`, `parse_number`; `string`, `primitive`: one each, called from each place | the loop and the tests written again at each place (four string loops) | `fn string`, `fn atom`, `fn number`, one each, called from each place |
| The stack's room | sized once from the input | `open_node` tests `open_cap` every node | reserved for the grammar's bound (`open_bound`), tested once at the start |
| The tests' language | — | the files' keys: a word no file reached refused | every key a direct run writes (`direct_entries`), then the files' |
| An error | a `Code`, after `cold_path()` | `Err(false)` | `refuse(why::…)`, `#[cold]`, its reason returned and named by the host |
| The source's form | indented, labels named by state | flat, `'b21`, `'m129` | indented, `'token_130: loop`, `break 'to_close_22;` |

The bound's formula, the reasons and the label scheme are the implementer's; 0537 lists what the tests hold and
what they do not. Every run's calls are limited to 1,024 nested, the JIT's and the interpreter's too (the user; 0537).

## Step 41, done: the kinds' numbers constants (D7, decision 0535); the tape's reads written directly (0534)

**D7.** A direct run's node numbers are literals at their words, as the lean parser writes a kind's constant and
simdjson a container's character. The generator takes the language whose numbers it writes
(`rust_source_16(…, lang)`). Each `OPEN` is then `s.open16(12)?`, each `LEAF` `s.leaf16(4, s.rec_end)?`, and a
keyword leaf a `match` of its word to its number, as the lean lexer's keyword `match`. A word whose node the language
has no number for refuses the file there (`UNNUMBERED`), as the JIT's run does at `fail::NUMBER`. The source states
its language (`LANG`, the table's fingerprint), and its runs take that language:

- rr's AOT build is generated for rr's shipped table (`f16::lang(13)`), the language production writes rr with;
- the tests generate for one language a grammar, every key its documents' format 15 tapes hold, made before the
  Rust is written, in place of one table a file.

| | simdjson / the lean parser | before | now |
|---|---|---|---|
| D7. A kind's code | a constant (`'['`, `'t'`) | `number16`: `numbers[k]` loaded every node, from the language given at run time | a literal at the word; a keyword's by a `match` of its word |

**The tape's reads** (`tape_format/src/reads.rs`, 0534) are a hand-written reader of format 16 and 15. They are no
longer the reads' CLIF translated, and a test holds them equal to the JIT's. The relations' and the numbering's Rust
reads are still translated, and are next.

## Step 40, done: one writer in Rust written directly, one stack (decision 0533)

Order items 2, 3 and 5 of step 38, and D2. The writer is one Rust source written directly,
`tape_format/src/writer16.rs`, in simdjson's `tape_builder` and `tape_writer` shape:

- its state's named fields;
- the block switch, the side lists, `settle`, `finish` and 16z's pieces and LZ4 as plain functions;
- each cell a store at its column's offset.

The host's writer runs it, and a generated tier 2 holds the same text as its module `w`. No Rust is translated
from the writer's CLIF any more; the CLIF is the JIT's and the device's.

A direct run has one stack of open nodes, the writer's frames, as simdjson has `open_containers`. The walk's slot
for a call's number is the frame's free word, and `close` and `accept` read the writer's counts. Next to the
references:

| | simdjson / the lean parser | before | now |
|---|---|---|---|
| D2. The open nodes | one stack | tier 2's (`S::open`, `nopen`, `nodes`) and the writer's frames | the writer's frames; the walk's slot at byte 12 |
| D5. Calls | `allocate` out of line | the cold paths through the translated ops | the cold paths plain Rust functions, `#[cold] #[inline(never)]` at their calls |
| D8. The source's form | named fields, expressions | the translation's `let mut vN`, offsets from `desc` | named fields, expressions, everywhere in the writer |

Left: D7, the kinds' numbers as constants. A number is the language's code table's, given at run time (the tests
make one per file from format 15's first-use table), so constants need one table per language, known when the
Rust is generated. Done in step 41 (decision 0535).

## Step 39, done: the writer's per-node work written directly (decision 0532)

Order item 1 of step 38. `rust_stream` writes `W16`, a direct run's writer, in the lean parser's `Walk` shape. Its
state is a field of the walk, and `open16`, `leaf16`, `close` and `accept` call its methods. Next to the two
references:

| | simdjson / the lean parser | the generated Rust, before | now |
|---|---|---|---|
| The writer's state | fields of a local struct (`tape_builder`, `Walk`) | the ops' `State` behind `desc`, loaded and stored every op | `S::wr`, a `W16`: read once at the start, stored only before a cold call and at the end |
| A node's place | a cursor (`next_tape_loc`), a cursor a column | `(b, j)`, the block's room from the table, every op | the current block's room kept; a cell at `room + at[r] + j * w[r]` |
| The calls a node | none | `w_open` + `w_commit`, `w_leaf` + `w_commit`, `w_close`, out of line | `open`, `leaf`, `close` inline; no commit |
| The cold paths | `allocate`, out of line | inside each op | a block's room, a side list, `settle`, a failure: `#[cold] #[inline(never)]`, still the ops |
| The source's form | named fields, expressions | SSA values as `let mut vN`, offsets as literals | named fields, expressions, in the per-node work |

Left of step 38's order: the cold paths and the document's end written directly, so that the `w` module leaves the
source (items 2 and 3); one stack of open nodes, not tier 2's beside the writer's frames (D2); the kinds' numbers
constants (D7); the host's writer (item 5).

## Step 38: the writer's ops next to simdjson's tape builder and the lean parser's `Walk`

**Why this step.** Steps 35–37 picked and kept their cuts by profiles and instructions a byte. The user (2026-11-20):
"no decisions on cycle counts, instruction counts, time, or anything like that. I want you to make the GENERATED RUST
CODE MORE LIKE THE SIMDJSON AND LEAN PARSER CODE." This step reads the generated Rust of the simdjson form's format 16
run (`json_tests::simdjson_forms_rust_trees_are_the_jits`, kept with `RERAIL_KEEP_AOT=1`) beside:

- simdjson's stage 2: `json_iterator::walk_document`, `tape_builder` and `tape_writer`;
- the lean parser's stage 2: `json_lean/src/stage2.rs`, `Walk::open`, `close` and `leaf`.

The walk itself (`r0`) already has `walk_document`'s shape (the table above). What differs is what each node's word
calls: `open16`, `leaf16` and `close`, which call the writer's ops `w_open`, `w_leaf`, `w_close` and `w_commit` in the
module `w`. The module is the ops in CLIF (`clif/src/writer.rs`), translated to Rust.

**The differences, as source shape.**

| | simdjson | the lean parser | the generated Rust |
|---|---|---|---|
| D1. Where the writer's state is | `tape_builder` is a local of `parse_document`. `tape.next_tape_loc` and `current_string_buf_loc` are its fields, and every visit is inline, so they stay in registers for the whole walk | `Walk` is a local struct: the columns' cursors, `n`, `block_base`, `last_start` | the writer's `State`, behind `desc`. Every op loads what it needs from it (`n`, `prev_start`, `source_len`, `open_len`, the parts' offsets) and stores `n`, `prev_start` and `open_len` back |
| D2. The open nodes | one stack, `open_containers[depth]` (`tape_index`, `count`), and `is_array[depth]` | `open` gives back `ord`, and `close(ord, end)` takes it: the walk's own state | two stacks and two node counts: tier 2's (`S::open`, `nopen`, `nodes`) and the writer's (the `OPEN` part, 24 bytes a frame: node, children, last child, start; `State::n`, `open_len`) |
| D3. A node's place | `*next_tape_loc++ = …`: one cursor | `self.code.add(n).write_unaligned(c)`: a cursor a column; `block_base` set where `n % STRIDE == 0` | each op works out `(b, j) = (n >> 10, n & 1023)`, then the block's room from the table (`room_of`) and its side area (`area_of`), then each cell's address from the column's offset |
| D4. Checks | the depth, once a container, in the walk; the tape's room once, at `allocate` | none: the columns are reserved for the bound once (`reserve_columns`) | the depth in the walk and the open part's room in `w_open`; in every op, the start's and end's bounds (`start < prev`, `start > len`, `n + 1 >= MAX_NODES`), a node outside the root, a close with none open; tier 2's `number16` checks for `u32::MAX` |
| D5. Calls | every visit `simdjson_inline`; nothing called a node | every method `#[inline(always)]`; `reserve_columns` alone `#[inline(never)]` | `w_open`, `w_close`, `w_leaf` are `#[inline(never)]` and called at every node; inside them, calls for the first node of a block (`w_new_block`), the slot of a block's first node (`w_put`), a block that is final (`w_settle`) and a failure |
| D6. A node made final | written final, but for a container's start word, patched at `end_container` | written final | `w_commit` after every `w_open` and `w_leaf`: the frontier moved to `n`, though in a direct run every node but the open ones is final as it is made |
| D7. A kind's code | a constant (`'['`, `'t'`) | `code_of`: a table the walk keeps, its first use cold | `number16`: `numbers[k]` loaded every node, from the language given at run time |
| D8. The source's form | named fields, expressions | named fields, expressions | the CLIF's SSA values as `let mut vN` declared at the top (hundreds an op), each phi as an assignment before a `break`, each field as an offset from `desc` (`v0 + 128`) |

**The order, by what unblocks what.** Corrected the same day (the user: "we should not be translating Rust from
CLIF. We should be generating it directly. I already told you to do that."): the order first planned these as edits
of the ops' CLIF, whose Rust is a translation. As tier 1's Rust is written directly (`rust_block`, 0451), so is the
writer's:

1. **`rust_stream` writes the writer's per-node work directly**, as methods of the walk's state, in the lean
   parser's `Walk` shape: the node count, the last start, one stack of open nodes (tier 2's, its frames the
   writer's), the current block's room. Each cell is a store at its column's offset from the block, its width and
   escape constants of the generation (D1, D2, D3, D5, D8). The rules are the reference writer's
   (`f16/reference.rs`): `open`, `close`, `leaf`, `put`, `put_code`.
2. **The block switch and the side lists are cold**, as simdjson's `allocate` is: a block's room once every 1,024
   nodes, the room checked there for its nodes (D4, R14 kept), an escaped value's side-list entry, a block made
   final (`settle`, its presence). No commit a node in a direct run (D6).
3. **The document's end written directly**: `finish` (the last block moved down, the tail laid) and 16z's
   compression, so the `w` module is no longer in the generated source.
4. **The kinds' numbers constants** where the language is known at generation (D7).
5. **The host's writer** (`f16::Writer`, R16) the same direct Rust, not the translation; the ops' CLIF stays the
   JIT's and the device's (R15), checked against the reference as now (R13).

Each step is judged on the generated Rust beside `Walk` and `tape_builder`, and kept by the tests that compare
the tape with the JIT's and the reference's byte for byte (`aot_tests`, `tier2_writes_format_16_by_the_ops`).

**Steps 35–37 re-decided by shape.**

- **Step 35 (0528), one writer kept from one document to the next:** kept. simdjson keeps its `parser`, its
  buffers allocated once; the lean parser reserves its columns once (`reserve_columns`).
- **Step 36 (0529), the newline index from tier 1's bits:** kept. Neither reference has a newline index, but
  simdjson's stage 1 makes, in its one pass over the bytes, every mask stage 2 needs. The amendment (0529 §5) is
  the lean lexer's shape: a token loop's tier 1 makes no mask, and its index is the scan's.
- **Step 37 (0530), the per-node ops:** `w_leaf` as one op is kept: it is the lean parser's `leaf`, `open` and then
  its two writes. The other cuts (`settle` only where needed, `w_commit` hot, a node's block once an op) are steps
  inside the ops whose shape items 1–4 replace. They are superseded by this order (the ops' CLIF is no longer the generated Rust's source), not kept as reasons.
- Their counts tables are withdrawn as reasons.

## Step 37, in progress: the writer's per-node ops (decision 0530)

The third of step 34's three changes. About two thirds of what was left was the writer's ops, called at every node:
about 300 instructions a node, where simdjson writes one tape word a value. The cuts so far:
- `w_leaf` is one op, not `w_open` then `w_close`;
- `w_close` makes no call to `w_settle` unless the block can be final;
- `w_commit`'s common case is inlined;
- each op works out its node's block (room, side area) once, not at each cell;
- a cell's mask is its array's escape, already loaded;
- a side-list entry past the list's last is placed with no search.

| | `twitter.json`, instr/byte | `numbers.json` |
|---|---:|---:|
| Before | 48.7 | 45.0 |
| Now | 38.4 | 34.6 |
| simdjson | 5.8 | 14.2 |

## Step 36, done: the newline index from tier 1's bits (decision 0529)

The second of step 34's three changes. Tier 1 now outputs the stream's `\n` bytes (`BlockProgram::newlines`), and the
format 16 writer makes the newline index from those words (`newlines::index_of`, through `Writer::begin_with`), not by
scanning the source again. The index is the scan's, byte for byte.

Instructions a byte are unchanged (`twitter.json` 48.7, `numbers.json` 45.0): `memchr` cost few instructions, but its
pass over the source cost time. In a `twitter.json` profile the index went from about 13% of the parse to about 4%.
Tier 1 went from 4.35 to 4.58 instructions a byte, for the one output more.

Left from step 34: the writer's ops, which are most of what remains, against simdjson's tape writes.

## Step 35, done: one writer from one document to the next (decision 0528)

The first of step 34's three changes. Tier 2's format 16 run now writes into a writer the caller keeps, as simdjson's
caller keeps its parser (`Parser::image16_in`). `Writer::begin` keeps each part's buffer and zeroes no room. Only the
presence's words, and in 16z the buckets and the LZ4 tables, are set for each document. A tape given back
(`Writer::recycle`) is the next one's room.

| File | A writer a document | One writer | simdjson |
|---|---:|---:|---:|
| `twitter.json`, instructions a byte | 59.9 | 48.7 | 5.8 |
| `numbers.json` | 56.2 | 44.6 | 14.2 |

Ours against simdjson's time: 13.5× → 9.4× on `twitter.json`. Of what is left on `twitter.json`, in a profile: the
writer's ops about 66%, the walk (`r0`) 16%, the newline index 13%. The zeroing is gone.

## Step 34, measured: the simdjson form end to end against simdjson (order item 4)

`json_tests::simdjson_form_against_simdjson` times both on simdjson's own inputs:
- ours: tier 1 on Cranelift, then tier 2 as the generated Rust (rustc, `-C opt-level=3`), writing
  format 16 (`image16`);
- simdjson's: its DOM parser, stage 1 and stage 2 to its tape (`rerail_simdjson_native::structurals`).

Measured with `--profile profiling` (debug gives the same counts), arm64, one thread, load about 20,
the median of 50 runs:

| File | Ours, instr/byte | simdjson, instr/byte | Instructions | Time |
|---|---:|---:|---:|---:|
| `twitter.json` (631 KB) | 60.6 | 5.8 | 10.4× | 13.5× |
| `numbers.json` (150 KB) | 56.9 | 14.2 | 4.0× | 4.1× |

Tier 1 alone is 4.35 instructions a byte on `twitter.json` (0.20 ns a byte). The rest is 55, about
640 instructions per simdjson structural.

Where the rest goes (`RERAIL_SPIN` and `sample`, `twitter.json`, about 4,300 samples):

| Where | Share |
|---|---:|
| zeroing the format 16 writer's room (`Writer::reserve_run`, `reserve_final`, `Writer::new`) | ~30% |
| the writer's ops, in the generated Rust (`w::f…`) | ~45% |
| the walk, `r0` (`walk_document`'s counterpart) | ~11% |
| format 16's newline index of the source (`newlines::index`, in `Writer::new`) | ~9% |
| tier 1 | ~5% |

So the walk is not the gap. The gap is around it, in format 16's writer:
- **A writer per document.** `write16` makes a new `Writer` for each file. Its room is reserved for
  the worst case, every block made final at once: `LAID_MAX`, about 37 KB a block. `Vec::resize`
  zeroes it all, about 3 MB for `twitter.json`. simdjson allocates its tape once per parser and
  reuses it.
- **The newline index is a pass of its own** over the source. Tier 1 already reads every byte.
- **The writer's ops cost four times the walk.** simdjson writes a tape word per value.

## Step 33, done: Kotlin's steps compile in seconds, not minutes (decision 0484)

Step 29 ran Kotlin's ladder, but each step took 5 to 7 minutes to compile, almost all of it in
the Bitlex compiler's proofs. Those steps were then run on farm hosts over about 3 hours, instead
of the compile being fixed. The user: "no 3 hours runs ... fix the kotlin speed, starting with
tiny examples."

Measured on the small steps first, each phase apart (`RERAIL_STEP_TIMES`):
- the proofs (`bitlex_masks::compile_shaped`) took almost all of the time;
- Cranelift took 1 to 3 s.

The cause was Kotlin's walk. Its strings, characters and comments are regions that never overlap,
but the provers did not know this. They searched for a byte inside two regions at once, which no
stream has.

Decision 0484 makes these changes:
- each pair of a walk's kinds is a lemma to the fixed-point prover;
- a row whose start never holds is dead;
- the lowering and the provers hash with a fixed seed, so one grammar compiles to one program;
- the gap ends check stops at a real stream, before it proves its lemmas.

| Step | Bitlex compile before | after |
|---|---:|---:|
| 09 | 31 s | 1.8 s |
| 12 | 57 s | 1.9 s |
| 35 | minutes | 14.5 s |

The whole ladder, all 35 steps (`aot_tests::ladder_steps_rust_tapes_are_the_jits`, `RERAIL_AOT_LADDER=kotlin`,
load about 18): 420 s wall, the Rust's build 69 s of it; the Bitlex compile 165 s of CPU over the 35 steps, Cranelift
29 s. Every step's tapes are the JIT's. It took about 3 hours before, over farm hosts.

Step 35's tier 2 is 15,424 words, down from 23,146. Kotlin's tests against the lean parser all
pass. The corpus is unchanged: 429 the same, 62 refused, 0 differ.

Then Kotlin's ladder was removed (the user: "stop keeping them at all"). Steps 01 to 34 are
deleted, and the grammar is `kotlin_parser/kotlin.bitlex`, the last step. The AOT test runs it as
Kotlin's one step, in 82 s.

## Step 32, done: the commit point moved back by an edit, shown to be unobservable in format 16

Step 28 left one mutation uncovered: an edit before the commit point not moving it back (`uncode`:
`Mem::Coded` in the JIT, `self.coded` in the Rust). Two probe grammars were added to reach it:
- `WRAPPED_TWICE`: a `!` wraps the top level's node made, and wraps the wrapper again
  (`(vrai)!!`, `((vrai)!!)!!;vrai!`, 2,000 lists wrapped twice);
- `FOLDED_CHAIN`: the root's two leaves folded, then each next leaf folded into the first
  (`vrai fausse vrai`, 2,000 more).

The mutant, in both backends at once, writes every one of their tapes as before. This is why, for
every grammar:
- a format 16 run commits only where a node at the top level closes (and at `ACCEPT`), and its
  commit point is that node's start, since a wrap may still move it;
- a wrap reaches the node made last, at or past the commit point, so it never moves the point;
- a fold needs the open node to hold two leaves and nothing else: at the top level, no node there
  has closed, so nothing was committed;
- what is left reaching before the commit point is a retag (of the root). Where the mutant passed
  the node, the retag goes to `w_retag`, and the tape is the same.

So the move back is kept, a guard, and the mutation is equivalent.

The test now also asserts what would have shown a failure both runs share: where the JIT writes
format 15, its format 16 run writes too. A document both refuse with `fail::PASSED` was otherwise
just not counted.

| | steps | format 16 homes, the JIT's | of 3,000 nodes or more |
|---|---:|---:|---:|
| the probes | 3 | 660 | 8 |

The double wrap found a fault on the way, in step 31's work: the Rust's `blocks`, called from a
wrap's shift, wrote format 15's block bases in a format 16 run.

## Step 31, done: a format 16 run holds none of format 15's buffers

Step 28 left the host reserving, for a format 16 run, format 15's image, its size, offset and tail
columns, its block bases and its code tables, and the rows for a run that needs none. Both
backends' code also still wrote the offsets, bases and tails in a format 16 run, which nothing read:
the writer makes its own blocks, and the pass reads only the sizes.
- `Tape::sized` reserves format 15's buffers only for a format 15 run. The rows, their sizes and
  the supertypes are reserved only where nodes are passed from rows (not `direct`, step 30). The
  stack of open nodes stays: the Rust's format 16 run keeps its own there.
- The room checks read the tape's room (`Tape::room`), not a buffer's capacity.
- The JIT's `push_off`, `blocks` and `cut_to` write nothing in a format 16 run, and `close_cells`
  writes only the size. `shift_rows`, `drop_rows` and `wrap_word` move or recompute no tails. The
  Rust's are the same, under `SIXTEEN`.
- A format 16 tape takes no code-id table from the thread, and gives none back.

**The check:** `Tape::held` gives the bytes a tape holds of format 15's buffers, and of rows. For
each ladder step the format 16 tape holds none of the first, and none of the second exactly where
the run is direct. An empty buffer's pointer is in the unmapped first 4 GB, so any read or write
of one faults. The mutation (the JIT's `push_off` writing the offsets again) faults on the JSON
ladder.

| | steps | format 16 homes, the JIT's | of 3,000 nodes or more |
|---|---:|---:|---:|
| JSON's ladder | 15 of 15 | 3,300 | 30 |
| `.rr`'s ladder, steps 1 to 6 | 6 | 1,464 | 12 |
| the probes | 3 | 660 | 8 |

On farm hosts, after the change:

| | steps | format 16 homes, the JIT's | of 3,000 nodes or more |
|---|---:|---:|---:|
| `.rr`'s ladder | 22 of 22 | 5,540 | 44 |

`rerail_rr_aot` with its generated tiers (`--features generated --profile profiling`): its 21
tests, `aot_format_16_tapes_are_the_jits` among them (2,017 files, 4,014 homes, each the JIT's).

## Step 30, done: the JIT's direct run, each node to the writer's ops at its word

Where each node's key is known at its word, the Rust passes each node to the ops as it is made
(steps 25 to 27). The JIT still wrote rows and passed them at commit points. Now it does what the
Rust does:
- `rust_stream::direct_keys`: where `coded` holds, and no field is provisional and no leaf is
  deferred or an extra, each node word's key bits from the words after it (`key_after`: its field,
  its supertypes' classes). The JIT takes them from the same analysis, so the two cannot disagree.
- `OPEN`: the number by the key (`number_of`), `w_open` and `w_commit`, one more open. `LEAF`: `w_leaf`
  and `w_commit`. A key with no number is `fail::NUMBER`.
- `CLOSE`: `w_close` at the token's end or the last one taken's. `ACCEPT`: `w_close` at the file's
  end. `NODE_FIELD` (here) and `NODE_SUPER`: nothing, their bits are in the number.
- No row, no supertype record, no commit, no pass.

A word's body is shared between places by their ops alone (`shared_runs`). Two runs with the same
ops but different key bits then shared one body, and the second place's leaf had the first's
field: `{"k":false,"t":true}` failed with `NUMBER` at node 5. Runs now share only where their key
bits are the same too.

**The check** (`simdjson_forms_rust_trees_are_the_jits`):
- the JIT's run is direct, and calls no step on the rows (`CompiledStream::row_calls`, from the
  CLIF's calls: `blocks`, `commit`, `uncode`, `finish`, `pass`, `retag`);
- its tape holds no format 15 buffer and no row (step 31);
- its home is the writer's of the file's format 15 tape (`f16::write_from_with_presence`), not only
  the Rust's.

96 inputs (65 parsed), and with simdjson's `jsonexamples` 120 (87 parsed), 16 and 16z. The
ladder test asserts that the JIT is direct exactly where `direct_keys` is.

Mutation: the field from `key_after` dropped: `NUMBER` at node 3 on the first file.

Only the simdjson form is direct. `coded` is computed on the one-function form (`OneFn`), which
the Rust backend makes only where tier 2 makes the records. No ladder step is that form, so none
is direct (0 of 15, 0 of 22).

## Step 29, done: a token check that reads back, and Kotlin's ladder, in the generated Rust

JSON's steps 14 and 15 were refused: the `\u` check reads 6 bytes before its token (0428), and
the Rust had no such check. It has now, as the JIT's (`token_check_short`, `ended_word`):
- the gate first (`token_gate`): the token's last byte against the check's 256-bit set, else no
  frame;
- the frame from `back` bytes before the token, clipped at the file's start (`pre`), each window
  shifted by what lies before the file;
- `ENDED` from the records before the token's: at most `back` of them end in the frame;
- `START` and `LAST` at `back` and `back + n - 1`, `WITHIN` from `pre`.

A region tier 2 splits stays refused for such a check, as in the JIT (0428).

Kotlin's ladder was refused at its first step ("more than one kind of extra"). The refusal is gone:
the Rust took each extra already. Kotlin's steps are compiled as `kotlin_parser` compiles its
grammar (`ladder::compile_step_as`: shaped by its rules, tier 1's blocks padded, the clusters by
rule).

Two compiler faults turned up on Kotlin's steps:
- step 8: tier 1 refused with "region 6's value on the vectors made outside its branch" (0423).
  The value was not made outside: `region`'s gate test carried the location of the last value
  made before it. `region` now clears the location before the test, as before its skip block.
- step 17: the prover panicked (`bitlex_prove`, no groups at a lag). The window's hypothesis
  (`whole_hypothesis`) reads values at lags the root never reads, and their bytes had no groups.
  `unrolled_readers` now walks from those places too.

The test gains the 0428 escape texts in three framings (at the file's start, after a string,
inside an array).

| | steps | format 16 homes, the JIT's | of 3,000 nodes or more |
|---|---:|---:|---:|
| JSON's ladder | 15 of 15 | 3,300 | 30 |
| Kotlin's ladder | 35 of 35 | 9,152 | 52 |

Kotlin's steps run in chunks on farm hosts: the JIT compiles a late step in 5 to 7 minutes there
(debug build), and the test compares only once every step of a run is built. A step alone had
no document past 3,000 nodes at step 35: one accepted sentence that does not parse joined to
itself refused every join. The long documents now join only the sentences that do.

Mutations, each failing the check:
- the read-back frame from one record before only;
- the gate testing no byte;
- the windows not shifted at the file's start.

`WITHIN` from 0 in place of `pre` is equivalent: every `within` is ANDed with a byte test that
reads 0 before the file.

## Step 28, done: format 16 for every program the Rust backend takes, in the generated Rust

Steps 25 to 27 wrote format 16 only where each node's key is known at its word (`coded`): no edits,
one function. No ladder step was such a program, so `rust_source_16` refused every step, and every
`.rr` program. Where a word edits nodes it made (a wrap, a fold, a retag, a drop, a reread), the
format 16 Rust now runs as the JIT's (R17):
- the rows are the run's window;
- a node goes to the writer's ops at a commit point, one commit point behind (`pass16`, as
  `native_cpu_tape::pass`): the ops' open nodes whose subtrees end before it closed at their ends,
  then the node opened by its number and committed;
- each kind change goes through `retag_at`: a node the ops already have (the root, held open) is
  retagged there (`retag16`, `w_retag`); before the commit point, the commit point moves back
  (`uncode`); an edit of a node already passed is the run's failure (`fail::PASSED`);
- `ACCEPT` passes the rest with the root closed, and the host finishes the writer.

A failure is the writer's own (`State::bad`, set as `w_fail` sets it), so the host reads it as it
reads the JIT's, and a full stack of open nodes is retried deeper. The `coded` path of steps 25
to 27 is unchanged.

On the way: a token's check calls `window`, `nib` and `bits16`, which were written only with a
fetched token step, so a step with a token check and no fetch (`json` 11 to 13) gave Rust that did
not compile. They are their own block now (`VECTORS`), written for either.

**The check** (`aot_tests::ladder_steps_rust_tapes_are_the_jits`): every step of JSON's ladder
and `.rr`'s, in formats 15 and 16, built by rustc, one library a ladder, run on the JIT's harness:
- format 15's image is the JIT's, on each step's documents;
- format 16's home (16 and 16z, the tape and its presence) is the JIT's, or both refuse. The language
  holds every key the step's tapes hold, as the engine's holds its grammar's. Each of a file's own
  tables lacks the kinds an edit replaced, so the root's first kind had no number, and both runs
  failed alike.

The documents are the ladder's sentences and their mutations, the compiler's format 16
documents for JSON's ladder, and the accepted ones joined past 3,000 nodes. A probe grammar
(`RETAGGED_ROOT`) retags the root after lists were passed: neither ladder edits a node the ops
already have.

| | steps | format 16 homes, the JIT's | of 3,000 nodes or more |
|---|---:|---:|---:|
| JSON's ladder | 13 of 15 | 2,798 | 28 |
| `.rr`'s ladder | 22 of 22 | 5,540 | 44 |
| the probe | 1 | 206 | 2 |

JSON's steps 14 and 15 are refused in both formats: a token check that reads back (0428) has no
Rust yet. Kotlin's ladder is refused at its first step: more than one kind of extra. (Both taken
since step 29.)

Mutations, each failing the check:
- the ops taking nodes up to the commit point, not one behind (the probe: `NUMBER` at the root);
- no `retag16` (the probe);
- the root closed at a commit point.

Not covered (step 32 shows the mutation equivalent): the commit point moved back by an edit before it (`uncode`). The ops read each row as
they pass it, so a tape differs only if a second edit reaches a node between the first edit and
the old commit point. None of the grammars has that.

**The `.rr` parser built ahead of time** (`rerail_rr_aot`) now has its format 16 tier 2:
- `build.rs` writes `tier2_16_aot.rs` from `rust_source_16`;
- `aot` checks its fingerprint and sets it on the format 16 run (`Parser::sixteen_mut`, no longer
  a test hook).

`aot_format_16_tapes_are_the_jits`: 2,008 of the corpus's 2,018 files taken, 4,016 homes, each the
JIT's.

Left (both done since: step 30, step 31):
- The JIT's format 16 run is still the general one (rows, then `pass` at commit points) where
  `coded` holds. `coded` is the Rust backend's analysis of its one function (`OneFn::stacked`), which
  the JIT does not make.
- The host still reserves the rows, and format 15's image and code tables, for a format 16 run
  (`Tape::sized`, `Vec::with_capacity`: not zeroed, so a run that writes none touches none of their
  pages). Before none is reserved, each of the JIT's table functions (`shift`, `drop`, `blocks`)
  must be shown not to reach them in a format 16 run.

## Step 27, done: no working rows in the format 16 run, only a stack of open nodes, in the generated Rust

After step 26, each node still wrote a working row of 12 bytes (its kind and field, start, end):
- the open stack held node numbers, and a call's number was kept in its container's row;
- a `CLOSE` wrote its node's end there, and `ACCEPT` the root's;
- a `NODE_SUPER` wrote a supertype record, and a `NODE_FIELD` the row's field;
- a leaf's row was read once, by `leaf16`, right after it was written.

In a format 16 run none of that is read: each node's field and supertypes are in its number,
taken at its word (`key_after`), and the ops hold the nodes. So the format 16 source has:
- `open16`: the number, `w_open` at the token's start, `w_commit`, and a slot pushed on the stack
  of open nodes, which then holds the call number (`keep_ret`, `taken_ret`), as simdjson's
  stack holds a container's kind alone;
- `leaf16`: the number, `w_leaf` with the token's bytes, `w_commit`;
- `close`: the end, `w_close`, the stack popped;
- no code at a `NODE_SUPER` or a `NODE_FIELD`, and no row at `ACCEPT`.

`rust_source_16` refuses a deferred or extra leaf and provisional fields, which read rows; JSON
has neither.

**The check:** the rule function reads no row and makes no supertype record; `open16`, `leaf16`,
`close`, `accept`, `keep_ret` and `taken_ret` touch no row and no format 15 cell; and the bytes
are still the JIT's on all 82 files. `taken_ret` reading the wrong slot fails it.

Left (done in step 31): the harness still sizes the rows for the bound (`CompiledStream::tape`) though this run
writes none; that is the host's allocation, not the parse's work.

## Step 26, done: no format 15 cells in the format 16 run, in the generated Rust

After step 25, each node's word still wrote format 15's cells beside the ops' own: its offset
and its block's base (`push_off`), and at its close its size and tail (`close_cells`). Only
format 15's image reads them, and the edits (a wrap, a fold, a reread). A format 16 run has no
image, and `coded`, which it needs, holds only where no word edits a node made. So in a format
16 run the node words' methods (`open`, `leaf`, `close`, `accept`) make none of them
(`cells16`). Each node's cells are now written once, by the ops.

What each node still writes besides the tape is its working row (kind and field, start, end):
the open stack and the call numbers (`keep_ret`) live in the rows of the open nodes, and a
leaf's row is read once, by `leaf16`. simdjson keeps only its open containers on its stack.

**The check:** the four methods of the format 16 source make no format 15 cell, and the bytes
are still the JIT's on all 82 files. Keeping the leaf's cells in fails it.

## Step 25, done: format 16 written by the writer's ops at each node word, in the generated Rust

Step 24 left one copy: format 15's columns follow the code column, a column's place known only
from the node count, so `finish` copies them there at the end. Format 16, as decision 0434 laid
it out (merged from the hub's tape branch, 664c1f8411), has no such copy: its tape is blocks of
1,024 nodes at a fixed stride, so each cell's place is known when it is written. At the end the
writer moves only the last block's columns down and lays the side lists (`w_finish`).

0434's writer is a set of ops written once in CLIF (`w_open`, `w_leaf`, `w_close`, `w_commit`,
…), with Rust generated from them (`tape_format`'s `generated_writer`; since step 40, decision 0533, the writer's Rust is written directly, `writer16.rs`). The JIT's tier 2 writes
format 16 through them (0434's R17), but from its working rows: at each top-level close, a pass
over the nodes since the last one opens each in the ops (`native_cpu_tape::pass`). That is a
second walk over every node.

simdjson writes each tape word as it parses. Step 24's check (`coded`) already proves that
every node's key, and so its number in the language's table, is known at its own word. So the
generated Rust (`rust_source_16`) holds the writer's ops as a module of its own (`mod w`,
`WRITER_RUST`) and calls them directly, with no pass:
- the `OPEN` and `LEAF` words: `open16` and `leaf16` (the node's number by its key, then
  `w_open` or `w_leaf`, then `w_commit`, as `pass` commits after each node);
- `CLOSE`: `w_close` at the node's end;
- `ACCEPT`: the root's `w_close`, and no format 15 image (no `finish`, no `commit`).

The host's harness is the JIT's (`write16`: the writer's state and the numbers in `Ctx`, the
room reserved for the run, `w_finish` after).

**The check.** On all 82 parsed files, the Rust's format 16 and 16z tapes equal the JIT's run's
byte for byte. Each of these fails it:
- the classes left out of format 16's key (the numbers then miss; tier 2 refuses);
- the `w_close` at a `CLOSE` taken out (the ops fail at the root's close).

The test had a fault of its own: `aot_entry` built every source into one library path, and the
loader gives back a library already loaded at its path, so the second source ran the first's
code. Each source now gets a directory of its own.

**Not yet:** the Rust's format 16 run still writes format 15's working columns (sizes, offsets,
tails, block bases), which nothing reads in it. Step 26 takes them out.

## Step 24, done: each node's code as it is made, in the generated Rust

simdjson writes each tape word once, in place. Our image was written in three passes:
- the rows and columns as each node is made;
- the codes in `commit_from`, after the top value closed: over every node again, its key read
  back from its row, its supertypes' classes from the supertype records, then `id_of`;
- the columns copied after the code column by `finish`.

A node's key is its kind and field, plus the classes of the supertypes after it. Where these
are known when the node is made, its code is written then (`code_now`, `Facts::coded`). That
holds where:
- no word edits a node made (`stacked`'s words, and a field set only on the node made last);
- every `NODE_SUPER` and `NODE_FIELD` is reached only from a node's `OPEN` or `LEAF`, by words that
  make no node (`coded`).

The words after the node give its classes and its field (`key_after`). Ids are numbered by first
use, in node order, as `commit_from` numbers them. No commit is left, in `close` or in `accept`.

The first try missed the field an object's value gets after its supertype (`NODE_FIELD` after
`NODE_SUPER`). `commit_from` read the key late enough to see it. The trees test caught that
(`{"k":false,"t":true}`). But it compared nodes, whose supertypes come from the supertype records,
so a code without its classes passed it. **The test now compares the tape's bytes too:** on all 82
parsed files the Rust's image equals the JIT's. Dropping the classes fails it at the first file,
and so does dropping the later field.

**Open, a format decision: the columns where the image has them.** The image's columns follow
the code column one after another, and a reader finds each from the node count (`padded(2n) +
padded(size·n) + …`, `tape_format`). So a column's place is known only once the parse ends, and
`finish` copies the columns there. Writing them in place in one pass needs each column's offset
in the header, from the tape's capacity: a new format version for every reader.

## Step 23, done: a string as `copy_and_find`, in the generated Rust

Three differences in a string's code:
- **The stop read 16 bytes a step, and was read again.** `part_stop` found the first closer or
  escape, then the arm read its byte (`matches!(s.byte(i), 0x22)`) to tell which. It now reads
  32 bytes a step, makes the closers' mask and the escapes' (`closers`, `parts`: one compare
  each for JSON, as simdjson's `quote_bits` and `bs_bits`), and gives back whether the first stop
  is a closer: `c & first`, simdjson's `has_quote_first`.
- **A content part then scanned again from the stop it had found.** Its place goes straight back
  to the part's word (a leaf, a `GOTO`). That is now written in place (`straight_to`), and the
  stop already found is taken on at once.
- **A one-byte token's length was tested right after it was set:** `if s.start + 1 != s.rec_end`,
  at each string opener. The one-byte arms now go past their place's `SPAN` (`past_span`), as the
  closer's has since step 16.

The bytes past the file are 0 on every path (step 20), and 0 is never a stop, so the last step
needs no mask, as in simdjson. `fetched_helpers` refuses a region whose stops hold 0.

The Rust trees test asserts 32 bytes a step, no byte read again, each content part followed by
its stop, and no length test. Inverting the closer's flag fails it at the first file. Not writing
the content in place fails it on its assertion.

## Step 22, done: one stack, the nodes', in the generated Rust

There were two stacks: the calls' (`s.ret`, `s.depth`) and the open nodes' (`s.open`, `s.nopen`).
Each had a test: `depth == DEPTH` at a call, `nopen == 0` at every `CLOSE`. simdjson has one,
`open_containers[depth]` with `is_array[depth]`.

The two don't nest alike: the tree has more levels than the calls (the document node, a pair,
a string with its parts), so they can't share one counter. But each call's container brackets
the call. Its `OPEN` comes just before the `PUSH`, and its `CLOSE` just before the `RETURN`. So
`rust_stream` checks this (`stacked`):
- each `PUSH` right after its container's `OPEN` (only a supertype or field word between);
- each `RETURN` right after a `CLOSE`;
- each rule's opens and closes counting alike on every path, a call counting as the close of the
  container before it: 0 at the `RETURN`'s `CLOSE`, at least 1 at every other `CLOSE` and every
  `PUSH`.

Where it holds:
- the `PUSH` keeps its number in the container's row (`keep_ret`), in the `end` that nothing
  reads before the `CLOSE` writes it;
- the `CLOSE` before the `RETURN` takes it back (`taken_ret`), and the return matches on it;
- no `CLOSE` tests for an open node (`close::<_, true>`): the count proves one is open.

The call stack is gone, and the open-node test with it. `s.depth` stays for one thing, the limit
on nesting, as simdjson's `depth >= max_depth`.

The Rust trees test asserts no `s.ret`, a `keep_ret`, and every `CLOSE` sure. Keeping the number
in the node below the container fails it at the first file. **Not tested:** a program for which
`stacked` is false. JSON's holds, and no other form makes its records in tier 2.

## Step 21, done: no record field written that no word reads, in the generated Rust

simdjson's `advance()` writes nothing but the index. Each token here wrote record fields:
- `s.code` in 47 places, read in 9;
- `s.live` every token, read nowhere (no marks);
- `s.rec_end = 0` every token, for the kind paths.

Three changes, all in `rust_stream`:
- **The kind paths were dead.** The program's one `RETURN` gives no `again`, so a call goes back
  only to its target. `plain_succ` listed target + 1 for every `PUSH` too, so the three switches
  there were not `fresh`, and kept a kind path no run takes. In one function a call's
  successors are now the places its rule's returns go back to (`OneFn::after`). The kind paths
  go, and with them `rec_end = 0`: `token_all` writes it only where a switch has one.
- **An atom writes its own leaf.** Its arm knows the word it matched, so the arm writes the
  `OPEN` and the leaf with the kind as a constant (`known_leaf`, `word_with`), as simdjson's
  `case 't'` does. There was a shared leaf whose `word_kind()` matched on the code to find it
  again.
- **The code only where it is read** (`code_set`). `reads_code` walks the graph from the arm's
  place to the next `TOKEN`. A word it doesn't know counts as a read.

Now `token_all` is: the end before kept (`close` reads it), the next start loaded, the select,
the increment. The code is written only by the escapes, whose word `match` reads it.

The Rust trees test asserts both: every code write is an escape's, and no `live`, `rec_end = 0`,
`rec_end == 0` or `word_kind()` is left. An atom's constant from the next word fails it at the
first file. Not inlining the atoms fails it at their code writes.

A check that ran before the kind paths went: on 113 files no run switched a made token again.
That is why a mutation treating those switches as reading no code passed. The paths were
dead.

## Step 20, done: the end tested only where it is taken, in the generated Rust

Each token tested the end three times: `token_all`'s `at_end`, its `i == n`, and the value
switch's `if s.at_end`. simdjson tests none of them mid-document: the index past the last is the
file's end, and the switch on its byte refuses it.

The inputs already allow the same. Every caller has the file's padding zero (`run`, the batch's
64 bytes, the slab's `SOURCE_MARGIN`), and two readable slots past the starts' count. So where
the records are made in tier 2 (`Facts::sentinel`):
- `token_all` loads the next slot, at most the one past the last. Past the last, its start is the
  file's length: a select, no branch.
- A switch whose end refuses, and whose arms (and its delegate's) take no `0x00`, has no end test
  (`zero_refused`): the padding byte goes to its default arm.
- Where the end is taken, at the document's end, the test is `s.start == s.len`, and the kind
  path reads the end's kind from that.

This holds only where no `TOKEN` follows a switch's end, and with no marks, whose `live` at the end
is the end's own. Otherwise the field stays.

The Rust trees test asserts no `at_end` is read, and no end test is a refusal. Taking the slot's
value past the last token, in place of the length, fails it on the first file.

## Step 19, done: no dispatch arm only sets `d` again, in the generated Rust

A loop's dispatch had an arm with no code of its own (`1 => { d = 3; continue 'b151; }`): its
entry was a word whose code is only its jump. Code that came in with `d = 1` went round the
loop twice. `rust_stream` now tells `Shape` which words are only a jump (`Shape::threaded`).
Each edge labelled for such an entry carries the label of the entry it jumps to, and an arm that
no edge labels isn't written. The `}` that closes an object now sets `d = 3` itself.

Removing those words from the graph everywhere was tried first, and is worse. They are the joins
that keep a loop's entries few: without them the value loop had 25 entries, and the `\u` arm
went through the dispatch.

The Rust trees test reads each dispatch's top-level arms and asserts that none only re-sets `d`
for its own loop. Turning the threading off fails it at `'b151: 1 => { d = 3; continue 'b151; }`.

A return is now one dynamic `match`, on `s.ret`. Two of its arms jump straight to their place
(`break`). The third sets `d = 2`, a constant, then `continue 'b151`: that is how Rust writes a
jump into a loop entered at several places, simdjson's `goto object_continue`. Making it a
direct branch is up to rustc's jump threading, and the Rust can't show it.

## Step 18, done: one function, the nesting on a stack of its own, in the generated Rust

Each rule was a Rust function: a value inside an array was a call (`r46(s)?`). It came back with
`Ok(again)`, and the caller then switched on the token again. Where tier 2 makes the records,
`rust_stream` now writes the whole program as one function, as simdjson's `walk_document`
(`OneFn`):
- a `PUSH` stores a number on `s.ret[depth]`, then goes on at the rule's first word;
- a `RETURN` takes the number off, and its `match` goes on at the place that call returns to.

Calls that every `RETURN` sends to the same place store one number. JSON has three: the document,
an array's value and an object's value. In simdjson that is `is_array[depth]` and the test of
depth 0.

The graph's loops for a value are now entered from three places. `Shape` gives each such loop a
dispatch on `d`, three in all. Copying the nodes doesn't remove them: a budget of 64 times the
graph's size left the same three. They are what simdjson's `goto object_continue` /
`goto array_continue` are, in a language without `goto`. One step is still too many: an arm of
a dispatch that only sets `d` again (item 6 below).

The Rust trees test asserts one function and the stack's `match`. Storing a wrong number (`k + 1`
mod 3) fails it.

## The generated Rust next to simdjson's stage 2

The source is `rust_stream::rust_source` of the form (`json_tier2_by_word` writes it as
`simdjson.rs`). simdjson's side is `json_iterator::walk_document`, `parse_string`,
`parse_number` and the atoms.

| simdjson | the generated Rust | the same? |
|---|---|---|
| `walk_document`: one function, its states labels (`object_begin`, `object_continue`, `scope_end`) | one function, `r0`, its states loops and labelled blocks | yes, since step 18 |
| nesting on a stack of its own (`is_array[depth]`); `scope_end` goes back to the state of the scope it returns to | one function, `r0`: a call stores its kind of scope on `s.ret[depth]` (the document, an array's value, an object's value: three numbers); a `RETURN` matches on it and goes on at that scope's place. The loops a value is entered at from those three are a `match d` each: Rust has no `goto` | yes, since step 18; the dispatches are the `goto`s |
| `advance()`: the next index; the index past the last is the file's end, which the switch refuses | `token_all()`: the next start, past the last the file's length (a select); the value switches refuse its byte, the end tested only where it is taken | yes, since step 20 (no room check since step 17) |
| `switch (*value)` | `match s.byte(s.start)`, the record made in the arm | yes |
| one switch a value; `case '['`: `]` peeked for, then `array_begin` and the one value switch | the array's loop was entered at each value's place from two switches (the first value's, which also takes `]`, and the one after `,`), so the arms set `d` and the loop's `match d` took them apart again; every switch also had a path on the kind | yes, since step 15: the first value's switch takes `]`, then sends every other byte to the one value switch; the kind path only where a call returns to switch on its token again |
| `case ','`, `case ']'` in the continue states | each punctuation byte straight to its place, every other byte refused | yes |
| atoms: `str4ncmp(src, "true")`, then the byte after not structural or white space; `visit_true_atom` writes its own tape word | the same test, a word load; the arm writes its leaf with its kind | yes, since step 21 |
| `parse_number`: the digits read once, validated as they are read | `run_0_check_0`: one read of 16 bytes, the end and the check from it, then on past the check | yes, since step 14 (a number past 15 bytes: as before) |
| `parse_string`: `copy_and_find`, 32 bytes a step, then one branch on `has_quote_first`; the stop handled in the same pass | `part_stop`, 32 bytes a step, the closer's and the escape's masks, the stop's kind from them; content, then its stop in the same pass | yes, since step 23 |
| an escape: `escape_map[c]`, the `\u` work only after a `u` | `CODES2[c]`, then `\u` to its two checks and the eight others, one range, past them | yes, since step 16 |
| the closing quote ends the string | the closer's arm goes on at once | yes, since step 16 |
| the tape: a word a value, written once, in place | format 16 (decision 0434): each node passed to the writer's ops at its word, as it is made (`open16`, `leaf16`, `w_close`), each cell at its final place; the ops' `finish` moves the last block down and lays the tail | since step 25 |
| the stack: open containers only (`is_array[depth]`) | format 16 run: a slot an open node, its call number; no row a node | since step 27 |
| | where a word edits a node it made (`.rr`, the ladders): the rows as the run's window, each node to the ops at a commit point, one behind | since step 28 |

What follows, in order, each change made in `rust_stream` or in the compiler, and judged on
the Rust it writes:
1. Done in step 14.
2. Done in step 15.
3. Done in step 16.
4. Done in step 17.
5. Done in step 18.
6. Done in step 19.

The next six, from reading the Rust after step 19 (each a step, its assertion and a mutation):
1. Done in step 20.
2. Done in step 21.
3. Done in step 22.
4. Done in step 23.
5. Done in step 23.
6. The tape written once: each node's code as it is made, done in step 24. The columns where the
   image has them: format 16 (decision 0434), written by the writer's ops at each node word, done
   in step 25.

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
