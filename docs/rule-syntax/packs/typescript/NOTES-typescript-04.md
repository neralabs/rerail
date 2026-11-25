# typescript-04: prefer-for-of, unified-signatures, vitest no-disabled-tests, no-focused-tests

Ground truth: ESLint 9.39 with typescript-eslint 8.74 and @vitest/eslint-plugin 1.6.28 (npx, on a box). All four
rules are now `same` as their JSON originals on every corpus (examples, fixtures, rerail-typescript). In every
difference found, the original was right and the translation was fixed.

## Engine gap: `earlier` has no rows under run-policies

`earlier` (and `visit_index`, core.rerail `.input`) yield no rows for TypeScript under `run-policies`, as react-04
found. Three of these rules ordered siblings by `earlier`; now they use `precedes`, as every node they order is a
child of one parent. Until the engine gives `earlier` rows, a rule ordering siblings should use `precedes`.

## typescript-eslint-unified-signatures

Baseline: the translation reported nothing (129 fixture findings original-only). `pair(a, b)` required
`earlier(a, b)`, so no pair formed. Now `member_of(g, s, t)` names the child of the container holding each
signature (the signature, or its `declare` / `export` statement) and `pair` orders them by `precedes`.
Counterexample: `function f(x: number): void; function f(x: string): void; function f(x: any) {}`: ESLint reports
line 2; the translation reported nothing.

## vitest-no-focused-tests, vitest-no-disabled-tests

Baseline: one translation-only finding each, on `no-focused-tests/rerail-mode-option-overridden.test.ts`:

```ts
test('a', { only: true, only() {} }, () => {});            // ESLint: nothing (the method is the last `only`)
test('b', { skip: true, get skip() { return false; } }, …); // ESLint: nothing (the getter is the last `skip`)
```

The plugin's `findVitestModeProperty` takes the last Property named for the mode and wants its value the literal
`true`. `tsx/ts-b-vitest`'s `mode_option` says so, but its `later_entry` finds "later" by `earlier`, so under
run-policies every `x: true` counted. The two rules share the fix, in the new `lib/tsx/typescript-04.rerail`:
`set_mode_option(c, x, k)`, `mode_option` with no later sibling property named `x` by `precedes`, the shorthand
`{ x }` included (a Property in ESTree; ESLint: `test("a", { only: true, only }, fn)` reports nothing, and neither
does the translation). Checked ad hoc against ESLint: `{ only: false, only: true }` and
`{ skip: false, "skip": true }` reported, `{ skip: true, set skip(v) {} }` and `{ skip: true, skip }` not; the
translation agrees.

Library change proposed: in `tsx/ts-b-vitest.rerail`, `later_entry` by `precedes(e, f)` instead of `earlier(e, f)`,
and `option_entry` taking `shorthand_property_identifier`; then `mode_option` is right and `lib/tsx/typescript-04`
can go.

## typescript-eslint-prefer-for-of

Baseline: one translation-only finding, `prefer-for-of/044.ts:2`:

```ts
for (let i = 0; i < arr.length; i++) {
  ({ foo: arr[i] }) = { foo: 0 };   // ESLint: nothing
}
```

typescript-estree keeps a parenthesized object literal on an assignment's left an ObjectExpression (no pattern),
and typescript-eslint's `isAssignee` counts a Property value of an ObjectExpression that is itself an assignee.
The translation now does too: `place` grows from `a[i]` to each object literal holding it as a pair value, and
`written_by` takes such a pair when its object is an assignee (recursively, through parentheses and TS casts).
Checked ad hoc against ESLint: `({ a: { b: arr[i] } }) = x` and `({ a: (arr[i]!) }) = x` not reported;
`({ a: [arr[i]] }) = x` (an ArrayExpression, which isAssignee does not follow) and `({ a: arr[i] } = x)` (an
ObjectPattern) reported; the translation agrees on all four.
