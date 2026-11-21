# 0082: Embedded grammars: names in the registry, one SQL grammar, global guests in a later wave

Status: accepted (2026-11-08). §1 built; §2 is the registry as it stands; §3 decided, not built (global embeds
are not built).

## Intro

Decision 0055 lets a library declare guests: `embed(host, at, grammar, role)`, `sub_tape(at, grammar, t)` and
`parsed(s, grammar, root)` name a grammar, and the engine parses the text at `at` with it. It left three questions
open:

1. Where the grammar names live: the language registry, or `core.rr`.
2. Which SQL dialects get grammars.
3. Whether a global embed's guests become documents in a later run.

```text
embed(d, s, "sql", "fragment") :- call_arg(c, 0, s), callee(c, f), sql_parameter(f, 0), docof(s, d).
parsed(s, "annotation", r)            // Python's `x: "List[int]"`, a Python expression
```

Until now, the dialect labels came from the registry (`languages/*.json`), but `annotation` was a string in
`corpus::guests`. A name nothing knew made no guest and said nothing.

## Decision

### 1. Grammar names live in the language registry

A grammar name is either:

- a supported dialect's label (`python`, `sql`, `typescript`, …), as before; or
- a **fragment**, declared in the entry of the language it parses.

```json
"fragments": [{ "name": "annotation", "dialect": "python", "start": "expression" }]
```

A fragment is its dialect parsed from a start symbol. `expression` is the one start symbol: the text inside a
written `( … )`, whose root is the expression inside (0055 §2's wrapper). Tree-sitter has no start symbols, so a
new one is a wrapper the engine knows, added to the closed list beside `expression`.

- `native/build.rs` validates fragments: a name is unique among every dialect label and fragment, its dialect is
  the entry's own, and its start is known. It generates `FRAGMENTS`. The host's schema
  (`scripts/generate/languages/registry.ts`) accepts the field and emits nothing for it.
- `corpus::guests` reads `language::fragment`; nothing names `annotation` in Rust any more.
- Lowering refuses a constant grammar the registry does not have, in an `embed` or `embed_value` head or a
  `parsed` or `sub_tape` atom (`rr::lower::grammar_names`): "no grammar "x"; a grammar is a language registry
  dialect's label or fragment". A dialect this build leaves out is still a name: the rule is valid, and its
  guests are not made in this build.

`core.rr` keeps describing the names a library may use, and owns none of them. The registry is already the
single source of truth for both the engine and the host (`languages/README.md`); a second list would drift.

### 2. One SQL grammar, `sql`

The registry has one SQL dialect, `sql`: the vendored tree-sitter-sql grammar, with SQLFluff's semantics, for
PostgreSQL, SQLite, MySQL and ANSI files alike (`language/adapter/sql.rs`). An embedded query is parsed with it.

`postgres`, `sqlite` or `mysql` are not grammars. Where they differ in a way that rules see, it is in meaning:
the catalog a name resolves against, how a literal reads (0058 §3, `value/sql.rs`). That is a fact about the guest
(a role, or a setting the library writes beside the `embed` row), not another tree. A dialect gets its own
grammar only when its syntax needs one that `sql` cannot parse, such as T-SQL or BigQuery. It is added to
`sql.json` with its own vendored grammar and tape tag, and its label is then a grammar name by §1.

### 3. A global embed's guests are documents in a later wave of the same run

A global embed reads another file's facts, so its guests exist only after the cross-file strata it reads. When
global embeds are built, those guests become documents **within the run**:

1. The cross-file part derives the embed rows.
2. The new guests are a batch of documents: parsed (tapes cached by text, 0055 §4), and their file-local part
   evaluated as any file's.
3. Their rows enter the cross-file part as changed inputs, which 0077 maintains by deltas.
4. This repeats while a wave makes new guests, to 0055's depth bound.

They do **not** become documents of the *next* run from the store, which is what 0055 asked. A run's findings
would then depend on the run before it: a one-shot run and an incremental one would disagree for one run after
every change. The target architecture says recursion runs as waves (03-architecture.md, point 8), and a guest
made from cross-file facts is recursion between the two parts. The store still saves the work: the guests' tapes
are cached by text, and their file-local rows are memoised by content (0052), so an unchanged guest costs a
lookup in the next run's wave.

## What the prior work says

- **Decision 0055 §2, §4:** grammar names are a dialect and a start symbol; a global embed runs in the
  evaluation, after the strata it reads. *Agrees*; §1 says where the names are written, §3 what follows the
  evaluation.
- **History index §2.20(d):** one-shot must equal incremental. *Agrees*: this is §3's reason against next-run
  visibility.
- **Target architecture, point 8:** recursion runs as multiple waves; bounded recursion inside one file stays in
  the dispatch. *Agrees*: global guests are a wave.
- **Lessons ledger #14:** each file owns its rows. A guest is its askers' files' (O96), so its rows go with them.
- **Language registry README:** one source of truth for engine and host. *Agrees*: §1 adds to it, not beside it.

## Open

- Global embeds (§3) are not built: an `embed` rule whose body reads another file's facts is still the embed
  slice's, made per chunk.
- No library writes an SQL setting yet (§2); the SQL value semantics are PostgreSQL's and SQLite's where they
  agree.
- `string_annotation` (Ruff's table of the positions that take a type as a string) is still not built (0055).
