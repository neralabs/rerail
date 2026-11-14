# Notes: batch js-2 (ESLint core rules, no-new-func to yoda, and three unicorn rules)

58 rules of the javascript pack, written by hand as .rr in four topical groups, then fixed after review. Each group
has its own new library, built on js-1's `eslint-a-*` libraries (around `lib/tsx/eslint-a-common.rr`) and on
statements, modules, names and values:

| Library | Lines | Group |
|---|---|---|
| `lib/tsx/eslint-b-literals.rr` | 65 | A: literals, strings, escapes, comments, comparisons (16 rules) |
| `lib/tsx/eslint-b-objects.rr` | 222 | B: objects, members, calls, destructuring, imports (14 rules) |
| `lib/tsx/eslint-b-control.rr` | 125 | C: statements, control flow, assignments (17 rules) |
| `lib/tsx/eslint-b-scope.rr` | 177 | D: declarations, scope, globals, file names (11 rules) |

One data table: `unicorn-prefer-node-protocol.modules.rr`, from `gen-unicorn-prefer-node-protocol.py`, checked by
`check-unicorn-prefer-node-protocol.py`. Rules: 2,425 lines in all.

Scores are `tools/rrlab/rrlab score javascript <id>` on the rule's own fixtures (ESLint 10.11.0 recording, default
options, matched on path, line and code). 54 of 58 are perfect. The other four are recording artefacts (the recording
drops per-case `languageOptions`: sourceType, ecmaVersion, globals) or a stated limit, each checked against local
ESLint 9.39.3:

| rule | TP/FP/FN | reason |
|---|---|---|
| eslint-no-redeclare | 48/6/21 | script-mode and `builtinGlobals: false` cases read as module defaults; exact against ESLint 9.39 in module, script and builtinGlobals:false modes, and on the review cases |
| eslint-no-obj-calls | 59/27/0 | old-ecmaVersion cases (no globalThis, Reflect, Intl, …); same as ESLint 9.39 at latest |
| eslint-prefer-exponentiation-operator | 169/3/2 | 3 FP ecmaVersion artefact; 2 FN computed names like `Math['p'+'o'+'w']` (a real limit) |
| eslint-no-unused-expressions | 1411/0/7 | ecmaVersion 3 directive cases; ESLint 9.39 at latest does not report them either |

"The global X" is said one way across the batch: eslint-b-scope's `builtin_ref(r, x)` (free, a global variable of
the document, not a named function or class expression's own name, not turned off by `/* global X: off */`).

## Library and core change requests (all groups)

Exact texts are in each group's section below. In short:

1. names.rr: bind a function or class expression's own name inside it (eslint-b-scope's `expr_name` and
   eslint-b-objects' `named_expr_around` then go).
2. names.rr: `directive_global_off(d, x)` for `/* global x: off */` (as js-1 asked), and fill `directive_global` in
   rrlab; eslint-b-scope parses the comments meanwhile (`global_item`).
3. names.rr: `undefined` (its own tree-sitter kind) as a reference and as a parameter name (groups A, D);
   `for (var k in o)` heads declared; `class_static_block` and `generator_function` in `fn_scope` (group D).
4. eslint-a-globals: `self` among the global object names (group D).
5. core: decode identifier escapes in `name` (group B; replaces `id_value`'s table); a `@js_compare(a, b)` functor
   for JavaScript's UTF-16 string order (group A, yoda's exceptRange).

Duplicates still to merge: eslint-b-scope's `called` against eslint-a-globals' `callee`; eslint-b-control's
`lit_val` beside `static_string` (kept apart on purpose: `1` and `"1"` differ). Merged in review: `objects_arg_count`
(now `call_arg_count`), `objects_nullish` (`null_or_undefined`), `fn_expr_own_name` (`expr_name`), the rule-local
`chain` copies (`optional_link`), and fork A's `same_ref` (`ref_same`).


## Group A: literals, strings, escapes, comments, comparisons (16 rules, library `lib/tsx/eslint-b-literals.rr`, 65 lines)

Scores are `rrlab score javascript <id>` on each rule's own fixtures (ESLint 10.11.0 recording, default options),
matched on (path, line, code). The column-level score is the same for all 16. Every ESLint rule was also run beside
local ESLint 9.39.3 (`Linter`, ecmaVersion latest, sourceType module, or script for the octal rules) on its
fixtures, at line:column, at the defaults and at the options listed. unicorn is not installed locally; its row is
the recording only.

| rule | lines | TP/FP/FN | deviations (short) |
|---|---|---|---|
| eslint-no-nonoctal-decimal-escape | 28 | 53/0/0 | none (columns are bytes: after an emoji, ESLint's UTF-16 column differs) |
| eslint-no-octal | 15 | 11/0/0 | none |
| eslint-no-octal-escape | 22 | 60/0/0 | none |
| eslint-no-script-url | 21 | 4/0/0 | none; JSX attribute strings checked too, as ESLint does |
| eslint-no-template-curly-in-string | 17 | 7/0/0 | none |
| eslint-no-useless-concat | 32 | 10/0/0 | none |
| eslint-prefer-numeric-literals | 51 | 62/0/0 | follows the recording (ESLint 10): a local parseInt / Number is not the global |
| eslint-prefer-template | 34 | 79/0/0 | ESLint's autofix (template rebuild with re-escaping) not reproduced |
| eslint-no-warning-comments | 46 | 8/0/0 | none; also checked at location, terms, decoration |
| eslint-no-sparse-arrays | 25 | 8/0/0 | none: one finding per hole, at its comma, as ESLint 9.39+ |
| unicorn-no-null | 54 | 43/0/0 | none against the recording |
| eslint-use-isnan | 58 | 73/0/0 | follows the recording (ESLint 10): a local NaN / Number is not the global |
| eslint-valid-typeof | 38 | 18/0/0 | none; also checked at requireStringLiterals and with `undefined` parameters (with defaults) |
| eslint-yoda | 118 | 103/0/0 | string bounds of exceptRange compared by code point; fix is a suggestion, multi-line operator gaps become one space |
| eslint-no-self-compare | 20 | 15/0/0 | none |
| eslint-no-unsafe-negation | 29 | 6/0/0 | none; also checked at enforceForOrderingRelations |

Total 557 rule lines and 63 library lines; 16 of 16 perfect on their own fixtures.

Options checked against ESLint 9.39 beyond the defaults, all identical at line:column except as noted:
use-isnan enforceForIndexOf and enforceForSwitchCase false; valid-typeof requireStringLiterals; yoda "always",
"always" + onlyEquality, "never" + onlyEquality, exceptRange with both; no-warning-comments location anywhere,
custom terms (`todo!`, `!todo`, `[aeiou]`, `foo-bar`, `[litera|$]`, `*todo`), decoration, terms ["eslint"];
no-unsafe-negation enforceForOrderingRelations.

### Deviations and remaining differences

- **use-isnan, prefer-numeric-literals: scope, after ESLint 10.** The recording (ESLint 10.11) does not report
  `let NaN; x === NaN`, `function f(Number) { return x === Number.NaN }` or `function f(parseInt) {
  parseInt("1", 2) }`; ESLint 9.39 does (it goes by name). The rules follow the recording: the name must not
  resolve to a declaration in the file (`!ref_site`). use-isnan fixtures 103–122 and prefer-numeric-literals
  026–029 are the cases; against 9.39 they are the only differences.
- **yoda exceptRange, string bounds.** `(x <= "bar" || "foo" < x)` compares "bar" <= "foo" as JavaScript does,
  by UTF-16 units. The rule compares by code point (with `join`, which sorts that way); the two differ only for
  characters above U+FFFF against U+E000–U+FFFF. Mixed bounds are coerced as `<=` does: null and false are 0,
  true 1, a numeric string its number, and a bound with no number ('a', a regex) makes the test false:
  `(0 <= x && x < '10')` and `(null <= x && x < 5)` are fine, `('a' <= x && x < 5)` is reported, as in ESLint 9.39.
  A numeric string is read by a regex (decimal forms; not `0x10` or surrounding whitespace, which ToNumber accepts).
  A string comparison functor would remove the code-point approximation (below).
- **yoda fix.** As ESLint's autofix: the operands keep their source text, parentheses included
  (`'red' === (a ? b : c)` → `(a ? b : c) === 'red'`), the spacing around the operator is kept, and a space is added
  where a touching token outside would join the new first or last token (`return"red"===color` →
  `return color==="red"`, b-objects' `touching`/`cannot_touch`). It differs in two ways: no fix when the comparison
  holds a comment, and a gap that spans lines becomes one space. It is a suggestion, not `safe`. rrlab does not apply
  fixes, so the fix text was checked with a probe violation that prints it.
- **prefer-numeric-literals fix.** A word just before the call gets a space, as ESLint's canTokensBeAdjacent
  asks: `return(parseInt)("1", 2)` → `return 0b1`, not `return0b1`.
- **prefer-template fix.** ESLint rebuilds the whole chain as one template, unquoting each literal, escaping
  `` ` `` and `${`, and refusing chains with octal escapes. Not reproduced (no fix offered); the finding is exact.
- **unicorn-no-null optional calls.** unicorn's API exceptions differ in whether `?.` is allowed: the rule follows
  the JSON (insertBefore and bare `useRef` may be optional calls; `Object.create` and `React.useRef` may not). The
  fixtures agree (005: `parentNode?.insertBefore(newNode, null)` is fine).
- **valid-typeof `undefined`.** tree-sitter gives `undefined` a kind of its own, so names.rr's `ref_site` cannot
  say whether it is shadowed. The library's `global_undefined` checks declarations named `undefined` in enclosing
  scopes and parameters spelled `undefined` (fixture 026: `function f(undefined) { typeof x === undefined }`), with a default
  too: `function k(undefined = 1)` is an assignment_pattern whose left field is the name.
- **Columns after non-ASCII text** (no-nonoctal-decimal-escape 048, 058): rrlab and core count bytes, ESLint
  UTF-16 units. Lines agree; the recording's column scores agree too.

### Library and core change requests

- **Proposal, core functor:** a string order, for ESLint rules that compare literal values (`yoda` exceptRange,
  and sort-keys / sort-vars style rules):
  ```
  // @js_compare("bar", "foo") = -1: JavaScript's relational order on strings (UTF-16 code units), -1, 0 or 1.
  .functor js_compare(a: symbol, b: symbol): number
  ```
- **names.rr:** `ref` reads only `identifier` nodes, but tree-sitter parses `undefined` as its own kind (also as a
  parameter, `function f(undefined)`, inside `required_parameter`). So `ref_site` never resolves `undefined`, and
  `decl_site` misses a parameter named `undefined`. Proposed, in `ref`:
  ```
  ref(r, "undefined", "value") :- kind(r, "undefined"), !undefined_param_site(r).
  ```
  plus a `decl_site` row for parameters whose pattern is kind `undefined`. With that, eslint-b-literals'
  `global_undefined` becomes `kind(n, "undefined"), !ref_site(n, _)`.
- **Harness note (not a library):** the shared ESLint runner `scratchpad/js2/eslint-run.cjs` gives no findings for
  `.jsx`/`.ts` files (flat config needs `files` globs and a relative filename); `scratchpad/js2/A/es.cjs` is the
  fixed copy.

### Duplicates to merge

None left. The first draft of eslint-b-literals.rr had `same_ref`, `same_ref_ask` and `ref_pair` (isSameReference)
next to fork C's; fork C renamed theirs to `ref_same` (with a key mode), and this library dropped its own: yoda
uses `ref_same(a, b, "static")` from `lib/tsx/eslint-b-control.rr`.

### Library interface (lib/tsx/eslint-b-literals.rr)

`global_undefined(n)` (the global `undefined`, by declarations in scope), `typeof_expr(e)`, `string_like(x)`
(astUtils.isStringLiteral), `compare_op(o)` (the eight comparison operators), `literal_like(e)` (yoda's Literal,
negative number or plain template), `literal_number(e, f)` (its value as a float). Builds on tsx/names and
tsx/eslint-a-expressions (which brings tsx/eslint-a-common).

## Group B: objects, members, calls, destructuring, imports (14 rules, library `lib/tsx/eslint-b-objects.rr`, 222 lines)

Scores come from `tools/rrlab/rrlab score javascript <id>` on each rule's own fixtures (ESLint 10.11.0 and
eslint-plugin-unicorn 76.0.0 recordings, default options, matched on path, line and code). The column-level score is
the same for every rule. Each ESLint rule was also diffed at line:column against local ESLint 9.39 (`Linter`,
ecmaVersion latest, sourceType module) on its fixtures. That ran at the defaults and at the options listed under the
table. The two agree except where noted. The remaining differences in that diff are fixtures that local ESLint cannot
parse as a module: `var yield`, `with`, `delete foo`.

| rule | lines | TP/FP/FN | deviations (short) |
|---|---|---|---|
| eslint-no-object-constructor | 39 | 51/0/0 | none (Object is a builtin_ref) |
| eslint-no-proto | 19 | 4/0/0 | none (escaped names decoded, as ESLint) |
| eslint-no-prototype-builtins | 52 | 19/0/0 | suggestion: Object's shadowing is checked by scope, not by eslint-scope's variable |
| eslint-no-useless-call | 20 | 25/0/0 | none |
| eslint-prefer-object-has-own | 41 | 33/0/0 | none (global_at: the global variable at the call) |
| eslint-prefer-spread | 19 | 18/0/0 | none |
| eslint-prefer-exponentiation-operator | 77 | 169/3/2 | FP: recording drops ecmaVersion (3); FN: computed names by static evaluation (2) |
| eslint-no-useless-computed-key | 52 | 65/0/0 | none |
| eslint-no-useless-rename | 49 | 130/0/0 | none |
| eslint-sort-keys | 90 | 98/0/0 | none; columns after non-ASCII keys differ by bytes (rrlab) |
| eslint-prefer-destructuring | 75 | 42/0/0 | none |
| unicorn-prefer-node-protocol | 39 (+17 table) | 24/0/0 | module list is the JSON's (generated table) |
| eslint-no-void | 20 | 3/0/0 | none |
| eslint-no-with | 16 | 12/0/0 | none |

Totals: 608 rule lines, 13 of 14 perfect on the recording.

Options checked against ESLint 9.39 beyond the defaults:
- no-void: allowAsStatement.
- no-useless-computed-key: enforceForClassMembers false.
- no-useless-rename: ignoreDestructuring; ignoreImport with ignoreExport.
- sort-keys:
  - "desc";
  - caseSensitive false;
  - natural;
  - "desc" with natural and case-insensitive;
  - minKeys 4;
  - allowLineSeparatedGroups;
  - ignoreComputedKeys.
- prefer-destructuring:
  - `{array: true}`;
  - the nested `{VariableDeclarator: {object}, AssignmentExpression: {array}}`;
  - enforceForRenamedProperties.

All are the same at line:column, except sort-keys' columns on two fixtures with non-ASCII keys (`{è:4, À:3}`).
rrlab's columns are bytes and ESLint's are UTF-16 units. The lines are the same.

### Deviations and remaining differences

- **The global X.** Each rule asks ESLint's own question, through eslint-b-scope:
  - no-object-constructor (`variable.identifiers.length === 0`) and prefer-exponentiation-operator
    (ReferenceTracker: no definitions, no writes) read `builtin_ref`, which honours `/* global X: off */` and a
    function or class expression named X around the reference.
    `/* global Object: off */ new Object();`   `var q = function Object() { return Object(); };`   not reported.
  - prefer-exponentiation-operator also skips a global the file writes (`Math = m;` or `Math++` anywhere), as
    isModifiedGlobal does: `Math = 1; Math.pow(a, b);` is not reported by ESLint 9.39 either.
  - prefer-object-has-own (getVariableByName finds Object in the global scope) reads `global_at(c, "Object")`: no
    declaration around the call but a script's top-level `var Object`, no named expression Object around it, no
    `off`. It is asked at the call, since `({}).hasOwnProperty.call(a, b)` names no Object to be a reference.
- **prefer-exponentiation-operator 021-023: a recording artefact.** `globalThis.Math.pow(a, b)` is a case at an old
  ecmaVersion, where globalThis is no global, and the recording drops that. ESLint 9.39 at ecmaVersion latest reports
  all three, as this rule does.
- **prefer-exponentiation-operator 034, 035: a real limit.** These are `` Math[`${'pow'}`](a, b) `` and
  `Math['p' + 'o' + 'w'](a, b)`. ReferenceTracker evaluates the computed name statically (getStaticValue); this rule
  reads only literal and plain-template names. Folding constant string expressions would need a `static_value` that
  folds `+` and templates. core's static_value is meant to follow constants. rrlab's is only `value`.
- **prefer-exponentiation-operator: aliases.** ReferenceTracker also follows `const m = Math; m.pow(a, b)` and
  `const { pow } = Math`. No fixture has them and this rule does not; eslint-b-scope's `global_value` follows such
  aliases and could take its place (a follow-up, as it changes no score).
- **no-prototype-builtins and prefer-object-has-own: is Object the global?** ESLint asks eslint-scope for the variable
  Object at the call. Here it is `decl_site(s, "Object", …)` with the call inside s. That is the same up to hoisting
  (`let Object` after the call in the same block still counts, as in eslint-scope).
  - prefer-object-has-own: a script's top-level `var Object` is the global variable, so it still reports.
  - no-prototype-builtins: the suggestion is withheld for any declaration, as ESLint's `defs.length > 0` does.
- **Optional chains.** prefer-destructuring and no-prototype-builtins read eslint-b-control's `optional_link`.
  no-prototype-builtins withholds the suggestion as ESLint does: after isAfterOptional, or when the callee is a
  parenthesised chain (`(a?.hasOwnProperty)(k)`); `(a?.b).hasOwnProperty(k)` ends the chain and gets one (the old
  local copy crossed parentheses there). tree-sitter-typescript writes `f?.()` with a `?.` token, not the
  `optional_chain` field optional_link reads, so no-prototype-builtins keeps one clause for `op(c, "?.")`; see the
  request below.
- **unicorn-prefer-node-protocol: the module list.** The list is the JSON's 54 names, generated into
  `unicorn-prefer-node-protocol.modules.rr` by `gen-unicorn-prefer-node-protocol.py`.
  `check-unicorn-prefer-node-protocol.py`
  round-trips it. require and getBuiltinModule are matched by name, as the JSON does, not checked to be the globals.
- **Fixes not scored.** rrlab drops fixes, so they were written from ESLint's source. Each is a suggestion (`fix`,
  never `fix safe`):
  - prefer-exponentiation-operator: parentheses by getPrecedence, and spaces by canTokensBeAdjacent for words and
    `+`/`-` runs, on both sides: `Math.pow(a, b)in c` → `a**b in c`, `a+Math.pow(++b, c)` → `a+ ++b**c` (the fix
    text was printed through a debug copy of the rule, as rrlab applies no fixes).
  - no-useless-computed-key and prefer-object-has-own: spacing.
  - no-useless-rename, prefer-destructuring and no-prototype-builtins: rewrites.
  - One difference: when an argument is already parenthesised, the fix keeps that argument's own parentheses. ESLint
    re-derives them from getText, which excludes them. So `Math.pow((a), b)` becomes `(a)**b`, not `a**b`.

- **sort-keys: names and natural order.** A key's escapes are decoded (`{\u0062: 1, a: 2}` compares b with a, as
  ESLint does), and natural order covers the control characters: codes 0-31 keep their own code and DEL is 64, as
  natural-compare's getCode maps them (`{' b': 1, '\x01a': 2}` is out of order under natural). Checked against
  ESLint 9.39.

### Library and core change requests

1. **Resolved by the js-1 refactor.** The js-1 libraries' clashes (`strip`, `parent_of`, `prop`, `key_name`,
   `keyword`, `fn_node`, `this_region`) are now settled by `lib/tsx/eslint-a-common.rr`. This library and its
   rules were ported to it: `deparen`, `estree_parent`, `static_prop`, `object_prop`, `call_arg`,
   `call_arg_count`, `token_text`/`token_text_ask`. prefer-exponentiation-operator could now use tsx/values'
   alias tracking (`const m = Math; m.pow(a, b)`); that is left as a follow-up.
2. **names.rr: a function or class expression binds its own name inside itself.** Exact text to add after the
   `decl_site` rules for functions:
   ```
   // `const f = function g() { g(); }`: g is bound in the function's own scope, not outside it (eslint-scope's
   // "function-expression-name" scope). The same holds for a class expression's name.
   decl_site(f, x, f, "function", "value") :- (kind(f, "function_expression") ; kind(f, "generator_function")),
       field(f, "name", n), text(n, x).
   decl_site(c, x, c, "class", "value") :- kind(c, "class"), field(c, "name", n), text(n, x).
   ```
   Then eslint-b-scope's `expr_name` and this library's `named_expr_around` go.
3. **names.rr: `/* global x: off */`** (as js-1 asked):
   `.decl directive_global_off(d: number, x: symbol)`. It is native, like directive_global. A rule that asks whether
   a name is the global would exclude these. Until then, these rules read eslint-b-scope's `global_off`, which
   parses the comments in Datalog; with the native fact, global_off becomes it.
4. **core: identifier names with escapes.** ESTree's `Identifier.name` is decoded: `a` is `a`.
   eslint-b-objects decodes the ASCII escapes in Datalog (`id_value`). It uses a 64-row table and one step per
   escape. A native fact would be exact and cover every code point. Proposal: extend core's `name(n, s)` or `value`
   to decode identifier escapes. With that, `id_value(n, x)` becomes `name(n, x)`.
5. **eslint-b-scope: `expr_name` at a position.** prefer-object-has-own asks "is the name Object the global here" at
   a call that may name no Object (`({}).hasOwnProperty.call(a, b)`). expr_name takes a reference, so this library
   keeps `named_expr_around(n, x)` (a named function or class expression x around n). Proposed for eslint-b-scope:
   ```
   // expr_name_at(n, x): n is inside a function or class expression named x, whose name binds x there.
   .decl expr_name_at(n: node, x: symbol)
   expr_name_at(n, x) :- (kind(f, "function_expression") ; kind(f, "generator_function") ; kind(f, "class")),
       field(f, "name", m), text(m, x), within(n, f).
   expr_name(r, x) :- ref(r, x, "value"), expr_name_at(r, x).
   ```
   Not applied: as written it stores a row for every node inside every named function or class expression; it
   wants a demand relation (the positions asked), like `token_text_ask`.
6. **Done:** eslint-b-control's `optional_link` now covers TypeScript's `f?.()` (a `?.` token, no field), and
   no-prototype-builtins' own clause is gone.

### Duplicates to merge

Merged in review: `objects_arg_count` is eslint-a-common's `call_arg_count`, `objects_nullish` is eslint-a-globals'
`null_or_undefined`, `objects_global_name`/`objects_global_object` became `global_named` over eslint-b-scope's
`builtin_ref`, and `fn_expr_own_name` is gone (builtin_ref's `expr_name`; `named_expr_around` stays until request 5).
The rule-local `chain` copies are eslint-b-control's `optional_link`; prefer-exponentiation-operator's `tagged` is
tsx/values'. Locals renamed where tsx/values or tsx/statements now come in: sort-keys' `entry`,
prefer-node-protocol's `module_loader`, prefer-exponentiation-operator's `pow_lead`/`pow_trail`, prefer-object-has-own's
`callee_glued`.

### Library interface (lib/tsx/eslint-b-objects.rr)

Includes eslint-a-functions, eslint-a-expressions and eslint-b-scope (so tsx/values and tsx/statements too).
- Names:
  - `id_value(n, x)`: an identifier's ESTree name, escapes decoded;
  - `module_name(n, x)`: an import or export name, an identifier or a string.
- Calls:
  - `forwarding(c, f, x, t)`: `f.call(t, …)` and `f.apply(t, …)` by static name, parentheses and chains aside;
  - `natural_this(c)`: ESLint's isValidThisArg, with receivers compared by tokens (`token_text`).
- Scope: `global_at(n, x)`, the name x at n resolves to the global variable (getVariableByName in the global
  scope), for the names of `global_at_ask`.
- Globals: `global_member(e, g, x)` reads member x of the global g, a `builtin_ref` the file never writes, directly
  or through globalThis (or declared window, self and global). It is ReferenceTracker's one-level
  iterateGlobalReferences.
- Fix helpers:
  - `precedence(e, p)`: ESLint's getPrecedence on tree-sitter kinds, asked for through `prec_ask`;
  - `touching(t, n)`: no space between token t and node n;
  - `cannot_touch(a, b)`: canTokensBeAdjacent for words and `+`/`-` runs, asked for through `touch_pair`.
- Order: `natural_key(s, k)`. Comparing k with `<` orders s as natural-compare does, control characters included.
  It is asked for through `nat_ask`.

## Group C: statements, control flow, assignments (17 rules, library `lib/tsx/eslint-b-control.rr`, 121 lines)

Scores are rrlab `score javascript <id>` on each rule's own fixtures (ESLint 10.11.0 recording, default options),
matched on (path, line, code). The column-level score is the same for every rule. Each rule was also scored with
`--all` (every fixture); those scores are clean too, except where noted. Rules with options were diffed at
line:column against ESLint 9.39.3 (`Linter`, ecmaVersion latest, sourceType module) on their fixtures at the
non-default value: no-plusplus allowForLoopAfterthoughts, no-return-assign "always", no-sequences
allowInParentheses false, no-self-assign props false, no-unneeded-ternary defaultAssignment false,
no-unsafe-optional-chaining disallowArithmeticOperators, operator-assignment "never". All the same, except for
files that do not parse as a module (`with`), which ESLint does not lint.

| rule | lines | TP/FP/FN | deviations (short) |
|---|---|---|---|
| eslint-no-plusplus | 27 | 25/0/0 | none |
| eslint-no-return-assign | 41 | 11/0/0 | none |
| eslint-no-self-assign | 55 | 43/0/0 | none; array holes counted as ESTree does |
| eslint-no-sequences | 38 | 12/0/0 | none |
| eslint-no-setter-return | 47 | 70/0/0 | the global Object/Reflect via builtin_ref; `get set()` in a descriptor counts, as in ESLint |
| eslint-no-ternary | 14 | 3/0/0 | none |
| eslint-no-unneeded-ternary | 65 | 13/0/0 | messages are ESLint's two message ids, not the JSON's per-fix texts |
| eslint-no-unsafe-finally | 55 | 17/0/0 | follows ESLint 10: a labelled `continue` climbs past loops |
| eslint-no-unsafe-optional-chaining | 64 | 66/0/0 | none |
| eslint-no-unused-expressions | 58 | 1411/0/7 | FN: ecmaVersion 3 cases (directives); option("ecmaVersion", "3") models them |
| eslint-no-unused-labels | 38 | 38/0/0 | none |
| eslint-no-useless-catch | 29 | 5/0/0 | none |
| eslint-no-useless-constructor | 55 | 64/0/0 | JavaScript only (the JSON's scope): TypeScript's accessibility and parameter properties not modelled |
| eslint-require-await | 52 | 31/0/0 | suggestion leaves the space after `async` |
| eslint-require-yield | 22 | 6/0/0 | reports at the function head, as ESLint 10 does (9.39: a method at its `(`) |
| eslint-operator-assignment | 60 | 37/0/0 | "never" fix may add parentheses ESLint would not |
| eslint-no-unexpected-multiline | 38 | 18/0/0 | follows ESLint 10: regex flags include d and v |

Total: 755 lines of rules, 17 rules, 15 perfect on their own fixtures.

### Deviations and remaining differences

- **no-unsafe-finally: ESLint 10.** ESLint 9.39 stops a labelled `continue` at the nearest loop, so
  `var foo = function() { a: while (true) try {} finally { while (true) continue a; } }` (fixture 026) is not
  reported by 9.39; the recording (10.11) reports it, and the jump does leave the finally block. The rule follows
  10: any labelled jump climbs to the function, unless its label is inside the finally block.
- **require-yield: ESLint 10's place.** The recording reports `var obj = { *foo() { return 0; } }` at column 13
  (`*foo`) and `class A { static *foo() {} }` at `static`: getFunctionHeadLoc. 9.39 reports the method's
  FunctionExpression, at `(`. The rule uses `fn_head`; columns match the recording (27/0/0 over every fixture).
- **no-unexpected-multiline: ESLint 10's flags.** `foo⏎ /bar/d.test(baz)` (fixtures 051, 052, flags d and v)
  are recorded; 9.39's flag regex is `[gimsuy]`. The rule uses `[dgimsuvy]`.
- **no-setter-return.** "The global Object" is eslint-b-scope's `builtin_ref`, which honours
  `/* globals Object:off */` (102) and a named function expression's own name,
  `x = function Object() { Object.defineProperty(…) }` (108). Both former FPs are gone. Confirmed against ESLint 9.39: a descriptor's
  getter named set, `Object.defineProperty(o, "a", { get set() { return 1; } })`, is reported, because isSetter asks
  only for a Property named set, whatever its kind. The rule now does the same.
- **operator-assignment and isSameReference on templates.** ESLint's isSameReference has no case for a
  TemplateLiteral, so ``a[`k`] = a[`k`] + 1`` is not reported (ESLint 9.39 agrees); `ref_eq`'s literal clause
  leaves templates out. In "static" mode (no-self-assign, yoda) a template key still matches by its static name
  through getStaticPropertyName, so ``a[`k`] = a[`k`]`` is still a self-assignment.
- **no-unused-expressions, 7 FNs.** `"use strict";`, `"directive one"; "directive two"; f();` and their
  function-body forms (034-037) and `function foo() { 'use strict'; this.eval(); }` (no-eval/077) are ESLint's
  ecmaVersion 3 cases, where there are no directives. At ecmaVersion latest ESLint 9.39 does not report them. The
  rule reports them under option("ecmaVersion", "3") (the JSON's es3-directive clause), which the recording
  does not pass.
- **no-unneeded-ternary messages.** The JSON's messages name the replacement per fix case; the rule keeps
  ESLint's two messages (boolean literals, default assignment) and puts the replacement in the fix, which
  computes ESLint's text: `!==`-style inversion, `!(…)` below unary precedence, `!!x`, `x || (y)`.
- **operator-assignment "never" fix.** ESLint parenthesises the right side only when it binds no tighter than
  the operator; the rule parenthesises any binary or looser right side (`x -= a * b` → `x = x - (a * b)`). Same
  meaning. The fixes keep ESLint's conditions (canBeFixed, no comment between the operators).
- **require-await suggestion.** Removing `async` keeps the space after it (ESLint removes up to the next token);
  `;` is written where ESLint writes it (a following `(`/`[` that would continue the line before: semi_needed for
  expressions, member_runs_on for class members).
- **no-useless-constructor `--all`.** 158 FPs, all in `.ts` fixtures, which the rule's scope (JavaScript, as the
  JSON) excludes; rrlab `--all` ignores scope.

### Library and core change requests

- names.rr: a named function expression's name is a binding in the function's own scope (the group-wide request 1).
  no-setter-return no longer needs it (builtin_ref's `expr_name` covers it), but `ref_site` would then be right for
  every rule.
- names.rr: `directive_global_off` (request 2) would let `builtin_ref`'s `global_off` read it instead of parsing
  comments.

### Duplicates to merge

- `ref_same(a, b, m)` is now the one isSameReference: yoda (group A) uses it in "static" mode, and fork A's
  copy is gone.
- `lit_val(n, k, v)` (typed literal value) overlaps eslint-a-common's `static_string` (untyped); kept apart
  because isSameReference compares `1` and `"1"` as different.

### Library interface (lib/tsx/eslint-b-control.rr)

Builds on eslint-a-common (deparen, static_prop). Precedence: `below_unary(e)`, `below_or(e)` (getPrecedence
thresholds, for fixes that add parentheses). ASI for class members: `member_runs_on(m)`. Arrays as ESTree numbers
them: `slot(a, e, i)` (holes counted, by walking top-level tokens), `slot_count(a, k)`. References:
`ref_same(a, b, m)` with `ref_same_ask(a, b, m)` (isSameReference, m "static" or "exact"), `lit_val(n, k, v)`.
Optional chains: `optional_link(x)`, `chain_at(x)` (where ESTree has a ChainExpression).

## Group D: declarations, scope and globals (11 rules, library `lib/tsx/eslint-b-scope.rr`, 177 lines)

Scores are rrlab `score` on each rule's own fixtures, matched on (path, line, code), against the ESLint 10.11
recording (eslint-plugin-unicorn 76 for filename-case). The column-level score is the same for every rule. Each
ESLint rule was also diffed at path:line against local ESLint 9.39.3 (`Linter`, ecmaVersion latest) on its own
fixtures, at sourceType module and at sourceType script (rrlab `--options '{"sourceType":"script"}'`). They agree,
except where ESLint 9.39 fails to parse (import/export in a script, and `.ts` fixtures, for which the local
typescript-eslint parser did not load) and the ESLint 9 vs 10 differences noted for radix and no-obj-calls.

| rule | lines | TP/FP/FN | deviations (short) |
|---|---|---|---|
| eslint-no-new-func | 20 | 10/0/0 | none |
| eslint-no-new-wrappers | 19 | 5/0/0 | none |
| eslint-symbol-description | 18 | 2/0/0 | none |
| eslint-radix | 50 | 36/0/0 | follows ESLint 10.11 (spread skip, `Number["parseInt"]`, signed literals); no "as-needed" |
| eslint-no-obj-calls | 37 | 59/27/0 | all 27 FPs are old-ecmaVersion cases (recording artefact); same as ESLint 9.39 at latest |
| eslint-no-var | 49 | 77/0/0 | fix is a sure subset of ESLint's (refuses where ESLint's is unsafe) |
| eslint-sort-vars | 39 | 26/0/0 | none; ignoreCase checked against ESLint |
| eslint-vars-on-top | 63 | 27/0/0 | none |
| eslint-no-unused-private-class-members | 65 | 42/0/0 | none |
| unicorn-filename-case | 32 | 116/0/0 | folders checked, as the policy and plugin v76 do |
| eslint-no-redeclare | 60 | 48/6/21 | recording artefacts (sourceType script, builtinGlobals false, ES5); matches ESLint 9.39 exactly in module, script and builtinGlobals:false modes |

Total: 452 rule lines, 177 library lines.

### Deviations and remaining differences

- **radix follows ESLint 10.11, not 9.39.** The recording reports `Number["parseInt"]("10")` and
  `` Number[`parseInt`]("10") `` (9.39 only reads the dotted `Number.parseInt`), `parseInt("10", -1)`, `+37`,
  `-0`, `+10.5` (9.39 only checks bare literals), and skips calls whose first or second argument is a spread:
  `parseInt(...args)` (9.39 reports missingRadix). Checked: ESLint 9.39 on `parseInt("10", -1);
  Number["parseInt"]("10"); parseInt(...a);` reports only the third. The policy has no `as-needed` option (ESLint
  10 dropped it), so only "always" is written.
- **no-obj-calls: 27 FPs, all recording artefacts.** The fixtures are ESLint's tests with their languageOptions
  dropped. The FP files are `globalThis.Math()`, `Reflect()`, `Atomics()`, `Intl()`, `Temporal()` written for an
  ecmaVersion that lacks those globals (ESLint tests them as valid at ES5/ES2017); the same texts appear again as
  invalid cases and are TPs (018 vs 096 are byte-identical). ESLint 9.39 at ecmaVersion latest reports 018, 024, 035
  and 084 exactly as this rule does. `Temporal` is ESLint 10's addition (9.39 lacks it). The browser `window`
  cases (119–124) give no finding here, as in the recording, because `window` is not a global without the browser
  environment.
- **no-obj-calls follows ReferenceTracker,** not the JSON's narrower shapes: aliases through `let m; m = Math`,
  defaults, destructuring of the global object, `a || Math`, `(0, Math)`, TypeScript `as`/`!`, and a global that
  the file assigns (`Math = f`) is not followed (isModifiedGlobal). The messages are the JSON's three.
- **no-redeclare: the score is the recording's options.** Its own FNs (015, 042, 045–048, 065) are cases ESLint
  tests with `sourceType: "script"`, where the top level is the global scope; its own FPs under
  `--options '{"sourceType":"script"}'` are cases with `builtinGlobals: false` or an old ecmaVersion. The remaining
  rows are in other rules' fixtures (`/*globals Reflect: true*/` in ES5 cases of no-obj-calls, which ESLint 9.39 at
  the latest ecmaVersion reports too). Against ESLint 9.39 on the own fixtures: 38/38 lines at module, 51/51 at
  script, 29/29 at script with builtinGlobals false; and on the reviewer's 24 cases (rev-js-2 rd1–rd24), all equal.
- **no-redeclare: what counts as a declaration** (after review). The built-in is a variable's first declaration in
  every document, and a `/* global x */` comment does not remove it, so `/* global Array */` alone is reported in a
  module too, and `/* global Object */ var Object;` in a script reports both lines. A declaration redeclares a
  built-in only in the global scope: a script's top level, per document. That includes a script's top-level
  `let`, `const` and `class`, which eslint-scope puts in the global scope:
  `let JSON; const Math = 1; class Reflect {}` reports all three lines. In a module only the comment line is
  reported: `/* global parseInt */ var parseInt;` gives line 1.
- **no-redeclare comment items** are reported on the item's own line (061–063 put `Array` on a later line of the
  comment). A line break inside `a:⏎ off` is not counted (ESLint drops the white space around `:` first; so does
  this parser). The finding sits on the comment, or on a `@place` of the item's line; the column is the comment's.
- **no-var's fix** (`fix safe to_let`) is offered only where `let` surely means the same: a statement of a
  function's own body (not the global scope, a block, a loop or a switch case), plain names declared nowhere
  else in eslint-scope's variable (no parameter, function or other var of the name: `function f(a) { var a; }`,
  `var a; function a() {}`), none named `let`, no use before the declaration, and no use inside a nested function
  declaration, which is hoisted and may run first. ESLint 9.39 fixes
  `function f() { g(); var a = 1; function g() { return a; } }` to `let`, which throws a ReferenceError (TDZ);
  this rule does not. ESLint fixes more of the safe cases (closures created after the declaration are fixed here
  too); the JSON offers no fix.
- **sort-vars' fix** is ESLint's: the plain names permuted into order, each declarator moved whole, only when
  every initialiser is a literal.
- **unicorn-filename-case** checks every folder of the repository path as well as the file (the policy's
  description; plugin v76 does too: `src/FooBar/index.js` is reported). The file name is checked up to its first
  dot, after leading underscores; a name starting with `$` is skipped. The engine's paths are repository-relative,
  the plugin's are relative to its cwd; they differ only when ESLint runs from a subdirectory.
  Unconfirmed (no local unicorn to check): whether plugin v76 skips or checks `__tests__`-style folders the same
  way as files (leading underscores dropped before the case test); the rule treats folders like file names.
- **no-unused-private-class-members** keys members by name, as ESLint's Map does: a getter/setter pair is one
  entry, reported at the later one (044.js).
- Messages are the JSON's throughout.

### Library and core change requests

1. **tsx/names.rr, function and class expression names.** eslint-scope binds a named function expression's (and
   class expression's) name in a scope of its own; names.rr does not, so `var g = function Function() {
   Function(); }` reads the inner `Function` as the global. This library works around it with `expr_name(r, x)`.
   Proposed, in names.rr's declarations:
   ```
   // `(function f() { f(); })`: a function or class expression's name binds inside the expression only.
   decl_site(f, x, n, "function", "value") :- (kind(f, "function_expression") ; kind(f, "generator_function") ;
       kind(f, "class")), field(f, "name", n), text(n, x).
   ```
   (`f` is not in fn_scope/block_scope for "class", so the lookup needs `class` added to block_scope as well.)
2. **tsx/names.rr, `for (var k in o)` heads.** tree-sitter has no declarator there (the loop's `left` field is the
   identifier), so names.rr declares nothing and `k` resolves outward. Proposed:
   ```
   decl_site(s, x, n, "var", "value") :- kind(f, "for_in_statement"), op(f, "var"), field(f, "left", n),
       kind(n, "identifier"), text(n, x), nearest(f, s, fn_scope).
   ```
3. **tsx/names.rr, var scopes.** fn_scope lacks `class_static_block` and `generator_function`, so `var a` in a
   static block is declared in the enclosing function. Proposed: `fn_scope("class_static_block").
   fn_scope("generator_function").` (statements.rr's var_scope already has both; var_site uses that.)
4. **tsx/names.rr, a parameter named `undefined`.** tree-sitter gives it the kind `undefined`, not `identifier`,
   so `function f(undefined) {}` declares nothing (radix works around it locally). Proposed: let param_name accept
   `kind(p, "undefined")` beside `identifier`.
5. **tsx/names.rr, directive_global.** rrlab has no rows for it, and global comments need their items' settings
   (`off`, `readonly`) and lines for no-redeclare. This library parses them in Datalog (`global_item`). Proposed: give
   directive_global rows from it, `directive_global(d, x) :- global_item(c, x, v, _), v != "off", docof(c, d).`,
   or keep the native reader and add the setting and line columns. Either way `global_off` (the `:off` case js-1
   asked for) is covered.
6. **tsx/eslint-a-globals.rr, global objects.** `global_name` lacks `self`, which ReferenceTracker follows
   (`globalObjectNames`: globalThis, global, self, window). This library's `global_object_name` has all four.

### Duplicates to merge

- `called(c, f)` here excludes tagged templates (ESLint's isCallee); eslint-a-globals' `callee(c, f)` includes them.
  Same meaning otherwise; merge as `callee` plus a `!tagged(c)` guard (values.rr has `tagged`).
- `own_name` was renamed `expr_name` (it clashed with eslint-b-objects.rr's).

### Library interface (lib/tsx/eslint-b-scope.rr)

Globals: `builtin_ref(r, x)` (a reference to a built-in no definition shadows; `/* global x:off */` removes it),
`expr_name`, `global_off`. Calls: `called` (no tagged templates), `constructed`, `member_call`,
`member_call_static`. Value tracking: `global_value(n, x)` with the rule's `traced_global(x)`, after
ReferenceTracker's global references (global object members, pass-through, aliases, destructuring, modified globals).
Comments: `global_item(c, x, v, l)`. Declarations: `var_site(s, x, id)` (var, function declarations and parameters
grouped by eslint-scope's scopes: a function body shares its function's scope, a static block is a var scope),
`global_scope(s)`.
