# Review: typescript-03

Branch reviewed: `tbelanger/pack-translations-typescript-03` at `23d8d666dd`. I read the five `.rerail` files and
their JSON originals, then the plugin's source on the box (`dist/rules/*.js`, `dist/util/collectUnusedVariables.js`,
`dist/util/isTypeOnlyReference.js`, `dist/util/class-scope-analyzer/*`), before the author's NOTES and result file.
The tool ran on a fleet box: ESLint 9.39.5, `typescript-eslint` 8.74.0 (parser, plugin and scope manager),
TypeScript 6.0.3, flat config with only the rule under test, `--no-inline-config` except where noted. Each
translation ran with `rerail-engine run-policies` (defaults) or `run-rr` (options), using this branch's
`docs/rule-syntax/lib`. Every counterexample below is a whole file. Line:column comes from ESLint; the engine gives
the line.

| Rule | Verdict |
|---|---|
| `typescript-eslint-no-unnecessary-parameter-property-assignment` | 2 defects (`accessor` field writes, a parenthesised field value) |
| `typescript-eslint-no-unused-private-class-members` | 7 defects (`accessor #x`, typed variables, generic parameter types, destructured `self`, reassigned `self`, literal and raw template keys, static method with a `this` parameter) |
| `typescript-eslint-no-unused-vars` | 5 defects (type predicates, merged exported type, storable functions, after-used with `typeof`, rest siblings with a default) and a scope note |
| `typescript-eslint-no-use-before-define` | 1 defect (type parameters under `typedefs: false`) |
| `typescript-eslint-parameter-properties` | 3 defects (`abstract` and `accessor` fields, computed `this[x]` and parenthesised targets, computed field keys) |

## typescript-eslint-no-unnecessary-parameter-property-assignment

Agreed: `=`, `||=`, `&&=`, `??=`, `this["b"]`, ``this[`c`]``, `b!`, `(c as string)`, `(this).a`, an arrow passed as an
argument (`foo(() => …)`) as an IIFE, `new Foo(() => …)` and a nested arrow IIFE not, a write to an arrow IIFE's own
parameter `a` before `this.a = a`, `+=` before, a `static` field writing `this.a`, a string
`"constructor"`, a parameter default `b = (this.a = a)`, `private readonly a = "d"`, a write inside `if`.

**Defect 1 (miss): a write in an `accessor` field counts as a field write.** The rule's selector is
`PropertyDefinition AssignmentExpression`; an `accessor` field is an `AccessorProperty`, so its initialiser's
`this.a = …` does not reach `assignedBeforeConstructor`. `field_write` takes any `public_field_definition`, which in
tree-sitter includes `accessor` fields.

```ts
// c1.ts
export class A {
  accessor b = (this.a = "x");
  constructor(public a: string) { this.a = a; }
}
```

- ESLint: `c1.ts:3:35` unnecessaryAssign.
- translation: nothing.

**Defect 2 (extra, minor): a parenthesised field value.** ESTree has no parentheses, so
`findParentPropertyDefinition(node)?.value === functionNode.parent` holds for `b = (foo(() => …))`. `field_write`'s
second clause reads `field(p, "value", c)` without `deparen`.

```ts
// c3.ts
export class A {
  b = (foo(() => { this.a = "x"; }));
  constructor(public a: string) { this.a = a; }
}
declare function foo(f: () => void): void;
```

- ESLint: nothing.
- translation: `c3.ts:3`.

## typescript-eslint-no-unused-private-class-members

Agreed: fields, methods, overloads (the last signature wins), getter/setter pairs (reported at the setter),
`declare private`, parameter properties with `private`/`private readonly`/default, `this.p = 2` writes, `+=` and
`(this.y) = 2` statements, `for (this.#z of …)`, `({ a: this.x } = …)`, `const { a } = this`, `({ b: b2 } = this)`,
`({ c } = this) => c`, `static f = function () { return this.s; }`, an object-literal method's `this`, a nested
`function f(this: A)`, `static u = this.t` (the instance map), `[1n]`.

**Defect 1 (extra): `accessor #x` is never reported.** The analyser's `PrivateIdentifier` visitor skips the key of a
`MethodDefinition` or `PropertyDefinition` only; an `AccessorProperty`'s own `#x` key is counted as a read of `#x`.
`access` excludes every `holder_key`. (`private accessor y` is reported by both.)

```ts
// d1.ts
export class A { accessor #x = 1; private accessor y = 2; }
```

- ESLint: `d1.ts:1:52` 'y' only.
- translation: `d1.ts:1` y and `d1.ts:1` #x.

**Defect 2 (miss): a typed variable is no access path.** `getObjectClass` returns early for a variable whose first
definition is a `VariableDeclarator` unless its init is `this`; the type annotation is read only for other
definitions (parameters). `site_type`'s first clause (`variable_declarator` with a `type`) makes `o.x` a read.

```ts
// d2.ts
export class C {
  private x = 1;
  static make() { const o: C = new C(); return o.x; }
}
```

- ESLint: `d2.ts:2:11` 'x'.
- translation: nothing.

**Defect 3 (extra): a generic parameter type.** `o: C<number>` is a `TSTypeReference` whose `typeName` is `C`, so
`o.x` reads `C`'s `x`. `site_type` takes the annotation's first child, a `generic_type` here, and `typed_site` wants
a `type_identifier`.

```ts
// d3.ts
export class C<T> {
  private x = 1;
  m(o: C<number>) { return o.x; }
  t?: T;
}
```

- ESLint: nothing.
- translation: `d3.ts:2` x.

**Defect 4 (extra): a name destructured from `this` is `self`.** The analyser checks the first definition's
`VariableDeclarator.init`, which is `this` for `const { y: self } = this` too, so `self.x` reads `this.x`.
`self_site` requires the declarator's name to be an identifier.

```ts
// d4.ts
export class A {
  private x = 1;
  private y = this;
  m() { const { y: self } = this; return self.x; }
}
```

- ESLint: nothing.
- translation: `d4.ts:2` x.

**Defect 5 (miss): `self` reassigned other than by `=` or `op=`.** The analyser gives up on `self` when any write
reference is not the initialiser: `self++`, `[self] = …`, `for (self of …)`. `reassigned` looks at assignment
expressions with an identifier left only.

```ts
// d6.ts
export class A {
  private x = 1;
  private y = 1;
  m() { let self = this; self++; return self.x; }
  n() { let self = this; [self] = [this]; return self.y; }
}
```

- ESLint: `d6.ts:2:11` 'x', `d6.ts:3:11` 'y'.
- translation: nothing.

**Defect 6 (miss): literal keys other than strings and numbers, and raw template keys.** `extractComputedName` keys a
`Literal` by `value?.toString() ?? 'null'` (`[true]` → `true`, `[null]` → `null`) and a template by its raw text
(`` [`a\x62`] `` → `a\x62`, not `ab`). `member_key` has no key for `true`/`null` and cooks the template, so the first
two are never reported and the third is matched by `this.ab`.

```ts
// d5.ts
export class A {
  private [true] = 1;
  private [null] = 2;
  private [1n] = 3;
  private [`a\x62`] = 4;
  m() { return this.ab; }
}
```

- ESLint: `d5.ts:2:12` 'true', `d5.ts:3:12` 'null', `d5.ts:4:12` '1', `d5.ts:5:12` 'a\x62'.
- translation: `d5.ts:4` 1 only.

**Defect 7 (miss, minor): a static method with a `this` parameter.** A method's frame is its `MethodDefinition`
(static map for `static n`); the `this: B` parameter matters only for plain functions. `frame_ctx`'s `this`-parameter
clause also matches `method_definition`, so `static n(this: B) { this.y }` reads the instance `y` as well.

```ts
// d10.ts
export class A {
  private x = 1;
  m() { function f(this: A) { return this.x; } return f; }
}
export class B {
  private x = 1;
  private y = 1;
  m(this: B) { return this.x; }
  static n(this: B) { return this.y; }
}
```

- ESLint: `d10.ts:7:11` 'y'.
- translation: nothing.

## typescript-eslint-no-unused-vars

Agreed: messages and positions (last write in the variable's own function scope, else the first declaration;
"assigned a value" for initialisers, defaults, for-of heads and destructuring), `let`/`const`/`var`, catch,
imports, `import X = require`, `export import`, type-only imports used in `typeof`, value imports used only in
`typeof` (no report), `let d; type D = typeof d` (only used as a type), self references in functions, `const f =
function`, arrows, enums, type aliases, interfaces, namespaces, `class D { static m() { return new D(); } }`, setter
and signature parameters (overloads, `declare function`, abstract methods, function types, interface members),
parameter properties, `this` parameters, mapped keys, `for (const k in o) return;`, `for (j of o) { return; }`,
`s = (s, 1)`, `l ??= 1`, a write in a `while` loop, `z = z + 1, 0`, `for (i++; ;)`, `b = () => b`, `c = foo(() => c)`,
`args: "after-used"` with defaults, destructuring and rest, `.d.ts` top levels, `declare module`/`declare namespace`
blocks and their nested namespaces, `export {}`/`export default 1 + 2` in an ambient block, `export default v` in a
`.d.ts`, `vars: "local"` (every file is a module), the four ignore patterns with `reportUsedIgnorePattern` and their
message suffixes, `ignoreUsingDeclarations`, `ignoreClassWithStaticInitBlock`, `caughtErrors: "none"`, JSX uses,
`/* global */` items (with inline config; the translation's message is its own).

**Defect 1 (message): a parameter used only in a type predicate.** `isTypeOnlyReference` is true for a reference in a
`TSTypePredicate` (`x is string`, `asserts y`), so the message is "only used as a type". `only_type` reads
`type_query` only.

```ts
// v1.ts
export function f(x: unknown): x is string { return true; }
export function g(y: unknown): asserts y { }
```

- ESLint: `v1.ts:1:19` "'x' is defined but only used as a type.", `v1.ts:2:19` "'y' is defined but only used as a
  type."
- translation: `v1.ts:1` "'x' is defined but never used.", `v1.ts:2` "'y' is defined but never used."

**Defect 2 (miss): an exported type merged with an unexported value.** `isExported` and `isMergeableExported` skip an
interface or type alias def when the variable is both a type and a value (`isMergedTypeDeclaration`), so the
`const` is unused. `used`'s export clause takes any exported declaration of the variable.

```ts
// v2.ts
export interface Foo { a: number }
const Foo = 1;
```

- ESLint: `v2.ts:2:7` "'Foo' is assigned a value but never used."
- translation: nothing.

**Defect 3 (miss): which functions in a right side store the read.** `isStorableFunction` climbs from the function's
parent while still inside the right side: a call or `new` argument, an assignment, a tagged template, a `yield` or a
statement stores it; an array, an object, a conditional, `as`, or reaching the right side's top does not, and the read
is then for itself. `stored` treats every function in the right side that is not called on the spot as stored.

```ts
// v3.ts
let a: unknown = 0;
a = [() => a];
let b: unknown = 0;
b = () => b;
let c: unknown = 0;
c = foo(() => c);
declare function foo(f: unknown): unknown;
```

- ESLint: `v3.ts:2:1` 'a', `v3.ts:4:1` 'b'.
- translation: `v3.ts:4` 'b' only.

**Defect 4 (miss): `args: "after-used"` counts a later parameter's `typeof` reference.** `isAfterLastUsedArg` takes a
later parameter as used only for a reference that is not type-only. `later_param_used` takes any `resolves`.

```ts
// v5.ts
export function f(a: number, b: number): typeof b { return 0; }
```

- ESLint: `v5.ts:1:19` "'a' is defined but never used.", `v5.ts:1:30` "'b' is defined but only used as a type."
- translation: `v5.ts:1` 'b' only.

**Defect 5 (miss): `ignoreRestSiblings` with a default.** `hasRestSibling(def.name.parent)` wants a `Property`; for
`{ a = 1, ...rest }` the name's parent is the `AssignmentPattern`, and so is each reference's, so `a` is not exempt.
`rest_sibling`'s `object_assignment_pattern` branch exempts it.

```ts
// w1.ts  (option {"ignoreRestSiblings": true})
export function f(o: any) { const { a = 1, ...rest } = o; return rest; }
export function g(o: any) { const { b, ...rest } = o; return rest; }
export function h(o: any) { let c; ({ c, ...o } = o); return o; }
export function k(o: any) { let d; ({ d = 1, ...o } = o); return o; }
```

- ESLint: `w1.ts:1:37` 'a', `w1.ts:4:39` 'd'.
- translation: nothing.

**Scope note.** The translation's `.scope include "**/*"` widens the original's `**/*.ts`, `**/*.tsx`, `**/*.mts`,
`**/*.cts`. The one rerail-typescript translation-only key, `test/scripts/pack-test-data.test.mjs:1299`, is in a
`.mjs` file the original's scope leaves out, whatever it does with string-keyed pair patterns. ESLint reports it when
it is configured to lint `.mjs` with the TypeScript parser (below); whether `.js`/`.mjs` files belong in this rule's
scope is the pack's call, and the NOTES do not mention the widening.

## typescript-eslint-no-use-before-define

Agreed with defaults and with `"nofunc"`, `allowNamedExports: true`, `classes`/`variables`/`enums: false` and
`ignoreTypeReferences: false`: write references (`a = 1; let a`), for-in targets, `export { x } from`, `export *
as`, computed keys in type literals, `export default a`/`export = b` (type and value references), class fields and
static blocks as their own variable scopes, initialisers (`g(a = a)`, `{ c = c }`, `for (const d of d)`,
`for (var e in e)`, `for (let i = i;;)`, `{ p: { q } = q }`), namespaces, `module M`, enums and their members,
imports, class decorators with and without `export`, `typeof` types, signatures (overloads, function types,
`declare function`), `export { f }` with `"nofunc"`, `export type { T }`, JSX, `extends Y`.

**Defect 1 (extra): type parameters under `typedefs: false`.** The scope manager defines a type parameter with a
`Type` definition, so `isTypedef` holds and `typedefs: false` exempts it. `exempt_kind` has no row for the
type-parameter kind (`type-param` in `ts-b-scope`), so `forbidden` keeps it.

```ts
// h1.ts  (options {"ignoreTypeReferences": false, "typedefs": false, "allowNamedExports": true})
export function f<T extends U, U>(a: T, b: U) { return [a, b]; }
export let x: I;
interface I {}
export const y = <V,>(v: V): W => v as any;
type W = number;
```

- ESLint: nothing.
- translation: `h1.ts:1` 'U'.
- With `{"ignoreTypeReferences": false}` alone both report `1:29` 'U', `2:15` 'I', `4:30` 'W'.

## typescript-eslint-parameter-properties

Agreed for `"class-property"`: decorated parameters (reported at the decorator's line), `readonly`, `override`, a
default, `?`, every accessibility and `readonly` pair. For `"parameter-property"`: `declare`, `static`, `?` and `!`
fields, a run of assignments broken by `;`, comments, `this.y = x` keyed by the right side, `this.x = (x)`,
`y as string` (no report), `x:string` against `x: string` (no report).

**Defect 1 (extra): `abstract` and `accessor` fields.** The rule records `PropertyDefinition` elements only;
`abstract x` is a `TSAbstractPropertyDefinition` and `accessor x` an `AccessorProperty`. `bare_field` takes every
`public_field_definition`, which in tree-sitter includes both.

```ts
// b1.ts  (option {"prefer": "parameter-property"})
export abstract class A {
  abstract x: string;
  constructor(x: string) { this.x = x; }
}
```

```ts
// b2.ts  (option {"prefer": "parameter-property"})
export class A {
  accessor x: string;
  constructor(x: string) { this.x = x; }
}
```

- ESLint: nothing on either.
- translation: `b1.ts:2`, `b2.ts:2`.

**Defect 2 (miss): the assignment run takes computed and parenthesised targets.** The rule's loop wants a
`MemberExpression` on `this` whose `property` is an `Identifier`: `this[x]` is one (computed), and ESTree drops the
parentheses of `(this.q)`. `this_assign` wants a `member_expression` with a `property_identifier`, and reads `left`
without `deparen`, so both end the run.

```ts
// b3.ts  (option {"prefer": "parameter-property"})
export class A {
  x: string;
  constructor(x: string) { this[x] = x; }
}
```

```ts
// b5.ts  (option {"prefer": "parameter-property"})
export class A {
  x: string;
  y: number;
  constructor(x: string, y: number) { this.y = x; (this.q) = y; }
}
```

- ESLint: `b3.ts:2:3` x; `b5.ts:2:3` x, `b5.ts:3:3` y.
- translation: nothing on b3; `b5.ts:2` x only.

**Defect 3 (miss): a computed field key that is an identifier.** The rule takes `element.key.type === Identifier`
without looking at `computed`, so `[x]: string` is the field `x`. `bare_field` wants a `property_identifier` name.

```ts
// b4.ts  (option {"prefer": "parameter-property"})
const x = "x";
export class A {
  [x]: string;
  constructor(x: string) { this.x = x; }
}
```

- ESLint: `b4.ts:3:3` x.
- translation: nothing.

## The author's result file

Read after the findings above. Both `different` entries say the translation is right; the tool agrees on each sample
key (files taken from the box's `corpora.tar`):

| Rule | Key | typescript-eslint 8.74.0 | translation |
|---|---|---|---|
| no-unused-vars | `recorded/typescript/files/typescript-eslint/consistent-indexed-object-style/037.ts` | `1:6` 'Foo' is defined but never used | `037.ts:1` |
| no-unused-vars | `…/consistent-indexed-object-style/088.ts` | `2:11` 'Foo' is defined but never used | `088.ts:2` |
| no-unused-vars | `real/rerail-typescript/test/scripts/pack-test-data.test.mjs` (config `files: ["**/*.mjs"]`) | `1299:55` '_licences' is assigned a value but never used | `:1299` |
| parameter-properties | `real/rerail-typescript/src/engine/storage/database.ts` | nothing (no `:35`, no `:267`) | nothing |

The three `same` verdicts hold for the corpora; the defects above are in cases the corpora do not have.
