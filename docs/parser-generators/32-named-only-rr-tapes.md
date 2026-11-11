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

This page says which `.rr` tokens the same rule leaves out,
and how the engine's parser, the lean parser and Bitlex now leave them out.
JSON follows: its punctuation and quotes are all implied.

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
  the `parameters` child's span says it, and an empty `()` is stored.
- `fix safe` is stored, so `has_token(f, "safe")` still holds.

## The parsers

The rule is applied per position in the grammar, not per gap at run time:
each literal the grammar writes is either implied or stored, everywhere it is written there.
`fix safe` stores `safe`; `fix` alone stores nothing.
The tape is still lossless: `from_tape` reads every statement back.

### The engine's parser and the lean parser

`syntax.rs` takes an implied token with `implied()`, which writes nothing.
A stored one is `p_stored()` or `anon()`. The lean parser does the same.
The lean parser's test still compares its tape with the engine's on 10,896 texts.

A node opened for children can now have none, e.g. `()` with both parens implied.
A comment inside it is its child.
A leaf is marked as made (`Built::leaf`), not inferred from having no children.

### Bitlex

The tree takes one new statement and one new word:

```
# The tokens written '(' ',' ')' in argument_list's own body make no leaf.
implied argument_list '(' ',' ')'

# A token written, then dropped at the next one: it stays, of kind
# gone, until the parse ends, so a reading (which names no gone
# node) still refuses what it would have.
node agg_argument_list nested =
  retag argument_list '(' [argument: term (',' drop argument: term)*] ')'
```

`implied` names a rule and tokens of its own body.
It is checked: a token the rule does not write is an error.
In tier 2 an implied token's arm writes no `LEAF`.
Where the language has extras, the arm keeps a `settle` (`LEAF m=3`):
it makes no node, but ends the extras pending, as a leaf does,
so a comment before it stays in the node it is in.
Without extras the arm has no word for the token at all.

`drop` is `RETAG m=2` to the dead kind on the node made last.
The dead records are compacted when the parse ends,
by the same pass that already removes them, so tier 2 needs no new word.

Two places in the grammar need `drop`, not `implied`.
A group, and an aggregate's word's arguments, are read again as terms.
That reading refuses a second literal (`(a(x), b(x)) + 1`) only because it sees the `,`.
So those `,` are written, then dropped.
They have rules of their own (`group_body`, `group_conjunction`, `agg_argument_list`).

The `.rr` grammar is `native/crates/rr_syntax/bitlex_ladder/21-named-only.bitlex`.
JSON's is `implied` on `object`, `pair`, `array` and `string`,
in `14-json.bitlex`, `15-jsonc.bitlex` and `lexers/json.bitlex`.

## Tests

| test | what |
|---|---|
| `rr_steps_parse_as_the_reference_and_the_engine` | step 21 against the engine's named-only tape; earlier steps against its named structure |
| `tape_is_the_engines` | the compiled step 21 against the engine on every `.rr` file |
| `lean_tape_is_the_engines` | the lean parser against the engine, 10,896 texts |
| `json_corpus_accuracy` | steps 14 and 15 against named-only tree-sitter-json: 1,603 of the repository's JSON files, all equal |
| `jsonc_comments_equal_tree_sitter` | comments placed after every token, delimiters too: 7,410 documents equal |
| the engine's `logic::rr`, `tape::tests`, `language::`, `facts::vtab::syntax` | 97, 35, 64, 28 pass |

## The numbers

The census after the change: 1,362 files, **1,156,952 tape nodes**
(was 1,915,215), **14,247 anonymous tokens** (was 772,510).

`rr_against_rust`, least of 5 rounds, load 26 to 55,
on the 1,360 files the grammar parses:

| | instructions | cycles | before |
|---|---|---|---|
| tier 1 | 532M | 107M | 532M, 106M |
| tier 2 | 559M | 136M | 642M, 151M |
| lean: lexer and parse | 389M | 76M | 466M, 84M |
| engine's Rust parser | 2,876M | 621M | 3,235M, 670M |

Tier 2's state machine grew from 234 states and 2,225 arms to 265 and 2,592,
with the three new rules.
A `RETURN` now has 68 places to go back to, up from 55,
and costs 219 instructions where it cost 180.
That would go with a native call and `ret` (page 30).

## Against RER-131's later rule

RER-131's last commit splits tokens in two classes, by text:

- a **delimiter** (brackets, `,` `;` `|` `<` `>`, quotes) is left out wherever its gap is implied;
- any **other token** (keywords, operators, `:`, `.`, `=>`) is left out
  only where its kind is implied in every gap it can stand in.

Here the rule is per position.
Under the later rule, `.rr` would store a few more tokens:

- the unary `-`, since a binary `-` is stored;
- the `:`, `.` and `!` that a setting's punctuation may also be.

None of them is read by the engine. Which rule `.rr` follows is open.
