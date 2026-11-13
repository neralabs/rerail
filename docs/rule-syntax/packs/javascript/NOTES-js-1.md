# Notes: batch js-1 (ESLint core rules, accessor-pairs to no-new)

59 ESLint core rules of the javascript pack, written by hand as .rr in four topical groups. Each group's
vocabulary is its own new library, over one shared library of the ESTree view they all need:

| Library | Lines | Group |
|---|---|---|
| `lib/tsx/eslint-a-common.rr` | 156 | all: parentheses, ESTree parents, static names, arguments, tokens, `this` |
| `lib/tsx/eslint-a-functions.rr` | 258 | A: functions, classes, accessors |
| `lib/tsx/eslint-a-control.rr` | 98 | B: statements, switch, loops, labels |
| `lib/tsx/eslint-a-expressions.rr` | 210 | C: literals, operators, members |
| `lib/tsx/eslint-a-globals.rr` | 186 | D: globals, shadowing, `this` |
| `lib/tsx/eslint-a-text.rr` | 92 | D: comments |

All six can be included together with statements, modules, values and names (checked by lowering a rule that
includes them all and runs under rrlab). No relation name of an eslint-a library is declared in another library.

The common library (`lib/tsx/eslint-a-common.rr`):
- `deparen(e, x)`, `deparen_count(e, x, k)`: e without the parentheses ESTree does not have.
  `estree_parent(x, p)`: the ESTree parent.
- `function_node(f)`: modules.rr's function_kind, as a node test.
- `static_string(s, v)` (getStaticStringValue), `key_static(k, x)` (a key's getStaticPropertyName),
  `static_prop(n, o, x)` (a member access's, o without parentheses).
- `call_arg(c, a, i)`, `call_arg_count(c, k)`: arguments of a call or `new`, comments skipped.
- `token_text(n, s)` for the nodes of `token_text_ask(n)`: tokens joined by one space (equalTokens).
- `this_frame(r)`, `this_owner(t, s)`: which function, field value, static block or program a `this` has.
- `upper_first(s, t)` for `upper_first_ask(s)`, `capital(x)`; `limit(o, d, k)` with `limit_rule(o, d)`.

Scores are `tools/rrlab/rrlab score javascript <id>` (ESLint 10.11.0 recording, default options, matched on path,
line and code). Since rrlab now scores a rule over the fixture set conformance.json names for it (every fixture of
the rule's file types, 8,104 files for most), nine rules' rows below were rescored: class-methods-use-this,
default-param-last, max-params, no-dupe-class-members, no-empty-function, dot-notation, no-magic-numbers,
no-array-constructor and no-dupe-keys. The two new misses are class-methods-use-this's FN at
no-setter-return/056.js:1 and no-magic-numbers' 5 FPs, numeric keys of destructuring patterns in no-self-assign
and no-useless-computed-key fixtures (`({ 1: a } = b)`), since fixed (review). Most groups also diffed against local ESLint 9.39.3 at
non-default options; see each group. The recording drops each test case's `languageOptions`, so script,
CommonJS, old-ecmaVersion and environment-globals cases are read as defaults: most remaining FNs in group D
come from that. Messages and codes are the JSON's.

Group 0 (coordinator): `eslint-no-debugger`, 13 lines, 1/0/0, no deviations.

## Group A (functions and classes)

### 1. Table

Scores are rrlab `score` on each rule's own fixtures, matched on (path, line, code). The column-level score is
the same for every rule. Every rule was also diffed at line:column against ESLint 9.39.3 (espree, and
@typescript-eslint/parser for .ts) on its fixtures, with the default and the non-default options listed below.

| rule | lines | TP/FP/FN | deviations (short) |
|---|---|---|---|
| eslint-accessor-pairs | 65 | 118/1/0 | FP: `/* globals Object:off */` is not modelled |
| eslint-arrow-body-style | 89 | 42/0/0 | fix is more conservative (comments, for heads) |
| eslint-class-methods-use-this | 63 | 787/0/1 | none |
| eslint-default-param-last | 22 | 16/0/0 | follows ESLint: optional `a?` counts as not required |
| eslint-func-name-matching | 104 | 47/1/0 | FP: recording artefact (ecmaVersion 5 case); identifiers by `\p{ID_Start}`/`\p{ID_Continue}` |
| eslint-func-style | 66 | 25/0/0 | none |
| eslint-grouped-accessor-pairs | 40 | 38/0/0 | none |
| eslint-max-classes-per-file | 27 | 11/0/0 | none |
| eslint-max-nested-callbacks | 32 | 1/0/0 | counts the real depth; ESLint's stack undercounts after a non-callback |
| eslint-max-params | 31 | 5/0/0 | none |
| eslint-max-statements | 39 | 2/0/0 | none |
| eslint-no-dupe-class-members | 40 | 57/0/0 | none |
| eslint-no-empty-function | 60 | 2069/0/0 | none |
| eslint-no-empty-static-block | 22 | 6/0/0 | none |
| eslint-no-extra-bind | 43 | 35/0/0 | follows the recording (ESLint 10.11): field initialisers and static blocks have their own `this` |

Library: `lib/tsx/eslint-a-functions.rr`, 315 lines.

Options checked against ESLint 9.39 beyond the defaults: max-params, max-statements and max-nested-callbacks with a
small max; max-statements ignoreTopLevelFunctions; no-empty-function allow lists; class-methods-use-this
exceptMethods, enforceForClassFields, ignoreOverrideMethods, ignoreClassesWithImplements (both values);
accessor-pairs getWithoutSet, setWithoutGet, enforceForClassMembers, enforceForTSTypes; grouped-accessor-pairs
both orders with enforceForTSTypes; arrow-body-style always, never, as-needed with requireReturnForObjectLiteral;
func-style declaration, allowArrowFunctions, allowTypeAnnotation, overrides.namedExports (all three);
func-name-matching never, includeCommonJSModuleExports, considerPropertyDescriptor. All the same at line:column,
except where noted below. The messages are ESLint's, with the JSON's advice appended where it had some.

### 2. Deviations and remaining differences

**accessor-pairs, 1 FP (fixture 067.js).** `/* globals Object:off */ Object.defineProperty(foo, 'bar', { set(v) {} })`.
ESLint 10.11 checks that `Object` and `Reflect` are the globals, and a `globals … :off` comment turns `Object` off,
so it does not report. The rule checks for a local declaration (names.rr `ref_site`), so
`function f(Object) { Object.defineProperty(…) }` is fine, but names.rr's `directive_global` has no `:off` form and
rrlab has no rows for it. ESLint 9.39 reports all three descriptor fixtures (063, 064, 067); the rule follows the
recorded 10.11.

**default-param-last.** ESLint reports every parameter that is not required (a default, or TypeScript's `a?`)
before a required one: `function f(a?: number, b: number) {}` reports `a?`. The JSON reported only
assignment patterns. The JSON's scope is JavaScript only, kept; the rule reads TypeScript parameters too.

**func-name-matching, 1 FP (fixture 104.js).** `var obj = { 'ᢅ': function foo() {} };` is byte-identical to
134.js, which the recording reports. The ESLint test runs one of them with ecmaVersion 5 (U+1885 is no ES5
identifier) and the recording drops languageOptions. ESLint 9.39 at ecmaVersion latest reports both, as this
rule does. Identifier names: esutils tests Unicode ID_Start/ID_Continue, and so does the rule, with the regex
classes `\p{ID_Start}` and `\p{ID_Continue}` (the first version spelled out UTF-8 byte ranges, an approximation
that stopped matching once rrlab moved to Rust regex: 46/0/1 until rewritten).
A computed key names the function only when it is a literal, as in ESLint: `obj['foo'] = function bar() {}` is
reported, ``obj[`foo`] = function bar() {}`` is not.
Also: under considerPropertyDescriptor, ESLint names a descriptor by `key.name` of the outer property, which is
`undefined` for a string key (`Object.create(p, { "a": { value: function b() {} } })` reports "…property name
`undefined`"). The rule only reads identifier keys there, so it does not report that case.

**max-nested-callbacks.** ESLint pops its callback stack on the exit of every function expression and arrow,
callback or not (popStack is unconditional), so a non-callback function inside a callback lowers the count of
what follows. The rule counts the real nesting, which is what the rule documents. Counterexample, max 2:
`foo(function () { bar(function () { baz(() => { const g = () => 1; ({ m() {} }); }); }); });`
ESLint 9.39 reports baz's arrow (depth 3) only; this rule also reports `m` (depth 3). Without `const g`, ESLint
reports `m` too. checkConstructorCallCallbacks (`new Foo(() => {})`) is read from the JSON's option list; ESLint
9.39 does not have it.

**max-params.** countThis is the JSON's option ("always", "never", "except-void"); the deprecated
countVoidThis: true is read as "always". The JSON's scope is JavaScript only, kept, so TypeScript's
TSDeclareFunction and TSFunctionType (which ESLint also checks) are not read.

**no-extra-bind.** ESLint 10.11, which made the recording, gives class field initialisers and static blocks
their own `this`: `var a = function () { class C { field = this.x; } }.bind(b)` is reported (fixtures 047–049).
ESLint 9.39 does not report these. The rule follows the recording. The fix replaces the call with the callee's
object as written, parentheses kept (`(() => a).bind(b)()` becomes `(() => a)()`, as ESLint's), only for a side-effect-free argument and with no comment inside; ESLint removes the same
tokens.

**arrow-body-style fixes.** The unbrace fix writes the returned value in the block's place, parenthesised when it
starts with `{` (ESLint tests the first token: `return {a: 1}.a` is an "unexpectedObjectBlock" too) or is a
sequence. It is withheld where ESLint's is (the next token is `(`, `[`, `/`, `` ` ``,
`+`, `-`), and also when a comment is inside the block (ESLint keeps comments by deleting tokens around them) and
inside any for statement (ESLint parenthesises when an `in` would change the for head's meaning). The brace fix
replaces the whole body, parentheses included (`() => (1)` becomes `() => {return (1)}`), and drops parentheses
that only made an object literal an expression (`() => ({})` becomes `() => {return {}}`), as ESLint does.

**no-empty-function, no-empty-static-block.** tree-sitter puts a comment that follows a block's `}` on the same
line inside the block node (`function f() {} // x`). `empty_block` counts a comment only when it starts before
the closing brace, as ESLint does.

**Soufflé `count` and `_`.** `count : { owned(b, f), block_stmt(b, _) }` counts distinct b, not statements:
the wildcard is projected away. Name the variable (`block_stmt(b, x)`). Also, a variable inside an aggregate
that is also bound outside it is the outer one: `count : { call_arg(c, a, i) } = 1, call_arg(c, a, 0)` counts
only that a. Worth a line in the language guide.

### 3. Library and core change requests

None needed for these rules. Two proposals:

- names.rr: `directive_global(d, x)` has no way to say `/* globals x:off */`. Proposed:
  ```
  // directive_global_off(d, "Object"): a `/* global Object:off */` comment removes a global (ESLint's "off").
  .decl directive_global_off(d: number, x: symbol)
  ```
  and `global_ref` (and rules that ask "is this the global") would exclude those names.
- modules.rr's `unparen(e, l)` only unwraps to strings. eslint-a-common's `deparen(e, x)` (any expression,
  comments skipped) is the general form: `unparen(e, l) :- deparen(e, l), kind(l, "string").`

### 4. Library interface (lib/tsx/eslint-a-functions.rr)

Functions and parameters: `fn`, `fn_param(f, p, i)`, `default_param`, `rest_param`, `optional_param`,
`this_param(p, void)`, `param_count`, `fn_block`, `empty_block`, `before_close`.
Members and keys: `fn_holder(f, h)` (ESTree's Property / MethodDefinition / PropertyDefinition parent),
`held_value`, `holder_key`, `estree_key` (computed key without brackets), `static_key` (getStaticPropertyName),
`private_key`, keys with no static name compared by common's `token_text`, `member_kind`, `is_static`,
`class_member`, `fn_id`, `generator`.
Accessors: `accessor(l, s, m, k, w)` (getters and setters by list, static-ness and key, as accessor-pairs and
grouped-accessor-pairs group them), `accessor_name`, `accessor_at`, `list_index`.
ESLint's names and places: `fn_name_kind` / `fn_words` (getFunctionNameWithKind), `fn_head`
(getFunctionHeadLoc), `fn_start` (where ESTree's function node starts).
Nesting: `own_fn`, `callback`. Calls: `method_call(c, o, x)` (isSpecificMemberAccess).
From eslint-a-common: `function_node`, `deparen`, `key_static`, `static_prop`, `call_arg`, `this_owner`,
`upper_first`, `limit`.

## Group B: control flow (19 rules, library `lib/tsx/eslint-a-control.rr`, 126 lines)

Scores are rrlab on each rule's own fixtures, matched on (path, line, code). The column-level score is the
same for all 19. Every rule was also run beside ESLint 9.39.3 (`Linter`, ecmaVersion latest, sourceType
module) on the same fixtures, at defaults and under the options listed. They agree exactly, except for the one
row noted under no-delete-var.

| rule | lines | TP/FP/FN | deviations (short) |
|---|---|---|---|
| eslint-default-case | 34 | 8/0/0 | commentPattern is a JS regex taken as given; also checked against ESLint with `commentPattern` |
| eslint-default-case-last | 15 | 14/0/0 | none |
| eslint-guard-for-in | 31 | 6/0/0 | none |
| eslint-max-depth | 43 | 2/0/0 | follows ESLint 10 (fixtures 011, 012): no 9.39 `else if` pop quirk, and `if (a) if (b)` nests (see below) |
| eslint-no-await-in-loop | 42 | 17/0/0 | a method's computed key is not a boundary; TS grammar cannot parse `for (await using x of xs)` (grammar gap) |
| eslint-no-case-declarations | 27 | 15/0/0 | suggestion fix added (ESLint's addBrackets) |
| eslint-no-continue | 15 | 4/0/0 | none |
| eslint-no-dupe-else-if | 71 | 59/0/0 | none: ESLint's split and subset algorithm in full |
| eslint-no-duplicate-case | 25 | 19/0/0 | none: token-for-token comparison, as in equalTokens; `.scope` is the JSON's 8 globs |
| eslint-no-empty | 43 | 17/0/0 | switch finding at the `{` (as in ESLint and the JSON); a `} // note` comment does not fill the block; suggestion fix added |
| eslint-no-extra-label | 34 | 19/0/0 | the fix is dropped when the jump has any comment, not only one between the keyword and the label |
| eslint-no-labels | 26 | 27/0/0 | a jump to an undeclared label (a syntax error) is not reported; checked against ESLint with allowLoop and allowSwitch |
| eslint-no-lonely-if | 37 | 14/0/0 | none: the fix has ESLint's comment and ASI guards |
| eslint-no-cond-assign | 42 | 11/0/0 | the "always" mode also matches ESLint on every fixture |
| eslint-no-negated-condition | 24 | 6/0/0 | none |
| eslint-no-nested-ternary | 21 | 2/0/0 | none |
| eslint-no-new | 20 | 1/0/0 | none |
| eslint-no-multi-assign | 22 | 22/0/0 | none; checked against ESLint with ignoreNonDeclaration |
| eslint-no-delete-var | 19 | 1/0/0 | none |

19 of 19 are perfect on their own fixtures.

### Notes

- **Messages.** Each rule keeps the JSON's message text, code and suppress lines. The JSON's per-clause
  variants are folded into one violation with a message column (no-labels, no-extra-label, no-cond-assign,
  no-empty). The revision is the JSON's plus one.
- **max-depth: ESLint 10, not 9.39.** In ESLint 9.39, `IfStatement:exit` pops the counter for every if, but
  `IfStatement` pushes only for an if whose parent is not an if. So each finished `else if` (or `if (a) if (b)`)
  lowers the count for the rest of its function: `function f() { if (a) {} else if (b) {} if (c) { if (d) {} } }`
  with `max: 1` gives no finding in 9.39, and `if (a) if (b) {}` none either. The 10.11 fixtures add exactly these
  two cases (011, 012), between the else-for and the while cases of ESLint's *invalid* list (fixtures are numbered
  in test order: 010 is the invalid `if (true) {} else { for(;;) {} }` at max 1, 013 the invalid `while` one).
  So ESLint 10 reports both, and the rule follows it: only an `else if` is exempt, and the count is plain
  nesting (`if (d)` at 1:52 in 011, `if (b)` at 1:8 in 012, at max 1). This is read from the fixtures' order;
  the recording itself runs at max 4 and has no finding on either.
- **max-depth: options.** `option` is a number, or `max`, or the deprecated `maximum`. The default is 4. They are
  read with common's `limit`; `maximum` wins over `max` when both are set, as ESLint's `option.maximum || option.max`.
- **default-case: commentPattern.** ESLint builds `new RegExp(pattern, "u")` and searches the trimmed last
  comment. Here it is `match("[\s\S]*(?:" + p + ")[\s\S]*", v)` on the trimmed text: a user regex, run as
  given. `[\s\S]` dates from rrlab's std::regex, which lacked `(?s)`; with Rust regex `(?s:.)` would do as well,
  and `[\s\S]` is kept (equal, and the engine's dialect reads both). The default is
  `(?i)no default`, a whole-text match.
- **no-await-in-loop: methods.** The climb stops at a function's own code (modules.rr's `fn_code`): for a method
  that is its parameters and body. A computed key runs in the surrounding code, so
  `for (;;) { class A { [await k]() {} } }` is reported, as ESLint does (checked with 9.39).
- **no-await-in-loop.** `for (await using x of xs)`: tree-sitter-javascript parses it (an `await` and a
  `using` token on the for_in_statement), and the rule reports it, as ESLint does (037.js). tree-sitter-typescript
  does not parse it: the file becomes an empty program, so in a .ts file nothing is reported. This is a grammar
  gap. TypeScript's `await using x = f()` statement is read through statements.rr's `var_decl(d, "await using")`,
  and the inner await_expression the TS tree puts on its first assignment is not reported a second time.
- **no-delete-var.** ESLint 9.39 with sourceType module rejects `delete x` as a parse error (strict mode), so
  it has no finding there. The 10.11 recording has one, presumably from a script-mode run. The rule reports
  regardless of strictness, as the recording does.
- **no-extra-label: the fix.** The fix rewrites the jump as `break;`/`break` (or `continue…`), keeping its
  `;` or ASI. It is withheld when the jump contains a comment anywhere. ESLint withholds it only for a comment
  between the keyword and the label. This rule is more conservative, because rewriting the whole jump would
  drop a comment after the label.
- **no-labels.** A `break x` with no enclosing `x:` is a syntax error. ESLint's getKind gives "other" and
  reports it; this rule has no labeled statement to look at, so it does not.
- **no-empty.** A block counts when nothing starts between its braces (functions' `empty_block`): tree-sitter
  hangs a comment after the `}` on the same line inside the block, so `if (a) {} // note` is still empty, as in
  ESLint. Its parent must not be a function, a static block or a TypeScript namespace or module, which are not
  BlockStatements in ESTree. A switch is empty when no case and no comment starts before its `}`: ESLint skips
  `switch (a) { /* later */ }` (commentsExistBetween) and reports `switch (a) {} // n`. Checked with 9.39. The
  JSON scope (JS files only) is kept.
- **Harness observation (not a rule issue).** `rrlab run` with files from two unrelated directories (a
  scratch file plus fixtures) seems to take the common ancestor as `--root`, and then hangs scanning it. Run
  files from one directory.

### Library and core change requests

None needed. One suggestion for integration: common's `deparen(e, x)` is generic, and `lib/tsx/modules.rr`'s
`unparen` covers only strings. Consider moving `deparen` to `tsx/statements.rr` or `tsx/modules.rr`, with
`unparen(e, l) :- deparen(e, l), kind(l, "string").`

### Library interface (`lib/tsx/eslint-a-control.rr`)

Includes `tsx/eslint-a-common.rr` and `tsx/statements.rr`.

- **Statements:**
  - `first_stmt(b, x)`: x is the first statement of block b;
  - `lone(b, x)`: body b runs the single statement x, braced or not;
  - `empty_body(b)`: b is `;` or `{}`;
  - `case_stmt(c, x, i)` and `case_last(c, x)`: the statements of a switch clause.
- **Loops and jumps:**
  - `loop`, `loop_kind`, `breakable`, `breakable_kind`;
  - `for_await(s)`;
  - `repeated(l, r)`: the parts of a loop that run on every iteration (ESLint's isLooped);
  - `label_of(l, x)`, `jump(j, w)` and `jump_label(j, x, l)`.
- **Frames:** `frame_kind` (functions, static blocks and the program, where nesting counts restart).
- **ESTree expressions:** `negated(e)`, `assignment(a)`. Parentheses (`deparen`, `deparen_count`) and
  token-joined text (`token_text` for the nodes a rule puts in `token_text_ask`, used by no-duplicate-case and
  no-dupe-else-if) are common's.

## Group C (expressions and literals)

Library: `lib/tsx/eslint-a-expressions.rr` (247 lines). Scores are `rrlab score javascript <id>` on each rule's own
fixtures (ESLint 10.11 recording, default options), matched on (path, line, code). Options were also checked
against ESLint 9.39 on ad-hoc files (eqeqeq, dot-notation, no-bitwise, no-empty-pattern, no-magic-numbers):
the findings were the same.

### (1) Table rows

| rule | lines | TP/FP/FN | deviations (short) |
|---|---|---|---|
| eslint-dot-notation | 63 | 189/0/0 | the JSON's 16 clauses are 2 findings and 2 fixes |
| eslint-eqeqeq | 51 | 39/0/0 | a template literal is not a Literal (the JSON counts `` `a` == "a" `` as fixable); the unsafe case gets a suggestion, as in ESLint |
| eslint-no-bitwise | 26 | 16/0/1 | FN is a recording artefact: two findings at one key |
| eslint-no-compare-neg-zero | 19 | 23/0/0 | — |
| eslint-no-eq-null | 14 | 3/0/0 | — |
| eslint-no-loss-of-precision | 26 | 50/0/0 | follows ESLint 10 (reports an underflow to 0, as in `1e-350`); exact bit count for 0x/0b/0o |
| eslint-no-magic-numbers | 101 | 1984/0/0 | — |
| eslint-no-array-constructor | 44 | 84/0/0 | `semi_needed` leaves out ESLint's TypeScript type-context cases |
| eslint-no-div-regex | 17 | 1/0/0 | — |
| eslint-no-dupe-keys | 30 | 57/0/0 | — |
| eslint-no-empty-pattern | 35 | 18/0/0 | handles the JavaScript grammar's parameter shapes too (the JSON handles only TypeScript's) |
| eslint-no-multi-str | 20 | 5/0/0 | U+2028/U+2029 match since rrlab uses Rust regex (were the 2 FN); a string inside JSX only counts when it is the direct child (ESLint), not anywhere inside (JSON) |
| eslint-no-iterator | 13 | 5/0/0 | identifiers written with `\u` escapes are not decoded |
| eslint-no-caller | 16 | 2/0/0 | — |

14 rules, 468 lines, 13 with perfect scores (no-multi-str since rrlab's Rust regex). The other miss, no-bitwise, is not a rule defect.

### (2) Deviations and remaining misses

- **no-bitwise, 010.js (`~1<<2 === -8`).** ESLint reports `~` and `<<`, and both are at line 1, column 1.
  A finding's key is (policy, code, path, line), so the two are one finding. The recording counts two.
- **no-multi-str, 006.js and 007.js** (`'foo\` followed by U+2028 or U+2029). The rule tests ESLint's
  LINEBREAK_MATCHER, `[\r\n\u2028\u2029]`. rrlab's old std::regex read bytes and missed them; with Rust regex they
  match, and the rule scores 5/0/0.
- **no-multi-str, JSX.** ESLint skips a literal only when its direct parent is a JSX node. The JSON skips any
  string inside a `jsx_expression`. Counterexample: in `<a b={f("x\⏎y")}/>` ESLint reports the string (its
  parent is the call), and the JSON does not. This rule follows ESLint.
- **eqeqeq, template literals.** The JSON's `string-lit` includes templates without `${}`, so its fix-loose
  clause autofixes `` `a` == "a" ``. In ESLint a TemplateLiteral is not a Literal: the case is reported with a
  suggestion only. This rule follows ESLint. ESLint offers that suggestion for every case it does not autofix;
  the JSON offers no fix there. This rule offers it.
- **no-loss-of-precision, underflow.** The recording (ESLint 10.11) reports `1e-350`, `1e-324` and `-1e-350`.
  ESLint 9.39's source skips every literal whose value is 0. This rule follows the recorded tool: a literal
  whose significant digits are not all zeros, but whose value is 0, loses precision. `0`, `0.0` and `0e5` do
  not.
- **no-loss-of-precision, method.** A base-10 literal is normalised the way ESLint's
  convertNumberToScientificNotation does it, with captures and `@replace_all`, and then compared with
  `@to_precision`. A base-2, -8 or -16 literal counts the bits from its first 1 to its last 1, from the digits
  (`significant_bits`). For an integer literal, that is the same as ESLint's
  `raw.endsWith(value.toString(base))`. The JSON writes the same thing as long regexes. rrlab's `to_precision`
  uses `floor(log10)` and a 64-byte buffer, so its rounding at a power of ten and above about 60 digits is
  approximate. No fixture is affected.
- **no-iterator, escapes.** `o.__iterator__` is `__iterator__` to ESLint. The rule compares the
  identifier's text, so it misses this. The JSON spells out every escape in a regex. The real fix is
  core-level (below).
- **no-magic-numbers.** A numeric key is a Property's in a destructuring pattern too (`({ 1: a } = b)`,
  `var { [0]: a } = o`), so it is not reported. `ignore: ["0n"]` also ignores `-0n`, as ESLint's BigInt
  comparison does. ignoreNumericLiteralTypes climbs unions only (`type T = 1 & 2` is reported);
  ignoreTypeIndexes climbs unions and intersections. Checked against ESLint 9.39.
- **dot-notation fixes.** The fix replaces the `[`, the key and the `]` with `.key` (`?.key` after `?.`, and
  ` .key` after a decimal integer). It is not offered when a comment sits between the brackets. As ESLint,
  it adds a space when the next token starts right after `]` with a name character (`a['b']in c` becomes
  `a.b in c`).
- **no-array-constructor fix.** `semi_needed` is ESLint's `needsPrecedingSemicolon`, case by case on the token
  before. It leaves out the TypeScript cases (`TSDeclareFunction`, import-equals, type annotations), where
  ESLint answers "no". The rule's scope is JavaScript files only, so those cases do not arise.
- **rrlab fact quirk (not a rule issue).** A token is its own `last_token`, so "no node ends at this `)`" has
  to be written `count : { last_token(m, t), m != t } = 0`. A capture fed straight into `to_number` was
  evaluated on rows the match had rejected (`to_number("")` aborted Soufflé). `cat("0", X)` keeps it total.
  Engine authors may want to check both.

### (3) Library and core change requests

- **Core: decoded identifier names.** Proposed:
  ```
  // ident(n, "__iterator__") for `__iterator__`: an identifier or property name with its \u escapes decoded,
  // as ESTree's Identifier.name holds it. Equal to text(n, _) for nearly every identifier.
  .decl ident(n: node, x: symbol)
  ```
  no-iterator, no-caller, dot-notation and no-dupe-keys would then compare names as ESLint does.
- **Core, documentation only:** say in core.rr that a token is its own `first_token` / `last_token` (if the
  engine does the same), so rules do not write `!last_token(_, t)` expecting "no node ends here".
- **Shared vocabulary that belongs in an existing library at integration:** common's `deparen` (a general form
  of modules.rr's `unparen`, which accepts only strings), and `stmt_start` / `semi_needed`, which other ESLint
  rules with fixes need (no-object-constructor, prefer-* rules). values.rr's `member` / `key_name` /
  `const_str` are narrower than common's `static_prop` / `key_static` / `static_string` (no number, boolean,
  null or regex keys; member also reads patterns), so they were not reused.

### (4) Library interface (lib/tsx/eslint-a-expressions.rr)

ESTree as tree-sitter shows it: parentheses removed and Literal types classified.
- From eslint-a-common: `deparen`, `estree_parent`, `static_string`, `key_static`, `static_prop`.
- `number_lit`, `bigint_lit`, `zero`, `neg_zero`, `null_lit`, `literal_type(e, t)`: literals, and what
  `typeof` their value gives.
- Numerals for precision: `raw_number`, `decimal`, `sci(s, f, c, m)` (ESLint's scientific normalisation),
  `stored(n, p)` (toPrecision), `significant_bits(n, k)`.
- `binop(n, o, l, r)`, `op_token(n, t)` (the operator token, for findings and fixes), `equality(o, strict)`.
- `dotted_member`, `object_prop(o, p, k, x, key)` (object-literal properties with kind init/get/set).
- `stmt_start(x)`, `semi_needed(x)` (needsPrecedingSemicolon), and the edit `as_array(n, lead)`.
- `es3_reserved(x)` (ESLint's ES3 keyword list), `ident_name(s)`.

## Group D: globals, comments, names, imports

Ten ESLint core rules, on two new libraries, `lib/tsx/eslint-a-globals.rr` and `lib/tsx/eslint-a-text.rr`, and one
generated data table (no-irregular-whitespace's characters).

### How the rules were checked

- `rrlab score` against the recording, at default options.
- A local comparison with ESLint 9.39.3: `Linter.verify` on the same fixtures, with `@typescript-eslint/parser` for
  `.ts`, compared by (file, line). It runs at default options and at the option sets each rule has, and also with
  `sourceType: "script"` and `"commonjs"` for the rules where that matters. The scripts were throwaway
  (not committed).
- Where ESLint needs environment globals (window, setTimeout), which rrlab cannot provide (it has no `es_global`
  rows), a throwaway copy of the rule with a few `es_global("…")` facts was compared with ESLint run with the same
  globals.

The recording is not one run of ESLint at defaults. Identical fixtures get different verdicts: `global.eval('foo')`
is 010.js (valid) and 046.js (invalid). The recording keeps each RuleTester case's expected errors, but drops the
case's `languageOptions`: sourceType, ecmaVersion and globals. This is the harness gap the rrlab README names for
eslint-strict. It also counts every report, so two reports on one line need two findings. A finding key is (path,
line), so a rule cannot produce two findings on one line. Most of the FPs and FNs below come from these two things.

### Table

| rule | lines | TP/FP/FN (own fixtures) | ESLint 9.39 local, defaults | deviations (short) |
|---|---|---|---|---|
| eslint-no-alert | 28 | 20 / 3 / 2 | 23 / 0 / 0 (script 25 / 0 / 0) | recording: ecmaVersion < 2020 and script cases |
| eslint-no-console | 39 | 23 / 0 / 0 | 23 / 0 / 0 (allow 16 / 0 / 0) | adds ESLint's suggestion as a fix; `(console).log` too |
| eslint-no-eval | 30 | 16 / 3 / 20 (script 32 / 8 / 4) | 19 / 0 / 3 parse errors; script 40 / 0 / 1; commonjs 45 / 0 / 0 | recording: script, browser/Node and ecmaVersion cases; follows ESLint 10's `Array.fromAsync` |
| eslint-no-implied-eval | 49 | 4 / 2 / 17 | 6 / 0 / 0 (commonjs 28 / 0 / 0; with globals 48 / 0 / 2) | recording: environment globals; static_value is not modelled in rrlab |
| eslint-no-extend-native | 30 | 22 / 0 / 0 | 21 / 0 / 1 parse error | none |
| eslint-capitalized-comments | 55 | 51 / 0 / 0 | 47 / 0 / 4 (+3 unused-directive reports) | lower case by `\p{Ll}\p{Lt}`; the fix upper-cases ASCII only until core has `@upper` |
| eslint-no-inline-comments | 27 | 36 / 0 / 0 | 35 / 0 / 0 (+2 unused-directive reports) | none |
| eslint-no-irregular-whitespace | 47 (+ 31 table) | 186 / 5 / 5 (columns too; rrlab now runs match_all) | 176 / 2 / 5 (variant) | one finding per run, at the run; U+2028/U+2029 line numbering (the 5 FP / 5 FN) |
| eslint-id-length | 116 | 101 / 0 / 1 | 86 / 0 / 1 | counts code points, not graphemes |
| eslint-no-duplicate-imports | 65 | 19 / 0 / 0 | 19 / 0 / 0 (every option set: perfect) | none |

Five rules score perfectly against ESLint 9.39 at every option set tried: no-console, no-extend-native,
no-inline-comments, no-duplicate-imports and no-alert. The `Unused eslint-disable directive` lines, which ESLint
itself reports, are not counted. The worst recording scores, no-eval, no-implied-eval and no-alert, all come from
cases whose environment the recording lost.

### Deviations, with counterexamples

**Environment globals (no-alert, no-eval, no-implied-eval, no-extend-native).** ESLint resolves a name to a
global variable only when the environment declares one. The ECMAScript built-ins are always declared (globalThis,
eval, Array, …). The browser's (window, setTimeout) and Node's (global) come only from config globals or
`/* global */` comments. A CommonJS file adds exports, global, module and require (ESLint's conf/globals.js).
`global_var(d, x)` is that set:
- `es_builtin`, a table in the library: the globals package's es2026 set, plus the four names ESLint 10 adds;
- the engine's `es_global`, which rrlab leaves empty;
- `directive_global`, also empty in rrlab;
- the CommonJS four.

So in rrlab, `window.eval(x)` and `setTimeout("x")` are not reported, as in ESLint at its defaults. The engine's
`es_global` will probably include browser and Node globals, so there it will report them, as ESLint does with
those environments. Which environment a policy means is an engine question. See "Library and core changes".
Shadowing: a name with no declaration in the file is `free_ref` (rule: names.rr's `ref` with no `ref_site`). In a
script, a top-level `var eval` is the global variable itself (`script_global`).

**Recording artefacts, by rule.**
- no-alert:
  - FP 016–018 (`globalThis.alert()`): run at an ecmaVersion before 2020, which has no globalThis.
  - FN 035, 036 (`this.alert(foo)`): run as scripts. With `--options '{"sourceType":"script"}'` both are found.
- no-eval:
  - FP 013, 014, 052: globalThis cases at an old ecmaVersion.
  - FN 042–081: script cases (`this.eval` at the top or in sloppy functions) and Node or browser cases
    (`global.eval`). Against ESLint 9.39 run with sourceType script, globals window and global, and the library
    given those two as `es_global`, the score is 54 / 0 / 0.
- no-implied-eval:
  - FP 025, 026 (`globalThis.setTimeout('foo')`): an old ecmaVersion.
  - FN 112–146: browser or Node environment cases. With `es_global` given window, setTimeout, setInterval and
    execScript, against ESLint with the same globals: 48 / 0 / 2. The two misses are 104
    `const s = 'x=1'; setTimeout(s, 100);` and 105 `setTimeout(String('x=1'), 100)`. The rule asks
    `static_value(a, v), value_type(v, "string")` for these. The engine's static_value follows file-local constants,
    so it should find 104; rrlab's static_value is `value`, so it finds neither. ESLint's getStaticValue also
    evaluates `String(…)` calls, which static_value does not, so 105 is a real limit.

**no-eval follows ESLint 10, not 9.39, on `Array.fromAsync(xs, f, thisArg)`.** ESLint 10's isDefaultThisBinding
reads fromAsync as binding f's this, and the recording lists
`Array.fromAsync(values, async function (value) { return this.eval(await value); }, context);` (037) as valid.
ESLint 9.39 reports it in a script. This rule follows the pinned tool and the JSON.

**no-console adds a fix.** The JSON has none. ESLint has a suggestion that deletes a statement that is only the
console call, unless the tokens around it would join (maybeAsiHazard). It is added here as a non-safe fix, as
ESLint marks it. The object may be parenthesised: `(console).log(x)` is reported, as in ESLint.

**capitalized-comments: 056, 057, 078, 079** (`//über`, `//π`, `// ꮳꮃꭹ`, `/* 𐳡𐳡𐳡 */`), once four FN. "Is this
letter lower case" (ESLint: `c !== c.toLocaleUpperCase()`) is now the regex class `[\p{Ll}\p{Lt}]`, which rrlab's
Rust regex and the engine's DFA both read, so all four are reported: 51/0/0. (A letter outside Ll and Lt with an
upper-case mapping, such as U+0345, is missed.) The fix still writes `upper(x)`, from common's `upper_first` table,
ASCII letters only: `//über` gets no fix until core has `@upper` (below; then `upper(x, @upper(x))`).
Also:
- ignoreInlineComments and ignoreConsecutiveComments ask ESLint's token store with includeComments. Here that is
  core's tokens plus the comment just before or after (`prev_comment`: a comment with no token between).
- The JSON's per-type options are kept as named: `line.ignorePattern` and `block.ignorePattern`.

**no-irregular-whitespace.**
- One `match_all` over each physical line (`src_line`) with a capture `(?<W>[…]+|[\x{2028}\x{2029}])`: one row
  per run, so each line is read once (no quadratic walk), and each finding sits on its run, at the run's column,
  as ESLint's does. A run inside a node the options skip is dropped (columns from `col` and `end_col`).
- Runs under rrlab since it models `match_all` and capture spans: 186 TP / 5 FP / 5 FN, the same at (line, column).
  The 5 and 5 are the U+2028/U+2029 line numbering below (181, 211–214). (Before that, a match-based variant gave
  176 / 2 / 15 by line.)
- Two runs on one line are now two findings at one key (line), which the findings contract merges; the recording
  counts each report, so a line-level score still loses a TP per extra run (133, 182, 183, 185, 190–192, 200, 201).
- U+2028 and U+2029 are line breaks to ESLint, but not to the engine's `line` or `src_line`. So on any line after
  one, the line numbers differ. Counterexample 181 is `var a = 'b', c = 'd',\ne = 'f' `: ESLint reports
  lines 2 and 3, and this rule reports line 2, the engine's line 2, for both. Also 211–214. Keys follow the engine's
  line numbers, as suppression comments do.
- The 24 characters are a generated table (`eslint-no-irregular-whitespace.chars.rr`, from the JSON's two character
  classes), written by `gen-eslint-no-irregular-whitespace.py`. The regex must hold its group name literally, so the
  rule writes the classes out with `\x{…}` escapes; `check-eslint-no-irregular-whitespace.py` checks the table
  against the JSON and the rule's regex against the table.
- A BOM that starts the file is not reported (ESLint never sees it). A run that only begins with one is reported;
  ESLint's columns on that line are one less.
- The JSON leaves JSX text out of `skipJSXText`'s scope and has separate regex clauses for whitespace before the
  first token. This rule follows ESLint: one test over every line, with the five skip options.

**id-length: FN 069** (`var 葛󠄀 = 2`). Length is counted in code points with `@slice(x, k - 1, k) = ""`; ESLint
counts graphemes. 葛󠄀 is one grapheme and two code points. Byte length (`strlen`) would miss all 30 non-ASCII
fixtures. Also:
- isImportAttributeKey is followed: keys of `import(m, { with: { type: "json" } })` are not names.
- Private names report as `Identifier name '#p' is too short (< 2).` and `Identifier name #'p' is too long (> n).`,
  ESLint 9.39's tooShortPrivate and tooLongPrivate texts (the quote placement of the second is ESLint's). Exceptions
  and exceptionPatterns test the name without its `#`, as ESLint tests PrivateIdentifier.name:
  `{ exceptionPatterns: ["^q$"] }` allows `#q`.
- `for (const x of xs)` and `for (let k in o)` are checked (the head's variable is a declarator's id in ESTree);
  `for (x of xs)` is not. `abstract class B {}` is checked. TypeScript parameter properties
  (`constructor(private c, readonly d, override g)`) are not: ESTree wraps them in TSParameterProperty, which
  id-length does not look into. All checked against ESLint 9.39 with @typescript-eslint/parser.

**no-duplicate-imports** follows ESLint where the JSON has only messages, including:
- isImportExportCanBeMerged: a namespace import and named imports do not merge; `export *` merges only with
  `export *` or a bare import; `import type D` and `import type { N }` do not merge;
- allowSeparateTypeImports;
- `import {} from "m"` counts as a bare import.
The JSON's message texts are kept, and the earlier import's line is added.

### Library and core change requests

1. **core: `@upper(s): symbol`**, the counterpart of `@lower` (JavaScript's toUpperCase). capitalized-comments
   needs "is a lower-case letter" and the fix's replacement; until then it upper-cases ASCII letters through
   common's `upper_first` table, and its `upper(x, u)` becomes `upper(x, @upper(x))`.
   ```
   // @upper("ß") = "SS": JavaScript's String.prototype.toUpperCase (Unicode default case mapping).
   .functor upper(s: symbol): symbol
   ```
2. **core: `@graphemes(s): number`**, the number of extended grapheme clusters (Intl.Segmenter, as ESLint's
   getGraphemeCount). id-length uses @slice, which counts code points.
   ```
   // @graphemes("葛󠄀") = 1: user-perceived characters (Unicode extended grapheme clusters).
   .functor graphemes(s: symbol): number
   ```
3. **Environment for es_global.** names.rr says es_global is "generated from ESLint's and TypeScript's lib
   tables". For ESLint parity, the browser and Node sets (window, setTimeout, global) should be a policy setting,
   not always on. ESLint's defaults declare only the ECMAScript built-ins. Proposal: `es_global` holds the ES
   built-ins only, and `env_global(x)` holds an environment's names, set by `option("globals", "browser,node")`.
   Until then, `eslint-a-globals.rr`'s `es_builtin` duplicates the ES part.
4. **rrlab (not mine, noted for its owner).**
   - `has(a, n, "kind")` filters a by the kind, as core says. Easy to misread as "a has a descendant of kind";
     this cost a debug round. No change needed.
   - `\x{…}` regex escapes are not mapped onto std::regex. (Since done: rrlab uses Rust regex.)
   - `@convert` is ASCII only.
   - `es_global`, `directive_global` and `match_all` have no rows.
   - A regex capture used as a span (`match_all("(?<W>…)", sp)`, then `col(W, c)`) is not modelled, and neither is
     a capture in a regex built with `cat`: no-irregular-whitespace cannot run under rrlab at all. (Since done:
     rrlab runs `match_all` with captures and capture spans; no-irregular-whitespace scores 186 / 5 / 5.)
5. **names.rr: `fn_scope` lacks `generator_function`** (a `function*` expression). Its parameters and vars are given
   to the enclosing function. `eslint-a-globals.rr`'s `any_scope` adds it locally.
6. **names.rr / values.rr:** `const_str` covers strings and plain templates. getStaticPropertyName also takes
   number keys (`o[0]`) and `${}`-free templates; no rule here needed numbers.

### Library interfaces

`lib/tsx/eslint-a-globals.rr` (186 lines; includes tsx/eslint-a-common):
- `es_builtin(x)`, `global_var(d, x)`: the ECMAScript globals; a document's global variables.
- `free_ref(r, x)`: an unshadowed name.
- `global_ref_to(r, x)`: a reference to the global variable x.
- `callee(c, f)`, `null_or_undefined(a)`; parentheses, arguments and static member names are common's
  (`deparen`, `estree_parent`, `call_arg`, `call_arg_count`, `static_prop`).
- `global_object_ref(o, x)`: window / global / globalThis and same-name chains (`window_name(x)`).
- `default_this(f)` (ESLint's isDefaultThisBinding, in full: constructors, @this, methods,
  member assignment, bind/call/apply, thisArg methods, Array.from, Reflect.apply, IIFE returns), and
  `global_this(t)`, over common's `this_owner(t, s)`.
- `in_global_scope(n)`.

`lib/tsx/eslint-a-text.rr` (92 lines; core only):
- `js_comment(c, "Line" | "Block", v)`.
- `end_col(n, c)`: the byte column past a node's end.
- `code_before(c)` / `code_after(c)` and `pre_text` / `post_text`: line text around a comment.
- `tool_comment(c)`: COMMENTS_IGNORE_PATTERN. `eslint_directive(c)`: isDirectiveComment.
- `token_neighbours(c)`: isInlineComment with comments. `follows_comment(c)`: isConsecutiveComment.
- `prev_comment(c, p)`: the latest earlier comment with the same next token, by visit_index (one max per comment).
