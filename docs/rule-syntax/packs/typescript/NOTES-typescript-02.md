# Notes: batch typescript-02 (five typescript-eslint rules made to report what their JSON originals report)

Rules: typescript-eslint-max-params, typescript-eslint-no-confusing-non-null-assertion,
typescript-eslint-no-invalid-void-type, typescript-eslint-no-magic-numbers, typescript-eslint-no-shadow. Each was
translated already and `different` at the start. Four are `same` now; no-invalid-void-type stays `different`, the
original being wrong (`translations:check --pack typescript --rules <id>`, corpora examples, fixtures,
rerail-typescript).

Ground truth: ESLint 9.39.5 with typescript-eslint 8.74.0 (`@typescript-eslint/parser`), default options, on each
counterexample and, for no-invalid-void-type and no-magic-numbers, on the whole rerail-typescript corpus. No library
file changed; no new library.

## Per rule

### typescript-eslint-max-params (1 fixture translation-only → same)

`class Foo { method(this: void, a, b, c) {} }` (fixture 006): ESLint reports nothing (`this: void` is not counted);
the translation reported "4 parameters", and did so for every function with `this: void`. `uncounted(f, 1)` and
`uncounted(f, 0) :- checked(f, …), !skipped_first(f)` both held, so the count was taken both ways. The cause is
engine behaviour E3 (below): `checked` reaches `fn_name_kind`, built on typed text, and the negation after it held.
The rule now counts `fn_param(f, p, _), !left_out(f, p)` inside the aggregate. Checked against ESLint:
`function f(this: void, a, b, c, d) {}` and `function g(this: Foo, a, b, c) {}` reported (4), `type T = (this:
void, a, b, c) => void` and `declare function h(this: void, a, b, c): void` not.

### typescript-eslint-no-confusing-non-null-assertion (3 fixtures original-only → same)

`a! = b;` (fixtures 014–016): ESLint reports it ("assignment like `a! = b`"); the translation never matched an
assignment, because a plain `assignment_expression` has no `op` fact. It now takes `=` from the kind. Checked against
ESLint: `x = a! = b` (the inner one), `a.b! = c`, `a!! = b`, `(a)! = b`, `a! /* x */ = b` reported; `(a!) = b`,
`for (a! in b) {}`, `x += a!` not.

### typescript-eslint-no-invalid-void-type (different; the translation is right)

The translation reports `void` as a type argument of a call: `functionGeneric<void>(undefined);` (fixture 041),
`Promise.withResolvers<void>()` (the 113 rerail-typescript keys: src/daemon/daemon.ts:176, src/engine/git.ts:423, …).
ESLint reports each ("void is only valid as a return type or generic type argument."): with the default
`allowInGenericTypeArguments: true` only a type reference's arguments may hold `void`, and a call's
`typeArguments` are no type reference. The original misses them. On rerail-typescript, ESLint's 114 findings and the
translation's are the same lines. Kept; recorded as `different`, right `translation`.

### typescript-eslint-no-magic-numbers (184 fixtures, 2553 rerail-typescript translation-only → same)

`const PROFILE_SCHEMA_VERSION = 1;`, `{ mode: 0o600 }`: ESLint reports neither (a variable's initialiser, a
property's value); the translation reported both, and every number held by a declarator or a property. Its clause
started with `signed(n, f, raw, _)`, built on `value`, and each negated tree test after it (`!kind(p,
"variable_declarator")`, `!named(f, p)`) held: engine behaviour E3. The rule now finds the number by tree facts
(`num(n, f)`) and reads its text and value (`signed`) last in each body; `listed_num(f)` stands for the ignore list
in negations. On rerail-typescript ESLint reports 5626 lines and the translation 5624, all among ESLint's; the two
left are test/support/build/script-bundle.ts:21 and :33, which the translation does not report in a run over the
whole corpus (cause not found) and does report when that file is run alone. Options checked against ESLint on one file
(`ignore: [1, 0]`, `ignoreArrayIndexes`, `enforceConst`): `f(1)`, `f(-0)`, `a[3]`, `const y = 5` fine; `f(-1)`,
`f(2)`, `enum E { A = 6 }`, `type T = 7 | 8` reported; `var x = 4` "must use 'const'". Same findings.

### typescript-eslint-no-shadow (1 fixture translation-only → same)

```ts
export type ArrayInput<Func> = Func extends (arg0: Array<infer T>) => any
  ? T[]
  : Func extends (...args: infer T) => any ? T : never;
```

(fixture 012) ESLint reports nothing; the translation reported the second `T`. typescript-eslint's scope manager
visits a conditional type's check type and false branch outside its scope (only `extends` and the true branch see
the `infer`s), and `lib/tsx/ts-b-scope`'s `scope_up` puts the inner conditional inside the outer one. The rule now
skips a conditional scope while looking up from inside its `left` or `alternative` (`blind`). Checked against ESLint:
`F extends (a: infer T) => any ? (F extends (b: infer T) => any ? T : never) : never` (true branch) and
`U extends Array<infer U> ? …` still reported; `F extends Array<infer T> ? T : F extends Set<infer T> ? T : never`
and `F extends Array<infer U> ? never : <U>(x: U) => U` not.

Library request: `lib/tsx/ts-b-scope.rerail`'s `scope_up(s, p)` should pass over a `conditional_type` p when s lies
in p's `left` or `alternative` field (as the scope manager's `TSConditionalType` visitor does); then `resolves` is
right there too, and no-shadow's `blind` can go.

## Engine behaviour met

- E3 (as NOTES-javascript-02.md): after a body atom built on `value` (a relation whose rule reads `value(n, v)`,
  whether its value column is used or not), a negated tree test holds whatever the tree says. Probe, on
  `const A = 1;`: `lv(n, v) :- number_lit(n), value(n, v).` then `lv(f, _), child(p, f, _), !kind(p,
  "variable_declarator")` holds, and so does `lv(f, _), !kind(f, "number")`. Right: the same with `text` for
  `value`; `value(f, v)` directly in the body; the negation before `lv(f, _)`; a negated pure table (`!zero(v)`) or
  a comparison after it. Seen with `~/pt/bin/rerail-engine run-policies` on the fleet box (binary of 2026-11-25).
  Workaround in max-params and no-magic-numbers: negations first, typed-text atoms last.
