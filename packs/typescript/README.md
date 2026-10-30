# Rerail rule pack: TypeScript

62 code rules for TypeScript, ported from typescript-eslint 8.70.1 (on ESLint 10.11.0), each with its default
options. 42 start off (`"on": false`): no approved typescript-eslint configuration turns them on, so they are
experimental. Each is scored against the tool's own findings on the tool's test fixtures and on their rename variants
(`holds-under-rename`), and against the labelled lines of the tool's real-code samples
([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace path, with every
disagreement declared in `divergences.json`: 27 today. 26 are `rerail-unknown`, each an upstream fixture or its rename
variant that the grammar reads with a syntax error or misreads; 1 is `not-comparable`, a candidate's sample. The
conformance check ([conformance/README.md](../../conformance/README.md#the-gate)) gates the shipped rules. ESLint's core
rules are in the [JavaScript pack](../javascript/README.md).

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
| `candidates/rules/` | Ports still being evaluated, never shipped: typescript-eslint and @vitest/eslint-plugin 1.6.27 ports that fail the conformance check ([conformance/README.md](../../conformance/README.md#the-gate)) or rerail's pack contract. Each but `prefer-optional-chain` (it requires type information, which the expected findings do not have) is listed in `conformance.json`, scored and reported, never gated. The three `vitest-*.json` documents are generated: edit `scripts/generate/packs/typescript-vitest-policies.mjs`, not the JSON, and run `node scripts/generate/packs/typescript-vitest-policies.mjs` from the repository root |
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

### `typescript-eslint-adjacent-overload-signatures`

Reports a function, method, call or construct signature whose name was already declared earlier in the same body when the member just before it is not an overload of the same name. Method names are compared as typescript-eslint's getNameFromMember does: literal and identifier keys by static name, a computed identifier by its name, and other computed keys by their source text.

### `typescript-eslint-array-type`

Default option `array` (for mutable and readonly arrays alike). Flags a type reference named `Array` or `ReadonlyArray` with exactly one type argument, and `Readonly<T[]>`, which should be written `T[]` or `readonly T[]`. As upstream walks the scope chain from the reference, any declaration of that name, value or type, in a scope on the chain other than the global one leaves the reference alone. The chain is built in the order upstream's scope manager opens scopes: a class member's computed key lies outside the member's own scope, and a scope opened inside a class decorator, a class method's parameter decorator, a switch discriminant, an interface method's computed key or a conditional type's false branch skips the scope that holds that position. No fix is offered.

### `typescript-eslint-ban-ts-comment`

Matches TypeScript's directive regexes on comment text (block comments: last line only); description length counts approximate grapheme clusters. Defaults as upstream: @ts-expect-error allowed with a description of at least 3 characters, @ts-ignore and @ts-nocheck banned, @ts-check allowed.

### `typescript-eslint-consistent-type-assertions`

Default options (assertion style `as`, object and array literal assertions allowed). Flags every angle-bracket type assertion `<T>value`, which should be written `value as T`.

### `typescript-eslint-explicit-member-accessibility`

Reports from the first token after a member's decorators.

### `typescript-eslint-explicit-module-boundary-types`

Flags functions that cross the module boundary without explicit types: functions, classes' public members and object members reached from an export (named, default, `export =`, or a local binding an export names, followed to its declaration and its writes within the reference's own scope), plus functions they return directly, that lack a return type, and their parameters that are untyped or typed `any`. Typed contexts, returned-into-typed functions, `as const` arrow bodies, higher-order functions (unless an export before them reaches them, which ESLint checks before it has seen their returns), constructors and setters are exempt from the return type; reports sit on the function head and on each parameter. Default options; they are declared in the `options` define.

### `typescript-eslint-max-params`

Extends ESLint's max-params; default max 3, countThis `except-void`. Counts parameters of functions, methods, arrows, declared functions and function types; the message names the function as getFunctionNameWithKind does.

### `typescript-eslint-no-dynamic-delete`

String, number and negated number literal keys are allowed.

### `typescript-eslint-no-extraneous-class`

Only classes without `extends`; a constructor with parameter properties does not count as constructor-only.

### `typescript-eslint-no-inferrable-types`

Types covered: bigint, boolean, number, null, string, symbol, RegExp, undefined. The fix removes the annotation.

### `typescript-eslint-no-invalid-void-type`

Port of typescript-eslint `no-invalid-void-type`. void is allowed as a return type (and as the annotation of non-identifier bindings, mirroring the rule's grandparent test); in a union whose members are all void, never, or generics with a void argument; in a union inside a function or method implementation that has overload signatures of the same name (an anonymous `export default function` has them when the module has an `export default function` signature; method names compared like getStaticMemberAccessValue, two names without a static value treated as equal); and, with allowInGenericTypeArguments (default true), as a generic type argument outside a call or as a type parameter default.

### `typescript-eslint-no-magic-numbers`

Default options (no `ignore` values, every `ignore*` switch, `detectObjects` and `enforceConst` off). Every numeric or BigInt literal is reported, a unary `-` or `+` in front counting as part of it, unless its parent node (parentheses and computed-key brackets looked through, as ESTree does) lets it pass: a variable initializer, an object literal or pattern property (key or value), the right side of an assignment to anything but a plain identifier, the second argument of `parseInt` or `Number.parseInt`, and a JSX expression container. Enum member values, numeric literal types, type indexes and readonly class fields are reported too; in an enum member value or a readonly field a leading `+` is left out of the reported span. At these options it reports nearly every number literal, and no approved typescript-eslint configuration turns it on, so it starts off.

### `typescript-eslint-no-misused-new`

Reports a bodiless class method named `new` returning its own class, an interface construct signature returning its own interface, and a `constructor` method signature in an interface or type literal.

### `typescript-eslint-no-non-null-asserted-nullish-coalescing`

Reports a non-null assertion that is the left operand of `??`. As upstream, an assertion that belongs to an optional chain (`a?.b!`) is left alone, since ESTree folds it into the chain. A bare asserted name is also left alone when it resolves to a variable of the file, or to a builtin or declared global (ESLint's globals, the esnext lib, `/* global */` comments), that nothing writes before the assertion: no initializer, definite-assignment `!`, parameter default, assignment or update of it occurs earlier in the file. As in ESLint's scope analysis, a plain, compound or update target counts as a write through parentheses and at most one `!`, `as` or `<T>` around the name (never `satisfies`), while a destructuring or for-in/of target counts through any of them.

### `typescript-eslint-no-redeclare`

Flags each definition after the first in the same scope; with builtinGlobals, definitions of a configured or `/* global */`-declared global.

### `typescript-eslint-no-require-imports`

`require(...)` is not flagged when a declaration of `require` is in scope. Default options: allow [], allowAsImport false.

### `typescript-eslint-no-unnecessary-parameter-property-assignment`

Inside a constructor (or an arrow function called directly there), flags `this.x = x`, `this.x ||= x`, `this.x &&= x` or `this.x ??= x` when `x` is a parameter and the constructor declares `x` as a parameter property, so TypeScript already makes that assignment. It is not flagged when, earlier in the constructor and within the same class body, the parameter was written or the member was compound-assigned (`+=` and the like), or when a class field initializer of the same class assigns the member. As upstream decides whether a name is the parameter by the first reference to that name in the enclosing scope, a parameter or write is ignored when that first reference is a type reference, as in `constructor(private config: config.Settings)`.

### `typescript-eslint-no-unused-private-class-members`

Flags a `#name` member, or a member or constructor parameter property declared `private`, that nothing in its class reads (for a getter, setter or `accessor` field a write counts too), at the last declaration of its key. Keys compare as upstream's do: names, literal values (`0x10` is `16`, escapes decoded) and a template's raw text; a class whose names are all written plainly is checked by text alone, which is faster. As upstream's class scope analysis, which covers the class's decorators and heritage as well as its body, a `#name` is used by any reference that resolves to it, and a `private` member through `this` in the matching static or instance context (functions rebind it, arrow functions do not, a field initializer is the instance), through a variable declared from `this`, through the class name (static), through a parameter typed as the class (`typeof` for static), or by destructuring `this`.

### `typescript-eslint-no-use-before-define`

Default options. Flags a value reference that resolves to a binding of this file whose first declaration comes later in the source, or that sits in the initializer of the very binding it reads (within the same scope). Type positions and `typeof` queries are ignored, as are names read from the scope of a function type or type-member signature and a class named inside its own decorator, while a named export is checked even when it names a type or nothing at all.

### `typescript-eslint-no-wrapper-object-types`

typescript-eslint 8.70.1, default options. Flags BigInt/Boolean/Number/Object/String/Symbol in type references, implements and interface extends only when the name resolves to no file-local type binding. Type references are auto-fixed to the lowercase primitive.

### `typescript-eslint-prefer-for-of`

Flags a `for` loop that declares one non-const index starting at 0, runs while the index is below an array's `length`, steps it by one, and uses the index in its body only to read elements of that same array.

### `typescript-eslint-class-literal-property-style`

Default option `fields`. Flags class getters (not `override`) whose first statement returns a literal, unless the class body has a setter of the same member name (typescript-eslint's getStaticMemberAccessValue: computed keys folded, constant bindings followed, `#a` same as `a`). Getters in object literals are not checked, as upstream checks class members only. A getter whose computed name has no static value is reported; a setter with none never pairs. The message tells the agent to check for subclasses that override the getter as an accessor, which a field would break (TS2611); the rule cannot see other classes, so it reports these getters as upstream does.

### `typescript-eslint-class-methods-use-this`

Flags a non-static, non-constructor class method, or a class field initialized with a function or arrow, whose own body never uses `this` or `super`. Nested functions, static blocks and nested class field initializers have their own `this`; arrows share it. A `this` parameter (`method(this: void)`) declares the receiver's type and is no use of `this`; a default value that reads `this` is. Default options.

### `typescript-eslint-consistent-generic-constructors`

Covers variable declarators, class fields and parameter defaults initialized with `new Name(...)`, default mode `constructor`. The nine typed-array built-ins are excepted when the name is the global, as upstream's isReferenceToGlobalFunction decides: a local class, variable, import or parameter of that name is checked like any other. The fix rebuilds the declaration from its parts, so whitespace and comments inside it are not kept, and a declaration with modifiers is not fixed. typescript-eslint skips the constructor-mode report when the `isolatedDeclarations` parser option is set; Rerail has no such fact, so the port reports it and its message says to keep the annotation under isolatedDeclarations (TS9010). Divergence: `new` followed by a line break, which the TypeScript grammar reads as an identifier (`rerail-unknown`).

### `typescript-eslint-consistent-indexed-object-style`

Default option `record`. Flags an interface or object type literal whose only member is an index signature, and a mapped type that does not use its key and whose constraint is not a bare `keyof`, unless the type refers back to the type alias or interface that declares it (directly or through other local type declarations). Fixes to a `Record` type where upstream does; no fix when a comment would be lost, for `-readonly`, for an interface with `extends` or under `export default`, when an interface's type parameter holds a comma and the parameters are not already joined by `, ` on one line (upstream re-joins them), or when the key or value type continues on a line indented with spaces or holds a multi-line string (the engine re-indents multi-line text, upstream copies it as is). Divergences: files with an index signature or mapped type that has no type, which the TypeScript grammar does not parse (`rerail-unknown`).

### `typescript-eslint-consistent-type-imports`

Default options (`prefer: 'type-imports'`, `disallowTypeAnnotations: true`). Flags an import statement, at its start, once, when some of its bindings are used and every used one is used only as a type: in a type position, as the head of a qualified type name, in `typeof`, in a computed key of a property signature or in `export type { … }`. Imports already written `import type` are skipped. Files with decorators are checked: upstream skips them only under `emitDecoratorMetadata` with `experimentalDecorators`, which the rule cannot see and which are off by default. Also flags `import('…')` used as a type annotation, at `import`.

### `typescript-eslint-explicit-function-return-type`

Allowances with defaults: typed function expressions, higher-order functions and direct const assertions in arrows on; expressions, IIFEs, void-started concise arrows, functions without type parameters and allowed names off. Reports at the function head: a method's or field's start for methods and fields, and for an `accessor` field the function's own head (the arrow's `=>`, or `function` or `async`), as upstream's getFunctionHeadLoc does. An `accessor` field's type annotation does not make its function typed, and allowed names do not cover it.

### `typescript-eslint-member-ordering`

Default options only: a member is flagged when an earlier member of the same body belongs to a group ranked later in typescript-eslint's default member-type order. The document compares member kind (signature, field, static block, constructor, getter, setter, method), then group, then accessibility (public, protected, private, `#private`), rather than writing out upstream's 117-entry order rank by rank.

### `typescript-eslint-method-signature-style`

Default option `property`. Flags method signatures in interfaces and type literals only; a class's overload signatures and the members of a `declare class` are not method signatures in upstream's syntax tree. Fixes to a function property except inside a module declaration, when the return type mentions `this`, or when the key is overloaded (typescript-eslint's merged-overload fix is not reproduced).

### `typescript-eslint-no-explicit-any`

Flags every `any` type. Default options (`ignoreRestArgs` and `fixToUnknown` off): no fix, as upstream only suggests one. With `fixToUnknown`, `keyof any` is fixed to `PropertyKey`, a rest parameter's `any` to `unknown[]` and any other `any` to `unknown`; with `ignoreRestArgs`, a rest parameter's `any` is not flagged.

### `typescript-eslint-no-invalid-this`

Standalone port (extends ESLint's no-invalid-this). Flags `this` whose nearest non-arrow function scope is the module top level or a function ESLint's isDefaultThisBinding treats as called without a receiver. Strictness and the global `this` come from the case's language options: a script's top level and a non-strict function's `this` are valid. `this` in a type predicate (`this is T`, `asserts this`) is no use of `this`; `typeof this` is.

### `typescript-eslint-no-shadow`

Default options. Flags a declaration whose name is already declared in an enclosing scope: the nearest enclosing scope declaring the name is found structurally (eslint-scope's scope boundaries, `var` hoisting to the function), and upstream's default exemptions apply: a type shadowing a value or a value shadowing a type (or a type-only import), parameters of signatures without a body, a name declared only by function signatures without a body (`declare function f(): void`), a later declaration that is not a function or type (hoist: functions-and-types), a function or class expression named after the variable it initializes, a static method's type parameter shadowing its class's, a module augmentation merging a type-only import, and `declare global` blocks.

### `typescript-eslint-no-unused-vars`

Reported where ESLint reports (last write from the variable's own function, else its first definition). Usedness, exports, `/* exported */` comments and report anchors come from the engine's per-variable references (`binds`, `resolves.write: "last"`); options filter on them. The head of a qualified type name (`ns.T`) reads only types, so it marks an import or namespace used and never a parameter of the same name; members declared with `declare` directly in a `declare module` body count as used. An unused `/* global */` name with no declaration is reported at the name in the first comment that declares it (first three names of a comment). Divergences (`rerail-unknown`): an index signature or mapped type with no type, and `using`/`await using` declarations, which the TypeScript grammar does not read.

### `typescript-eslint-no-useless-constructor`

Exempt: private/protected constructors, `public` constructors in derived classes, parameter properties and decorated parameters. Flags an empty-body constructor in a class without extends, or one whose body is only `super(...)` passing its own parameters through unchanged (ESLint isPassingThrough for any number of simple parameters: positional name match, spread of a rest parameter, or one spread of `arguments`). The parameters compared are the nearest constructor's, not those of a method or constructor the class expression sits in.

### `typescript-eslint-parameter-properties`

Default options, `class-property` mode: flags constructor parameter properties (an accessibility modifier, `readonly` or `override`) whose modifiers are not in the (empty) allow list.

### `typescript-eslint-prefer-function-type`

Applies to a type literal, or an interface with no supertype other than Function, whose only member is a call or construct signature with a return type; in such an interface the first `this` type (outside nested literals) is reported instead. Fixes cover call signatures without comments; checks are split by fix shape.

### `typescript-eslint-unified-signatures`

Compares each overload with every earlier overload of the same name in the same container, pairing parameters by position for any number of parameters, requiring equal return types and type parameters. As upstream: only class declarations are class containers (an anonymous `export default class` too; a class expression's overloads are not compared), anonymous `export default function` overloads share one name, and so do computed keys that are neither an identifier nor a literal. `export declare function` overloads count, and an interface method signature is reported at its start. Not ported: the outer type-parameter usage test. With three or more overloads of a name, the message names the line of the other overload ESLint picks (a `line` transform).
