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
| A2 | `rounds … until settled` is not compiled | comments that hide quotes; quotes that hide `//` | the same, f-strings | a fixed number of rounds per block, each a copy of the body; a file that does not settle falls back. **Done** on the CPU as `rounds until settled` over a strictly causal body (03 §2.7): one group of the block program, a bounded loop per block in Cranelift. Metal refuses a group so far |
| A3 | `text("…")` is refused | directive words: `.decl` is one token, `.mode` two | keywords in tier 1 | a chain of `next` tests |
| A4 | a token's kind comes only from its first byte | `:` and `:-`, `<` and `<=` and `<:` | operators | one row per first byte, the longer spellings as keywords. **Done**: a row whose tokens run to their last byte tests each keyword, then takes the plain token |
| A5 | no wrap: a node cannot start before the token that decides it | `a - b * c`, comparisons, `x ~ \`…\``, `;` | every binary, call and attribute chain | **wrap**: open a node around the last node made at this level (§2.1). **Done** (`wrap K F [as M]`, word `WRAP`) |
| A6 | no retag: a node's kind is fixed when it opens | `.decl` and `.mode` both open at `.`; an atom and a call look alike until the token after | — | **retag** a node by the next token (§2.1). **Done** for the open node (`retag K A`, word `RETAG`), and for the node made last at the level (`relabel F as M [inner G]`, words `RETAG m=2`, `NODE_FIELD m=2`, `NODE_FIELD m=3`: a modal before `(` is the relation; an atom an operator follows is a call). A parenthesized term at a literal's start, which the engine tells from a group only by going back after its `)`, goes to the engine |
| A7 | no merge: two tokens cannot make one leaf | `.` and `decl` are one `.decl` leaf; `- 1` is one `number` leaf | — | **merge**: the previous leaf and this token become one leaf (§2.1). **Done** (`merge 'K' A`, word `MERGE`): the extras between are the leaf's text |
| A8 | no opaque nodes | a `{…}` JSON object, a `raw_text`, are one leaf each | — | **opaque** nested nodes: their tokens make one leaf. **Done** for balanced pairs (`balanced K O C`) |
| A9 | newlines are never tokens | a setting ends at its line's end; a declaration's tail is on one line | `NEWLINE` | **hidden tokens**: a token the grammar may name; in a state that does not name it, it is skipped and makes no node |
| A10 | keywords are reserved everywhere they have rows | `decl` after a statement's `.` only; `count`, `min`, `code`, `fix` are words elsewhere | soft keywords | **soft keywords**: the keyword's kind only in a state with an arm for it, the word's kind elsewhere. **Done** (`keywords K { } soft { … }`) |
| A11 | a nested level cannot start with another nested call, nor end optionally | `((a + b))`, an aggregate's body | suites, calls | lift both limits in the automaton. **Done** for ending optionally (§2.5), and for a call at a level's first token inside another: an arm makes every call it enters, outermost first (`PUSH` each), so the innermost returns first |
| A12 | with a tree, a byte in no token is not refused by the compiled path | — | — | refuse it, as the evaluator does |
| A13 | the file's end is no line break | a setting on the file's last line | NEWLINE at the end | **`mark K = E, end`**: the mark comes after the last token too. **Done** |
| A14 | an extra's leaf goes where its token is | a comment after a setting's last word | comments, everywhere | **extras placed late** (§2.3). **Done** |
| A15 | two rows may not begin with one byte | `//` beside the `/` operator | `#` comments, `.` and `...` | **`ROW`**: a word tests a row's start mask at the token's start (§2.4). **Done** |
| A17 | a keyword is at most 8 bytes | `violation` | `__future__`, `nonlocal` | `WORD` compares up to 64 bytes, 8 at a time, unrolled. **Done** |
| A18 | a choice cannot use itself | a term inside a named argument's value | every expression | **nested choices**: `choice N nested`, a level with no node of its own (§2.5). **Done** |
| A19 | a token's leaf has its token's kind | a word that is a variable, or a call's function, by the token after | names, attributes | **`A as K`**: the leaf of `A` is a `K`; a wrap gives the moved leaf its final kind. **Done** |
| A16 | which rows a state can meet is not known | — (it keeps `ROW` off JSON's strings) | string prefixes, f-strings | **proofs over the block program and modes from toggles** (§2.4). **Done** |

### 2.1 Wrap, retag and merge

These keep tier 2 an LL(1) automaton over the tokens, with one loop over the tokens and
linear work. Precedence comes from the rules' nesting, as in a Pratt parser:

```
choice term    = product (wrap binary_expression left operator: ('+' | '-') right: product)*
choice product = unary (wrap binary_expression left operator: ('*' | '/' | '%') right: unary)*
```

- **`wrap K F [as M] REST`** opens a `K` around the last node made at this level (the last
  child of the innermost open node), then matches the rest of its sequence inside it. `K`
  takes the moved node's place, field and start; the moved node takes field `F` (`_`: none)
  and, with `as M`, kind `M`. In a repetition it wraps again: `a - b - c` is `(a - b) - c`.
  Each operand is one node, so this is the Pratt scheme above: in `a + b * c`, the `*` wraps
  `b`, the last node made inside the `+`.
- **`A as K`**: the leaf of token `A` is a `K`. A word is a `variable` until the token after
  makes it a call's function:

  ```
  choice term nested = number | string | '_' as wildcard
                     | identifier as variable [wrap call function as identifier arguments: argument_list
                                              | wrap named_argument name as identifier ':' value: term]
  ```

- The node table is written in preorder, so a wrap's node must go before nodes already
  written. Each open node's frame holds its subtree's base (the first of its records, by
  when they were made) and the last node made at its level. A wrap's record is written at the
  end of the table and linked into its base's chain. At `ACCEPT`, one generated pass over the
  table writes it in preorder: each record in the order made, and before each its base's
  chain, latest (outermost) first; a second pass maps the supertypes' nodes to their places.
  Both run only when the file made a wrap, and a program without `WRAP` has no frames, no
  chains and no pass.
- **`retag`** changes a node's kind when the next token decides. `retag K A`: entering `A`,
  the innermost open node becomes a `K`. A statement opened at `.` is a setting until `decl`
  makes it a declaration:

  ```
  node setting = '.' (retag declaration merge '.decl' 'decl' … | name: identifier …)
  ```

  The node just closed is the other case: a call `f(x)` that a comparison does not follow is
  an atom (rr-07).
- **`merge 'K' A`** joins the leaf just written and the token `A` into one leaf of kind `K`.
  Extras between them are the leaf's text, as the engine has it: `. // c` and then `decl` is
  one `.decl` leaf. In tier 2 they are the pending extras (§2.3), dropped.

A step reserves the words of what it does not parse yet: `keywords identifier { "edit" ->
reserved … }`. The grammar never takes `reserved`, so a file with one goes to the engine's
parser. The last step reserves nothing.

### 2.5 Levels that end optionally, nested choices, chained returns

A nested level is a call: `PUSH` the state to return to, and go to the level's start. Three
things are new.

- **A level may end where it could go on.** After a word, a term may end, or a `(` or a `:`
  may wrap it. The state takes what its arms take and ends on any other token: it closes and
  returns to switch on that token again (`RETURN m=1`). The reference tries what may come
  before what follows; where that fails later, the reference may go back and the words refuse,
  so the file goes to tree-sitter, never to a different tree. An extra in such a state is
  deferred: the token after it, an arm's or the level's end, places it (§2.3).
- **A nested choice** (`choice N nested`) is a level with no node of its own. Its call opens
  nothing: the call's field and supertypes go to the level's first node, as an inlined
  choice's do, and its end closes only what it opened inside.
- **Returns chain.** A level that ends returns to a state that may end too, on the same
  token. That is a cycle with no `TOKEN`, through `RETURN`. The verifier allows it: from a
  `PUSH`, no `RETURN` is reached before a `TOKEN`, so a cycle with no `TOKEN` makes no call,
  and each turn pops one. A file's returns are at most its calls. Cranelift names each
  `RETURN` a loop's head (`Return`) where returns chain, and the tape's bounds count a
  return's segment once per call. A call whose level ends at its first token makes no call:
  the arm writes the level's closes and goes to the state it would return to. A state that
  takes no token gets a second entry, of a state's shape, for returns to land on.

### 2.3 Line breaks: marks, hidden marks, extras placed late

`.rr` reads lines in a few places only:
- A setting runs to the end of its line. A line that ends in `,` goes on.
- A declaration's tail is the words on the line of its `)`. Here a `,` does not join lines.
- Everywhere else, line breaks mean nothing.

The engine's rule, exactly: a token begins a new line when a `\n` lies between the start of
the token before it and its own start. The `\n` may be in a gap, in a comment, or inside the
token before: a string may span lines.

So a line break is not a byte between two tokens, and it can't be a token of its own. There
may be no byte for it:

```
.scope "a
b"x        // x is on a new line: the string holds the break
```

Three additions handle this, each one general.

**Marks (A9).** A mark is a token of no width.
- It is declared with a mask: `mark line = lf`.
- Each set bit puts the mark before the first grammar token that starts at or after it.
- Several bits before one token make one mark.
- Marks of several kinds before one token come in the order they are declared.
- The compiler places them in tier 1: the token starts with a bit since the start before,
  `starts & (m | prev(fill(m & ~starts, starts)))`.

`.rr` declares two marks:
- `line`: any line break;
- `newline`: a line break whose token before is not a `,`.

Python needs the same for NEWLINE, INDENT and DEDENT, which are also tokens of no width.

**Hidden marks.** `hidden line newline` in the tree:
- A hidden mark is taken where the grammar can take it, and skipped elsewhere. A clause never
  sees its line breaks.
- A mark that is not hidden and comes where the grammar can't take it refuses the file.

The reference decides this with follow sets: after each token, the mark is kept when the
kinds that may come next include it. That is the automaton's rule (a state with an arm on the
mark), so the two agree on every LL(1) grammar.

Why not grammar alone: "after a word, a break ends the setting; after a `,`, it is skipped"
has no one-unambiguous expression. Its two states form one orbit whose gates differ
(Brüggemann-Klein and Wood), so no Glushkov automaton is deterministic for it. The lexer
knows the `,`; the grammar does not need to.

In tier 2:
- `MARK k` tests mark k's bit at the token's start. If it is set, it takes the mark and runs
  the next word, the arm. If not, it skips that word.
- A state tests the marks it names, in order, before its `SWITCH`.
- A taken mark clears itself and the marks before it, and `SWITCH` clears them all.
- An arm that takes no bytes leaves the token for the next state. That state's `TOKEN` takes
  the same token again; it moves on only after a token's bytes are taken (`SPAN`).

**Extras placed late (A14).** A comment at the end of a setting's line:

```
.mode block   // a comment
.scope tests false
```

The comment goes in the setting when more words follow it, and after the setting when the
next line starts something else. Only the next token tells which. The comment's leaf is
written at once and counted as **pending**:
- a `CLOSE` leaves the pending leaves out of the node it closes;
- the next `OPEN` or `LEAF` ends the pending run.

A node table in preorder then holds each comment where tree-sitter's rule puts it: after the
nodes that end before it, before those that start after it. This needs no buffer and no copy
of a state. States where every way on closes the same nodes keep the early placement.

### 2.4 Rows that begin with one byte

Tier 2 reads a token's row from its first byte: one `SWITCH`, as simdjson reads its structural
characters. Two rows may begin with one byte, though:

```
// a comment       row `comment`, first byte '/'
a / b              row `punct`, first byte '/'
```

Then a `ROW` word tests a row's start mask at the token's start, one load and a branch. It
must cost nothing where the rows never meet: JSON's string text and its numbers both begin
with nearly any byte, and JSON should still switch on the byte alone. Two analyses keep it
there, both general:

**Proofs over the block program.** A small decision procedure answers "can this value have a
bit at byte b?". Its free values are the bytes around the position, and the carried values
(`toggle`, `escaped`, `fill`). Each carried value keeps its defining equation, so the search
knows, for example:

```
escaped(a) = prev(a) & ~prev(escaped(a))     # an escaped byte has the escape before it
toggle(x)  = prev(toggle(x)) ^ x
fill(s, e) = s | (prev(fill(s, e)) & ~e)
```

So a JSON number never begins with `"`: outside a string, an escaped quote follows a `\`,
which is itself a word byte. The same proof drops a fallback pair whose two rows can never
start at one byte.

**Modes from toggles.** A row's start often fixes a toggle: a string's text is inside the
quotes, a number outside. When every bit of the toggle's input is some token's start, only
those tokens change the mode. Then each state's modes follow from the grammar's machine:
arms, calls and returns, starting in mode 0. A state meets only the rows that fit one of its
modes. Arms it can't meet are dead, and a byte shared only with rows it can't meet needs no
`ROW`.

The result, checked by a test: JSON's tier 2 has no `ROW` word.

### 2.2 Regions: strings, backticks and comments

`.rr` has four kinds of region:
- strings `"…"`, which may span lines, with `\` escapes;
- backtick literals, `` `…` `` (fences of two and three are rare: 12 files);
- line comments `//…`;
- block comments `/*…*/`.

The first opener wins. Each kind is a `fill` from its opener to its closer, both from raw
bytes. One carry, the hidden bytes, hides the openers. It runs from the byte after each
opener through its closer, so it is `prev` of the fills: a strictly causal body, whose
`rounds until settled` is exact (03 §2.7). No `reflect` is needed. The engine's own readings
hold too: a block comment ends at the first `*/` from its `/`, so `/*/` is one; n backticks
end at the next n, whatever is between. Two cases go to the engine. One is a `` `` `` with
no closing pair, which the engine reads as the empty literal by looking to the file's end.
The other is a run of four backticks or more.

## 3. The `.rr` ladder

The steps are in `bitlex_ladder/rr/`. Each one is a `.bitlex` file and passes the ladder tests
before the next one starts. The last step is the whole language, checked against the
reference on the corpus.

| step | adds | compiler |
|---|---|---|
| rr-01 | `//` comments, `#include "…"` | — |
| rr-02 | settings: `.mode x`, `.scope "a", "b"`, the line's end, continued after a `,` | A3, A9 |
| rr-03 | `.decl r(a: t, b: t<u>)` and its tail on the same line | A4 |
| rr-04 | facts and clauses: heads, `:-`, atoms, numbers, strings, variables, `_`, calls, `n: v`, comparisons of single terms | A5, A11, A12, A18, A19 |
| rr-05 | `!`, `must`/`may` heads, `~` with backtick literals, strings, comments and block comments in any order | A2, A5, A6 |
| rr-06 | arithmetic with precedence, unary `-`, `- 1` as one leaf, `( )`, `@f(…)` | A5, A7, A11 |
| rr-07 | `;` alternatives, groups, atom or call | A5, A6 |
| rr-08 | aggregates: `count`, `min`, `max`, `sum`, `mean`, `join … by … sep`; their words soft, a value with no named argument; `min (…) : {…}` (told from a call after its `)`) goes to the engine | A10 |
| rr-09 | `.violation` (code, tag, fix, suppress, options), `=>` edits, JSON objects; a `fix` clause right after a `.violation` goes to the engine | A8, A10 |
| rr-10 | `.policy {…}`, its JSON checked as the engine checks it (`fold`, `refuse`); a document nested deeper than the JSON reader's limit goes to the engine | A8 |
| rr-11 | `.input`/`.output`, `.type`, `.lattice`, `.pragma`, `.functor`, `.edit`: all of `.rr`; what the engine refuses by the token after a directive's word or a functor's name goes to it | A8 |

At rr-11:
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
