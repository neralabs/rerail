# 0176: Python's format strings are guest dialects, `percent-format` and `brace-format`

Status: decided (2026-11-13, in the work to run all of main's Python rules: "We need to get all of these running").

## Intro

Some of main's rules read a Python string as a format string. They parse the string's value with a grammar,
`percentFormat` for `"%s" % x` or `braceFormat` for `"{}".format(x)`, and match the tree. For example, F501 reports
`"%(a" % d` because the tree has an `error` node holding Ruff's message, "incomplete format key".

The translated rule asks for the tree with `parsed`:

```
// s: a string node; r: the root of its value's tree, parsed as a percent format
parsed(s, "percentFormat", r), kind(e, "error"), inside(e, r), value(e, msg)
```

Before this decision the engine knew no grammar of that name. The reader refused these rules, and the rules that
need a format string's tree could not run.

## Decision

1. **Each format grammar is a dialect of a new registry language, `python-format`** (`languages/python-format.json`,
   family `format`). The dialects are `percent-format` (tape tag 15) and `brace-format` (tag 16), parsed by
   `literal` with grammars `native:percent-format` and `native:brace-format`. A rule may name either dialect in
   camelCase (`percentFormat`). `language::dialect_by_label` maps that to the dashed label.
2. **A document of either dialect is a guest**: its text is a string's value, never a corpus file. Every capability
   (`scope`, `value`, `calls` and so on) is refused, and the family's adapter (`adapter::format`) reads no files.
3. **The trees come from Ruff's parsers, ported** (`literal::percent` and `literal::brace`, already in the engine
   for main's matchers). `literal::format_tape::FormatStrings` is the dialect's `TreeParser`: it writes the parser's
   tree in preorder onto a tape, with kind ids taken from the grammar's `KINDS` starting at 1. Rank 0 allows a
   hand-written parser as a stopgap. The final shape is each grammar as a transducer program.
4. **Node values come from the same parse.** An `error`'s message and a field's name or index are core `value`
   facts. `core_rr` asks the family's `DialectFacts::node_values` for them, parsing once per document.
5. **The language is built with Python** (`"builtWith": "python"`): it has no `lang-python-format` feature. A build
   without `lang-python` leaves it out, along with its adapter.

## Consequences

- With this decision, 535 of main's 537 rules pass the reader. UP030 and UP032 still fail: they need `formatArgs`,
  which matches a format string against the call's arguments and is not yet written in Datalog.
- F501, F502, F503, F506, F508, F509, F521, F525 and UP031 run. Their findings have not yet been compared with
  main's.
- The value parse repeats the tape's parse once per document. Folding the two into one pass belongs with the
  transducer port.
