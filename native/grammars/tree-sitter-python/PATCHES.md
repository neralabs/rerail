# tree-sitter-python: vendored with patches

Upstream: https://github.com/tree-sitter/tree-sitter-python, tag `v0.25.0`
(commit `293fdc02038ee2bf0e2e206711b69c90ac0d413f`), the version `ast-grep-language` 0.45.3 pins
and the latest release. Upstream `master` adds one commit after it (`26855ea`, marking
`expression_statement` a supertype), which fixes neither gap below. tree-sitter and its grammars
accept contributions only from maintainers, so Rerail carries its fixes here instead of upstream.

## Files

| File | Origin |
| --- | --- |
| `LICENSE` | upstream, verbatim |
| `src/scanner.c` | upstream, with patch 2 |
| `src/tree_sitter/{alloc,array,parser}.h` | written by `tree-sitter generate` 0.27.0 |
| `src/grammar.json` | regenerated from the patched `grammar.js` |

Upstream's `src/parser.c` is not vendored: `native/build.rs` generates the parser from
`grammar.json` with tree-sitter-generate 0.27.0, as it does Blade's and TypeScript's.

## Regenerating

```sh
git clone --branch v0.25.0 https://github.com/tree-sitter/tree-sitter-python
cd tree-sitter-python
git apply <the patches below>
tree-sitter generate   # tree-sitter-cli 0.27.0
tree-sitter test       # upstream's corpus: 117 of 117 parse as before
```

then copy `src/grammar.json`, `src/scanner.c` and `src/tree_sitter/*.h` here. Unpatched, this
reproduces upstream's `grammar.json` byte for byte.

## Patch 1: PEP 696 type parameter defaults

Upstream issue: tree-sitter/tree-sitter-python#346 (open). An earlier attempt, #315, was closed
unmerged; it replaced the declarations' `type_parameter` with new `type_params`/`type_param` nodes.

Why: Python 3.13 accepts a default for a type parameter (`def f[T = int]()`, `class C[T: B = int]`,
`type A[*Ts = *tuple[int]] = ...`, `[**P = [int]]`). Upstream parses each into an `ERROR`, so
Rerail declines the whole file as having syntax errors: every Ruff fixture finding in
`pyflakes/F821_17.py` was missed.

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -468,11 +468,19 @@ module.exports = grammar({
     type_parameter: $ => seq(
       '[',
-      commaSep1($.type),
+      commaSep1(choice($.type, $.type_default)),
       optional(','),
       ']',
     ),
 
+    // PEP 696: a type parameter's default, `T = int`, `T: B = int`, `*Ts = *tuple[int]`,
+    // `**P = [int]`.
+    type_default: $ => seq(
+      field('name', $.type),
+      '=',
+      field('value', choice($.type, $.list_splat)),
+    ),
+
     parenthesized_list_splat: $ => prec(PREC.parenthesized_list_splat, seq(
```

One node kind is added, `type_default`, with the existing field names `name` (the parameter as
before: a `type` holding an `identifier`, `constrained_type` or `splat_type`) and `value` (a `type`,
or a `list_splat` for a starred default). A `type_parameter` element is now a `type` or a
`type_default`; nothing else changes. The engine binds the `name` as before and reads the `value`
lazily in the annotation scope, as it reads a bound (`scope/python.rs`, `type_parameters`).

`type_parameter` is also the subscript of an annotation's `generic_type`, so `x: list[T = int]`,
which CPython rejects, now parses, as `x: list[T: int]` already did upstream.

## Patch 2: a lone carriage return ends a line

Upstream issue: none found (searched for carriage return and `\r` in tree-sitter-python's issues
and pull requests; the related #263 is about CRLF checkouts).

Why: CPython's tokenizer reads `\r\n`, `\r` and `\n` as newlines (universal newlines). Upstream
ends a line only at `\n`: its scanner skips a lone `\r` as indentation whitespace, a comment runs
through it (`/.*/`), and a backslash continuation needs `\n` after an optional `\r`. A file whose
statements are separated by lone `\r` parses with errors (`suppress/ruff/terminators.py` in Rerail's
oracles).

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -1105,7 +1113,7 @@ module.exports = grammar({
         /x[a-fA-F\d]{2}/,
         /\d{1,3}/,
-        /\r?\n/,
+        /\r\n?|\n/,
         /['"abfrntv\\]/,
         /N\{[^}]+\}/,
       ),
@@ -1189,9 +1197,9 @@ module.exports = grammar({
-    comment: _ => token(seq('#', /.*/)),
+    comment: _ => token(seq('#', /[^\r\n]*/)),
 
-    line_continuation: _ => token(seq('\\', choice(seq(optional('\r'), '\n'), '\0'))),
+    line_continuation: _ => token(seq('\\', choice(seq('\r', optional('\n')), '\n', '\0'))),
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -200,7 +200,7 @@ bool tree_sitter_python_external_scanner_scan(void *payload, TSLexer *lexer, con
-            } else if (lexer->lookahead == '\n' && has_content && !is_triple(delimiter)) {
+            } else if ((lexer->lookahead == '\n' || lexer->lookahead == '\r') && has_content && !is_triple(delimiter)) {
                 return false;
             }
@@ -221,7 +221,11 @@ bool tree_sitter_python_external_scanner_scan(void *payload, TSLexer *lexer, con
             indent_length++;
             skip(lexer);
-        } else if (lexer->lookahead == '\r' || lexer->lookahead == '\f') {
+        } else if (lexer->lookahead == '\r') {
+            found_end_of_line = true;
+            indent_length = 0;
+            skip(lexer);
+        } else if (lexer->lookahead == '\f') {
             indent_length = 0;
             skip(lexer);
         } else if (lexer->lookahead == '\t') {
@@ -240,7 +244,7 @@ bool tree_sitter_python_external_scanner_scan(void *payload, TSLexer *lexer, con
-            while (lexer->lookahead && lexer->lookahead != '\n') {
+            while (lexer->lookahead && lexer->lookahead != '\n' && lexer->lookahead != '\r') {
                 skip(lexer);
             }
@@ -249,8 +253,10 @@ bool tree_sitter_python_external_scanner_scan(void *payload, TSLexer *lexer, con
             skip(lexer);
             if (lexer->lookahead == '\r') {
                 skip(lexer);
-            }
-            if (lexer->lookahead == '\n' || lexer->eof(lexer)) {
+                if (lexer->lookahead == '\n') {
+                    skip(lexer);
+                }
+            } else if (lexer->lookahead == '\n' || lexer->eof(lexer)) {
                 skip(lexer);
             } else {
                 return false;
```

Node kinds and fields are unchanged. A `comment` no longer holds the `\r` of a CRLF line end (it
held it upstream, as `.` matches `\r`), matching the comment text CPython and Ruff read. A
single-quoted string no longer runs through a lone `\r`, which CPython rejects as unterminated.

Tree-sitter itself still counts rows at `\n` only, so a node's row and column (and a tape's, which
equals the native tree's) do not change; findings are placed by Rerail's line table, which follows
Ruff's `\r\n`, `\r` and `\n` (`policy_dsl/lines.rs`).
