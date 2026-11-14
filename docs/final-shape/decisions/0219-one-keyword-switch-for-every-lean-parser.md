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
  -> one 16-byte load (two u64 lanes), masked to the length by a table
     (a word past 16 bytes: its last 16 bytes too, by an overlapping load)
  -> [a case-insensitive list] OR 0x20 into each byte
  -> one multiply hash of the lanes and the length
  -> CHD perfect hash: bucket = top bits; slot = ((h >> 32) ^ disp[bucket] * K) * K2 >> shift
  -> compare the slot's lanes and length: equal -> its code; else 0
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

`Words<SLOTS, BUCKETS, FOLD_CASE>`:

- `const fn build(words: &'static [&'static [u8]]) -> Self`: `words[0]` is `b""` (code 0, none);
  a word's code is its index. Words are at most 64 bytes; at most 4,096 words.
- `fn find(&self, src, s, e) -> u32`, inline: the probe above. A load past the text's end is a short
  copy only within 16 bytes of the end.
- `slots_for(n)`: a power of two of at least `2n`; `buckets_for(n)`: a power of two of about `n/4`.
- `FOLD_CASE`: ASCII case folded by OR `0x20` (exact on word bytes: letters, digits, `_`, `$`).
  SQL and PHP fold; the others do not.

The build groups words by bucket by a counting sort (a `const fn` has no sort), then places the
buckets largest first, trying displacements until every word of the bucket lands in an empty slot.

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

**Cost** (the `*_parse_costs` tests, profiling build, the lean tape, instructions retired a node),
against the number each decision recorded:

| parser | files | recorded | now |
|---|---|---|---|
| SQL (0214) | 117 | 990 | 886 |
| ECMAScript, JavaScript (0178) | 1,235 | 757 | 597 |
| Python (0163) | 1,000 | 604 | 524 |
| Java (0195) | 926 | 576 | 584 |
| Rust (0193) | 1,971 | 487 | 493 |
| C# (0196) | 795 | 538 | 537 |
| PHP (0198) | 803 | 977 | 983 |

The recorded numbers are each decision's, measured when it was written; later commits changed the
parsers too, so this is not an A/B of the switch alone. For SQL it is: the crate's profiling loop
over the same files went from 22.2 M to 19.8 M instructions a pass, and 5.8 M to 4.2 M cycles. Where
a parser had a hash probe already (Java, Rust, C#, PHP), the cost is within 1.5% of the recorded
one: the change there is one shared, generic lookup in place of five copies, not speed. Cycles were
measured under a load average near 100 and are not compared.

## Open

- The SQL parser still asks the state's questions by call (`is`, `kw`, `words`); with the code in
  each token those are table reads, and its remaining cost is the automata walk and the writer.
