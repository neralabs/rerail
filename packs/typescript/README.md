# Rerail rule pack: TypeScript

24 code rules for TypeScript, ported from typescript-eslint 8.70.1 (on ESLint 10.11.0), each with its default
options. 11 start off (`"on": false`): no approved typescript-eslint configuration turns them on, so they are
experimental. Each is scored against the tool's own findings on the tool's test fixtures and on their rename variants
(`holds-under-rename`), and against the labelled lines of the tool's real-code samples
([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace path, with every
disagreement declared in `divergences.json`: 2 today, both `rerail-unknown`, each an upstream fixture the grammar reads
with a syntax error. The conformance check ([conformance/README.md](../../conformance/README.md#the-gate)) gates the
shipped rules. ESLint's core rules are in the [JavaScript pack](../javascript/README.md).

Every typescript-eslint rule runs on `.ts`, `.tsx`, `.mts` and `.cts` files only. On JavaScript a port steers CommonJS
files away from
`require` (`no-require-imports` on `jest.config.js`), and with both packs added a port with an ESLint core twin would
report the same violation as the twin, which runs on JavaScript files only.

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack typescript"`, against the
[pack structure and validation contract](../../docs/pack-format.md).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and, for a rule that starts off, `on`; one rule carries examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`, their rename variants under `fixtures/renamed/`), its expected findings (`fixtures.expected.json`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`); `upstream/eslint-plugin-vitest/UPSTREAM.md` lists the fixtures Rerail wrote (`fixtures/*/rerail-*`), which the generator holds and lints like the copied ones |
| `upstream/generate.mjs` | typescript-eslint's generator; `upstream/eslint-plugin-vitest/generate.mjs` is @vitest/eslint-plugin's |
| `candidates/rules/` | Ports still being evaluated, never shipped: typescript-eslint and @vitest/eslint-plugin 1.6.27 ports that fail the conformance check ([conformance/README.md](../../conformance/README.md#the-gate)) or rerail's pack contract. Each but `prefer-optional-chain` (it requires type information, which the expected findings do not have) and `no-explicit-any` (the expected findings do not cover it yet) is listed in `conformance.json`, scored and reported, never gated. The three `vitest-*.json` documents are generated: edit `scripts/generate/packs/typescript-vitest-policies.mjs`, not the JSON, and run `node scripts/generate/packs/typescript-vitest-policies.mjs` from the repository root |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/typescript.ts`) runs each tool's generator
against the pinned tool versions and the upstream checkouts (the generator's header lists them). The fixtures Rerail
wrote (`fixtures/*/rerail-*`) are authored: a change to them is staged into `publish/` with the derived data.

```sh
npm run fixtures:derive -- --pack typescript                                # every tool
npm run fixtures:derive -- --pack typescript --tool eslint-plugin-vitest    # one
npm run fixtures:stage -- --pack typescript --tool eslint-plugin-vitest     # copy what changed into upstream/eslint-plugin-vitest/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack typescript --rule typescript-eslint-no-namespace --fixtures
```

## Not ported: `no-restricted-types`

typescript-eslint 8.70.1's `no-restricted-types` reports only the types a project lists in its `types` option, and that
list is empty by default. At its default options it reports nothing, so there is nothing to port. A project that wants a
type banned writes its own code rule.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `typescript-eslint-ban-tslint-comment`

Flags comments whose value matches TSLint's enable/disable directive regex; the fix deletes the comment.

### `typescript-eslint-consistent-type-definitions`

Default option `interface`: flags type aliases whose type is an object literal type (through any parentheses) and fixes them to interfaces.

### `typescript-eslint-default-param-last`

Identical to the ESLint base port. Only functions with a body; a trailing rest parameter does not count as required.

### `typescript-eslint-no-array-constructor`

Exempt: exactly one argument, or type arguments. No scope check (typescript-eslint does not resolve `Array` either). Calls with comments inside the argument list get no fix.

### `typescript-eslint-no-dupe-class-members`

ESLint's base rule over non-computed members only, ignoring overload signatures; compares static names within the same staticness; a getter may pair with a setter.

### `typescript-eslint-no-duplicate-enum-values`

Initializers considered: string, number or static template literal under any chain of unary + and -; compared by Object.is; NaN and other initializers ignored. Default options (typescript-eslint 8.70.1).

### `typescript-eslint-no-empty-function`

A body with a comment is not empty; constructors with parameter properties are exempt. Default options (allow: []).

### `typescript-eslint-no-empty-object-type`

Default options (allowInterfaces: never, allowObjectTypes: never, no allowWithName). Reports the name of an interface that declares no members and extends at most one type, and every `{}` type literal with no members that is not an operand of an intersection type. Upstream only suggests replacements, so there is no fix.

### `typescript-eslint-no-extra-non-null-assertion`

Parentheses are transparent, as in ESTree.

### `typescript-eslint-no-namespace`

typescript-eslint 8.70.1, default options. allowDefinitionFiles is fixed at its default (true): files matched by isDefinitionFile (`\.d\.(ts|cts|mts|.*\.ts)$`) are not checked. Not flagged: `declare global`, `global {}` inside `declare module 'name'`, and `declare module 'name'`. Declared namespaces are flagged (allowDeclarations default false).

### `typescript-eslint-no-non-null-assertion`

typescript-eslint 8.70.1, default options. Where tree-sitter misparses a prefix operator applied to an assertion (`!x!` as `(!x)!`), the finding starts at the real operand.

### `typescript-eslint-no-non-null-asserted-optional-chain`

A parenthesized optional chain that is non-null asserted (`(x?.y)!`) is reported at the chain.

### `typescript-eslint-no-unsafe-declaration-merging`

Reports the name of a class declaration when an interface of the same name is declared in the same scope, and the name of an interface without type parameters (an interface with them is read in a scope of its own) when a class of the same name is declared in the same scope: TypeScript merges the two without checking that the class implements the interface's members. The scope is matched structurally: the nearest program, block (a function, namespace, module, `declare global` or static block body), switch body, `with` statement, or loop whose head declares a let or const binding above the declaration, reached through `export`, `declare` and `case` wrappers and through statement bodies that open no scope (if, else, while, do, labels, and loops whose head declares no let or const binding).

### `typescript-eslint-no-unsafe-function-type`

typescript-eslint 8.70.1, default options. Flags `Function` as a type reference, in class implements, or interface extends, only when the name resolves to no file-local type binding.

### `typescript-eslint-no-unused-expressions`

Unwraps parentheses and TS `as` / `<T>` / `!` / instantiation wrappers; allowed: call, new, assignment, update, await, yield, import(), void, delete; directive prologue strings exempt. tree-sitter reads `e as A.B.C` and `e satisfies A.B.C` (type name with two or more dots) as a member access on `e as A.B` / `e satisfies A.B`; such a member access is treated as the `as` (unwrapped) or `satisfies` (never flagged, as upstream).

### `typescript-eslint-prefer-as-const`

typescript-eslint 8.70.1, default options. Flags a literal type that repeats the raw text of the literal it asserts (`x as 'a'`, `<'a'>x`) or annotates (`let v: 'a' = 'a'`, class fields); assertions are auto-fixed to `const`.

### `typescript-eslint-prefer-literal-enum-member`

Allowed initializers: a literal, a template literal without substitutions, or a signed literal; allowBitwiseExpressions option additionally allows bitwise expressions.

### `typescript-eslint-prefer-namespace-keyword`

Only `module Name {}` with an identifier name; reports the whole declaration (from `declare` when present) and fixes the keyword.

### `typescript-eslint-triple-slash-reference`

Default options (lib: always, path: never, types: prefer-import). Looks only at the line comments that come before the file's first token (a shebang may precede them). Every `/// <reference path="..." />` directive there is reported. A `/// <reference types="..." />` directive there is reported once for each import declaration or `import x = require(...)` anywhere in the file whose module string has the same value. A comment counts as leading when the first child of the program that is either a code node or the comment itself turns out to be that comment. Because that test is stated positively, the rule still runs on files with syntax errors. The directive regexes list ECMAScript's whitespace characters (which include U+FEFF but not U+0085) instead of using the engine's Unicode `\s`, so they accept exactly the spacing upstream's JavaScript regex accepts.
