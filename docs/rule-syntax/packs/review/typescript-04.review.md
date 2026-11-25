# Review: typescript-04

Branch reviewed: `tbelanger/pack-translations-typescript-04` at `1557c1788f`. I read the four `.rerail` files, their JSON
originals and the libraries they include (`tsx/ts-b-scope`, `tsx/ts-b-vitest`, `tsx/typescript-04`), then the tools'
source (`dist/rules/prefer-for-of.js`, `dist/rules/unified-signatures.js`, `dist/util/isAssignee.js`,
`@vitest/eslint-plugin`'s `dist/index.cjs`: `parseVitestFnCall`, `resolveScope`, `findVitestModeProperty`,
`ValidVitestFnCallChains`) and their tests (`tests/rules/{prefer-for-of,unified-signatures}.test.ts` at v8.70.1,
`tests/{no-disabled-tests,no-focused-tests}.test.ts` at v1.6.27), before the author's NOTES and result file.

The tools ran on a fleet box: ESLint 9.39.5 with `typescript-eslint` 8.70.1 and `@vitest/eslint-plugin` 1.6.27 (the
versions `pack.json` covers), flat config with the four rules; every case was run again with `typescript-eslint` 8.74.0
and the plugin 1.6.28 (the author's versions) with the same output. Each translation ran with `rerail-engine
run-policies` (defaults) or `run-rr` with an options file, using this branch's `docs/rule-syntax/lib`. Each
counterexample below is a whole file. ESLint gives line:column, the engine gives the line.

| Rule | Verdict |
|---|---|
| `typescript-eslint-prefer-for-of` | 4 defects (computed `[length]`, optional chain anywhere in the length's object, `var` redeclared in the body, `0n`) |
| `typescript-eslint-unified-signatures` | 5 defects (computed identifier keys, `declare global`, `export default class`, mapped types, multi-line JSDoc under `ignoreOverloadsWithDifferentJSDoc`) |
| `vitest-no-disabled-tests` | 6 defects; one misses 3 findings in the `rerail-typescript` corpus (`context.skip()`) |
| `vitest-no-focused-tests` | 6 defects (test-context resolution, chain validity, a call used as an object, option forms) |

Messages were not compared beyond the key; fixes were not compared (`run-policies` reports no edits).

## typescript-eslint-prefer-for-of

The tool: `ForStatement:exit`; `init` a non-`const` `VariableDeclaration` with one declarator, an `Identifier` set to a
`Literal` whose value is `=== 0`; `test` a `BinaryExpression` `<` with the index on the left and on the right a
`MemberExpression` whose `property` is an `Identifier` named `length`; the update `++`, `+= 1`, `= i + 1` or
`= 1 + i`; then every reference of the declared variable inside the body has a `MemberExpression` parent (not on
`this`) whose property is the reference, whose object's text equals the length object's text, and which
`isAssignee` rejects. Agreed: parentheses everywhere, `0x0`, `0.0`, `0e0`, `-0`, `1.0`/`0x1` steps, `a!.length`,
`a?.[i]`, `<number>a[i]`, `a[i as number]`, shorthand `{ i }`, `typeof i`, shadowing in a nested block, closures,
`for (i in o)`, `[a[i] = 5] = []`, `({ k: a[i] } = o)`, `for (a[i] of b)`, `(a[i] as number)++`, `delete a[i]`,
`this.#length`, `a["length"]`.

**Defect 1 (miss): a computed `length`.** The tool tests `node.right.property` is an `Identifier` named `length`
without looking at `computed`, so `a[length]` qualifies. `counting` takes a `member_expression` only; the JSON's
`length` util also takes a `subscript_expression` whose index is the identifier `length`.

```ts
// p1.ts
declare const a: number[];
declare const length: number;
declare function use(x: unknown): void;
for (let i = 0; i < a[length]; i++) { use(a[i]); }
```

- ESLint: `p1.ts:4:1`.
- translation: nothing.

**Defect 2 (miss): an optional chain inside the length's object.** The tool rejects a `ChainExpression` as the
test's right side, which is only `a?.length` or `a?.b.length` (the chain continuing up to `.length`). `!has(m, _,
"optional_chain")` rejects any `?.` anywhere below the member, so a chain closed by parentheses or inside a call
argument is lost too. The JSON's `optional-chain` util follows `object`/`function`/`!` only, which is the
`ChainExpression` boundary.

```ts
// p2.ts
declare const ob: { b: number[] } | undefined;
declare function f(x?: unknown): number[];
declare function use(x: unknown): void;
for (let i = 0; i < (ob?.b).length; i++) { use((ob?.b)[i]); }
for (let i = 0; i < f(ob?.b).length; i++) { use(f(ob?.b)[i]); }
```

- ESLint: `p2.ts:4:1`, `p2.ts:5:1`.
- translation: nothing.
- `i < ob?.b.length` is not reported by either (checked).

**Defect 3 (extra): `var` index redeclared in the body.** eslint-scope gives a `var` redeclaration with an
initialiser (`var i = 0`) and a `for (var i in …)` head a write reference to the same variable; its parent is a
`VariableDeclarator` / `ForInStatement`, so the loop is not reported. `other_use` goes through `resolves`, and
`tsx/ts-b-scope`'s `use` excludes declaring identifiers, so these references are not seen. The JSON's second
`other-use` branch (`binds` … `variable_declarator` with a value, or `for_in_statement`) covers this, by its text
(the original was not run on these files).

```ts
// p3.ts
declare const a: number[];
declare const o: object;
declare function use(x: unknown): void;
export function g() {
  for (var i = 0; i < a.length; i++) { var i = 0; use(a[i]); }
  for (var i = 0; i < a.length; i++) { for (var i in o) use(a[i]); }
}
```

- ESLint: nothing.
- translation: `p3.ts:5`, `p3.ts:6`.

**Defect 4 (extra): a BigInt zero.** `isLiteral(init, 0)` is `node.value === 0`, false for `0n`. `literal(z, "0")`
takes the `number` node `0n`.

```ts
// p4.ts
declare const a: number[];
declare function use(x: unknown): void;
for (let i = 0n; i < a.length; i++) { use(a[i]); }
```

- ESLint: nothing.
- translation: `p4.ts:3`.

## typescript-eslint-unified-signatures

The tool: scopes are `Program`, `TSModuleBlock`, `TSTypeLiteral`, `TSInterfaceDeclaration` and `ClassDeclaration`
(with the last two's type parameters); an overload joins the innermost scope when its parent (or its export's) is
that scope's node; keys are `computed`+`static`+info, info `identifier_<name>` for any `Identifier` key (computed or
not), `private_identifier_<name>`, else `key.raw` (`undefined` for a non-literal expression). Agreed: the control
cases, `this: A`/`this: B`, `this: void`, same and different constraints, `T[]` with the class's `T`, a type literal in
a generic alias (no type parameters tracked), construct and call signatures, static apart, `["a"]` apart from `a`,
`[Symbol.iterator]` grouped with `[Symbol.asyncIterator]` (the tool's `undefined` key), getters skipped, `#m`,
`declare module "m"`, abstract methods, `export default function` overloads, a class expression skipped, three
overloads ("…the one on line N"), the report line of a class method whose parameters start on a later line,
`ignoreDifferentlyNamedParameters`, single-line JSDoc under `ignoreOverloadsWithDifferentJSDoc`.

**Defect 1 (extra): computed identifier keys all share one group.** `[sa]` is an `Identifier` key, so the tool keys
it `01identifier_sa` and `[sb]` `01identifier_sb`: two groups. `member_key` gives every non-literal computed key
`undefined`, so `[sa]` and `[sb]` pair (parentheses too: `[(sa)]` is still the `Identifier`). The opening comment's
"computed keys other than literals all share one" is wrong for identifiers; only other expressions (`[Symbol.x]`)
share `undefined`.

```ts
// u1.ts
declare const sa: unique symbol;
declare const sb: unique symbol;
export interface I {
  [sa](x: number): void;
  [sb](x: string): void;
}
```

- ESLint: nothing.
- translation: `u1.ts:5`.

**Defect 2 (miss): `declare global { … }`.** ESTree makes it a `TSModuleDeclaration` (kind `global`) whose body is a
`TSModuleBlock`, a scope. tree-sitter parses it as an `ambient_declaration` holding `global` and a
`statement_block`; `ts_module_body` takes `internal_module` and `module` only.

```ts
// u2.ts
export {};
declare global {
  function g(x: number): void;
  function g(x: string): void;
}
```

- ESLint: `u2.ts:4:14`.
- translation: nothing.

**Defect 3 (miss): an anonymous default-exported class.** `export default class { … }` is a `ClassDeclaration`
(with `id: null`) in ESTree, so it is a scope. `container` takes `class_declaration` and
`abstract_class_declaration`; tree-sitter gives the anonymous one another kind (`class`, I infer; not inspected).

```ts
// u3.ts
export default class {
  m(x: number): void;
  m(x: string): void;
  m(x: unknown) {}
}
```

- ESLint: `u3.ts:3:5`.
- translation: nothing.

**Defect 4 (extra): an outer type parameter reached through a mapped type.** `typeContainsTypeParameter` follows
`type.typeAnnotation ?? type.elementType`; `TSMappedType` has a `typeAnnotation` (its value type), so
`{ [K in "a"]: T }` uses the interface's `T`, the two overloads differ in that, and they are not unified. `reaches`
follows arrays, `readonly`, `keyof` and parentheses only.

```ts
// u4.ts
export interface I<T> {
  m(x: { [K in "a"]: T }): void;
  m(x: number): void;
}
```

- ESLint: nothing.
- translation: `u4.ts:3`.

**Defect 5 (extra, option): multi-line JSDoc under `ignoreOverloadsWithDifferentJSDoc`.** The tool compares the last
block comment before each overload (`getCommentsBefore(…).reverse().find(Block)`), so two different `/** … */`
blocks keep the pair apart. `block_before` tests `match("/\\*.*", c)`; on a comment spanning lines it gives no row
(the `.` does not cross a newline, by the result), both overloads get `sig_doc(s, "")`, and they pair. Single-line
`/** one */` vs `/** two */` is handled. Most JSDoc is multi-line, so the option is mostly inert.

```ts
// u5.ts  (options: { ignoreOverloadsWithDifferentJSDoc: true })
export interface I {
  /**
   * Takes a number.
   */
  m(x: number): void;
  /**
   * Takes a string.
   */
  m(x: string): void;
}
```

- ESLint with the option: nothing (without it: `u5.ts:9:5`).
- translation, `run-rr` with `{"ignoreOverloadsWithDifferentJSDoc": true}`: `u5.ts:9`.

## vitest-no-disabled-tests

The plugin: for each `CallExpression` that `parseVitestFnCall` accepts, `missingFunction` when its type is `test`,
`arguments.length < 2` and every member is `skip`; `disabledSuite`/`disabledTest` at the first `skip` member, else at
the options object's last `skip` key with value `true` (types `describe`/`test`), else at the head when the resolved
name starts with `x`; `pending()` with a bare `pending` callee nothing in scope declares. Agreed: the controls,
imports and aliases, `require`, `.extend` bindings, `check.todo("e")`, `check.skip("f")`, `check.skip.each([1])("h")`,
the later `skip() {}` method overriding `skip: true`.

**Defect 1 (miss): a test context's `skip()`.** `resolveScope` returns `"testContext"` for a name declared by a
function whose first `Identifier` parameter belongs to a function passed straight to an identifier-callee
`it`/`test`/`xit`/`xtest`/`fit`/`bench` call: any parameter name, not only `it`. Chain validity is not checked for
test contexts, the name is the local name, and the `skip` member reports. `context_root` takes only the name `it`.
This is Vitest's documented way to skip at run time, and the `rerail-typescript` corpus has it: ESLint reports
`test/engine/no-follow.test.ts:35:15`, `:53:15` and `test/daemon/daemon.test.ts:798:34`; the translation (and the
JSON original, so the suite says `same`) reports none.

```ts
// d1.test.ts
import { test } from "vitest";
test("needs a link", (context) => {
  if (Math.random() > 2) context.skip();
});
```

- ESLint: `d1.test.ts:3:34` disabledTest.
- translation: nothing.

**Defect 2 (extra): "missing function" with a member other than `skip`.** The plugin requires
`members.every(s => s === "skip")`; `fixture_call` exempts only `todo`, `extend` and `scoped`, so `only`,
`concurrent`, `each`, `for`, `fails`, … without a function are reported.

```ts
// d2.test.ts
it.only("no function");
it.concurrent("no function");
test.each([1])("no function");
```

- ESLint (this rule): nothing. (`no-focused-tests` reports line 1.)
- translation: `d2.test.ts:1`, `:2`, `:3` "This test has no function".

**Defect 3 (miss): `suite`, `bench` and `fit`.** `ValidVitestFnCallChains` holds `suite.*`, `bench.*` and `fit`;
`bench` and `fit` are `TestCaseName`s (type `test`), `suite` has type `unknown`, but the `skip` member reports for any
type. `vitest_name` has none of the three ("neither are suite and bench").

```ts
// d3.test.ts
suite.skip("s", () => {});
bench.skip("b", () => {});
fit("no function");
```

- ESLint: `d3.test.ts:1:7`, `2:7` disabledTest, `3:1` missingFunction.
- translation: nothing.

**Defect 4 (extra): chains the plugin does not have.** `parseVitestFnCall` drops a global call whose
`[name, ...members].join(".")` is not in `ValidVitestFnCallChains` (for `xit`: `xit`, `xit.each` only). `bad_chain`
accepts any up to three distinct modifiers after any head, so `xit.skip(…)` reports twice on its line. The comment's
"a table (`each`, `for`) or condition before another property" is not implemented either (no-focused-tests Defect 4).

```ts
// d4.test.ts
xit.skip("x", () => {});
```

- ESLint: nothing.
- translation: `d4.test.ts:1`.

**Defect 5 (line): `xit` with a `skip: true` option.** `xit` is type `test`, so the plugin reports at the `skip` key
(`skipNode ?? head`), once. `mode_option` takes groups `test`/`describe` only, so the translation reports the `x`
head at the call's line instead.

```ts
// d5.test.ts
xit("r",
  { skip: true },
  () => {});
```

- ESLint: `d5.test.ts:2:5`.
- translation: `d5.test.ts:1`.

**Defect 6 (miss): a parenthesised `pending`.** ESTree drops the parentheses, so `CallExpression[callee.name="pending"]`
matches `(pending)()`; the translation wants the call's `function` field to be the identifier itself.

```ts
// d6.test.ts
(pending)();
```

- ESLint: `d6.test.ts:1:1`.
- translation: nothing.

## vitest-no-focused-tests

The plugin: for each accepted call whose resolved name is `it`, `test` or `describe`, the first `only` member, else
the options object's last `only` key whose value is the literal `true`. Agreed: the controls, imports and aliases,
`.extend` bindings, `{ only: true }` in a test context's `it(…)`.

**Defect 1 (extra): a destructured `it` that is not the first property.** `resolveScope` returns `"testContext"`
for an object pattern only when the first `Property` of the function's first `ObjectPattern` parameter is keyed by
the name; otherwise (no `Identifier` parameter) the name is `"local"` and the call is no Vitest call.
`context_name` accepts any shorthand property; the JSON's `vitest-first-context-property` had the position test.

```ts
// f1.test.ts
test("t", ({ expect, it }) => {
  it.only("inner", () => {});
});
```

- ESLint: nothing.
- translation: `f1.test.ts:2`.

**Defect 2 (miss): `.only` after another member on a test context.** `members.find(only)` takes `only` at any index.
`(!context_head(c) ; i = 0)` keeps index 0 only; the comment's "the first property counts, whatever follows it" is
not what the plugin does.

```ts
// f2.test.ts
test("t", ({ it }) => {
  it.concurrent.only("inner", () => {});
});
```

- ESLint: `f2.test.ts:2:17`.
- translation: nothing.

**Defect 3 (miss): a test context outside a test call.** The object-pattern branch of `resolveScope` checks only that
the declaring node is a function; it need not be passed to a test. `context_fn` requires a test call's argument.

```ts
// f3.test.ts
export const helper = ({ it }) => {
  it.only("h", () => {});
};
```

- ESLint: `f3.test.ts:2:6`.
- translation: nothing.

**Defect 4 (extra): chains the plugin does not have.** As no-disabled-tests Defect 4: `it.each.only` and
`it.only.runIf` are not in `ValidVitestFnCallChains`.

```ts
// f4.test.ts
it.each([1]).only("t", () => {});
it.only.runIf(true)("t", () => {});
```

- ESLint: nothing.
- translation: `f4.test.ts:1`, `:2`.

**Defect 5 (extra): a test call that is the object of a member or the callee/argument of a call.**
`parseVitestFnCallWithReasonInner` returns null when the call's parent is a `CallExpression` or `MemberExpression`.
Nothing in `vitest_call` looks at the call's parent. The same holds for no-disabled-tests (`it.skip(…).then(…)`,
`wrap(describe.skip(…))`, not run).

```ts
// f5.test.ts
it.only("t", () => {}).then(() => {});
```

- ESLint: nothing.
- translation: `f5.test.ts:1`.

**Defect 6 (miss): option forms.** `findVitestModeProperty` reads `property.value` after ESTree dropped parentheses,
and `isSupportedAccessor(property.key, "only")` ignores `computed`, so `[only]` (an `Identifier` key) counts.
`mode_option` wants `kind(v, "true")` without `deparen`, and `opt_key` takes only static strings in brackets. The
same two forms apply to `skip` in no-disabled-tests (not run).

```ts
// f6.test.ts
const only = "only";
test("p", { only: (true) }, () => {});
test("q", { [only]: true }, () => {});
```

- ESLint: `f6.test.ts:2:13`, `3:14`.
- translation: nothing.

## The author's result file

`typescript-04.result.json` lists all four rules `same` and no `different` entry, so there is no `right` side to
check. `ptcheck --box pt-w07 --ref tbelanger/pack-translations-typescript-04 -- --pack typescript --rules <the four>`
confirms `4 same` on examples, fixtures (2397 files) and `rerail-typescript` (602 files). It also prints four
problems: `equivalence.json` still lists all four rules under `different` with their baseline counts ("recorded by the
suite, not yet explained", `right: unknown`) and the suite asks to remove them. The pack does not gate
(`equivalent: false`), so the check passes, but the entries are stale. `same` compares with the JSON originals, and
the corpora do not exercise these cases, with one exception: no-disabled-tests Defect 1 is in `rerail-typescript`,
so the JSON original misses those 3 findings too.
