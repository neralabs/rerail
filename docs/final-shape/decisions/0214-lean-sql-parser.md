# 0214: SQL is read by a lean parser that writes tree-sitter-sql's tape, or refuses

Status: built (user, 2026-11-14: "We also need SQL, I forgot to mention it last time"). SQL is a
product language (`languages/sql.json`, `lang-sql`), read by tree-sitter-sql until now.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node.

The lean parsers of 0163 through 0205 write that tape directly. Each one writes exactly the tape
tree-sitter's tree encodes to, and refuses any file it cannot read exactly. This decision builds the
same parser for SQL, at rank 0, before tree-sitter-sql.

SQL differs from the other languages in two ways:

- **Its keywords are words.** tree-sitter-sql has about 500 keywords, and almost any of them may
  also be a name. tree-sitter's lexer takes a word as a keyword only where the parse state takes that
  keyword. The parser asks the same question at each word.
- **Its tape leaves out most punctuation by the parent's grammar.** Which tokens the tape stores is
  decided by the engine's implied-token automata, per node. The parser runs the same automata on
  each node it writes.

```
a .sql file
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8): blanks, word bytes, `/*`, line ends,
     quotes
  -> statements, top down, writing the tape:
       a word: the keywords it is (one lookup a token); a keyword where the state takes it,
       a name where it does not, refused where the parser cannot tell
       expressions: precedence climbing, by tree-sitter-sql's precedence levels
       each node's tokens: stored or left out by the engine's automata
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-sql
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery.

## Prior work consulted

- **ast-index-spike indexes:** the spike's sample corpus (`ast-index-sample-corpus`) holds 3,353
  SQL files (PostgreSQL's tests and contrib, sqlfluff's dialect fixtures, TimescaleDB). The oracle's
  set is 600 of them.
- **0205 (lean Markdown), 0200 (lean CSS), 0199, 0198, 0196, 0195, 0193, 0163.** **Agrees and
  follows:** the same crate shape, writer and oracle; "no error recovery: one behavior".
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`.
- **The engine's implied-token automata (`tape::tokens`).** **Followed, by the automata
  themselves:** the engine now prints them (`TokenGrammar::automata_text`, the ignored test
  `token_automata_text`), and the crate's generator turns them into tables. The parser builds each
  node's children with all their tokens, then decides which the tape stores exactly as the engine
  does (a gap's tokens are left out only where the gap is the automaton's implied word and the token
  is not kept). It never writes a token to filter it out afterwards: the arena holds the parse, and
  the tape is written once, from it.
- **tree-sitter-sql** (DerekStride's grammar, as the engine vendors it): its `grammar.json` gives
  the kinds, fields, keywords and precedences. The readings below follow it, checked by the oracle.

## Decision

### The crate: `rerail-sql-lean`

`native/crates/sql_lean`, in the shape of `rerail-css-lean`:

- `block`: stage 1. A mask a class for each 64-byte block: blanks, word bytes (`[A-Za-z0-9_$]` and
  bytes past ASCII), `/*` and `*/`, line ends, `'` and `\`, `"`. Blank runs, words, comments
  (nested) and quoted strings end at the lowest set bit of a mask.
- `parse`: the lexer, the arena (nodes and their children, in parse order), the automata walk, and
  the writer's pass. `parse::expr`: expressions, types, arguments. `parse::stmt`: statements.
- `gen_kinds.py`: writes `kinds.rs` from tree-sitter-sql's grammar and the engine's automata
  (`automata.tsv`, `implied.tsv`): 773 kinds, 61 fields, the 517 keyword words, the automata
  tables, and the FIRST sets of an expression and a type.

A key is `kind | field << 10 | extra << 16`.

### Keywords

A word's keywords are looked up once a token (a word may be two: `character`). Where the parser
reads a name, it says which keywords the state could take there:

- **Sure:** a keyword the state does not take is a name (`select a from t`: `a` and `t`).
- **Unsure:** where the parser cannot tell, a keyword is refused (`alter table only add …`: is
  `only` the table?).

The keywords that may follow a term (`AFTER_TERM`) end an expression, so `select a from t` reads
`from` as a keyword, not as `a`'s alias.

### Where tree-sitter's choices are kept

These were found by the oracle, and the parser keeps each as tree-sitter reads it:

- **A node's field is the innermost.** tree-sitter's cursor looks for a node's field from its
  nearest hidden parent out, so a field given inside a hidden rule wins over one given outside it.
- **Extras** (comments, psql meta-commands) go in the deepest node with children on both sides of
  them. The program holds the leading and trailing ones, and its span runs from its first token or
  extra to the text's end.
- **Operators:** the lexer takes the longest of the grammar's operator strings and `op_other`'s two
  patterns; a string wins a tie. Precedence levels are tree-sitter-sql's, from `OR` (1) to the `::`
  cast (21).
- **GLR choices:** a string argument after a comma is a `term`. `begin; …; commit` is a
  `transaction` node: the parser reads it from a saved state and falls back to
  `transaction_statement` where it does not hold. This and a few two-token looks are the only
  places the parser goes back.
- **Small shapes:** a function in `from` is `relation > invocation`, with its alias; `float8` is a
  `double` leaf with no keyword in it; `(values …)` in `from` uses the relation's own parentheses; a
  send meta-command (`\g`, `\gset`, …) is a named `meta_command` in the program, in a `;` place; an
  `alter table … add constraint` body is a `constraint` node; `character varying` is one
  `keyword_varchar` over both words, `keyword_varying` in it; a `filter (where …)` is the
  invocation's last child; `name '…'` and `timestamp '…'` are a `literal` of the name or type and the
  string (`_string_casting`), where `date` is an identifier.
- **`not` after an expression** that no `in`, `like` or `between` follows ends the expression
  (`default 0 not null`).

### Read

`select` (with CTEs, set operations, joins and lateral joins, subqueries, `group by`, `having`, `window`, `order by`,
`limit`, `offset`, `fetch`), `values`, `insert` (with `on conflict`), `update`, `delete` (with `using`),
`truncate`; `create table` (columns, column and table constraints, storage parameters),
`create index` (columns and function fields), `create view`, `create materialized view`,
`create function`, `create trigger`, `create sequence`, `create extension`, `create schema`,
`create type`, `create role`; `drop` (tables, views, materialized views, indexes, functions,
sequences); `refresh materialized view`; `alter table` (`only`; add and drop columns, `alter column`'s
`set`/`drop not null`, `set`/`drop default` and `type`, add, drop and validate constraints, `rename
[column] … to`, `rename to`, `owner to`, `set schema`, `set (…)` and `reset (…)`),
`alter sequence`, `alter function`, `alter extension`; `grant`, `revoke`; transactions; `set`,
`reset`; `comment on`; `call`; `do`; psql meta-commands. Constraints take their attributes (`[not]
deferrable`, `initially deferred`, `not valid`, …). Expressions: literals, strings with
continuations, dollar-quoted strings, string casts, parameters (`$1`, `?`, `:name`), casts, `case`, `exists`, `in`,
`like`, `between`, `is` forms, `collate`, arrays, `interval`, subscripts, window functions (`over`
a name or a specification: `partition by`, `order by`, `rows`/`range`/`groups` frames), `filter`, an
aggregate's `distinct` and `order by`, `extract(unit from …)`.

pg_dump's output (a Rails `db/structure.sql`: `set` lines, extensions, tables with `character
varying` and `timestamp(6) without time zone`, sequences and `owned by`, `alter table only … alter
column … set default nextval(…)`, `add constraint`, `create index … using btree (lower(…))`,
triggers, enums, views, functions) is read whole.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error;
- the statements not listed above: `create operator`, `operator class` and `family`, `aggregate`,
  `foreign data wrapper`, `transform`, `access method`, `domain`; `alter operator`, `alter role`,
  `alter index`; `copy`, `load`, `prepare`, `execute`, `explain`, `merge`, `vacuum`;
- the clauses not listed above: a table's `partition by` and `inherits`, `tablesample`, index hints, nested joins in parentheses, `exclude` frames, `is json`, `escape`, a parenthesized
  index field;
- forms of other dialects that tree-sitter-sql reads (backticks, `!`, Delta and Spark SQL);
- bytes that are not UTF-8;
- quoted parameters (`:'name'`, psql's).

### In the engine

`tape::sql_lean::Lean` is SQL's `TreeParser`, registered at rank 0 under `lang-sql`. A file it
refuses goes to tree-sitter-sql. Its kinds and fields map to tree-sitter-sql's ids by name.

## Results

**The oracle** (`rerail-sql-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line, over the 600 files:

| | same | differ | refused |
|---|---|---|---|
| clean texts (224) | 124 | 0 | 100 |
| texts with errors (376) | — | 0 | 376 |

**Hand-written files** (scratch, not in the repository; their forms are in the engine test below):
two `structure.sql` files of pg_dump's forms, window functions, upserts and string casts; then
SQL as source code holds it: application queries (parameters, `distinct on`, `ilike`,
`returning`, aggregates), migrations, JSON operators, lateral joins, `alter table`'s actions, and
ORM-written queries (Rails', Django's, Hibernate's). Their tapes come from tree-sitter through
`python_tape_expect`. All 11 are the same; Hibernate's `id=?` is a syntax error to tree-sitter
(`=?` is one `op_other`), and is refused.

No tape differs. Of the texts with errors, 277 are sqlfluff's fixtures, many of other dialects
(T-SQL, BigQuery, Snowflake, …); the rest are PostgreSQL's and TimescaleDB's tests, with forms
tree-sitter-sql does not read.

**Engine tests:**

- `sql_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape, among them pg_dump's forms, window functions, upserts and string
  casts; four texts with errors are refused.
- `the_product_reads_sql_with_the_lean_parser`: the engine's parse route gives the lean tape, and a
  refused file gets tree-sitter's.

**Cost** (`sql_parse_costs`, profiling build, over the 124 corpus files the lean parser reads:
168,895 bytes, 25,972 nodes; with 0219's keyword switch):

| | instructions a node | cycles a node |
|---|---|---|
| lean: the parser's columns | 844 | 221 |
| lean: the tape | 881 | 244 |
| tree-sitter-sql: parse | 4,030 | 1,099 |
| tree-sitter-sql: the tape | 6,982 | 1,939 |

(Cycles under a load average near 100.) The first measurement, before 0219, over 117 files: 990
instructions a node for the lean tape, 6,978 for tree-sitter's.

The lean tape costs a seventh of tree-sitter's in instructions, about a tenth in cycles.

(The keyword lookup below was replaced by 0219's switch: one probe a word, in the lexer.) The keyword lookup was a bucket by length and first byte, its words compared by their first eight
bytes, and cached for the last two words (a two-token look goes between them); a token read ahead
and given back is not lexed again; the arena is sized from the text. The first build searched all
517 words at each question and allocated in the automata walk: 1,769 instructions a node.

## Open

- **The refusals listed above.** Each is a file tree-sitter reads instead; none is a wrong tape.
  The clean files' refusals are a long tail of one or two files each; the largest group is
  PostgreSQL's operator definitions (7 files), extension internals.
- **The cost.** The lean tape is a seventh of tree-sitter's in instructions, where the other lean
  parsers reach a twentieth. tree-sitter-sql's trees are fine-grained (every keyword a leaf), so
  tree-sitter's own cost a node is low; the parser's remaining cost is its keyword questions (several a token, by
  call) and the automata walk a node. A generated keyword dispatch (one table lookup a token,
  giving the set of keywords the word is) would replace most of the questions.
