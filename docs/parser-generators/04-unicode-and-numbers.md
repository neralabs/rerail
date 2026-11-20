# Parser generators: Unicode and numbers

Status: design, 2026-11-06. It follows `03-bitlex.md`, the lexer language, and adds its `ucode`
construct.

## Intro

Daniel Lemire and his co-authors have shown that two jobs usually written as byte-at-a-time loops
can be done with SIMD at gigabytes per second:

- **checking and converting UTF-8** (the simdutf library);
- **parsing numbers**: decimal digits to integers, and decimal text to the nearest double.

This document looks at how those methods work, and at what a tree-sitter lexer actually needs from
them. The answer is less than one might expect:

- **Tree-sitter positions are byte offsets.** Nothing is converted to UTF-16 or counted in
  characters, so a tape needs no transcoding.
- **UTF-8 keeps ASCII facts exact.** Every byte of a multi-byte character is 0x80 or above. A
  class such as "is `"`" or "is a digit" can never match inside a multi-byte character, so every
  mask in `03-bitlex.md` is already correct on UTF-8 input.
- **Code points matter in few places.** For Python they are identifiers and three whitespace
  characters, and only outside strings and comments. For JSON there are none.
- **The tape stores no numeric values.** A number token needs its extent, its kind (`integer` or
  `float`) and a check that tree-sitter would accept it.

So Unicode becomes a small layer under the byte masks. It decodes a character only where a rule
asks about one, and its cost grows with the non-ASCII characters in code, which are close to zero.
Numbers need validation, not conversion, and most of them are validated by a single mask.

## 1. Lemire's methods

### 1.1 UTF-8 validation

Keiser and Lemire, "Validating UTF-8 in less than one instruction per byte" (Software: Practice
and Experience, 2021). It is the validator in simdjson and simdutf.

Every UTF-8 error shows in the first two bytes of a sequence, or in the count of continuation
bytes that follow a lead byte. The method does three 16-entry table lookups per byte:

| lookup | indexed by | |
|---|---|---|
| 1 | the high nibble of the previous byte | |
| 2 | the low nibble of the previous byte | |
| 3 | the high nibble of this byte | |

Each lookup returns 8 error bits: TOO_SHORT, TOO_LONG, OVERLONG_3, TOO_LARGE, SURROGATE,
OVERLONG_2, TOO_LARGE_1000 (sharing its bit with OVERLONG_4) and TWO_CONTS. The AND of the three
results is nonzero exactly where the byte pair is wrong.

- **Longer sequences:** one more test uses the bytes two and three back. It marks where a
  continuation byte must follow a 3- or 4-byte lead, and XORs that with the TWO_CONTS bit.
- **Between blocks:** the carry is the previous block's last three bytes.
- **At the end:** a lead byte in the last three positions that wants more bytes than remain is an
  error.
- **ASCII:** simdutf skips a block whose bytes are all below 0x80. That is a branch per block.

It runs at well over 10 GB/s on recent CPUs. It needs only per-byte lookups and a carry of three
bytes, so it fits the block model of `06-block-parallel.md` as an input-only fact.

### 1.2 Transcoding

Lemire and Keiser, "Transcoding billions of Unicode characters per second with SIMD instructions"
(Software: Practice and Experience, 2022). UTF-8 is turned into UTF-16 or UTF-32:

- **Character ends:** a bitmask marks the last byte of each character.
- **Shuffles from a table:** that mask, 12 bits at a time, indexes a table of shuffle patterns.
  Each pattern moves the bytes of up to 6 characters into fixed lanes.
- **Code points:** shifts and masks assemble them, with no branch per character.
- **Counting characters** is a popcount of the bytes that are not continuation bytes.

Rerail does not transcode. The idea that carries over is to decode only at chosen positions,
gathered by a mask, with the position count known from a popcount.

### 1.3 Integers

The SWAR ("SIMD within a register") conversion used in simdjson, after Muła, Lemire and others.
Eight ASCII digits in one 64-bit word become their value in three multiply-and-shift steps:

```c
// v holds 8 ASCII digits, the first digit in the low byte.
v = (v & 0x0F0F0F0F0F0F0F0F) * 2561 >> 8;                // pairs:   d0*10 + d1
v = (v & 0x00FF00FF00FF00FF) * 6553601 >> 16;            // quads:   p0*100 + p1
v = (v & 0x0000FFFF0000FFFF) * 42949672960001 >> 32;     // eight:   q0*10000 + q1
```

`is_made_of_eight_digits_fast` checks that a word holds eight digits with two adds and an AND.
Wojciech Muła's SIMD parsing of integer sequences goes further. A delimiter mask indexes a table
of shuffle patterns, and multiply-add instructions then convert several numbers at once.

### 1.4 Floating point

Lemire, "Number parsing at a gigabyte per second" (Software: Practice and Experience, 2021). It
is the Eisel–Lemire algorithm, shipped in fast_float and, since Rust 1.55, in Rust's own float
parsing:

- the digits are read as a 64-bit integer, using the eight-digit step above;
- the decimal exponent selects a precomputed 128-bit power of five;
- one 64×64→128-bit multiply gives the result's significand;
- a slower exact path handles the rare inputs that sit too close to a halfway point.

simdjson parses each number this way in its second stage, one number at a time, with branches.

## 2. What a tree-sitter lexer needs

| need | JSON | Python |
|---|---|---|
| valid UTF-8 | given: Rerail reads files as `&str` | given |
| byte offsets only | yes | yes |
| code points outside strings | none: any byte ≥ 0x80 there is an error | identifiers, and U+FEFF, U+2060, U+200B |
| Unicode whitespace | none: `\s` is ASCII in tree-sitter | none beyond the three above |
| numeric values | not stored | not stored |
| number validation | `-?(0\|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?` | integer and float patterns |

These were checked with the grammars' own parsers, not inferred from the grammar text:

- **`\s` is ASCII in tree-sitter.** It is space, `\t`, `\n`, `\v`, `\f` and `\r`. U+00A0,
  U+0085, U+1680, U+2000, U+2028, U+3000 and U+FEFF are errors in tree-sitter-json. In
  tree-sitter-python the same holds, except for the three characters its extras list by name.
- **A byte-order mark is just U+FEFF** in tree-sitter-python. `module` starts after it.
- **Identifiers follow the tables in `parser.c`.** tree-sitter-python's identifier is
  `[_\p{XID_Start}][_\p{XID_Continue}]*`, generated into `sym_identifier_character_set_1` (682
  ranges, 141,194 code points) and `_2` (796 ranges, 144,459 code points). Its Unicode version is
  whatever the tree-sitter CLI used to generate it. Reading these arrays from `parser.c` keeps
  that version, instead of using a newer one.
  - `x·y`, `x` + U+0301 and `ℕ` are identifiers.
  - `x²` is the identifier `x`, then an error at `²`.
  - `＿x` (U+FF3F) and `゛` (U+309B) are errors at their first character.

How often each case occurs, on files tree-sitter parses without error:

| | data-layer Python | CPython 3.15 stdlib | data-layer JSON |
|---|---:|---:|---:|
| bytes | 167.1 M | 37.8 M | 567.7 M |
| non-ASCII bytes | 0.050% | 0.043% | 6 bytes |
| files with any | 4.0% | 9.6% | 2 files |
| where they are | 90% comments, 10% strings | 90% strings, 8% comments | strings |
| non-ASCII characters in code | 0 | 132 (3.5 per MB) | 0 |

## 3. Composing Unicode with the masks

### 3.1 Three rules

1. **ASCII classes need nothing.** A class made of ASCII bytes is exact on UTF-8. Strings,
   comments, brackets, quotes and escapes are found with no Unicode work at all.
2. **A Unicode class is a byte mask.** It is set on every byte of each character in the class, so
   `fill`, `span`, `prev` and the rest apply to it unchanged. A word that contains `é` is two
   bytes of `xid_continue`, and `fill(wstart, ~idc)` runs straight through them.
3. **Decoding is gated by a mask built from ASCII facts.** `ucode over M` decodes only the
   characters whose lead byte is in `M`. For Python, `M` is `code & high`: bytes of 0x80 or above
   that are outside strings and comments. `code` comes from the `strings` automaton, which reads
   only ASCII events, so there is no cycle.

### 3.2 The construct

From `lexers/python.bitlex` §7:

```text
ucode over code & high {
  class xid_start    = table "parser.c" sym_identifier_character_set_1
  class xid_continue = table "parser.c" sym_identifier_character_set_2
  class uspace       = U+FEFF U+2060 U+200B
}

mask idc       = code & (idcont | xid_continue)       # ASCII letters, digits, '_', or XID_Continue
mask wstart    = code & (idstart | xid_start) & ~prev(idc) & ~number
mask word      = fill(wstart, ~idc)
skip code & uspace

error "a non-ASCII character tree-sitter rejects in code"
      when code & high & ~xid_continue & ~uspace
error "an identifier that starts with a non-start character"
      when code & high & ~prev(idc) & ~xid_start & ~uspace
```

Each class may be:

- a `table` read from a grammar's generated `parser.c`;
- a list of code points or ranges (`U+FEFF`, `U+0300..U+036F`);
- a Unicode property (`\p{XID_Start}`), when a grammar's pattern names one and no generated table
  exists. The generator then fixes the Unicode version.

A character class in a `check` regex may name a `ucode` class. The regex is then compiled over a
class alphabet rather than over bytes. A UTF-8 DFA for XID_Continue alone has far more than 64
states. Over the class alphabet it is one symbol.

### 3.3 Lowering

Per 64-byte block, with `M` the `over` mask:

1. **Leads:** `lead = M & ~cont`, where `cont` is the class 0x80..0xBF. If `lead` is empty, as it
   is in more than 99.99% of Python blocks, the next steps have no work. On the CPU, the
   flattening loop of step 2 then runs zero iterations. On Metal, the gathered list is empty.
2. **Gather:** the positions of the set bits. On the CPU this is simdjson's flattening loop
   (count trailing zeros, clear the lowest bit, always 8 per round). On Metal it is a prefix sum
   of the mask and a scatter.
3. **Decode,** per position, with no branch on length:
   - the length comes from a 16-entry table on the lead byte's high nibble;
   - four bytes are loaded, and each length's masks and shifts are applied;
   - the result for the actual length is selected.
4. **Look up:**
   - `stage1[cp >> 8]` gives a page number;
   - the page holds 2 bits per code point, for `xid_start` and `xid_continue`;
   - a third bit holds `uspace` (three code points).
5. **Scatter back:** each class bit goes to its lead position, then over the character's other
   bytes with `span(lead, len)`.

The table, built from tree-sitter-python's `parser.c`:

| page size | stage 1 | stage 2 | total |
|---|---|---|---|
| 64 code points | 17,408 entries, u16 | 314 pages × 16 B | 39.8 KB |
| 128 | 8,704 entries, u8 | 203 pages × 32 B | 15.2 KB |
| **256** | 4,352 entries, u8 | 131 pages × 64 B | **12.7 KB** |

- **Most of stage 1 is unused.** Every XID_Start code point is below U+32400. Above that, only
  the 240 variation selectors U+E0100..U+E01EF are XID_Continue.
- **Shrinking it:** with stage 1 cut at U+40000 and one compare for plane 14, the table is about
  9.4 KB. That fits in L1, and in Metal threadgroup memory, which is 32 KB.
- **Start ⊂ Continue:** every XID_Start code point is also XID_Continue, so the bits could be
  packed further. That is not needed at this size.

### 3.4 Cost

The work in steps 2–5 is proportional to the number of decoded characters. In code these are 0
per MB on the data-layer Python and 3.5 per MB on the CPython library. Step 1 is one AND and a
test per block. Non-ASCII text in strings and comments costs nothing, since rule 1 already
handles it.

A language whose code holds many non-ASCII characters would pay one decode and two dependent
loads per character. Two examples:

- **JavaScript:** U+2028 and U+2029 end lines, and lines decide automatic semicolon insertion.
  They are events of an automaton, and also code points.
- **Identifiers in Chinese, Japanese and similar scripts:** every character of the identifier is
  decoded.

Even then this is per character, not per byte, and it has no branches.

### 3.5 Validation, if it is ever needed

Rerail reads source as `&str`, so on the CPU path the bytes are already valid UTF-8. If the device
ever reads raw bytes from a slab, Keiser and Lemire's validator (§1.1) slots in as one more
input-only fact:

- its three lookups per byte are the same kind of operation as the class lookups;
- its carry is the last three bytes of the previous block, a halo in `06-block-parallel.md`;
- an invalid file falls back, as a file with a syntax error does.

### 3.6 Block edges

A character is at most 4 bytes, so a block needs a 3-byte halo on each side to classify its own
bytes. `lead` and `cont` are local. A character that starts in one block and ends in the next is
decoded by the block that holds its lead byte. The class bits for its trailing bytes reach the
next block through the halo, as `span` does for any token.

## 4. Numbers

### 4.1 What is checked

A number token needs three things:

- **its extent,** from masks (`lexers/python.bitlex` §7, `lexers/json.bitlex`);
- **its kind:** Python's float is any number with a `.` or a decimal exponent, carried to the
  token's first byte by `back_fill`;
- **a check** that tree-sitter's pattern accepts the whole token (§4.2).

| | data-layer Python | CPython | data-layer JSON |
|---|---:|---:|---:|
| number tokens per byte | 0.0010 | 0.0046 | 0.0052 |
| bytes in number tokens | 0.17% | 0.95% | 1.94% |
| tokens that are all digits | 89.4% | 83.8% | 94.8% |

### 4.2 Validation as masks, with no automaton

simdjson and fast_float validate a number as they convert it, one number at a time, with a
branch per character class. Lemire's speed comes from removing the per-digit loop: eight digits
are tested and converted at once in a word. Bitlex goes one step further. It tests every number
in a 64-byte block at once, with the same word-wide logic, and needs no automaton at all.

**Why that works.** Each number pattern is a fixed sequence of parts. JSON's is

```text
-?  (0 | [1-9][0-9]*)  (\.[0-9]+)?  ([eE][+-]?[0-9]+)?
```

A token matches such a pattern exactly when three kinds of rule hold:

| kind of rule | JSON example | mask form |
|---|---|---|
| which bytes are allowed in which part | the exponent holds digits and a sign | a class, ANDed with a part's region |
| what may stand next to what | a '.' has a digit on each side | `prev` and `next`, at most 2 bytes away |
| how often and in what order the separators appear | one '.', one 'e', the '.' first | "a '.' came earlier in this token": `fill(prev(dot), ~number)` |

The regions are fills that start after each separator and stop at the token's end, so each costs
one subtraction and a one-bit carry. In a regular-expression sense, the patterns are *locally
testable* once the regions are known. Nothing about them needs the memory an automaton keeps.

**The cost** is fixed per block: about 25 mask operations per 64 bytes for JSON and about 40 for
Python. It does not grow with the number of numbers, and no token list is gathered. The rules are
in `lexers/json.bitlex` (8 rules) and `lexers/python.bitlex` (7 rules, covering integers in
four bases, floats and suffixes). Both were checked against tree-sitter's parsers on these
cases:

- **valid:** `1_`, `007`, `1_.5`, `1.e5`, `0x_1f`, `1_j` and `1.5_`;
- **errors:** `0b12`, `1__2` and `1._5`.

**When this does not apply.** A pattern with repetition of a group that holds a separator, or
with nesting, is not a fixed sequence of parts. `(ab|cd)*` and balanced brackets are examples.
Such a pattern still needs the automaton of `03-bitlex.md` §3.3. tree-sitter's token patterns
for numbers, identifiers, escapes and comments are all fixed sequences. The minimal DFAs would
have been small too (11, 16 and 13 states), but the masks need no gather and no table walk.

### 4.3 Values, later

Rules may one day compare literal values, such as "a timeout below 1000". A value is then a later
fact over the `integer`, `float` and `number` tokens, not a tape column:

- integers: §1.3's eight-digit step, with `_` removed by a compress first in Python;
- floats: Eisel–Lemire, which Rust's `str::parse::<f64>` already uses.

Both run over the compacted list of number tokens, so they cost nothing for files that do not
ask.

## 5. Summary

- **Unicode composes as a gated character layer.** ASCII masks are untouched, Unicode classes are
  byte masks, and decoding happens only where an ASCII-derived mask says code holds a non-ASCII
  byte.
- **Fidelity comes from tree-sitter's own tables** in `parser.c`, not from a Unicode library.
- **Lemire's validator and transcoder** are not needed on Rerail's path. Their gather-and-decode
  pattern is what `ucode` lowers to.
- **Numbers need validation only, and validation is masks.** Each pattern is a fixed sequence of
  parts, so it becomes a few class, neighbour and order rules over every byte of a block, with no
  automaton. Values are an optional later fact.
