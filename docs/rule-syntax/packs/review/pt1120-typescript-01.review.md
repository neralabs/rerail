# pt1120-typescript-01 translation review

Ground truth: typescript-eslint 8.74.0 (ESLint 9.39.5, TypeScript 6.0.3, `@typescript-eslint/parser`, the rule alone
at `"error"` or with the options named per case). I read the rule at `main`
(`packages/eslint-plugin/src/rules/explicit-module-boundary-types.ts`, `util/explicitReturnTypeUtils.ts`) and its
docs, then ran my own cases with ESLint on pt-w01 and with `rerail-engine-1120 run-policies` (`run-rr` with an
options file where a case sets options) on `docs/rule-syntax` at `72295a2206`, on pt-w02 (pt-w01, pt-w07 and
ts-prisma-b have no `-1120` engine build). Translation lines are the finding's line (the engine's output gives no
column). `R` = "Add an explicit return type to this exported function.", `P` = "Give this exported function's
parameter an explicit type other than `any`."

`ptcheck --ref tbelanger/pt1120-typescript-01` gives `same` (fixtures 74 both, rerail-typescript 13 both). The suite
prints one problem: "reports the same findings as its original now; remove it from different" (a stale list entry
at that ref, not in this rule's files).

## `typescript-eslint-explicit-module-boundary-types`: defects

What agrees with ESLint, on my cases: `export = v` (the author's fix, also `export = /* c */ a`), `export default` of
an object (pairs, methods, a setter, a getter), class members (`accessor` fields, `#private`, `private`/`protected`
abstract methods, constructor overloads, parameter properties with and without defaults, abstract get/set
signatures, static fields), destructured and block-`var` declarations followed through `export { … }`, class
expressions, `export type { E }`, overloads with and without `allowOverloadFunctions`, the four boolean options off
(`allowArgumentsExplicitlyTypedAsAny` on), `export import A = N.B`, `export default a = …`.

### 1. A followed name is looked up in the module scope, not in the identifier's own scope

Upstream `followReference` reads `context.sourceCode.getScope(node).set.get(name)`: the variables of the
identifier's innermost scope only. In a class field's initializer that scope (class-field-initializer) declares
nothing, so nothing is followed; in a namespace or a function it is that namespace's or function's scope, so a local
of it is followed and a module-level name is not. The translation always looks the name up in `program`
(`module_site`) and drops any identifier inside a function (`in_function`).

`c02/a.ts` (false positive):

```ts
const a = (x) => x;
export class C {
  f = a;
  static g = [a];
}
```

```text
ESLint:      (nothing)
Translation: a.ts:1 R
             a.ts:1 P
```

`c03/a.ts` (false positive):

```ts
const a = (x) => x;
export namespace N {
  export const b = a;
}
```

```text
ESLint:      (nothing)
Translation: a.ts:1 R
             a.ts:1 P
```

`c04/a.ts` (false negative):

```ts
export namespace N {
  const a = (x) => x;
  export const b = a;
}
```

```text
ESLint:      a.ts:2:14 Argument 'x' should be typed.
             a.ts:2:17 Missing return type on function.
Translation: (nothing)
```

`c17/a.ts` (false negative; the array is a write to the exported `a`, reached through its reference, and `b` is
looked up in `init`'s scope):

```ts
let a;
function init() {
  const b = (x) => x;
  a = [b];
}
export { a, init };
```

```text
ESLint:      a.ts:2:1 Missing return type on function.
             a.ts:3:14 Argument 'x' should be typed.
             a.ts:3:17 Missing return type on function.
Translation: a.ts:2 R
```

### 2. Writes to a followed variable: only `x = v`

Upstream checks every reference's `writeExpr` (not the initialiser). typescript-eslint's scope manager gives one for
`x ??= v`, `x ||= v`, `x &&= v` and compound assignments (read-write reference, `writeExpr` = the right side), for
`x! = v` and `(x as T) = v` (it unwraps the left side), and for a destructuring assignment (`writeExpr` = the whole
right side, so every function in it is checked). `followed` takes only an `assignment_expression` whose left side
deparens to the identifier.

`c05/a.ts`:

```ts
let a: unknown;
a ??= (x) => x;
a ||= (y) => y;
a = (z) => z;
export { a };
```

```text
ESLint:      a.ts:2:8 Argument 'x' should be typed.        a.ts:2:11 Missing return type on function.
             a.ts:3:8 Argument 'y' should be typed.        a.ts:3:11 Missing return type on function.
             a.ts:4:6 Argument 'z' should be typed.        a.ts:4:9 Missing return type on function.
Translation: a.ts:4 R
             a.ts:4 P
```

`c06/a.ts`:

```ts
let a: unknown;
a! = (x) => x;
(a as any) = (y) => y;
export { a };
```

```text
ESLint:      a.ts:2:7 Argument 'x' should be typed.        a.ts:2:10 Missing return type on function.
             a.ts:3:15 Argument 'y' should be typed.       a.ts:3:18 Missing return type on function.
Translation: (nothing)
```

`c07/a.ts`:

```ts
let a: unknown;
[a] = [(x) => x];
({ a } = { a: (y) => y });
export { a };
```

```text
ESLint:      a.ts:2:9 Argument 'x' should be typed.        a.ts:2:12 Missing return type on function.
             a.ts:3:12 Missing return type on function.    a.ts:3:16 Argument 'y' should be typed.
Translation: (nothing)
```

### 3. `export declare class` is not entered

`export declare class A {}` is an `export_statement` whose declaration is an `ambient_declaration` around the
`class_declaration`; `checked` stops at the `ambient_declaration`. In ESTree the declaration is the
`ClassDeclaration` (`declare: true`), whose body-less methods are `TSEmptyBodyFunctionExpression`s and are checked.
Every exported class in a `.d.ts` file is written this way.

`c08/a.ts`:

```ts
export declare class A {
  m(a);
}
export declare abstract class B {
  abstract n(b);
}
```

```text
ESLint:      a.ts:2:4 Missing return type on function.    a.ts:2:5 Argument 'a' should be typed.
             a.ts:5:13 Missing return type on function.   a.ts:5:14 Argument 'b' should be typed.
Translation: (nothing)
```

### 4. A `this` parameter is never checked

`checkParameters` walks `node.params`, where TypeScript's `this` parameter is an `Identifier` named `this`: untyped it
is `missingArgType`, typed `any` it is `anyTypedArg`. `fn_param` drops it (`!kind(p, "this")`).

`c09/a.ts`:

```ts
export function f(this, a: number): void {}
export function g(this: any, a: number): void {}
```

```text
ESLint:      a.ts:1:19 Argument 'this' should be typed.
             a.ts:2:19 Argument 'this' should be typed with a non-any type.
Translation: (nothing)
```

### 5. A function checked before its returns are collected is not excused as higher-order

Upstream checks an export at the export node's `:exit`, while the returns `allowHigherOrderFunctions` reads
(`functionReturnsMap`) are collected as the traversal reaches each `return`. A function the export reaches through a
name, written below the export, is checked with no returns yet: `doesImmediatelyReturnFunctionExpression` is false
for any function with a block body, and its missing return type is reported. `immediate_fn` has no order. (An arrow
with an expression body is excused either way; the returned function is checked at `Program:exit` in both.)

`c11/a.ts`:

```ts
export { f };
function f() {
  return () => 1;
}
function g() {
  return () => 1;
}
export { g as h };
```

```text
ESLint:      a.ts:2:1 Missing return type on function.
             a.ts:3:13 Missing return type on function.
             a.ts:6:13 Missing return type on function.
Translation: a.ts:3 R
             a.ts:6 R
```

`c14b/a.ts` (a write below the export) and `c16/a.ts` (a class method):

```ts
export { a };
let a;
a = function () {
  return () => 1;
};
```

```ts
export { A };
class A {
  m() {
    return () => 1;
  }
}
```

```text
ESLint:      c14b a.ts:3:5 Missing return type on function.    c14b a.ts:4:13 Missing return type on function.
             c16  a.ts:3:3 Missing return type on function.    c16  a.ts:4:15 Missing return type on function.
Translation: c14b a.ts:4 R
             c16  a.ts:4 R
```

With the export below (`c14/a.ts`: the same with `export { a };` last) both report line 3 only.

### 6. `allowedNames`

`isAllowedName` takes a declarator's or a function declaration's `id`, or the static key
(`isStaticMemberAccessOfValue`) of a `MethodDefinition`, `TSAbstractMethodDefinition`, `PropertyDefinition`,
`AccessorProperty` or a `Property` with `method: true`. Three differences:

- an object literal's getter or setter is a `Property` with `method: false`, so its name is never allowed;
  `allowed_name` takes every `method_definition`;
- a named function expression held by a declarator is allowed by the declarator's name; `held_name` requires the
  function to have no name of its own;
- a computed key is resolved with `getStaticValue` in the member's scope, so `[k]` with `const k = "foo"` is
  `"foo"`; `key_static` resolves literals only.

`c10/a.ts` with `{"allowedNames": ["foo"]}`:

```ts
export const o = {
  get foo() {
    return 1;
  },
};
export const foo = function named(a) {
  return a;
};
```

```text
ESLint:      a.ts:2:3 Missing return type on function.
Translation: a.ts:6 R
             a.ts:6 P
```

`c22/a.ts` with `{"allowedNames": ["foo"]}`:

```ts
const k = "foo";
export class A {
  [k](a) {}
  ["foo"](b) {}
}
```

```text
ESLint:      (nothing)
Translation: a.ts:3 R
             a.ts:3 P
```

### Note (library, not this rule)

`return_head` puts an arrow's finding on `arrow_op`, a whole-line span, so two arrows on one line are one finding.
With `{"allowHigherOrderFunctions": false}`, `export const g = () => () => 1;` is two ESLint findings (1:21, 1:27)
and one translation finding on line 1; on two lines both report both. The suite compares line keys, so this shows
nowhere.

## Author's result file

`pt1120-typescript-01.result.json` lists the rule as `same`, with no `different` entry. The key the author fixed,
`typescript-eslint/explicit-module-boundary-types/059.ts:2` (`const foo = arg => arg; export = foo;`), checked with
the tool: ESLint reports 2:13 `Argument 'arg' should be typed.` and 2:17 `Missing return type on function.`; the
translation reports line 2 `P` and `R`. The suite's `same` holds on its corpora; the defects above are outside them.
