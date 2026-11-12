# 0058: What a rule sees of guests, normalized strings, and tape operations

Status: accepted (2026-11-07). Built for `.py` and `.rs` corpus runs; see [What was built](#what-was-built).

## Intro

Decision 0055 made embedded code into *guest documents*. A guest is a document of the batch: a Python string
annotation, or SQL in an `execute("…")` call. It is parsed with its own grammar, and its nodes are `node` rows like a
file's. Four questions were left open:

1. **What does a rule see?** Guests are documents of the batch, so `kind(n, "call")` also matched calls inside a
   parsed string annotation. A SQL guest's nodes were in the same tables as the file's.
2. **What is a SQL guest's kind called?** Kinds were resolved in the run's grammar. In a Python run,
   `"object_reference"` named no Python kind, so no rule could find SQL nodes.
3. **What text is parsed?** SQL in `"SELECT \"name\" FROM t"` parses only once the escapes are gone. Its nodes still
   need places in the file.
4. **How does a rule make a tree from a computed text** (`@parse`), and how does it build an edited tree (`@splice`)
   or get a tree's text back (`@unparse`)?

This record answers all four. A fifth point, the shared tape cache's race, is written down as an open item (O95);
it is not fixed here.

## Decision

### 1. What a rule sees: one bound on the document key

A document's key says what kind of document it is:

| keys | documents | shown to a rule that does not name a guest |
|---|---|---|
| `1 .. 2^30` | files | always |
| `2^30 .. 2^30 + 2^29` | guests in the run's language (a string annotation in a Python run) | unless `.guests off` |
| `2^30 + 2^29 .. 2^31` | every other document: guests in another grammar, values from `@parse` and `@splice` | never |

A node's id is its document's key `<< 32`, plus its index. So "shown" is one comparison on a value the join already
has: `n < bound << 32` for a node column, `d < bound` for a document column. No row is copied, no table is filtered
first, and a run that makes no guests is not changed at all.

The lowering decides, per rule, which variables are *scoped*. A scoped variable is one the rule reached through a
guest relation. It gets no bound. Every other location variable in a positive base atom gets one:

```
// Unscoped: n gets `n < bound << 32`. In a Python run this sees the file and its Python guests.
calls(n) :- kind(n, "call").

// Scoped: g comes from `guest`, so the node atom over g, and n with it, is the guest's.
sql(n) :- guest(g, _, _, "sql"), node(g, n, "object_reference", _, _).
```

A variable is scoped when it is:

- in a guest column of a guest relation: `guest`, `parsed`, `sub_tape`, `doc_root`, `doc_role`, `tape_of`,
  `host_node`, `parse_of`, `splice_of`;
- in the same base atom's location columns as a scoped variable, since one atom's locations are in one document;
- in a derived relation's column that some rule fills from a scoped variable. This is a fixpoint over the
  program's rules.

Wildcards in an unscoped atom's location columns become fresh variables, so they can be bounded too. An
aggregate's atom is its own scope. An aggregate over a base atom reads that atom through a helper rule
(`rr_shown_agg<k>`) that carries the bounds and keeps every column, so a count still counts the atom's rows.
`kind`, `docof` and `precedes` are views over `node` that the lowering writes as rules. They are read as base
relations, and their own rules are left whole.

`.guests off` lowers the bound to files only. It no longer stops a library's `embed`s from being made: a guest is a
fact either way, and `guest`, `doc_role` and `parsed` still reach it. This is what 0055 §5 asked for.

The transform runs only when some program in the run derives a request relation (`embed`, `embed_value`, or a
rule's own demand). It is applied to each program before a combined run renames them. The requests are shared
across a combined run's programs (`guests::SHARED`), so there is one batch and one set of guests. Each program's
own rules are still bounded by its own `.guests` setting.

### 2. A guest's kinds are its grammar's

A scoped variable also knows its grammar when the guest relation names one: `guest(g, _, _, "sql")`,
`parsed(s, "sql", r)`, or the tape of `@parse("sql", …)`. A kind or field constant on such a node is resolved in
that grammar. The lowering writes it as `"sql:object_reference"`, and the constant resolver looks the name up in the
SQL dialect's catalog. A rule may also write the qualified name itself.

A kind's value already carries its grammar (`tag * KIND_STRIDE + code`, and a field's `tag << 16 | code`). Output
therefore decodes each kind and field value with the catalog of its own tag. A SQL kind prints as SQL's name, even
in a Python run.

A rule may read a derived relation with a scoped variable: `sel(n) :- kind(n, "select")`, read as
`guest(g, _, _, "sql"), node(g, n, _, _, _), sel(n)`. That relation is then *specialized*. It is copied as
`rr_in_<grammar>_<column>_<relation>`, and the copy's rules start with that head column scoped in the grammar. The
copy's kinds are therefore the guest's, and its rows are not bounded away. The run's own reads keep the original. A
kind set that `nearest`, `inside` or `has` reads over a scoped node is copied the same way, as
`rr_in_<grammar>_<set>`, with each kind qualified. This runs to a fixpoint, because a copy can read other
relations.

```
.decl clauses(kind: symbol)
clauses("relation"). clauses("function_definition").
// Over a SQL node, the set is read as rr_in_sql_clauses: {"sql:relation", "sql:function_definition"}.
enclosing(x, k) :- guest(g, _, _, "sql"), node(g, n, "object_reference", _, _), text(n, x),
                   nearest(n, a, clauses), node(_, a, k, _, _).
// Over a file's node, it is the run's: Python's function_definition.
outer(k) :- kind(n, "call"), nearest(n, a, clauses), node(_, a, k, _, _).
```

An unscoped qualified kind (`kind(n, "sql:object_reference")` with nothing reaching a guest) still gets the bound,
so it matches nothing in a Python run. Naming a grammar is not a way to see hidden documents; reaching them through
a guest relation is.

### 3. Normalized strings

`str(n, v)` is a language's decoded string: the text the program holds at run time, with escapes resolved. A guest
made from a string is parsed from this text. A rule that computes a text from `str` and parses it uses the same
text (§4).

The normalizer (`facts::vtab::syntax::normalized`) returns that text with a *source map*. The map is a list of
stretches, each copied byte for byte from the source or standing whole at one escape:

```
source   "SELECT \"name\" FROM t"
text      SELECT "name" FROM t
stretches [SELECT ] copied · ["] at \" · [name] copied · ["] at \" · [ FROM t] copied
```

The scanner only cuts the body into copied runs and escapes. It knows escape lengths for Python, Rust and the ECMA
languages. Each escape is decoded by the language's own decoder, the one `str` uses. The joined text must then equal
that decoder's value for the whole literal. When it does not, there is no map, and the guest stands at the literal
as a whole, as before. Two cases fall back this way: a rule the scanner does not know, and Python's universal
newlines. So the normalizer can make a place less exact, but it can never change the text that is parsed.

A guest placed by a map (`Placement::Mapped`) gives each node the source bytes of its first and last stretch. A
copied stretch keeps the file's columns. An escape's output stands at the escape. A concatenation (`"a" "b"`) is one
map over its parts. Maps compose, so a string inside a parsed annotation is placed through both.

### 4. Tape operations: `@parse`, `@unparse`, `@splice`

The three are functions in a rule's body. Each lowers to a relation that the engine fills on demand:

```
t = @parse("sql", q)      // parse_of(q, "sql", t): t is the tape of q parsed as SQL
x = @unparse(t)           // unparse_of(t, x): x is t's text
t3 = @splice(t, n, t2)    // splice_of(t, n, t2, t3): t with node n's text replaced by t2's, parsed again
```

A tape is its content key, as in 0055. The tape's document is reached with `tape_of(d, t)`, and its nodes with
`node(d, …)`:

```
// The tables a call's SQL reads. q is the string's normalized text.
tables(x) :- kind(c, "call"), field(c, "arguments", a), child(a, s, _), str(s, q),
             t = @parse("sql", q), tape_of(d, t), node(d, n, "object_reference", _, _), text(n, x).
```

- **`@parse` and `@splice` are demands**, like `parsed` in 0055. The lowering writes `rr_parse_req(at, g, x)` and
  `rr_splice_req(n, t2)`. `at` is a location from the rule's context: the document whose rule asked. A demand
  reads only the literals joined to its inputs, and never the ones that read its result. A demand of constants
  (`@parse("sql", "SELECT 1")`) is made once per document. A demand may ask for more than the rule reads, never
  less.
- **The demands run per chunk, in the guest rounds.** The embed slice is cut from the whole program, not only from
  its file-local part. A value is made from its chunk's documents, so a demand whose context the file-local split
  would run once over all files (`q(_, x)`, or a value's nodes) is still asked in the chunk. A splice of a node of
  a parsed value is made in the round after the parse.
- **Value rows place nothing.** `parse_of`, `splice_of` and `unparse_of` have no location column, and a chunk makes
  their rows for itself. The file-local split therefore does not count them as reads of any file's rows.
- **Each made tape is a value document** in the hidden key range. Its key is hashed from what made it: (grammar,
  text) for a parse, (node, tape) for a splice. Its role is `value`, and it has no host. Its lines and columns are
  its own text's.
- **A value belongs to the files that asked for it.** A chunk makes one value per (grammar, text) or (node, tape),
  whichever of its files asked first. The value's rows are attributed to every file of the chunk that asked. The
  memo stores them under each of those files, so each file reads them back alone. In the file-local split, a value's
  tape stands with the one atom that gave its text: `q(s, x)` in `q(s, x), t = @parse("sql", x)`. `tape_of(d, t)`
  puts d there too, and a spliced tape stands with its node. A rule that reads a value's nodes is therefore as
  file-local as the rule that gave its text. This holds even when simplification leaves that atom as
  `q(_, x)`. If two atoms give one text, the text joins two documents, and the rule stays cross-file.
- **`@unparse` needs no demand.** `unparse_of` has a row for every guest and value document when a rule reads it.
  A wrapped fragment's text is the expression inside its `( … )`. A file's own tape has no row: unparsing a whole
  file is not asked for.
- **`@splice`'s grammar** is the grammar of node n's document. Its text is that document's unparsed text, with n's
  bytes replaced by t2's unparsed text.

## Why

- **One bound, not a filtered copy.** The spike kept embedded regions as their own tape rows and never dropped the
  candidate-file semijoin for multi-language files (S070). Here the document key plays that role: hiding is a range
  test on a column the join reads anyway. Copying the shown rows per program would be the per-query materialization
  the ledger rejects (lessons 6 and 27).
- **Names resolve in the query's grammar.** The spike found that resolving a query's names in the primary grammar
  gave a Rust query 0 matches against 71 (S133). Its kind codes are global, and that removed a 13% tax (S071). A
  guest's grammar is known where the rule reaches it, so the constant is resolved there. Kind atoms already carry
  their tag, so decoding needs no extra state.
- **Demand-driven, file-local.** Values are made only for the texts a rule asks for (lesson 13). They are made in
  the chunk that asked, so the single-file partition holds (lesson 20). Each text is parsed once, through the tape
  cache (lesson 25).
- **The normalizer cannot be wrong about the text.** It is checked against the decoder `str` already uses. Two
  decoders that can disagree would make `str(s, q), @parse("sql", q)` and the library's embed of `s` parse different
  texts.

## Prior work consulted

- Spike design index §21: S070 (embedded segments as tapes), S133 (names in the query's grammar), S071 (global
  codes), S135 (vocabulary per language).
- Lessons ledger: 6 and 27 (no per-query materialization), 13 (demand-driven extraction), 14 (each file owns its
  rows: why value documents are not memoised), 20 (partition by file), 25 (the tape cache).

## Departs from

- 0055 §5 and its "What was built": `.guests off` now hides guests instead of stopping embeds.
- 0055's placement of escaped and concatenated strings at the literal: these are mapped now.

## What was built

- `logic::plan::corpus::hiding`: the scope analysis, the bounds and grammar qualification. `plan::lower::Constants`
  resolves `"label:name"`, and `plan`'s output decodes kinds and fields by their tag.
- `facts::vtab::syntax::normalized` (Python, Rust, ECMA), `Placement::Mapped` and its composition. Guests use it
  for every string kind.
- `@parse`, `@unparse` and `@splice` in `rr::lower` (`demand`, `value_demand`). Value documents and splices are
  in `corpus::guests`, and `parse_of`, `splice_of` and `unparse_of` are in `syntax::guests`. The embed slice is
  cut from the whole program (`corpus::embed_slice`), and the split's `CHUNK_VALUES` lists the value relations
  (`typing::file_local`).

Tests (`logic/rr/tests.rs`, and the normalizer's own):

- `embeds_declare_guests_and_guests_off_leaves_them_out`: shown Python guests, and `.guests off` hiding them;
- `sql_in_python_is_an_sql_tape_hidden_from_unscoped_rules`: a SQL tape, scoped SQL kinds and how they print,
  hiding, and escaped SQL placed through the map;
- `string_annotations_are_guests_at_their_host_lines`: escaped and concatenated annotations at their exact
  columns;
- `parse_splice_and_unparse_make_value_documents`;
- `value_documents_are_memoised_with_the_files_that_asked`: two files sharing a text, a second run read whole from
  the memo with equal rows (O96);
- `combined_programs_see_guests_by_their_own_policy`: `.guests off` beside a program without it, in one run (O97);
- `kind_sets_over_a_guest_are_its_grammars`: one kind set read over a SQL node and a Python node (O98);
- `normalized::tests`.

Amended (2026-11-07, O110): a variable read from a derived column is scoped when some rule fills that column from a
guest, but it is only *definitely* a guest's node when every rule does. `calls.rr`'s `tv` holds host and guest
nodes alike, and `tbv(s, x, k, v) :- live_asg(s, x, r), tv(r, k, v)` read only the annotation copy of `live_asg`:
every host assignment's builtin value was lost (four receivers of `pkg/typed.py` Unknown). The analysis now tracks
`definite` (a guest entry, a copy's seed, or a column only definite variables fill, to a fixpoint). Specializing an
atom and qualifying its kinds need a definite variable. A rule whose variable may be a guest's is split once: it
stays as it is for the host's nodes, and a clone with the variable seeded as the guest's reads the copies
(`a_maybe_guest_variable_keeps_the_host_rows_and_clones_for_the_guest`).

## Open

- O95 (the shared tape cache's race) is closed: a source and its tape are stored in one transaction.
- O96, O97 and O98 are closed (see the tests above).
- O99: guests and hiding on the 9,010-file repository, in CPU time.
- O100: normalized strings as native operations that rules read, and later as a transducer side program.

## Built: hiding's copies that are another relation, merged (2026-11-12)

**What was wrong.** At O125 step 07, hiding's `rr_in_annotation_*` copies were 419 relations holding 751 k rows,
and 39% of the step's join work read or wrote one. Two kinds were pure duplication in the program:

- **Copies whose rules are another relation's**, renamed: context helpers (`rr_ctx12` as `rr_ctx11`) and copies of
  copies (`rr_in_annotation_0_annotation_2_wrap`).
- **Repeated rules.** Hiding's O110 clones were made again, so 407 of the cross part's 1,483 rules were exact
  repeats of another rule. Each was planned and run on its own, though it derives nothing new. A repeat also kept
  its relation apart from its twin, since `simplify`'s congruence compares rule lists.

**What runs now.** After hiding and combining, and again after the joins are pushed, the program drops repeated
rules and merges identical relations (`simplify::merge_identical`), in turn until neither changes anything
(`corpus::merge_to_fixpoint`). `RERAIL_SAME=off` turns it off. The test
`the_combined_program_has_no_relation_twice_and_no_rule_twice` guards the combined `calls` program; with the
switch off it finds 39 classes.

**Measured at O125 step 07** (outputs equal to a full run's):

| | Before | After |
|---|---:|---:|
| Cross-part rules | 1,483 | 1,053 |
| Strata (run / all) | 775 / 829 | 596 / 645 |
| Session CPU, step 07 | 17.4 s | 12.8 s |
| Instructions, whole replay (cold run at 06 included) | 695.1 G | 621.9 G (first version) |
| Peak memory | 1,152 MB | 1,049 MB |

**Not merged: the copies that equal their originals only on this corpus.** `fret` reads `retval` and its copy,
while `rr_in_annotation_0_fret` reads the copy only. The two hold the same rows because no hidden guest's rows
reach them here, not because their rules agree. Removing them needs the copy to be a filtered view of its original,
or the program specialised by a phase fact saying the run has no hidden guest documents. Both are open.

