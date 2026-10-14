# tree-sitter-typescript: vendored with patches

Upstream: https://github.com/tree-sitter/tree-sitter-typescript, tag `v0.23.2`
(commit `f975a621f4e7f532fe322e13c4f79495e0a7b2e7`), the version `ast-grep-language` 0.45.3 pins.
tree-sitter and its grammars accept contributions only from maintainers, so Rerail carries its
fixes here instead of upstream.

## Files

| File | Origin |
| --- | --- |
| `LICENSE` | upstream, verbatim |
| `common/scanner.h` | upstream with Patches 6, 7, 12, 13, 20 and 21 |
| `typescript/src/scanner.c`, `tsx/src/scanner.c` | upstream, verbatim |
| `common/tree_sitter/{alloc,array,parser}.h` | written by `tree-sitter generate` 0.27.0 (one copy for both grammars) |
| `typescript/src/grammar.json`, `tsx/src/grammar.json` | regenerated from the patched `common/define-grammar.js` |

Upstream's `src/parser.c` (8 MB per grammar) is not vendored: `native/build.rs` generates each
parser from its `grammar.json` with tree-sitter-generate 0.27.0, as it does Blade's.

## Regenerating

```sh
git clone --branch v0.23.2 https://github.com/tree-sitter/tree-sitter-typescript
cd tree-sitter-typescript
npm install --no-save --ignore-scripts tree-sitter-javascript@0.23.1   # package.json's ^0.23.1
git apply <the patches below>
(cd typescript && tree-sitter generate)   # tree-sitter-cli 0.27.0
(cd tsx && tree-sitter generate)
```

then copy `{typescript,tsx}/src/grammar.json` and `common/scanner.h` here. Unpatched, this reproduces upstream's
`grammar.json` but for the `"reserved": {}` key tree-sitter 0.25 added.

## Patch 1: a postfix `!` binds at member precedence

Upstream issue: tree-sitter/tree-sitter-typescript#299 (open; also at upstream HEAD `75b3874`).
Mirrors: WillBooster/tree-sitter-typescript#31 (merged as `f0405ff`, released in 1.2.7), its
`non_null_expression` change only.

Why: upstream parses `non_null_expression` as `expression '!'` at unary precedence, so a postfix
`!` asserts the whole binary, unary or `new` expression before it: `a - b!` is
`(a - b)!`, `n < x!.y` is `(n < x)!.y`, `-x!` is `(-x)!`, `new A!()` is `(new A)!()`.
TypeScript parses `!` inside the member access chain (`parseMemberExpressionRest`), so it asserts
only the operand: `a - (b!)`, `n < (x!).y`. On vscode@729f257, 298 `!` sites in 172 files were
misplaced.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -330,8 +330,8 @@ module.exports = function defineGrammar(dialect) {
         seq('export', 'as', 'namespace', $.identifier, $._semicolon),
       ),
 
-      non_null_expression: $ => prec.left('unary', seq(
-        $.expression, '!',
+      non_null_expression: $ => prec('member', seq(
+        choice($.primary_expression, $.new_expression), '!',
       )),
 
       variable_declarator: $ => choice(
```

The fork's rule is `prec('member', seq($.primary_expression, '!'))`, alongside a rework that makes
a `new` with arguments a primary expression. Rerail keeps 0.23.2's `new_expression` and admits it as
the operand instead, as 0.23.2's `call_expression` does for its template-call `function`; without
it, `new Foo()!` and `new Foo()!.bar` would not parse. Both grammars build the trees TypeScript
does for every non-null case in the fork's corpus and for `new Foo()!`.

Node kinds and field names are unchanged. `non_null_expression`'s one child, formerly any
`expression`, is now a `primary_expression` or a `new_expression`. Inputs TypeScript rejects
because `!` follows a non-member expression (`foo++!`, `x as Y!`) now parse with an error, as in
the fork.

## Patch 2: type arguments follow a primary expression

Upstream issue: none open at tree-sitter/tree-sitter-typescript (present at tag `v0.23.2` and at
HEAD `75b3874`). The fork's issue is WillBooster/tree-sitter-typescript#13.
Mirrors: WillBooster/tree-sitter-typescript#22 (merged as `35573c1`). The fork's tagged-template
alternative is left out, because 0.23.2 has no tagged template with type arguments.

Why: upstream's `call_expression` takes any `expression` before its optional type arguments, and
`instantiation_expression` does the same. Four declared conflicts with `binary_expression`,
`unary_expression`, `update_expression` and `await_expression` leave both readings to GLR, and GLR
keeps the wrong one. So a prefix operator, `await` or a binary operator before a generic call or an
instantiation becomes part of its callee: `!q<H>('x')` is `(!q)<H>('x')`, `await f<T>()` is
`(await f)<T>()`, `a + b<T>(c)` is `(a + b)<T>(c)`, and `a + b<T>` is `(a + b)<T>`. Every prefix
operator does this (`!`, `-`, `+`, `~`, `typeof`, `void`, `delete`, `++`, `--`, `await`), in
TypeScript and in TSX. `yield` and `new` already parsed right. TypeScript parses type arguments
only after a member expression (`parseCallExpressionRest`), so it calls `q`, `f` and `b`. The same
GLR choice also read some generic calls as comparisons:
`managedPermissionsPolicy<Record<string, boolean>>(…)` became comparisons around a `>>` shift, and
`for (let i = 1; i < unique.length; i++)` failed to parse. vscode@729f257 had 1,045 calls of a
prefix-operator callee in 289 files, and a private web application had 4 in 3 files; with the
patch both have none. Syntax-error files drop from 14 to 13 on vscode and from 13 to 4 on the
private application (`await f<typeof import("./api")>()` now parses), and no file gains an error.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -15,6 +15,7 @@ module.exports = function defineGrammar(dialect) {
     ]),
 
     precedences: ($, previous) => previous.concat([
+      ['new', 'generic_call', 'instantiation', 'unary', 'binary'],
       [
         'call',
         'instantiation',
@@ -71,10 +72,7 @@ module.exports = function defineGrammar(dialect) {
     ]),
 
     conflicts: ($, previous) => previous.concat([
-      [$.call_expression, $.instantiation_expression, $.binary_expression],
-      [$.call_expression, $.instantiation_expression, $.binary_expression, $.unary_expression],
-      [$.call_expression, $.instantiation_expression, $.binary_expression, $.update_expression],
-      [$.call_expression, $.instantiation_expression, $.binary_expression, $.await_expression],
+      [$.expression, $.call_expression, $.instantiation_expression],
 
       // This appears to be necessary to parse a parenthesized class expression
       [$.class],
@@ -173,9 +171,15 @@ module.exports = function defineGrammar(dialect) {
       call_expression: $ => choice(
         prec('call', seq(
           field('function', choice($.expression, $.import)),
-          field('type_arguments', optional($.type_arguments)),
           field('arguments', $.arguments),
         )),
+        // Type arguments follow a primary expression, as in TypeScript: `!f<T>(x)` and `a + f<T>(x)` call `f`.
+        // Where comparisons could also read the input, as in `a < b > (c)`, TypeScript reads a generic call.
+        prec.dynamic(1, prec('generic_call', seq(
+          field('function', choice($.primary_expression, $.new_expression)),
+          field('type_arguments', $.type_arguments),
+          field('arguments', $.arguments),
+        ))),
         prec('template_call', seq(
           field('function', choice($.primary_expression, $.new_expression)),
           field('arguments', $.template_string),
@@ -495,10 +499,10 @@ module.exports = function defineGrammar(dialect) {
         $.type,
       )),
 
-      instantiation_expression: $ => prec('instantiation', seq(
-        $.expression,
+      instantiation_expression: $ => prec.dynamic(-1, prec('instantiation', seq(
+        choice($.primary_expression, $.new_expression),
         field('type_arguments', $.type_arguments),
-      )),
+      ))),
 
       class_heritage: $ => choice(
         seq($.extends_clause, optional($.implements_clause)),
@@ -1061,8 +1065,10 @@ module.exports = function defineGrammar(dialect) {
       ),
       readonly_type: $ => seq('readonly', $.type),
 
-      union_type: $ => prec.left(seq(optional($.type), '|', $.type)),
-      intersection_type: $ => prec.left(seq(optional($.type), '&', $.type)),
+      // TypeScript allows a leading `|` or `&` only at the start of a type, so `a < b || c > (d)` compares instead
+      // of calling `a` with `b | | c`. The negative dynamic precedence outweighs that of the generic call.
+      union_type: $ => prec.left(choice(seq($.type, '|', $.type), prec.dynamic(-2, seq('|', $.type)))),
+      intersection_type: $ => prec.left(choice(seq($.type, '&', $.type), prec.dynamic(-2, seq('&', $.type)))),
 
       function_type: $ => prec.left(seq(
         field('type_parameters', optional($.type_parameters)),
```

As in the fork, a generic call's callee and an instantiation's operand are a `primary_expression`
or a `new_expression`. That removes the ambiguity at its root, so the four conflicts become one.
The generic call's dynamic precedence of 1 and the instantiation's -1 decide `a < b > (c)` as a
call, which is how TypeScript reads it. The leading `|` and `&` of a type get -2, so
`a < b || c > (d)` stays a comparison. Without that, the generic call's precedence reads it as
`a<b | | c>(d)`.

Node kinds and field names are unchanged. A generic call's `function` field and an
`instantiation_expression`'s child, formerly any `expression`, are now a `primary_expression` or a
`new_expression`. A call without type arguments keeps any `expression` as its callee. Each
generated parser has 5,331 states instead of 5,878, and parsing vscode takes 6.2 s of CPU instead
of 6.4 s.

## Patch 3: a bare `&` in a JSX attribute string is text

Upstream issues: tree-sitter/tree-sitter-typescript#279 (closed by tree-sitter/tree-sitter-javascript#305, which
admits an `&` followed by neither `#` nor a letter, and so fixes `class="a & b"` only) and
tree-sitter/tree-sitter-javascript#304 (closed with it). Present at tag `v0.23.2` and at tree-sitter-javascript HEAD
`58404d8`. The open tree-sitter-javascript#388 and tree-sitter-typescript#371 make a bare `&` text in JSX children
(`<p>Shipping & Handling</p>`, tree-sitter-javascript#366), through the external scanner; neither touches attribute
strings, and Rerail carries the typescript one as Patch 7.

Why: a JSX attribute string may hold `&` as text. TypeScript and Babel decode a complete character reference (`&amp;`,
`&#38;`, `&#x26;`) and read any other `&` literally. Upstream's string fragment stops at an `&` before `#` or a letter,
and the only other token there is `html_character_reference`, which needs the closing `;`. So `src="/a?w=8&h=8"` or
`href="…?family=Inter&display=swap"` is a parse error, and the element around it is lost: on shadcn-ui@0e3abd6,
`@next/next/no-img-element`'s port found 30 of the plugin's 123 findings (all 93 misses held such an `&`), and 21
of its 3,421 files had syntax errors from it.
The fragment's escape for any other `&` also admitted the closing quote, so `title="Q&"` ran on past its end.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -261,6 +261,37 @@ module.exports = function defineGrammar(dialect) {
         ),
       ),
 
+      // A bare `&` in a JSX string is text, as TypeScript and Babel read it: `src="/a?w=8&h=8"`, `title="Q&"`.
+      // Only a complete character reference, such as `&amp;`, is an html_character_reference.
+      _jsx_string: $ => choice(
+        seq(
+          '"',
+          repeat(choice(
+            alias($.unescaped_double_jsx_string_fragment, $.string_fragment),
+            $.html_character_reference,
+            alias($._jsx_string_ampersand, $.string_fragment),
+          )),
+          '"',
+        ),
+        seq(
+          '\'',
+          repeat(choice(
+            alias($.unescaped_single_jsx_string_fragment, $.string_fragment),
+            $.html_character_reference,
+            alias($._jsx_string_ampersand, $.string_fragment),
+          )),
+          '\'',
+        ),
+      ),
+
+      unescaped_double_jsx_string_fragment: _ => token.immediate(prec(1, /([^"&]|&[^"#A-Za-z])+/)),
+
+      unescaped_single_jsx_string_fragment: _ => token.immediate(prec(1, /([^'&]|&[^'#A-Za-z])+/)),
+
+      // An `&` that no character reference completes: the lexer reads `&amp;` whole and falls back
+      // to this `&` when the reference breaks off, as at `&h=8`.
+      _jsx_string_ampersand: _ => token.immediate(/&/),
+
       // This rule is only referenced by expression when the dialect is 'tsx'
       jsx_opening_element: $ => prec.dynamic(-1, seq(
         $._jsx_start_opening_element,
```

The added `&` token is read only where the lexer cannot complete a character reference: it reads on from `&` while a
reference can still match (both have precedence 0), accepts `&amp;` whole, and falls back to the lone `&` when the
reference breaks off, as at `&h=8` or `&#;`. The fragment keeps its precedence of 1, so `a & b` stays one fragment.
The other two rules are tree-sitter-javascript 0.23.1's, unchanged but for the closing quote excluded after `&`.

Node kinds and field names are unchanged. A bare `&` is its own `string_fragment`, so `"/a?w=8&h=8"` is a `string`
of three fragments, `/a?w=8`, `&` and `h=8`, as `"a&amp;b"` was already three children. The TypeScript dialect's
parser is byte-identical (it has no JSX); the TSX parser has the same 5,580 states and one more token. Upstream's
corpus parses as before (111 of 112, the one failure patch 1's `foo++!`), and so does tree-sitter-javascript 0.23.1's
under TSX. A bare `&` in JSX children (`<p>Terms & Conditions</p>`) was still a parse error, the other 82 of
shadcn-ui's syntax-error files, until Patch 7.

## Patch 4: variance annotations on type parameters

Upstream issue: tree-sitter/tree-sitter-typescript#370 (open; present at tag `v0.23.2` and at upstream HEAD).
Mirrors: tree-sitter/tree-sitter-typescript#364 (open), with the follow-up its review proposes
(brajevicm/tree-sitter-typescript@9fcaa13) so `out` alone still names a parameter. tree-sitter/tree-sitter-typescript#361
(open, draft) adds the same syntax as a named `variance` node instead.

Why: TypeScript 4.7 allows `in`, `out` and `in out` before a type parameter (`interface Producer<out T>`,
`class Box<in out T>`), after `const` when both appear. Upstream's `type_parameter` has only `const`, so every
annotated parameter is a parse error that takes its whole interface, class or type alias with it (NER-1791;
botpress's `packages/zui/src/z/typings.ts` declares `interface IZodObject<…, out T extends ZodRawShape = …>`).
With only #364's change, `out` becomes a keyword at the start of a type parameter and `type F<out> = out[]` stops
parsing, so the name also accepts `out`, as TypeScript reads it before `>`, `,` or `=`.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -1039,7 +1039,9 @@ module.exports = function defineGrammar(dialect) {
 
       type_parameter: $ => seq(
         optional('const'),
-        field('name', $._type_identifier),
+        optional(choice('in', seq(optional('in'), 'out'))),
+        // `out` before `>`, `,` or `=` is the parameter's name, as in TypeScript: `type F<out> = out[]`.
+        field('name', choice($._type_identifier, alias('out', $.type_identifier))),
         field('constraint', optional($.constraint)),
         field('value', optional($.default_type)),
       ),
```

`out in T` stays a parse error (TypeScript's TS1029, "'in' modifier must precede 'out' modifier").

Node kinds and field names are unchanged. The annotations are anonymous tokens, as `const` is, so a type parameter's
named children are as before, and `out` is one more anonymous kind. A parameter named `out` is a `type_identifier`, as
any other name is. Each generated parser has 13 more states (5,344 for TypeScript, 5,593 for TSX).

## Patch 5: a function type's parameter may be named by a type keyword

Upstream issue: none open at tree-sitter/tree-sitter-typescript (present at tag `v0.23.2` and at upstream HEAD).

Why: TypeScript reads `(any) => void` as a function type with one parameter named `any`, as it does
`(foo) => void` (an error only under `noImplicitAny`, TS7051, not a syntax error). Upstream reads `(any)` as a
`parenthesized_type` and fails at `=>` for `any`, `unknown`, `number`, `string`, `boolean`, `never`, `object`,
`symbol` and `undefined`: the precedences `[literal_type, pattern]` and `[predefined_type, pattern]` decide `(kw)`
before the `)` is past, and `unknown` and `never` are missing from the words a parameter may be named by. So
`Record<number, (any) => void>`, `(unknown, x) => void` and `interface I { n: (string) => void }` are parse errors
(NER-1804); `(bigint) => void` parsed only because `bigint` is no keyword of the grammar.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -62,8 +62,6 @@ module.exports = function defineGrammar(dialect) {
       [$.type_query, $.primary_expression],
       [$.override_modifier, $.primary_expression],
       [$.decorator_call_expression, $.decorator],
-      [$.literal_type, $.pattern],
-      [$.predefined_type, $.pattern],
       [$.call_expression, $._type_query_call_expression],
       [$.call_expression, $._type_query_call_expression_in_type_annotation],
       [$.new_expression, $.primary_expression],
@@ -94,6 +92,12 @@ module.exports = function defineGrammar(dialect) {
       [$.primary_expression, $.pattern, $.primary_type],
       [$._parameter_name, $.primary_type],
       [$.pattern, $.primary_type],
+      // `(any) => void` and `(any)` read alike up to the `)`: GLR keeps both, so a function type's parameter may be
+      // named by a type keyword, as in TypeScript, and `=>` decides.
+      [$.literal_type, $.pattern],
+      [$.primary_expression, $.pattern, $.literal_type],
+      [$.predefined_type, $.pattern],
+      [$.primary_expression, $.pattern, $.predefined_type],
 
       [$.optional_tuple_parameter, $.primary_type],
       [$.rest_pattern, $.primary_type, $.primary_expression],
@@ -1129,6 +1133,8 @@ module.exports = function defineGrammar(dialect) {
         'symbol',
         'export',
         'object',
+        'unknown',
+        'never',
         'new',
         'readonly',
         previous,
```

The two precedences become conflicts, so GLR keeps both readings of `(kw)` until the token after `)` decides: `=>`
keeps the function type, anything else the parenthesized type. The generator then also reports each keyword's
`primary_expression` reading at the same point, hence the two three-way conflicts. `(string)` and
`(string)[]` stay a `parenthesized_type`, and `(void) => void`, `(null) => void` and `(true) => void` stay parse
errors, as in TypeScript: those words cannot name a parameter.

Node kinds and field names are unchanged. The parameter has the shape any other named parameter has:
`formal_parameters > required_parameter` with an `identifier` as its `pattern` (`undefined` is an `undefined` node
there, as in the arrow function `(undefined) => 1`). `unknown` and `never` may now also name a parameter or a
variable where the grammar reads both a keyword and a name; elsewhere they already parsed as names. The state counts
are unchanged.

## Patch 6: no automatic semicolon is withheld before a member named `in` or `instanceof`

Upstream issue: tree-sitter/tree-sitter-typescript#373 (open) for the identifiers that start with `in`; none open for
members named `in` or `instanceof`. Present at tag `v0.23.2` and at upstream HEAD, and in tree-sitter-javascript's
`src/scanner.c`. tree-sitter/tree-sitter-typescript#372 (closed unmerged) proposed the identifier half.

Why: the external scanner inserts no semicolon at a line break before `in` or `instanceof`, so that `a⏎instanceof B`
stays one binary expression. In an object type, an interface or a class body, a member on the next line may be named
`in` or `instanceof`, and without the semicolon the member before it swallows the keyword:
`a: typeof b⏎  instanceof: typeof c` and `a: number⏎  in: string` are parse errors, and the declaration around
them is lost (botpress's `packages/zui/src/z/typings.ts` declares `instanceof: typeof createInstanceOf` this way,
line 2008). The scanner also takes an `in` or `instanceof` followed by `_`, a digit or `$` for the keyword, so
`a: string⏎  in_point: number` fails too.

```diff
--- a/common/scanner.h
+++ b/common/scanner.h
@@ -79,6 +79,17 @@ static bool scan_whitespace_and_comments(TSLexer *lexer, bool *scanned_comment)
     }
 }
 
+static bool is_identifier_char(int32_t c) { return iswalnum(c) || c == '_' || c == '$'; }
+
+// After `in` or `instanceof` at the start of a line where no binary operator can follow: whether a `:` or `?`
+// follows, so the keyword names a member (`in: T`, `instanceof?(): T`) that no expression continues with.
+static bool scan_member_name_end(TSLexer *lexer) {
+    while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
+        skip(lexer);
+    }
+    return lexer->lookahead == ':' || lexer->lookahead == '?';
+}
+
 static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols, bool *scanned_comment) {
     lexer->result_symbol = AUTOMATIC_SEMICOLON;
     lexer->mark_end(lexer);
@@ -163,7 +174,7 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
             return lexer->lookahead != '=';
 
             // Don't insert a semicolon before `in` or `instanceof`, but do insert one
-            // before an identifier.
+            // before an identifier, such as `in_point`, or before a member named `in` or `instanceof` after a type.
         case 'i':
             skip(lexer);
 
@@ -172,8 +183,8 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
             }
             skip(lexer);
 
-            if (!iswalpha(lexer->lookahead)) {
-                return false;
+            if (!is_identifier_char(lexer->lookahead)) {
+                return !valid_symbols[LOGICAL_OR] && scan_member_name_end(lexer);
             }
 
             for (unsigned i = 0; i < 8; i++) {
@@ -183,8 +194,8 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
                 skip(lexer);
             }
 
-            if (!iswalpha(lexer->lookahead)) {
-                return false;
+            if (!is_identifier_char(lexer->lookahead)) {
+                return !valid_symbols[LOGICAL_OR] && scan_member_name_end(lexer);
             }
             break;
     }
```

A semicolon goes in only where the keyword is followed, on its line, by `:` or `?`, which no `in` or `instanceof`
expression allows, and only where no binary operator could continue the line (the `||` token is not valid, as the
scanner already tests before `(` and `[`). So `a⏎instanceof B` and `a⏎instanceof (B)` stay binary expressions, and
the class field `a = b⏎  in: number` stays a parse error, as in TypeScript, which reads `b in` there. A method
signature named `instanceof` with no `?` (`a: number⏎  instanceof(): void`) stayed a parse error until Patch 12.

Node kinds and field names are unchanged, and so are both grammars' `grammar.json`.

With Patches 4 to 6, upstream's corpus parses as before (114 of 115 with three added cases, one per patch; the one
failure patch 1's `foo++!`). Over 22,898 TypeScript and TSX files of vscode@729f257 and eight other local corpora
(shadcn-ui, excalidraw, prettier, fastify, Next.js examples and commerce, and two private applications), 2 files
that had syntax errors parse (prettier's variance tests), none gains an error, and every other tree is unchanged but
for the error recovery in two prettier files that have syntax errors before and after.

## Patch 7: a bare `&` in JSX text is text

Upstream issue: tree-sitter/tree-sitter-javascript#366 (open; present at tag `v0.23.2`, at upstream HEAD, and in
tree-sitter-javascript's `src/scanner.c`). Mirrors: tree-sitter/tree-sitter-typescript#371 (open, `f8637eb`), with
its corpus case; tree-sitter/tree-sitter-javascript#388 (open) is the same change there.

Why: JSX text may hold `&` as text. TypeScript and Babel decode a complete character reference (`&amp;`, `&#38;`,
`&#x26;`) and read any other `&` literally, as React renders it. Upstream's `jsx_text` is an external token that stops
at every `&`, and the only other token there, `html_character_reference`, needs the closing `;`. So
`<span>Help & Support</span>`, `<h2>{a} & {b}</h2>`, `<p>a&b</p>` and `<p>&nbsp</p>` are parse errors, and the
element around them is lost (NER-1764; resume-matcher's settings page writes
`{t('settings.uiLanguage')} & {t('settings.contentLanguage')}`). These were the 82 syntax-error files Patch 3 left on
shadcn-ui@0e3abd6.

```diff
--- a/common/scanner.h
+++ b/common/scanner.h
@@ -284,6 +284,34 @@ static bool scan_closing_comment(TSLexer *lexer) {
     return true;
 }
 
+// After an `&`: whether the rest of a character reference follows, as html_character_reference reads one (`&amp;`,
+// `&#38;`, `&#x26;`). Consumes the name or number either way.
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
@@ -292,8 +320,22 @@ static bool scan_jsx_text(TSLexer *lexer) {
     // immediately follows a newline.
     bool at_newline = false;
 
+    lexer->result_symbol = JSX_TEXT;
+
     while (lexer->lookahead != 0 && lexer->lookahead != '<' && lexer->lookahead != '>' && lexer->lookahead != '{' &&
-           lexer->lookahead != '}' && lexer->lookahead != '&') {
+           lexer->lookahead != '}') {
+        if (lexer->lookahead == '&') {
+            // A character reference is its own token, so the text ends before it. Any other `&` is text, as
+            // TypeScript and Babel read it: `Help & Support`, `&nbsp` without its `;`.
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
@@ -321,7 +363,7 @@ static bool scan_jsx_text(TSLexer *lexer) {
         advance(lexer);
     }
 
-    lexer->result_symbol = JSX_TEXT;
+    lexer->mark_end(lexer);
     return saw_text;
 }
 
```

At an `&` the scanner marks the end of the text and reads on with the bounds of `html_character_reference`'s pattern
(1 to 30 letters, `#` and 1 to 5 digits, or `#x` and 1 to 6 hex digits, then `;`). A complete reference ends the
text before it and is lexed as before; any other `&` and what the scanner read after it stay in the text. The code is
#371's but for its comments. Whitespace and newlines count as before, so a text that is only layout whitespace before a
reference is still no `jsx_text`.

Node kinds and field names are unchanged, and so are both grammars' `grammar.json`. A bare `&` is part of the
`jsx_text` around it: `<span>Help & Support</span>` has one `jsx_text`, `Help & Support`, and `<p>a &amp; b</p>` keeps
`a `, `&amp;` and ` b`. The TypeScript dialect cannot reach the change: its only external lex state with `jsx_text`
valid is the error-recovery state, where every external token is valid and the scanner returns at the template-chars
test before it scans JSX text, as it does in TSX.

Upstream's corpus parses as before (115 of 116 with #371's case; the one failure patch 1's `foo++!`), and
tree-sitter-javascript 0.23.1's corpus builds the same trees under TSX as before. Over 22,173 TypeScript and TSX files
of the Next.js examples@6194d64, commerce@3761e52, shadcn-ui@0e3abd6, excalidraw@ed10ac7, vscode@729f257 and two
private applications, 103 TSX files that had syntax errors parse (82 in shadcn-ui, 16 in vscode, 3 in the Next.js
examples, 2 in the private applications), none gains an error, and every other tree is unchanged. No TSX file in them
has a syntax error left.

## Patch 8: a re-export takes import attributes

Upstream: none open at tree-sitter/tree-sitter-typescript; tree-sitter/tree-sitter-javascript#363 (closed unmerged)
proposed the same for JavaScript. Present at tag `v0.23.2` and at upstream HEAD `75b3874`. The same change as Patch 3
in `grammars/tree-sitter-javascript/PATCHES.md`.

Why: ECMAScript 2025 and TypeScript allow import attributes on `export … from`, as on `import`:
`export { default as x } from './x.json' with { type: 'json' }`, `export * from 'y' with { type: 'json' }`. Upstream
takes them only on `import`, so a re-export with them is a parse error whose remainder reads as a label `export:`, and
`no-labels` reports a false positive at the `with {` (NER-1764, NER-1777).

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -358,6 +358,17 @@ module.exports = function defineGrammar(dialect) {
 
       export_statement: ($, previous) => choice(
         previous,
+        // A re-export takes import attributes, as an import does: `export * from './x.json' with { type: 'json' }`.
+        seq(
+          'export',
+          choice(
+            seq('*', $._from_clause),
+            seq($.namespace_export, $._from_clause),
+            seq(optional('type'), $.export_clause, $._from_clause),
+          ),
+          $.import_attribute,
+          $._semicolon,
+        ),
         seq(
           'export',
           'type',
```

The JavaScript rule this grammar extends stays as it is; the added alternative is a re-export followed by its
attributes. As in TypeScript's parser, `export type { A } from 'x'` takes them too, and so does the deprecated
`assert`, which this grammar's `import_attribute` already reads.

Node kinds and field names are unchanged. The attribute is an `import_attribute` child of the `export_statement`, as it
is of an `import_statement`. Each generated parser has 24 more states (5,368 for TypeScript, 5,617 for TSX).

## Patch 9: a `{` at the start of a statement opens a block

Upstream issue: tree-sitter/tree-sitter-javascript#259 (open), whose `labeled_statement` this grammar inherits.
Present at tag `v0.23.2` and at upstream HEAD. The same change as Patch 4 in
`grammars/tree-sitter-javascript/PATCHES.md`.

Why: ECMAScript forbids an expression statement and an arrow function's expression body to start with `{`, so there
`{` always opens a block: `if (foo) { bar: 'baz' }`, `() => { bar: 1 }` and `{ bar: 1 }` hold a labeled statement, in
TypeScript as in JavaScript. Upstream reads each as an object literal, with no error, because GLR keeps both readings
and the labeled statement's dynamic precedence of -1 makes the object win. The tree is silently wrong: `curly`,
`no-labels`, `no-unused-labels` and `arrow-body-style` see an object where ESLint sees a label.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -172,6 +172,15 @@ module.exports = function defineGrammar(dialect) {
         field('body', $.statement_block),
       ),
 
+      // A `{` that starts a statement or an arrow function's body opens a block, never an object (ECMAScript's
+      // ExpressionStatement and ConciseBody lookahead restrictions): `if (a) { b: 1 }` and `() => { b: 1 }` hold a
+      // labeled statement. Where only an object parses, as in `{ a: 1, b: 2 }`, GLR keeps the object.
+      labeled_statement: $ => prec.dynamic(1, seq(
+        field('label', alias(choice($.identifier, $._reserved_identifier), $.statement_identifier)),
+        ':',
+        field('body', $.statement),
+      )),
+
       call_expression: $ => choice(
         prec('call', seq(
           field('function', choice($.expression, $.import)),
```

The rule is tree-sitter-javascript 0.23.1's but for the dynamic precedence, which now outweighs the object. GLR still
keeps the object where the block does not parse, as in `{ a: 1, b: 2 }`, and a `{` in an expression or a type never had
the block reading: `x = { a: 1 }`, `() => ({ a: 1 })`, `{ a: 1 } satisfies T` after `=`, and an arrow function's
return type `(): { a: string } => x` are as before.

Node kinds and field names are unchanged. `{ a: 1 }` at the start of a statement or as an arrow function's body is a
`statement_block` holding a `labeled_statement`, no longer an `object` holding a `pair`. tree-sitter-javascript
0.23.1's corpus case `Objects`, which asserts the object for `{ a: "b" };` at statement level, now reads the block,
as this patch intends. The state counts are unchanged.

## Patch 10: `yield` may label a statement

Upstream issue: tree-sitter/tree-sitter-javascript#303 (open: `await` and `yield` are not read by context). Present at
tag `v0.23.2` and at upstream HEAD. The same change as Patch 7 in `grammars/tree-sitter-javascript/PATCHES.md`.

Why: outside generators and strict code `yield` is a name, so `yield: while (foo) { … break yield }` is a labeled
statement. Upstream reads `yield` at the start of a statement as the keyword, so the label is a parse error.
`var yield = 5` and `break yield` already parsed, since the keyword is not valid there.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -176,7 +176,8 @@ module.exports = function defineGrammar(dialect) {
       // ExpressionStatement and ConciseBody lookahead restrictions): `if (a) { b: 1 }` and `() => { b: 1 }` hold a
       // labeled statement. Where only an object parses, as in `{ a: 1, b: 2 }`, GLR keeps the object.
       labeled_statement: $ => prec.dynamic(1, seq(
-        field('label', alias(choice($.identifier, $._reserved_identifier), $.statement_identifier)),
+        // Outside generators and strict code `yield` is a name, so it may label a statement: `yield: while (a) {}`.
+        field('label', alias(choice($.identifier, $._reserved_identifier, 'yield'), $.statement_identifier)),
         ':',
         field('body', $.statement),
       )),
```

Node kinds and field names are unchanged: the label is a `statement_identifier`, as any other is. The generated
parsers have 2 (TypeScript) and 3 (TSX) more states. `yield` as a name inside an expression (`yield + 1`, `f(yield)`)
still reads as a `yield_expression`: telling the name from the keyword needs to know whether the enclosing function is
a generator, which the grammar does not track (#303).

## Patch 11: a destructuring assignment's target may be parenthesized

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD. The same change as Patch 6 in
`grammars/tree-sitter-javascript/PATCHES.md`.

Why: in a destructuring assignment, ECMAScript and TypeScript allow a parenthesized target, as on the left of a plain
assignment: `({foo: (foo)} = obj)`, `({foo: (/**/foo)} = {})`, `[(a), (b.c) = 1, ...(d)] = e`, and in TypeScript
`({a: (b as any)} = c)`. Upstream's patterns take no parentheses, so each is a parse error.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -224,6 +224,19 @@ module.exports = function defineGrammar(dialect) {
 
       _lhs_expression: ($, previous) => choice(previous, $.non_null_expression),
 
+      pattern: $ => prec.dynamic(-1, choice(
+        $._lhs_expression,
+        $.rest_pattern,
+        // A destructuring assignment's target may be parenthesized, as an assignment's is: `({a: (b)} = c)`,
+        // `[(a), (b.c) = 1, ...(d)] = e`.
+        $.parenthesized_expression,
+      )),
+
+      rest_pattern: $ => prec.right(seq(
+        '...',
+        choice($._lhs_expression, $.parenthesized_expression),
+      )),
+
       primary_expression: ($, previous) => choice(
         previous,
         $.non_null_expression,
```

Both rules are tree-sitter-javascript 0.23.1's but for the added `parenthesized_expression`.

Node kinds and field names are unchanged. A pattern (a `pair_pattern`'s value, an `array_pattern`'s element, an
`assignment_pattern`'s left side, a `rest_pattern`'s target) may be a `parenthesized_expression`, as an
`assignment_expression`'s left side already may. The grammar has one `pattern` for binding and assignment patterns,
so parenthesized binding patterns, which TypeScript rejects (`let {a: (b)} = c`, `function f((a)) {}`), now parse too;
the parameter is then a `required_parameter` whose pattern is a `parenthesized_expression`. The state counts are
unchanged.

## Patch 12: a semicolon is inserted before a token that cannot continue the statement

Upstream issue: none open for these. Present at tag `v0.23.2` and at upstream HEAD `75b3874`. Patch 8 in
`grammars/tree-sitter-javascript/PATCHES.md` fixes the same gaps in JavaScript's scanner with five never-returned
external tokens; this scanner already tells a type or a name from an expression by whether a binary operator (the
`||` token) can come next, as it does before `(` and `[`, so the patch extends that test instead.

Why: ECMAScript and TypeScript insert a semicolon at a line break before a token the grammar cannot accept there. The
scanner never inserted one before a line starting with `*` or a binary `+` or `-`, nor before `instanceof` unless a
`:` or `?` followed it (Patch 6). That is right after an expression, which the line continues (`a⏎*b`), and wrong where
no expression precedes: `var a⏎+b > 0 ? c : d` after a declarator, `let a: T⏎-1` after a type, and a class field
before a generator method or a member named `instanceof` (`j⏎*k() {}`, `a: string⏎*gen() {}`, `#l⏎instanceof() {}`,
and in an interface `a: number⏎instanceof(): void`). Each is a parse error.

```diff
--- a/common/scanner.h
+++ b/common/scanner.h
@@ -131,7 +131,6 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
         case ',':
         case '.':
         case ';':
-        case '*':
         case '%':
         case '>':
         case '<':
@@ -150,23 +149,25 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
             }
             break;
 
-            // Don't insert a semicolon before a '[' or '(', unless we're parsing
-            // a type. Detect whether we're parsing a type or an expression using
-            // the validity of a binary operator token.
+            // Don't insert a semicolon before a '[', '(' or '*', unless we're parsing
+            // a type, or no binary operator can come next, as after a declarator's or a class field's name
+            // (`var a⏎[b] = c`, `a⏎*g() {}`). Detect it using the validity of a binary operator token.
         case '(':
         case '[':
+        case '*':
             if (valid_symbols[LOGICAL_OR]) {
                 return false;
             }
             break;
 
-            // Insert a semicolon before `--` and `++`, but not before binary `+` or `-`.
+            // Insert a semicolon before `--` and `++`, and before a `+` or `-` where no binary operator can come
+            // next (`var a⏎+b`), but not before binary `+` or `-`.
         case '+':
             skip(lexer);
-            return lexer->lookahead == '+';
+            return lexer->lookahead == '+' || !valid_symbols[LOGICAL_OR];
         case '-':
             skip(lexer);
-            return lexer->lookahead == '-';
+            return lexer->lookahead == '-' || !valid_symbols[LOGICAL_OR];
 
             // Don't insert a semicolon before `!=`, but do insert one before a unary `!`.
         case '!':
@@ -174,7 +175,8 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
             return lexer->lookahead != '=';
 
             // Don't insert a semicolon before `in` or `instanceof`, but do insert one
-            // before an identifier, such as `in_point`, or before a member named `in` or `instanceof` after a type.
+            // before an identifier, such as `in_point`, before a member named `in` after a type, or before an
+            // `instanceof` that no binary operator can take here: a member's name (`#a⏎instanceof() {}`).
         case 'i':
             skip(lexer);
 
@@ -195,7 +197,7 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
             }
 
             if (!is_identifier_char(lexer->lookahead)) {
-                return !valid_symbols[LOGICAL_OR] && scan_member_name_end(lexer);
+                return !valid_symbols[LOGICAL_OR];
             }
             break;
     }
```

Where a binary operator can come next, the line continues the statement as before: `a⏎*b`, `a⏎+b`, `a⏎instanceof B`
and `a⏎instanceof (B)` stay binary expressions. `in` keeps Patch 6's test: after `for (const k`, `in` continues the
head and no binary operator can come next there either. A `return` followed by a line break and a `+` or `-` operand
now ends the statement, as ECMAScript's restricted production does and as this scanner already did before `(` and
`[` (`return⏎-1` is `return;` and `-1;`); after `yield`, a `*` still continues the statement.

Node kinds and field names are unchanged, and so are both grammars' `grammar.json`. The statements around such a line
break split as ECMAScript and TypeScript split them.

## Patch 13: an arrow function's block body ends the statement at a line break

Upstream issue: tree-sitter/tree-sitter-javascript#354 (open), for a line starting with `(`. Present at tag `v0.23.2`
and at upstream HEAD. The same change as Patch 9 in `grammars/tree-sitter-javascript/PATCHES.md`.

Why: an arrow function is no operand in ECMAScript or TypeScript: it cannot be called, indexed, tagged or the left
side of a binary expression, so after its block body a line break ends the statement unless the next line goes on
with a list's `,`, a ternary's `:` or a `;`. Upstream's `arrow_function` is a `primary_expression`, so the next line
continues it: `const f = () => {}⏎[1, 2].forEach(g)` indexes the function with a sequence expression, with no error
(`no-sequences` reports a false positive), `const f = (): void => {}⏎(g)()` calls it, and `y = () => {}⏎() => {}`
and a class field `a = () => {}⏎*gen() {}` are parse errors.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -7,6 +7,8 @@ module.exports = function defineGrammar(dialect) {
     externals: ($, previous) => previous.concat([
       $._function_signature_automatic_semicolon,
       $.__error_recovery,
+      // Never returned: valid only after an arrow function's block body (see arrow_function).
+      $._arrow_body_end,
     ]),
 
     supertypes: ($, previous) => previous.concat([
@@ -182,6 +184,25 @@ module.exports = function defineGrammar(dialect) {
         field('body', $.statement),
       )),
 
+      arrow_function: $ => seq(
+        optional('async'),
+        choice(
+          field('parameter', choice(
+            alias($._reserved_identifier, $.identifier),
+            $.identifier,
+          )),
+          $._call_signature,
+        ),
+        '=>',
+        choice(
+          field('body', $.expression),
+          // An arrow function is no operand, so a line break after its block body ends the statement where the next
+          // line cannot continue an assignment: `f = () => {}⏎[1, 2].map(f)` is two statements, as in ECMAScript.
+          // `_arrow_body_end` marks this place for the scanner.
+          seq(field('body', $.statement_block), optional($._arrow_body_end)),
+        ),
+      ),
+
       call_expression: $ => choice(
         prec('call', seq(
           field('function', choice($.expression, $.import)),
--- a/common/scanner.h
+++ b/common/scanner.h
@@ -13,6 +13,8 @@ enum TokenType {
     JSX_TEXT,
     FUNCTION_SIGNATURE_AUTOMATIC_SEMICOLON,
     ERROR_RECOVERY,
+    // Never returned: valid only after an arrow function's block body.
+    ARROW_BODY_END,
 };
 
 static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }
@@ -126,6 +128,13 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
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

The rule is tree-sitter-javascript 0.23.1's but for the marker `_arrow_body_end`, which follows an arrow function's
block body and is never returned, so the scanner is told when it scans after one. The marker is the last external
token, after this grammar's own two, so the scanner's numbering of the others is unchanged. A function expression's
body is not marked: `e = function () {}⏎[1]` stays a subscript, as in ECMAScript.

Node kinds and field names are unchanged: the `body` field holds the same `statement_block`. The statements around it
split as ECMAScript splits them. Each generated parser has 37 more states (5,407 for TypeScript, 5,657 for TSX).

## Patch 14: `yield` is a name where a yield expression cannot parse

Upstream issue: tree-sitter/tree-sitter-javascript#303 (open: `await` and `yield` are not read by context). Present at
tag `v0.23.2` and at upstream HEAD. The same change as Patch 10 in `grammars/tree-sitter-javascript/PATCHES.md`.

Why: outside generators and strict code `yield` is a name, so `yield = 2`, `yield += 1`, `yield++`, `[yield] = a`,
`for (yield of xs);` and `yield.x` are valid scripts, and TypeScript parses them. Upstream reads `yield` in an
expression only as the keyword. So the assignments, the update and the `for` head are parse errors. `yield.x` silently
reads as a member of an empty `yield_expression`, which ECMAScript never allows.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -115,6 +115,8 @@ module.exports = function defineGrammar(dialect) {
       [$.array_pattern, $.tuple_type],
 
       [$.template_literal_type, $.template_string],
+
+      [$.yield_expression, $._yield_identifier],
     ]).concat(
       dialect === 'typescript' ? [
         [$.primary_type, $.type_parameter],
@@ -241,9 +243,17 @@ module.exports = function defineGrammar(dialect) {
         field('right', $.expression),
       )),
 
-      _augmented_assignment_lhs: ($, previous) => choice(previous, $.non_null_expression),
+      _augmented_assignment_lhs: ($, previous) => choice(
+        previous,
+        $.non_null_expression,
+        alias($._yield_identifier, $.identifier),
+      ),
 
-      _lhs_expression: ($, previous) => choice(previous, $.non_null_expression),
+      _lhs_expression: ($, previous) => choice(
+        previous,
+        $.non_null_expression,
+        alias($._yield_identifier, $.identifier),
+      ),
 
       pattern: $ => prec.dynamic(-1, choice(
         $._lhs_expression,
@@ -258,6 +268,39 @@ module.exports = function defineGrammar(dialect) {
         choice($._lhs_expression, $.parenthesized_expression),
       )),
 
+      member_expression: $ => prec('member', seq(
+        field('object', choice($.expression, $.primary_expression, $.import, $._yield_operand)),
+        choice('.', field('optional_chain', $.optional_chain)),
+        field('property', choice(
+          $.private_property_identifier,
+          alias($.identifier, $.property_identifier))),
+      )),
+
+      update_expression: $ => prec.left(choice(
+        seq(
+          field('argument', choice($.expression, $._yield_operand)),
+          field('operator', choice('++', '--')),
+        ),
+        seq(
+          field('operator', choice('++', '--')),
+          field('argument', $.expression),
+        ),
+      )),
+
+      // Outside generators and strict code `yield` is a name. The grammar does not know whether a function is a
+      // generator, so `yield` is a name where a yield_expression cannot parse (`yield = 2`, `[yield] = a`,
+      // `for (yield of a)`), and where one would be an operand, which ECMAScript forbids (`yield.x`, `yield++`).
+      // Where both read (`f(yield)`, `yield + 1`, `yield [0]`, `yield (a)`), the yield_expression stays.
+      _yield_identifier: _ => prec.dynamic(-1, 'yield'),
+
+      _yield_operand: $ => prec.dynamic(2, alias($._yield_identifier, $.identifier)),
+
+      yield_expression: $ => prec.right(choice(
+        // A `yield` with an operand outweighs a `yield` name before `++` or `--`: `yield++⏎x` is `yield ++x`.
+        prec.dynamic(3, seq('yield', choice(seq('*', $.expression), $.expression))),
+        'yield',
+      )),
+
       primary_expression: ($, previous) => choice(
         previous,
         $.non_null_expression,
```

`member_expression` and `update_expression` are tree-sitter-javascript 0.23.1's but for the added `_yield_operand`;
the two assignment targets extend this grammar's. As in JavaScript, `yield` is an `identifier` only where the keyword's
reading fails: an assignment target (also in patterns and `for … of` heads), an augmented assignment target, and the
operand of a member access or postfix update, where the identifier's dynamic precedence of 2 outweighs the
`yield_expression` ECMAScript forbids there. Elsewhere its dynamic precedence of -1 keeps the generator reading
wherever both parse: `f(yield)`, `yield + 1`, `yield;`, `yield [0]` and `yield (a)` stay `yield_expression`s, as in a
generator. A `yield` with an operand outweighs both, with a dynamic precedence of 3, so in a generator
`yield++⏎x` and `yield--⏎x` yield `++x` and `--x`, as ECMAScript reads them there, while `yield++;`, where no operand
follows, is the name's update. The empty `yield` is its own alternative of `yield_expression`, so its node is as
before. `yield()` stays a parse error. In strict code, such as a module, TypeScript rejects `yield` as a name
(TS1214); the grammar does not tell scripts from modules, so it reads the name there too.

Node kinds and field names are unchanged. A `yield` that is a name is an `identifier`, as `var yield` already gave.
Each generated parser has 27 more states (5,434 for TypeScript, 5,684 for TSX).

With Patches 8 to 14, upstream's corpus parses as before (122 of 123 with seven added cases, one per patch; the one
failure patch 1's `foo++!`). Under either dialect, tree-sitter-javascript 0.23.1's corpus builds the same trees as
before but for its `Objects` case (Patch 9). Over the 22,173 TypeScript and TSX files above, no tree changes: none of
them holds these constructs. tree-sitter-javascript's Patch 5 (`using` declarations in a `for` head) has no
counterpart here: this grammar has no `using_declaration`, and reads `using r = f()` as an `assignment_expression`
with a leading `using`, so `for (await using r = f(); ;) {}` already parsed without an error.

## Patch 15: an import type is a primary type

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD `75b3874`.

Why: TypeScript's import type (`import('x').T`) is a primary type, so an array type, an indexed access or type
arguments may follow it: `import('x').T[]`, `readonly import('x').T[]`, `import('x').T['a']['b'][number]`,
`import('x').I<T>`. Upstream reads it only as a whole `type`, never as a `primary_type`, so each of these is a parse
error. vscode@729f257 writes them in 11 files, among them `claudeSubagentSignals.ts`, `codexChatDiscovery.test.ts`,
`hookSchema.test.ts`, `issueFormService.ts`, `terminalProfileService.integrationTest.ts`, `workbench.ts`,
`sessions.ts` and `build/next/index.ts`; eslint-plugin-unicorn's fixture `prefer-node-protocol/064` is
`type fs = import("fs").fs<"fs">`.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -863,8 +863,6 @@ module.exports = function defineGrammar(dialect) {
         $.readonly_type,
         $.constructor_type,
         $.infer_type,
-        prec(-1, alias($._type_query_member_expression_in_type_annotation, $.member_expression)),
-        prec(-1, alias($._type_query_call_expression_in_type_annotation, $.call_expression)),
       ),
 
       tuple_parameter: $ => seq(
@@ -900,6 +898,10 @@ module.exports = function defineGrammar(dialect) {
 
       primary_type: $ => choice(
         $.parenthesized_type,
+        // An import type is a primary type, as in TypeScript, so an array type, an indexed access or type arguments
+        // may follow it: `import('x').T[]`, `import('x').T['a'][number]`, `import('x').I<T>`.
+        prec(-1, alias($._type_query_member_expression_in_type_annotation, $.member_expression)),
+        prec(-1, alias($._type_query_call_expression_in_type_annotation, $.call_expression)),
         $.predefined_type,
         $._type_identifier,
         $.nested_type_identifier,
@@ -955,6 +957,8 @@ module.exports = function defineGrammar(dialect) {
         field('name', choice(
           $._type_identifier,
           $.nested_type_identifier,
+          alias($._type_query_member_expression_in_type_annotation, $.member_expression),
+          alias($._type_query_call_expression_in_type_annotation, $.call_expression),
         )),
         field('type_arguments', $.type_arguments),
       )),
```

The two import-type alternatives move from `type` to `primary_type` with the same precedence, and a `generic_type`'s
name may be an import type, qualified or not (`import('x').I<T>`, `import('x')<T>`). `typeof import('x')`,
`keyof import('x').T`, unions of import types and `import('x')` alone read as before. Type arguments after a
`typeof import('x')` (`typeof import('x')<T>`, which TypeScript also reads) stay a parse error, as before: they belong
to the type query, which this patch leaves alone.

Node kinds and field names are unchanged. An import type is the `member_expression` or `call_expression` it was; it
may now be the element of an `array_type`, the object of a `lookup_type` or the `name` of a `generic_type`. Each
generated parser has 15 more states (14 for TSX).

## Patch 16: a tuple member may be named by a type keyword

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD.

Why: TypeScript names a tuple member by an identifier or any keyword but a reserved word such as `class` or `if`:
`[spoken: string, symbol: string][]`, `[string: number]`, `[any?: boolean]`, `[void: number]`, `[null?: string]`.
Upstream's tuple member name is an `identifier`, and in a type the lexer reads `symbol`, `string`, `any`, `void`,
`null`, `typeof` and the other type keywords as keywords, so each is a parse error (vscode@729f257's
`terminalVoice.ts`: `const symbolMap: [spoken: string, symbol: string][] = []`).

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -102,6 +102,10 @@ module.exports = function defineGrammar(dialect) {
       [$.primary_expression, $.pattern, $.predefined_type],
 
       [$.optional_tuple_parameter, $.primary_type],
+      // `[any?: boolean]` and `[any?]`, `[null?: T]` and `[null?]` read alike up to the `?`: GLR keeps both, and the
+      // `:` decides.
+      [$.optional_tuple_parameter, $.predefined_type],
+      [$.optional_tuple_parameter, $.literal_type],
       [$.rest_pattern, $.primary_type, $.primary_expression],
 
       [$.object, $.object_type],
@@ -134,6 +138,7 @@ module.exports = function defineGrammar(dialect) {
       .concat([
         $._type_identifier,
         $._jsx_start_opening_element,
+        $._tuple_member_name,
       ]),
 
     rules: {
@@ -865,17 +870,28 @@ module.exports = function defineGrammar(dialect) {
         $.infer_type,
       ),
 
+      // A tuple member may be named by a type keyword, as in TypeScript: `[symbol: string]`, `[any?: boolean]`,
+      // `[void: number]`.
       tuple_parameter: $ => seq(
-        field('name', choice($.identifier, $.rest_pattern)),
+        field('name', choice($._tuple_member_name, $.rest_pattern)),
         field('type', $.type_annotation),
       ),
 
       optional_tuple_parameter: $ => seq(
-        field('name', $.identifier),
+        field('name', $._tuple_member_name),
         '?',
         field('type', $.type_annotation),
       ),
 
+      _tuple_member_name: $ => choice(
+        $.identifier,
+        alias(choice(
+          $._reserved_identifier,
+          $.this, $.null, $.undefined, $.true, $.false,
+          'void', 'typeof', 'keyof', 'unique', 'infer', 'import',
+        ), $.identifier),
+      ),
+
       optional_type: $ => seq($.type, '?'),
       rest_type: $ => seq('...', $.type),
 
```

The name accepts the words the grammar already lets name a variable (`_reserved_identifier`), as a parameter's name
does, and the type keywords a type may start with: `void`, `null`, `undefined`, `true`, `false`, `this`, `typeof`,
`keyof`, `unique`, `infer` and `import`. GLR keeps both readings up to the `?` or `:`: `[any?]`, `[null?]` and
`[this?]` stay `optional_type`s and `[string, number]` a tuple of types, `[typeof x]`, `[keyof T]`, `[unique symbol]`,
`[infer U]` and `[import('x').T]` read as before. The literal keywords are named through their rules (`$.null`, …),
so a `null` type keeps its leaf `null` node. `[class: string]` and `[if: T]`, which TypeScript rejects, read as named
members, as they did: the lexer reads a reserved word where no keyword is valid as an identifier.

Node kinds and field names are unchanged: the name is an `identifier`, as any other tuple member's is. Each generated
parser has 23 more states.

## Patch 17: a default export may be an anonymous function signature

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD.

Why: TypeScript allows overloads and ambient declarations of an anonymous default export:
`export default function (x: string): string;`, also in `declare module 'm' { … }`. Upstream's `function_signature`
needs a name, so each is a parse error (next-examples' `with-electron-typescript/electron-src/electron-next.d.ts`;
typescript-eslint fixtures `no-invalid-void-type/031` and `unified-signatures/023`, `024`, `063`, `064`).

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -470,6 +470,20 @@ module.exports = function defineGrammar(dialect) {
         ),
         seq('export', '=', $.expression, $._semicolon),
         seq('export', 'as', 'namespace', $.identifier, $._semicolon),
+        // A default export's overloads and ambient declaration may be anonymous signatures, as in TypeScript:
+        // `export default function (x: string): string;`.
+        seq(
+          'export',
+          'default',
+          field('declaration', alias($._anonymous_function_signature, $.function_signature)),
+        ),
+      ),
+
+      _anonymous_function_signature: $ => seq(
+        optional('async'),
+        'function',
+        $._call_signature,
+        choice($._semicolon, $._function_signature_automatic_semicolon),
       ),
 
       non_null_expression: $ => prec('member', seq(
```

Node kinds and field names are unchanged. The signature is a `function_signature` without a `name`, the
`export_statement`'s `declaration`, as a named one is. `export default function (x) {}` stays a
`function_expression`. Each generated parser has 27 more states.

## Patch 18: `accessor` follows `static`, `abstract` and `override`

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD.

Why: TypeScript allows `accessor` after `static`, `abstract` and `override` (`static accessor foo = …`,
`private abstract accessor foo: number`, `override accessor m = …`). Upstream takes `accessor` only alone, so each
of these is a parse error (ESLint fixtures `class-methods-use-this/066`, `067`, `081`; typescript-eslint fixtures
`explicit-member-accessibility/005`, `no-extraneous-class/011`, `020`, `021`). And upstream reads `accessor` as a
keyword at the start of a class member, so a member named `accessor` (`accessor = 1`, `accessor() {}`) is one too.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -149,10 +149,11 @@ module.exports = function defineGrammar(dialect) {
           seq($.accessibility_modifier, optional('declare')),
         )),
         choice(
-          seq(optional('static'), optional($.override_modifier), optional('readonly')),
-          seq(optional('abstract'), optional('readonly')),
+          // `accessor` follows `static`, `abstract` and `override`, as in TypeScript: `static accessor a = 1`,
+          // `private abstract accessor b: number`, `override accessor c = 2`.
+          seq(optional('static'), optional($.override_modifier), optional(choice('readonly', 'accessor'))),
+          seq(optional('abstract'), optional(choice('readonly', seq(optional($.override_modifier), 'accessor')))),
           seq(optional('readonly'), optional('abstract')),
-          optional('accessor'),
         ),
         field('name', $._property_name),
         optional(choice('?', '!')),
@@ -1268,6 +1269,8 @@ module.exports = function defineGrammar(dialect) {
         'unknown',
         'never',
         'new',
+        // A modifier only before a class member's name, as `static` is: `accessor = 1`, `{ accessor }`.
+        'accessor',
         'readonly',
         previous,
       ),
```

The modifiers follow TypeScript's order (`accessor` last, `readonly` never with it). `accessor` joins the words that
may name a variable or a member (`_reserved_identifier`), as `static` has: with the modifier valid after `static` and
`override`, the lexer would otherwise read `accessor` as the keyword in more places, and a variable named `accessor`
(`{ accessor, r }`, `accessor.dispose()`, in 97 vscode files) would stop parsing.

Node kinds and field names are unchanged: `accessor` stays an anonymous token of the `public_field_definition`, and a
member or a variable named `accessor` is a `property_identifier` or an `identifier`. Each generated parser has 337 more
states (5,836 for TypeScript, 6,085 for TSX): the class member's modifier prefixes can no longer share states.

`static` on its own line before a member's name (`static⏎foo = 1`, `static⏎accessor⏎y = 2`) still ends a field named
`static`, as upstream reads every modifier before a line break; TypeScript, whose `static` may be followed by a line
break, reads one static member. This automatic semicolon is upstream's, not this patch's.

## Patch 19: a type-only re-export of a whole module

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD.

Why: TypeScript 5.0 allows `export type * from 'x'` and `export type * as ns from 'x'`. Upstream takes `type` only
before an export clause, so both are parse errors (ESLint fixtures `no-duplicate-imports/044`, `060`, `071`, `072`,
`073`).

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -469,6 +469,18 @@ module.exports = function defineGrammar(dialect) {
           optional($._from_clause),
           $._semicolon,
         ),
+        // A type-only re-export of a whole module, as in TypeScript 5.0: `export type * from 'x'`,
+        // `export type * as ns from 'x'`.
+        seq(
+          'export',
+          'type',
+          choice(
+            seq('*', $._from_clause),
+            seq($.namespace_export, $._from_clause),
+          ),
+          optional($.import_attribute),
+          $._semicolon,
+        ),
         seq('export', '=', $.expression, $._semicolon),
         seq('export', 'as', 'namespace', $.identifier, $._semicolon),
         // A default export's overloads and ambient declaration may be anonymous signatures, as in TypeScript:
```

Node kinds and field names are unchanged: the statement is an `export_statement` with the `type` token, as
`export type { A } from 'x'` is, and a `namespace_export` for `* as ns`. Each generated parser has 28 more states.

## Patch 20: the scanner reads every ECMAScript whitespace and line terminator

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD. The same change as Patch 11 in
`grammars/tree-sitter-javascript/PATCHES.md`, with the same line terminators.

Why: the grammar's extras skip every ECMAScript WhiteSpace and LineTerminator
(`/[\s\p{Zs}\uFEFF\u2028\u2029\u2060\u200B]/`). The external scanner uses C's `iswspace`, which in the C locale the
engine runs in is true only for ASCII whitespace, so where the scanner decides on an automatic semicolon, a ternary
`?` or an HTML comment, a U+3000, U+00A0, U+FEFF or other Unicode space ends its look: `` `template`　 `` (an
ideographic space after the backtick), `a = 1　⏎b = 2` and `x = a　? b : c` are parse errors. And it reads only `\n`
as a line break, so a lone `\r`, a U+2028 or a U+2029 inserts no semicolon (`a = 1\rb = 2`, `a\u2028b`) and does not
end a `//` comment before it (`c = 1 // d\u2028e = 2`). JSX text, in turn, tells layout whitespace from text with
`iswspace`, so in a UTF-8 locale a line holding only a U+00A0 (`<a>⏎\u00a0</a>`) is no `jsx_text`, and in the C
locale it is.

```diff
--- a/common/scanner.h
+++ b/common/scanner.h
@@ -21,6 +21,42 @@ static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }
 
 static void skip(TSLexer *lexer) { lexer->advance(lexer, true); }
 
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
+// An ECMAScript LineTerminator: `\n`, `\r`, U+2028 or U+2029 (CRLF reads as one, the `\n` skipped as whitespace).
+static bool is_line_terminator(int32_t c) { return c == '\n' || c == '\r' || c == 0x2028 || c == 0x2029; }
+
+// The whitespace JSX text may be laid out with, which is no text before or after a line break: ASCII whitespace only,
+// so a U+00A0 or U+3000 stays text whatever the C library's locale says.
+static bool is_jsx_layout_space(int32_t c) {
+    return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\v' || c == '\f';
+}
+
 static bool scan_template_chars(TSLexer *lexer) {
     lexer->result_symbol = TEMPLATE_CHARS;
     for (bool has_content = false;; has_content = true) {
@@ -46,7 +82,7 @@ static bool scan_template_chars(TSLexer *lexer) {
 
 static bool scan_whitespace_and_comments(TSLexer *lexer, bool *scanned_comment) {
     for (;;) {
-        while (iswspace(lexer->lookahead)) {
+        while (is_space(lexer->lookahead)) {
             skip(lexer);
         }
 
@@ -55,7 +91,7 @@ static bool scan_whitespace_and_comments(TSLexer *lexer, bool *scanned_comment)
 
             if (lexer->lookahead == '/') {
                 skip(lexer);
-                while (lexer->lookahead != 0 && lexer->lookahead != '\n') {
+                while (lexer->lookahead != 0 && !is_line_terminator(lexer->lookahead)) {
                     skip(lexer);
                 }
                 *scanned_comment = true;
@@ -107,16 +143,16 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
             // Therefore, disable automatic semicolons when followed by typing
             do {
                 skip(lexer);
-            } while (iswspace(lexer->lookahead));
+            } while (is_space(lexer->lookahead));
             if (lexer->lookahead == ':') {
                 return valid_symbols[LOGICAL_OR]; // Don't return false if we're in a ternary by checking if || is valid
             }
             return true;
         }
-        if (!iswspace(lexer->lookahead)) {
+        if (!is_space(lexer->lookahead)) {
             return false;
         }
-        if (lexer->lookahead == '\n') {
+        if (is_line_terminator(lexer->lookahead)) {
             break;
         }
         skip(lexer);
@@ -216,7 +252,7 @@ static bool scan_automatic_semicolon(TSLexer *lexer, const bool *valid_symbols,
 
 static bool scan_ternary_qmark(TSLexer *lexer) {
     for (;;) {
-        if (!iswspace(lexer->lookahead)) {
+        if (!is_space(lexer->lookahead)) {
             break;
         }
         skip(lexer);
@@ -236,7 +272,7 @@ static bool scan_ternary_qmark(TSLexer *lexer) {
         /* TypeScript optional arguments contain the ?: sequence, possibly
            with whitespace. */
         for (;;) {
-            if (!iswspace(lexer->lookahead)) {
+            if (!is_space(lexer->lookahead)) {
                 break;
             }
             advance(lexer);
@@ -259,7 +295,7 @@ static bool scan_ternary_qmark(TSLexer *lexer) {
 }
 
 static bool scan_closing_comment(TSLexer *lexer) {
-    while (iswspace(lexer->lookahead) || lexer->lookahead == 0x2028 || lexer->lookahead == 0x2029) {
+    while (is_space(lexer->lookahead)) {
         skip(lexer);
     }
 
@@ -347,7 +383,7 @@ static bool scan_jsx_text(TSLexer *lexer) {
             at_newline = false;
             continue;
         }
-        bool is_wspace = iswspace(lexer->lookahead);
+        bool is_wspace = is_jsx_layout_space(lexer->lookahead);
         if (lexer->lookahead == '\n') {
             at_newline = true;
         } else {
```

`is_space` is tree-sitter-javascript's Patch 11 function: the extras' set, spelled out. `is_line_terminator` is
ECMAScript's LineTerminator (`\n`, `\r`, U+2028, U+2029); a CRLF still reads as one line break, the `\n` skipped as
whitespace after the `\r`. JSX text tests layout whitespace with `is_jsx_layout_space`, the ASCII whitespace C's
`iswspace` reads in the C locale, so its trees are those the engine built before, in every locale, and a U+00A0 stays
text. A block comment that spans a line break (`a /*⏎*/ b`) still inserts no semicolon, as in tree-sitter-javascript.

Node kinds, field names and the grammar are unchanged.

## Patch 21: a comment between a JSX closing tag's `<` and `/`

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD. The same change as Patch 13 in
`grammars/tree-sitter-javascript/PATCHES.md`.

Why: Babel (also with its `typescript` plugin) allows comments between a JSX closing tag's tokens, also between its `<`
and `/`: `<></* c *//>`. The lexer reads `</` as one token, so there it reads `</`, then `* c *//>`, and fails. The
TypeScript compiler (5.9) rejects it (TS1003, its scanner reads `</` as one token too), so in TSX this reads more than
`tsc` does, as Babel-compiled TSX may hold it; the dialects stay alike.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -9,6 +9,8 @@ module.exports = function defineGrammar(dialect) {
       $.__error_recovery,
       // Never returned: valid only after an arrow function's block body (see arrow_function).
       $._arrow_body_end,
+      // The `<` of a JSX closing tag whose `/` a comment follows (see jsx_closing_element).
+      $._jsx_closing_tag_lt,
     ]),
 
     supertypes: ($, previous) => previous.concat([
@@ -401,6 +403,14 @@ module.exports = function defineGrammar(dialect) {
         '/>',
       )),
 
+      jsx_closing_element: $ => seq(
+        // The lexer reads `</` as one token, so a comment between the `<` and the `/` (`<></* c *//>`), which
+        // Babel and espree allow, needs the scanner: it reads that `<` alone, and the comment is an extra.
+        choice('</', seq(alias($._jsx_closing_tag_lt, '<'), '/')),
+        optional(field('name', $._jsx_element_name)),
+        '>',
+      ),
+
       export_specifier: (_, previous) => seq(
         optional(choice('type', 'typeof')),
         previous,
--- a/common/scanner.h
+++ b/common/scanner.h
@@ -15,6 +15,8 @@ enum TokenType {
     ERROR_RECOVERY,
     // Never returned: valid only after an arrow function's block body.
     ARROW_BODY_END,
+    // The `<` of a JSX closing tag whose `/` a comment follows: `<></* c *//>`.
+    JSX_CLOSING_TAG_LT,
 };
 
 static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }
@@ -294,6 +296,18 @@ static bool scan_ternary_qmark(TSLexer *lexer) {
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
 static bool scan_closing_comment(TSLexer *lexer) {
     while (is_space(lexer->lookahead)) {
         skip(lexer);
@@ -320,15 +334,7 @@ static bool scan_closing_comment(TSLexer *lexer) {
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
 
 // After an `&`: whether the rest of a character reference follows, as html_character_reference reads one (`&amp;`,
@@ -426,6 +432,29 @@ static inline bool external_scanner_scan(void *payload, TSLexer *lexer, const bo
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
     if (valid_symbols[AUTOMATIC_SEMICOLON] || valid_symbols[FUNCTION_SIGNATURE_AUTOMATIC_SEMICOLON]) {
         bool scanned_comment = false;
         bool ret = scan_automatic_semicolon(lexer, valid_symbols, &scanned_comment);
```

The scanner reads a `<` where a closing tag may start only when `/*` follows it, and then only the `<`; every other
`<` and `</` is the lexer's, as before. Because the scanner then sees the `<` first, it also reads an HTML comment's
`<!--` there, as `scan_closing_comment` does; the rest of such a comment's line now ends at any line terminator
(Patch 20's `is_line_terminator`). The marker is the last external token, so the others keep their numbers. The `<`
token begins where the scanner began, after any `jsx_text`: when only layout whitespace precedes it, the token spans
that whitespace, as in tree-sitter-javascript. The comment's own `/` is no closing `/`: `<a></* x */a>` stays a parse
error, as in Babel, and so does an unterminated `</* x`. The TypeScript dialect has no JSX, so the token is valid
there only during error recovery, where the scanner returns at the template-chars test first.

Node kinds and field names are unchanged: the closing tag's `<` and `/` are the anonymous `<` and `/`, and the
comment is a `comment` child of the `jsx_closing_element`. The TSX parser has 36 more states; the TypeScript one has
the same count.

tree-sitter-javascript's Patch 12 (a JSX name that is a reserved word or starts with `this`) has no counterpart here:
tree-sitter-typescript 0.23.2 has no reserved-word sets, and `<this.props.tag />`, `<this />`, `<A default="x" />` and
`<label for="x" class="y" />` already parse in TSX.

## Patch 22: `abstract` may name a member or a variable

Upstream issue: none open. Present at tag `v0.23.2` and at upstream HEAD.

Why: `abstract` is a modifier only before `class` or a class member, and TypeScript reads it as a name elsewhere. The
grammar reads it as a keyword wherever the modifier may start, so a member named `abstract` is a parse error in an
object type, an interface and a class: `{ abstract: Set<string>; concrete: Set<string> }` (vscode@729f257's
`typescriptContext/serverPlugin/src/common/typescripts.ts`), `interface I { abstract?(): void }`,
`class C { abstract = 1 }`.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -1293,6 +1293,8 @@ module.exports = function defineGrammar(dialect) {
         'new',
         // A modifier only before a class member's name, as `static` is: `accessor = 1`, `{ accessor }`.
         'accessor',
+        // A modifier only before a class or a member, so it may name a member or a variable: `{ abstract: T }`.
+        'abstract',
         'readonly',
         previous,
       ),
```

`abstract class A {}`, `export abstract class B {}`, `abstract m(): void` and `abstract x: number` read as before.

Node kinds and field names are unchanged: a member named `abstract` is a `property_identifier`, and a variable an
`identifier`. Each generated parser has 19 more states (20 for TSX; 5,883 for TypeScript and 6,169 for TSX in all).

With Patches 15 to 22 and the `yield++⏎x` fix to Patch 14, upstream's corpus parses as before (134 of 135 with twelve
added cases; the one failure patch 1's `foo++!`), and tree-sitter-javascript 0.23.1's corpus builds the same trees
under either dialect. Over 26,252 TypeScript and TSX files (the corpora above, every pack's upstream sources and the
repository's fixtures), 32 files that had syntax errors parse, none gains an error, and every other tree is unchanged,
in the C locale as in a UTF-8 one. The TypeScript and TSX files of those corpora that still fail to parse are broken or
truncated fixtures (vscode's copilot simulation and prompt-selection fixtures, and an 8 MiB copy of TypeScript's
checker cut off mid-expression).

## Patch 23: a class declares a static index signature

Upstream issue: none open (present at tag `v0.23.2` and at HEAD `75b3874`). Rerail: NER-1875.

Why: TypeScript 4.3 allows `static [k: string]: number;` in a class, and upstream's `index_signature` has no
`static`. An interface or an object type may not declare one, so only the class body accepts it.
typescript-eslint's `member-ordering` invalid\[44\] failed on it.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -621,6 +621,7 @@ module.exports = function defineGrammar(dialect) {
             choice(
               $.abstract_method_signature,
               $.index_signature,
+              alias($._class_index_signature, $.index_signature),
               $.method_signature,
               $.public_field_definition,
             ),
@@ -1230,6 +1231,17 @@ module.exports = function defineGrammar(dialect) {
             'readonly',
           ),
         ),
+        $._index_signature_body,
+      ),
+
+      // A class may declare a static index signature (TypeScript 4.3); an object type or an interface may not.
+      _class_index_signature: $ => seq(
+        'static',
+        optional('readonly'),
+        $._index_signature_body,
+      ),
+
+      _index_signature_body: $ => seq(
         '[',
         choice(
           seq(
```

`interface I { static [k: string]: number; }` stays a parse error.

Node kinds and field names are unchanged: the node is an `index_signature` with the same fields (`name`,
`index_type`, `type`, `sign`), and `static` is an anonymous token in it. `_index_signature_body` is hidden, so its
fields stay on the `index_signature`.

## Patch 24: `global { … }` without `declare` is a global augmentation

Upstream issue: none open (present at tag `v0.23.2` and at HEAD `75b3874`). Rerail: NER-1875.
WillBooster/tree-sitter-typescript reads it as an `internal_module` named `global`, the shape used here.

Why: inside `declare module 'x' { … }`, TypeScript accepts `global { interface String {} }` without `declare`,
as a `ModuleDeclaration` named `global`. Upstream accepts `global` only after `declare`. typescript-eslint's
`consistent-type-definitions` invalid\[12\] failed on it.

```diff
--- a/common/define-grammar.js
+++ b/common/define-grammar.js
@@ -651,6 +651,7 @@ module.exports = function defineGrammar(dialect) {
         $.abstract_class_declaration,
         $.module,
         prec('declaration', $.internal_module),
+        prec('declaration', alias($._global_module, $.internal_module)),
         $.type_alias_declaration,
         $.enum_declaration,
         $.interface_declaration,
@@ -757,6 +758,13 @@ module.exports = function defineGrammar(dialect) {
         $._module,
       ),
 
+      // A global augmentation without `declare`, as inside `declare module 'x' { global { … } }`.
+      // `declare global { … }` stays an ambient_declaration, as before.
+      _global_module: $ => prec(-1, seq(
+        field('name', alias('global', $.identifier)),
+        field('body', $.statement_block),
+      )),
+
       _module: $ => prec.right(seq(
         field('name', choice($.string, $.identifier, $.nested_identifier)),
         // On .d.ts files "declare module foo" desugars to "declare module foo {}",
@@ -1293,6 +1301,7 @@ module.exports = function defineGrammar(dialect) {
         'override',
         'readonly',
         'module',
+        'global',
         'any',
         'number',
         'boolean',
```

`declare global { … }` stays an `ambient_declaration`; the lower precedence keeps that reading where both apply.
`global` joins the reserved identifiers, so `global.process`, `const g = global`, a parameter, a label or a member
named `global` stay identifiers or property identifiers. One difference from TypeScript remains: `global` followed by
`{` on the next line reads as an expression statement and a block here, an augmentation in TypeScript. The fork
reads it with its external scanner.

Node kinds and field names are unchanged: the augmentation is an `internal_module` with `name` (an `identifier`,
`global`) and `body`. Each generated parser has fewer states (5,878 for TypeScript and 6,165 for TSX, from 5,883
and 6,169).

With Patches 23 and 24, upstream's corpus parses as before (111 of its 112 cases; the one failure patch 1's
`foo++!`). Over 5,766 TypeScript and TSX files (rerail and nera-landing), no file gains an error and every tree is
unchanged but for these two constructs.
