# tree-sitter-typescript: vendored with patches

Upstream: https://github.com/tree-sitter/tree-sitter-typescript, tag `v0.23.2`
(commit `f975a621f4e7f532fe322e13c4f79495e0a7b2e7`), the version `ast-grep-language` 0.45.3 pins.
tree-sitter and its grammars accept contributions only from maintainers, so Rerail carries its
fixes here instead of upstream.

## Files

| File | Origin |
| --- | --- |
| `LICENSE` | upstream, verbatim |
| `common/scanner.h` | upstream, verbatim |
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

then copy `{typescript,tsx}/src/grammar.json` here. Unpatched, this reproduces upstream's
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
prefix-operator callee in 289 files, and nera `web/src` had 4 in 3 files; with the patch both have
none. Syntax-error files drop from 14 to 13 on vscode and from 13 to 4 on nera
(`await f<typeof import("./api")>()` now parses), and no file gains an error.

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
