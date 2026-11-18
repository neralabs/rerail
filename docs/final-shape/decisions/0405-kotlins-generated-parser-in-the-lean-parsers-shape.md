# 0405: Kotlin's generated parser, in the lean parser's shape

Status: decided (the user, 2026-11-18: "since we already have a fast Kotlin parser in Rust, and
the bitlex program can generate Rust, I want you to work on getting the generated Rust code
structurally close to the lean parser. Start with the broad structure"). Follows 0177 (tier 1 on
the CPU is a token loop), which brought `.rr` to that shape; this record does the same for
Kotlin, whose grammar uses what `.rr`'s does not.
Numbered 0403 where it was written.

## Intro

**The problem.** Kotlin has two parsers that write the same tapes. The lean parser
(`rerail-kotlin-lean`) is hand-written Rust. The Bitlex parser is compiled from the grammar
(`kotlin_parser/bitlex_ladder`), and its code can be written out as Rust and compiled by rustc
(`rerail-kotlin-aot`). Both go through the same compiler, so any difference in what the machine
runs comes from a difference in the Rust source. This record compares the two sources at the
broadest level, the passes and what each keeps, and sets the order in which the generated one is
brought to the lean one's shape.

## The broad structure, side by side

The lean parser, for one file (`kotlin_lean::tape`):

```text
stage 1   one pass over the 64-byte blocks: 5 masks a block
          (a name's bytes, blanks, line ends, a string's stops, a comment's stops)
parse     recursive descent, a function a rule (parse.rs, parse/*.rs: 147 functions)
  lexer   pulled by the parser, a batch at a time (Blocks::lex_blocks):
          a block's token starts from 2 of the masks, each token read by its first byte;
          a string or a comment by its own reader, which moves the cursor
  peek    the next token, or the one after (Parser::peek, peek_at)
  asi     the automatic semicolon: asked by the parser at a token's end, where its state
          takes one, from the bytes after it (Lexer::asi); the parser's state (AsiCtx: a
          class header, a `by`) is an argument. The swallowed block comment is its answer too.
tape      written as the rules return (Tw)
```

The generated parser (`rerail-kotlin-aot`, from the step-35 grammar):

```text
tier 1    7 passes over every block of the file:
            forward  1,743 instructions a block   419 words stored a block
            back     21 back_fills
            forward  1,168 instructions           87 stored
            back     11 back_fills
            forward    184 instructions           15 stored
            back      2 back_fills
            forward    254 instructions, and the token loop writing the records
          555 words a block kept between passes, for 8 bytes of text each
tier 2    a function a rule (110 `r…` functions, 155 in all), reading the records in order
```

Tier 2 is already in the lean shape: rules as functions, tokens read in order, each rule a
`match` on the token's kind. Tier 1 is not. The lean lexer has no backward pass and keeps 5 words
a block; tier 1 makes the grammar's 116 outputs as masks over every byte, in passes, through
slots.

## Why tier 1 has passes

A pass ends at each `back_fill`: the value at a byte is a value at a later byte, so the whole
file has to be made before the fill can run backward over it. Kotlin's grammar has 36 of them,
nested three deep. They are of three kinds:

| Kind | What it asks | Count | The lean parser's form |
|---|---|---|---|
| The next token | `back_fill(X, solid)`, `back_fill(X, ~ws)`: X holds at the next byte a token starts at (`goes`, `->`, `::`, `}`, `(`) | 21 | `peek`: the next token, already lexed |
| Inside a token | `back_fill(X, ~w)`, `w_last`, `~nb`: X holds later in the same word or number | 5 | the token's reader, which reads those bytes as it goes |
| Context over tokens | a lambda's parameters (`lam_last`), a type's run (`~type_byte`, `~(w \| dot \| < \| >)`), `~decl_run`, `~sw_blk`, a class header to its `constructor`, a raw string's stop, an annotation's end | 10 | the parser's state: what it is parsing when it gets there (`AsiCtx`, the rule it is in) |

The forward `fill`s (a class header, a declaration's run, a region) are the same third kind, run
forward: the lean parser holds them as its state, not as a mask a byte.

## The decision

Tier 1 on the CPU becomes the lean lexer's one pass, a stage at a time, in this order. Each stage
removes a kind of lookahead from the byte masks and gives it the lean parser's form; each is a
construct of the program with two lowerings, as 0177 requires (Metal keeps the masks).

1. **The next token at the token.** A `back_fill(X, G)` whose run `G` is the bytes between two
   tokens (blanks, line ends, extras) is "the next token starts with X". Where it is read at a
   token or a gap mark (lifted to the token after the gap), it is a test of the next record: the
   token loop, or tier 2 where it asks, reads it as `peek` does. Its pass and slots go.
2. **Inside a token, by its reader.** A fill within a word or a number is the token's own rule:
   tested where the token's bytes are read (0177's token tests, `native_cpu_bytes::needs`).
3. **Context as state.** The fills over tokens (a class header, a declaration's run, the swallowed
   comment, a template's code) become state the token loop or tier 2 carries, as the lean parser
   carries its rule and `AsiCtx`. The marks the scanner's semicolon makes (`ends_line` and its
   kin) are asked by tier 2 where its state takes one, as `Lexer::asi` is (0177's lazy marks).
4. **The masks that are left** are the lean lexer's: the starts, the runs, the line ends, a
   string's and a comment's stops.

The gate of each stage is structural: the passes and the slots it removes, and no instruction
in the CPU body for what it moved. The end state: one forward pass, no slots, the body's masks
the lean lexer's five. Tapes stay byte-identical at every step (`aot_tapes_are_the_jits`, the
kotlin-parser corpus tests against the lean tapes).

`rerail-kotlin-aot`'s `structure` test prints the shape (passes, instructions, slots) for each
step; `jit_against_aot` measures it at the end of a stage, as information.

## Where it stands

| | Passes | Slots a block | CPU body's instructions |
|---|---|---|---|
| step 35, before this work | 7 (4 forward, 3 back) | 555 | 1,743 + 1,168 + 184 + 254 |
| step 36, no back fills (0406) | 1 (forward) | 0 | 2,462 |

Measured as information (`jit_against_aot`, the 429 corpus files the grammar takes, load 60–80):
tier 1 ahead of time 289 cycles a node; the lean parser's whole parse 115.

Step 36 (load 27–33): tier 1 ahead of time 524M instructions, 146M cycles, 43.5 ms CPU (956
instructions and 267 cycles a node); tier 2 ahead of time 376M instructions, 93M cycles,
28.7 ms; tiers 1 and 2 ahead of time 436 cycles a node, the JIT's 563; the lean parser's whole
parse 296M instructions, 64.5M cycles, 19.2 ms (118 cycles a node).

## Stages 3 and 4: where the operations are (2026-11-18)

After Q222 to Q225 (step 36, the corpus 431 / 60 / 0), tier 1 is one forward pass of 2,460
operations a block. `kotlin_parser`'s `tier1_cones` counts them by the statement that made each
(`Compiled::origin`) and by the outputs that need them:

| What | Operations |
|---|---|
| Two token rows that can start at one byte, each pair tested at every byte for the fallback | 600 (355 pairs) |
| The other escapes and errors (the fallback's other outputs) | about 400 |
| The marks' masks (`vis_w`, `head_stop`, `jat`, `two_start`, `hdr_line`, `ctor_w`, …) | about 900 |
| The regions, the token rows' starts and lasts | about 500 |

The lean lexer asks none of the first kind: its state picks one reader, so two kinds never start
at one byte. Here the compiler asks it of each pair whose first bytes meet, and keeps the test
where it cannot prove the pair apart. It asked the plain search only; the fixed points' prover
(`Induction`, as the other row questions use) proves 210 of the 355 apart: a row in a region
against one outside it. Tier 1: 2,014 operations a block, from 2,460; the corpus and the
generated Rust's tapes unchanged.

The 145 pairs left are two string regions' rows (a string's content and a raw string's, a
template tail's, a character's). The walk makes a byte one region's at most; the prover does not
know it. Next: that fact given to the prover, then the marks and escapes at the token
(stages 3 and 4).
