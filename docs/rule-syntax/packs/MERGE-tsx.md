# Merge: tsx libraries (JavaScript, TypeScript, React, Node, Next, Vue)

This pass consolidates lib/tsx and lib/vue after the hand conversion of the javascript, typescript, react, node,
next and vue packs. Each concept is now defined once and the rules use that definition. Every change request
against these libraries was either applied or answered below. Core, native, check_rr and rrlab requests are in
scratchpad/batches/CORE-QUEUE-tsx.md.

The scores below come from `tools/rrlab/rrlab score`. Every rule was scored before and after the pass, with the
same rrlab (08d977aa5).

## 1. Scope models

There are four scope models:
- **names.rr** (`decl_site`, `look`, `ref_site`) resolves names lexically. Function scopes and block scopes form
  one chain, and a function's body block is a scope of its own.
- **ts-b-scope.rr** (`scope_node`, `variable`, `resolves`) is typescript-eslint's scope manager. It has scope types
  and definition kinds, and a function's parameters and body are one scope. It adds class-field-initializer scopes,
  enum, namespace and type scopes, the function-expression-name scope, `signature-param` and `enum-member`. A
  parameter default does not see the body, and a parameter decorator sits outside the function.
- **eslint-b-scope.rr** (`var_site`, `escope_of`) groups names.rr's declarations the way eslint-scope does for
  no-redeclare. The function body joins the function, and a static block is a var scope.
- **statements.rr** (`var_scope`, `decl_scope`) follows ESLint's one-var stacks. A namespace body opens no scope
  there.

**Decision.** The four are not merged into one model; they share their vocabulary instead.

They answer different questions. ref_site is the resolution that about 200 rules (and sym_of across files) use, in
eslint-scope's terms. ts-b-scope reproduces the scope manager's own tree, so that no-shadow, no-redeclare,
no-use-before-define and no-unused-vars can compare scope types and definition kinds. These two trees really
differ:
- In `function f(a) { let a; }`, names.rr has two scopes, while the scope manager has one scope and two
  definitions.
- In `var v; function f(p = v) { let v; }`, a default reads the outer v in the scope manager. names.rr would need
  the body to stop being a scope of its own.

Rebuilding names.rr on the scope manager's tree would change ref_site for every rule and break eslint-scope parity
for ESLint's core rules. Rebuilding ts-b-scope on names.rr would lose the nine rules' semantics, which the ts-2
reviewer verified against ESLint.

**What is shared now:**
- names.rr's `fn_scope` is the one list of var-scope kinds. statements.rr's `var_scope(k) :- fn_scope(k)`, and
  statements.rr now includes names.rr. ts-b-scope's `var_home(k) :- fn_scope(k)` adds the namespace kinds, as the
  scope manager does. eslint-a-globals' `any_scope` is gone; it was names.rr's `lex_scope`.
- names.rr took the semantic fixes the others had worked around:
  - `class_static_block` is a var scope;
  - a class expression's name is bound in the class, heritage included (`block_scope("class")`);
  - `undefined` is a reference and a parameter name;
  - the T of `N.T` is no type reference;
  - component JSX tags are references (`jsx_tag_ref`).

  With these, eslint-b-scope's `expr_name` is gone, and so are the dead `look` workarounds:
  consistent-type-imports' `shadowed` and `qualified_member`, explicit-module-boundary-types' `local_too`, and
  react-hooks' `deps_site`.
- ts-b-scope keeps building on names.rr's `pattern_name` and `bound_name`, and on statements.rr's `var_decl` and
  `for_head`.

`first_decl` and `declaring_stmt` (ts-b-scope) do not duplicate statements.rr's `decl_stmt`. declaring_stmt maps a
declared identifier to its statement; decl_stmt maps a declaration to its `declare` wrapper.

## 2. Triage

| Concept | Copies | Survivor | Owner | Rules changed |
|---|---|---|---|---|
| Call callee, parentheses skipped | eslint-a-globals `callee`; eslint-b-scope `called` (no tagged template); node.rr `node_call` (same as called) | `callee(c, f)` keeps tagged templates, for no-eval and no-alert. `called(c, f) :- callee(c, f), !tagged(c)` is ESTree's CallExpression | eslint-a-common | 12 node rules (node_call → called) |
| Tagged template | values.rr `tagged(c)`; no-script-url local `tagged(s)` and vue no-ref-as-operand local `tagged(t)`, both on the template | `tagged(c)` (the call) and `tagged_template(t)` (its template) | eslint-a-common | eslint-no-script-url, vue no-ref-as-operand |
| Parentheses | eslint-a-common `deparen`; modules.rr `unparen` (strings only) | `deparen`, moved to modules.rr so that modules can use it; unparen is gone | modules | none |
| Pass-through (isPassThrough) | eslint-b-scope `pass_through`; node-trace's extra `val` clauses (C7); values' paren and assignment `val` clauses | `pass_through(n, p)`, which values' `val` follows | values | none |
| Aliased loader | node-trace `loader(f) :- val(f, "require", "api")` | the same clause | values | none |
| Global the file writes (isModifiedGlobal) | eslint-b-scope `global_written`; node-trace `trace_written`; eslint-b-objects `written_global` (the only one with parentheses) | `env_written(d, g)`, parentheses skipped; `env_global` excludes it | values | none |
| String constant (getStringIfConstant) | values `const_str` (literals); n-prefer-process-get-builtin-module `const_string`, `const_part` and `tpl_upto` (folds) | `const_str` with the folds: `+`, `${const}`, `true ? a : b`, parentheses | values | n-prefer-process-get-builtin-module |
| String import names | node-trace `trace_spec`; names.rr's `text` of a string name, quotes included | names.rr `spec_name(n, x)` in import_binding, local_of, export_from and export_name; values' api_use | names | none |
| Configured Node range | node.rr `node_runs_on`/`node_version_opt`; the exemplar's `runs_on`/`version_opt` | `node_runs_on` | values | n-no-unsupported-features-node-builtins |
| Var-scope kinds | names `fn_scope`; statements `var_scope`; ts-b-scope `var_home`; eslint-a-globals `any_scope` | `fn_scope` (`lex_scope` for any_scope) | names | none |
| Function or class expression name | eslint-b-scope `expr_name`; names.rr (body only) | names.rr `decl_site`, scoped to the class | names | none |
| TS parameter default as a constructor | no-invalid-this `ts_bound` clause 1 | `bound_context` clause | eslint-a-globals | typescript-eslint-no-invalid-this |
| `this: T` is no use of this | (missing) | `this_use` / `this_annotation` in `this_owner` | eslint-a-common | none |

The TypeScript rows are in section 2a and the React rows in section 2b. Both were written by sub-passes over
ts-a, ts-b and react*/jsx.

### 2a. ts-a / ts-b

| Concept | Copies | Survivor | Owner | Rules changed |
|---|---|---|---|---|
| Parameter property | ts-a `param_property`; no-unused-vars `marked`; unified-signatures `prop_param`; no-unnecessary-parameter-property-assignment's modifier test | `param_property` | ts-a | those three |
| Heritage | ts-a `heritage`; ts-b `heritage_ref` | `heritage_ref(r) :- type_ref(r, _), heritage(r).` (ts-b now includes ts-a) | ts-a/ts-b | none |
| Type parentheses | ts-a `type_child`; prefer-as-const `type_deparen` | `type_child` | ts-a | prefer-as-const |
| Keyword type | ts-a `ts_keyword`; unified-signatures' `void` test | `ts_keyword(t, "void")` | ts-a | unified-signatures |
| Enum initialiser | ts-b `enum_member`/`enum_init`; no-duplicate-enum-values `enum_member_init` | ts-b's | ts-b | no-duplicate-enum-values |
| Inside a module declaration | ts-b `ts_module_decl`; method-signature-style `no_fix` | `within(m, g), ts_module_decl(g)` | ts-b | method-signature-style |
| Member head after decorators | explicit-member-accessibility `head`; ts-a-returns' two `return_head` clauses | `member_head(m, h)` | ts-a | explicit-member-accessibility, ts-a-returns |
| Dead look workarounds | consistent-type-imports `shadowed`, `qualified_member`; explicit-module-boundary-types `local_too` | removed | names | those two |
| Clash from the new include | vitest-expect-expect `assertion` | renamed `assert_call` | (rule) | vitest-expect-expect |

Kept apart:
- `ts_ctor` and `ctor`. `ts_ctor` includes overloads; `ctor` is ESLint core's.
- `return_head` and `fn_head`. They are typescript-eslint's and ESLint core's getFunctionHeadLoc, and share only
  `member_head`.
- member-ordering's `report_at`. It reports at the decorators, so it is not a head.
- `in_type` against `type_head`/`type_ref`. The first asks whether a node is anywhere in a type; the others are
  per-identifier space choices.
- parameter-properties' `modifiers`. It needs the modifier text, and it covers fields as well.

### 2b. React

| concept | copies | survivor | owner file | rules changed |
|---|---|---|---|---|
| resolve a name or a JSX tag to its declaration | react-hooks `deps_site` (+ `deps_candidate`, max-depth), react-compiler `rc_tag_site` (+ `rc_tag_decl`) | names.rr `ref_site` (tags are refs via `jsx_tag_ref`; `look` walks one chain) | tsx/names.rr | exhaustive-deps, static-components |
| TS tag reads in deps | react-hooks `deps_read` clause 2 (tags `[A-Z]`, incl. closing tags) | `deps_read` clause 1 over names' `ref` (jsx_tag_ref: not `^[a-z]`, as typescript-eslint) | react-hooks.rr | (exhaustive-deps, no score change) |
| call argument i | react.rr `create_arg`, react-hooks `deps_arg` | eslint-a-common `call_arg` (react.rr now includes eslint-a-common) | tsx/eslint-a-common.rr | exhaustive-deps, void-dom-elements-no-children, no-unstable-nested-components |
| function kinds (6: decl, expr, arrow, both generators, method) | react-hooks `hook_fn_kind` / `hook_fn` | modules `function_kind` / eslint-a-common `function_node` | tsx/modules.rr, eslint-a-common.rr | exhaustive-deps |
| function kinds (3, no generators/methods) | jsx.rr `fn_kind` (only in `returned`) | `function_kind` — deliberate fix: a `return` in a method or generator nested in a component was attributed to the component | tsx/modules.rr | none (no score change) |
| what a function returns | react-class `fn_returns` (own return/arrow-body clauses) | jsx.rr `returned`, `fn_returns(f, x) :- returned(f, e), deparen(e, x).` (also skips a comment after `return`) | tsx/jsx.rr | none |
| parameter i | react-compiler `rc_param` | eslint-a-functions `fn_param` (react-compiler includes it) | tsx/eslint-a-functions.rr | hooks-immutability |
| parentheses and `as` | react-hooks `deps_bare` (own paren walk, child 0) | kept (drops `as` too) but rebuilt on `deparen` (comment inside parens now skipped) | react-hooks.rr | none |

Kept apart:

- `rc_fn_kind` = function_kind without methods (Babel's Function); a derived subset, not a copy.
- `rc_state_setter` (compiler: second pattern element) vs exhaustive-deps' `deps_second` (exactly two elements, holes counted).
- `fn_component` / `ret_jsx` (Components.detect) vs jsx.rr `component` / `render_root` (capitalised declarations by symbol, for cross-file render).
- `rc_name` vs `hook_fn_name` (compiler's vs plugin's getFunctionName, as NOTES said).
- `scope_holder` / `same_scope` (react-class) vs eslint-b-scope `escope_of`: same eslint-scope model, different shapes (a node set for `nearest` vs a block→scope map; scope_holder also has for/catch/switch/class body/static block/field initialiser). Now merged: react-class's same_scope reads eslint-a-common's new `escope_node`, which also fixes the switch discriminant, which scope_holder put inside the switch scope.
- jsx-no-undef keeps its own tag lookup: it follows ESLint quirks (both namespaces, enums/namespaces, the module's first child scope), so `ref_site` would change it.

## 3. Change requests

| Request | Source | Outcome |
|---|---|---|
| Bind a function or class expression's own name | js-2 1, react | Done before this pass for functions. The class name is now scoped to the class, heritage included. expr_name is gone |
| `for (var k in o)` declared; generator_function in fn_scope | js-2 3, js-1 5 | Already done (`for_binding`) |
| class_static_block in fn_scope | js-2 3 | Done |
| `undefined` as a reference and as a parameter | js-2 (A, D), react | Done (`ref` clause, `param_id`) |
| `/* global x:off */` | js-1, js-2 2 | Native fact. Queued (core 1); eslint-b-scope's `global_off` parser stays until then |
| directive_global rows from global_item | js-2 5 | Declined. names.rr builds on core, and the native reader is the source. The `off` case is queued |
| `self` among the global objects | js-2 6 | Declined for eslint-a-globals. no-eval and no-implied-eval use ESLint's own three names (global, window, globalThis). ReferenceTracker's four are eslint-b-scope's `global_object_name`, which already has self |
| `called` / `callee` merge | js-2 dup | Done: `called(c, f) :- callee(c, f), !tagged(c)` |
| `lit_val` beside `static_string` | js-2 dup | Kept apart: `1` and `"1"` differ in isSameReference |
| `expr_name_at` (position form) | js-2 5 | Not needed. names.rr binds the names, and eslint-b-objects' `named_expr_around` (asked at a position with no reference) stays |
| deparen / unparen | js-1 | Done (deparen in modules.rr) |
| stmt_start / semi_needed into a library | js-1 | Deferred: eslint-a-common has no second user yet |
| const_str numbers | js-1 6 | Declined: no rule needs them. static_string has numbers. const_str now folds as getStaticValue does |
| es_global environments, @upper, @graphemes, @js_compare, decoded names | js-1, js-2 | Queued (core 6–10) |
| JSX tags as references | react | Done: `jsx_tag_ref`, for capitalised tags on opening and self-closing elements |
| `look` starts at the nearer scope | react hooks, ts-1 5 | Already done (lex_scope chain). The workarounds were removed |
| `undefined` header note | react | Done (comment at the `ref` clause) |
| this_owner skips `this: T` | ts-1 1 | Done |
| default_this: TS default, JSDoc climb, capIsConstructor | ts-1 2 | The TS default is done. The JSDoc climb stays in no-invalid-this (`ts_bound`): it uses within_stop and eslint-a-text, which eslint-a-globals does not include. capIsConstructor is declined: no rule asks for it |
| `N.T` is no reference | ts-1 3 | Done (`qualified_name`) |
| return_head / fn_head | ts-1 7 | See 2a |
| next: es_global("window") | next | Waits on core 6 |
| Node C1–C3 | node | Queued (core 2–4) |
| Node C4 | node | Already done |
| Node C5: comment globals without node_global | node | Done |
| Node C6 | node | check_rr, queued (core 12) |
| Node C7: pass-through, aliased loader, string import names, isModifiedGlobal | node | Done (see the triage) |
| cjs_ref against env_global | node dup | Kept apart. env_global is bounded by the policy's node_global table; cjs_ref is the CommonJS view (no-exports-assign needs `exports = …` itself) |
| node_builtin against node_module | node dup | Kept apart. They are different tables: builtinModules, and the trace map's keys |
| gitignore matcher (pkg_listed, npm_ignored) | node dup | Queued as one native `@ignore_match` (core 3). npm_ignored needs ignore-file text that only native code reads; the gi_* relations then go |
| Generator scripts that read trace maps | node dup | Left; not library code |
| vue | vue | Nothing outstanding |

## 4. Scores

All 307 rules of the six packs were rescored: 259 have a recording and 48 do not. Before is a
fresh baseline: the HEAD libraries and rules (08d977aa5), exported to scratchpad/merge-tsx/head, scored with the
current rrlab. The raw rows are in scratchpad/merge-tsx/before2.tsv and after.tsv. (before.tsv, taken under the
older rrlab, differs only by no-irregular-whitespace, which could not run then.)

256 recorded rules have the same TP/FP/FN as before. Three changed, all deliberate fixes from the rrlab regex
update:

| Rule | Before (TP/FP/FN) | After | Why |
|---|---|---|---|
| javascript/eslint-capitalized-comments | 47/0/4 | 51/0/0 | fix (regex): `[\p{Ll}\p{Lt}]` is lower case, so über, π and non-Latin letters are reported |
| javascript/eslint-func-name-matching | 46/0/1 | 47/1/0 | fix (regex): the identifier test is `\p{ID_Start}`/`\p{ID_Continue}`; the one FP is the known recording artefact 104.js |
| react/react-no-unescaped-entities | 2/0/1 | 3/0/0 | fix (regex): match_all capture spans, so two quotes on one line give two findings, as recorded |

No score moved from a library merge.

**Rules with no recording.** The react rules were run with `rrlab run` over the 1,066 upstream fixtures and the 68
reviewer cases, at HEAD and after the pass (scratchpad/merge-tsx/react-runs/before and final).
- react-hooks-static-components no longer reports the lowercase `<x/>` (B/nc2.tsx:10). ESLint does not report it
  either, so this is a fix.
- react-no-unescaped-entities changed columns (see above).
- react-jsx-no-constructed-context-values read react-class's removed `scope_holder` set. It now reads `escope_node`
  and gives identical findings.

**Changes no recording exercises, checked against eslint-plugin-n 18.4.1** (scratchpad/rev-node/lint.mjs). These
are all fixes, and rrlab now agrees with ESLint on each:
- `require("f" + "s").readFile(a, cb)` and `` require(`${"f"}s`) `` are reported by n/prefer-promises/fs. This
  comes from const_str's folds; before, they were not reported.
- After `require = g`, `require("fs").readFile(a, cb)` is no longer reported. This is env_written; before, it was
  reported.
- After `Buffer = 1`, `globalThis.Buffer.from(x)` is still reported by n/prefer-global/buffer "never".
  ReferenceTracker checks isModifiedGlobal only on the variable it starts from. Before, node-trace filtered it out.

<details><summary>Every rule, before and after</summary>

| Rule | Before | After |
|---|---|---|
| javascript/eslint-accessor-pairs | 118/1/0 | 118/1/0 |
| javascript/eslint-arrow-body-style | 42/0/0 | 42/0/0 |
| javascript/eslint-capitalized-comments | 47/0/4 | 51/0/0 |
| javascript/eslint-class-methods-use-this | 787/0/1 | 787/0/1 |
| javascript/eslint-curly | 239/0/0 | 239/0/0 |
| javascript/eslint-default-case | 8/0/0 | 8/0/0 |
| javascript/eslint-default-case-last | 14/0/0 | 14/0/0 |
| javascript/eslint-default-param-last | 16/0/0 | 16/0/0 |
| javascript/eslint-dot-notation | 189/0/0 | 189/0/0 |
| javascript/eslint-eqeqeq | 39/0/0 | 39/0/0 |
| javascript/eslint-func-name-matching | 46/0/1 | 47/1/0 |
| javascript/eslint-func-style | 25/0/0 | 25/0/0 |
| javascript/eslint-grouped-accessor-pairs | 38/0/0 | 38/0/0 |
| javascript/eslint-guard-for-in | 6/0/0 | 6/0/0 |
| javascript/eslint-id-length | 101/0/1 | 101/0/1 |
| javascript/eslint-max-classes-per-file | 11/0/0 | 11/0/0 |
| javascript/eslint-max-depth | 2/0/0 | 2/0/0 |
| javascript/eslint-max-nested-callbacks | 1/0/0 | 1/0/0 |
| javascript/eslint-max-params | 5/0/0 | 5/0/0 |
| javascript/eslint-max-statements | 2/0/0 | 2/0/0 |
| javascript/eslint-no-alert | 20/3/2 | 20/3/2 |
| javascript/eslint-no-array-constructor | 84/0/0 | 84/0/0 |
| javascript/eslint-no-await-in-loop | 17/0/0 | 17/0/0 |
| javascript/eslint-no-bitwise | 16/0/1 | 16/0/1 |
| javascript/eslint-no-caller | 2/0/0 | 2/0/0 |
| javascript/eslint-no-case-declarations | 15/0/0 | 15/0/0 |
| javascript/eslint-no-compare-neg-zero | 23/0/0 | 23/0/0 |
| javascript/eslint-no-cond-assign | 11/0/0 | 11/0/0 |
| javascript/eslint-no-console | 23/0/0 | 23/0/0 |
| javascript/eslint-no-continue | 4/0/0 | 4/0/0 |
| javascript/eslint-no-debugger | 1/0/0 | 1/0/0 |
| javascript/eslint-no-delete-var | 1/0/0 | 1/0/0 |
| javascript/eslint-no-div-regex | 1/0/0 | 1/0/0 |
| javascript/eslint-no-dupe-class-members | 57/0/0 | 57/0/0 |
| javascript/eslint-no-dupe-else-if | 59/0/0 | 59/0/0 |
| javascript/eslint-no-dupe-keys | 57/0/0 | 57/0/0 |
| javascript/eslint-no-duplicate-case | 19/0/0 | 19/0/0 |
| javascript/eslint-no-duplicate-imports | 19/0/0 | 19/0/0 |
| javascript/eslint-no-empty | 17/0/0 | 17/0/0 |
| javascript/eslint-no-empty-function | 2069/0/0 | 2069/0/0 |
| javascript/eslint-no-empty-pattern | 18/0/0 | 18/0/0 |
| javascript/eslint-no-empty-static-block | 6/0/0 | 6/0/0 |
| javascript/eslint-no-eq-null | 3/0/0 | 3/0/0 |
| javascript/eslint-no-eval | 16/3/20 | 16/3/20 |
| javascript/eslint-no-extend-native | 22/0/0 | 22/0/0 |
| javascript/eslint-no-extra-bind | 35/0/0 | 35/0/0 |
| javascript/eslint-no-extra-label | 19/0/0 | 19/0/0 |
| javascript/eslint-no-implied-eval | 4/2/17 | 4/2/17 |
| javascript/eslint-no-inline-comments | 36/0/0 | 36/0/0 |
| javascript/eslint-no-irregular-whitespace | 186/5/5 | 186/5/5 |
| javascript/eslint-no-iterator | 5/0/0 | 5/0/0 |
| javascript/eslint-no-labels | 27/0/0 | 27/0/0 |
| javascript/eslint-no-lonely-if | 14/0/0 | 14/0/0 |
| javascript/eslint-no-loss-of-precision | 50/0/0 | 50/0/0 |
| javascript/eslint-no-magic-numbers | 1984/0/0 | 1984/0/0 |
| javascript/eslint-no-multi-assign | 22/0/0 | 22/0/0 |
| javascript/eslint-no-multi-str | 5/0/0 | 5/0/0 |
| javascript/eslint-no-negated-condition | 6/0/0 | 6/0/0 |
| javascript/eslint-no-nested-ternary | 2/0/0 | 2/0/0 |
| javascript/eslint-no-new | 1/0/0 | 1/0/0 |
| javascript/eslint-no-new-func | 10/0/0 | 10/0/0 |
| javascript/eslint-no-new-wrappers | 5/0/0 | 5/0/0 |
| javascript/eslint-no-nonoctal-decimal-escape | 53/0/0 | 53/0/0 |
| javascript/eslint-no-obj-calls | 59/27/0 | 59/27/0 |
| javascript/eslint-no-object-constructor | 51/0/0 | 51/0/0 |
| javascript/eslint-no-octal | 11/0/0 | 11/0/0 |
| javascript/eslint-no-octal-escape | 60/0/0 | 60/0/0 |
| javascript/eslint-no-plusplus | 25/0/0 | 25/0/0 |
| javascript/eslint-no-proto | 4/0/0 | 4/0/0 |
| javascript/eslint-no-prototype-builtins | 19/0/0 | 19/0/0 |
| javascript/eslint-no-redeclare | 48/6/21 | 48/6/21 |
| javascript/eslint-no-return-assign | 11/0/0 | 11/0/0 |
| javascript/eslint-no-script-url | 4/0/0 | 4/0/0 |
| javascript/eslint-no-self-assign | 43/0/0 | 43/0/0 |
| javascript/eslint-no-self-compare | 15/0/0 | 15/0/0 |
| javascript/eslint-no-sequences | 12/0/0 | 12/0/0 |
| javascript/eslint-no-setter-return | 70/0/0 | 70/0/0 |
| javascript/eslint-no-sparse-arrays | 8/0/0 | 8/0/0 |
| javascript/eslint-no-template-curly-in-string | 7/0/0 | 7/0/0 |
| javascript/eslint-no-ternary | 3/0/0 | 3/0/0 |
| javascript/eslint-no-unexpected-multiline | 18/0/0 | 18/0/0 |
| javascript/eslint-no-unneeded-ternary | 13/0/0 | 13/0/0 |
| javascript/eslint-no-unsafe-finally | 17/0/0 | 17/0/0 |
| javascript/eslint-no-unsafe-negation | 6/0/0 | 6/0/0 |
| javascript/eslint-no-unsafe-optional-chaining | 66/0/0 | 66/0/0 |
| javascript/eslint-no-unused-expressions | 1411/0/7 | 1411/0/7 |
| javascript/eslint-no-unused-labels | 38/0/0 | 38/0/0 |
| javascript/eslint-no-unused-private-class-members | 42/0/0 | 42/0/0 |
| javascript/eslint-no-useless-call | 25/0/0 | 25/0/0 |
| javascript/eslint-no-useless-catch | 5/0/0 | 5/0/0 |
| javascript/eslint-no-useless-computed-key | 65/0/0 | 65/0/0 |
| javascript/eslint-no-useless-concat | 10/0/0 | 10/0/0 |
| javascript/eslint-no-useless-constructor | 64/0/0 | 64/0/0 |
| javascript/eslint-no-useless-rename | 130/0/0 | 130/0/0 |
| javascript/eslint-no-var | 77/0/0 | 77/0/0 |
| javascript/eslint-no-void | 3/0/0 | 3/0/0 |
| javascript/eslint-no-warning-comments | 8/0/0 | 8/0/0 |
| javascript/eslint-no-with | 12/0/0 | 12/0/0 |
| javascript/eslint-one-var | 222/0/0 | 222/0/0 |
| javascript/eslint-operator-assignment | 37/0/0 | 37/0/0 |
| javascript/eslint-prefer-destructuring | 42/0/0 | 42/0/0 |
| javascript/eslint-prefer-exponentiation-operator | 169/3/2 | 169/3/2 |
| javascript/eslint-prefer-numeric-literals | 62/0/0 | 62/0/0 |
| javascript/eslint-prefer-object-has-own | 33/0/0 | 33/0/0 |
| javascript/eslint-prefer-spread | 18/0/0 | 18/0/0 |
| javascript/eslint-prefer-template | 79/0/0 | 79/0/0 |
| javascript/eslint-radix | 36/0/0 | 36/0/0 |
| javascript/eslint-require-await | 31/0/0 | 31/0/0 |
| javascript/eslint-require-yield | 6/0/0 | 6/0/0 |
| javascript/eslint-sort-imports | 43/0/0 | 43/0/0 |
| javascript/eslint-sort-keys | 98/0/0 | 98/0/0 |
| javascript/eslint-sort-vars | 26/0/0 | 26/0/0 |
| javascript/eslint-strict | 55/23/26 | 55/23/26 |
| javascript/eslint-symbol-description | 2/0/0 | 2/0/0 |
| javascript/eslint-use-isnan | 73/0/0 | 73/0/0 |
| javascript/eslint-valid-typeof | 18/0/0 | 18/0/0 |
| javascript/eslint-vars-on-top | 27/0/0 | 27/0/0 |
| javascript/eslint-yoda | 103/0/0 | 103/0/0 |
| javascript/unicorn-filename-case | 116/0/0 | 116/0/0 |
| javascript/unicorn-no-null | 43/0/0 | 43/0/0 |
| javascript/unicorn-prefer-node-protocol | 24/0/0 | 24/0/0 |
| next/next-google-font-display | 4/0/0 | 4/0/0 |
| next/next-google-font-preconnect | 2/0/0 | 2/0/0 |
| next/next-inline-script-id | 4/0/0 | 4/0/0 |
| next/next-next-script-for-ga | 4/0/0 | 4/0/0 |
| next/next-no-assign-module-variable | 1/0/0 | 1/0/0 |
| next/next-no-async-client-component | 5/0/0 | 5/0/0 |
| next/next-no-before-interactive-script-outside-document | 6/0/0 | 6/0/0 |
| next/next-no-css-tags | 2/0/0 | 2/0/0 |
| next/next-no-document-import-in-page | 2/0/0 | 2/0/0 |
| next/next-no-duplicate-head | 3/0/0 | 3/0/0 |
| next/next-no-head-element | 1/0/0 | 1/0/0 |
| next/next-no-head-import-in-document | 4/0/0 | 4/0/0 |
| next/next-no-img-element | 6/0/0 | 6/0/0 |
| next/next-no-location-assign-relative-destination | 29/0/0 | 29/0/0 |
| next/next-no-page-custom-font | 3/0/0 | 3/0/0 |
| next/next-no-script-component-in-head | 1/0/0 | 1/0/0 |
| next/next-no-styled-jsx-in-document | 1/0/0 | 1/0/0 |
| next/next-no-sync-scripts | 3/0/0 | 3/0/0 |
| next/next-no-title-in-document-head | 1/0/0 | 1/0/0 |
| next/next-no-typos | 5/0/0 | 5/0/0 |
| next/next-no-unwanted-polyfillio | 4/0/0 | 4/0/0 |
| node/n-callback-return | 24/0/0 | 24/0/0 |
| node/n-exports-style | 25/0/0 | 25/0/0 |
| node/n-global-require | 8/0/0 | 8/0/0 |
| node/n-handle-callback-err | 17/0/0 | 17/0/0 |
| node/n-hashbang | 37/13/1 | 37/13/1 |
| node/n-no-callback-literal | 10/0/0 | 10/0/0 |
| node/n-no-deprecated-api | 119/0/1 | 119/0/1 |
| node/n-no-exports-assign | 1/0/0 | 1/0/0 |
| node/n-no-extraneous-require | no recording | no recording |
| node/n-no-missing-import | no recording | no recording |
| node/n-no-missing-require | no recording | no recording |
| node/n-no-mixed-requires | 4/0/0 | 4/0/0 |
| node/n-no-new-require | 2/0/0 | 2/0/0 |
| node/n-no-path-concat | 20/0/0 | 20/0/0 |
| node/n-no-process-env | 14/0/0 | 14/0/0 |
| node/n-no-process-exit | 3/0/0 | 3/0/0 |
| node/n-no-sync | 21/0/0 | 21/0/0 |
| node/n-no-top-level-await | no recording | no recording |
| node/n-no-unpublished-bin | no recording | no recording |
| node/n-no-unpublished-import | no recording | no recording |
| node/n-no-unpublished-require | no recording | no recording |
| node/n-no-unsupported-features-es-builtins | no recording | no recording |
| node/n-no-unsupported-features-node-builtins | no recording | no recording |
| node/n-prefer-global-buffer | 12/0/0 | 12/0/0 |
| node/n-prefer-global-console | 12/0/0 | 12/0/0 |
| node/n-prefer-global-crypto | no recording | no recording |
| node/n-prefer-global-process | 10/0/0 | 10/0/0 |
| node/n-prefer-global-text-decoder | 12/0/0 | 12/0/0 |
| node/n-prefer-global-text-encoder | 12/0/0 | 12/0/0 |
| node/n-prefer-global-timers | no recording | no recording |
| node/n-prefer-global-url | 12/0/0 | 12/0/0 |
| node/n-prefer-global-url-search-params | 12/0/0 | 12/0/0 |
| node/n-prefer-import-assert-strict | no recording | no recording |
| node/n-prefer-node-protocol | 36/0/0 | 36/0/0 |
| node/n-prefer-process-get-builtin-module | no recording | no recording |
| node/n-prefer-promises-dns | 27/0/0 | 27/0/0 |
| node/n-prefer-promises-fs | 40/0/0 | 40/0/0 |
| react/react-button-has-type | 21/0/0 | 21/0/0 |
| react/react-hooks-error-boundaries | no recording | no recording |
| react/react-hooks-exhaustive-deps | 226/0/4 | 226/0/4 |
| react/react-hooks-globals | no recording | no recording |
| react/react-hooks-immutability | no recording | no recording |
| react/react-hooks-purity | no recording | no recording |
| react/react-hooks-refs | no recording | no recording |
| react/react-hooks-rules-of-hooks | 104/0/0 | 104/0/0 |
| react/react-hooks-set-state-in-effect | no recording | no recording |
| react/react-hooks-set-state-in-render | no recording | no recording |
| react/react-hooks-static-components | no recording | no recording |
| react/react-iframe-missing-sandbox | no recording | no recording |
| react/react-jsx-key | 26/0/0 | 26/0/0 |
| react/react-jsx-no-comment-textnodes | 7/0/0 | 7/0/0 |
| react/react-jsx-no-constructed-context-values | no recording | no recording |
| react/react-jsx-no-duplicate-props | 3/0/0 | 3/0/0 |
| react/react-jsx-no-leaked-render | no recording | no recording |
| react/react-jsx-no-script-url | no recording | no recording |
| react/react-jsx-no-target-blank | 38/0/0 | 38/0/0 |
| react/react-jsx-no-undef | 7/0/0 | 7/0/0 |
| react/react-no-access-state-in-setstate | no recording | no recording |
| react/react-no-array-index-key | 41/0/0 | 41/0/0 |
| react/react-no-children-prop | 23/0/0 | 23/0/0 |
| react/react-no-danger | 2/0/0 | 2/0/0 |
| react/react-no-danger-with-children | 14/0/0 | 14/0/0 |
| react/react-no-deprecated | no recording | no recording |
| react/react-no-did-mount-set-state | no recording | no recording |
| react/react-no-did-update-set-state | no recording | no recording |
| react/react-no-direct-mutation-state | 15/0/0 | 15/0/0 |
| react/react-no-find-dom-node | 4/0/0 | 4/0/0 |
| react/react-no-is-mounted | 3/0/0 | 3/0/0 |
| react/react-no-object-type-as-default-prop | no recording | no recording |
| react/react-no-render-return-value | 7/0/0 | 7/0/0 |
| react/react-no-string-refs | 3/0/0 | 3/0/0 |
| react/react-no-unescaped-entities | 2/0/1 | 3/0/0 |
| react/react-no-unknown-property | no recording | no recording |
| react/react-no-unsafe | no recording | no recording |
| react/react-no-unstable-nested-components | no recording | no recording |
| react/react-self-closing-comp | 16/0/0 | 16/0/0 |
| react/react-void-dom-elements-no-children | 10/0/0 | 10/0/0 |
| typescript/typescript-eslint-adjacent-overload-signatures | 36/0/0 | 36/0/0 |
| typescript/typescript-eslint-array-type | no recording | no recording |
| typescript/typescript-eslint-ban-ts-comment | 7/0/0 | 7/0/0 |
| typescript/typescript-eslint-ban-tslint-comment | 8/0/0 | 8/0/0 |
| typescript/typescript-eslint-class-literal-property-style | 11/0/0 | 11/0/0 |
| typescript/typescript-eslint-class-methods-use-this | 1/0/0 | 1/0/0 |
| typescript/typescript-eslint-consistent-generic-constructors | 19/0/1 | 19/0/1 |
| typescript/typescript-eslint-consistent-indexed-object-style | no recording | no recording |
| typescript/typescript-eslint-consistent-type-assertions | no recording | no recording |
| typescript/typescript-eslint-consistent-type-definitions | 5/0/0 | 5/0/0 |
| typescript/typescript-eslint-consistent-type-imports | 11/0/0 | 11/0/0 |
| typescript/typescript-eslint-default-param-last | 54/0/0 | 54/0/0 |
| typescript/typescript-eslint-explicit-function-return-type | 9/0/0 | 9/0/0 |
| typescript/typescript-eslint-explicit-member-accessibility | 14/0/0 | 14/0/0 |
| typescript/typescript-eslint-explicit-module-boundary-types | no recording | no recording |
| typescript/typescript-eslint-max-params | 5/0/0 | 5/0/0 |
| typescript/typescript-eslint-member-ordering | 18/0/0 | 18/0/0 |
| typescript/typescript-eslint-method-signature-style | 43/0/0 | 43/0/0 |
| typescript/typescript-eslint-no-array-constructor | 13/0/0 | 13/0/0 |
| typescript/typescript-eslint-no-confusing-non-null-assertion | 11/0/0 | 11/0/0 |
| typescript/typescript-eslint-no-dupe-class-members | 11/0/0 | 11/0/0 |
| typescript/typescript-eslint-no-duplicate-enum-values | 16/0/0 | 16/0/0 |
| typescript/typescript-eslint-no-dynamic-delete | 10/0/0 | 10/0/0 |
| typescript/typescript-eslint-no-empty-function | 7/0/0 | 7/0/0 |
| typescript/typescript-eslint-no-empty-object-type | no recording | no recording |
| typescript/typescript-eslint-no-explicit-any | no recording | no recording |
| typescript/typescript-eslint-no-extra-non-null-assertion | 8/0/0 | 8/0/0 |
| typescript/typescript-eslint-no-extraneous-class | 12/0/0 | 12/0/0 |
| typescript/typescript-eslint-no-import-type-side-effects | 4/0/0 | 4/0/0 |
| typescript/typescript-eslint-no-inferrable-types | 40/0/0 | 40/0/0 |
| typescript/typescript-eslint-no-invalid-this | 71/0/0 | 71/0/0 |
| typescript/typescript-eslint-no-invalid-void-type | 35/0/0 | 35/0/0 |
| typescript/typescript-eslint-no-magic-numbers | no recording | no recording |
| typescript/typescript-eslint-no-misused-new | 7/0/0 | 7/0/0 |
| typescript/typescript-eslint-no-namespace | 6/0/0 | 6/0/0 |
| typescript/typescript-eslint-no-non-null-asserted-nullish-coalescing | no recording | no recording |
| typescript/typescript-eslint-no-non-null-asserted-optional-chain | 10/0/0 | 10/0/0 |
| typescript/typescript-eslint-no-non-null-assertion | 22/0/5 | 22/0/5 |
| typescript/typescript-eslint-no-redeclare | 13/0/0 | 13/0/0 |
| typescript/typescript-eslint-no-require-imports | 11/0/0 | 11/0/0 |
| typescript/typescript-eslint-no-shadow | no recording | no recording |
| typescript/typescript-eslint-no-this-alias | 5/0/0 | 5/0/0 |
| typescript/typescript-eslint-no-unnecessary-parameter-property-assignment | no recording | no recording |
| typescript/typescript-eslint-no-unnecessary-type-constraint | 23/0/0 | 23/0/0 |
| typescript/typescript-eslint-no-unsafe-declaration-merging | no recording | no recording |
| typescript/typescript-eslint-no-unsafe-function-type | 5/0/0 | 5/0/0 |
| typescript/typescript-eslint-no-unused-expressions | 21/0/0 | 21/0/0 |
| typescript/typescript-eslint-no-unused-private-class-members | no recording | no recording |
| typescript/typescript-eslint-no-unused-vars | 78/0/2 | 78/0/2 |
| typescript/typescript-eslint-no-use-before-define | no recording | no recording |
| typescript/typescript-eslint-no-useless-constructor | 9/0/0 | 9/0/0 |
| typescript/typescript-eslint-no-useless-empty-export | 9/0/0 | 9/0/0 |
| typescript/typescript-eslint-no-wrapper-object-types | 16/0/0 | 16/0/0 |
| typescript/typescript-eslint-parameter-properties | 32/0/0 | 32/0/0 |
| typescript/typescript-eslint-prefer-as-const | 15/0/0 | 15/0/0 |
| typescript/typescript-eslint-prefer-enum-initializers | 5/0/0 | 5/0/0 |
| typescript/typescript-eslint-prefer-for-of | no recording | no recording |
| typescript/typescript-eslint-prefer-function-type | 18/0/0 | 18/0/0 |
| typescript/typescript-eslint-prefer-literal-enum-member | 16/0/0 | 16/0/0 |
| typescript/typescript-eslint-prefer-namespace-keyword | 4/0/0 | 4/0/0 |
| typescript/typescript-eslint-triple-slash-reference | no recording | no recording |
| typescript/typescript-eslint-unified-signatures | 53/0/0 | 53/0/0 |
| typescript/vitest-expect-expect | 24/0/0 | 24/0/0 |
| typescript/vitest-no-disabled-tests | 23/0/0 | 23/0/0 |
| typescript/vitest-no-focused-tests | 28/0/0 | 28/0/0 |
| vue/eslint-plugin-vue-attribute-hyphenation | 16/0/0 | 16/0/0 |
| vue/eslint-plugin-vue-no-async-in-computed-properties | 96/0/0 | 96/0/0 |
| vue/eslint-plugin-vue-no-child-content | 12/0/0 | 12/0/0 |
| vue/eslint-plugin-vue-no-deprecated-v-on-native-modifier | 2/0/0 | 2/0/0 |
| vue/eslint-plugin-vue-no-dupe-keys | 35/0/0 | 35/0/0 |
| vue/eslint-plugin-vue-no-duplicate-attributes | 2/0/0 | 2/0/0 |
| vue/eslint-plugin-vue-no-export-in-script-setup | 15/0/0 | 15/0/0 |
| vue/eslint-plugin-vue-no-mutating-props | 64/0/0 | 64/0/0 |
| vue/eslint-plugin-vue-no-ref-as-operand | 43/0/0 | 43/0/0 |
| vue/eslint-plugin-vue-no-setup-props-reactivity-loss | 45/0/0 | 45/0/0 |
| vue/eslint-plugin-vue-no-side-effects-in-computed-properties | 50/0/0 | 50/0/0 |
| vue/eslint-plugin-vue-no-template-shadow | 19/0/0 | 19/0/0 |
| vue/eslint-plugin-vue-no-unused-refs | 7/0/0 | 7/0/0 |
| vue/eslint-plugin-vue-no-unused-vars | 39/0/0 | 39/0/0 |
| vue/eslint-plugin-vue-no-use-v-if-with-v-for | 10/0/0 | 10/0/0 |
| vue/eslint-plugin-vue-no-v-html | 3/0/0 | 3/0/0 |
| vue/eslint-plugin-vue-no-v-text-v-html-on-component | 11/0/0 | 11/0/0 |
| vue/eslint-plugin-vue-no-watch-after-await | 5/0/0 | 5/0/0 |
| vue/eslint-plugin-vue-require-v-for-key | 9/0/0 | 9/0/0 |
| vue/eslint-plugin-vue-require-valid-default-prop | 68/0/0 | 68/0/0 |
| vue/eslint-plugin-vue-return-in-computed-property | 23/0/0 | 23/0/0 |
| vue/eslint-plugin-vue-v-on-event-hyphenation | 9/0/0 | 9/0/0 |
| vue/eslint-plugin-vue-valid-v-for | 23/0/0 | 23/0/0 |

</details>

## 5. To delete

No file became unused. The relations that were removed are listed in the triage; their files remain.

## 6. Line counts

These counts are for lib/tsx, lib/vue and the rules of the six packs, against HEAD.

| Part | Lines |
|---|---|
| Libraries | +3 |
| Rules | −58 |
| **Total** | **−55** |

- **Libraries.** values.rr grew by 46 lines: it took pass_through, the folds, env_written and node_runs_on.
  eslint-a-common grew by 29 lines: callee, called, tagged, escope_node and this_use. names.rr grew by 28 lines. The
  copies that went shrank eslint-b-scope by 28, node-trace by 24, react-hooks by 16, react-class by 13, node by 13
  and react-compiler by 11.
- **Rules.** The local copies went.

The NOTES changes from the regex follow-up are not counted.
