# 0169: The lean Python parser runs on any CPU, finds keywords by a hash, and joins lines as tree-sitter does

Status: built (user, 2026-11-13: "please use or port SIMDJSON's portable SIMD operations so that this
can run on other CPUs, we need at least x86_64 / Take a look at using other techniques instructions for
the keyword recognition (it's decision 162 in the typing support worktree) / figure out the patterns
for the more-forgiving parser; we should match or mostly match tree-sitter on these, especially for
Python where we have very strong restart signals from the indents"; then: "recovery can always bail
to treesitter. I don't want to have different behavior in different "modes", it becomes impossible to
reason about")

## Intro

The lean Python parser (0163) writes tree-sitter-python's tape directly. It had three gaps:

- **Arm64 only.** Its lexer found runs of word bytes and blanks 64 bytes a step with NEON. On any
  other CPU it did not build.
- **Keywords by comparison.** A name was checked against the keywords one by one.
- **Line ends.** tree-sitter-python's scanner joins a line whose statement goes on (`x =⏎b`); the
  lean parser refused those files.

This decision fills all three:

- The SIMD operations are simdjson's, ported to Rust.
- Keywords are found by a perfect hash (0162's technique).
- A line end tree-sitter's scanner does not give is joined, as the scanner joins it.
- A syntax error is always a refusal: tree-sitter-python reads the file. Recovery was built and
  measured, then removed (below).

```
a Python file -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
              -> the lexer: runs and stops by trailing zeros; keywords by one hash probe
              -> recursive descent, writing the tape
                   a statement fails at a NEWLINE tree-sitter's scanner would not give
                     -> that line end joined, the text lexed again
                   anything else -> refused -> tree-sitter-python
```

## Prior work consulted

- **ast-index-spike indexes:** the spike parsed with tree-sitter only. It has no lexer or recovery to
  compare against.
- **0134, 0148 (JSON through simdjson; a kernel per CPU family).** **Agrees:** these chose simdjson's
  per-family kernels picked at run time. This decision ports simdjson's `simd8x64` layer to Rust, so
  Rust code gets the same kernels.
- **0162 (Typing-support worktree: SIMD lexing techniques).** **Agrees, and takes two:**
  - nibble-table byte classes;
  - keywords by a length-masked 8-byte load and a multiply-shift perfect hash.

  Not taken:
  - the double-shufti two-byte operators (operators are a few percent of the bytes, and the lexer
    reads them from the byte after a stop);
  - prefix XOR for strings (Python strings nest f-string interpolations, so quote parity does not
    give their extent);
  - Teddy (0162 itself says it is not for tokenizers).
- **0163.** **Keeps** its refusal of every syntax error. Its Open item "Error recovery" is closed:
  not built (below).
- **0158 (JSONC trailing commas).** **Agrees:** the same principle, being permissive exactly where
  the reference parser is.

## Decision

### Portable SIMD: `rerail-simd8`

The new crate `native/crates/simd8` is simdjson's `simd8x64` in Rust:

- **The trait.** `unsafe trait Simd8x64` holds 64 bytes as registers. Its operations are load, splat,
  eq, and, or, the high and low nibbles, 16-entry table lookup, nonzero, to-bitmask, eq-mask, and
  `classify` (`LO[c & 15] & HI[c >> 4]`).
- **Four backends:**
  - Arm64 (NEON);
  - Haswell (AVX2, with bmi1, bmi2, lzcnt, popcnt, pclmulqdq);
  - Westmere (SSE4.2, pclmulqdq);
  - Fallback (scalar).
- **The backend** is picked at run time by `best()` and cached. `dispatch!` wraps an
  `#[inline(always)]` kernel in one `#[target_feature]` function a backend, so the kernel compiles
  inline into each.

The lean lexer's stage 1 is one such kernel. It gives four masks a block (word, blank, line end, stop)
from nibble tables. Stage 2 is the scalar lexer, which reads runs and stops off the masks by trailing
zeros.

The `.rr` parser's micro-lexers (`rr_syntax/src/micro.rs`) are kernels over the same trait, so
they also run on any CPU. Before, they had NEON code and an 8-bytes-a-word portable version. The
trait took three operations so the NEON code maps one to one:

- `test` (NEON's `vtst`): a class test kept as a vector, so tests are combined before one pack;
- `ge` (`vcge`): a non-ASCII first byte (`>= 0xc0`);
- `shift_in` (`vext`): each byte's next one, for the two-byte punctuation's pair test.

The portable byte compare (8 bytes a word, SWAR) is now the Fallback backend's `eq_mask`, so wasm
keeps it.

The first port cost 1% (713 to 721 M instructions of lean rows over the repository's `.rr` files,
`rr_parse_costs`). By the disassembly:

- `nonzero(and(..))` cost an op a vector where `vtst` is one (now `test`);
- the non-ASCII mask was packed apart, a second pack (now one `or` before the pack);
- a load at offset 1 stood for `vext` (now `shift_in`);
- `dispatch!` made its function `#[inline]`, which inlined the string and comment scans into the
  lexer's loop and spilled its registers (the caller now picks);
- the window's padded path inlined a second copy of the kernel (now a cold call).

That left the call into the string scan, about 25 instructions a string where the old code inlined
it. A string that closes within its first 16 bytes with no `\` before its quote is now read inline
by two zero-byte word tests, and only the rest make the call.

Over the 1,461 `.rr` files, best of 9 passes, instructions retired:

| | before the port | the port | now |
|---|---:|---:|---:|
| lean rows | 748.5 M | 752.0 M | 745.4 M |

So the port is 0.4% faster than the NEON-only code.

### Keywords by a perfect hash

The 33 keywords and 8 soft keywords (`async`, `await`, `match`, `case`, `type`, `print`, `exec`, `_`)
each take one slot in a 128-slot table:

```
key  = the name's bytes, one 8-byte load masked to its length   // names over 8 bytes: not keywords
slot = ((key ^ len) * 0x8f58_640b_360e_7c81) >> 57
hit  = WORDS[slot].key == key && WORDS[slot].len == len
```

A soft keyword stays a name. Its code goes in the token's spare byte (`Tok.soft`), and the parser reads
it where the grammar takes the soft keyword.

### Joined lines, as tree-sitter-python's scanner joins them

tree-sitter-python's scanner gives a NEWLINE only where the parse state takes one. Where it takes none
(after `x =`, after `col:`), the line end is a blank, and the statement goes on in the next line. The
lean lexer runs ahead of the parser, so the parser reports such a line end, and the text is lexed again
with it joined (`lex::lex_joined`).

- A statement that fails at a NEWLINE asks for a join (`parse::Relex::Join`), unless a DEDENT follows.
  Where the next line is indented less, the scanner gives the DEDENT instead.
- There are at most 64 joins a text. The text is lexed and parsed again for each; a clean text that
  needs none is lexed once.
- Any other failure is a refusal, a joined text's too.

So `_ =⏎b = 1` is the assignment `_ = b = 1`, and `col:⏎message: str` is `col: message: str`. That
needs `constrained_type`, which the parser now writes in annotations: `a: b` is
`constrained_type(a, b)`, right to left. A union beside a constraint is refused: tree-sitter reads it
there as a `binary_operator`.

One more refusal keeps a clean-looking text exact: in an interpolation, after a bare name, `:=` is
tree-sitter's walrus token (`f"{x:=}"` is a `named_expression` with a missing value). After any other
expression it is a format specifier, as the lean parser reads it.

### No error recovery: one behavior

The lean parser gives tree-sitter-python's tape, or refuses. There is no second mode where it gives
some other tape for a text with an error.

Recovery was built first, from tree-sitter's runtime rules (reduce while no shift; try a missing
token; recover to a state in the stack summary, wrapping what was popped in an `ERROR`). It wrote one
`ERROR` where a statement failed at the end of its line, with tree-sitter's kept subtrees, dissolved
open nodes and leaves. Measured against tree-sitter's trees:

| set | error texts: as tree-sitter / differ / refused |
|---|---|
| e-cut (1,000 files, each cut at 4 points) | 1,576 / 215 / 1,160 |
| e-edit (1,000 files, 4 lines cut short each) | 57 / 76 / 1,800 |

The texts that differ were wrong tapes the product would carry. Most were root `ERROR`s and choices
tree-sitter makes by error cost between competing stack versions. No feature of the lean parse
separated the exact recoveries from the wrong ones: depth, a statement over several lines, the parent
kinds, the first child. Exact recovery needs tree-sitter's parse table and its cost competition, which
is tree-sitter itself. So every error is refused, and tree-sitter-python recovers.

If error texts become a cost, there are two paths that keep one behavior:

- port tree-sitter's recovery over its parse table (exact by construction);
- reparse only the damaged statement's region with tree-sitter and splice its tape in.

## Results

The oracle compares the lean parser's tapes with tree-sitter-python's, line by line. Every set is one
oracle file a text:

- **e-cut:** 1,000 files, each cut at 4 points.
- **e-edit:** 1,000 files, each with 4 lines cut short at half their length, the rest of the file
  kept.
- **e-4k:** 3,917 whole files.
- **cases:** 56 hand-written cases.

| set | clean texts: same / differ / refused | error texts: as tree-sitter / differ / refused |
|---|---|---|
| e-4k | 3,916 / 0 / 0 | 0 / 0 / 1 |
| cases | 56 / 0 / 0 | — |
| e-cut | 2,049 / 0 / 0 | 0 / 0 / 2,951 |
| e-edit | 2,138 / 0 / 5 | 0 / 0 / 1,933 |

No tape differs: each is tree-sitter-python's or a refusal. The 5 clean texts refused are constrained
types with a union. Before the join, e-edit's clean texts were 2,098 same, 28 differ and 17 refused.

e-4k's error text is `f"{labels:=}"`: tree-sitter reads a walrus with a missing value, the lean
parser read a format specifier; it is now refused (above).

The x86_64 build was checked three ways:

- under Rosetta, the Westmere backend's masks match the Fallback's;
- the full oracle on x86_64 gives the same results as on arm64;
- the Haswell kernel's disassembly inlines `vpcmpeqb`, `vpshufb` and `vpmovmskb`.

Costs on arm64 (M5), profiling build, over the 1,000 files (`python_parse_costs`), in instructions a
node:

| | before (0163) | with recovery | now |
|---|---:|---:|---:|
| lean: the tokens alone | 228 | 236 | 234 |
| lean: the tape | 604 | 618 | 615 |
| tree-sitter-python: the tape | 8,912 | 8,945 | 8,921 |

A clean text is never lexed again.

## Open

- **Error texts** go to tree-sitter-python. If they become a cost: tree-sitter's recovery over its
  parse table, or the damaged region reparsed by tree-sitter and spliced in (above).
- **Lexing again on a join** reads the whole text again. A lexer that resumes from a saved layout
  state would read only from the join.
