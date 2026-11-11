# Named-only `.rr` tapes: which tokens are stored

## Intro

Tree-sitter tapes are becoming named-only:
a token the grammar implies is not written.
The `(` and `,` of `f(a, b)` are implied: the kind and the named children say where they are.
The `+` of `a + b` is not: `-` would fit as well, so it is stored.

The rule, from the tree-sitter work:
between two named children of a node (or before the first, or after the last),
the tokens are a *gap*.
A gap's tokens are implied when they are the **only shortest token word**
the grammar allows from there to the next named child.
A gap whose word is anything else is stored whole.
An ERROR node keeps all its tokens.

`.rr` tapes, from the engine's parser and from Bitlex, still write every token.
This page says which of them the same rule leaves out.

## The census

`native/crates/rr_syntax/tests/token_census.rs` walks the tape of every `.rr` file
and lists each node kind's gaps and the words seen in them.

```
cargo test -p rerail-rr-syntax --test token_census -- --ignored --nocapture
```

| | count |
|---|---|
| files | 1,362 |
| tape nodes | 1,915,215 |
| anonymous tokens | 772,510 (40% of the nodes) |
| in gaps where one word was seen | 761,512 |
| in gaps where several were seen | 10,998 |

`argument_list` alone holds 480,611 tokens: its `(`, `,` and `)`.

## Implied: not stored

Each of these is the only shortest word of its gap in the grammar,
not only in the corpus.

| Kind | Implied |
|---|---|
| `argument_list`, `column_list` | `(`, `,`, `)`; `( )` when empty |
| `clause` | `,` between heads and edits, `:-`, `=>`, `.` |
| `conjunction`, `disjunction` | `,`, `;` |
| `group`, `parenthesized_expression`, `io_relation` | `(`, `)` |
| `column`, `named_argument`, `functor_declaration` | `:` |
| `negation`, `functor_call`, `pattern_match`, `unary_expression` | `!`, `@`, `~`, `-` |
| the directives | `.decl`, `.edit`, `.functor`, `.type`, `.lattice`, `.pragma`, `.violation`, `.policy`, `#include`, a setting's `.` |
| `.violation`'s parts | `code` before a string, `code {` … `}`, `tag {` … `}`, `fix` alone, `suppress`, `over {` … `}`, the `,` between rules |
| `aggregate` | `: {` and `}`, `by` alone, `sep` alone, `,` between keys |
| `type` | `<` and `>` around an argument |
| `lattice_declaration`, `lattice_operation` | `<`, `> {`, `}`, `->` |

## Stored

| Where | Why |
|---|---|
| comparison and arithmetic operators | several fit (`=`, `!=`, `<`, …; `+`, `-`, …) |
| a head's `must`, `may` | none is shortest |
| an aggregate's word (`count`, `min`, `max`, `sum`, `mean`, `join`) | several fit |
| `fix safe`, `by (`, `) sep` | not the shortest word: the gap is stored whole |
| `.input`, `.output` | two fit |
| a type's `< >` with no argument | the shortest is none |
| a setting's and a declaration's punctuation | any punctuation fits |

## What reads them now

Three places in the engine's `from_tape.rs` read a token the rule leaves out:

- `ty` asks for a `<` to tell `T<a>` from `T`: an `argument` child says it.
  `T<>` keeps its tokens.
- `.input`/`.output` parameters are cut between the `(` and the `)`:
  the `parameters` child's span says it.
- `fix safe` is stored, so `has_token(f, "safe")` still holds.

## To do

- The engine's `.rr` parser, the reference, stops writing implied tokens.
  The lean parser follows it. The tape test still compares them.
- Bitlex's tier 2 drops the `LEAF` of an implied literal at compile time,
  for `.rr` and for JSON.
- A test checks every kept and dropped token against the rule,
  on the census corpus.
