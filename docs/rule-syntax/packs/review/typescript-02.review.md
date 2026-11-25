# Review: typescript batch 02

Branch reviewed: `tbelanger/pack-translations-typescript-02` (6cb469fbd9). The tool is the pack's pinned runtime,
`scripts/pack-data/runtimes/typescript-eslint-8.70.1/package.json` installed as is: typescript-eslint 8.70.1 on ESLint
10.8.1 and TypeScript 6.0.3, with `@typescript-eslint/parser` and the rule at `"error"` plus the options named. (The
five rules' code is the same in 8.74.0 but for a comment; ESLint 10's base `max-params` and `no-magic-numbers` are the
ones that ran.) The translation side is `~/pt/bin/rerail-engine run-policies` (`run-rr` with an options file where
options are named) with the branch's `docs/rule-syntax/lib`. Everything ran on pt-w01 in `/tmp/rv-ts02`: case files
in `/tmp/rv-ts02/es2/c/*`, the minimal counterexamples below in `/tmp/rv-ts02/es2/c/rev/*`, runners `es.sh`,
`rp.sh`, `cmp.sh` and `d.sh` (the `file:line` keys whose finding counts differ). "Tool" means typescript-eslint's
output, "translation" the `.rerail`'s. The translation reports lines only, so findings are compared by `file:line`
and count.

`ptcheck --ref tbelanger/pack-translations-typescript-02 -- --pack typescript --rules <the five>` (on pt-w07, exit 0,
ref 6cb469fbd9): four `same`, no-invalid-void-type `different` (fixtures 1 translation-only, rerail-typescript 113
translation-only). Its `problems` lists max-params, no-confusing-non-null-assertion, no-magic-numbers and no-shadow as
"reports the same findings as its original now; remove it from different", and no-invalid-void-type as "reports
different findings and is not listed in different": `docs/rule-syntax/packs/typescript/equivalence.json` on the branch
is out of date for all five, and the coordinator should update it. "Same as the original" says little here: the
corpora agree with the JSON originals, and several defects below are shared with them.

| rule | verdict |
|---|---|
| typescript-eslint-max-params | defects 1 (decorated methods on the wrong line), 2 (`export default` signature), 3 (`this: (void)`), 4 (`maximum: 0`) |
| typescript-eslint-no-confusing-non-null-assertion | ok (findings); messages are not 8.70.1's |
| typescript-eslint-no-invalid-void-type | defects 5 (abstract properties), 6 (named tuple members), 7 (tagged templates), 8 (object methods in overloaded functions), 9 (`T<(void)>` in a union), 10 (allow-list names with spaces) |
| typescript-eslint-no-magic-numbers | defects 11 (`ignore` entries that are floats: `-0`, `1e3`, `2.0`), 12 (`readonly [5]` key) |
| typescript-eslint-no-shadow | defects 13 (named function or class expression wrapped in a call: `forwardRef(function Button…)`), 14 (parameter default), 15 (static overload type parameter), 16 (conditional check type with `hoist: "all"`) |

Defects 1 and 13 are the ones real code meets: a NestJS/Angular controller method with its decorator on the line
above, and React's `const Button = forwardRef(function Button() …)` / `memo(function Card() …)`.

## typescript-eslint-max-params

### 1. A method with a decorator on the line above is reported one line too low

typescript-eslint reports at `getFunctionHeadLoc`, which starts at the MethodDefinition, and a TSESTree
MethodDefinition starts at its first decorator. tree-sitter puts a class member's decorators beside it in the class
body, so `fn_head` (the holder, `method_definition`) starts after them. A field (`@dec f = (a, b, c, d) => {}`) is
fine: tree-sitter keeps its decorator inside `public_field_definition`.

```ts
class Controller {          // rev/mp/mp1.ts
  @Post()
  create(a: string, b: string, c: string, d: string) {}
}
```
```
tool:        mp1.ts:2:3 @typescript-eslint/max-params Method 'create' has too many parameters (4). Maximum allowed is 3.
translation: mp1.ts:3 Method 'create' has too many parameters (4). Maximum allowed is 3; …
```

`static` after the decorator (`@dec\n static\n m(a, b, c, d) {}`) gives the same (tool line 1 of the member, translation
the line of `m`'s start). The cause is `tsx/eslint-a-functions.rerail`'s `fn_head`, so `eslint-max-params`,
`eslint-require-await` and `eslint-require-yield` share it wherever their parser accepts decorators [INFERENCE: not run].
`typescript-eslint-class-methods-use-this` uses `fn_head` too but was checked and reports at the key on both sides.

### 2. An `export default function (…): T;` signature is missed

```ts
export default function (a: string, b: string, c: string, d: string): void;   // rev/mp/mp2.ts:1
export default function (a: string, b: string, c: string, d: string) {}
```
```
tool:        mp2.ts:1:16 … Function has too many parameters (4). Maximum allowed is 3.
             mp2.ts:2:16 … Function has too many parameters (4). Maximum allowed is 3.
translation: mp2.ts:2 Function has too many parameters (4). …
```

The signature is a TSDeclareFunction without an id. `checked`'s `function_signature` clause requires a `name` field.

### 3. `this: (void)` is counted

TSESTree drops the parentheses, so the `this` parameter's annotation is TSVoidKeyword and it is not counted.
`void_type` (eslint-a-functions) wants the annotation's first child to be `predefined_type` and gets the
`parenthesized_type`.

```ts
function f(this: (void), a: string, b: string, c: string) {}   // rev/mp/mp3.ts
```
```
tool:        (nothing)
translation: mp3.ts:1 Function 'f' has too many parameters (4). Maximum allowed is 3; …
```

### 4. `maximum: 0` is not ESLint's `option.maximum || option.max`

ESLint takes `numParams = option.maximum || option.max`, so a `maximum` of 0 falls through to `max`, or to
`undefined` without one (then nothing is ever reported). `most` takes `maximum` whenever it is set.

```ts
function f(a: string) {}                                                          // rev/mpo/mp4.ts
function g(a: string, b: string, c: string, d: string, e: string, f: string) {}
```
```
{ maximum: 0 }          tool: (nothing)                 translation: mp4.ts:1 (1 > 0), mp4.ts:2 (6 > 0)
{ maximum: 0, max: 5 }  tool: mp4.ts:2:1 … (6) … is 5   translation: mp4.ts:1, mp4.ts:2 "Maximum allowed is 0"
```

`maximum` is deprecated and 0 is an odd limit; minor.

Checked and agreeing (`es2/c/mp`, 31 lines): overload signatures, `declare function` (alone, exported, in
`declare module`, `declare namespace`, `declare global`), function types, interface method/call/construct signatures
and class overloads and abstract methods (not checked on either side), constructor parameter properties, `this: void`
and `this: Window`, object and class methods, private static async methods, generic arrows, getters and setters,
`countVoidThis: true`, `max`, names in the messages.

## typescript-eslint-no-confusing-non-null-assertion: ok

33 cases (`es2/c/cn`): the four operators and `=`, parentheses around the left (`(a!)`, `(((a!)))`, `(a! /* c */)`),
comments between `!` and the operator, `1 + b! === 3`, `a == b! == c`, `x = a! = b`, `(a!)! == b`, `a!! == b`,
`<T>a! == b`, `a?.b! == c`, a line break before `==`, and the non-cases (`a!.x == b`, `!a == b`, `a!==b`, `a! !== b`,
`a! += b`, `for (a! in b)`, `a[b!] == c`, line comments ending in `!` after the left side). All agree but
`a as any! == b` (tool only), which TypeScript rejects (TS17020), so it is left out.

Not a finding difference, but the messages are not 8.70.1's: the translation says `equal test like "a! = b", which
looks very similar to not equal "a != b"` where 8.70.1 says `Confusing combination of non-null assertion and
assignment like `a! = b`, which looks very similar to `a != b`.` (and `equality test like `a! == b`` for `==`).
8.70.1 also offers "Remove unnecessary non-null assertion" when the left side is the assertion; the translation has
only the wrap fix.

## typescript-eslint-no-invalid-void-type

### 5. Abstract properties are reported

TSESTree has TSAbstractPropertyDefinition and TSAbstractAccessorProperty, which are not among the rule's
`invalidGrandParents` (PropertyDefinition, AccessorProperty, TSPropertySignature, CallExpression, Identifier), so
their `void` annotation is valid. tree-sitter makes both a `public_field_definition`, which `bad_holder` lists.

```ts
abstract class A {            // rev/vt/vt1.ts
  abstract p: void;
  abstract accessor q: void;
}
```
```
tool:        (nothing)
translation: vt1.ts:2, vt1.ts:3 void is only valid as a return type or generic type argument.
```

(`declare q: void`, `accessor r: void`, `static`, `readonly w?: void` are reported on both sides.)

### 6. Named tuple members are missed

A TSNamedTupleMember holds its type directly (no TSTypeAnnotation), so the `void` is reported. tree-sitter gives the
member a `type_annotation`, and `valid_place`'s first clause takes any `type_annotation` whose holder is not a
`bad_holder`.

```ts
type Args = [        // rev/vt/vt2.ts
  name: void,
  rest?: void,
];
```
```
tool:        vt2.ts:2:9, vt2.ts:3:10 void is only valid as a return type or generic type argument.
translation: (nothing)
```

(`[void, …]` and `...c: void[]` agree.)

### 7. A tagged template's type argument is reported

`tag<void>\`x\`` is a TaggedTemplateExpression in TSESTree, not among `invalidGrandParents`, so the type argument is
valid. In tree-sitter it is a `call_expression`, which `value_arguments` treats as a call.

```ts
declare const tag: any;   // rev/vt/vt3.ts
tag<void>`x`;
```
```
tool:        (nothing)
translation: vt3.ts:2 void is only valid as a return type or generic type argument.
```

### 8. A union in an object method inside an overloaded function is reported

`getParentFunctionDeclarationNode` climbs to the nearest FunctionDeclaration or MethodDefinition; an object literal's
method is a Property, so it climbs past it to `f`, which has an overload. `overload_host` includes every
`method_definition`, object literal methods too, so `nearest` stops at `m`, which has none.

```ts
function f(): void;                         // rev/vt/vt4.ts
function f(x?: unknown) {
  const o = { m(a: string | void) {} };
}
```
```
tool:        (nothing)
translation: vt4.ts:3 void is not valid as a constituent in a union type
```

### 9. A union member `T<(void)>` does not make the union valid

`isValidUnionType` looks at the member's `typeArguments.params` types, without parentheses (TSESTree has none).
`void_generic` takes only a direct `void` child of `type_arguments`.

```ts
type T = void | Promise<(void)>;   // rev/vt/vt5.ts
```
```
tool:        (nothing)
translation: vt5.ts:1 void is not valid as a constituent in a union type
```

With `allowInGenericTypeArguments: false` the tool reports only the inner `void` and the translation both. Minor.

### 10. Allow-list names are compared without removing their spaces

8.70.1 removes spaces from both the type's name and each list entry (`s.replaceAll(' ', '')`); `generic_arg` removes
them from the name only.

```ts
declare namespace A { interface B<T> {} }   // rev/vto/vt6.ts, allowInGenericTypeArguments: ["A . B"]
type T = A.B<void>;
```
```
tool:        (nothing)
translation: vt6.ts:2 A.B may not have void as a type argument.
```

Checked and agreeing (`es2/c/vt*`, 45 + 15 + 17 lines, under the defaults, `allowInGenericTypeArguments` false and a
list, `allowAsThisParameter` true and with generics false): variables, parameters (plain, optional, defaulted,
`this`, parameter properties; destructured and rest ones are valid on both sides), catch parameters, index
signatures, mapped types, type predicates, calls and optional calls, `new`, instantiation expressions, `typeof
f<void>`, `import("x").Y<void>`, `extends`/`implements` arguments, unions with `never`, `T<void>`, `undefined`,
parenthesised and nested unions, `<T = void>`, `<T extends void = void>`, `<T = (void)>`, overloaded functions and
methods (the `export default` pair too), return types of functions, accessors, call/construct signatures, function
and constructor types, arrays, `<void>x`, `as void`, `satisfies void`, `keyof void`, `Foo<void | string>`, and the
messages.

## typescript-eslint-no-magic-numbers

### 11. `ignore` entries that are floats never match

The options reach the rule as text: a JSON `-0` arrives as `"-0.0"`, `1e3` as `"1000.0"`, `2.0` as `"2.0"`
(checked with a scratch policy printing `option_item("ignore", _, i)`), while a literal's `value` is `0`, `1000`, `2`.
So `ignored("-0", "0")` never fires, and a float-written entry never equals any literal. ESLint compares numbers.

```ts
declare function f(n: number): void;   // rev/mno/mn1.ts, ignore: [-0, 1e3, 2.0]
f(0);
f(1000);
f(2);
```
```
tool:        (nothing)
translation: mn1.ts:2 No magic number: 0.  mn1.ts:3 No magic number: 1000.  mn1.ts:4 No magic number: 2.
```

`ignore: [0]` (and `-0`/`0.0`/`0x0` literals against it), `[16]` against `0x10`, `[1.5]` against `1.50` and BigInt
entries all agree. The fix is to compare the entry's number, or to render whole floats without `.0` where options
become text (an engine change that touches every rule reading a number list).

### 12. `readonly [5] = …`: the key is reported under ignoreReadonlyClassProperties

`isParentTSReadonlyPropertyDefinition` asks only whether the literal's parent is a readonly PropertyDefinition, so a
computed numeric key counts as well as the value. `ts_place`'s `ignoreReadonlyClassProperties` clause requires the
value.

```ts
class K {              // rev/mn/mn2.ts, ignoreReadonlyClassProperties: true
  readonly [19] = 1;
}
```
```
tool:        (nothing)
translation: mn2.ts:2 No magic number: 19.
```

Without the option both report `19` and `1`. Minor.

Checked and agreeing (`es2/c/mn*`, 59 lines under the defaults, all seven `ignore…` options, `detectObjects` +
`enforceConst` + an ignore list, `ignoreArrayIndexes` + `[0]`): `as const`, `<number>5`, array indexes in every
notation (`1.0`, `1e1`, `0x10`, `1_0`, `-0`, `0n`, `-0n`, `4294967294`/`4294967295`, `a?.[2]`, `a[(3)]`), raws
(`.5`, `5.`, `0b11`, `0o7`, `+2n`), parseInt radixes (`Number?.parseInt`, `Number["parseInt"]`, `(parseInt)`,
`parseInt?.()`), assignments (`a! = 5`, `(a) = 6`, `||=`, `??=`, `+=`), defaults, class fields (readonly, static,
`accessor`, method keys), object keys and methods, destructuring keys, enum members (`-27`, `+28` reported as `28`,
`1 << 29`, `(30)`), literal types in aliases, nested unions, intersections, index types (unions and intersections),
`-43` types, other literal types, `~48`, `for (let i = 0; …)` with enforceConst, `using`.

## typescript-eslint-no-shadow

### 13. A named function or class expression wrapped in a call is not reported

8.70.1's `isOnInitializer` allows the inner name only when the expression is the outer declarator's initialiser
itself, or reached through `||`/`&&`/`??` and the branches of `?:` (`unwrapExpression`): it "allows `var a = function
a() {}` but reports `var a = wrap(function a() {})`". `on_initializer` allows any function or class expression whose
name lies anywhere inside the outer declaration, as the rule did before.

```tsx
import { forwardRef, memo } from "react";                                   // rev/sh/sh1.tsx
export const Button = forwardRef(function Button() { return null; });
export const Card = memo(function Card() { return null; });
export const Panel = wrap(class Panel {});
declare function wrap<T>(t: T): T;
```
```
tool:        sh1.tsx:2:43 'Button' is already declared in the upper scope on line 2 column 14.
             sh1.tsx:3:35 'Card' is already declared in the upper scope on line 3 column 14.
             sh1.tsx:4:33 'Panel' is already declared in the upper scope on line 4 column 14.
translation: (nothing)
```

This is the usual way React components get a display name, so real code meets it often. (`cond ? function w() {} :
null`, `x || function w() {}`, `(function w() {})` and `class C {}` directly are allowed on both sides.)

### 14. A function expression named like the parameter whose default it is

`isOnInitializer` also accepts an AssignmentPattern's right side as the initialiser. `on_initializer` takes only
`var`/`let`/`const`/`using` outer declarations.

```ts
export function f(p = function p() {}) {}   // rev/sh/sh2.ts
```
```
tool:        (nothing)
translation: sh2.ts:1 Rename 'p'; it shadows a declaration of the same name in an enclosing scope.
```

### 15. A static method's overload signature: its type parameter is reported

`isGenericOfStaticMethod` accepts a FunctionExpression or a TSEmptyBodyFunctionExpression (an overload signature)
under a static MethodDefinition. The third `ignored` clause wants a `method_definition`; tree-sitter makes the
signature a `method_signature`.

```ts
export class K<T> {        // rev/sh/sh3.ts
  static m<T>(): void;
  static m<T>(x?: T) {}
}
```
```
tool:        (nothing)
translation: sh3.ts:2 Rename 'T'; it shadows a declaration of the same name in an enclosing scope.
```

### 16. A conditional type's check type is inside its scope (seen with `hoist: "all"`)

The scope manager visits every part of a conditional type but its false branch inside the conditionalType scope
(`this.visitChildren(node, ['falseType'])`), the check type included. `blind` puts the check type outside, so a type
parameter there never meets an `infer` of the same name. Under the default hoist the inner one comes first and the
TDZ check hides it, so this shows only with `hoist: "all"`.

```ts
export type C<V> = (<W>(w: W) => W) extends (a: infer W) => unknown ? V : 0;   // rev/sho/sh4.ts, hoist: "all"
```
```
tool:        sh4.ts:1:22 'W' is already declared in the upper scope on line 1 column 55.
translation: (nothing)
```

The notes' library request (`scope_up` to pass over a `conditional_type` from its `left` or `alternative`) should
name the `alternative` only; with `left` too the library would carry this defect to `resolves`.

Checked and agreeing (`es2/c/sh*`, under the defaults, `builtinGlobals`, `ignoreOnInitialization`,
`ignoreTypeValueShadow: false`, the five hoist values, `ignoreFunctionTypeParameterNameValueShadow: false`, `allow`):
parameters, class expression names inside the class, a static method's type parameter (with a body) and an instance
method's, imports and type imports, type/value pairs, function type and `declare function` parameters, class
overload parameters, TDZ against `const`/function/type/overloads/interface/class/enum, enum members, namespaces,
`declare global`, catch, for, switch, static blocks, field initialisers, mapped and conditional (`infer`) types,
`export default function`/`class` names, `xs.map(z => z)` and `xs.map(function (z) …)` with and without
ignoreOnInitialization, `.d.ts` `declare` declarations, the ES 2025/2026 globals (`Iterator`, `Float16Array`,
`AsyncDisposableStack`, `SuppressedError`, `Temporal`), `/* global */` comments with and without `off`, `.cts` and
`.mts`, parameter decorators and index signatures.

## The result file

`typescript-02.result.json` has one `different` entry, no-invalid-void-type, right `translation`. Confirmed with
8.70.1 (the notes used 8.74.0 on ESLint 9.39.5, not the pack's pinned runtime; the rule's code is the same): the
fixture key `typescript-eslint/no-invalid-void-type/041.ts:1` (`functionGeneric<void>(undefined);`) is reported by
the tool (`041.ts:1:17 … void is only valid as a return type or generic type argument.`) and by the translation; on
the whole rerail-typescript corpus (copied from `~/pt/corpora/real/rerail-typescript`) the tool reports 114 lines and
the translation the same 114 `file:line` keys, among them the sample `src/daemon/daemon.ts:176`. The right side holds.

The other four on the same corpus, tool against translation, `.ts`/`.tsx`/`.mts`/`.cts` keys (the tool was not
configured for the corpus's `.mjs` files, which no-magic-numbers' and no-shadow's `**/*` scope takes in): max-params
342 = 342, no-confusing-non-null-assertion 0 = 0, no-shadow the same keys, no-magic-numbers the same keys but
`test/support/build/script-bundle.ts:21` (3) and `:33` (1), tool only. Those are the two the notes could not explain:
`run-policies` skips every file under a `build/` or `dist/` directory, with no `.gitignore` anywhere (`let x: void;`
in `build/a.ts`, `dist/a.ts`, `out/a.ts`, `src/a.ts`: no-invalid-void-type reports only `out/` and `src/`; the same
file reports as `script-bundle.ts` or `q/script-bundle.ts`). That is the engine's file selection, the same for the
original, not the rule.
