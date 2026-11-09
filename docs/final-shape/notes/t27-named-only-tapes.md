# T27: named-only tapes, assessment and plan (RER-131)

## Intro

A tape stores every tree-sitter node today, including anonymous tokens
such as `(`, `,`, `def` and `+`. Those tokens are 35–46% of all nodes.
Most of them hold nothing new: the parent's kind and its named children
already imply them. simdjson writes no tape entry for `{`, `:` or `,`
for the same reason.

This note covers four things:

- what a named-only tape saves;
- which tokens are not implied, and where each kind of token would go;
- every reader that has to change;
- a plan in steps, with one classification rule for every supported
  language, so that nodes are never reclassified again.

Base: the merge hub `final-shape-serverplan-quality` at efd4f325f.

## 1. What it saves

The data comes from a model of v9np's columns, using the parsers'
real trees. Rust keeps its macro token trees. Each parent's code also
holds its tokens that are not punctuation.

| Language | Corpus | Nodes | Bytes | zstd | Codes |
| -- | -- | -- | -- | -- | -- |
| Python | data-layer, 1 file in 5 (6.8M nodes) | 0.65× | 0.64× | 0.77× | 418 → 502 |
| TypeScript | Rerail and data-layer (2.4M) | 0.54× | 0.54× | 0.71× | 593 → 694 |
| JavaScript | Rerail and data-layer (0.69M) | 0.54× | 0.55× | 0.72× | 420 → 504 |
| Rust | data-layer and `native/src` (7.1M) | 0.60× | 0.60× | 0.75× | 496 → 654 |
| PHP | Rerail's fixtures (0.13M) | 0.55× | 0.56× | 0.71× | 465 → 477 |
| Vue | Rerail's fixtures (939) | 0.57× | 0.59× | 0.76× | 37 → 28 |

- Codes stay far below v9np's limit of 1023.
- Every pass that walks the tape would visit 35–46% fewer nodes:
  `inside`, `has`, `nearest`, kind postings, GPU slabs, the facts
  projection and the prefilter. This has not been measured.
- The PHP and Vue corpora are small. Blade and SQL are not in the
  table (SQL's run had not finished).

## 2. Which tokens are implied: proof from the grammar

The corpus can only show what happened to occur. A grammar can prove
what is possible. A prototype, kept in the session's scratchpad (not
committed), checks every visible kind in a `grammar.json`. It runs in
seconds per grammar.

1. Build the automaton over the kind's visible children: named children
   (with their field) and anonymous tokens. Hidden rules are inlined.
2. Determinize it. In a deterministic automaton each child sequence has
   exactly one path.
3. Erase the token labels.
4. Test the erased automaton for ambiguity with the square construction.
   If it is unambiguous, the named children alone fix the tokens. If it
   is ambiguous, the test gives a witness: the point where two token
   words can differ, and whether that point can repeat within one node.

The grammar's language contains every tree the parser can build, so a
"determined" verdict is sound. Kinds per grammar:

| Grammar | Determined | Punctuation, once | Choice, once | Repeats per element | Unbounded tokens |
| -- | -- | -- | -- | -- | -- |
| Python | 49 | 21 | 19 | 4 | 2 |
| JavaScript | 50 | 9 | 27 | 0 | 6 |
| TypeScript / TSX | 88 | 13 | 49 | 2 | 6 |
| PHP | 82 | 23 | 25 | 1 | 1 |
| SQL | 103 | 3 | 15 | 24 | 2 |
| Vue | 12 | 1 | 0 | 1 | 0 |
| JSON | 4 | 0 | 0 | 0 | 0 |
| Rust | not finished: macro token trees blow up the automaton | | | | |

What the columns mean:

- **Punctuation, once:** a trailing comma, the optional parentheses in
  `from x import (…)`, `{ , }`. One bit or a few bits per node.
- **Choice, once:** operators, `async def` versus `def`, `except*`,
  `const`/`let`/`var`, `get`/`set`, `readonly`. A small variant per
  node.
- **Repeats per element:** Python's comparison chains (`a < b <= c`),
  `;` between Python statements, the modifiers in Vue's
  `v-on:x.stop.prevent`, many SQL clauses.
- **Unbounded tokens:** JavaScript holes such as `[a,,b]` and
  `f(a,,)`, empty `;` members in class bodies, Python's relative-import
  dots, `case _ | _`, SQL's `[][]`.

## 3. Where each token goes

There is one rule for all languages. It is computed from the grammar,
not chosen by hand.

1. **Implied tokens are not stored.** "Implied" means the token sits
   on an edge of the kind's automaton that the erased automaton never
   needs to tell apart.
2. **A token that is not implied and occurs at most once per node goes
   into the parent's code.** Its edge is not on a cycle. This makes
   `op(n, "+")` a code test, so the kind postings answer it as an
   index.
3. **A token that is not implied and can repeat stays a node**, an
   anonymous leaf as today. It is kept only where it differs from the
   gap's default word (the shortest word, then the first in order), so
   `[a, b]` stores no commas and `[a,,b]` stores one. Rust's
   `token_tree` keeps all of its tokens under this rule.

The table that says which edge is which, and the defaults, is generated
from `grammar.json`. A test re-proves it whenever a grammar changes. A
new grammar version therefore never needs anyone to reclassify nodes:
it gets a new table, or the test fails and names the kind.

Edges are classified, not token kinds. In an array the separator comma
and the hole comma are different edges, so only holes are stored.
Classifying by token kind would keep every comma in every array.

### Cases outside the rule

- **Text that differs from the kind:**
  - PHP has 66 keywords that match in any case (`FUNCTION`);
  - Forte has `<!DOCTYPE` and `<?php` in any case;
  - Rust writes `b"` and `c"` where the kind is `"` (2,148 seen);
  - JavaScript and TypeScript have zero-width automatic semicolons.

  A token's text always comes from the source, so this is fine once
  token positions can be recovered (§4). The kind stays in the code
  or in the default.
- **Errors:** an ERROR subtree and any MISSING token keep all of their
  anonymous children as nodes, because the grammar's automaton does
  not describe them. Files with errors are already withheld from rules.
- **Own parsers:** Forte (Blade) and `logic::rr` emit their tokens
  themselves. The same classification is stated in their code, with a
  check over their node tables. Bitlex tier 2 drops its `LEAF` for an
  implied literal at compile time (decision 0088, `grammar.rs`
  `Pos::Tok`). The CPU writer of 0088 was removed in 7a4eeec86 and is
  being rebuilt through the compiler, so this is one change to that
  work, made with its session.

## 4. Token positions and ids

Rules use tokens as values with identity:

- `first_token`, `last_token`, `token_before` and `token_after` are
  used about 260 times;
- fixes edit tokens (`delete(t)`);
- findings are placed on tokens;
- `t != n` compares a token with a node: a named leaf is its own token.

What has to stay true:

- **Ids.** A token's id is a function of the tape. Each node's token
  count follows from its code and its stored tokens, and a prefix sum
  over those counts numbers the tokens. Today the facts projection
  numbers them after the nodes (`p.len() + i`); that numbering moves to
  the same arithmetic.
- **Positions.** A token's position is recovered on demand from the
  source between two named neighbours. The gap's words are known, and
  between tokens there is only white space (comments are named extras,
  so they are nodes already). So token i starts at the first byte after
  token i−1 that is not white space. This is a non-white-space mask over
  a 64-byte word, then a count of trailing zeros: word operations, with
  no loop per byte. Bitlex languages read the starts from tier 1's
  `stream start` mask instead.

## 5. The readers that have to change

About 330 sites read anonymous nodes or count them in preorder; about
60 of them depend on what a token is.

| Area | What it does | Kind of work |
| -- | -- | -- |
| ast-grep `Pattern` over `TapeDoc` | `.rr` `~` patterns (`core_rr.rs:852`) and `matcher/`: tokens in patterns must match a real child | hard |
| `transducer/packed` | `PatternTree::Terminal{named:false}`, `exact()`, `transducer_structural_key`, mode 0 traversal | hard |
| `facts/vtab/syntax` and `core_rr.rs` | `op`, `op_at`, token relations, `tree_digest` | hard, but local |
| native analyses: `scope/`, `value/`, `calls/`, `matcher/`, `literal/`, `blade/view.rs`, `embed.rs` | about 140 `children().filter(is_named)` sites; about 45 that count commas or read operators and keywords | mostly mechanical |
| prefilter, kind postings, `stats/ingest`, search index (one FTS token per preorder node) | preorder ordinals change | mechanical, plus version bumps |
| GPU: slabs, `metal_tape.metal`, `MetalNode.next_any` | named and all-children walks become one | mechanical |
| Preorder-keyed state: supertypes, overrides, labels, guest extents, `slice_match`, memo frames | renumbered | version bump |

**The way through: a token view.**

- `TapeDoc` keeps serving the full tree to the readers that want it.
  Its `children()`, `next()` and `prev()` interleave synthesized token
  leaves, built from the code, the defaults and §4's positions.
- With this view, ast-grep, the packed matchers and the native analyses
  keep working unchanged on the first day.
- Each reader then moves to the named-only walk with code tests,
  starting with the hot ones: the facts projection, the prefilter and
  the GPU.
- The view is the readers' working state and is never stored
  (decision 0026).

## 6. Plan of attack

Each step is committed on its own and checked with named tests only.

1. **Prove the classification in Rust.**
   - A generator reads each `grammar.json` (vendored grammars, plus
     Rust, PHP and JSON from the crates) and writes the edge table:
     implied, in the code, or stored.
   - A test re-proves the table on every grammar.
   - Rust's token trees are declared "store all", so the automaton does
     not have to model them.
   - Forte and rr get the same table from their own kinds.
2. **Measure with the table, before any format change.**
   - The real encoder runs on a named-only image beside the full one:
     sizes, the share of stored tokens, codes.
   - Corpora: the whole data-layer repository; Rerail; a public
     Laravel/Vue/SQL corpus, if you agree to fetching one.
3. **The token view in `TapeDoc`**, with a parity test: on the corpora,
   the synthesized children (kind, span, order) equal tree-sitter's.
   This test is the safety net for everything after it.
4. **Format bump.**
   - The encoder writes named nodes, folded codes and stored tokens.
   - Bump `TAPE_FORMAT_VERSION` 11 → 12, the positional and prefilter
     index versions, and the memo-frame schema constants.
   - Regenerate the expectations in `corpus_parity` and `v9np` tests.
5. **Rule-layer readers.** `op` and `op_at` read from the code.
   The token relations read from the view, by id arithmetic.
   Then run the S7 keys and the data-layer gate.
6. **Port the hot readers to named-only walks:** the projection,
   the prefilter and postings, the GPU slabs and kernels, the packed
   matcher's traversal modes. Then measure the CPU time of the tape
   passes.
7. **Tier 2** drops implied literals (with the Parser-generators
   session). Its byte-for-byte check compares against a named-only
   projection of tree-sitter's tree.
8. **The other readers** move off the view as they are touched.
   The view stays for ast-grep patterns.
9. **Decision record** with the measured outcome.

## 7. Open questions for you

1. **Stored tokens are leaves inside the tape.** The alternative is a
   side column of gap exceptions. Leaves keep the format regular, and
   Rust's token trees need them anyway. Recommended: leaves.
2. **Corpus for PHP, Blade and Vue:** may I fetch a public one (for
   example Laravel), or is the fixture set enough?
3. **The token view as the long-term interface for ast-grep `~`
   patterns:** acceptable, or should `~` patterns lower fully to named
   children plus code tests, as `logic/convert.rs` already does for some?

## 8. Integration results (stage A)

Stage A stores named-only tapes and restores them whole. It is behind
`RERAIL_NAMED_ONLY_TAPES=1` (commits fa9fbfb4b, cf9345e49). The real
encoder folds each tree-sitter tree (`tape/tokens.rs`) and sets a
header bit. Restore unfolds the tree and re-packs it, so every reader,
the GPU's included, sees the same full tree as before. This tests the
fold on real data and finds every reader of stored bytes before the
format changes.

How the fold decides, in short:

- Each kind gets an automaton over its children, built from
  `grammar.json`.
- A gap's default is the shortest token word to the next named child.
- Tokens are dropped only when the source matches the default and
  restore rebuilds the exact spans and kind ids.
- Anything else is stored, so the fold is correct by construction.
  A node the automaton cannot walk keeps all its tokens (`CODE_TOKENS`).

### Correctness

- **Round trip:** 0 failures on 4,691 files. The languages are Python,
  TypeScript, TSX, JavaScript, Rust, PHP, SQL and Vue, and 656 of the
  files contain parse errors.
- **S7, 101 Python files, 158 clauses:** the four backends
  (interpreter, JIT, Metal interpreter, Metal JIT) find the same 519
  keys with or without the switch.
- **Module tests with the switch on:** `prefilter::` and
  `facts::families::` pass. Five `tape::` tests and one `residency::`
  test fail because they assert the stored layout itself (digest, node
  counts, spans read in place). They change with the format bump.
- **Readers of stored bytes found:**
  - The SQL fallback's structure (`prefilter/tapes.rs`). It now unfolds
    first.
  - The device's in-place admission (`slabs::admit`). Under stage A it
    takes the host image (`admit_docs`).
  - All the other `from_bytes` callers go through restore.

**Which rules read tokens.** This is the work list for step 5. In a
run that read named-only trees with no token view, S7 lost 140 keys
and gained 3:

| Rules | Keys | Token they need |
| -- | -- | -- |
| `ret505`–`ret508` (superfluous else) | 104 | `else` |
| `e722` bare except, `b110` | 12 | `except` with no type |
| `b006` mutable default | 9 | `=` in a default parameter |
| `b404`, `b024`, `sim118`, `pie808` | 15 | keywords and `in` |
| `sim300` yoda conditions (gained 3) | +3 | operator: without it the comparison is guessed |

So the token view, or `op` read from the code, has to serve these
before stage B can ship. The gained keys show that a missing token can
cause false positives, not just missed findings.

### Size and memory (real encoder, whole fixture corpora)

| Language | Tokens / nodes | Implied | Nodes | Bytes | zstd | Restored records |
| -- | -- | -- | -- | -- | -- | -- |
| Python (1,309 files) | 34.9% | 89.6% | 0.69× | 0.69× | 0.81× | 0.71× |
| TypeScript (759) | 46.0% | 87.9% | 0.60× | 0.60× | 0.78× | 0.63× |
| TSX (34) | 46.0% | 92.1% | 0.58× | 0.61× | 0.78× | 0.60× |
| JavaScript (260) | 45.8% | 85.0% | 0.61× | 0.62× | 0.79× | 0.65× |
| Rust (608) | 45.6% | 87.9% | 0.60× | 0.60× | 0.77× | 0.62× |
| PHP (105) | 44.5% | 79.8% | 0.65× | 0.66× | 0.81× | 0.67× |
| SQL (1,607) | 42.1% | 31.4% | 0.87× | 0.87× | 0.88× | 0.87× |
| Vue (9) | 42.9% | 85.9% | 0.63× | 0.69× | 0.85× | 0.63× |

- "Restored records" is `TapeDoc`'s ~40 B per node. It is the memory
  a document takes once restored, and in stage B it shrinks by the
  ratio shown.
- In S7 the process footprint was 188 MB with full trees and 176 MB
  with named-only trees in memory (−6%). Most of the footprint is not
  tapes, so on S7 the in-memory gain is small. The gain grows with how
  many documents are held at once.
- Stage A itself saves no memory (190 MB), because it restores whole.
- **SQL is the weak case.** 1.15M quoted identifiers (`` `x` ``) do not
  match the unquoted default. A per-kind choice bit in the code
  (step 1's "choice, once" class) would remove most of them. The
  `end` kind is flagged in 148k nodes; I have not looked into why.
- **Stored because they are not the default** (fixable by policy):
  - JS/TS's explicit `;` (`expression_statement`, `lexical_declaration`
    and `return_statement`) is about 60k tokens. ASI makes the empty
    word the shortest, so the empty word is the default.
  - Python's trailing `, )` is about 30k.
  - Comparison operators and `async` belong in the code (§3 rule 2),
    not stored as leaves.

### Speed (CPU, debug build, load average about 100)

| Step | Cost |
| -- | -- |
| fold, at encode | 1.7–2.1× pack time |
| unfold, at restore | 1.2–1.9× restore time |
| pack, folded vs full | 0.65–0.87× |
| restore, folded vs full | 0.62–0.88× |
| S7 interpreter, full / named-only in memory / stage A | 34.8 s / 32.2 s / 37.9 s |

- Fold and unfold run once per encode and once per restore. Stage A
  pays both. Stage B pays only the fold, and readers that need tokens
  pay a per-gap recovery.
- Named-only in memory cut S7's interpreter CPU by 7%, while still
  reading a tree that was missing tokens. The machine was loaded, so
  treat the timings as indicative. Re-measure on a quiet machine before
  the decision record.
- The fold is a first cut in debug code: it walks a DFA per child, with
  hash-map lookups. It has not been tuned.

### What this changes in the plan

- Step 3 (the token view) is confirmed as the gate. Without it, 27% of
  S7's keys change.
- Policy work before the format bump:
  - put "choice, once" tokens in the code;
  - treat ASI as a pseudo-label;
  - a quoting bit for SQL identifiers;
  - check SQL's `end` and Rust's `let_chain` (no automaton yet).
- The device path has to read named-only slabs (step 6). Until then
  stage A's host-image detour is the fallback.
