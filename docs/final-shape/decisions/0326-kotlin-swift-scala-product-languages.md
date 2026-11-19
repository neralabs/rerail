# 0326: Kotlin, Swift and Scala become product languages, read by their lean parsers

Status: decided (user, 2026-11-16: "please make sure that these are the top-tier parsers for their
languages").

## Intro

0305 built lean parsers for Kotlin, Swift and Scala and tested them against tree-sitter, but the
three grammars were test dialects: no file was routed to them and no rule could name them. This
makes them product languages, the way 0195 made Java one. Each language's files are read by its
lean parser first (rank 0 in the parser router). A file the lean parser refuses goes to
tree-sitter's grammar, so either way the tape is tree-sitter's.

```
a .kt / .kts file  -> lean Kotlin parser (rank 0) -> tree-sitter-kotlin's tape
                      refused -> tree-sitter-kotlin (ast-grep's grammar) -> the same tape shape
a .swift file      -> lean Swift parser  (rank 0), else tree-sitter-swift
a .scala / .sc file-> lean Scala parser  (rank 0), else tree-sitter-scala
```

## What changes

- **Registry**: `languages/kotlin.json`, `swift.json` and `scala.json`. Each is `supported`, and
  every capability is `not-supported-yet`, as Java's is. Their tape tags are new and persisted:
  Kotlin 24, Swift 25, Scala 26. Scala's old test tag, 4, is retired with the test dialect.
- **Features**: `lang-kotlin`, `lang-swift` and `lang-scala` in every crate that has `lang-java`,
  and in `all-languages`.
- **Dialects**: the three leave `grammar_dialect!`'s test-only list. Their token automata
  (`token_automata::TEST_ONLY`, now empty) and `grammar_json` arms are built in the product.
- **Adapters**: `adapter/{kotlin,swift,scala}.rs`, which are syntax only, modeled on Java's: no
  scope, values, arguments, calls or format arguments yet, so a rule that asks for them on these
  files is refused at compile time. Comment kinds are the grammars' own
  (`multiline_comment` joins the walks' comment kinds).
- **Router**: `tape::{kotlin,swift,scala}_lean`, each the glue `tape::java_lean` is. Each maps the
  lean parser's kind, field and class tables to the grammar's ids, and is registered at rank 0
  under its feature.
- **Contextual aliases**: tree-sitter-scala names some symbols differently by context. For
  example, the `(` of a parameter list is a symbol whose own name is another kind's. tree-sitter's
  tape stores such a node's name as a contextual alias (`encode_node`). The lean parser writes
  the node's name, so the glue gives each kind whose symbol is spelled or named otherwise that
  alias. It numbers aliases by first use, as `pack` does, and the tape takes them through
  `TapeImage::with_kind_aliases`. Without this, Scala's tapes read the same by name but differed
  in their bytes.
- **Tests**: `<lang>_tapes_from_the_lean_parser_are_tree_sitters`. Each checks a handful of
  texts byte for byte against tree-sitter's tape, checks that invalid texts are refused, and
  checks that the product's route parses with the lean parser and falls back to tree-sitter. The
  corpus comparison stays with each crate's oracle (0305).

## Not here

Scope, values and suppression comments for these languages. Those are later work, as they still
are for Java and C#.
