# Parser generators: `.rr` and `.bitlex` in Bitlex, the plan

Status: plan, 2026-11-08. The work after JSON (docs 21 to 23) and before Python.

## Intro

Two more languages get Bitlex files, compiled by the Bitlex compiler and lowered through the
transducer's own backends, with no Rust between the bytes and the nodes:

- **`.rr`**, the rule language: Soufflé's Datalog with rerail's additions;
- **`.bitlex`**, the Bitlex format itself.

Each is built by a ladder of small steps, test first. Each step adds one construct to the
language, and the compiler grows only where a step needs it. Python comes next. So every
addition to the compiler is general, specified in the reference evaluator first, and
lowered in every backend, as `reflect` and `count` were meant to be.

This page covers:

- what decides that a parse is right: the references (§1);
- what the compiler lacks today, and the additions this work makes (§2);
- the two ladders (§3 and §4);
- how speed is measured (§5);
- the process (§6).

## 1. The references

**`.rr`.** The engine's own parser (`logic/rr/syntax.rs`) already builds a tree: kinds and
fields from `logic/rr/tape.rs`, byte spans, comments attached to the innermost node that holds
them. That tree is the reference, node for node: kind, field, start, end, and the nesting.
- The kind and field names in `rr.bitlex` are `tape.rs`'s names.
- The corpus is the repository's 1,346 `.rr` files, 11 MB.
- The reference marks a statement it cannot parse as an `ERROR` node and goes on. A Bitlex
  parser does not recover: it refuses the file, and the file goes to the reference parser.
  A refusal is never a wrong tree. The figure that counts is how many files are accepted
  with equal trees, and the count of refusals is reported beside it.

**`.bitlex`.** Its parser (`tools/bitlex/src/syntax.rs`) records no positions. It gets a
taped mode, as `.rr`'s parser has: the same parse, building a tree with byte spans, whose
kinds follow its `Stmt`, `Expr` and `Gram` forms.
- The corpus is the repository's 651 `.bitlex` files.
- Every corpus file is also mutated a byte at a time. The generated parser must accept
  exactly the files `syntax::parse` accepts, with equal trees.

**The compiler** is checked as the ladder of doc 20 checks it. For each step and each
document, these must give the same nodes or the same refusal:
1. the reference evaluator (`tools/bitlex`);
2. the interpreters;
3. Cranelift;
4. Metal, for tier 1.

Each step also has the promised shape: tier 1 branch-free per block, tier 2 one loop over
the tokens, no calls.

## 2. What the compiler lacks, and what this work adds

The gaps below are the ones the two languages hit. Each addition is general, and each is one
Python needs too.

| | gap | `.rr` needs it for | Python needs it for | addition |
|---|---|---|---|---|
| A1 | `scan … = reflect(…)` is not compiled | `"` strings beside backtick literals, the first opener wins | `'` and `"` strings | lower as designed (doc 08 §2.2): a toggle, then a prefix sum mod 3 over two bit planes |
| A2 | `rounds … until settled` is not compiled | comments that hide quotes; quotes that hide `//` | the same, f-strings | a fixed number of rounds per block, each a copy of the body; a file that does not settle falls back |
| A3 | `text("…")` is refused | directive words: `.decl` is one token, `.mode` two | keywords in tier 1 | a chain of `next` tests |
| A4 | a token's kind comes only from its first byte | `:` and `:-`, `<` and `<=` and `<:` | operators | one row per first byte, the longer spellings as keywords (works today: test it) |
| A5 | no wrap: a node cannot start before the token that decides it | `a - b * c`, comparisons, `x ~ \`…\``, `;` | every binary, call and attribute chain | **wrap**: open a node at a saved anchor, around what came since (§2.1) |
| A6 | no retag: a node's kind is fixed when it opens | an atom and a call look alike until the token after | — | **retag** the node just closed, decided by the next token (§2.1) |
| A7 | no merge: two tokens cannot make one leaf | `- 1` is one `number` leaf | — | **merge**: the previous leaf and this token become one leaf (§2.1) |
| A8 | no opaque nodes | a `{…}` JSON object, a `raw_text`, are one leaf each | — | **opaque** nested nodes: their tokens make one leaf |
| A9 | newlines are never tokens | a setting ends at its line's end; a declaration's tail is on one line | `NEWLINE` | **hidden tokens**: a token the grammar may name; in a state that does not name it, it is skipped and makes no node |
| A10 | keywords are reserved everywhere they have rows | `count`, `min`, `code`, `fix` are words elsewhere | soft keywords | **soft keywords**: the keyword's kind only in a state with an arm for it, the word's kind elsewhere |
| A11 | a nested level cannot start with another nested call, nor end optionally | `((a + b))`, an aggregate's body | suites, calls | lift both limits in the automaton |
| A12 | with a tree, a byte in no token is not refused by the compiled path | — | — | refuse it, as the evaluator does |

### 2.1 Wrap, retag and merge

These keep tier 2 an LL(1) automaton over the tokens, with one loop over the tokens and
linear work. Precedence comes from the rules' nesting, as in a Pratt parser:

```
choice term    = product (wrap binary_expression left operator: ('+' | '-') right: product)*
choice product = unary (wrap binary_expression left operator: ('*' | '/' | '%') right: unary)*
```

- **An anchor** is saved where a rule with a `wrap` takes its first token: the node table's
  next index. A frame of a nested level holds its anchors.
- **`wrap K f`** opens a `K` around everything since the anchor. The first node moved gives
  `K` its field and takes `f`. A wrap nested in a repetition wraps again: `a - b - c` is
  `(a - b) - c`.
- The node table is written in preorder, so a wrap's node must go before nodes already
  written. A wrap is written at the end of the table with a link to its anchor. The final
  extract pass, which writes the tape, puts each wrap before the node it was anchored at: one
  pass, linear, run only when a wrap was made.
- **`retag`** changes the kind of the node just closed, and the field of its first child,
  when the next token decides: a call `f(x)` that a comparison does not follow is an atom.
- **`merge`** joins the leaf just written and the current token into one leaf.

### 2.2 Regions: strings, backticks and comments

`.rr` has four kinds of region:
- strings `"…"`, which may span lines, with `\` escapes;
- backtick literals, `` `…` `` (fences of two and three are rare: 12 files);
- line comments `//…`;
- block comments `/*…*/`.

The first opener wins. The quotes are one `reflect` scan of two kinds. The comments are
fills from openers found `out` of both kinds. A comment hides the quotes after its opener:
`rounds` feeds them back until nothing changes. Most blocks settle in one round, and
the confirming round can be skipped when no comment opener sits beside a quote.

## 3. The `.rr` ladder

The steps are in `bitlex_ladder/rr/`. Each one is a `.bitlex` file and passes the ladder tests
before the next one starts. The last step is the whole language, checked against the
reference on the corpus.

| step | adds | compiler |
|---|---|---|
| rr-01 | `//` comments, `#include "…"` | — |
| rr-02 | settings: `.mode x`, `.scope "a", "b"`, the line's end, continued after a `,` | A3, A9 |
| rr-03 | `.decl r(a: t, b: t<u>)` and its tail on the same line | A4 |
| rr-04 | facts and clauses: heads, `:-`, atoms, numbers, strings, variables, `_` | A12 |
| rr-05 | `!`, comparisons, `~` with backtick literals, strings and comments in either order | A1, A2, A5 |
| rr-06 | arithmetic with precedence, unary `-`, `- 1` as one leaf, `( )`, `@f(…)`, `n: v` | A5, A7, A11 |
| rr-07 | `;` alternatives, groups, atom or call | A5, A6 |
| rr-08 | aggregates: `count`, `min`, `max`, `sum`, `mean`, `join … by … sep` | A10 |
| rr-09 | `.violation` (code, tag, fix, suppress, options), `=>` edits, JSON objects, `.policy {…}` | A8, A10 |
| rr-10 | `.input`/`.output`, `.type`, `.lattice`, `.pragma`, `.functor`, `.edit`, block comments: all of `.rr` | A8 |

At rr-10:
- every corpus file is parsed by both parsers;
- the trees are compared node by node;
- each refusal is counted by its reason.

## 4. The `.bitlex` ladder

The steps are in `bitlex_ladder/bitlex/`. Bitlex is line-based: a newline ends a statement,
unless the brackets are open or an operator ends or starts the next line.

| step | adds | compiler |
|---|---|---|
| bl-01 | `#` comments, `lexer NAME`, blank lines | A9 |
| bl-02 | `class` with characters, numbers, ranges and `U+` | — |
| bl-03 | `mask` with `\| ^ & ~` and calls, by precedence | A5 |
| bl-04 | the line joins: open brackets, a trailing operator, a leading `\|` or literal | A9, A10 |
| bl-05 | `token`, `keywords` (with `->`, `soft`, `required`), `skip`, `error`, `escape` | — |
| bl-06 | `scan`, `rounds`, `hypotheses`, `block`, the tests `.in` and `.frame` | A11 |
| bl-07 | `tree`, `node`, `choice`, `check`, `extra`, with `*`, `+`, `[ ]` and fields | A5 |
| bl-08 | the rest: `machine`, `munch`, `ucode`. All of `.bitlex`, against the corpus | — |

## 5. Speed

Each language's last step is timed on its corpus. For each figure, the median CPU time of 20
runs:
- tier 1 and tier 2, on Cranelift;
- the hand-written parser: `.rr`'s taped parse, and `tools/bitlex`'s `syntax::parse`.

The machine's load is given with every table. Tier 1 is also timed on Metal. Each step's shape
is pinned, as JSON's is: the operations per block, the CLIF body, the words by opcode.

## 6. Process

- Test first, at every step: the step's tests fail until the compiler handles it.
- Each compiler addition is added to the reference evaluator first, then the interpreters,
  then Cranelift, then Metal where tier 1 is touched.
- One step at a time. Each step is committed when it passes.
- Before each report: a table of plan item → `file:line` → done, changed or missing.
