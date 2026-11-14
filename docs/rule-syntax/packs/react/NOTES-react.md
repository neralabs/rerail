# Notes: batch react (eslint-plugin-react 7.37.5, eslint-plugin-react-hooks 7.1.1)

The pack's 40 rules, written by hand as .rr in four groups. Each group has its own library over one base:

| Library | Lines | Group |
|---|---|---|
| `lib/tsx/react.rr` | 216 | all: the jsxRuntime gate (`react_doc`), the pragma, JSX open tags and props, createElement, class components |
| `lib/tsx/react-hooks.rr` | 395 | rules-of-hooks, exhaustive-deps |
| `lib/tsx/react-compiler.rr` | 264 | the eight React Compiler rules |
| `lib/tsx/react-class.rr` | 216 | class components and older APIs (13 rules) |

The libraries combine with each other and with jsx, names, modules, values and eslint-a-common: check_rr shows no
line naming them.

**How it was scored.** Recorded rules were scored with `rrlab score react <id> --all`. Unrecorded rules were run
under ESLint 9.39.3 with the plugins at the recorded versions and no settings, as `upstream/common.mjs` runs
them, over every fixture of both recordings and over ad-hoc cases. Matching is on (path, line).

**Rules the scorer cannot place.** `rrlab score` stops with "cannot tell which upstream tool" for rules that
conformance.json does not list. The pack has two tools, so these rules were compared by hand.

Totals: 40 rules, 1932 lines of rules (most 20–85), plus a 272-line generated table. The four libraries add
1,091 lines.

Open after the review: immutability reports `A/ed1.jsx:50` and `A/ed3.jsx:27` (an effect reading `ref.current`
into a const; `a.b.c = 1` in an effect), which ESLint does not. Not investigated; likely older than the review
fixes.

## JSX and createElement rules (17)

Library: `lib/tsx/react.rr` (base for the whole batch): the `file.jsxRuntime` gate (`react_doc`), the pragma, JSX
open tags and props, createElement calls and their props, class components. Scores are `rrlab score react <id>
--all` (eslint-plugin-react 7.37.5 recording, 637 fixtures); rules the recording does not run were compared with
ESLint 9.39 + eslint-plugin-react 7.37.5 (no settings, as the generator) on every fixture of both recordings and on
ad-hoc cases, matched on (path, line).

| Rule | Lines | TP / FP / FN | Deviations |
|---|---|---|---|
| button-has-type | 64 | 22 / 0 / 0 | none |
| iframe-missing-sandbox | 53 | not recorded; 0/0 on all fixtures, 9 ad-hoc agree | none |
| jsx-key | 60 | 28 / 0 / 0 (+1 explained: Stencil file) | none |
| jsx-no-comment-textnodes | 21 | 7 / 0 / 0 | none |
| jsx-no-duplicate-props | 20 | 3 / 0 / 0 | none |
| jsx-no-leaked-render | 50 | not recorded; 1/1 on all fixtures, ad-hoc agree | none |
| jsx-no-script-url | 23 | not recorded; ad-hoc agree | JSX strings read raw (1) |
| jsx-no-target-blank | 76 | 38 / 0 / 0 | no fix (2) |
| jsx-no-undef | 41 | 189 / 0 / 0 | own tag lookup (3) |
| no-array-index-key | 75 | 42 / 0 / 0 | none |
| no-children-prop | 24 | 34 / 0 / 0 | none |
| no-danger-with-children | 48 | 14 / 0 / 0 | none |
| no-danger | 19 | 7 / 0 / 0 | none |
| no-unescaped-entities | 33 | 2 / 0 / 1 | (4) |
| no-unknown-property | 54 + 272 table | not recorded; 3/3 on all fixtures, 14 ad-hoc agree | tool's fix on both renames (5) |
| self-closing-comp | 28 | 114 / 0 / 0 | tool's fix added (6) |
| void-dom-elements-no-children | 39 | 10 / 0 / 0 | none |

A later review's cases agree with ESLint line for line: jsx-key (array keys must be exactly `key`, iterator keys any
case; a comment after `return`), jsx-no-undef (`this`, TypeScript declarations, self-names, loop bindings),
jsx-no-leaked-render (TypeScript `f?.()`, a template `` is not safe), no-array-index-key (`{ key }` in
cloneElement, `i?.toString()` not checked), self-closing-comp and no-unknown-property.

1. jsx-no-script-url: a JSX attribute string has no escapes, so `href="\x01javascript:"` is not a script URL.
   ESLint agrees. core's `value`/`str` unescape it as JS, which is why react.rr has `jsx_string`. HTML entities
   (`&#106;avascript:`) are decoded by JSX and not by `jsx_string`, so they are a known miss.
2. jsx-no-target-blank, jsx-no-leaked-render: ESLint's fixes are not offered, as in the JSON.
3. jsx-no-undef: names.rr's `ref` skips every `name` field, so JSX tags have no `ref_site`. The rule looks the tag
   up in enclosing scopes with `decl_site`, in both namespaces, plus TypeScript's enums, namespaces and type
   parameters, as typescript-eslint's scope manager declares them. ESLint's quirk is followed too: the module scope's first child scope
   counts, so `function m() { var V } <V/>` is reported unless m is the first function.
4. no-unescaped-entities: each quote is a `match_all` capture over the text node, a span with its own line and
   column, so two quotes on one line are two findings, as ESLint reports them (3 TP / 0 FP / 0 FN; before
   match_all, `invalid-007.jsx:2` with two quotes was one finding and one FN). The JSON checked only `'` and `"`; the rule's default list also has `>` and `}`. Both are syntax errors
   in JSX text for espree and typescript-eslint (checked), so only quotes can be reported. The rule does not
   report them either. Each quote is placed on its own line with `src_line`'s span, walking the text one quote at
   a time with captures. (Superseded: see the start of this item.)
5. no-unknown-property follows the tool's algorithm: normalise the any-case names, skip data-*/aria-*, apply the
   tag map, look up the HTML/SVG attribute name, then the case-insensitive DOM property. The tables are
   `react-no-unknown-property.props.rr`, generated from the JSON by `gen-react-no-unknown-property.py`.
   `check-react-no-unknown-property.py` rebuilds every name set of the JSON from them, runs the JSON's own
   `transform` steps on each HTML/SVG name to check the standard-name column, and prints `ok`. The JSON
   fixes only the HTML/SVG rename; ESLint also fixes the wrong-case rename, and so does this rule.
6. self-closing-comp: the JSON has no fix; ESLint's (`>…</X>` becomes ` />`) is offered as a plain `fix`.

### Library and core change requests (JSX part)

- Done in this batch: `lib/tsx/names.rr` binds a named function or class expression's own name inside it, and
  `for (const x of …)` / `for (var k in …)` bindings; `generator_function` joined fn_scope so a generator
  expression's parameters and name have a scope. `lib/tsx/jsx.rr`'s `returned` skips a comment after `return`.
  Rescored after the names.rr change: javascript no-redeclare, no-setter-return, no-var, id-length,
  prefer-numeric-literals, no-prototype-builtins, use-isnan, no-array-constructor unchanged; accessor-pairs one
  FP fewer. (radix fails on `to_number("inf")` with or without the change.)
- `lib/tsx/names.rr`: `ref` excludes identifiers in any `name` field, which drops JSX tag names
  (`jsx_opening_element.name`, `jsx_self_closing_element.name`). Proposed: also add
  `ref(r, x, "value") :- kind(r, "identifier"), text(r, x), field(o, "name", r), (kind(o, "jsx_opening_element") ; kind(o, "jsx_self_closing_element")).`
  jsx-no-undef and react-compiler.rr could then use `ref_site`. Done at the merge (names.rr `jsx_tag_ref`, for tags
  not starting lower-case): react-compiler's `rc_tag_site` and react-hooks' `deps_site` are gone. jsx-no-undef keeps
  its own lookup: it follows ESLint's quirks (both namespaces, enums and namespaces, the first child scope).
- rrlab: `match_all` is not modelled, and neither is `(?s)` in match (the functor's regex dialect). Write
  `[\s\S]` instead. Done in rrlab 08d977aa5 (Rust regex, match_all, capture spans); no-unescaped-entities,
  jsx-no-comment-textnodes, iframe-missing-sandbox and jsx-no-target-blank now use them.
- rrlab `run` uses the files' common directory as `--root` and reads file and package facts below it. Files under
  both /private/tmp and /Users give `/`, and the run walks the whole disk (2 GB RSS before it was killed).
  Proposed: refuse a root of `/`, or read only the files given and their package.json chain.
- No `tsconfig` fact carries `compilerOptions.jsxImportSource`, so the `react_doc` gate reads only the comment,
  the factory import and package.json dependencies. Rerail's own gate also reads tsconfig. Proposed core input:
  `.decl tsconfig_option(dir: symbol, key: symbol, v: symbol)`.

## Hooks: rules-of-hooks and exhaustive-deps

Library: `lib/tsx/react-hooks.rr` (395 lines), over `tsx/react.rr`. Scores are `rrlab score react <id> --all`
(eslint-plugin-react-hooks 7.1.1 recording, 429 fixtures), matched on path, line and code; the column-level scores
are the same. Ad-hoc cases were diffed against ESLint 9.39 with the plugin at 7.1.1 (scratch `cases-hooks/`), all
equal by line except the unreachable-code quirk below.

| Rule | Lines | TP / FP / FN | Deviations |
|---|---|---|---|
| react-hooks-rules-of-hooks | 76 | 106 / 0 / 0 (1 explained FN: the Vue-package fixture, divergences.json) | unreachable code after earlier branches (below) |
| react-hooks-exhaustive-deps | 133 | 227 / 0 / 4 | the 4 FNs are duplicate recorded findings (below) |

**rules-of-hooks.** The plugin counts code-path segments; the rule reads the same facts off the tree, which is
exact for structured code: branches of if, ?:, && || ?? and their assignments, switch cases, catch clauses, the
arguments of an optional call, default values, the finally of a try without catch (only when the try block can
throw), and what follows the first throwable node of a try block with a catch (`try { f(); useX(); } catch {}`).
Earlier `return`s and `break label`s make later calls conditional; a `throw` does not. Loops: for test, update and
body, while/do, for-in/of body (not the right side). Unreachable calls (after an unconditional return, throw, break or
continue in the same block) are not checked, as valid-008 wants. Deviation: ESLint does report some unreachable calls
when a branch comes earlier in the function, e.g. `function useC() { if (a) {} return; useW(); }` (ESLint reports
useW, the rule does not); this is an artefact of the plugin's segment counting on unreachable segments, not
modelled. The useEffectEvent checks follow the plugin, except its `lastEffect` quirk (an inner effect call's exit
clears the outer one) is read as "inside any effect call's arguments". Messages are the JSON's.
Accepted gap: code after an infinite loop is unreachable to the plugin (`for (;;) {}  useState();` is not
reported; `while (true) { if (a) return; }  useState();` is). The rule does not know a loop never ends: it misses
the second and reports the first.

**exhaustive-deps.** One finding per Hook call at its array for missing, unnecessary and duplicate keys (the
plugin's collectRecommendations over key paths, as prefix relations), plus the plugin's other reports at their own
nodes. The 4 FNs (invalid-031:7, invalid-111:3 and :6, invalid-172:3) are lines where the recording has two
findings (two Hooks list the same built-anew value, or not-array plus missing at one array); a finding key is
(policy, code, path, line), so the second copy can never match. Notes:
- JSX tag names are references only in .ts/.tsx files: typescript-eslint's scope manager makes `<Comp/>` read
  Comp (invalid-191), espree makes no JSX references (checked: ESLint reports nothing for the same code in .jsx).
- `typeof x` in a type is not a read (valid-116, valid-118).
- A stale-assignment finding is placed at the first assignment in the plugin's scope order (outer scope first,
  invalid-086), approximated as (scope's visit index, position).
- `useInsertionEffect` is not checked by exhaustive-deps (not in getReactiveHookCallbackIndex), but is an effect
  for rules-of-hooks' useEffectEvent check and the compiler. One list, `effect_hook`; exhaustive-deps' `deps_effect`
  is the effect Hooks it has a callback index for.

### Library and core changes (hooks)
- `lib/tsx/names.rr`: `look` starts at both the nearest block scope and the nearest function scope, so a reference
  inside an expression-bodied arrow resolves to the arrow's parameter *and* to an outer declaration of the same name
  (`setCount(count => increment(count))` with an outer `count`). react-hooks.rr works around it with `deps_site`
  (the candidate in the deepest scope). Proposed fix: start only at the nearer of the two:
  `look(r, x, sp, s) :- ref(r, x, sp), nearest(r, s, scope_any).` with `scope_any` the union of fn_scope and
  block_scope kinds.
- `lib/tsx/names.rr`: tsx `undefined` is its own node kind (`undefined`), not an identifier; rules testing
  `text(n, "undefined")` on identifiers miss it. Worth a line in the header.
- No core changes.

### Duplicates to merge (hooks)
- `deps_bare` (as-casts and parentheses dropped) is close to eslint-a-common's `deparen` plus `as`; kept separate
  because the plugin also drops `as`.
- `deps_arg` / `react.rr`'s `create_arg` / eslint-a-common's `call_arg`: the same argument numbering.
- `hook_fn_kind` and jsx.rr's `fn_kind`/modules.rr's `function_kind`: function kinds (this one includes methods
  and generators, as eslint-scope's function scopes).

## React Compiler rules (eslint-plugin-react-hooks 7.1.1)

Eight rules the plugin runs through the React Compiler (compilationMode "infer"). Nothing in the recording covers
them, and `rrlab score react <id> --all` stops with "cannot tell which upstream tool" (the pack has two tools and
conformance.json does not list these ids). So each was scored two ways against ESLint 9.39 with the plugin at
7.1.1 (scratch harness): ad-hoc cases written from the compiler's behaviour and the JSON's clauses, and every
fixture file of both recordings (1,066 files; mostly rules-of-hooks and exhaustive-deps cases, which exercise the
compiler too), matched on (path, line).

| Rule | Lines | Ad-hoc cases (files, all agree) | Every fixture TP/FP/FN | Deviations |
|---|---|---|---|---|
| react-hooks-error-boundaries | 40 | 1 file, 13 functions | 0/0/0 | none seen |
| react-hooks-globals | 26 | 2 files | 1/0/0 | none seen |
| react-hooks-immutability | 120 | 4 files | 15/0/0 | frozen values followed one alias step (not `\|\|`/`??`, not a `...rest` copy); mutating calls only on useMemo results |
| react-hooks-purity | 30 | 3 files (1 FN) | 0/0/0 | aliases of Date.now not followed (FN `const now = Date.now; now()`) |
| react-hooks-refs | 59 | 3 files (1 FN) | 5/0/0 | refs through props objects and return values not followed; a write to a ref parameter's own member (`ref.x = 1`) is not reported |
| react-hooks-set-state-in-effect | 65 | 2 files | 8/0/0 | a setter whose argument reads a ref (directly or through a const in the effect) is allowed, as the tool does |
| react-hooks-set-state-in-render | 30 | 2 files | 0/0/0 | none seen |
| react-hooks-static-components | 34 | 2 files | 0/0/0 | none seen |

Shared vocabulary: `lib/tsx/react-compiler.rr` (264 lines, over `tsx/react-hooks.rr`, whose Hook names,
callees, function kinds, memo/forwardRef wrappers, `deps_name` and `effect_hook` it reuses): which functions the compiler compiles
(`rc_compiled`, `rc_component`: getComponentOrHookLike, isValidComponentParams, returnsNonNode, the outermost
candidate, classes skipped, the plugin's mayContainReactCode file test, and the two bail-outs the JSON names),
and which code runs during render (`rc_render_fn`: the function and its useMemo callbacks; `rc_live_fn`: a positive
fixpoint: a function written in place in live code (not as a JSX prop, a hook argument or a useMemo callback), or a
named one with a reference, outside those slots, in live code; so one called only from an effect, a handler or a
useCallback body is not live; `rc_deferred`: handed on to run later; `rc_called_fn`; `rc_always`: no
branch, loop, try or earlier return around a node), plus `rc_hook_call`, `rc_react_call`, `rc_set_state`,
`rc_state_setter`, `rc_ref_site`, `rc_current` and `rc_tag_site`.

### What the compiler does that these rules do not

The compiler validates its own IR: inferred types, aliasing and mutation effects, and its knowledge of the
signatures of built-ins and hooks. The rules are the syntactic core of each validation. Concretely:
- **Aliasing.** A value is followed through one `const`: `const { a } = props`, `const t = state.items`, `const b =
  a`. The compiler follows any data flow (function returns, array elements, `let` reassignment). Likewise a ref is
  `const r = useRef()` or a name `ref`/`…Ref`; purity's impure functions are only called by name
  (`Date.now()`), not through `const now = Date.now`. immutability does not follow `const style = props.style ||
  {}` or `?? {}` (the tool reports `style.x = 1`), and does not freeze a rest copy (`const { a, ...rest } = props;
  rest.x = 1` is fine, as in the tool).
- **Mutating calls.** The compiler knows which methods mutate (its type signatures). immutability reports a
  method call only for an Array, Map or Set mutator on a useMemo result (`list.push(x)`, `m.set(k, v)`), which is
  what the tool reported on the cases; `props.items.push(x)` is not reported by the tool either.
- **Errors per function.** The compiler stops a function at some errors. Modelled where cases showed it: the
  first setter call per effect (set-state-in-effect), the first late reassignment per closure (immutability).
  Elsewhere every occurrence is reported; the keys are lines, so most coincide.
- **Bail-outs.** Only the two the JSON names: a `try` without `catch`, and a destructured `catch` parameter.
  Other "Todo" constructs of the compiler (it does not lower every syntax) would make it report nothing.
- **JSON clauses left out.** immutability's `deps-frozen-mutation` and `reassign-in-memo-result` (no case made
  the tool report them separately from the clauses kept); refs' clauses for refs passed whole (`foo(ref)`), refs
  in object literals and lost loads (`ref-passed`, `literal-passed…`, `ref-beside-lost-load`). Every case the
  tool reported is covered; these clauses are where the compiler's aliasing decides.
- **Followed the tool against the JSON:** purity ignores `new Date()` (7.1.1's impure list is Date.now,
  Math.random and performance.now); refs does not treat `createRef()` as a ref (the tool reports nothing for
  `createRef().current` in render); a ref read in a useMemo/useCallback dependency list is not reported (the
  compiler drops manual memo dependencies: fixture exhaustive-deps/invalid-074); immutability's
  `param-reassign` without a closure is not reported (`function C({ x }) { x = 2; }`: the tool is silent).
- **jsxRuntime.** Gated with react.rr's `react_doc`.

### Library and core change requests

- `lib/tsx/names.rr`: `ref` excludes every `name` field, so a JSX tag (`<Row/>`, the opening element's `name`) is
  never a reference and has no `ref_site`. react-compiler.rr resolves tags itself (`rc_tag_site`). Proposed: in
  `ref`, allow `field(o, "name", r)` when `o` is a `jsx_opening_element` or `jsx_self_closing_element` and r starts
  with a capital (React's component rule); then `rc_tag_site` goes.
- check_rr did not report a body atom of an undeclared relation (`tagged`, declared in values.rr, used without
  including it); rrlab found it only by giving no rows.

### Duplicates to merge

- Merged with react-hooks.rr: `rc_fn_kind` is `hook_fn_kind` without methods, and scopes are `hook_fn`;
  `rc_hook_ref`/`rc_hook_call` are react-hooks' callee less bare `use`; `rc_react_call` is a call of `deps_name`;
  memo/forwardRef is `hook_wrapped`; `rc_effect_call` uses `effect_hook`. Both still overlap modules.rr's
  `function_kind` and jsx.rr's `fn_kind` (no generators).
- `rc_param` duplicates eslint-a-functions.rr's `fn_param` (not included, to keep the include set small).
- `rc_name` (the compiler's getFunctionName, through parentheses) and react-hooks' `hook_fn_name` (the plugin's,
  with object methods) differ on purpose; `rc_state_setter` and exhaustive-deps' stable setters (`deps_second`)
  read the same pattern and could share one relation.

## Class components and older APIs (13 rules)

Library: `lib/tsx/react-class.rr` (216 lines), over `tsx/react.rr` and `tsx/eslint-a-common.rr`. Class
components come from tsx/react (`class_component`, `react_component`, `comp_member`, TypeScript included). The
library adds the component a node belongs to (`comp_owner`, and `parent_component` with function components),
member keys as ESTree's `key.name` (`method_key`, `estree_name`, `member_named`), the kinds of fresh values
(`fresh_kind`, shared by no-object-type-as-default-prop and jsx-no-constructed-context-values), `this.x` reads and
calls, `set_state_in` (makeNoMethodSetStateRule), `same_scope` (ESLint's getScope), and
function-component detection (`fn_component`, `detected`, `ret_jsx`, `ret_jsx_null`, `cap_name`) after
Components.detect's getStatelessComponent.

Ground truth: ESLint 9.39.3 with eslint-plugin-react 7.37.5 and **no settings**, as the recording's generator
(`upstream/common.mjs`) runs it. The plugin then assumes React 999.999.999. With `settings.react.version: "19.0"`
no-did-mount-set-state and no-did-update-set-state become no-ops (their `shouldBeNoop`), so the harness's
`lint.js` takes `REACT_SETTINGS=none`. "Recorded" is `rrlab score react <id> --all`; "fixtures" is the plugin run
by hand on all 1,068 files of both react recordings (`bulk.sh`), since most of these rules are not recorded;
"ad hoc" is the plugin run on the case files in `cases-class/`, written to cover each branch of the plugin's code.

| Rule | Lines | Recorded (TP/FP/FN) | Fixtures (ESLint/rr/both) | Ad hoc | Deviations |
|---|---|---|---|---|---|
| react-no-access-state-in-setstate | 84 | not recorded | 0/0/0 | 20 cases, agree | none known |
| react-no-did-mount-set-state | 22 | not recorded | 0/0/0 | 16 cases ×2 files, agree | none known |
| react-no-did-update-set-state | 22 | not recorded | 0/0/0 | agree | none known |
| react-no-direct-mutation-state | 53 | 15/0/0 (columns too) | 15/15/15 | 19 cases + review's 23, agree | function components approximated (below) |
| react-no-is-mounted | 23 | 3/0/0 | 3/3/3 | — | none |
| react-no-string-refs | 21 | 3/0/0 | 3/3/3 | — | none |
| react-no-unsafe | 47 | not recorded | 0/0/0 | 14 cases ×2, agree | none known |
| react-no-deprecated | 116 | not recorded | 32/32/32 | 21 cases ×3 files + review's 3, agree | none known |
| react-no-find-dom-node | 19 | 4/0/0 | 4/4/4 | — | none |
| react-no-render-return-value | 26 | 7/0/0 | 7/7/7 | review's 10, agree | none |
| react-no-object-type-as-default-prop | 42 | not recorded | 0/0/0 | 19 cases ×2 + review's 27, agree | component detection approximated |
| react-no-unstable-nested-components | 83 | not recorded | 1/1/1 | 31 cases ×2, agree | component detection approximated |
| react-jsx-no-constructed-context-values | 53 | not recorded | 0/0/0 | 27 cases ×2, agree | none known |

Messages, codes and suppressions are the JSON's. Where the JSON's message has a capture, the rule fills it from
the plugin's own data: no-deprecated's `{old}` and `{new}` come from the plugin's table (getDeprecated) and the
two entries with no replacement (`React.isValidClass`, `React.addons.LinkedStateMixin`) get a message without one.

### What follows the plugin rather than the obvious reading

- **setState in componentDidMount / componentDidUpdate:** any Property, MethodDefinition or PropertyDefinition
  with the key, in a component or not (`const o = { componentDidMount() { this.setState() } }` is reported); every
  enclosing key is tried, not only the nearest; arrows count as functions for the depth (the plugin's regex
  `/Function(Expression|Declaration)$/` matches ArrowFunctionExpression); `#componentDidMount` and
  `[componentDidMount]` count, `"componentDidMount"` does not; `this[setState]()` counts.
- **no-direct-mutation-state:** the constructor exemption follows the plugin's `inCallExpression` flag, which any
  call's exit clears. It is on when the write's nearest enclosing call holds no call that ended before it:
  `constructor() { foo(bar(), () => { this.state.a = 1; }); }` is **not** reported (bar() ended before the
  assignment), nor is `fetch(u).then(() => { this.state.a = 1; })` (the inner `fetch(u)` starts at the same offset
  and ends first), while `foo(() => { this.state.a = 1; })` is. The report sits at the assignment's
  `left.object` (`this.state.a.b = 1` → `this.state.a`). Parentheses are transparent (`(this.state).a = 1`).
- **no-access-state-in-setstate:** the plugin walks the file once, so a variable or a method counts only after its
  this.state read; a method is matched only by a bare call (`getNext()`), never `this.getNext()`; a variable only in
  the same ESLint scope (a use inside an `if { }` block is a different scope, not reported). Each is mirrored with
  `earlier` and `same_scope`. `this.setState(() => ({ v: this.state.v }))` is reported, as in the plugin.
- **no-unsafe:** the first member with the name is reported (two `UNSAFE_componentWillMount`s give two findings at
  the first); a decorated member is reported at its first decorator.
- **no-deprecated:** lifecycle members are matched by their key's ESTree name (`member_named`):
  `#componentWillMount() {}`, `[componentWillUpdate]() {}` and a spec's shorthand `{ componentWillUpdate }` are
  reported, `"componentWillUpdate"() {}` is not. Member expressions are matched by their source text, so `React . render` is not reported;
  `<React.DOM.div/>` (a JSXMemberExpression) and `typeof React.PropTypes` (a TSQualifiedName) are not; `@jsx Foo`
  moves the `React.*` entries to `Foo.*` but imports still name `React.x`.
- **no-render-return-value:** `ReactDOM?.render(…)` and `ReactDOM.render?.(…)` are not reported: ESTree wraps
  them in a ChainExpression, which is not one of the parents the plugin checks.
- **no-object-type-as-default-prop:** both `{ a = [] }` and `{ a: b = [] }` defaults count, parentheses aside
  (`{ a = ({}) }`); a first parameter with its own default (`({ a = [] } = {})`) is an AssignmentPattern and is
  not checked; `<></>` is not reported, `<Fragment></Fragment>` is.
- **jsx-no-constructed-context-values:** a variable counts only when declared in the provider's own scope
  (`callScope.set`); `<Ctx>` is a provider when `Ctx` is declared `= createContext(…)` or `React.createContext(…)`;
  the finding sits at the constructed value, which may be on the variable's line.
- **The JSDoc `@extends React.Component` form** of isES6Component is not modelled: ESLint 9 removed
  `sourceCode.getJSDocComment`, the plugin's try/catch swallows the error, and no such class is detected.

### Approximations (no counterexample found, but not exact)

- **Function components** (`fn_component`): getStatelessComponent's main cases are modelled (declarations, a
  capitalised variable or assignment, export default, a capitalised property or object method, memo/forwardRef
  wrappers, a function returned or an arrow's body, a member assignment). Left out: the factory cases
  (`X = () => () => <div/>`, a function returned from a capitalised property, `{ [a.b]: … }`) and async generators
  (banned). `isReturningJSX` follows a `const el = <div/>` binding one step, as the plugin's `isJSX(variable)` does,
  but not other variable shapes. These matter to no-object-type-as-default-prop, no-unstable-nested-components,
  jsx-no-constructed-context-values (is the provider inside a component) and no-direct-mutation-state (a `this.state`
  write in a function component).
- `cap_name` (isFirstLetterCapitalized) and the unstable-nested parent-name test are ASCII: a name starting with a
  non-ASCII lower-case letter counts as capitalised.

### Library and core change requests

1. **Done:** tsx/react.rr's `class_component` now also reads TypeScript's `extends_clause`
   (`class_heritage > extends_clause > value`); react-class.rr's `cls_component` and `class_or_spec` workarounds
   are gone, replaced by `class_component` and `react_component`.
2. **Soufflé aggregate capture (a note for every author):** a variable used inside `max … : { … }` that also appears
   later in the body is the outer variable, not a fresh one. no-direct-mutation-state first had this bug.

### Duplicates to merge

- `fn_component` / `ret_jsx` overlap with tsx/jsx's `component` and `returned` (theirs: by symbol, for render
  position; mine: the plugin's detection, with `null` returns and positions). Possibly with react-hooks.rr's
  component detection, if the hooks fork wrote one.
- `same_scope` / `scope_holder` may duplicate eslint-b-scope's `escope_of` once that library is committed.

## Merged at integration (see packs/MERGE-tsx.md)

The duplicates listed above are settled: `deps_site` and `rc_tag_site` are names.rr's `ref_site` (JSX tags are
references there); `deps_arg` and `create_arg` are eslint-a-common's `call_arg`; `hook_fn_kind` / `hook_fn` are
modules.rr's `function_kind` / eslint-a-common's `function_node` (the same six kinds); `rc_param` is
eslint-a-functions' `fn_param`; jsx.rr's `fn_kind` is gone (`returned` now stops at any function, methods and
generators included), and react-class's `fn_returns` is built on `returned`. Kept apart on purpose: `rc_fn_kind`
(Babel's Function, no methods), `deps_bare` (also drops `as`), `rc_state_setter` / `deps_second` (the compiler takes
the second element, the plugin wants exactly two), `fn_component` / jsx.rr's `component`, and `scope_holder`.
