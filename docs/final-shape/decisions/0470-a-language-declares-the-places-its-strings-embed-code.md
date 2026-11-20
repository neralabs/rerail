# 0470. A language declares the places its strings embed code

Status: decided (2026-11-19), by the user: choice 2 of three for "a string we know is HTML": a language declares
the strings at known places that hold code. Comes after 0466 (a fragment starts at a node kind). Continues 0431 §4
(the grammar decides what a file holds as programs).

## Intro

Some strings in a program hold code of another language, and the place says so. In Python:

```python
from markupsafe import Markup
from django.utils.html import format_html

banner = Markup("<b>Welcome</b>")              # the argument is HTML
link = format_html('<a href="{}">{}</a>', u, t)  # the first argument is HTML
```

0431 made two kinds of embedded text:

- A **tape value** is a string a rule asks to parse (`parsed(s, g, r)`). Its nodes are not code: a rule set does not
  run over them.
- A **program** is code its file holds, made by the file's grammar (an HTML file's `<script>`). A rule set of the
  program's language runs over it, and its findings land at the file's lines.

A string at a known place is the second kind: the HTML pack should check `"<b>Welcome</b>"` as HTML. Rules do not
decide this (0431 R4 keeps rules from making code). The language's registry entry does.

## Decision

### 1. An embed lists its places

A language's `embeds` entry may name the places its strings hold the embedded language:

```json
"embeds": [{
  "language": "html",
  "via": "call-argument",
  "places": [
    { "callee": "markupsafe.Markup", "argument": 0 },
    { "callee": "django.utils.safestring.mark_safe", "argument": 0 },
    { "callee": "django.utils.html.format_html", "argument": 0 }
  ]
}]
```

- `callee` is a qualified name: the module path and the name, dot-separated.
- `argument` is the positional argument's index, from 0.
- `places` belongs to the `call-argument` embedding: `build_languages` refuses it on any other `via`, and refuses
  `call-argument` without places.

### 2. The file alone decides

A call is at a place where its callee resolves to the place's qualified name in the file's own scope table, read as
Ruff reads it (`resolve_qualified_name`): through the file's imports and aliases. A name the file binds otherwise (a
`def Markup`, an assignment) is not the callee. Nothing outside the file counts: no other module's re-exports, no
type inference. The same file therefore always holds the same programs.

### 3. The argument is a plain string

The argument is a program only where it is a string literal with no interpolation and no escapes: a `string`, or an
implicit concatenation of such strings (`"<b>" "x</b>"`, one program). An f-string, a bytes literal, a name or an
expression is not. A string with an escape sequence is left out for now (its value is not its bytes; 0058's mapped
values would place it).

The program's text is the string's content, byte for byte. Each part maps to its bytes in the file, so a node of
the program stands at its file line and column, as a `<script>`'s does.

### 4. The batch makes the programs from the file's tape

The embedder runs where a file's programs are made, in its batch, from its tape (0431 step 3). It reads the host
through the tape document the batch holds, so the scope table reads the tape in place. Nothing is copied (0353).

A file whose text names no place's last segment (`Markup`, `mark_safe`, `format_html`) builds no scope table for
this embed: one substring search per name settles it.

The parse-time `FileImage::embedding` has no tape document, so it makes no `call-argument` program. 0431 already
reads a file's programs from the batch, not from the parse (Q268).

### 5. A place's program stands at its argument, and is not the file's own program

- Each program stands at the argument it is read from, the `string` or the `concatenated_string`, so a rule reaches
  it as `parsed(s, "html", p)` at that node (`Program::at`). A `<script>`'s stands at its `raw_text`, the innermost
  node over its text, as before.
- An embedding at places is not the file's program. A Blade view or a Vue component's script is the file's whole
  program, and the file's semantics read it (its calls, scopes and `ast.embedded` clauses); a Python file's
  semantics stay its own tree's. `GrammarDialect::program_embeds` is `embeds` less the embeddings that list places,
  and the semantic readers ask it.

### 6. The program is parsed as its language's file

Each program is parsed with its dialect's lean parser where one reads it (HTML's), else tree-sitter, as a
`<script>`'s is (`Parse::read`). A Python string of HTML is a document of nodes, not one element. A place that holds
a part of a language may name a fragment (0466) in a later step. No place does yet.

## Requirements (each with its test before the code)

- R1. The registry's places are checked: a `callee` is a dotted name of identifiers, `places` appears only with
  `call-argument`, and `call-argument` has places. The generated table gives each Python place.
- R2. `Markup("<b>x</b>")`'s argument is a program of its file, in HTML, at the string: its `element` stands at
  the file's line and column.
- R3. The callee is matched as the file resolves it:
  - `from markupsafe import Markup as M; M("…")` embeds;
  - `import markupsafe; markupsafe.Markup("…")` embeds;
  - a file that defines its own `Markup`, or never imports one, does not;
  - `format_html`'s argument 0 embeds, and its argument 1 does not.
- R4. Only a plain string embeds: an f-string, a bytes literal, a name, or a string with an escape does not. An
  implicit concatenation is one program over its parts.
- R5. A rule over HTML finds the program's nodes, and its finding is at the Python file's line.
- R6. PostHog's findings: unchanged (29,747, fa0efeaed6ff), or each change named.

## Consequences

- Python embeds HTML, so a Python file's programs are looked for in its batch. The text test keeps the cost to a
  search for three names in files that hold none.
- A place's resolution is the scope table's, so it is exactly as good as the table: a call through a re-export or a
  wrapper function is not a place.

## Steps

1. Done: the registry's places, the `call-argument` embedder, Python's three places, the batch's programs from the
   tape, `program_embeds`, R1–R4, R5 for a Python rule, and R6.
2. Open: a run of the embedded language reads the files that hold its programs. A run reads its own language's
   files only (`CorpusFiles::of`), so an HTML run sees no Python file's program, and a JavaScript run sees no HTML
   file's `<script>` either (found here; it predates this decision). The run is to read each file whose dialect
   embeds the run's, with the host's own nodes hidden (0058), and, for a place embedding, only the files whose text
   names a place (the same test as §4). Its test is `an_html_run_reads_a_python_files_html_program`, ignored until
   then.

## Measured (R6)

PostHog cold on the CPU, on the M3 (load average 50 from other jobs): 29,747 findings, fa0efeaed6ff, unchanged.
2,417 G instructions, against 2,421 G at 0431 step 5.

## Tests

- R1: `build_languages`'s `an_embed_lists_the_places_its_strings_hold_code`, `a_place_names_a_qualified_callee`,
  `places_belong_to_a_call_argument_embed` and `a_call_argument_embed_has_places`.
- R2, R3, R4: `call_argument_a_string_at_a_place_is_a_program_at_its_bytes`,
  `call_argument_a_callee_the_file_does_not_import_is_no_place` and `call_argument_only_a_plain_string_is_a_program`.
- R5: `a_string_at_a_declared_place_is_a_program_of_its_file`, and `an_html_run_reads_a_python_files_html_program`
  (ignored: step 2).
- Lean parsers read a file that embeds code: `a_file_that_embeds_code_is_read_by_its_lean_parser`.

