# 0198: PHP is read by a lean parser that writes tree-sitter-php's tape, or refuses

Status: built (user, 2026-11-14: "next we need PHP (including Blade), Markdown, HTML and CSS"; PHP
first). PHP has been a product language since the registry's start (`languages/php.json`, the
`lang-php` feature), read by tree-sitter-php's `php_only` grammar until now. Blade templates embed
PHP: their compiled program (`blade::view`) is PHP text, read by the same grammar.

## Intro

The engine reads a source file into a tape: its syntax tree in preorder, one code a node (kind,
field, supertypes).

The lean Python, ECMAScript, Rust, Java and C# parsers (0163, 0169, 0178, 0193, 0195, 0196) write
the tape directly. Each writes exactly the tape tree-sitter's tree encodes to, and refuses any file
it cannot read exactly. This decision builds the same parser for PHP, at rank 0 before
tree-sitter-php.

```
a .php file
  -> its view (parse_input): the text outside <?php … ?> blanked, every offset kept
  -> stage 1: byte classes, 64 bytes a step (rerail-simd8: NEON, AVX2, SSE4.2, scalar)
  -> the lexer: runs and stops by trailing zeros; every word one hash probe (case-insensitive)
       a string, a shell command, a heredoc or a nowdoc is one token over the whole literal
  -> recursive descent over tree-sitter-php's grammar, writing the tape (no backtracking)
       a string's pieces are cut where tree-sitter-php's scanner cuts them
       a syntax error, or a construct not read exactly -> refused -> tree-sitter-php
```

There is one behavior: tree-sitter's tape, or a refusal. A file with a syntax error is always
refused, and tree-sitter reads it with its own recovery. There are no modes.

## Prior work consulted

- **ast-index-spike indexes:** the spike's sample corpus (`ast-index-sample-corpus`) holds the PHP
  repositories `framework` (Laravel) and `symfony`; 805 of their files are the oracle's first set,
  and 77 of their Blade templates its Blade set.
- **0196 (lean C#), 0195 (lean Java), 0193 (lean Rust), 0178 (lean ECMAScript), 0163 and 0169
  (lean Python), 0181 (their profile).** **Agrees and follows:** the same crate shape, stage 1,
  keyword hash, writer, token buffer and oracle. "No error recovery: one behavior" is kept as is.
- **0140 (the tape's writer).** **Agrees:** the parser writes through `rerail_tape_format::Writer`,
  and the engine takes the columns as written (`TapeImage::written`).
- **The engine's implied-token automata (`tape::tokens`).** **Followed:** the parser stores exactly
  the tokens the automata keep (`implied.tsv`, `kinds::DROPPED`, and a gap's tokens by the state it
  starts from). It never writes a token to filter it out afterwards.
- **tree-sitter-php 0.24.2's grammar and external scanner** (`common/define-grammar.js`,
  `common/scanner.h`): the readings below follow them, checked by the oracle.

## Decision

### The crate: `rerail-php-lean`

`native/crates/php_lean`, in the shape of `rerail-csharp-lean`:

- `block`: stage 1, masks a 64-byte block (name bytes, blanks, line ends, a double-quoted string's
  stops, a single-quoted string's stops, a line comment's stops).
- `lex`: tokens where the parser asks, with no state. A word's code comes from one probe of a
  perfect hash over its bytes in lower case (PHP's keywords are case-insensitive); the parser
  decides where a word is a keyword. A comment ends at its line's end or before `?>`.
- `parse`: statements (both block and colon forms), declarations, expressions (precedence climbing
  over tree-sitter-php's `PREC`), types, names; `parse/string`: strings taken apart.
- `gen_kinds.py`: writes `kinds.rs` from the token grammar and `implied.tsv`.

The parser never backtracks over the source. Choices are made by looking ahead over tokens (`[`
starts a destructuring list where its `]` is followed by `=`; `(` starts a cast where a cast's type
word and `)` follow). Where a stored token depends on what a gap's next child turns out to be
(below), the parser writes the token, reads the child, and where the token is implied reads the
child again without it: a step over one interpolation or one list item, never over the file.

### Strings

The lexer reads a whole literal as one token; the parser then cuts its pieces where
tree-sitter-php's external scanner (`scan_encapsed_part_string`) ends a `string_content`: before
the closing quote, before `\` and a character it escapes, before `$` and a name's first character
or `{`, before `{$`, and in a heredoc before a line's end. Right after a variable, `[…]` and `->name`
continue it (`subscript_expression`, `member_access_expression`). An interpolation's expression
(`{$…}`, `${…}`) is lexed again as tokens and parsed as any other.

A heredoc's body is its lines' pieces (an empty line has none); a nowdoc's is `nowdoc_string`s,
each from the end of the one before, line breaks and indentation included. An end tag that would
end a heredoc in the middle of a line (after an interpolation) is refused.

### Where tree-sitter's choices are kept

The tape must be tree-sitter's even where its grammar reads a text in a surprising way. These were
found by the oracle, and the parser keeps each as tree-sitter reads it:

- **An assignable operand before `=`** is the assignment's left at any precedence: `!$a = f()` is
  `!($a = f())`.
- **`: static`** is a `named_type` holding the name `static`.
- **`A::$b(…)`** is a `scoped_call_expression` whose name is a variable.
- **`const FALSE = …`** in a class: tree-sitter lexes the word as a type and errors, so the parser
  refuses (any `primitive_type` word before `=`). After a type, a reserved word as the constant's
  name is refused the same way.
- **A text of white space alone** (a template's view) is a `program` starting at its end.
- **Tokens stored by the state a gap starts from** (the automata's words are counted from the state
  after the previous child, to the next child's kind):
  - an interpolation's `{` is stored before a kind that could stand without braces (a variable, a
    member access, a subscript), and at an encapsed string's start; its `}` is stored only in `}{`
    before such a kind;
  - `->{…}` and `${…}` store their `{` before a variable;
  - in a destructuring list, a `,` after a variable and before an item that may be a value is
    stored (`=>` is as short there); `list(` stores its `(`;
  - an empty string stores both quotes, any other its opening one (a `b` prefix is a hidden token);
  - a `use` list's `}` after a `;` is stored (the `;` may be empty);
  - `private(set)` stores its `(`;
  - `reference_modifier` and `variadic_placeholder` hold their token.

### Refused, for tree-sitter

A file is refused where the lean parser does not read the construct exactly:

- any syntax error;
- text after `?>` (a file mixing HTML and PHP blocks is read only up to its first `?>`);
- `b'…'` and `b"…"` strings;
- a cast's value followed by `=` (`(int) $a = …`), `A::$$b(…)`, `"$a ["`-style spacing in a
  heredoc after a variable, and `readonly public` in a promoted parameter.

### In the engine

`tape::php_lean::Lean` is PHP's `TreeParser`, registered at rank 0 under `lang-php`. It parses each
file's view (`parse_input`, the bytes tree-sitter-php parses) and the tape reads the file. A file it
refuses goes to tree-sitter-php.

Blade's compiled programs are read exactly by the parser (the oracle's Blade set); the Blade route
(`embed::Program`, which carries a tree-sitter tree built in `View::read`) is changed in a follow-up
step below.

## Results

**The oracle** (`rerail-php-lean`'s ignored `oracle` test) compares the parser's tape with
tree-sitter's, line by line. Its sets:

- **ast-index corpus:** 805 files of `framework` and `symfony` (2 with tree-sitter errors).
- **Stored repositories:** 356 files of the sampo repositories (12 a repository; 4 with errors).
- **Blade programs:** the compiled programs of 77 Blade templates, as the Blade embedding parses
  them (no `<?php` tag).
- **Cut and edited:** every file of the first two sets, each cut at 3 places and with 3 lines cut
  short in the middle.

| set | clean texts: same / differ / refused | texts with errors: as tree-sitter / differ / refused |
|---|---|---|
| ast-index corpus, 805 files | 803 / 0 / 0 | 0 / 0 / 2 |
| stored repositories, 356 files | 352 / 0 / 0 | 0 / 0 / 4 |
| Blade programs, 77 | 77 / 0 / 0 | 0 / 0 / 0 |
| cut and edited (corpus) | 1,245 / 0 / 2 | 0 / 0 / 3,655 |
| cut and edited (repositories) | 572 / 0 / 1 | 0 / 0 / 1,605 |

No tape differs: each is tree-sitter's or a refusal. Every clean whole file is read.

**Engine tests:**

- `php_tapes_from_the_lean_parser_are_tree_sitters`: hand-written cases, byte for byte against
  tree-sitter's encoded tape. Syntax errors are refused.
- `the_product_reads_php_with_the_lean_parser`: the engine's parse route gives the lean tape, and a
  refused file gets tree-sitter's.

**Cost** (`php_parse_costs`, the 803 corpus files the lean parser reads, 3.9 MB, 645,806 nodes;
best of 3 passes; profiling build; load 76 to 83):

| step | instructions a node | cycles a node | ns a byte |
|---|---:|---:|---:|
| lean: the parser's columns | 732 | 189 | 9.7 |
| lean: the tape (the view included) | 977 | 247 | 12.9 |
| tree-sitter: parse | 4,486 | 1,138 | 59.8 |
| tree-sitter: parse and tape | 8,214 | 2,204 | 116.5 |

The lean tape takes 8.4 times fewer instructions and 8.9 times fewer cycles than tree-sitter's
parse and tape. A quarter of the lean tape's instructions are the view (`parse_input`, made for
each file) and the columns' conversion.

## Open

- **The refusals listed above.** Each is a file tree-sitter reads instead; none is a wrong tape.
- **Text after `?>`**: a template mixing HTML and PHP is refused; reading `text_interpolation`
  would take those files too.
- **The view's cost**: the lean route makes each file's view (`parse_input`) again; a file with no
  text outside `<?php` could skip it.
- **Not yet profiled** against 0181's list of simdjson tricks. The parser's own 732 instructions a
  node are above C#'s 512: a string's interpolations lex their expression again.
