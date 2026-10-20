# Rerail rule pack: TypeScript

21 code rules for TypeScript, ported from typescript-eslint 8.70.1 (on ESLint 10.11.0), each with its default
options. 11 start off (`"on": false`): no approved typescript-eslint configuration turns them on, so they are
experimental. Each is scored against the tool's own findings on the tool's test fixtures and on a pinned corpus
(vscode), through the engine's production workspace path, with every disagreement declared in `divergences.json`
(none today). ESLint's core rules are in the [JavaScript pack](../javascript/README.md).

Every typescript-eslint rule runs on `.ts`, `.tsx`, `.mts` and `.cts` files only, and the vscode corpus is scored on
its TypeScript files only (`exclude` in `conformance.json`). On JavaScript a port steers CommonJS files away from
`require` (`no-require-imports` on `jest.config.js`), and with both packs added a port with an ESLint core twin would
report the same violation as the twin, which runs on JavaScript files only.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack typescript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and, for a rule that starts off, `on`; one rule carries examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (vscode) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`) and its recordings (`fixtures.expected.json`, `corpus-vscode.expected.json`); `upstream/eslint-plugin-vitest/UPSTREAM.md` lists the fixtures Rerail wrote (`fixtures/*/rerail-*`), which the recorder holds and records like the copied ones |
| `upstream/generate.mjs` | typescript-eslint's recorder; `upstream/eslint-plugin-vitest/generate.mjs` records @vitest/eslint-plugin |
| `candidates/rules/` | Ports still being evaluated, never shipped: typescript-eslint and @vitest/eslint-plugin 1.6.27 ports that fail a rule gate ([docs/port-readiness.md](../../docs/port-readiness.md)) or rerail's pack contract. Each but `prefer-optional-chain` (it requires type information, which the recordings do not have) and `no-explicit-any` (the fixtures recording does not cover it yet) is listed in `conformance.json`, scored and reported on the fixtures, never gated; the vscode recording covers the shipped rules only. The three `vitest-*.json` documents are generated: edit `scripts/generate/packs/typescript-vitest-policies.mjs`, not the JSON, and run `node scripts/generate/packs/typescript-vitest-policies.mjs` from the repository root |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against the pinned tool versions (the recorder's header lists them):

```sh
node upstream/generate.mjs --tools <dir> --typescript-eslint <typescript-eslint checkout at v8.70.1> \
  --fixtures --corpus vscode=<vscode checkout>
node upstream/eslint-plugin-vitest/generate.mjs --tools <dir> --vitest <eslint-plugin-vitest checkout at v1.6.27>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack typescript --rule typescript-eslint-no-namespace --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `typescript-eslint-adjacent-overload-signatures`

Reports a function, method, call or construct signature whose name was already declared earlier in the same body when the member just before it is not an overload of the same name. Method names are compared as typescript-eslint's getNameFromMember does: literal and identifier keys by static name, a computed identifier by its name, and other computed keys by their source text.

### `typescript-eslint-ban-ts-comment`

Matches TypeScript's directive regexes on comment text (block comments: last line only); description length counts approximate grapheme clusters. Defaults as upstream: @ts-expect-error allowed with a description of at least 3 characters, @ts-ignore and @ts-nocheck banned, @ts-check allowed.

### `typescript-eslint-ban-tslint-comment`

Flags comments whose value matches TSLint's enable/disable directive regex; the fix deletes the comment.

### `typescript-eslint-class-literal-property-style`

Default option `fields`. Flags getters (not `override`) whose first statement returns a literal, unless the class body has a setter of the same member name (typescript-eslint's getStaticMemberAccessValue: computed keys folded, constant bindings followed, `#a` same as `a`). A getter whose computed name has no static value is reported; a setter with none never pairs. The message tells the agent to check for subclasses that override the getter as an accessor, which a field would break (TS2611); the rule cannot see other classes, so it reports these getters as upstream does (NER-1785).

### `typescript-eslint-class-methods-use-this`

Flags a non-static, non-constructor class method, or a class field initialized with a function or arrow, whose own body never uses `this` or `super`. Nested functions, static blocks and nested class field initializers have their own `this`; arrows share it. Default options.

### `typescript-eslint-consistent-generic-constructors`

Covers variable declarators, class fields and parameter defaults initialized with `new Name(...)`, default mode `constructor`; typed-array built-ins are excepted. The fix rebuilds the declaration from its parts, so whitespace and comments inside it are not kept, and a declaration with modifiers is not fixed. typescript-eslint skips the constructor-mode report when the `isolatedDeclarations` parser option is set; Rerail has no such fact, so the port reports it and its message says to keep the annotation under isolatedDeclarations (TS9010, NER-1786).

### `typescript-eslint-consistent-type-definitions`

Default option `interface`: flags type aliases whose type is an object literal type (through any parentheses) and fixes them to interfaces.

### `typescript-eslint-default-param-last`

Identical to the ESLint base port. Only functions with a body; a trailing rest parameter does not count as required.

### `typescript-eslint-explicit-function-return-type`

Allowances with defaults: typed function expressions, higher-order functions and direct const assertions in arrows on; expressions, IIFEs, void-started concise arrows, functions without type parameters and allowed names off. Reports at the function head.

### `typescript-eslint-explicit-member-accessibility`

Reports from the first token after a member's decorators.

### `typescript-eslint-max-params`

Extends ESLint's max-params; default max 3, countThis `except-void`. Counts parameters of functions, methods, arrows, declared functions and function types; the message names the function as getFunctionNameWithKind does.

### `typescript-eslint-member-ordering`

Default options only: a member is flagged when an earlier member of the same body belongs to a group ranked later in typescript-eslint's default member-type order.

### `typescript-eslint-method-signature-style`

Default option `property`. Fixes to a function property except inside a module declaration, when the return type mentions `this`, or when the key is overloaded (typescript-eslint's merged-overload fix is not reproduced).

### `typescript-eslint-no-array-constructor`

Exempt: exactly one argument, or type arguments. No scope check (typescript-eslint does not resolve `Array` either). Calls with comments inside the argument list get no fix.

### `typescript-eslint-no-dupe-class-members`

ESLint's base rule over non-computed members only, ignoring overload signatures; compares static names within the same staticness; a getter may pair with a setter.

### `typescript-eslint-no-duplicate-enum-values`

Initializers considered: string, number or static template literal under any chain of unary + and -; compared by Object.is; NaN and other initializers ignored. Default options (typescript-eslint 8.70.1).

### `typescript-eslint-no-dynamic-delete`

String, number and negated number literal keys are allowed.

### `typescript-eslint-no-empty-function`

A body with a comment is not empty; constructors with parameter properties are exempt. Default options (allow: []).

### `typescript-eslint-no-extra-non-null-assertion`

Parentheses are transparent, as in ESTree.

### `typescript-eslint-no-extraneous-class`

Only classes without `extends`; a constructor with parameter properties does not count as constructor-only.

### `typescript-eslint-no-inferrable-types`

Types covered: bigint, boolean, number, null, string, symbol, RegExp, undefined. The fix removes the annotation.

### `typescript-eslint-no-invalid-this`

Standalone port (extends ESLint's no-invalid-this). Flags `this` whose nearest non-arrow function scope is the module top level or a function ESLint's isDefaultThisBinding treats as called without a receiver. Strictness and the global `this` come from the case's language options: a script's top level and a non-strict function's `this` are valid.

### `typescript-eslint-no-invalid-void-type`

Port of typescript-eslint `no-invalid-void-type`. void is allowed as a return type (and as the annotation of non-identifier bindings, mirroring the rule's grandparent test); in a union whose members are all void, never, or generics with a void argument; in a union inside a function or method implementation that has overload signatures of the same name (an anonymous `export default function` has them when the module has an `export default function` signature; method names compared like getStaticMemberAccessValue, two names without a static value treated as equal); and, with allowInGenericTypeArguments (default true), as a generic type argument outside a call or as a type parameter default.

### `typescript-eslint-no-misused-new`

Reports a bodiless class method named `new` returning its own class, an interface construct signature returning its own interface, and a `constructor` method signature in an interface or type literal.

### `typescript-eslint-no-namespace`

typescript-eslint 8.70.1, default options. allowDefinitionFiles is fixed at its default (true): files matched by isDefinitionFile (`\.d\.(ts|cts|mts|.*\.ts)$`) are not checked. Not flagged: `declare global`, `global {}` inside `declare module 'name'`, and `declare module 'name'`. Declared namespaces are flagged (allowDeclarations default false).

### `typescript-eslint-no-non-null-assertion`

typescript-eslint 8.70.1, default options. Where tree-sitter misparses a prefix operator applied to an assertion (`!x!` as `(!x)!`), the finding starts at the real operand.

### `typescript-eslint-no-non-null-asserted-optional-chain`

A parenthesized optional chain that is non-null asserted (`(x?.y)!`) is reported at the chain.

### `typescript-eslint-no-redeclare`

Flags each definition after the first in the same scope; with builtinGlobals, definitions of a configured or `/* global */`-declared global.

### `typescript-eslint-no-require-imports`

`require(...)` is not flagged when a declaration of `require` is in scope. Default options: allow [], allowAsImport false.

### `typescript-eslint-no-unsafe-function-type`

typescript-eslint 8.70.1, default options. Flags `Function` as a type reference, in class implements, or interface extends, only when the name resolves to no file-local type binding.

### `typescript-eslint-no-unused-expressions`

Unwraps parentheses and TS `as` / `<T>` / `!` / instantiation wrappers; allowed: call, new, assignment, update, await, yield, import(), void, delete; directive prologue strings exempt. tree-sitter reads `e as A.B.C` and `e satisfies A.B.C` (type name with two or more dots) as a member access on `e as A.B` / `e satisfies A.B`; such a member access is treated as the `as` (unwrapped) or `satisfies` (never flagged, as upstream).

### `typescript-eslint-no-unused-vars`

Reported where ESLint reports (last write from the variable's own function, else its first definition). Usedness, exports, `/* exported */` comments and report anchors come from the engine's per-variable references (`binds`, `resolves.write: "last"`); options filter on them. An unused `/* global */` name with no declaration is reported at the name in the first comment that declares it (M12a eslintDirective parse; first three names of a comment).

### `typescript-eslint-no-useless-constructor`

Exempt: private/protected constructors, `public` constructors in derived classes, parameter properties and decorated parameters. Flags an empty-body constructor in a class without extends, or one whose body is only `super(...)` passing parameters through unchanged (ESLint isPassingThrough for any number of simple parameters: positional name match, spread of a rest parameter, or one spread of `arguments`).

### `typescript-eslint-no-wrapper-object-types`

typescript-eslint 8.70.1, default options. Flags BigInt/Boolean/Number/Object/String/Symbol in type references, implements and interface extends only when the name resolves to no file-local type binding. Type references are auto-fixed to the lowercase primitive.

### `typescript-eslint-parameter-properties`

Default options, `class-property` mode: flags constructor parameter properties whose modifiers are not in the (empty) allow list.

### `typescript-eslint-prefer-as-const`

typescript-eslint 8.70.1, default options. Flags a literal type that repeats the raw text of the literal it asserts (`x as 'a'`, `<'a'>x`) or annotates (`let v: 'a' = 'a'`, class fields); assertions are auto-fixed to `const`.

### `typescript-eslint-prefer-function-type`

Applies to a type literal, or an interface with no supertype other than Function, whose only member is a call or construct signature with a return type; in such an interface the first `this` type (outside nested literals) is reported instead. Fixes cover call signatures without comments; checks are split by fix shape.

### `typescript-eslint-prefer-literal-enum-member`

Allowed initializers: a literal, a template literal without substitutions, or a signed literal; allowBitwiseExpressions option additionally allows bitwise expressions.

### `typescript-eslint-prefer-namespace-keyword`

Only `module Name {}` with an identifier name; reports the whole declaration (from `declare` when present) and fixes the keyword.

### `typescript-eslint-unified-signatures`

Compares each overload with every earlier overload of the same name in the same container, pairing parameters by position for any number of parameters, requiring equal return types and type parameters. Not ported: the outer type-parameter usage test and message line numbers. With three or more overloads of a name, the message names the line of the other overload ESLint picks (M12a `line` transform).

### `vitest-no-disabled-tests`

Disabled forms: `.skip`, `xit`, `xtest`, `xdescribe`, a test with no function, `pending()`.

### `typescript-eslint-no-explicit-any`

Port of typescript-eslint's `no-explicit-any` rule (https://typescript-eslint.io/rules/no-explicit-any). Flags every `any` type (default options: no fix; the rule only suggests). Options (`ignoreRestArgs`, `fixToUnknown`) are declared in the `options` define with the rule's defaults; a variant is a config patch.
