# 0061: `.rr` files are a dialect with a tape

Status: built (2026-11-07)

## Intro

Rules are written in `.rr` files. Until now the engine read them only to run them: the parser
turned the text into statements, lowering turned the statements into a program, and nothing of
that was kept or could be queried.

This record makes `.rr` a language like Python or Blade. A `.rr` file gets a tape, so:

- **Rerail can check rules with rules.** That covers its own library (`docs/rule-syntax/lib`) and
  rules users write. For example, it can flag a rule that reads a file outside the repository, or
  a regex that can backtrack without bound.
- **A compiled program can be stored as facts.** A rule file is a source in the content store. Its
  tape and the facts derived from it are cached, memoised and collected like any other file's.

This record is the spec: what a `.rr` tape holds (kinds, fields, spans, extras, errors), the
text facts it gives, and how it is produced.

A small example. This file:

```
// Calls of eval.
.decl bad(n: node)
bad(c) :- kind(c, "call"), !safe(c).
```

has this tape. Named nodes are shown with their fields; anonymous tokens are in quotes.

```
source_file
├─ comment                                  "// Calls of eval."
├─ declaration
│  ├─ ".decl"
│  ├─ name: identifier                       bad
│  └─ columns: column_list
│     ├─ "("
│     ├─ column: column
│     │  ├─ name: identifier                 n
│     │  ├─ ":"
│     │  └─ type: type
│     │     └─ name: identifier              node
│     └─ ")"
└─ clause
   ├─ head: head
   │  ├─ relation: identifier                bad
   │  └─ arguments: argument_list
   │     └─ argument: variable               c
   ├─ ":-"
   ├─ body: conjunction
   │  ├─ literal: atom
   │  │  ├─ relation: identifier             kind
   │  │  └─ arguments: argument_list
   │  │     ├─ argument: variable            c
   │  │     └─ argument: string              "call"
   │  └─ literal: negation
   │     ├─ "!"
   │     └─ operand: atom
   │        ├─ relation: identifier          safe
   │        └─ arguments: argument_list …
   └─ "."
```

## Decision

### 1. A dialect: `rr`

- **Registry.** `GrammarDialect::Rr`, with label `rr`, the extension `.rr` and a tape tag of its
  own. Like Blade, it has no tree-sitter grammar. `KindCatalog` gains `OwnKinds::Rr`, read from
  `logic::rr::tape`'s `KIND_NAMES` and `FIELD_NAMES`.
- **Numbering.** Kind ids start at 1: named kinds first, then anonymous tokens from
  `FIRST_ANONYMOUS`. ERROR shares every grammar's id (`u16::MAX`), and field ids start at 1.
  Adding a kind or field bumps the grammar version, so stored tapes are made again.
- **No supertypes** in the first version. The kinds below are already grouped by field
  (`literal`, `argument`), which is what a supertype would give.

### 2. The tree

Every node spans the bytes from its first token's start to its last token's end. Children are in
source order. Each named node below lists its fields. A field marked `*` repeats.

**The file and its statements**

| kind | fields | written |
|---|---|---|
| `source_file` | (statements and comments) | the whole file |
| `include` | `path`: `string` | `#include "python/ast.rr"` |
| `declaration` | `name`, `columns`, `modifier`* | `.decl r(a: node) ordered by earlier` |
| `edit_declaration` | `name`, `columns` | `.edit fix(n: node)` |
| `functor_declaration` | `name`, `columns`, `return_type`, `modifier`* | `.functor f(x: symbol): number` |
| `type_declaration` | `name`, `definition` | `.type T <: symbol` |
| `io_directive` | `relation`*: `io_relation` | `.input a, b(IO=json, filename="x")` |
| `io_relation` | `name`, `parameters` | `b(IO=json, filename="x")` |
| `lattice_declaration` | `name`, `base`, `operation`* | `.lattice L<T> { Lub -> @f }` |
| `lattice_operation` | `name`, `value` | `Lub -> @f` |
| `pragma` | `argument`*: `string` | `.pragma "cross-rounds" "4"` |
| `violation_declaration` | `name`, `columns`, `code`, `tag`, `message`, `fix`*, `suppress`*, `options` | `.violation V(at: loc) code "E1" "message"` |
| `code_column` | `column`, `codes` | `code {c} in "E[0-9]+"` |
| `tag_column` | `column` | `tag {t}` |
| `fix_line` | `relation`, `description` | `fix safe rewrite "Use x"` |
| `suppress_line` | `tool`, `rule`*: `string`, `over` | `suppress noqa "E501" over {n}` |
| `policy_document` | `document`: `json_object` | `.policy { … }` |
| `setting` | `name`, `argument`* | `.mode block`, `.scope include "**/*.py"` |
| `clause` | `head`*, `body`, `edit`* | `a(x), b(x) :- c(x) => e(x).` |
| `head` | `modal`, `relation`, `arguments` | `must r(x, y)` |

- **Directive keywords are one anonymous token.** `.decl`, `.input`, `.output`, `.violation` and
  so on span the `.` and the word together. `#include` is one token too.
- **What each field holds.** `name`, `relation`, `tool`, `base`, `column`, `over` and `modifier`
  are `identifier`s. A `modifier` is a `number` where the word is a number (`widen 64`).
  `columns` is a `column_list`, `type` and `return_type` are `type`s, and `message`, `codes` and
  `description` are `string`s.
- **`definition` and `parameters`** hold the text as written, as one `raw_text` leaf: a type's
  definition, or the parameters inside an I/O relation's parentheses.
- **A setting's arguments** are its words (`identifier`, `string`, `number`, `json_object`), in
  order. Its punctuation is anonymous.
- **`code` is either a `code_column` or a `string`** (`code "E501"`). `modal` is the anonymous
  token `must` or `may`.

**Bodies and literals**

| kind | fields | written |
|---|---|---|
| `conjunction` | `literal`* | `a(x), !b(x), x != y` |
| `disjunction` | `alternative`*: `conjunction` | `a(x) ; b(x)` |
| `atom` | `relation`, `arguments` | `kind(n, "call")` |
| `negation` | `operand`: `atom`, `pattern_match` or `group` | `!safe(n)` |
| `comparison` | `left`, `operator`, `right` | `x <= y` |
| `pattern_match` | `subject`, `pattern`: `code_literal` | ``n ~ `eval($X)` `` |
| `group` | `body`: `conjunction` or `disjunction` | `(a(x) ; b(x))` |

- **A body is always a node.** It is a `conjunction`, or a `disjunction` when the body has a
  top-level `;`. A conjunction of one literal is still a `conjunction`, so a rule over literals
  reads one shape.
- **`operator` is the anonymous token itself** (`=`, `!=`, `<`, `<=`, `>`, `>=`), so
  `op(cmp, "<=")` and `op_at(cmp, 0, "<=")` read it.

**Terms**

| kind | fields | written |
|---|---|---|
| `variable` | | `x`, `snippet` |
| `wildcard` | | `_` |
| `number` | | `42`, `-1` |
| `float` | | `0.5` |
| `string` | | `"a\nb"` |
| `code_literal` | | `` `eval($X)` ``, ``` ``f(x)`` ``` |
| `json_object` | | `{ "kind": "call" }` |
| `call` | `function`: `identifier`, `arguments` | `cat(a, b)`, `ast(n, { … })` |
| `functor_call` | `function`: `identifier`, `arguments` | `@replace_all(s, "a", "b")` |
| `binary_expression` | `left`, `operator`, `right` | `a - b * 2` |
| `unary_expression` | `operator`, `operand` | `-f(x)` |
| `parenthesized_expression` | `expression` | `(a + b)` |
| `aggregate` | `operator`, `value`, `key`*, `separator`, `body` | `join x by i sep "," : { … }` |
| `named_argument` | `name`, `value` | `NAME: v` |
| `argument_list` | `argument`* | `(a, "b", _)` |
| `column_list` | `column`* | `(n: node, s: symbol)` |
| `column` | `name`, `type` | `n: node` |
| `type` | `name`, `argument` | `flat<symbol>`, `shortlex_min<>` |

- **A negative literal is one token.** `-1` is a `number` spanning the sign. `-x` is a
  `unary_expression`. The parser already reads them this way.
- **An aggregate's `operator`** is its anonymous word: `count`, `min`, `max`, `sum`, `mean` or
  `join`. Its `body` is a `conjunction` or `disjunction`, as a clause's is.
- **Leaves are `identifier`, `variable`, `wildcard`, `number`, `float`, `string`, `code_literal`,
  `json_object`, `raw_text` and `comment`.** A `json_object` is one leaf here: its contents are a
  guest (section 5).

**Anonymous tokens.** Every punctuation token the lexer knows is an anonymous leaf: `:-`, `=>`,
`(`, `)`, `,`, `.`, `;`, `:`, `{`, `}`, `@`, `!`, `~`, `<:`, `->`, `[`, `]`, `|`, `^`, `&`, and
the operators. So are the directive keywords and the words the grammar reserves where they
appear: `must`, `may`, `code`, `in`, `tag`, `fix`, `safe`, `suppress`, `over`, `options`, `by`,
`sep` and the aggregate operators. Anonymous tokens are children like any others, so `op(n, s)`
lists them. They are not counted by `child(p, c, i)`, which counts named children only.

### 3. Comments are extras

`// …` and `/* … */` are `comment` nodes, flagged extra in their code (ledger lesson 24, decision
0025). A comment is a child of the innermost node whose span contains it, at its place in source
order. A comment between two statements is a child of `source_file`. Patterns and `child` skip
extras.

The lexer keeps comment spans beside the tokens; it no longer drops them.

### 4. Errors

The parser no longer stops at the first error in a tape.

- **One bad statement is one ERROR node.** It spans from the statement's first token to the point
  where parsing resumes, and holds no children.
- **Resuming.** Parsing resumes at the next token that can only start a statement: a `#include`, a
  `.` followed by a word at the start of a line, or the token after the next `.` that ends a
  clause.
- **An unterminated string, backtick literal or comment** runs to the end of the file, as one
  ERROR.
- **`syntax_error(d, line, msg)`** has one row per document, at the first error, with the message
  the parser gives today.

Running a program still refuses a file with an error, exactly as now. The tape exists so a rule
can report the error.

### 5. Text facts and guests

The `core.rr` relations over a `.rr` tape:

| relation | holds for |
|---|---|
| `text(n, s)` | every node: its source text |
| `name(n, s)` | `identifier` and `variable` |
| `str(n, v)` | `string`: its unescaped value (`syntax::unescape`); `code_literal`: its body without fences |
| `value(n, v)` | `string` as `str`; `number` and `float` normalised |
| `comment(n, s)` | `comment`: its text without `//` or `/* */` |
| `op(n, s)`, `op_at(n, i, s)` | a node's anonymous tokens, in order |
| `syntax_error(d, l, m)` | a file the parser refuses |

The code inside a `.rr` file is in other languages, and each becomes a guest through decision
0055's `embed` rules, in a library file `lib/rr/embeds.rr`:

```
// A pattern is code of the program's language (`.language python`), with metavariables.
// (`program_language` would read the file's `.language` setting.)
embed(d, p, g, "pattern") :- node(d, m, "pattern_match", _, _), field(m, "pattern", p),
    program_language(d, g).
// A JSON object is a JSON document: an `ast` matcher, a policy, a violation's options.
embed(d, j, "json", "fragment") :- node(d, j, "json_object", _, _).
// The first argument of `match` is a regex.
embed_value(d, s, "regex", "fragment", v) :- node(d, a, "atom", _, _), field(a, "relation", r),
    name(r, "match"), field(a, "arguments", l), child(l, s, 0), str(s, v).
```

The grammars these name (`json`, `regex`, a pattern grammar per language) are separate work: each
row is made once its grammar is registered. Until then the rows exist and make no guest.

### 6. How it is produced

- **One parser.** `logic::rr::syntax::Parser` builds its statements and the tape in the same
  parse. Each production that makes a node opens it at its first token and closes it after its
  last, with the node's kind and field. Leaves come from the tokens. There is no second parser, so
  the tape cannot drift from what runs.
- **Encoding.** The parser's nodes go through `TapeImage::finish`, as Blade's do (`encode_blade`).
  Comments are merged in at their places.
- **Where tapes come from.** A `.rr` file in a corpus run is a document like any other: stored by
  content, taped, memoised. So a run over a repository that has rules checks them with the
  repository's other files.
- **Rules about rules** live in `lib/rr/`. For example (`derived`, `input` and `read` would be
  `lib/rr/` relations over the tape, written with these rules):

```
// A relation that is declared but never derived, never an input and never read.
.violation unused_relation(at: loc) code "RR001" "relation declared but never used"
unused_relation(n) :- node(_, d, "declaration", _, _), field(d, "name", n), name(n, r),
    !derived(r), !input(r), !read(r).

// An input read from outside the repository: an absolute path or one that climbs out of it.
.violation outside_input(at: loc) code "RR101" "input file outside the repository"
outside_input(p) :- node(_, p, "io_relation", _, _), field(p, "parameters", t), text(t, s),
    match("filename=\"(/|\\.\\./)", s).
```

### 7. What a tape changes for compiled programs

This record makes the tape. It does not yet move lowering onto the tape. The next step is for the
statements a program is lowered from to be read from its tape, so that a rule file's compiled IR is
a set of facts derived from a stored tape and memoised per source. That would replace the
in-process caches of decision 0060 with the store's. That step gets its own decision.

## As built

- **Registry.** `languages/rr.json`: dialect `Rr`, tape tag 13, parser `rr`, grammar `native:rr`, feature `lang-rr`
  (in `all-languages`). Every capability (scope, values, arguments, calls, format arguments) is `refused`. The
  family's adapter (`language/adapter/rr.rs`) answers "nothing known" behind those refusals.
- **Tree.** `logic/rr/tape.rs` holds the kinds, the fields and the tree. `syntax::Parser::taped` builds the tree in
  the same parse as the statements. The lexer keeps comment spans and makes a `Bad` token where it cannot read.
  A parse without a tree gives the same statements and errors as before.
- **Tape.** `TapeImage::encode_rr` goes through `finish`, as Blade's does. The grammar build id hashes
  `syntax.rs` and `tape.rs`.
- **Facts.** `core_rr` reads a `.rr` tape: `name` of a `variable` too, `str` and `value` by the engine's own
  unescaping (`rr::string_value`), and a backtick literal's `str` without its fences (`rr::code_body`).
  `syntax_error` carries the message running the file gives (`rr::syntax_error`). A file that does not parse is
  withheld, as any other language's is: it gives its `syntax_error` row only.
- **Corpus.** A program with `.language rr` reads the repository's `.rr` files (`corpus_language`). Other runs
  read none: a run still reads one language (O105).
- **Rules about rules.** `packs/rr/rr-input-outside-repository.rr` (RR101) flags an `.input` read from an absolute
  path or from `../`. `lib/rr/embeds.rr` has the guest rows, with `program_language` from the file's `.language`
  setting. They make no guest until `json`, `regex` and the pattern grammars are registered.
- **Tests** (`logic/rr/tape_tests.rs`):
  - the example above, tree for tree;
  - the shapes of terms, bodies and directives;
  - recovery, with an ERROR per bad statement and an unterminated string running to the end of the file;
  - every `.rr` file under `docs/rule-syntax` (1,130 files): an ERROR exactly where running it fails, spans
    that nest, and leaves and comments that cover every byte except whitespace exactly once;
  - an encode round trip through the catalog;
  - RR101 over real files, and the text facts.
- **Cost.** The replay is unchanged (3988 303 298 0, 8.2 s user + 1.9 s sys at a load of 84): a parse without a
  tree builds none.

## Prior work consulted

- **Lessons ledger 1:** a compact tape is the source of truth, not one row per node. Rule files
  get the same tape, so their facts derive from it.
- **Lessons ledger 24:** comments are extras, flagged per kind, so wildcards skip them (section 3).
- **Decision 0025 (v9np):** one tape format, a per-tape table of codes, and kinds and fields from
  the dialect's catalog. Nothing in the format changes for `.rr`.
- **Decision 0055:** code in another language is a guest made by `embed` rules (section 5).
- **Blade (`blade::forte`):** a dialect Rerail parses itself, with its own kind table and ERROR
  leaves. `.rr` follows it.
- **ast-index-spike, history index L919-920:** the language is part of every cache key. A `.rr`
  tape has its own tag and grammar version.

## Open

- **Patterns over `.rr` code** (``n ~ `…` `` with `rr` as the language) need `$X` metavariables in
  the lexer. They are not part of the first version.
- **Supertypes** (`_statement`, `_literal`, `_term`), if rules want them.
- **Lowering from the tape** (section 7).
