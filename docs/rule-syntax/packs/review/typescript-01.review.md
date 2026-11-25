# Review: typescript-01

Branch reviewed: `tbelanger/pack-translations-typescript-01` at `0bfbf02912`. I read the five `.rerail` files and
their JSON originals, then the plugin's source (`src/rules/*.ts` at v8.74.0, and `dist/util` on the box) and its
tests (`tests/rules/*.test.ts`), before the author's NOTES and result file. The tool ran on a fleet box: ESLint
9.39.5, `typescript-eslint` 8.74.0 (parser and plugin), TypeScript 6.0.3, flat config with only the rule under test.
Each translation ran with `rerail-engine run-policies` (defaults) or `run-rr` (options), using this branch's
`docs/rule-syntax/lib`. Every counterexample below is a whole file. Line:column comes from ESLint; the engine gives
the line.

| Rule | Verdict |
|---|---|
| `typescript-eslint-class-literal-property-style` | 6 defects (computed keys by text, constants, constructor detection, nested class, object destructuring, `accessor` fields) |
| `typescript-eslint-class-methods-use-this` | 1 defect (decorated field reported at its decorator) |
| `typescript-eslint-consistent-indexed-object-style` | 3 defects (names by text not binding, type parameters of function types, shadowed mapped key) |
| `typescript-eslint-consistent-type-definitions` | ok |
| `typescript-eslint-explicit-member-accessibility` | 1 minor (non-ASCII string key under `ignoredMethodNames`) |

Fixes and suggestions were not compared: `run-policies` reports no edits in its diagnostics.

## typescript-eslint-class-literal-property-style

`"fields"` (default): a class `MethodDefinition` of kind `get`, not `override`, whose first statement returns a
`Literal`, a `TemplateLiteral` or a `TaggedTemplateExpression` with one quasi; skipped when `getStaticMemberAccessValue`
gives a name and a `set` of the same static name is in the class body. `"getters"`: a `readonly` `PropertyDefinition`
(not `declare`, not `override`) with such a value, unless a `this.<name>` assignee whose nearest function is a
`constructor` `MethodDefinition` was seen while that class body was the innermost one. Agreed: tagged templates with
and without substitutions, `String.raw`, `tag<T>`, regex, BigInt, `-1`, parentheses, a leading comment, `;` first,
number keys (`1`/`"1"`, `0x10`/`16`), `get #p` beside `set p`, `static get` beside an instance `set`, decorators,
`override`, return types, object-literal getters, `#a` assigned in the constructor, `[this.b] = …`, `static readonly`,
`declare readonly`.

**Defect 1 (miss): a computed key with no static value pairs by its text.** `has_setter`'s second clause pairs
`get [k]()` with `set [k](v)` by source text. In 8.74 `getStaticMemberAccessValue` returns `undefined` for `[k]` when
`k` is not a resolvable constant, `name && …` is false, and the getter is reported. The comment's "8.70 does" is not
what 8.74 does; the plugin's valid test pairs `[p1]` only because `let p1 = 'p1'` has a static value.

```ts
// f2.ts
export function f(k: string) {
  return class {
    get [k]() { return 1; }
    set [k](v: number) {}
  };
}
```

- ESLint: `f2.ts:3:10` preferFieldStyle.
- translation: nothing.

**Defect 2 (extra): constants are not resolved.** `getStaticMemberAccessValue` runs `getStaticValue` in scope, so a
`const` (or never-reassigned `let`) key has its value: `[k]` with `const k = "a"` is the name `a`. `key_static` and
`static_prop` read literals only, so neither the setter pairing (fields) nor the constructor exclusion (getters) sees
it.

```ts
// f1.ts
const k = "a";
export class A {
  get [k]() { return 1; }
  set a(v: number) {}
}
```

```ts
// g5.ts  (option "getters")
const k = "a";
export class A {
  readonly a = 1;
  constructor() { this[k] = 2; }
}
```

- ESLint: nothing on either.
- translation: `f1.ts:3` preferFieldStyle, `g5.ts:3` preferGetterStyle.

**Defect 3 (miss and extra): which method is the constructor.** `ctor_write` takes a `method_definition` whose name
reads `constructor` as written. ESTree's `kind: "constructor"` is a non-static method keyed `constructor` or
`"constructor"`: a `static constructor()` is a plain static method, and `"constructor"()` is the constructor.
`tsx/ts-a.rerail`'s `ts_ctor` (or `eslint-a-functions`' `ctor`) already says this.

```ts
// g1.ts  (option "getters")
export class A {
  readonly a = 1;
  static constructor() { this.a = 2; }
}
```

```ts
// g2.ts  (option "getters")
export class A {
  readonly a = 1;
  "constructor"() { this.a = 2; }
}
```

- ESLint: `g1.ts:2:12` preferGetterStyle; nothing on g2.
- translation: nothing on g1; `g2.ts:2` preferGetterStyle.

**Defect 4 (miss): an assignment in a class nested in the constructor counts for the outer class.** The plugin adds
the name to the excludes of the innermost class body being visited (`propertiesInfoStack` top), which for
`this.a` in `B`'s field initialiser is `B`'s. Its walk to the nearest function skips the field, so the `this` passes
the constructor test, but the exclusion lands on `B`. `ctor_write` ties it to the constructor's own class body, so
`A.a` is excluded.

```ts
// g3.ts  (option "getters")
export class A {
  readonly a = 1;
  constructor() {
    class B { b = (this.a = 2); }
  }
}
```

- ESLint: `g3.ts:2:12` preferGetterStyle.
- translation: nothing.

**Defect 5 (miss): object destructuring is no assignee.** `isAssignee`'s `Property` case requires the property's
parent to be an `ObjectExpression`; in `({ a: this.a } = o)` it is an `ObjectPattern`, so the plugin does not count it
(arrays and rest elements do count). `assignee` accepts `pair_pattern`.

```ts
// g4.ts  (option "getters")
export class A {
  readonly a = 1;
  constructor(o: { a: number }) { ({ a: this.a } = o); }
}
```

- ESLint: `g4.ts:2:12` preferGetterStyle.
- translation: nothing.

**Defect 6 (extra): `accessor` fields.** `readonly accessor e = 1` is an `AccessorProperty`, not a
`PropertyDefinition`; the plugin does not look at it. `literal_field` takes any `public_field_definition`.

```ts
// g6.ts  (option "getters"; the rest of the file agrees)
export abstract class B {
  readonly accessor e = 1;
}
```

- ESLint: nothing.
- translation: `g6.ts:2` preferGetterStyle.

## typescript-eslint-class-methods-use-this

Agreed: methods, getters, generator and async methods, `#p`, overload signatures (only the implementation),
abstract methods, `accessor` arrow fields (reported at `=>`), `super`, arrows inside methods, object-literal methods
inside a method, `this` in a nested class's computed key or field, `foo(a = this.x)`, `this` types (`a: this`,
`(): this`) are no use and `typeof this.x` is one, a field function under `as`, `"constructor"()`, static members;
`exceptMethods` (`#p`, `"bar"`, `1`, a computed `["foo"]` always checked), `enforceForClassFields: false`,
`ignoreOverrideMethods`, and both `ignoreClassesThatImplementAnInterface` values.

**Defect (wrong line): a decorated field is reported at its first decorator.** `getFunctionHeadLoc` starts a
`PropertyDefinition`'s head at the first token after its last decorator. `fn_head(f, h) :- fn_holder(f, h)` reports at
the `public_field_definition`, which in tree-sitter holds its decorators (a method's decorators belong to the class
body, so decorated methods agree). `ts-a.rerail`'s `member_head` skips them.

```ts
// cm1.ts
declare function dec(...a: any[]): any;
export class A {
  @dec()
  bar = () => {};
  @dec() @dec()
  baz = function () {};
}
```

- ESLint: `cm1.ts:4:3`, `cm1.ts:6:3`.
- translation: `cm1.ts:3`, `cm1.ts:5`.

## typescript-eslint-consistent-indexed-object-style

`"record"`: a `TSInterfaceDeclaration` or `TSTypeLiteral` whose one member is an index signature with an identifier
parameter, a key type and a value type, unless `isDeeplyReferencingType` finds its declaration's variable; a
`TSMappedType` unless its key has a type reference, its constraint is an unparenthesised `keyof`, or it is circular.
`"index-signature"`: a `TSTypeReference` named `Record` with two arguments. Agreed: `interface A { [k: string]: A }`,
`A[]` stops the search, `extends` (reported), indexed access, a union with `E[]`, `Array<F>`, a property signature
stops, a parameter annotation, `infer`, `as` clauses, `(keyof T)`, `-readonly`/`-?`; `Record` under `extends`,
`implements`, as a heritage type argument, `globalThis.Record`, one argument.

**Defect 1 (miss): type names are matched by text, not by binding.** The plugin follows an identifier to its
variable with `findVariable(superVar.scope, name)`, which looks in the declaration's scope and the scopes around it,
and compares references by the variable's own reference list. `named_decl` follows every alias or interface of that
name anywhere in the file, and `deep` matches any `type_identifier` with the text. A same-named alias in an inner
scope, or a type parameter of that name, makes the translation call the type circular.

```ts
// i1.ts
export type Bar = number;
export type Foo = { [k: string]: Bar };
export function f() {
  type Bar = Foo;
  return 0 as unknown as Bar;
}
```

```ts
// i5.ts
export type Foo = { [k: string]: Bar<string> };
type Bar<Foo> = Foo;
```

- ESLint: `i1.ts:2:19`, `i5.ts:1:19` preferRecord.
- translation: nothing.

**Defect 2 (extra): a type literal in a function or constructor type's type parameters.** `findParentDeclaration`
stops only at a `TSTypeAnnotation`, so from a constraint `<T extends { … }>` it climbs through `TSFunctionType` to the
alias, and the literal's reference to the alias makes it circular. `decl_stop` stops at every `function_type` and
`constructor_type`, which is right for their parameters and return type but not for their type parameters.

```ts
// i2.ts
export type Foo = <T extends { [k: string]: Foo }>(a: T) => void;
export type Foo2 = new <T extends { [k: string]: Foo2 }>(a: T) => void;
```

- ESLint: nothing.
- translation: `i2.ts:1`, `i2.ts:2` preferRecord.

**Defect 3 (miss): a mapped type's key shadowed in its value.** The plugin asks whether the key's own type variable
has a type reference. `uses_key` matches any `type_identifier` with the key's text, including one bound by an inner
type parameter.

```ts
// i3.ts
export type M = { [K in "a" | "b"]: <K>(x: K) => K };
```

- ESLint: `i3.ts:1:17` preferRecord.
- translation: nothing.

## typescript-eslint-consistent-type-definitions

ok. `"interface"`: aliases of an object type literal, parenthesised, empty, with call, construct, accessor and
index members, with variance annotations and generic defaults, in `declare global`, a namespace and a function; not a
mapped type (`+readonly`/`+?`, `-readonly`/`-?`, `as`, a trailing `;`, a comment, no value, `?`, multi-line), an
intersection or a constructor type. `"type"`: every interface, including `export default`, `declare global`,
`declare`, variance annotations. A file TypeScript cannot parse (`{ [K in X]: 1, }`) gets nothing from either.

## typescript-eslint-explicit-member-accessibility

Agreed with no options, with `accessibility: "no-public"`, and with overrides (`constructors: "off"`, `accessors:
"explicit"`, `properties: "off"`, `parameterProperties: "explicit"`) and `ignoredMethodNames`: fields, decorated
fields and parameter properties (head after the decorator), `#private` members, parameter properties with
`readonly`/`override`, methods, accessors, abstract methods, fields and accessors, `accessor` fields, overloads,
`declare` and `static` fields, computed keys (`["y"]`, `[Symbol.iterator]`), class expressions, object literals (not
members), a string-keyed or static `constructor`, `public` on its own line (a field named `public`), number keys
(`0x10` → `"16"`, `1.50`, `1e3`), escapes (`"\u0061b"` → `ab`).

**Minor (extra): a string key starting with a non-ASCII letter.** `getNameFromMember` quotes a name only when
`requiresQuoting` (TypeScript's `isIdentifierStart`/`isIdentifierPart`) says so; `member_key_name` decides with
`[A-Za-z_$][A-Za-z0-9_$]*`, so `"ä"` is the name `"ä"` instead of `ä` and `ignoredMethodNames: ["ä"]` does not skip it.
The message's name differs the same way.

```ts
// p1.ts  (option {"accessibility": "no-public", "ignoredMethodNames": ["ä"]})
export class A {
  public "ä"() {}
}
```

- ESLint: nothing.
- translation: `p1.ts:2` "Public accessibility modifier on method definition "ä"."

## The author's result file

Read after the findings above. Both `different` entries say the translation is right; the tool agrees on each key:

| Rule | Key | typescript-eslint 8.74.0 | translation |
|---|---|---|---|
| consistent-indexed-object-style | `fixtures/typescript-eslint/consistent-indexed-object-style/088.ts` | `2:1` preferRecord | `088.ts:2` |
| consistent-type-definitions | `…/consistent-indexed-object-style/037.ts` | `1:6` interfaceOverType | `037.ts:1` |
| consistent-type-definitions | `…/consistent-indexed-object-style/038.ts` | `1:6` interfaceOverType | `038.ts:1` |

The three `same` verdicts hold for the corpora; the defects above are in cases the corpora do not have. The NOTES'
reason for `has_setter`'s text pairing ("8.70 does") does not hold for 8.74.0, the version the NOTES checked
against (class-literal-property-style defect 1).
