# NOTES: batch ts-2 (typescript-eslint and Vitest, syntax only)

There are 32 rules. Each rule is checked against one of two references:

- **rrlab score.** `rrlab score typescript <id> --all` compares the rule with the recording, across every
  fixture: typescript-eslint 8.70.1 (1,588 files), or @vitest/eslint-plugin 1.6.27 (137 files).
- **ad-hoc.** Seven rules have no recording here ("not in conformance.json"). For those, the reference is the
  installed ESLint 9.39 with @typescript-eslint 8.56.1 (in `nera-landing-2/Data-layer-main-up/node_modules`).
  It runs on every fixture of the typescript-eslint recording and on hand-written cases, under several option
  sets. "matches" means the two agree on every reported line:column.

Each rule's revision is the JSON's revision plus 1. Its codes, messages, suppressions and options follow the
JSON, except where noted below. A fix is `fix safe` only where ESLint autofixes. Suggestions are not offered.

The rules use three new libraries:

- `lib/tsx/ts-b.rr` holds declaration files, namespaces and modules, type references and enum members.
- `lib/tsx/ts-b-scope.rr` models typescript-eslint's scope manager: scopes, variables (by declaration kind and
  value/type space), references and their resolution, and reads, writes and init writes.
- `lib/tsx/ts-b-vitest.rr` recognises Vitest calls as the plugin's parseVitestFnCall does: heads, chains, mode
  options and test functions.

| rule | lines | score (TP/FP/FN) | deviations |
|---|---:|---|---|
| no-misused-new | 31 | 9/0/0 | |
| no-namespace | 20 | 55/0/0 | |
| no-non-null-asserted-nullish-coalescing | 36 | ad-hoc, matches (8 cases) | |
| no-non-null-asserted-optional-chain | 25 | 10/0/0 | |
| no-non-null-assertion | 14 | 74/0/7 | FN: several findings at one place (1) |
| no-redeclare | 91 | 31/0/0 | `/* global */` item columns (2) |
| no-require-imports | 29 | 13/0/0 | |
| no-shadow | 124 | ad-hoc, matches on 6 option sets | builtinGlobals and TypeScript's lib names (3) |
| no-this-alias | 27 | 5/0/0 | |
| no-unnecessary-parameter-property-assignment | 62 | ad-hoc, matches (7 cases) | |
| no-unnecessary-type-constraint | 16 | 23/0/0 | |
| no-unsafe-declaration-merging | 20 | ad-hoc, matches (7 cases) | interface with type parameters (4) |
| no-unsafe-function-type | 16 | 7/0/0 | |
| no-unused-expressions | 59 | 79/0/0 | |
| no-unused-private-class-members | 158 | ad-hoc, matches | duplicate constructors (5) |
| no-unused-vars | 213 | 1563/0/2 | FN: `using` declarations (6) |
| no-use-before-define | 81 | ad-hoc, matches on 5 option sets | |
| no-useless-constructor | 67 | 53/0/0 | |
| no-useless-empty-export | 33 | 10/0/0 | |
| no-wrapper-object-types | 23 | 16/0/0 | |
| parameter-properties | 70 | 70/0/0 | |
| prefer-as-const | 37 | 15/0/0 | |
| prefer-enum-initializers | 17 | 16/0/0 | |
| prefer-for-of | 62 | ad-hoc, matches (24 cases) | |
| prefer-function-type | 78 | 23/0/0 | |
| prefer-literal-enum-member | 61 | 28/0/0 | |
| prefer-namespace-keyword | 21 | 9/0/0 | |
| triple-slash-reference | 39 | ad-hoc, matches (no fixtures) | |
| unified-signatures | 204 | 129/0/25 | FN: several findings at one place (1) |
| vitest-expect-expect | 48 | 81/0/0 | |
| vitest-no-disabled-tests | 29 | 31/0/0 | |
| vitest-no-focused-tests | 21 | 37/0/0 | |

Lines counts every non-blank line. The libraries add 147 (ts-b), 311 (ts-b-scope) and 188 (ts-b-vitest).
Every miss in the table is one of the numbered notes. There are no false positives.

## Deviations, explained

1. **Several findings at one place.** The recording counts every report. rrlab keeps one finding per
   place. `x!!!` gives the recording three non-null findings at one node. Three or more overloads in
   unified-signatures give one finding per pair, and two pairs can end at the same parameter. At the level of
   line:column sets, both rules match exactly.
2. **no-redeclare, `/* global */` items.** A redeclared comment item is reported on the comment's line. The
   column falls at the comment's start, not at the item's name.
3. **no-shadow, builtinGlobals.** This option is off by default. With it on, the rule reports names of ES
   built-ins and `/* global */` items. typescript-eslint also counts TypeScript's lib names (`ClassDecorator`
   and the like, three fixtures). No list of those is modelled.
4. **no-unsafe-declaration-merging.** An interface with type parameters is not reported when it merges with a
   class. The installed rule does the same.
5. **no-unused-private-class-members.** A class with two constructors (invalid TypeScript) has two sets of
   parameter properties. Only the later constructor's properties count, as the rule's map keeps them. Both
   fixtures match.
6. **no-unused-vars, `using`.** tree-sitter's TypeScript grammar parses `using x = …` and `await using x = …`
   as expressions, so they declare nothing. These are the two FNs, fixtures 234 and 235.

Other choices, all matching the references:

- unified-signatures follows the recording (8.70) where it differs from the installed 8.56. Type parameter
  constraints compare by text, not by node type. Identical overloads are reported: "… with identical
  parameters." That message is in the JSON. It has no counterpart in 8.56.
- prefer-for-of misses `({ k: a[i] } = o)` and `for (a[i] of b)` as writes. The rule does too.
- no-unused-vars treats a JSX element as a read of `React`, as the parser's default jsxPragma does.
- no-redeclare and no-shadow do not read `/* global x: off */` (`global_off`). That switch is an espree
  feature, and under typescript-eslint's parser it has no effect.
- In a .d.ts file, no-shadow skips a variable only when its own declaration is `declare`d (`declare const`,
  `declare class`, `declare enum`, `declare namespace`). A `const` inside `declare namespace X { … }` and the
  parameters of a `declare class` are still checked, as typescript-eslint does.
- no-non-null-asserted-nullish-coalescing treats an ECMAScript built-in the file does not declare
  (`Symbol! ?? 1`) as unassigned, so it is not reported. typescript-eslint is silent there too.
- ts-b-vitest does not add Jest's `fit` and `fdescribe`. The plugin is not installed, so whether it accepts
  them cannot be checked.
- The vitest rules are inferred from the recording and the plugin's documented behaviour, because the plugin
  is not installed. Context `it` (`test("t", ({ it }) => …)`) skips the chain check. A global `test.extend()`
  is not a head. `expect.to.be.skip` and `expectTypeOf.returns.not.skip` count for no-disabled-tests. These
  all match the Rerail-written fixtures.

## Library and core changes

No existing library or core file was edited. These are requests:

- **Why ts-b-scope and not names.rr.** names.rr now looks names up along one scope chain, so an inner
  declaration shadows an outer one. ts-b-scope stays for another reason: these rules compare scopes and
  declaration kinds, so they need typescript-eslint's scope manager's own tree and its definition kinds. That
  means a class-field-initializer scope, enum and namespace scopes, `signature-param` and `enum-member` kinds,
  parameter defaults that do not see the body, and parameter decorators outside the function. A merge should
  settle on one model.
- **Disjunctions are not the problem.** An earlier draft blamed the lowering for a rule like
  `opt(o, "true") :- (o = "functions" ; o = "classes" ; …), !option(o, _).` That was wrong. lower.py and
  Soufflé derive every branch. The missing findings came from rrlab printing one finding per place.
  no-use-before-define keeps the `default(o, d)` fact table because it reads more plainly.
- **check_rr: a relation called with the wrong meaning.** In no-shadow, a call to core's `value(n, v)` with
  (node, symbol) arguments meant for a local relation passed check_rr silently. Arity and types fit by chance.
  check_rr cannot catch this. It is a naming hazard, so local helpers there now have specific names.
- **rrlab run: absolute and relative paths together.** Mixing one absolute path with relative ones made rrlab
  run grow past 3.7 GB of RSS before it was killed. It looks like it walks from `/`. Keep inputs all relative
  or all absolute.

## Duplicates to merge

ts-b was written beside ts-1's ts-a. These overlap and should become one relation each:

- Parameter properties: ts-a `param_property(p)`. ts-b reimplements it locally in parameter-properties
  (`modifiers`), no-unused-vars (`marked`), unified-signatures (`prop_param`) and
  no-unnecessary-parameter-property-assignment (`has_property_param`).
- Heritage: ts-a `heritage(r)` and ts-b `heritage_ref(r)`.
- Type parentheses: ts-a `type_child(x, c)` / `type_parent` and prefer-as-const's local `type_deparen`.
- Keyword types: ts-a `ts_keyword(y, t)` and the local `predefined_type` text tests (unified-signatures
  `this_void`).
- Type positions: ts-a `in_type(n)` and ts-b's `type_ref(r, x)` with ts-b-scope's `type_head(r)`. ts-a's is
  the wider set ("a value reference there is a type-only use"). ts-b-scope needs the per-identifier
  value/type space.
- Constructors: ts-a `ts_ctor(m)` and eslint-a-functions `ctor(m)`, which ts-b rules use.
- no-non-null-asserted-optional-chain already includes ts-a (`non_null`, `in_chain`).
- First definition: ts-b-scope `first_decl(s, x, id, k)` serves no-shadow, no-use-before-define and
  no-unused-vars. Declaring statements: ts-b-scope `declaring_stmt(id, n)` and statements.rr `decl_stmt`.
- Variable scope: ts-b-scope `variable_scope(s, f)` and statements.rr `var_scope`. They model different scope
  trees: the scope manager's scopes, and eslint-scope's.
