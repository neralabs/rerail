# 0219: One keyword switch for every lean parser: a 16-byte load, one hash, one probe

Status: built (user, 2026-11-14: "nearly all parsers need a way to switch on a large number of
keywords … I want a generic one or at least a performant one, that takes advantages of SIMD tricks
like SIMDJSON and can be integrated into all of the parsers reasonably").

## Intro

A lean parser's lexer finds a word (stage 1 gives its end: the lowest set bit of the word-stop
mask), then asks which keyword it is. Each parser answered that its own way: ECMAScript, Java,
Rust, C# and PHP hashed the word's first bytes into a bucket of candidates and compared them;
Python searched by length; SQL bucketed its 517 words by length and first byte and cached the last
two answers.

This decision replaces all of them with one switch, `rerail_simd8::words::Words`, built at compile
time from each parser's word list. A word's code is found in one probe and one compare, with no
loop over the word's bytes and no loop over candidates.

```
the word src[s..e] (its end from stage 1)
  -> a length outside the list's shortest..longest: 0, before any load
  -> one 8-byte load, masked to the length by one shift
  -> [a case-insensitive list] OR 0x20 into each byte
  -> the slot, by one of two shapes chosen at compile time from the list:
       direct: ((lo ^ len * K) * mult) >> (64 - bits)      (mult searched at compile time)
       CHD:    h = mix(first 16 bytes, last 16, len); slot = f(h, disp[bucket(h)])
  -> one load of the slot (its key and code, 16 bytes), one compare of lane and length
  -> past 8 bytes: the next lane (his[code]); past 16: the rest, a cold compare
```

## Prior work consulted

- **simdjson** (Langdale and Lemire): its atom check loads a word's bytes as one integer and
  compares it to `true`, `false`, `null` at once, its end from the structural bitmask. The switch
  widens that compare to any list through a perfect hash.
- **CHD, hash and displace** (Belazzougui, Botelho and Dietzfelbinger; the `phf` crate builds the
  same): a displacement a bucket, placed largest bucket first. The table is built by a `const fn`,
  so there is no build script and no run-time setup; a list that cannot be placed (two words equal
  after folding, too few slots) fails the compile.
- **0163 through 0214 (the lean parsers).** **Agrees:** each parser keeps its own word list and its
  own codes; only the lookup moves. The switch returns the word's index in the list, so each
  parser's `match` and tables are unchanged.
- **gperf / re2c keyword matchers.** A generated `switch` on length and bytes compares per
  candidate; the switch here needs no generator and costs the same for 60 words or 4,000.

## Decision

### `rerail_simd8::words`

A parser declares its switch with one line, the sizes computed from the list:

```rust
// java_lean/src/lex.rs: the list is the generated keyword texts, word 0 the empty word
rerail_simd8::keyword_switch!(static WORDS = w::TEXT, fold: false);
// a word's code: its index in w::TEXT, 0 for none
let code = WORDS.find(src, s, e);
```

`Words<SLOTS, BUCKETS, N, FOLD_CASE>`:

- `const fn build(words: &'static [&'static [u8]]) -> Self`: `words[0]` is `b""` (code 0, none);
  a word's code is its index. Words are at most 64 bytes; at most 4,096 words.
- `fn find(&self, src, s, e) -> u32`, inline: the probe above. A load within 8 bytes of the text's
  end is a short copy, out of line and cold, so the probe keeps few registers.
- A slot holds its word's first lane, length and code (16 bytes), so the probe has no load that
  waits on another: 1,024 slots are 16 KB.
- **The shape** (`slots_for`, `buckets_for`, `BUCKETS == 0` for direct): direct where the list has
  at most 256 words, their first lanes and lengths distinct, and a multiplier of the first 4,096
  tried places them in 1,024, 2,048 or 4,096 slots; this is the shape the lexers had each built by
  hand, now searched for by the compiler. Otherwise hash and displace (CHD), slots a power of two
  of at least `2n`, buckets about `n/4`: SQL's 517 words, and lists whose words share a first lane
  and length (`current_date`, `current_time`).
- `FOLD_CASE`: ASCII case folded by OR `0x20` (exact on word bytes: letters, digits, `_`, `$`).
  SQL and PHP fold; the others do not.

The CHD build groups words by bucket by a counting sort (a `const fn` has no sort), then places the
buckets largest first, trying displacements until every word of the bucket lands in an empty slot.
A list that cannot be placed fails the compile.

### Its users

| parser | words | fold | before |
|---|---|---|---|
| ECMAScript (`ecma_lean`) | its keyword list | no | first-bytes hash, candidates compared |
| Java (`java_lean`) | | no | the same |
| Rust (`rust_lean`) | | no | the same |
| C# (`csharp_lean`) | | no | the same |
| PHP (`php_lean`) | | yes | the same, folded |
| Python (`python_lean`) | keywords and soft keywords | no | search by length |
| SQL (`sql_lean`) | 517 keyword words (distinct texts) | yes | bucket by length and first byte, a two-word cache |

SQL computes a word's code once, in the lexer (`Tok.w`), and reads its keywords from a table
(`WORD_AT[code]`, a run of `WORDS`): every keyword question after that is a table read, with no
lookup. `gen_kinds.py` writes the word list and the table.

## Results

Every crate's unit tests pass, and every oracle is unchanged, none differing:

| oracle | same / differ / refused |
|---|---|
| ECMAScript, 1,236 files | 1,235 / 0 / 1 |
| Java | 926 / 0 / 0 |
| Rust, 2,022 files | 1,971 / 0 / 0 |
| C#, 881 files | 795 / 0 / 25 |
| PHP, 805 files | 803 / 0 / 0 |
| SQL, 224 clean files | 117 / 0 / 107 |

The engine's tape tests (`*_tapes_from_the_lean_parser_are_tree_sitters`,
`the_product_reads_*_with_the_lean_parser`) pass, 22 of 22.

**Cost: an A/B of the switch alone.** Two builds of a bench (`tmp/ab-words`), one against the
crates before this decision, one against them with it; the same files, instructions retired, the
best of five passes (deterministic to about 0.2%; cycles were measured under a load average near
100 and are not compared).

The probe alone (every `[A-Za-z_][A-Za-z0-9_]*` run of the corpus through the lexer's `word()`),
instructions a word:

| parser | words | before | the switch |
|---|---|---|---|
| ECMAScript | 1,183,079 | 57.80 | 57.84 |
| Java | 845,878 | 59.23 | 59.09 |
| Rust | 2,714,645 | 58.81 | 59.31 |
| C# | 1,405,391 | 63.93 | 62.79 |
| PHP | 310,840 | 64.80 | 61.44 |

The whole lean tape (`tape()`), instructions a file's tape key (the same files on both sides):

| parser | files | before | the switch | change |
|---|---|---|---|---|
| ECMAScript | 1,235 | 11,687.5 | 11,685.1 | -0.0% |
| Java | 926 | 6,845.2 | 6,786.9 | -0.9% |
| Rust | 1,971 | 8,568.9 | 8,545.4 | -0.3% |
| C# | 803 | 8,811.6 | 8,767.1 | -0.5% |
| PHP | 802 | 10,462.7 | 10,427.7 | -0.3% |
| Python | 1,000 | 12,221.0 | 12,166.0 | -0.5% |

For SQL the gain is larger, as its lookup was a search: the crate's profiling loop over the same
files went from 22.2 M to 19.8 M instructions a pass.

The first build of the switch kept slots as two-byte codes and the keys in a second table, by code;
it cost 3 to 7 instructions a word more than the hand-built tables (a second load waiting on the
first, and the short load's copy inlined, saving two more register pairs). Holding the key in the
slot and moving the short load out of line gave the numbers above. Where a parser had a hash probe
already (ECMAScript, Java, Rust, C#, PHP) the gain is small: the change there is one shared, generic
lookup in place of five hand-built copies, at the same cost.

## Open

- The SQL parser still asks the state's questions by call (`is`, `kw`, `words`); with the code in
  each token those are table reads, and its remaining cost is the automata walk and the writer.
