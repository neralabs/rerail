# 0305: Go is read by a lean parser that writes tree-sitter-go's tape, or refuses

Status: built (user, 2026-11-16: "please create and test Rerail lean parsers for Go, C and C++ …
Your goal is to get to Treesitter-equivalence for direct-to-tape parsers on valid input"). Go became
a product language in the commit before this one (`languages/go.json`, the `lang-go` feature, tape
tag 27), read by tree-sitter-go until now.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node (kind,
field, supertypes).

The lean parsers (0163, 0169, 0178, 0193, 0195, 0196) write that tape directly. Each writes exactly
the tape tree-sitter's tree encodes to, and refuses any file it cannot read exactly. This decision
builds the same parser for Go, at rank 0 before tree-sitter-go.

```
a .go file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the lexer: runs and stops by trailing zeros; every word one hash probe
       a token carries the first `\n` before it and the one tree-sitter reads as a terminator
  -> recursive descent over tree-sitter-go's grammar, writing the tape (no backtracking)
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-go
```

There is one behavior: tree-sitter's tape, or a refusal.

## Prior work consulted

- **0196 (lean C#), 0195 (lean Java), 0193 (lean Rust).** **Agrees and follows:** the same crate
  shape, stage 1, keyword switch (`rerail_simd8::keyword_switch!`), writer and oracle.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`;
  the engine takes the columns as written (`TapeImage::written`).
- **The engine's implied-token automata.** **Followed:** the parser stores exactly the tokens the
  automata keep (`implied.tsv`, from `ecma_implied_tokens` with `RERAIL_IMPLIED_DIALECTS=go`).
- **tree-sitter-go's grammar** (`native/grammars/token-grammars/go/grammar.json`): the readings
  below follow it, checked by the oracle.

## Decision

### The crate: `rerail-go-lean`

`native/crates/go_lean`, in the shape of `rerail-java-lean`: `block` (stage 1: name bytes, blanks,
line ends, a string's stops, a comment's stops, backticks), `lex` (tokens where the parser asks,
no state), `parse` (`decl`, `types`, `stmt`, `expr`), `gen_kinds.py` (`kinds.rs` from the token
grammar and `implied.tsv`), `oracle`.

### tree-sitter-go's readings the parser keeps

- **Terminators.** tree-sitter-go's `\n` terminator is a hidden token: no node, but a statement or
  spec list's span ends after it. The `\n` taken is the last of the first run of white space that
  holds one; comments after that run stay outside the list. A `\n` ends a statement only where the
  grammar's state takes a terminator, so the lexer gives each token its preceding `\n` and the
  parser decides.
- **Declarations' parentheses.** `const (…)` stores its `(`; `type (…)` stores its `(`; a `)` after
  the last spec is implied. Only `const_spec`'s names hold their `,`s in the `name` field.
- **Escapes.** `\x` takes exactly two hex digits; the content after it is a new run.
- **GLR ties, settled by dynamic precedence (oracle-checked):**
  - `F[T](x)`, `pkg.F[T](x)` and `m[k](x)` with one argument are a `generic_type`'s
    `type_conversion_expression`; with none or several, a call with `type_arguments`, unless the
    brackets hold just `x` or `*x`, which index;
  - `(*T[A])(x)`, `(*[]T)(x)`, `(*map[K]V)(x)` convert through a `parenthesized_type`; `(*T)(x)` and
    `(*pkg.T)(x)` are calls; `*(*[]T)(x)` is a `pointer_type`'s conversion;
  - a composite literal's type is never read in an `if`/`for`/`switch` header (prec −1);
  - `interface{}(x).(type)` heads a type switch.
- **Supertypes.** `_type` is not a supertype set: a `parenthesized_type` in a type slot has no
  class; the simple types carry `_simple_type`.

### In the engine

`tape::go_lean::Lean` is Go's `TreeParser`, registered at rank 0 under `lang-go`. A file it refuses
goes to tree-sitter-go.

## Results

**The oracle** (`rerail-go-lean`'s ignored `oracle` test, run live under Relax) compares the
parser's tape with tree-sitter's, line by line. Its sets, from the shared corpus
(`~/orca/rerail-shared/lean-corpus/go`: 873 files of the stored repositories, 12 a repository):

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| whole files, 873 | 860 / 0 / 0 | 9 / 0 / 4 |
| cut (2 places) and edited (2 lines), 873 files | 1,575 / 0 / 0 | 11 / 0 / 2,266 |

Every clean file is read and none differs. The 4 refused error texts use `new(expr)` (Go 1.26),
which tree-sitter-go reads with an error.

**Engine tests:** `go_tapes_from_the_lean_parser_are_tree_sitters` (hand-written cases, byte for
byte against tree-sitter's encoded tape; syntax errors refused) and
`the_product_reads_go_with_the_lean_parser` (the engine's parse route gives the lean tape, a refused
file tree-sitter's).

**Cost:** `go_parse_costs` (ignored, `RERAIL_PY_FILES`) measures it; not yet run on a quiet host.

## Open

- **Cost on a quiet host** (`scripts/dev/quiet-run`), as 0196 reports it.
- **Go's capabilities** are `not-supported-yet`: rules cannot yet ask Go-specific questions.
