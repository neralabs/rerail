# Review: next-01

Branch reviewed: `tbelanger/pack-translations-next-01` at `6a0e4e7636`. I read the three `.rerail` files, their JSON
originals and the plugin's source (`dist/rules/*.js`) before the author's NOTES. The tools ran on a fleet box:
ESLint 9.39.5, `@next/eslint-plugin-next` 16.3.8, `@typescript-eslint/parser` 8.74.0 for `.ts`/`.tsx`, and
`@eslint-community/eslint-utils` 4.9.1. The config enables the three rules and declares the JSON's globals
(`location`, `window`, `document`, `self`; `globalThis` is an ES global). Each translation ran with
`rerail-engine run-policies`, using this branch's `docs/rule-syntax/lib`. Every counterexample below is a whole
file. Line:column comes from ESLint; run-policies gives the line.

| Rule | Verdict |
|---|---|
| `next-no-async-client-component` | 1 defect (parentheses, 3 forms), 1 minor |
| `next-no-duplicate-head` | 3 defects (`next/document` import order, abstract class, parenthesised superclass) |
| `next-no-location-assign-relative-destination` | 2 defects (reassigned parameters, constant expressions read as prefixes), 3 minor |

## next-no-async-client-component

The plugin walks `Program.body` in order. A top-level `ExpressionStatement` whose expression is the `Literal`
`'use client'` sets the flag. After that, an `ExportDefaultDeclaration` is reported if it is an async
`FunctionDeclaration` with a capitalised name. If it is a capitalised `Identifier`, the plugin reports the first
top-level `FunctionDeclaration`/`VariableDeclaration` that declares it, provided that declaration is async: an async
function, or an async `ArrowFunctionExpression` as the first declarator of that name. Everything else agreed:
generators, `"use client"` after an import, `"use\u0020client"`, a TS return type, a lowercase name, `export { Page
as default }`, `let Page; Page = async () => …`, an async function expression, and `"use client"` after the export.

**Defect (miss): parentheses.** ESTree has no node for parentheses, so the plugin sees through them in three places.
The translation reads tree-sitter's `parenthesized_expression` as a different node in each:
`use_client` takes the statement's child 0, `client_default` takes the export's `value`, and `async_arrow` takes the
declarator's `value`. The rule should unwrap parentheses as `next-no-duplicate-head`'s `unparen_jsx` does.

```jsx
// a1.jsx
("use client");
export default async function Page() { return null }
```

```jsx
// a2.jsx
"use client";
async function Page() { return null }
export default (Page);
```

```jsx
// a3.jsx
"use client";
const Page = (async () => null);
export default Page;
```

- ESLint: `a1.jsx:2:1`, `a2.jsx:2:1`, `a3.jsx:2:1` `@next/next/no-async-client-component`.
- translation: nothing on any of them.

**Minor (extra): a repeated name in one `var` statement.** The plugin checks only the first declarator of the
name (`declarations.find`). `async_arrow` accepts any declarator.

```jsx
// a4.jsx
"use client";
var Page = 1, Page = async () => null;
export default Page;
```

- ESLint: nothing.
- translation: `a4.jsx:2`.

## next-no-duplicate-head

**Defect 1 (extra): the `next/document` import is read regardless of position.** The plugin's `ImportDeclaration`
handler overwrites `documentImportName` as the traversal reaches each `next/document` import that has a default
specifier. A `ReturnStatement` therefore sees the last such import *above it*, and none if the import comes later.
`default_import` (`lib/tsx/next.rerail`) accepts every default import in the file. It has two gaps.
`later_default` orders imports with core's `earlier`, which has no rows on JS/TS, so every default import counts
(d5; the author's NOTES names this). Nothing ties the import to the return's position either (d4). The class
should extend the last default import of `next/document` that starts before the return.

```jsx
// d4.jsx
export default class MyDocument extends Document {
  render() { return (<Html><Head /><Head /><Main /></Html>) }
}
import Document, { Html, Head, Main } from 'next/document'
```

```jsx
// d5.jsx
import Document, { Html, Head, Main } from 'next/document'
import NextDocument from 'next/document'
export default class MyDocument extends Document {
  render() { return (<Html><Head /><Head /><Main /></Html>) }
}
```

- ESLint: nothing on either.
- translation: `d4.jsx:2`, `d5.jsx:4`.

The controls agree. Both report e1 (the class extends the second import `NextDocument`) and e2 (a class between two
imports extends the first, which is the one in effect at its return).

**Defect 2 (miss): a TypeScript `abstract class`.** ESTree has a `ClassDeclaration` with `abstract: true`, which
the plugin accepts. Tree-sitter has an `abstract_class_declaration`, and `doc_class` accepts neither it nor its
`export default abstract class` form.

```tsx
// d3.tsx
import Document, { Html, Head, Main } from 'next/document'
abstract class BaseDocument extends Document {
  render() { return (<Html><Head /><Head /><Main /></Html>) }
}
export default BaseDocument
```

- ESLint: `d3.tsx:3:36 @next/next/no-duplicate-head`.
- translation: nothing.

**Defect 3 (miss, rare): a parenthesised superclass.** `extends (Document)` is an `Identifier` superclass in
ESTree. `superclass` reads the heritage's child 0, which here is a `parenthesized_expression`.

```jsx
// d6.jsx
import Document, { Html, Head, Main } from 'next/document'
export default class MyDocument extends (Document) {
  render() { return (<Html><Head /><Head /><Main /></Html>) }
}
```

- ESLint: `d6.jsx:3:36`.
- translation: nothing.

Agreed: a fragment `return (<><Head /><Head /></>)` (reported at 7:9 by both), the anonymous `export default
class extends Document`, a class expression (neither reports), a return in an arrow inside `render`, and
`extends Document<DocumentProps> implements …` with three Heads (2nd and 3rd).

## next-no-location-assign-relative-destination

The plugin calls `getStaticStringPrefix(node)`. It first tries `getStringIfConstant` (eslint-utils
`getStaticValue`, then `String(value)`). Only when that returns `null` does it read a prefix: a template's first
cooked quasi, the left side of `+`, or, for an identifier whose last definition is a `Variable`, the last write
before the read (else the initialiser).

**Defect 1 (extra): reassigned parameters and catch bindings.** `getStaticStringPrefix` returns `null` for an
identifier whose definition is not `def.type === 'Variable'`: parameters, catch clauses, function and class names.
`write_before` follows writes to any `ref_site`, so it reads `path = "/"` as the value. It should be limited to
sites declared by a variable declarator (`decl_site` form `"let"`/`"var"`).

```js
// l1.js
export function go(path) {
  if (!path) path = "/";
  location.href = path;
}
```

```js
// m2.js
export function f() {
  try { throw 0 } catch (e) { e = "/a"; location.href = e; }
}
```

- ESLint: nothing on either.
- translation: `l1.js:3`, `m2.js:2`.

A parameter's default value (`function go(to = "/home") { location.href = to }`) agrees: neither reports.

**Defect 2 (extra): an expression with an absolute constant value is still read by its prefix.** The plugin reads
a prefix only when the whole expression is not constant. The translation's template and `+` clauses fire next to
the `static_value` clause, not after it. A probe confirmed that the engine's `static_value` folds both examples
below to the absolute URL (`https://example.com/login`, `https://example.com/a`). Those two clauses, and the
identifier clauses, should require `!static_value(v, _)`.

```js
// l2.js
const BASE = "https://example.com";
window.location.href = `${BASE}/login`;
```

```js
// l3.js
location.href = "" + "https://example.com/a";
```

- ESLint: nothing on either.
- translation: `l2.js:2`, `l3.js:1`.

With a non-constant substitution, both report (``location.assign(`${BASE}/a/${Date.now()}`)``,
`` `${APP_URL}/login?next=${encodeURIComponent(next)}` ``). The first quasi is then `""`, which counts as relative.

**Minor (miss): parenthesised assignment targets.** ESTree drops the parentheses. `member`'s `object` and
`assign`'s `left` meet a `parenthesized_expression`.

```js
// l8.js
(location).href = "/a";
(location.href) = "/b";
```

- ESLint: `l8.js:1:1`, `l8.js:2:1`.
- translation: nothing.

**Minor (miss): `String()` of non-string constants.** `getStringIfConstant` stringifies any static value. This is
what makes `null`, `undefined`, `true` and `5` relative, and the translation agrees on those. The plugin also
reaches arrays and objects. An object literal stringifies to `"[object Object]"`, which is relative whatever it
holds. A `for … of` binding's write expression is the iterated array, so `["/a"]` gives `"/a"`.

```js
// l9.js
for (const p of ["/a"]) location.href = p;
```

```js
// l10.js
const { a } = { a: "https://x.example" };
location.href = a;
```

- ESLint: `l9.js:1:25`, `l10.js:2:1`.
- translation: nothing.

**Minor (extra): an escaped template prefix.** The plugin tests the first quasi's *cooked* text. The translation
matches the raw source text.

```js
// l14.js
export function f(x) { location.href = `\x2F\x2Fevil.example/${x}`; }
```

- ESLint: nothing (cooked prefix `//evil.example/` is absolute).
- translation: `l14.js:1`.

Agreed: the last write by position, including one inside a closure above the read; a write after the read; `+=`
and `||=`; `self["location"]["assign"]`; `globalThis.location.href`; `document.location.assign`;
`window.location = …` (not reported); optional calls `window.location?.assign` and `location.assign?.`; `as
string` and `url!` in `.ts`; a no-substitution template; a conditional or `||` value (not reported); a shadowing
parameter `location` or local `window` (not reported); `const`/`let` initialisers built from a template or `+`; and
a `let` assigned in a `try` block.

## The author's result file

`next-01.result.json` marks all three rules `same`, with no `different` entries, so it has no `right` side to check
against the tool. The NOTES' cause for the commit is consistent with what I saw. With `src_before`, each
order-dependent case the NOTES name is reported where the plugin reports it. My a7, d1 and d10 cover the first
two. The NOTES' invalid-026 form, `let url = 'https://example.com'; url = '/other-path'; location.href = url`, gives
3:1 from both. The NOTES' list of other `earlier` users includes `tsx/next.rerail`'s `later_default`. That is the
second half of no-duplicate-head's defect 1 above, and it is still open in this rule.
