# Review: react-practice-a

Branch reviewed: `tbelanger/pack-translations-react-practice-a` at `ff69e29009`. I reviewed the six `.rerail` files,
their JSON originals and the libraries they include (`lib/tsx/react-practice-a.rerail`, `lib/tsx/react.rerail`). I did
not read the author's NOTES first. These rules are Rerail's own practice rules, not ports, so the "tool" is the JSON
original, run through the product's path. Its own tests are the rule cases in
`packs/react/upstream/react-practice/<id>.cases.json` (pack test data, `npm run fixtures:pull`).

All runs were on `junction-pt-w01`:

- original: `node --import tsx scripts/ship/product-run.ts ~/src/rerail <corpus> <out> <rules dir>`, the suite's
  original side, using the box's checkout at `d54013c841` (this branch's base) with only the rule's JSON in the rules
  dir;
- translation: `~/pt/bin/rerail-engine run-policies <corpus> <out> <rule.rerail>` with `RERAIL_RR_LIB` pointing to this
  branch's `docs/rule-syntax/lib`.

I laid each rule's cases out as files (`caught<i>/<path>`, `passed<i>/<path>`) and compared both sides on them, then on
my own cases. Findings are listed as `path:line CODE`.

| Rule | Verdict |
|---|---|
| all six (shared `react_doc`) | 2 defects: a `@jsxImportSource` pragma and a tsconfig `jsxImportSource` are ignored |
| `react-practice-cache-update-in-mutate-call` | the shared defects, plus 1 (a query-client receiver spanning lines) |
| `react-practice-callback-for-dom-only` | the shared defects only |
| `react-practice-fresh-dependency` | the shared defects only |
| `react-practice-hoist-static-value` | the shared defects, plus 1 (type parameters of the component are not its reads) |
| `react-practice-lifecycle-hook` | the shared defects only |
| `react-practice-memo-trivial-expression` | the shared defects, plus 2 (a multi-line async callback; a parenthesised identity) |

On the rule cases the two sides differ only on 4 `passed` cases: `cache-update` passed7, and `hoist` passed13, 15 and
25. Each of these comes from a defect below. All other caught and passed cases agree on every rule.

## All six rules: the JSX-runtime gate (`react_doc`, `lib/tsx/react.rerail`)

Each JSON carries `file.jsxRuntime: ["react", "unknown"]`. The engine (`jsx_runtime_in_source` in
`native/src/code_rule_dsl/language.rs`, `Manifest::jsx_runtime` in `package_manifest.rs`) decides the runtime from a
`@jsxImportSource` pragma, then a `@jsx React…` pragma, then a top-level `h`/`createElement` import from another JSX
library, then the package's `compilerOptions.jsxImportSource`, then its dependencies. `react_doc` handles the factory
import and the dependencies correctly. It misses the other two sources:

**Defect A: a `@jsxImportSource` pragma is never read.** `first_source_comment` takes `min` over `visit_index`, and the
suite's engine gives `visit_index` no rows. A probe policy reporting `visit_index(c, i)` for comments,
`import_statement` and `program` nodes reported nothing. `earlier(o, c)` between two pragma comments of one file also
gave no rows. So `jsx_source` is empty, `import_source_names` never holds, and a file whose pragma names Preact,
Solid, Vue and so on is treated as React's. A `// @jsxImportSource preact` line comment and a `/* … */` block comment
fail the same way. `import { h } from "preact"`, which goes through the factory-import path, is handled correctly. The
order the pragma depends on needs a relation this engine fills.

```tsx
// src/Card.tsx  (hoist-static-value's own case passed15)
/** @jsxImportSource preact */
import { Box } from './Box';

export function Card() {
  const padding = { x: 2, y: 4 };
  return <Box padding={padding} />;
}
```

- original: nothing (Preact's JSX).
- translation: `src/Card.tsx:5 HOIST_STATIC_VALUE` ("padding reads nothing from this component or hook, …").

The same pragma (`/** @jsxImportSource preact */` or `solid-js`) produces an extra finding on every rule. The other
five cases are a pragma line added above otherwise reported code. The original reports nothing on each:

| Rule | Case | Translation's extra finding |
|---|---|---|
| cache-update-in-mutate-call | own case passed7 (`@jsxImportSource solid-js`, `bump.mutate(undefined, { onSuccess: () => queryClient.invalidateQueries() })`) | `src/widgets/Counter.tsx:7 CACHE_UPDATE_IN_MUTATE_CALL` |
| callback-for-dom-only | `const save = useCallback(() => submit(), [submit]); return <button onClick={save}>` under `@jsxImportSource solid-js` | `src/Save.tsx:5 CALLBACK_FOR_DOM_ONLY` |
| fresh-dependency | `const open = props.items.filter(Boolean); useEffect(() => {}, [open]); useEffect(() => {}, [props]);` under `@jsxImportSource preact` | `src/Card.tsx:6 FRESH_DERIVED_DEPENDENCY`, `src/Card.tsx:7 FRESH_PROPS_DEPENDENCY` |
| lifecycle-hook | `useMount(() => { console.log(id); })` (`useMount` from `react-use`, `id` a prop) under `@jsxImportSource preact` | `src/Mount.tsx:5 LIFECYCLE_HOOK_HIDES_DEPS` |
| memo-trivial-expression | `const s = useMemo(() => a + b, [a, b]);` under `@jsxImportSource preact` | `src/Preact.tsx:5 MEMO_TRIVIAL_EXPRESSION` |

**Defect B: the package's `compilerOptions.jsxImportSource` is not read.** `other_jsx` reads only the
dependencies of `package.json`. The engine first takes the `jsxImportSource` from the package's project configuration
(`Manifest::jsx_import_source`, set from `module_graph::Config`). A package that depends on `react` but compiles its
JSX with Preact's runtime is Other to the original. The translation treats it as React's.

```
tsc/package.json   { "name": "tsc-app", "dependencies": { "react": "18.0.0" } }
tsc/tsconfig.json  { "compilerOptions": { "jsx": "react-jsx", "jsxImportSource": "preact" } }
```

```tsx
// tsc/src/Hoist.tsx
import { Box } from './Box';

export function Hoist() {
  const padding = { x: 2, y: 4 };
  return <Box padding={padding} />;
}
```

- original: nothing.
- translation: `tsc/src/Hoist.tsx:4 HOIST_STATIC_VALUE`.

In the same project, each rule's smallest reported case is reported only by the translation:
`Memo.tsx:4 MEMO_TRIVIAL_EXPRESSION` (`useMemo(() => a + b, [a, b])`), `Cb.tsx:4 CALLBACK_FOR_DOM_ONLY`,
`Fresh.tsx:5 FRESH_DERIVED_DEPENDENCY`, `Mount.tsx:4 LIFECYCLE_HOOK_HIDES_DEPS` and
`Cache.tsx:5 CACHE_UPDATE_IN_MUTATE_CALL`. As controls, a sibling project with
`"dependencies": { "preact": … }` is excluded by both sides, and one with both `preact` and `react` is reported by both.

## react-practice-cache-update-in-mutate-call

**Defect: a query-client receiver whose text spans lines is not matched.** For `refetchQueries` in a file that imports
no query library, the JSON tests the receiver's text with `(?i)(?:^|\.)(?:query_?client|qc)$`, which only anchors the
end. `cache_call` uses `match("(?i)(.*\\.)?(query_?client|qc)", t)`, a whole-text match in which `.` does not match a
newline. A receiver written across lines therefore never matches. Use `[\s\S]*` or `(?s)` for the leading part.

```tsx
// src/Sync.tsx
import { useMutation } from 'some-lib';

export function Sync({ store }: { store: any }) {
  const m = useMutation({});
  return <button onClick={() => m.mutate(undefined, {
    onSuccess: () => store
      .queryClient.refetchQueries(),
  })}>x</button>;
}
```

- original: `src/Sync.tsx:6 CACHE_UPDATE_IN_MUTATE_CALL`.
- translation: nothing.

These agreed: options written in the call (with a comment before the vars, in parentheses, `satisfies`, a method
`onSettled() {}`), held in a `const`, or returned from a `useMemo` factory through an `if` block; a tRPC
utils object destructured from `api.useUtils()`; and the 18 caught and 14 of 15 passed cases (passed7 is Defect A).

## react-practice-callback-for-dom-only

Only the shared defects. These agreed: an anonymous `export default function`; `useCallback` from `'React'`
(case-insensitive) and from `require('react')`; `function` callbacks; handlers behind `?:`, `as` and `!`; calls through
`x?.()`; `connect(…)(function …)` and `observer(…)` components; `ref={x}`, a capitalised component, `motion.div`, a
hyphenated element and a deps array (identity reads); unused callbacks; lowercase non-hook functions; and every
case.

## react-practice-fresh-dependency

Only the shared defects. These agreed: object-literal method components; `Object.keys`/`Object.assign({}, …)`/
`.split` behind `as`, parentheses and `!`; `const { useMemo } = React`; a forwarded-ref second parameter and a
non-ref second parameter; components memoized by name (`React.memo(Memoized)`), by `observer(…)` and by
`React.forwardRef`; defaulted and array-typed parameters; deps in a nested arrow; props passed on whole, spread or
destructured in `useMemo`/`useCallback`; and every case.

## react-practice-hoist-static-value

**Defect: the component's own type parameters are not counted as its reads.** The JSON's `component-read` includes
`type_identifier`s whose binding site is in the component, and `static-const` skips a `const` whose type annotation
names one of the component's type parameters. Both depend on a type parameter having a binding site. In
`lib/tsx/names.rerail`, `ref_site` gives a type parameter no site: a probe reporting `type_identifier`s without a
`ref_site` listed every use of `Row`/`TRow` in the two cases below, while the imported `Column`/`ColumnDef` resolved
to their `import_specifier`. So `reads_component` ignores them, and `typed_by_component` (whose `ref_site(t, site)`,
`inside(site, ps, "type_parameters")` never holds) is dead. Generic components and hooks get findings the original
does not report. Two of the rule's own `passed` cases show it:

```tsx
// src/grid/TypedGrid.tsx  (case passed13)
import { Grid, Column } from './Grid';

export function TypedGrid<Row>({ rows }: { rows: Row[] }) {
  const columns = [] as Column<Row>[];
  return <Grid rows={rows} columns={columns} />;
}
```

```tsx
// src/table/DataTable.tsx  (case passed25)
import { Table, type ColumnDef } from './Table';

export function DataTable<TRow>({ rows }: { rows: TRow[] }) {
  const columns: ColumnDef<TRow>[] = [{ id: 'select' }];
  return <Table rows={rows} columns={columns} />;
}
```

- original: nothing on either.
- translation: `src/grid/TypedGrid.tsx:4 HOIST_STATIC_VALUE` and `src/table/DataTable.tsx:4 HOIST_STATIC_VALUE`.

The same happens with functions whose parameters are typed by the component's type parameter:

```tsx
// src/GenericFn.tsx
import { List } from './List';

export function GenericFn<T>({ items }: { items: T[] }) {
  function render(item: T) {
    return String(item);
  }
  return <List items={items} render={render} />;
}
```

```ts
// src/useGeneric.ts
import { useSorted } from './sorted';

export function useGeneric<T>(items: T[]) {
  const compare = (a: T, b: T) => 0;
  return useSorted(items, compare);
}
```

- original: nothing.
- translation: `src/GenericFn.tsx:4 HOIST_STATIC_FUNCTION` ("Function render reads nothing …") and
  `src/useGeneric.ts:4 HOIST_STATIC_VALUE` ("compare reads nothing …").

These agreed: `as const`, `satisfies`, consts in a nested `if` block; a context provider's `value`; `??` and a
parenthesised cast in props; React deps arrays; custom hook arguments, including the aliased React hook
`useState as useLocal` (identity-free); in-place mutation by assignment, `.sort()`, a member chain's `.push()` and
`Object.assign`; components passed to `memo`/`forwardRef` unnamed; object-method and `Store.useX =` hooks; async
declarations and arrows (skipped); generators; an imported enum member, a capitalised component, an imported object,
a module `let`/`var`, `new Date()` and `window` in literals; imported reads inside a nested arrow; a component-local
type alias; and a local helper function referenced by an object.

## react-practice-lifecycle-hook

Only the shared defects. These agreed: a namespace import (`H.useMount`); a per-hook default import with `.js`;
`@react-hookz/web/esm` with an alias; a local function declaration passed on; a setter whose array pattern has a hole
(`[, setCount]`); a template `const`; `React.useRef`; `useId`, `useReducer` and `useTransition` results; `default…`/
`initial…` names and `.defaultX` reads; a named function callback; `typeof` in types; a mount call in a nested
component; a callback in parentheses with a cast; and every case.

## react-practice-memo-trivial-expression

**Defect 1: an async callback that spans lines is not excluded.** `plain_callback` tests `!match("async\\b.*", cb)`,
a whole-text match in which `.` does not cross a newline. Only a one-line `async` callback is excluded. The JSON's
`regex: "^async\\b"` matches any async callback. Use `prefix("async", …)` plus a word boundary, or `[\s\S]*`.

```tsx
// src/AsyncMulti.tsx
import { useMemo } from 'react';

export function AsyncMulti({ total }: { total: number }) {
  const p = useMemo(async () => {
    return total;
  }, [total]);
  return <div>{String(p)}</div>;
}
```

- original: nothing.
- translation: `src/AsyncMulti.tsx:4 MEMO_IDENTITY` ("Use `total` directly instead of this useMemo …").

`useMemo(async function () {\n return total;\n}, [total])` and `useMemo(async () =>\n total, [total])` show the same
difference (translation `MEMO_IDENTITY`, original nothing).

**Defect 2: a dependency returned in parentheses is `MEMO_TRIVIAL_EXPRESSION`, not `MEMO_IDENTITY`.** The JSON
compares the returned expression with each dependency as a reference (`compare … text: "reference"`,
`isSameReference`), which looks through parentheses. `identity` asks `ref_same` only for identifiers and chain nodes
in the callback, and for the first child of a cast. A parenthesised returned value `(user)` is never compared, so
`identity` fails and the call falls through to the operator form. Asking `ref_same_ask(x, e, "static")` for the
returned `e` itself would let `ref_same`'s `deparen` handle it.

```tsx
// src/Paren.tsx
import { useMemo } from 'react';

export function Paren({ user, a }: { user: string; a: number }) {
  const u = useMemo(() => (user), [user]);
  const w = useMemo(() => { return (a); }, [a]);
  return <div>{u}{w}</div>;
}
```

- original: `src/Paren.tsx:4 MEMO_IDENTITY`, `src/Paren.tsx:5 MEMO_IDENTITY`.
- translation: `src/Paren.tsx:4 MEMO_TRIVIAL_EXPRESSION` ("Compute `(user)` inline …") and `src/Paren.tsx:5
  MEMO_TRIVIAL_EXPRESSION` ("Compute `(a)` inline …").

These agreed: `a?.b` against `[a.b]` and the reverse, `a['b']` against `[a.b]` (identity); `[(a)]` in the deps;
`(user as unknown)` (operator form) and `(user).name` (property read); a comment before the body; `typeof`, unary
`-`, `void` (not cheap); a trailing comma; a third argument; a block with a comment and one return; a template
without substitutions (no identifier); `c as number`; a return-type annotation; and every case.

## The author's result file and the suite

`react-practice-a.result.json` lists all six rules as `same`, with no `different` entries, so there was no `right`
side to check. I re-ran the suite (`ptcheck --ref tbelanger/pack-translations-react-practice-a -- --pack react
--rules <the six>`). It reports 6 same, 0 different, 0 failed on examples (100 files), fixtures (1570) and
samples-react (3139). None of those corpora has a JSX-runtime pragma, a `jsxImportSource` tsconfig, a generic
component handing on a type-parameterised value, a multi-line async `useMemo` or a parenthesised identity, so
"same" does not cover the defects above.

The suite also says the pack does not gate: `docs/rule-syntax/packs/react/equivalence.json` still lists all six rules
under `untranslated` ("no .rerail translation; recorded by the suite, not yet explained"), and the suite asks for
each to be removed.

## The author's NOTES, read after the findings above

- Defect A is in the NOTES under "Library changes needed", with the same `visit_index` cause, but as a lifecycle-hook
  difference only. The other five rules are described as "Follows the JSON", although every one of them gates on
  `react_doc` and differs in the same way (table above). The branch leaves `lib/tsx/react.rerail` as it is, so
  the defect ships with all six.
- The NOTES say type parameters and `@jsxImportSource` files were tried ad hoc, and describe hoist-static-value as
  following the JSON. The rule's own cases passed13 and passed25 contradict that (hoist defect).
- Defect B, the cache-update receiver regex and both memo defects are not mentioned.
- The two divergences the NOTES record on purpose hold up:
  - cache-update: with `const o1 = {…}; const o2 = {…}; m.mutate(v, o1); m.mutate(v, o2);` the original reports only
    `o1`'s `invalidateQueries` (line 5) and the translation also reports `o2`'s `removeQueries` (line 6). The rule's
    description ("held in a same-file `const`") supports the translation.
  - lifecycle-hook: with `const useRef = React; const box = useRef(null); useMount(() => { register(box); });` the
    original reports nothing and the translation reports line 7. `useRef` there names React's object, not the Hook,
    so the translation is right.

  Neither divergence appears in the result file, because no corpus contains such code.
