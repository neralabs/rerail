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
