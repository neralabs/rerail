# 0482. A fragment is parsed from a node kind, by a hand-written parser

Status: decided (2026-11-19), by the user: "we need to allow a parse to start from a given node type. For example, a
Python annotation is not a module. It's a type. A JSX document doesn't start with a <html> tag. And so on." For
dialects that only tree-sitter parses: "Hand-written parsers only". Comes before 0431's next step, embeds a language
declares at known places (Q338, the user's choice 2). Numbered 0466 on django-bugs; 0482 here (roll-up 15).

Amends 0082 §1 (a fragment's start symbol). Uses 0431's tape values.

## Intro

A rule can ask for a string's text to be parsed in some grammar. A Python string annotation is the usual case:

```python
def f(x: "List[int] | None") -> None: ...
```

The text `List[int] | None` is not a Python file. It is what Python reads after `x:`, a type. Until now the engine had
one way to parse a part of a language: it put the text in `( … )`, parsed the result as a whole module, and picked
the expression out of the tree. The tree still held the `module`, the statement and the parentheses above it, and
the code that picked the root out named Python's kinds.

That does not reach other cases. An HTML or JSX snippet is an element, not a document. A Vue template's expression
is an expression. Each needs a parse that begins at a given kind of node.

## Decision

### 1. A fragment names the node kind it starts at

A fragment, declared in its language's entry (0082), names a node kind of its dialect as its start:

```json
"fragments": [{ "name": "annotation", "dialect": "python", "start": "type", "tapeTag": 30 }]
```

- The parse of a fragment's text begins at that kind. Its tape's root is one node of that kind. The node spans the
  text, less blanks at either end. The positions are the text's own. No node is written above the root, and nothing
  of a wrapper: no `module`, statement or parentheses.
- Nothing in the engine names a language's kinds to find the root (`wrapped_root` goes, and the `( … )` wrapper).

### 2. Only a hand-written parser starts at a kind

tree-sitter has one start rule per compiled grammar. A fragment is parsed only by its dialect's hand-written parser
(the lean parsers: Python, ECMAScript and its JSX, HTML, …), from an entry at that kind.

- Each such parser lists the kinds it may start at (`STARTS`) and parses from one (`parse_from`).
- A fragment whose dialect has no hand-written parser, or whose start is not in its parser's list, is refused when
  the registry is checked (a test over every fragment), not at a run.
- A text the parser refuses is one `ERROR` node over it, less blanks (`TapeImage::refused`). No tree is made up for
  it, and `.on guest_error` sees it at its place: skip withholds it, report gives the finding, partial keeps the
  `ERROR`. There is no tree-sitter fallback for a fragment. (First written as "no tape"; that left `.on guest_error`
  nothing to see, so its test `guests_that_do_not_parse_follow_the_policy` failed.)

### 3. A fragment's tapes have a tag of their own

A fragment is a grammar name (0082), and its tapes are not its dialect's whole-file tapes: `List[int]` read as a
type is not `List[int]` read as a module. A fragment has a tape tag of its own in the registry. Its tapes are stored
and found by their text and that tag (0431 R6). They are read with its dialect's kinds (`from_tape_tag` gives the
dialect).

### 4. Each start reads as its language reads that place

The start's text is read as the language reads it at that place:

- **Python `type`**: what follows `x:` in an annotated assignment, read as Ruff reads a string annotation, as if in
  parentheses: line ends inside the text are blanks. Its tree is the one the parser writes for an annotation in a
  file, so a rule reads a string annotation and a written one alike.
- **TSX/JSX `jsx_element`**: one element, as it is read where an expression starts. Its root takes that slot's
  supertype (`expression`), as `x = <a/>`'s does. `<a />` is a `jsx_self_closing_element`, a start of its own; each
  start refuses the other's kind. TypeScript (`.ts`) reads no JSX and starts at no element. The registry's fragment
  is `jsx` (JavaScript, tag 31).
- **HTML `element`**: one element and what it holds, with no implied `html`, `head` or `body`. It ends where the
  scanner ends it, as in a document. An element left open at the text's end ends there only where a document's
  would (a void element, …); elsewhere the text is refused, as a document is. `<script>` and `<style>` are kinds of
  their own (`script_element`, `style_element`), so `element` refuses them. The element's scripts are its
  `raw_text`; no program is made of them. The registry's fragment is `html-element` (tag 32).

## Requirements (each with its test before the code)

- R1. A fragment's tape has one root of its start kind, spanning the text less blanks, and no node above it
  (per parser: Python `type`, TSX `jsx_element`, HTML `element`).
- R2. A Python `type` fragment's tree is the parser's tree of the same text after `x: ` in a file, read at the
  `type` node (positions shifted by the prefix).
- R3. A Python `type` fragment joins line ends: `"Dict[\n str, int]"` parses.
- R4. Every registry fragment's start is in its dialect's hand-written parser's `STARTS` (`build_languages`'s
  check and an engine test).
- R5. `parsed(s, "annotation", r)`'s root is a `type` node at the string's place, and the libraries' rules read it
  as they read a written annotation's `type`.
- R6. A fragment's tape is stored and found by its text and its own tag. A whole-file tape of the same text is not
  found for it.
- R7. PostHog's findings: unchanged (29,747, fa0efeaed6ff), or each change named with Ruff's verdict on it.

## Consequences

- The libraries read an annotation's root as a `type`, whose child is the expression or a `generic_type`, like a
  written annotation's. Rules written for the old root (the expression itself) move to that shape.
- Nested string annotations (`"List['A']"`'s `'A'`) are found at their range in the outer fragment's text, with no
  offset for a `(`.

## Measured (R7)

PostHog cold on the CPU, on the M3 (load average 52 during the run, from other jobs), at the change with the Python
`type` start and refused texts as `ERROR`: 29,747 findings, fa0efeaed6ff, unchanged. It took 2,651 G instructions,
against 2,421 G before (0431 step 5), on a quieter host. The instruction count is not compared under that load.

## Tests

- R1: `rerail-python-lean`'s `a_type_fragment_is_rooted_at_its_type`, `rerail-ecma-lean`'s
  `a_jsx_fragment_is_rooted_at_its_element` and `a_jsx_fragment_is_one_element`, `rerail-html-lean`'s
  `an_element_fragment_is_rooted_at_its_element` and `an_element_fragment_is_one_element`, and the engine's
  `a_fragment_is_parsed_from_its_start_by_its_parser` and `an_html_fragment_is_one_element`.
- R2: `a_type_fragment_is_tree_sitters_written_annotation`, against tree-sitter-python's tree of `x: T`, unions
  included, and `a_type_fragment_is_a_written_annotations_type`. JSX: `a_jsx_fragment_is_the_files_element`.
- R3: `a_type_fragment_joins_its_lines`.
- R4: `every_fragment_starts_where_its_parser_starts`, and `build_languages`'s checks of a fragment's start and tag.
- R5: `string_annotations_are_guests_at_their_host_lines` (the root is `type`) and
  `string_annotations_are_ruffs_with_their_trees`.
- R6: `a_run_stores_its_tape_values_by_their_text` (under the fragment's tag, not Python's).
- A refused text: `guests_that_do_not_parse_follow_the_policy`.
