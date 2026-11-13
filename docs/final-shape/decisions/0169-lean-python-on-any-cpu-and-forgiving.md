# 0169: The lean Python parser runs on any CPU, finds keywords by a hash, and recovers as tree-sitter does

Status: built (user, 2026-11-13: "please use or port SIMDJSON's portable SIMD operations so that this
can run on other CPUs, we need at least x86_64 / Take a look at using other techniques instructions for
the keyword recognition (it's decision 162 in the typing support worktree) / figure out the patterns
for the more-forgiving parser; we should match or mostly match tree-sitter on these, especially for
Python where we have very strong restart signals from the indents")

## Intro

The lean Python parser (0163) writes tree-sitter-python's tape directly. It had three gaps:

- **Arm64 only.** Its lexer found runs of word bytes and blanks 64 bytes a step with NEON. On any
  other CPU it did not build.
- **Keywords by comparison.** A name was checked against the keywords one by one.
- **No recovery.** A file with a syntax error went to tree-sitter-python. A file being typed mostly
  has one: the line being written.

This decision fills all three:

- The SIMD operations are simdjson's, ported to Rust.
- Keywords are found by a perfect hash (0162's technique).
- An error where a line is being typed is recovered as tree-sitter-python recovers it, node for node.
  Where the recovery cannot be predicted, the file still goes to tree-sitter.

```
a Python file -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
              -> the lexer: runs and stops by trailing zeros; keywords by one hash probe
              -> recursive descent, writing the tape
                   a statement fails at a NEWLINE tree-sitter's scanner would not give
                     -> that line end joined, the text lexed again
                   a statement fails where the line (or the text) ends
                     -> one ERROR, as tree-sitter builds it
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
- **0163.** **Amends:** its Open item "Error recovery" is built here, for errors where a line is being
  typed.
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

### Recovery, as tree-sitter-python recovers

The rules come from tree-sitter's runtime, and each is checked against its trees by the oracle:

- tree-sitter's parser reduces while a state has no shift;
- then it tries a missing token;
- then it recovers to a state in a stack summary, wrapping what it popped in an `ERROR` that is an
  extra.

**Joined lines.** tree-sitter-python's scanner gives a NEWLINE only where the parse state takes one.
Where it takes none (after `x =`, after `col:`), the line end is a blank, and the statement goes on in
the next line. The lean lexer runs ahead of the parser, so the parser reports such a line end, and the
text is lexed again with it joined (`lex::lex_joined`).

- A statement that fails at a NEWLINE asks for a join (`parse::Relex::Join`), unless a DEDENT follows.
  Where the next line is indented less, the scanner gives the DEDENT instead.
- A statement that then fails at the joined line's first token asks for the join to be undone
  (`Relex::Keep`). tree-sitter failed at that token too, so its `ERROR` is the first line's.
- There are at most 64 joins and kept line ends a text. The text is lexed and parsed again for each,
  which costs only on texts with errors.

So `_ =⏎b = 1` is the assignment `_ = b = 1`, and `col:⏎message: str` is `col: message: str`. That
needs `constrained_type`, which the parser now writes in annotations: `a: b` is
`constrained_type(a, b)`, right to left. A union beside a constraint is refused: tree-sitter reads it
there as a `binary_operator`.

**One ERROR at the failing line.** A statement that fails where its line ends (or the text does)
becomes one `ERROR`, an extra of the innermost block or the module:

- **Kept subtrees:** a subtree the parser closed is kept whole, without its field. An import's names
  keep `name:`, which a hidden rule gives them.
- **Open nodes:** dissolved; their pieces are the `ERROR`'s.
- **Tokens:** every token not inside a kept subtree is a leaf. `not in` and `is not` are their
  two-token operator nodes.
- **The last token:** whether tree-sitter reduced it depends on the token before it.
  - After `(` `[` `{` `=` `->` or a keyword that takes an operand, it is not reduced. Closed nodes that
    end at it are dissolved, and the leaf loses its supertypes.
  - Otherwise it is reduced; a trailing expression-primary becomes a primary.
- **Comments** before the statement stay outside the `ERROR`.
- **A block's first statement:** the `ERROR` is the compound statement's, before its block. The block
  is empty, at the `ERROR`'s end.
- **The rest of the line:** the tokens from the failure to the line's end are the `ERROR`'s too.
  Refused if one of them closes a bracket: tree-sitter picks the statement up again there.

**Refused, for tree-sitter-python:**

- a statement that fails over a joined line end. There tree-sitter's error mode reads keywords as
  names, and its `ERROR`s go on through the lines after. A join is undone (`Relex::Keep`) only where
  the next line is not indented past the first. Where it is, tree-sitter mostly skips the token into
  the `ERROR`. (Keeping `return` and `raise` at any indent gave 2 more exact texts and 47 more wrong
  ones; not taken.)
- a statement over a line end inside brackets that fails before the text's last token
  (`x = (1,⏎y = 2`). tree-sitter takes the statement up again after its `ERROR`.
- a string cut short;
- errors whose recovery tree-sitter picks by error cost: a root `ERROR` from the text's start, and a
  token skipped inside an assignment.

None of these can be predicted without tree-sitter's parse table.

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
| e-4k | 3,916 / 0 / 0 | 0 / 1 / 0 |
| cases | 56 / 0 / 0 | — |
| e-cut | 2,049 / 0 / 0 | 1,565 / 225 / 1,161 |
| e-edit | 2,138 / 0 / 5 | 57 / 76 / 1,800 |

Before this decision, every error text was refused. With recovery but before the join, e-edit's clean
texts were 2,098 same, 28 differ and 17 refused: recovery gave an `ERROR` where tree-sitter joined the
line. The 5 refused now are constrained types with a union.

The error texts that differ are wrong tapes, not refusals: the product would carry those tapes. They
are the open item below.

The x86_64 build was checked three ways:

- under Rosetta, the Westmere backend's masks match the Fallback's;
- the full oracle on x86_64 gives the same results as on arm64;
- the Haswell kernel's disassembly inlines `vpcmpeqb`, `vpshufb` and `vpmovmskb`.

Costs on arm64 (M5), profiling build, over the 1,000 files (`python_parse_costs`), in instructions a
node:

| | before (0163) | now |
|---|---:|---:|
| lean: the tokens alone | 228 | 236 |
| lean: the tape | 604 | 618 |
| tree-sitter-python: the tape | 8,912 | 8,945 |

Stage 1 and the hash replaced code that cost about the same. The rest is the recovery path's checks.
A clean text is never lexed again.

## Open

- **Error texts that differ.** They are recovered wrongly, so the product carries a tape tree-sitter
  would not write. Each remaining shape is to become a rule or a refusal; most are root `ERROR`s and
  error-cost choices.
- **Lexing again on a join** reads the whole text again. A lexer that resumes from a saved layout
  state would read only from the join.
- **rr_syntax's `micro.rs`** has NEON and portable code only. It could move onto `rerail-simd8`.
