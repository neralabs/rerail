# Rerail rule pack: TypeScript, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

Every typescript-eslint 8.70.1 rule that needs no type information has a port, shipped or a candidate here, except
`no-restricted-types` (the pack README says why).

## `init-declarations` can't match upstream's tests

typescript-eslint declares `'always'` as `init-declarations`' default. Its wrapper around ESLint's core rule has no
`meta.defaultOptions`, so with no options ESLint 10 passes the core rule no mode, and it reports nothing. Every upstream
test that expects a finding sets the option, so upstream's own tests never notice. The generator lints each case at the
rule's default options and leaves out the cases that set options, so the expected findings hold no upstream finding of
the rule at all, whatever its document says. The candidate ports the declared default, `'always'`. With that option
set, it agrees with typescript-eslint on every upstream test case.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `typescript-eslint-class-literal-property-style`

Default option `fields`. Flags getters (not `override`) whose first statement returns a literal, unless the class body has a setter of the same member name (typescript-eslint's getStaticMemberAccessValue: computed keys folded, constant bindings followed, `#a` same as `a`). A getter whose computed name has no static value is reported; a setter with none never pairs. The message tells the agent to check for subclasses that override the getter as an accessor, which a field would break (TS2611); the rule cannot see other classes, so it reports these getters as upstream does.

### `typescript-eslint-class-methods-use-this`

Flags a non-static, non-constructor class method, or a class field initialized with a function or arrow, whose own body never uses `this` or `super`. Nested functions, static blocks and nested class field initializers have their own `this`; arrows share it. Default options.

### `typescript-eslint-consistent-generic-constructors`

Covers variable declarators, class fields and parameter defaults initialized with `new Name(...)`, default mode `constructor`; typed-array built-ins are excepted. The fix rebuilds the declaration from its parts, so whitespace and comments inside it are not kept, and a declaration with modifiers is not fixed. typescript-eslint skips the constructor-mode report when the `isolatedDeclarations` parser option is set; Rerail has no such fact, so the port reports it and its message says to keep the annotation under isolatedDeclarations (TS9010).

### `typescript-eslint-consistent-indexed-object-style`

Default option `record`. Flags an interface or object type literal whose only member is an index signature, and a mapped type that does not use its key and whose constraint is not a bare `keyof`, unless the type refers back to the type alias or interface that declares it (directly or through other local type declarations). Fixes to a `Record` type where upstream does; no fix when a comment would be lost, for `-readonly`, for an interface with `extends` or under `export default`, when an interface's type parameter holds a comma and the parameters are not already joined by `, ` on one line (upstream re-joins them), or when the key or value type continues on a line indented with spaces or holds a multi-line string (the engine re-indents multi-line text, upstream copies it as is).

### `typescript-eslint-explicit-function-return-type`

Allowances with defaults: typed function expressions, higher-order functions and direct const assertions in arrows on; expressions, IIFEs, void-started concise arrows, functions without type parameters and allowed names off. Reports at the function head.

### `typescript-eslint-init-declarations`

In `always` mode, the default the rule declares. typescript-eslint 8.70.1 does not hand that default on to ESLint's core rule, so configured without options it reports nothing; this document is the rule as `["error", "always"]` runs it. It reports each variable declared by a plain name without an initial value, at the name. The head of a `for` loop counts as initialized; a declaration that is itself the body of a `for`, `for...in` or `for...of` loop counts as uninitialized even with a value, and is then reported over the whole declarator. Declarations marked `declare`, and everything inside a `declare namespace`, `declare module` or `declare global` block, are skipped. The ancestor walk (`outside-declared-module`) recognizes a declared module by its leading `declare` keyword using positive tests only, so the document is still evaluated in files with syntax errors. As in TypeScript, comments after `declare` are skipped, and a line break after it (bare or inside a block comment) makes `declare` an ordinary name rather than a modifier, so what follows is checked as a plain statement.

### `typescript-eslint-member-ordering`

Default options only: a member is flagged when an earlier member of the same body belongs to a group ranked later in typescript-eslint's default member-type order.

### `typescript-eslint-method-signature-style`

Default option `property`. Fixes to a function property except inside a module declaration, when the return type mentions `this`, or when the key is overloaded (typescript-eslint's merged-overload fix is not reproduced).

### `typescript-eslint-no-invalid-this`

Standalone port (extends ESLint's no-invalid-this). Flags `this` whose nearest non-arrow function scope is the module top level or a function ESLint's isDefaultThisBinding treats as called without a receiver. Strictness and the global `this` come from the case's language options: a script's top level and a non-strict function's `this` are valid.

### `typescript-eslint-no-shadow`

Default options. Flags a declaration whose name is already declared in an enclosing scope: the nearest enclosing scope declaring the name is found structurally (eslint-scope's scope boundaries, `var` hoisting to the function), and upstream's default exemptions apply: a type shadowing a value or a value shadowing a type (or a type-only import), parameters of signatures without a body, a later declaration that is not a function or type (hoist: functions-and-types), a function or class expression named after the variable it initializes, a static method's type parameter shadowing its class's, a module augmentation merging a type-only import, and `declare global` blocks.

### `typescript-eslint-no-unused-vars`

Reported where ESLint reports (last write from the variable's own function, else its first definition). Usedness, exports, `/* exported */` comments and report anchors come from the engine's per-variable references (`binds`, `resolves.write: "last"`); options filter on them. An unused `/* global */` name with no declaration is reported at the name in the first comment that declares it (M12a eslintDirective parse; first three names of a comment).

### `typescript-eslint-no-useless-constructor`

Exempt: private/protected constructors, `public` constructors in derived classes, parameter properties and decorated parameters. Flags an empty-body constructor in a class without extends, or one whose body is only `super(...)` passing parameters through unchanged (ESLint isPassingThrough for any number of simple parameters: positional name match, spread of a rest parameter, or one spread of `arguments`).

### `typescript-eslint-parameter-properties`

Default options, `class-property` mode: flags constructor parameter properties whose modifiers are not in the (empty) allow list.

### `typescript-eslint-prefer-function-type`

Applies to a type literal, or an interface with no supertype other than Function, whose only member is a call or construct signature with a return type; in such an interface the first `this` type (outside nested literals) is reported instead. Fixes cover call signatures without comments; checks are split by fix shape.

### `typescript-eslint-unified-signatures`

Compares each overload with every earlier overload of the same name in the same container, pairing parameters by position for any number of parameters, requiring equal return types and type parameters. Not ported: the outer type-parameter usage test and message line numbers. With three or more overloads of a name, the message names the line of the other overload ESLint picks (M12a `line` transform).

### `vitest-no-disabled-tests`

Disabled forms: `.skip`, `xit`, `xtest`, `xdescribe`, a test with no function, `pending()`.

### `typescript-eslint-no-explicit-any`

Port of typescript-eslint's `no-explicit-any` rule (https://typescript-eslint.io/rules/no-explicit-any). Flags every `any` type (default options: no fix; the rule only suggests). Options (`ignoreRestArgs`, `fixToUnknown`) are declared in the `options` define with the rule's defaults; a variant is a config patch.
