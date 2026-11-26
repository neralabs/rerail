# 0698: Python's Bitlex grammar, and a level's indent

Status: building (the user, 2026-11-25: "now, I want to switch to Python. Please get the .bitlex file for Python
both correct and performant. Once again, we proceed by addressing differences with the lean version.").
Follows 0163 (the lean Python parser), 0305 (Kotlin's grammar against its lean parser) and 0685 (tier 2 has the
lean parser's functions).

## Intro

Python has a lean parser (`rerail_python_lean`, 0163) that writes tree-sitter-python's tape directly, and a Bitlex
lexer that is a document only (`docs/parser-generators/lexers/python.bitlex`). No test compiles that document, and
it uses constructs Bitlex has since removed (back fills, 0416). Kotlin and `.rr` each have a grammar, lexer and tree
in one `.bitlex` file. It compiles to tier 1 and tier 2, and its tapes are checked node for node against the lean
parser's.

Python needs one thing those grammars did not: layout. A block is the lines indented past its header's, and it ends
at the first line indented less. tree-sitter's scanner keeps a stack of widths, and gives INDENT, NEWLINE and DEDENT
tokens from it. The lean lexer does the same. None of those tokens is a node on the tape. They only decide where
statements and blocks end.

## Decisions

1. **A crate of its own.** `rerail-python-parser` (`native/crates/python_parser`) holds `python.bitlex`, compiled as
   Kotlin's is (`rerail_kotlin_parser`). Its tests compare each tape with the lean parser's: kinds, fields, supertypes,
   the extra flag and spans. A file the grammar does not read is refused, to the lean parser. The corpus is a folder
   of `.py` files (`RERAIL_PYTHON_CORPUS`). Tier 2 as Rust gets its own crate once the grammar reads a corpus, as
   `rerail-kotlin-aot` holds Kotlin's.
2. **The lexer is written anew** in the constructs the compiler has: regions for strings and comments, the token
   loop's rows, keywords, pairs. The document keeps its place as the design study it is.
3. **Line breaks inside brackets are blanks, in tier 1.** A bracket depth over code (`count`) leaves a line break
   inside `()`, `[]` or `{}` no mark, as tree-sitter's scanner gives no NEWLINE there. So an expression that goes
   on over lines (`(a\n + b)`) is read with no mark in it.
4. **A statement ends at `newline`**, a mark at each line break of code outside brackets and not after a line
   continuation, and at the file's end. Blank and comment lines between two tokens are one mark.
5. **A level's indent.** Each level holds a width, its indent. A nested level starts with its caller's indent and
   gives it back when it ends, as a remembered word is (`remember`). A level named by `indented` (`indented block`,
   a statement of the tree, as `remember` is) sets its indent, at its first token, to that token's column: the word
   `INDENT`, after the `PUSH` of each call into it. The interpreter keeps it beside each return; the JIT keeps it in
   the return's high 16 bits (`Mem::Indent`), and refuses a column past them.
6. **Two questions compare a column with the indent.** `, deeper` holds where the column of the token the mark is
   before is past the asking level's indent. `, shallower` holds where it is less. Each level asks with its own
   indent. So a level that ends at a `shallower` mark (`ends`) leaves it to its caller, which asks again with its own
   indent:
   - `mark indent = …, deeper`: a block's start, after its header's `:`. The header takes it (`':' indent
     consequence: block`), so it is asked with the header's indent; the block then sets its own. A block that took
     it itself would ask it with the indent it had just set.
   - `mark dedent = …, shallower, ends`: the end of every block wider than the line, one level after another.
   Each mark's question is a table of the program (`markcolumn K C`, `Words::mark_columns`).
   A block's statements then go on while the next line's column is at least the block's indent, which is
   tree-sitter's rule. A line indented past the block but not after a header is a statement of the block, as
   tree-sitter's scanner gives NEWLINE there and no INDENT. A dedent to a width between two blocks' widths ends the
   inner one only, as the scanner pops widths while the line is less.
7. **A column** is the token's start less its line's start: the byte after the last line break before it. The bytes
   between the record before a token and the token are blanks and line breaks only, so tier 2 finds that line break
   in that gap, a word at a time, as `first_on_line` finds a terminator. A line whose indentation holds a tab or a
   form feed is refused for now, since those count as tree-sitter's scanner counts them (to the next multiple of
   8; back to 0).
8. **The Rust tier 2 refuses an `indented` grammar for now** (`bitlex_rust::tier2`). The JIT and the interpreter read
   it; Metal runs the JIT's tier 2 on its records, so it needs nothing of its own.
9. **Strings: one reader over four regions.** A region per quote kind and length (`s1`, `d1`, `s3`, `d3`). A raw
   string closes as any string does, at a quote no backslash escapes, so it is no region of its own. `in_raw`, a fill
   from a raw opener, says which strings' backslashes are text. One reader (`string_start`) reads them all, as
   Kotlin's reads its strings and raw strings: a triple's two more quotes are a part (`quote2`), and every escape is
   one part (`escape_sequence`, `last` its last byte). A backslash that starts no escape is a part and a leaf of its
   own (lean's `Backslash`). The closer is `string_end`. A prefix (one or two prefix letters at a word's start, then a
   quote) is a token of its own; the tree merges it, the opener and `quote2` into one `string_start` leaf (`merge`).
   Not read yet, so refused: f-strings, `\N{…}`, `\N`, `\u` and `\U` in bytes strings, carriage returns in strings,
   a prefix of three letters, a triple opened or closed by four quotes or more, and a raw string of backslashes
   only, whose end the lean lexer starts at its first backslash.
10. **The provers evaluate each value once per choice** (`bitlex_prove::Proof::memo`). A block program's ops are a
    graph: a mask that many others read (a region's fill, `escaped`) was evaluated once for every path to it, at every
    step of the search. Python's string masks made one question run for minutes. Each value at a lag is kept for the
    bytes and carried values chosen now, and dropped when a choice changes. Its first evaluation sets the search's
    open choice where it was unknown, so the answers and the search's steps are the ones it had.
