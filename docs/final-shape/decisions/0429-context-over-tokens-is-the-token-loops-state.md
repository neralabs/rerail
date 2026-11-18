# 0429: Context over tokens is the token loop's state

Status: decided (2026-11-18), 0405's stage 3 made concrete. Follows 0405, 0177 and 0407.

## Intro

Tier 1 finds where each token starts and ends, and the grammar's marks, by testing every byte of
a file with bitmasks: 64 bytes at once, a bit a byte. That suits the things a lexer asks of
bytes: where a word or a string ends. It does not suit the things a parser asks of tokens: "is
this word `private`?", "are we inside a class's header?". Kotlin's grammar asks many of those.
Written as masks, `private` is seven byte compares and six shifts at each of the 64 bytes. "In a
class's header" is a fill, a run of bits from a `class` word to the token that ends the header.

The lean parser asks the same things once a token: it compares the token's word, and carries
"in a header" as its state. Kotlin's tier 1 is 1,500 operations a 64-byte block on the CPU, about
9,200 machine instructions with the spills: 144 a byte, against about 5 for the lean lexer.
About 420 of the operations are keyword tests (`vis_w`, `head_stop`, `jat`, `ctor_w`, …), and
the fills and marks built on them add more (`tier1_cones`).

## The decision

A mask that is a question about tokens is answered once a token, in the token loop, on the CPU.
Metal keeps the masks (0177: one construct, two lowerings).

1. **Token-level masks.** A mask is *token-level* when, in the program, its value at a byte
   matters only at grammar token starts, and there it is a function of the tokens:
   - a word test: `kw_start & text(W) & ~next(w, |W|)`, a word token whose text is W, is the
     token's record's word, compared (the lean parser's `is_id`). The compiler proves the term
     holds exactly at those tokens (`never`, as 0407's mark words);
   - a token's kind, or its first byte's class;
   - `&`, `|` and `~` of token-level masks, and `prev(…)` of one taken at the token before;
   - a fill whose start and stop are token-level (`fill(A, S)`): one bit of state the loop
     carries over tokens, set at A and cleared after S, as `AsiCtx` is.

   The compiler finds them from the outputs back: each mark and output the program reads at
   token starts only (a mark, a gap mark lifted to the next token, a token test) pulls its
   operands in. A mask read at any other byte keeps its bytewise form, and so do its operands.
2. **The token loop's step.** The token-level masks become a small program the token loop runs
   at each grammar token, after it has the token's kind and word. The program has straight-line
   word compares and bit logic, and a few state bits for the fills. Its results are the marks'
   bits in the token's code, where `late_marks` would set them now.
3. **Gap marks.** A gap mark read at the token after its gap (`Records::gaps`) reads the state
   as it was after the token before, as the lean parser's `asi` reads its context.
4. **Proved, not assumed.** Each lowering rests on a proved fact: that a word test holds only at
   that word's start; that a fill's start and stop hold only at token starts; and that a mask's
   readers read it at token starts only. Where a proof fails, the mask stays bytewise. That is
   a cost, never a wrong tape.

The gate is structural. The CPU body loses each token-level mask's operations (`structure`
prints them), the token loop gains the step's few, and the tapes stay byte-identical
(`aot_tapes_are_the_jits`, the corpus against the lean tapes). Measurements are information
(`jit_against_aot`).

## Order of the work

1. The analysis alone: which masks are token-level, and the operations they would remove.
   `tier1_cones` prints it.
2. Word tests and their `&`, `|` and `~`: the step in the token loop, and the marks set from it.
3. Fills over tokens: the state bits.
4. Gap marks read through the state.

## Where it stands

Before the work (2026-11-18, after the late marks' merge walk, load about 12): the CPU form has
1,500 operations a block. Tier 1 ahead of time takes 443M instructions and 88M cycles, against the
lean lexer's 54M and 15M. Tiers 1 and 2 take 799M instructions and 179M cycles, against the lean
parser's 299M and 60M.
