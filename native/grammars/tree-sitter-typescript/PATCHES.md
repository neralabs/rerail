# tree-sitter-typescript: vendored with patches

Upstream: https://github.com/tree-sitter/tree-sitter-typescript, tag `v0.23.2`
(commit `f975a621f4e7f532fe322e13c4f79495e0a7b2e7`), the version `ast-grep-language` 0.45.3 pins.
tree-sitter and its grammars accept contributions only from maintainers, so Rerail carries its
fixes here instead of upstream.

## Files

| File | Origin |
| --- | --- |
| `LICENSE` | upstream, verbatim |
| `common/scanner.h` | upstream with Patch 6 |
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
strings, and Rerail does not carry them.

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
under TSX. A bare `&` in JSX children (`<p>Terms & Conditions</p>`) is still a parse error: the other 82 of
shadcn-ui's syntax-error files.

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
signature named `instanceof` with no `?` (`a: number⏎  instanceof(): void`) is still a parse error: a `(` after the
keyword does not tell a call signature from `a⏎instanceof (B)`.

Node kinds and field names are unchanged, and so are both grammars' `grammar.json`.

With Patches 4 to 6, upstream's corpus parses as before (114 of 115 with three added cases, one per patch; the one
failure patch 1's `foo++!`). Over 22,898 TypeScript and TSX files of vscode@729f257 and eight other local corpora
(shadcn-ui, excalidraw, prettier, fastify, Next.js examples and commerce, and two private applications), 2 files
that had syntax errors parse (prettier's variance tests), none gains an error, and every other tree is unchanged but
for the error recovery in two prettier files that have syntax errors before and after.
