# tree-sitter-javascript: vendored with patches

Upstream: https://github.com/tree-sitter/tree-sitter-javascript, tag `v0.25.0`
(commit `44c892e0be055ac465d5eeddae6d3e194424e7de`), the version `ast-grep-language` 0.45.3 pins
and the latest release. Upstream `master` adds two CI commits after it (to `58404d8`), which fix
none of the gaps below. tree-sitter and its grammars accept contributions only from maintainers,
so Rerail carries its fixes here instead of upstream.

## Files

| File | Origin |
| --- | --- |
| `LICENSE` | upstream, verbatim |
| `src/scanner.c` | upstream, with Patches 2, 8, 9, 11, 13, 14 and 15 |
| `src/tree_sitter/{alloc,array,parser}.h` | written by `tree-sitter generate` 0.27.0 |
| `src/grammar.json` | regenerated from the patched `grammar.js` |

Upstream's `src/parser.c` is not vendored: `native/build.rs` generates the parser from
`grammar.json` with tree-sitter-generate 0.27.0, as it does Python's and TypeScript's.

## Regenerating

```sh
git clone --branch v0.25.0 https://github.com/tree-sitter/tree-sitter-javascript
cd tree-sitter-javascript
git apply <the patches below>
tree-sitter generate   # tree-sitter-cli 0.27.0
tree-sitter test       # upstream's corpus: 115 of 116 parse as before, `Objects` as Patch 4 says
```

then copy `src/grammar.json`, `src/scanner.c` and `src/tree_sitter/*.h` here. Unpatched, this
reproduces upstream's `grammar.json` byte for byte, and its `parser.c` but for the header comment
naming the generator's version. So vendoring alone changes no tree: over 11,540 `.js`, `.jsx`,
`.mjs` and `.cjs` files (the Next.js, React and TypeScript corpora, and every pack's and fixture's
upstream sources), the unpatched grammar builds the trees ast-grep's bundled tree-sitter-javascript
0.25.0 builds.

## Patch 1: a bare `&` in a JSX attribute string is text

Upstream issues: tree-sitter/tree-sitter-typescript#279 (closed by tree-sitter/tree-sitter-javascript#305, which
admits an `&` followed by neither `#` nor a letter, and so fixes `class="a & b"` only) and
tree-sitter/tree-sitter-javascript#304 (closed with it). Present at tag `v0.25.0` and at HEAD `58404d8`. The same
change as Patch 3 in `grammars/tree-sitter-typescript/PATCHES.md` (NER-1760).

Why: a JSX attribute string may hold `&` as text; Babel and TypeScript decode a complete character reference
(`&amp;`, `&#38;`, `&#x26;`) and read any other `&` literally. Upstream's string fragment stops at an `&` before `#`
or a letter, and the only other token there, `html_character_reference`, needs the closing `;`. So
`href="https://fonts.googleapis.com/css2?family=Krona+One&display=swap"` is a parse error, and the element around it
is lost (NER-1764): all 13 of the Next.js plugin fixtures that failed to parse (`google-font-display`,
`no-css-tags`, `no-page-custom-font`'s `pages/_document.js`) and 10 of next-examples' files. The fragment's escape for
any other `&` also admitted the closing quote, so `title="Q&"` ran on past its end.

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -743,12 +743,15 @@ module.exports = grammar({
       )),
     ),
 
+    // A bare `&` in a JSX string is text, as Babel and TypeScript read it: `src="/a?w=8&h=8"`, `title="Q&"`.
+    // Only a complete character reference, such as `&amp;`, is an html_character_reference.
     _jsx_string: $ => choice(
       seq(
         '"',
         repeat(choice(
           alias($.unescaped_double_jsx_string_fragment, $.string_fragment),
           $.html_character_reference,
+          alias($._jsx_string_ampersand, $.string_fragment),
         )),
         '"',
       ),
@@ -757,6 +760,7 @@ module.exports = grammar({
         repeat(choice(
           alias($.unescaped_single_jsx_string_fragment, $.string_fragment),
           $.html_character_reference,
+          alias($._jsx_string_ampersand, $.string_fragment),
         )),
         '\'',
       ),
@@ -766,10 +770,14 @@ module.exports = grammar({
     // We give names to the token() constructs containing a regexp
     // so as to obtain a node in the CST.
     //
-    unescaped_double_jsx_string_fragment: _ => token.immediate(prec(1, /([^"&]|&[^#A-Za-z])+/)),
+    unescaped_double_jsx_string_fragment: _ => token.immediate(prec(1, /([^"&]|&[^"#A-Za-z])+/)),
 
     // same here
-    unescaped_single_jsx_string_fragment: _ => token.immediate(prec(1, /([^'&]|&[^#A-Za-z])+/)),
+    unescaped_single_jsx_string_fragment: _ => token.immediate(prec(1, /([^'&]|&[^'#A-Za-z])+/)),
+
+    // An `&` that no character reference completes: the lexer reads `&amp;` whole and falls back
+    // to this `&` when the reference breaks off, as at `&h=8`.
+    _jsx_string_ampersand: _ => token.immediate(/&/),
 
     _jsx_attribute_value: $ => choice(
       alias($._jsx_string, $.string),
```

The `&` token is read only where the lexer cannot complete a character reference: the lexer reads `&amp;` whole and
falls back to the lone `&` when the reference breaks off, as at `&h=8` or `&#;`. The fragment keeps its precedence of
1, so `a & b` stays one fragment.

Node kinds and field names are unchanged. A bare `&` is its own `string_fragment`, so `"/a?w=8&h=8"` is a `string`
of three fragments, `/a?w=8`, `&` and `h=8`, as `"a&amp;b"` was already three children. The state count is unchanged.

## Patch 2: a bare `&` in JSX text is text

Upstream issue: tree-sitter/tree-sitter-javascript#366 (open; present at tag `v0.25.0` and at HEAD).
Mirrors: tree-sitter/tree-sitter-javascript#388 (open, head `c76af4f`), its scanner change verbatim and its corpus
case; tree-sitter/tree-sitter-typescript#371 makes the same change there (Patch 7 in
`grammars/tree-sitter-typescript/PATCHES.md`).

Why: JSX text may hold `&` as text, as React renders it. Upstream's `jsx_text` is an external token that stops at
every `&`, and the only other token there, `html_character_reference`, needs the closing `;`. So
`<span>Help & Support</span>` and `<h2>{a} & {b}</h2>` are parse errors (NER-1764): 2 next-examples files and
eslint-plugin-react's `no-unescaped-entities` fixtures, whose findings are exactly such `&`s.

```diff
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -289,6 +289,35 @@ static bool scan_html_comment(TSLexer *lexer) {
     return true;
 }
 
+// After an `&`: is what follows the rest of an HTML character reference
+// (`&amp;`, `&#38;`, `&#x26;`), as the grammar's html_character_reference
+// token defines it? Consumes the name or number either way.
+static bool scan_character_reference_tail(TSLexer *lexer) {
+    unsigned length = 0;
+    if (lexer->lookahead == '#') {
+        advance(lexer);
+        if (lexer->lookahead == 'x' || lexer->lookahead == 'X') {
+            advance(lexer);
+            while (length < 6 && iswxdigit(lexer->lookahead)) {
+                advance(lexer);
+                length++;
+            }
+        } else {
+            while (length < 5 && iswdigit(lexer->lookahead)) {
+                advance(lexer);
+                length++;
+            }
+        }
+    } else {
+        while (length < 30 && ((lexer->lookahead >= 'a' && lexer->lookahead <= 'z') ||
+                               (lexer->lookahead >= 'A' && lexer->lookahead <= 'Z'))) {
+            advance(lexer);
+            length++;
+        }
+    }
+    return length > 0 && lexer->lookahead == ';';
+}
+
 static bool scan_jsx_text(TSLexer *lexer) {
     // saw_text will be true if we see any non-whitespace content, or any whitespace content that is not a newline and
     // does not immediately follow a newline.
@@ -297,8 +326,22 @@ static bool scan_jsx_text(TSLexer *lexer) {
     // immediately follows a newline.
     bool at_newline = false;
 
+    lexer->result_symbol = JSX_TEXT;
+
     while (lexer->lookahead != 0 && lexer->lookahead != '<' && lexer->lookahead != '>' && lexer->lookahead != '{' &&
-           lexer->lookahead != '}' && lexer->lookahead != '&') {
+           lexer->lookahead != '}') {
+        if (lexer->lookahead == '&') {
+            // A character reference is its own token: the text ends before it.
+            // A bare ampersand (`Shipping & Handling`) is text.
+            lexer->mark_end(lexer);
+            advance(lexer);
+            if (scan_character_reference_tail(lexer)) {
+                return saw_text;
+            }
+            saw_text = true;
+            at_newline = false;
+            continue;
+        }
         bool is_wspace = iswspace(lexer->lookahead);
         if (lexer->lookahead == '\n') {
             at_newline = true;
@@ -326,7 +369,7 @@ static bool scan_jsx_text(TSLexer *lexer) {
         advance(lexer);
     }
 
-    lexer->result_symbol = JSX_TEXT;
+    lexer->mark_end(lexer);
     return saw_text;
 }
 
```

The scanner reads past an `&`: where a character reference as `html_character_reference` defines it follows, the text
ends before the `&` and the reference is its own token as before; otherwise the `&` is text.

Node kinds and field names are unchanged. A bare `&` is part of the `jsx_text` around it. The grammar is unchanged.

## Patch 3: a re-export takes import attributes

Upstream: tree-sitter/tree-sitter-javascript#363 (closed unmerged) proposed the same, but also after `export { a }`
without `from`, which ECMAScript does not allow, and with the deprecated `assert`. Present at tag `v0.25.0` and at
HEAD.

Why: ECMAScript 2025 allows import attributes on `export … from`, as on `import`:
`export { default as x } from './x.json' with { type: 'json' }` and `export * from 'y' with { type: 'json' }`.
Upstream takes them only on `import`, so a re-export with them is a parse error whose remainder reads as a label, and
`no-labels` reports a false positive at the `with {` (NER-1777; 4 ESLint fixtures: `id-length/086`, `087`,
`no-array-constructor/065`, `067`).

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -169,9 +169,10 @@ module.exports = grammar({
       seq(
         'export',
         choice(
-          seq('*', $._from_clause),
-          seq($.namespace_export, $._from_clause),
-          seq($.export_clause, $._from_clause),
+          // A re-export takes import attributes, as an import does: `export * from './x.json' with { type: 'json' }`.
+          seq('*', $._from_clause, optional($.import_attribute)),
+          seq($.namespace_export, $._from_clause, optional($.import_attribute)),
+          seq($.export_clause, $._from_clause, optional($.import_attribute)),
           $.export_clause,
         ),
         $._semicolon,
```

Node kinds and field names are unchanged. The attribute is an `import_attribute` child of the `export_statement`, as
it is of an `import_statement`. The generated parser has 9 more states.

## Patch 4: a `{` at the start of a statement opens a block

Upstream issue: tree-sitter/tree-sitter-javascript#259 (open). Present at tag `v0.25.0` and at HEAD.
tree-sitter/tree-sitter-javascript#86 (closed by #104) made an empty `{}` a block; the negative dynamic precedence
dates from `d16ed03` (2018, "ensure objects are preferred over labelled statements").

Why: ECMAScript forbids an expression statement and an arrow function's expression body to start with `{`, so there
`{` always opens a block: `if (foo) { bar: 'baz' }`, `() => { bar: 1 }` and `{ bar: 1 }` hold a labeled statement.
Upstream reads each as an object literal, with no error, because GLR keeps both readings and the labeled statement's
dynamic precedence of -1 makes the object win. The tree is silently wrong: `curly`, `no-labels`, `no-unused-labels`
and `arrow-body-style` see an object where ESLint sees a label (ESLint fixtures `arrow-body-style/011`,
`no-unused-labels/021`, `prefer-exponentiation-operator/040`).

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -498,7 +498,10 @@ module.exports = grammar({
 
     empty_statement: _ => ';',
 
-    labeled_statement: $ => prec.dynamic(-1, seq(
+    // A `{` that starts a statement or an arrow function's body opens a block, never an object (ECMAScript's
+    // ExpressionStatement and ConciseBody lookahead restrictions): `if (a) { b: 1 }` and `() => { b: 1 }` hold a
+    // labeled statement. Where only an object parses, as in `{ a: 1, b: 2 }`, GLR keeps the object.
+    labeled_statement: $ => prec.dynamic(1, seq(
       field('label', alias(choice($.identifier, $._reserved_identifier), $.statement_identifier)),
       ':',
       field('body', $.statement),
```

A labeled statement now outweighs the object. GLR still keeps the object where the block does not parse, as in
`{ a: 1, b: 2 }` (which ECMAScript rejects either way), and a `{` in an expression, as in `x = { a: 1 }`,
`() => ({ a: 1 })` or `f({ a: 1 })`, never had the block reading.

Node kinds and field names are unchanged. `{ a: 1 }` at the start of a statement or as an arrow function's body is a
`statement_block` holding a `labeled_statement`, no longer an `object` holding a `pair`. So upstream's corpus case
`Objects`, which asserts the object for `{ a: "b" };` and `{\n  g: h\n}` at statement level, fails, as this patch
intends; with its expectation changed to the block, it passes. Upstream's tag test `test/tags/functions.js` reads
`{ source: $ => … }` at statement level as an object too; parenthesized, it passes. The state count is unchanged.

## Patch 5: `using` declarations head a `for` statement

Upstream issue: none open (tree-sitter/tree-sitter-javascript#350, closed, added `using` declarations as statements
and in `for … of` heads only). Present at tag `v0.25.0` and at HEAD.

Why: ECMAScript's explicit resource management allows `for (using r = f(); ;)` and
`for (await using r = getResource(); ;) {}`, as `for (const …; ;)`. Upstream's `for` head takes only `var`, `let` and
`const` declarations, so these are parse errors (ESLint fixture `no-await-in-loop/020`, declared in
`packs/javascript/divergences.json`).

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -377,7 +377,8 @@ module.exports = grammar({
       'for',
       '(',
       choice(
-        field('initializer', choice($.lexical_declaration, $.variable_declaration)),
+        // `for (using r = f(); ;)` and `for (await using r = f(); ;)`, as `for (const …; ;)`.
+        field('initializer', choice($.lexical_declaration, $.variable_declaration, $.using_declaration)),
         seq(field('initializer', $._expressions), ';'),
         field('initializer', $.empty_statement),
       ),
```

Node kinds and field names are unchanged. The `for_statement`'s `initializer` may be a `using_declaration`, the node
a `using` statement already is. The generated parser has 4 more states.

## Patch 6: a destructuring assignment's target may be parenthesized

Upstream issue: none open. Present at tag `v0.25.0` and at HEAD.

Why: in a destructuring assignment, ECMAScript allows a parenthesized target, as it does on the left of a plain
assignment: `({foo: (foo)} = obj)`, `({foo: (/**/foo)} = {})`, `[(a), (b.c) = 1, ...(d)] = e`. Upstream's patterns
take no parentheses, so each is a parse error (7 ESLint fixtures, `no-useless-rename/056`, `073`, `124` to `129`;
the last five are declared in `packs/javascript/divergences.json`).

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -1251,11 +1251,14 @@ module.exports = grammar({
     pattern: $ => prec.dynamic(-1, choice(
       $._lhs_expression,
       $.rest_pattern,
+      // A destructuring assignment's target may be parenthesized, as an assignment's is: `({a: (b)} = c)`,
+      // `[(a), (b.c) = 1, ...(d)] = e`.
+      $.parenthesized_expression,
     )),
 
     rest_pattern: $ => prec.right(seq(
       '...',
-      $._lhs_expression,
+      choice($._lhs_expression, $.parenthesized_expression),
     )),
 
     method_definition: $ => seq(
```

Node kinds and field names are unchanged. A pattern (a `pair_pattern`'s value, an `array_pattern`'s element, an
`assignment_pattern`'s left side, a `rest_pattern`'s target) may be a `parenthesized_expression`, as an
`assignment_expression`'s left side already may. tree-sitter-javascript has one `pattern` for binding and assignment
patterns, so parenthesized binding patterns, which ECMAScript rejects (`let {a: (b)} = c`, `function f((a)) {}`,
`((a)) => a`), now parse too. The state count is unchanged.

## Patch 7: `yield` may label a statement

Upstream issue: tree-sitter/tree-sitter-javascript#303 (open: `await` and `yield` are not read by context). Present
at tag `v0.25.0` and at HEAD.

Why: outside generators and strict code `yield` is a name, so `yield: while (foo) { … break yield }` is a labeled
statement. Upstream reads `yield` at the start of a statement as the keyword, so the label is a parse error (ESLint
fixtures `no-array-constructor/070`, `no-object-constructor/054`). `var yield = 5` and `break yield` already parsed,
since the keyword is not valid there.

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -503,7 +503,8 @@ module.exports = grammar({
     // ExpressionStatement and ConciseBody lookahead restrictions): `if (a) { b: 1 }` and `() => { b: 1 }` hold a
     // labeled statement. Where only an object parses, as in `{ a: 1, b: 2 }`, GLR keeps the object.
     labeled_statement: $ => prec.dynamic(1, seq(
-      field('label', alias(choice($.identifier, $._reserved_identifier), $.statement_identifier)),
+      // Outside generators and strict code `yield` is a name, so it may label a statement: `yield: while (a) {}`.
+      field('label', alias(choice($.identifier, $._reserved_identifier, 'yield'), $.statement_identifier)),
       ':',
       field('body', $.statement),
     )),
```

Node kinds and field names are unchanged: the label is a `statement_identifier`, as any other is. The generated parser
has 2 more states. Patch 10 reads `yield` as a name in more places.

## Patch 8: a semicolon is inserted before a token that cannot continue the statement

Upstream issue: none open for these. tree-sitter/tree-sitter-javascript#389 (closed unmerged) proposed the
identifier half (`in_point`). tree-sitter/tree-sitter-javascript#354 (open) declares `(`, `[` and `{` as external
tokens for the same purpose. Present at tag `v0.25.0` and at HEAD.

Why: ECMAScript inserts a semicolon at a line break before a token the grammar cannot accept there. The external
scanner instead never inserts one before a line starting with `[`, `(`, `*` or a binary `+` or `-`, and before `in`
or `instanceof` (or an identifier that starts with either, followed by `_`, a digit or `$`). That is right after an
expression, which the line continues (`a⏎[b]` is a subscript), and wrong where no expression precedes:
`var a⏎+b > 0 ? c : d` and `var e⏎[f] = g` after a declarator, and a class field with no initializer before a member
starting with `[` or `*` or named `instanceof` (`h⏎[i]`, `j⏎*k() {}`, `#l⏎instanceof() {}`). Each is a parse error
(6 ESLint fixtures: `no-unused-vars/409`, declared in `packs/javascript/divergences.json`,
`no-unexpected-multiline/033` and `034`, `no-unused-private-class-members/054` to `056`, which the private-member
port now reads as ESLint does).

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -22,6 +22,13 @@ module.exports = grammar({
     $.escape_sequence,
     $.regex_pattern,
     $.jsx_text,
+    // The scanner never returns these: they tell it whether the token can come next, so that it inserts a semicolon
+    // before a line starting with one only where the token cannot continue the statement (`var a⏎[b] = c`).
+    '[',
+    '(',
+    '*',
+    '+',
+    '-',
   ],
 
   extras: $ => [
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -12,6 +12,12 @@ enum TokenType {
     ESCAPE_SEQUENCE,
     REGEX_PATTERN,
     JSX_TEXT,
+    // Never returned: whether the token is valid next (see scan_automatic_semicolon).
+    LEFT_BRACKET,
+    LEFT_PAREN,
+    STAR,
+    PLUS,
+    MINUS,
 };
 
 void *tree_sitter_javascript_external_scanner_create() { return NULL; }
@@ -107,7 +113,10 @@ static WhitespaceResult scan_whitespace_and_comments(TSLexer *lexer, bool *scann
     }
 }
 
-static bool scan_automatic_semicolon(TSLexer *lexer, bool comment_condition, bool *scanned_comment) {
+static bool is_identifier_char(int32_t c) { return iswalnum(c) || c == '_' || c == '$'; }
+
+static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
+    bool comment_condition = !valid_symbols[LOGICAL_OR];
     lexer->result_symbol = AUTOMATIC_SEMICOLON;
     lexer->mark_end(lexer);
 
@@ -157,13 +166,10 @@ static bool scan_automatic_semicolon(TSLexer *lexer, bool comment_condition, boo
         case ',':
         case ':':
         case ';':
-        case '*':
         case '%':
         case '>':
         case '<':
         case '=':
-        case '[':
-        case '(':
         case '?':
         case '^':
         case '|':
@@ -171,26 +177,36 @@ static bool scan_automatic_semicolon(TSLexer *lexer, bool comment_condition, boo
         case '/':
             return false;
 
+        // Insert a semicolon before a token that cannot continue the statement, as ECMAScript does:
+        // `var a⏎[b] = c`, a class field `a⏎*g() {}`. Where the token can, the line continues the statement.
+        case '[':
+            return !valid_symbols[LEFT_BRACKET];
+        case '(':
+            return !valid_symbols[LEFT_PAREN];
+        case '*':
+            return !valid_symbols[STAR];
+
         // Insert a semicolon before decimals literals but not otherwise.
         case '.':
             skip(lexer);
             return iswdigit(lexer->lookahead);
 
-        // Insert a semicolon before `--` and `++`, but not before binary `+` or `-`.
+        // Insert a semicolon before `--` and `++`, and before a `+` or `-` that cannot continue the statement
+        // (`var a⏎+b`), but not before binary `+` or `-`.
         case '+':
             skip(lexer);
-            return lexer->lookahead == '+';
+            return lexer->lookahead == '+' || !valid_symbols[PLUS];
         case '-':
             skip(lexer);
-            return lexer->lookahead == '-';
+            return lexer->lookahead == '-' || !valid_symbols[MINUS];
 
         // Don't insert a semicolon before `!=`, but do insert one before a unary `!`.
         case '!':
             skip(lexer);
             return lexer->lookahead != '=';
 
-        // Don't insert a semicolon before `in` or `instanceof`, but do insert one
-        // before an identifier.
+        // Don't insert a semicolon before `in` or `instanceof`, but do insert one before an identifier, such as
+        // `in_point`, or before an `instanceof` that no binary operator can take here: a class member's name.
         case 'i':
             skip(lexer);
 
@@ -199,7 +215,7 @@ static bool scan_automatic_semicolon(TSLexer *lexer, bool comment_condition, boo
             }
             skip(lexer);
 
-            if (!iswalpha(lexer->lookahead)) {
+            if (!is_identifier_char(lexer->lookahead)) {
                 return false;
             }
 
@@ -210,8 +226,8 @@ static bool scan_automatic_semicolon(TSLexer *lexer, bool comment_condition, boo
                 skip(lexer);
             }
 
-            if (!iswalpha(lexer->lookahead)) {
-                return false;
+            if (!is_identifier_char(lexer->lookahead)) {
+                return !valid_symbols[LOGICAL_OR];
             }
             break;
 
@@ -387,7 +403,7 @@ bool tree_sitter_javascript_external_scanner_scan(void *payload, TSLexer *lexer,
 
     if (valid_symbols[AUTOMATIC_SEMICOLON]) {
         bool scanned_comment = false;
-        bool ret = scan_automatic_semicolon(lexer, !valid_symbols[LOGICAL_OR], &scanned_comment);
+        bool ret = scan_automatic_semicolon(lexer, valid_symbols, &scanned_comment);
         if (!ret && !scanned_comment && valid_symbols[TERNARY_QMARK] && lexer->lookahead == '?') {
             return scan_ternary_qmark(lexer);
         }
```

The five tokens are declared external but never returned: in each parse state the scanner is told whether each can
come next, so it inserts a semicolon before one only where it cannot. A line that starts with `instanceof` gets one
where no binary operator can come next, as no `||` can (the test the scanner already makes for comments, and Patch 6
in `grammars/tree-sitter-typescript/PATCHES.md` makes for members named `in`). `in` is left out: after `for (var a`,
`in` continues the head, and no binary operator can come next there either. A `return` or `yield` followed by a line
break and an operand (`return⏎[a]`) is Patch 15's: there the operand's first token can continue the statement.

Node kinds and field names are unchanged. The generated parser has 2,135 states instead of 1,885: the parse states
the five tokens tell apart can no longer be merged.

## Patch 9: an arrow function's block body ends the statement at a line break

Upstream issue: tree-sitter/tree-sitter-javascript#354 (open), for a line starting with `(`; it reworks
`new_expression` and moves `arrow_function` out of `primary_expression`, which changes trees beyond arrow functions.
Present at tag `v0.25.0` and at HEAD.

Why: an arrow function is no operand in ECMAScript: it cannot be called, indexed, tagged or the left side of a binary
expression, so after its block body a line break ends the statement unless the next line goes on with a list's `,`,
a ternary's `:` or a `;`. Upstream's `arrow_function` is a `primary_expression`, so the next line continues it:
`var foo = () => { return bar }⏎[1, 2, 3].map(foo)` indexes the function with a sequence expression, with no error
(`no-sequences` reported a false positive on ESLint's `arrow-body-style/053`), and `y = () => {}⏎() => {}` and a
class field `a = () => {}⏎*gen() {}` are parse errors (ESLint fixtures `no-unused-vars/439`,
`no-unexpected-multiline/036`).

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -29,6 +29,8 @@ module.exports = grammar({
     '*',
     '+',
     '-',
+    // Never returned either: valid only after an arrow function's block body (see arrow_function).
+    $._arrow_body_end,
   ],
 
   extras: $ => [
@@ -863,10 +865,13 @@ module.exports = grammar({
         $._call_signature,
       ),
       '=>',
-      field('body', choice(
-        $.expression,
-        $.statement_block,
-      )),
+      choice(
+        field('body', $.expression),
+        // An arrow function is no operand, so a line break after its block body ends the statement where the next
+        // line cannot continue an assignment: `f = () => {}⏎[1, 2].map(f)` is two statements, as in ECMAScript.
+        // `_arrow_body_end` marks this place for the scanner.
+        seq(field('body', $.statement_block), optional($._arrow_body_end)),
+      ),
     ),
 
     // Override
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -18,6 +18,8 @@ enum TokenType {
     STAR,
     PLUS,
     MINUS,
+    // Never returned: valid only after an arrow function's block body.
+    ARROW_BODY_END,
 };
 
 void *tree_sitter_javascript_external_scanner_create() { return NULL; }
@@ -161,6 +163,13 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
         return false;
     }
 
+    // An arrow function is no operand: after its block body, a line that does not go on with a list, a ternary's
+    // `:` or a `;` starts a new statement (`f = () => {}⏎[1, 2].map(f)`), where after any other expression it
+    // would continue it.
+    if (valid_symbols[ARROW_BODY_END]) {
+        return lexer->lookahead != ',' && lexer->lookahead != ':' && lexer->lookahead != ';';
+    }
+
     switch (lexer->lookahead) {
         case '`':
         case ',':
```

The marker `_arrow_body_end` follows an arrow function's block body and is never returned, so the scanner is told
when it scans after one. A function expression's body is not marked: `e = function () {}⏎[1]` stays a subscript, as
in ECMAScript.

Node kinds and field names are unchanged: the `body` field holds the same `statement_block`. The statements around it
split as ECMAScript splits them. The generated parser has 20 more states (2,155).

## Patch 10: `yield` is a name where a yield expression cannot parse

Upstream issue: tree-sitter/tree-sitter-javascript#303 (open: `await` and `yield` are not read by context). Present
at tag `v0.25.0` and at HEAD.

Why: outside generators and strict code `yield` is a name, so `yield = 2`, `yield += 1`, `yield++`, `[yield] = a`,
`for (yield of xs);` and `yield.x` are valid scripts. Upstream reads `yield` in an expression only as the keyword. So
the assignments, the update and the `for` head are parse errors. `yield.x` silently reads as a member of an empty
`yield_expression`, which ECMAScript never allows.

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -158,6 +158,7 @@ module.exports = grammar({
     [$.computed_property_name, $.array],
     [$.binary_expression, $._initializer],
     [$.class_static_block, $._property_name],
+    [$.yield_expression, $._yield_identifier],
   ],
 
   word: $ => $.identifier,
@@ -605,12 +606,11 @@ module.exports = grammar({
       $.call_expression,
     ),
 
-    yield_expression: $ => prec.right(seq(
+    yield_expression: $ => prec.right(choice(
+      // A `yield` with an operand outweighs a `yield` name before `++` or `--` (Patch 10): `yield++⏎x` is `yield ++x`.
+      prec.dynamic(3, seq('yield', choice(seq('*', $.expression), $.expression))),
       'yield',
-      choice(
-        seq('*', $.expression),
-        optional($.expression),
-      ))),
+    )),
 
     object: $ => prec('object', seq(
       '{',
@@ -908,7 +908,7 @@ module.exports = grammar({
     )),
 
     member_expression: $ => prec('member', seq(
-      field('object', choice($.expression, $.primary_expression, $.import)),
+      field('object', choice($.expression, $.primary_expression, $.import, $._yield_operand)),
       choice('.', field('optional_chain', $.optional_chain)),
       field('property', choice(
         $.private_property_identifier,
@@ -927,6 +927,7 @@ module.exports = grammar({
       $.subscript_expression,
       $._identifier,
       alias($._reserved_identifier, $.identifier),
+      alias($._yield_identifier, $.identifier),
       $._destructuring_pattern,
     ),
 
@@ -940,6 +941,7 @@ module.exports = grammar({
       $.member_expression,
       $.subscript_expression,
       alias($._reserved_identifier, $.identifier),
+      alias($._yield_identifier, $.identifier),
       $.identifier,
       $.parenthesized_expression,
     ),
@@ -1014,7 +1016,7 @@ module.exports = grammar({
 
     update_expression: $ => prec.left(choice(
       seq(
-        field('argument', $.expression),
+        field('argument', choice($.expression, $._yield_operand)),
         field('operator', choice('++', '--')),
       ),
       seq(
@@ -1316,6 +1318,15 @@ module.exports = grammar({
       ']',
     ),
 
+    // Outside generators and strict code `yield` is a name. The grammar does not know whether a function is a
+    // generator, so `yield` is a name where a yield_expression cannot parse (`yield = 2`, `[yield] = a`,
+    // `for (yield of a)`), and where one would be an operand, which ECMAScript forbids (`yield.x`, `yield++;`).
+    // Where both read (`f(yield)`, `yield + 1`, `yield [0]`, `yield (a)`, and `yield++⏎x`, which a generator reads as
+    // `yield ++x`), the yield_expression stays.
+    _yield_identifier: _ => prec.dynamic(-1, 'yield'),
+
+    _yield_operand: $ => prec.dynamic(2, alias($._yield_identifier, $.identifier)),
+
     _reserved_identifier: _ => choice(
       'get',
       'set',
```

The grammar does not know whether the function around `yield` is a generator. So `yield` is an `identifier` only where
the keyword's reading fails, or where it would be an operand ECMAScript forbids. The first case is an assignment
target (`_lhs_expression`, also in patterns and `for … of` heads) or an augmented assignment target, where no yield
expression parses. The second case is the object of a member access (`yield.x`) or the operand of a postfix update
(`yield++;`). There, an empty yield expression would also parse, so the name's dynamic precedence of 2 outweighs it.
Elsewhere the name's dynamic precedence of -1 keeps the generator's reading wherever both parse: `f(yield)`,
`yield + 1`, `yield;`, `yield [0]` and `yield (a)` stay `yield_expression`s, as in a generator. In a script,
ECMAScript reads them as the name. A yield expression with an operand carries a dynamic precedence of 3, so it also
outweighs the postfix-update name. A generator's `yield++⏎x` is then `yield (++x)`, as ECMAScript reads it, not
`yield++; x;`. `yield()` stays a parse error. A call reading of it would also make a generator's `yield(1) < a` a
call of the yield expression.

Node kinds and field names are unchanged. A `yield` that is a name is an `identifier`, as `var yield` already gave.
The generated parser has 20 more states (2,175). Over the 11,540 files above, no tree changes from Patch 9's.

## Patch 11: the scanner reads every ECMAScript whitespace and line terminator

Upstream issue: none open (tree-sitter/tree-sitter-javascript#135, closed, was U+3000 read as an identifier). Present
at tag `v0.25.0` and at HEAD.

Why: the grammar's extras skip every ECMAScript WhiteSpace and LineTerminator
(`/[\s\p{Zs}\uFEFF\u2028\u2029\u2060\u200B]/`). The external scanner uses C's `iswspace`, which in the C locale
the engine runs in is true only for ASCII whitespace. So where the scanner decides on an automatic semicolon, a
ternary `?` or an HTML comment, a U+3000, U+00A0, U+FEFF or other Unicode space ends its look: an automatic semicolon
is refused before it. `` `　template`　 `` (an ideographic space after the closing backtick, ESLint fixtures
`no-irregular-whitespace/192` and `193`) is a parse error with a missing `;`. The result also depends on the C
library's locale. The scanner also ends a line only at LF, U+2028 and U+2029, not at a lone CR, so `a = 1\rb = 2`
and `a = 1 // c\rb = 2` are parse errors, though the grammar's `//` comment ends at the CR. JSX text's layout test
uses `iswspace` too, so `<a>\n\u00A0</a>` holds a `jsx_text` in the C locale and none in a UTF-8 one.

```diff
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -34,6 +34,40 @@ static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }
 
 static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }
 
+// Whitespace as the grammar's extras read it (`\s`, `\p{Zs}`, U+FEFF, U+2028, U+2029, U+2060, U+200B): every
+// ECMAScript WhiteSpace and LineTerminator, such as U+00A0 and U+3000, whatever the C library's locale says.
+static bool is_space(int32_t c) {
+    switch (c) {
+        case '\t':
+        case '\n':
+        case '\v':
+        case '\f':
+        case '\r':
+        case ' ':
+        case 0x0085:
+        case 0x00A0:
+        case 0x1680:
+        case 0x200B:
+        case 0x2028:
+        case 0x2029:
+        case 0x202F:
+        case 0x205F:
+        case 0x2060:
+        case 0x3000:
+        case 0xFEFF:
+            return true;
+        default:
+            return c >= 0x2000 && c <= 0x200A;
+    }
+}
+
+// An ECMAScript LineTerminator: LF, CR, U+2028 and U+2029, as the grammar's `//` comment ends at.
+static bool is_line_terminator(int32_t c) { return c == '\n' || c == '\r' || c == 0x2028 || c == 0x2029; }
+
+// Whitespace JSX text may hold between lines, as `iswspace` reads it in the C locale: ASCII alone, so a JSX text's
+// range does not depend on the locale (U+00A0 or U+3000 on a line of its own stays text, as React renders it).
+static bool is_jsx_space(int32_t c) { return c == ' ' || (c >= '\t' && c <= '\r'); }
+
 static bool scan_template_chars(TSLexer *lexer) {
     lexer->result_symbol = TEMPLATE_CHARS;
     for (bool has_content = false;; has_content = true) {
@@ -70,7 +104,7 @@ static WhitespaceResult scan_whitespace_and_comments(TSLexer *lexer, bool *scann
     bool saw_block_newline = false;
 
     for (;;) {
-        while (iswspace(lexer->lookahead)) {
+        while (is_space(lexer->lookahead)) {
             skip(lexer);
         }
 
@@ -79,8 +113,7 @@ static WhitespaceResult scan_whitespace_and_comments(TSLexer *lexer, bool *scann
 
             if (lexer->lookahead == '/') {
                 skip(lexer);
-                while (lexer->lookahead != 0 && lexer->lookahead != '\n' && lexer->lookahead != 0x2028 &&
-                       lexer->lookahead != 0x2029) {
+                while (lexer->lookahead != 0 && !is_line_terminator(lexer->lookahead)) {
                     skip(lexer);
                 }
                 *scanned_comment = true;
@@ -99,7 +132,7 @@ static WhitespaceResult scan_whitespace_and_comments(TSLexer *lexer, bool *scann
 
                             break;
                         }
-                    } else if (lexer->lookahead == '\n' || lexer->lookahead == 0x2028 || lexer->lookahead == 0x2029) {
+                    } else if (is_line_terminator(lexer->lookahead)) {
                         saw_block_newline = true;
                         skip(lexer);
                     } else {
@@ -146,11 +179,11 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
             return true;
         }
 
-        if (lexer->lookahead == '\n' || lexer->lookahead == 0x2028 || lexer->lookahead == 0x2029) {
+        if (is_line_terminator(lexer->lookahead)) {
             break;
         }
 
-        if (!iswspace(lexer->lookahead)) {
+        if (!is_space(lexer->lookahead)) {
             return false;
         }
 
@@ -249,7 +282,7 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
 
 static bool scan_ternary_qmark(TSLexer *lexer) {
     for (;;) {
-        if (!iswspace(lexer->lookahead)) {
+        if (!is_space(lexer->lookahead)) {
             break;
         }
         skip(lexer);
@@ -278,7 +311,7 @@ static bool scan_ternary_qmark(TSLexer *lexer) {
 }
 
 static bool scan_html_comment(TSLexer *lexer) {
-    while (iswspace(lexer->lookahead) || lexer->lookahead == 0x2028 || lexer->lookahead == 0x2029) {
+    while (is_space(lexer->lookahead)) {
         skip(lexer);
     }
 
@@ -367,7 +400,7 @@ static bool scan_jsx_text(TSLexer *lexer) {
             at_newline = false;
             continue;
         }
-        bool is_wspace = iswspace(lexer->lookahead);
+        bool is_wspace = is_jsx_space(lexer->lookahead);
         if (lexer->lookahead == '\n') {
             at_newline = true;
         } else {
```

`is_space` is the extras' set, spelled out: ASCII whitespace, U+0085, U+00A0, U+1680, U+2000 to U+200A, U+200B,
U+2028, U+2029, U+202F, U+205F, U+2060, U+3000 and U+FEFF. `is_line_terminator` is LF, CR, U+2028 and U+2029, where
the scanner ends a line for an automatic semicolon, a `//` comment, a block comment's line break and an HTML comment's
line. JSX text keeps its layout rule, with its whitespace spelled out as the C locale's `iswspace` reads it (ASCII
alone), so a `jsx_text`'s range no longer depends on the locale and is the one the engine already built: a U+00A0 or
U+3000 on a line of its own stays text, as React renders it. Node kinds, field names and the grammar are unchanged.
A line break inside a block comment (`a = 1 /* c⏎ */ b = 2`) is Patch 14's.

## Patch 12: a JSX name may be a reserved word or start with `this`

Upstream issue: none open. Present at tag `v0.25.0` and at HEAD. tree-sitter 0.25's reserved words (upstream
`reserved: { global: … }`, added with 0.25.0) bar every reserved word from an `identifier`, JSX names included.

Why: a JSX name is any word: Babel and TypeScript read `<a class="x" for="y" default="z" />`, `<a.default />` and
`<class />`, `<this.props.tag />` names a component through `this`, and `<this />` is an element named `this`. Upstream reads `class`, `for` or `default`
there as the keyword, a parse error that takes the element with it. vscode's colorize fixture `test.jsx` has
`default="World"`. `this` is not a JSX object either, so eslint-plugin-react's `jsx-no-undef/valid-006.jsx`
(`<this.props.tag />`) fails too.

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -714,21 +714,26 @@ module.exports = grammar({
 
     jsx_identifier: _ => /[a-zA-Z_$][a-zA-Z\d_$]*-[a-zA-Z\d_$\-]*/,
 
+    // A JSX name may be any word, reserved or not, as Babel and TypeScript read it: `<a class="x" for="y" />`,
+    // `<a.default />`.
     _jsx_identifier: $ => choice(
       alias($.jsx_identifier, $.identifier),
-      $.identifier,
+      reserved('properties', $.identifier),
     ),
 
     nested_identifier: $ => prec('member', seq(
-      field('object', choice($.identifier, alias($.nested_identifier, $.member_expression))),
+      // `<this.props.tag />` names a component through `this`.
+      field('object', choice(reserved('properties', $.identifier), $.this, alias($.nested_identifier, $.member_expression))),
       '.',
-      field('property', alias($.identifier, $.property_identifier)),
+      field('property', reserved('properties', alias($.identifier, $.property_identifier))),
     )),
 
     jsx_namespace_name: $ => seq($._jsx_identifier, ':', $._jsx_identifier),
 
     _jsx_element_name: $ => choice(
       $._jsx_identifier,
+      // `<this />` is an intrinsic element named `this`, as Babel and TypeScript read it.
+      $.this,
       alias($.nested_identifier, $.member_expression),
       $.jsx_namespace_name,
     ),
```

Each identifier of a JSX name is read under the empty `properties` reserved set, as a member expression's property
already is. `this` may be an element's name or the object of a JSX member expression.

Node kinds and field names are unchanged. A reserved word in a JSX name is an `identifier` (a `property_identifier` as
an attribute name or a member's property), as any other word is. `this` there is a `this` node, as it is in an
expression. The generated parser has the same number of states.

## Patch 13: a comment between a JSX closing tag's `<` and `/`

Upstream issue: none open. Present at tag `v0.25.0` and at HEAD.

Why: Babel and espree read `<` and `/` of a closing tag as two tokens, so a comment may stand between them:
`<></* c *//>` (eslint-plugin-react's `jsx-no-comment-textnodes/valid-012.jsx`). Upstream's `</` is one token, and
the lexer prefers it to `<` at `</* c */`, so the comment's `/*` is split and the element is a parse error.
TypeScript 5.9 does not allow this (TS1003, "Identifier expected"), but ESLint parses `.js` and `.jsx` with espree, and
the fixture is valid there.

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -31,6 +31,8 @@ module.exports = grammar({
     '-',
     // Never returned either: valid only after an arrow function's block body (see arrow_function).
     $._arrow_body_end,
+    // The `<` of a JSX closing tag whose `/` a comment follows (see jsx_closing_element).
+    $._jsx_closing_tag_lt,
   ],
 
   extras: $ => [
@@ -739,7 +741,9 @@ module.exports = grammar({
     ),
 
     jsx_closing_element: $ => seq(
-      '</',
+      // The lexer reads `</` as one token, so a comment between the `<` and the `/` (`<></* c *//>`), which
+      // Babel and espree allow, needs the scanner: it reads that `<` alone, and the comment is an extra.
+      choice('</', seq(alias($._jsx_closing_tag_lt, '<'), '/')),
       optional(field('name', $._jsx_element_name)),
       '>',
     ),
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -20,6 +20,8 @@ enum TokenType {
     MINUS,
     // Never returned: valid only after an arrow function's block body.
     ARROW_BODY_END,
+    // The `<` of a JSX closing tag whose `/` a comment follows: `<></* c *//>`.
+    JSX_CLOSING_TAG_LT,
 };
 
 void *tree_sitter_javascript_external_scanner_create() { return NULL; }
@@ -310,6 +312,18 @@ static bool scan_ternary_qmark(TSLexer *lexer) {
     return false;
 }
 
+// The rest of an HTML comment's line, once its `<!--` or `-->` is read.
+static bool scan_html_comment_rest(TSLexer *lexer) {
+    while (lexer->lookahead != 0 && !is_line_terminator(lexer->lookahead)) {
+        advance(lexer);
+    }
+
+    lexer->result_symbol = HTML_COMMENT;
+    lexer->mark_end(lexer);
+
+    return true;
+}
+
 static bool scan_html_comment(TSLexer *lexer) {
     while (is_space(lexer->lookahead)) {
         skip(lexer);
@@ -336,15 +350,7 @@ static bool scan_html_comment(TSLexer *lexer) {
         return false;
     }
 
-    while (lexer->lookahead != 0 && lexer->lookahead != '\n' && lexer->lookahead != 0x2028 &&
-           lexer->lookahead != 0x2029) {
-        advance(lexer);
-    }
-
-    lexer->result_symbol = HTML_COMMENT;
-    lexer->mark_end(lexer);
-
-    return true;
+    return scan_html_comment_rest(lexer);
 }
 
 // After an `&`: is what follows the rest of an HTML character reference
@@ -443,6 +449,29 @@ bool tree_sitter_javascript_external_scanner_scan(void *payload, TSLexer *lexer,
         return true;
     }
 
+    // A `<` before `/` and a comment, where a JSX closing tag may start: the `<` alone, so the lexer does not read
+    // `</` across the comment. Any other `<` is the lexer's, but for an HTML comment's `<!--`, read here as below.
+    if (valid_symbols[JSX_CLOSING_TAG_LT] && lexer->lookahead == '<') {
+        advance(lexer);
+        if (lexer->lookahead == '/') {
+            lexer->mark_end(lexer);
+            advance(lexer);
+            lexer->result_symbol = JSX_CLOSING_TAG_LT;
+            return lexer->lookahead == '*';
+        }
+        if (valid_symbols[HTML_COMMENT] && !valid_symbols[LOGICAL_OR] && !valid_symbols[ESCAPE_SEQUENCE] &&
+            !valid_symbols[REGEX_PATTERN]) {
+            for (unsigned i = 0; i < 3; i++) {
+                if (lexer->lookahead != "!--"[i]) {
+                    return false;
+                }
+                advance(lexer);
+            }
+            return scan_html_comment_rest(lexer);
+        }
+        return false;
+    }
+
     if (valid_symbols[AUTOMATIC_SEMICOLON]) {
         bool scanned_comment = false;
         bool ret = scan_automatic_semicolon(lexer, valid_symbols, &scanned_comment);
```

The lexer still reads `</` as one token wherever no comment follows the `<`. Where a JSX closing tag may start and the
next characters are `<`, `/` and `*`, the scanner reads the `<` alone (`_jsx_closing_tag_lt`). The comment is then an
extra and the `/` its own token. The scanner already looked at a `<` there for an HTML comment's `<!--`, which it
still reads. A comment between the `/` and the name (`</ /* c */ a>`) already parsed.

Node kinds and field names are unchanged. A closing tag with such a comment has the anonymous children `<` and `/`
around a `comment`, where any other has `</`. The generated parser has 32 more states (2,207).

## Patch 14: a block comment holding a line break ends the line

Upstream issue: none open. Present at tag `v0.25.0` and at HEAD.

Why: ECMAScript reads a multi-line comment as a line terminator for automatic semicolon insertion, so
`a = 1 /*⏎*/ b = 2` is two statements and `x /*⏎*/ ++y` is `x; ++y;`. Upstream's scanner sees the line break inside a
comment it looks past, but uses it only where no binary operator can come next, the comment condition. After an
expression it reads on as if the comment held no line break. It returns no semicolon, the comment is lexed as an
extra, and the line break inside it is lost to the scanner's next call. So `a = 1 /*⏎*/ b = 2` is a parse error, and
so is `x /*⏎*/ ++y`.

```diff
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -97,6 +97,8 @@ typedef enum {
     REJECT,     // Semicolon is illegal, ie a syntax error occurred
     NO_NEWLINE, // Unclear if semicolon will be legal, continue
     ACCEPT,     // Semicolon is legal, assuming a comment was encountered
+    // A block comment holding a line terminator, which ECMAScript reads as one: semicolon legal as after a line break.
+    BLOCK_LINE_TERMINATOR,
 } WhitespaceResult;
 
 /**
@@ -129,7 +131,7 @@ static WhitespaceResult scan_whitespace_and_comments(TSLexer *lexer, bool *scann
                             *scanned_comment = true;
 
                             if (lexer->lookahead != '/' && !consume) {
-                                return saw_block_newline ? ACCEPT : NO_NEWLINE;
+                                return saw_block_newline ? BLOCK_LINE_TERMINATOR : NO_NEWLINE;
                             }
 
                             break;
@@ -152,6 +154,8 @@ static WhitespaceResult scan_whitespace_and_comments(TSLexer *lexer, bool *scann
 
 static bool is_identifier_char(int32_t c) { return iswalnum(c) || c == '_' || c == '$'; }
 
+static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment);
+
 static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
     bool comment_condition = !valid_symbols[LOGICAL_OR];
     lexer->result_symbol = AUTOMATIC_SEMICOLON;
@@ -168,9 +172,17 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
                 return false;
             }
 
-            if (result == ACCEPT && comment_condition && lexer->lookahead != ',' && lexer->lookahead != '=') {
+            bool block_line_terminator = result == BLOCK_LINE_TERMINATOR;
+            if ((result == ACCEPT || block_line_terminator) && comment_condition && lexer->lookahead != ',' &&
+                lexer->lookahead != '=') {
                 return true;
             }
+
+            // A block comment holding a line terminator is one, as ECMAScript reads it (`a = 1 /*⏎*/ b = 2`): the line
+            // ends there. (After a `//` comment, the scanner is called again at the line break it ends at.)
+            if (block_line_terminator) {
+                return scan_after_line_terminator(lexer, valid_symbols, scanned_comment);
+            }
         }
 
         if (lexer->lookahead == '}') {
@@ -193,7 +205,11 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
     }
 
     skip(lexer);
+    return scan_after_line_terminator(lexer, valid_symbols, scanned_comment);
+}
 
+// After a line terminator: whether the token starting the next line ends the statement before it.
+static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
     if (scan_whitespace_and_comments(lexer, scanned_comment, true) == REJECT) {
         return false;
     }
```

Where a block comment holds a line terminator and the comment condition does not already decide, the scanner goes
on as after a line break, with the rule for the token that starts the next line (`scan_after_line_terminator`, the
second half of `scan_automatic_semicolon`). So the token decides as it does after a plain line break: `a /*⏎*/ [b]`
and `a /*⏎*/ (b)` stay one expression, as in ECMAScript, and `a /*⏎*/ b` is two. A `//` comment keeps the old path:
the scanner is called again at the line break the comment ends at, so the comment stays inside the statement before
it. Upstream's corpus case `Regexps` (`/a///comment`) holds such a comment.

Node kinds, field names and the grammar are unchanged. Where the semicolon goes in before such a comment, the comment
is an extra between the two statements, as it was where the comment condition already inserted one
(`var a /*⏎*/ +b`). The state count is unchanged.

## Patch 15: no line terminator after `return` or `yield`

Upstream issue: none open. Present at tag `v0.25.0` and at HEAD.

Why: `return` and `yield` are restricted productions in ECMAScript: no line terminator may stand before the operand, so
a line break there ends the statement. `return⏎[a]`, `return⏎(a)`, `return⏎-1`, `return⏎+a`, ``return⏎`t` ``,
`return⏎/re/` and `return⏎<div/>` are `return;` followed by an expression statement. In a generator, `yield⏎(a)`,
`yield⏎[a]` and `yield⏎-1` are `yield;` followed by one. Upstream's scanner inserts a semicolon there only before a
token that cannot continue the statement, such as an identifier (`return⏎a` was already right). It read each of these
as the operand, a silently wrong tree: the function returns `[a]` where ECMAScript returns `undefined`.

```diff
--- a/grammar.js
+++ b/grammar.js
@@ -33,6 +33,9 @@ module.exports = grammar({
     $._arrow_body_end,
     // The `<` of a JSX closing tag whose `/` a comment follows (see jsx_closing_element).
     $._jsx_closing_tag_lt,
+    // Never returned: marks where ECMAScript allows no line terminator before an operand, after `return` and
+    // `yield`, so that the scanner ends the statement at a line break there (`return⏎[a]` is `return; [a];`).
+    $._no_line_terminator_here,
   ],
 
   extras: $ => [
@@ -499,6 +502,7 @@ module.exports = grammar({
 
     return_statement: $ => seq(
       'return',
+      optional($._no_line_terminator_here),
       optional($._expressions),
       $._semicolon,
     ),
@@ -610,7 +614,7 @@ module.exports = grammar({
 
     yield_expression: $ => prec.right(choice(
       // A `yield` with an operand outweighs a `yield` name before `++` or `--` (Patch 10): `yield++⏎x` is `yield ++x`.
-      prec.dynamic(3, seq('yield', choice(seq('*', $.expression), $.expression))),
+      prec.dynamic(3, seq('yield', optional($._no_line_terminator_here), choice(seq('*', $.expression), $.expression))),
       'yield',
     )),
 
--- a/src/scanner.c
+++ b/src/scanner.c
@@ -22,6 +22,9 @@ enum TokenType {
     ARROW_BODY_END,
     // The `<` of a JSX closing tag whose `/` a comment follows: `<></* c *//>`.
     JSX_CLOSING_TAG_LT,
+    // Never returned: valid right after `return` and `yield`, where ECMAScript allows no line terminator before the
+    // operand (see scan_after_line_terminator).
+    NO_LINE_TERMINATOR_HERE,
 };
 
 void *tree_sitter_javascript_external_scanner_create() { return NULL; }
@@ -210,8 +213,14 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
 
 // After a line terminator: whether the token starting the next line ends the statement before it.
 static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
+    // After `return` or `yield`, ECMAScript allows no line terminator before the operand: a line break there ends the
+    // statement before any token that can start an expression (`return⏎[a]`, `yield⏎(a)`, `return⏎/a/`). A token that
+    // cannot (`,`, `;`, `=`, a binary operator) goes on as before.
+    bool restricted = valid_symbols[NO_LINE_TERMINATOR_HERE];
+
     if (scan_whitespace_and_comments(lexer, scanned_comment, true) == REJECT) {
-        return false;
+        // A `/` that starts no comment: a regular expression where an operand may start.
+        return restricted;
     }
 
     // An arrow function is no operand: after its block body, a line that does not go on with a list, a ternary's
@@ -222,13 +231,11 @@ static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols
     }
 
     switch (lexer->lookahead) {
-        case '`':
         case ',':
         case ':':
         case ';':
         case '%':
         case '>':
-        case '<':
         case '=':
         case '?':
         case '^':
@@ -237,14 +244,19 @@ static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols
         case '/':
             return false;
 
+        // A template or a JSX element starts an operand.
+        case '`':
+        case '<':
+            return restricted;
+
         // Insert a semicolon before a token that cannot continue the statement, as ECMAScript does:
         // `var a⏎[b] = c`, a class field `a⏎*g() {}`. Where the token can, the line continues the statement.
         case '[':
-            return !valid_symbols[LEFT_BRACKET];
+            return restricted || !valid_symbols[LEFT_BRACKET];
         case '(':
-            return !valid_symbols[LEFT_PAREN];
+            return restricted || !valid_symbols[LEFT_PAREN];
         case '*':
-            return !valid_symbols[STAR];
+            return restricted || !valid_symbols[STAR];
 
         // Insert a semicolon before decimals literals but not otherwise.
         case '.':
@@ -252,13 +264,13 @@ static bool scan_after_line_terminator(TSLexer *lexer, const bool *valid_symbols
             return iswdigit(lexer->lookahead);
 
         // Insert a semicolon before `--` and `++`, and before a `+` or `-` that cannot continue the statement
-        // (`var a⏎+b`), but not before binary `+` or `-`.
+        // (`var a⏎+b`) or starts a restricted operand (`return⏎-1`), but not before binary `+` or `-`.
         case '+':
             skip(lexer);
-            return lexer->lookahead == '+' || !valid_symbols[PLUS];
+            return lexer->lookahead == '+' || restricted || !valid_symbols[PLUS];
         case '-':
             skip(lexer);
-            return lexer->lookahead == '-' || !valid_symbols[MINUS];
+            return lexer->lookahead == '-' || restricted || !valid_symbols[MINUS];
 
         // Don't insert a semicolon before `!=`, but do insert one before a unary `!`.
         case '!':
```

A never-returned external token, `_no_line_terminator_here`, marks the place after `return` and after `yield` before its
operand. After a line break there, the scanner inserts a semicolon before every token that can start an expression:
`` ` ``, `<`, `[`, `(`, `*`, `+`, `-`, `.` before a digit and a `/` that starts no comment. Other tokens already got one.
A token that cannot start one, such as `,`, `;`, `=`, `?` or a binary operator, goes on as before, so
`x = yield⏎, b` stays a sequence and `return⏎;` one statement. `yield⏎*h()` is now a parse error, as in ECMAScript.

`yield⏎(a)` in a script, where `yield` is a name, is a call in ECMAScript. Like Patch 10, the grammar reads the
generator's `yield;` there. Before this patch it read neither: it gave a yield of `(a)`. `break⏎l` and `continue⏎l`, the
other restricted productions with an operand, already ended at the line break: a label is an identifier, before which
the scanner always inserts a semicolon. `a⏎++b` was already `a; ++b`. `throw⏎a`, which ECMAScript rejects, and
`x⏎=> x`, which it rejects too, still parse as before.

Node kinds and field names are unchanged. The generated parser has 26 more states (2,233).

## Corpus and tree comparison

With all fifteen patches, upstream's corpus parses as before but for Patch 4's `Objects` case (115 of 116). With that
case's expectation changed and one case added per patch (Patch 2's taken from #388), 131 of 131 parse. Over the
11,540 files above, 56 files that had syntax errors parse. They are all 13 Next.js plugin fixtures and 12
next-examples files that failed, 5 eslint-plugin-react fixtures, a shadcn-ui test fixture, vscode's colorize
`test.jsx` and 24 ESLint fixtures. No file gains an error. 9 trees change, each as a patch intends. 3 change by
Patch 4 (a `{` at statement start or as an arrow body). 5 change by Patch 9 (`arrow-body-style/053`, `054`,
`no-unexpected-multiline/035`, `no-unused-vars/430`, `431`). The last is the error recovery of next-examples'
`with-custom-babel-config/pages/index.js` (a Babel `do` expression), which has a syntax error before and after.
espree 11.2.0 reads every changed case as the patched grammar does. Patches 10 to 13 change no tree but the 5 files
they fix: `no-irregular-whitespace/192` and `193`, `test.jsx`, `jsx-no-undef/valid-006` and
`jsx-no-comment-textnodes/valid-012`. Patches 14 and 15 change no tree of these files. Both comparisons parse in the C
locale, as the engine does (Python sets a UTF-8 locale, under which the scanner's old `iswspace` hid Patch 11's
failures). The files that still fail to parse are invalid JavaScript: Flow annotations, a Babel `do` expression,
and truncated or deliberately broken fixtures.
