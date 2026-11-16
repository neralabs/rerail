# 0307: C and C++ are read by one lean parser that writes tree-sitter's tape, or refuses

Status: built (user, 2026-11-16: "please create and test Rerail lean parsers for Go, C and C++ …
Your goal is to get to Treesitter-equivalence for direct-to-tape parsers on valid input"). C and
C++ became product languages in e298ce7ee (`languages/c.json`, `languages/cpp.json`, the `lang-c`
and `lang-cpp` features, tape tags 28 and 29), read by tree-sitter-c and tree-sitter-cpp until now.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node (kind,
field, supertypes).

The lean parsers (0163, 0169, 0178, 0193, 0195, 0196, 0305) write that tape directly. Each one
writes exactly the tape that tree-sitter's tree encodes to, and refuses any file it cannot read
exactly. This decision builds the same kind of parser for C and C++, as a single crate with two
dialects. tree-sitter-cpp's grammar extends tree-sitter-c's, so C's node kinds are among C++'s.
Both run at rank 0, before tree-sitter.

```
a .c / .h / .cc / .cpp / .hpp file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the lexer: runs and stops by trailing zeros; every word one hash probe
       a token carries the `\n` before it (a directive ends there)
  -> recursive descent over tree-sitter-c's grammar, or tree-sitter-cpp's, writing the tape
       C++ only: a bounded speculation (mark, reset) where the grammar's GLR forks
       a syntax error, or a construct not read exactly -> refused -> tree-sitter
```

There is one behavior: tree-sitter's tape, or a refusal.

## Prior work consulted

- **0305 (lean Go), 0196 (lean C#), 0195 (lean Java).** **Agrees and follows:** the same crate
  shape, stage 1, keyword switch (`rerail_simd8::keyword_switch!`), writer and oracle.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`.
  The engine takes the columns as written (`TapeImage::written`). A node found after its first
  child is put before it (`Writer::insert`), and a speculation that fails is undone
  (`Writer::checkpoint`, `rewind`).
- **The engine's implied-token automata.** **Followed:** the parser stores exactly the tokens the
  automata keep, using one table per dialect (`kinds::DROPPED_C`, `kinds::DROPPED_CPP`, from
  `ecma_implied_tokens` with `RERAIL_IMPLIED_DIALECTS=c,cpp`).
- **tree-sitter-c's and tree-sitter-cpp's grammars**
  (`native/grammars/token-grammars/{c,cpp}/grammar.json`): the readings below follow them, and the
  oracle checks each one.

## Decision

### The crate: `rerail-c-lean`

The crate is `native/crates/c_lean`, built in the shape of `rerail-go-lean`:

- `block`: stage 1.
- `lex`: tokens are read where the parser asks, with no lexer state. Keywords are decided by the
  parser, as tree-sitter's keyword extraction decides them.
- `parse`: `decl`, `expr`, `stmt`, `pre` (the preprocessor's directives, by the item level they
  sit at), and `cpp` (C++'s additions).
- `kinds`: generated from the token grammars.
- `oracle`.

`rerail_c_lean::tape(src, Dialect::C | Dialect::Cpp)` is the entry point. The parser's keys name
kinds by tree-sitter-cpp's names. The engine maps each key to the dialect's ids by name.

### C: tree-sitter-c's readings the parser keeps

- **Declaration or expression.** A block's ambiguous item (`a * b;`, `T (x);`) is read as a
  declaration when it is first in its block. A typeless `M(a, b) {` is a function definition when
  its arguments read as parameters. A macro's type `M(T) x` is a `macro_type_specifier`.
- **Directives** are read as items at the level they sit at (top, block, field list, enumerator
  list, parameters), with tree-sitter-c's kinds for each level. `#if`'s condition is followed by a
  dropped `\n`.
- **K&R definitions**: `f(a, b) int a; char *b; {`.

### C++: tree-sitter-cpp's GLR ties, settled as its tapes show

tree-sitter-cpp forks where C++'s syntax is ambiguous. Its dynamic precedences and the order its
forks merge settle each tie. The parser reads each tie locally, by looking ahead or by a bounded
speculation, and the oracle checks each one:

- **Every ambiguous block item reads as an expression** (C reads the first as a declaration).
  Example: `void (*f)() = g;` in a block is a call. One shape keeps C's rule: `T [a](b);` declares
  only as a block's first item.
- **Most vexing parse.** `T x(a, b);` declares a function when every item reads as a parameter.
  - A literal's word (`nullptr`, `this`, `true`, `sizeof`) as the first item means an initializer.
    In later items, or after a pointer's or a reference's declarator (where the parse forks at
    the `(`), the word reads as a parameter's type.
  - A parenthesized declarator, as in `T (&x)`, loses to the initializer reading.
- **Out-of-line constructors.** In `A::A(std::unique_ptr<B> p)`, the first parameter's argument
  `B` reads as an expression. A primitive, a qualified name or a `const` argument reads as a type.
- **Casts and calls.**
  - `(T&)x`, `(T (*)(…))` and `(a) && b` are casts. The last is a cast of `&&b`, whose `&&` splits
    into two `&`s.
  - `(A::f)(x)` is a cast unless a call stands alone in the parentheses. `(T::max)()` is a call.
  - `sizeof(v<4>)` reads as an expression.
- **Assignments in parentheses.** An assignment alone in parentheses is
  `_assignment_expression_lhs`: any expression may be assigned there.
  - `++a %= b` is an update of the assignment.
  - `(a = b, c)` is an error, so the parser refuses it.
- **Template arguments** prefer a type. `true`, `false`, `nullptr` and `this` are expressions. A
  `>>` closing two argument lists splits in two. `a < b > (c) d` is a pair of comparisons.
- **Nested template arguments past tree-sitter's version cap are refused.** Each template argument
  forks tree-sitter-cpp's parse: a type (dynamic precedence 3) or an expression (1). Its GLR keeps
  at most 6 stack versions. Where nested lists fork past that, it drops versions by their order,
  not their precedence, and arguments read as expressions. Small files run through tree-sitter
  show it is not a property of the local syntax:

  | text | arguments read as expressions |
  |---|---|
  | `std::pair<A, B<C>> f(int);` | none |
  | `[[nodiscard]] std::pair<A, B<C>> f(int);` | `B<C>`, `C` |
  | `void f() { auto y = new X<A, M<N>>(x); }` | none |
  | `void f() { g(new X<A, M<N>>(x)); }` | `A` |
  | `x = static_cast<A<B<C>>*>(p);` | none |
  | `x = *static_cast<A<B<C<double>>>*>(p);` | `B<C<double>>` |

  The parser refuses a file with a template argument list inside a list's later argument, or
  lists four deep. The refusal is a flag no speculation undoes, so a failed template reading
  cannot fall back to comparisons. Every flip observed falls under the rule; it also refuses files
  tree-sitter happens to read with types (58 of the corpus's 63), which cost only speed.
- **A directive's line end is `token.immediate`.** After a blank, the line end cannot be read
  there, so the next line's text is the value (`#define A \nint x;` defines `A` as `int x;`).
  A `\r` just before the `\n` belongs to the line end. In C++, `0x` with no digit is `0` with
  the suffix `x`.
- **Implied tokens that tree-sitter-cpp stores anyway:**
  - `#if`'s `\n`, which spans a run of `\n`s.
  - alignof's `(`.
  - A lambda's `[` before a bare `&x` or `*this` (not before an init capture `&x = e`).
  - A `,` before `virtual` in a base class clause.
  - A field list's `}` after a bare `;`.
  - `delete []`'s brackets.
  - A raw string's delimiter.
- **Errors tree-sitter-cpp makes**, which the parser refuses:
  - `"a" N "b"`: a concatenated string's second piece must be a string.
  - `T &x(a.b)`: no initializer's arguments after a pointer's or a reference's declarator.
  - `M(T) x`: no `macro_type_specifier` in C++.
  - `a.operator*()`: an operator's name is a field's only with template arguments.
  - `p->A::~A()`.
  - `f(float, n)`.
  - `[&] -> T {}`.
  - `void f(T x = {})`: a parameter's default is an expression, so tree-sitter-cpp reads the
    braces as a compound literal with a MISSING type. (A member's `= {}` is valid.)
  - `delete [] (T*)p`, and `operator delete[](p)` as an expression: the `[]` reads as a lambda's
    head.

### In the engine

`tape::c_lean::Lean` is the `TreeParser` for both C and C++. It is registered at rank 0 under
`lang-c` and `lang-cpp`. A file it refuses goes to tree-sitter-c or tree-sitter-cpp.

## Results

**The oracle** is `rerail-c-lean`'s ignored `oracle` test, run live under Relax with shared caches.
It compares the parser's tape with tree-sitter's, line by line. Its sets come from the shared
corpus (`~/orca/rerail-shared/lean-corpus/{c,cpp}`, files of the stored repositories):

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| C, whole files | 829 / 0 / 0 | 0 / 0 / 744 |
| C, cut (2 places) and edited (2 lines) | 1,161 / 0 / 10 | 0 / 0 / 3,277 |
| C++, whole files | 937 / 0 / 63 | 0 / 0 / 560 |
| C++, cut (2 places) and edited (2 lines) | 1,271 / 0 / 113 | 0 / 0 / 3,614 |

No tape differs from tree-sitter's, and every text tree-sitter reads with an error is refused.
Every clean C file is read. Of the 1,000 clean C++ files, 937 are read and 63 refused, all by the
version-cap rule above (5 of them flip in tree-sitter). The refused cut and edited texts are those
files' cuts and fragments mid-typing that tree-sitter reads by taking a keyword as a name where
only a name fits (`type break;` declares `break`); those files go to tree-sitter.

**Cost** (`cpp_parse_costs`, `--profile profiling`, through `scripts/dev/quiet-run` on a leased
M1 Max, 2026-11-16; the best of three passes over the files both parsers read; "the tape" is the
engine's whole route, a parse and its tape):

| C++, 937 files, 10.6 MB, 1.92 M nodes | instructions a node | cycles a node | ns a byte |
|---|---|---|---|
| lean: the parser's columns | 564 | 143 | 8.2 |
| lean: the tape | 584 | 153 | 8.8 |
| tree-sitter-cpp: parse | 6,160 | 1,707 | 97.7 |
| tree-sitter-cpp: the tape | 9,141 | 2,446 | 139.9 |

| C, 829 files, 8.6 MB, 1.37 M nodes | instructions a node | cycles a node | ns a byte |
|---|---|---|---|
| lean: the parser's columns | 533 | 141 | 7.1 |
| lean: the tape | 555 | 153 | 7.8 |
| tree-sitter-c: parse | 6,098 | 1,700 | 86.2 |
| tree-sitter-c: the tape | 9,349 | 2,508 | 127.1 |

The lean route costs a sixteenth of tree-sitter's in cycles (C++ 153 against 2,446 a node; C 153
against 2,508). A refused file pays for the lean parser's attempt as well as tree-sitter's.

**Engine tests:**
- `c_tapes_from_the_lean_parser_are_tree_sitters` and
  `cpp_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, compared byte for byte
  with tree-sitter's encoded tape, with syntax errors refused.
- `the_product_reads_c_and_cpp_with_the_lean_parser`: the engine's parse route gives the lean
  tape, and a refused file gives tree-sitter's.

## Open

- **A narrower version-cap rule.** Reading those 58 files too needs a model of tree-sitter-cpp's
  stack versions (forks, merges, the cap of 6, the order versions are dropped in).
- **C's and C++'s capabilities** are `not-supported-yet`: rules cannot yet ask C- or C++-specific
  questions.
