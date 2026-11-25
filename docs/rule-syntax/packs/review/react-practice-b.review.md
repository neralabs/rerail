# Review: react practice batch b

Branch reviewed: `tbelanger/pack-translations-react-practice-b` (6f014d5220). These five rules have no upstream linter, so
the reference ("the tool") is each JSON original (`packs/react/rules/<id>.json`, run through the product as
`scripts/ship/product-run.ts` runs it) and the intent its description and comments document. The translation side is
`rerail-engine run-policies` with the branch's `docs/rule-syntax/lib` (`RERAIL_RR_LIB`). Both ran on pt-w01, in
`/tmp/rpb-review`, over the case files below. Each finding is `path:line CODE`.

`ptcheck --ref tbelanger/pack-translations-react-practice-b -- --pack react --rules <the five>` (exit 0): all five
`same`. The corpora barely exercise the rules, though: one example finding each for four of the rules, and 2 + 8 + 6
findings for then-without-stale-guard. So the review rests on 63 case files of its own, each run through all five rules
on both sides, including the reported and unreported examples in each `.rerail` header. They were written as 46 files
per rule, 10 on name resolution, 5 on further guards and frameworks, and 2 for the defect below. Across them the two
sides differ on 3 lines, all from one defect in then-without-stale-guard.

The suite's report also lists one problem that is outside the translations: `docs/rule-syntax/packs/react/equivalence.json`
still lists all five rules as untranslated ("no .rerail translation; recorded by the suite, not yet explained"). The pack
does not gate (`equivalent: false`) until the five entries are removed. The writer's brief did not let the writer edit that
file, so the coordinator needs to remove them.

## react-practice-mounted-ref-never-reset: ok

Agrees with the original on every case: `useRef(/* c */ true)`, `React.useRef<boolean>(true)` with
`React.useLayoutEffect`, a cleanup in a `?:` branch, a `function` effect and cleanup, `useRef` destructured and renamed
from React (`const { useRef: ur } = React`), an undeclared `useRef`/`useEffect` whose names are parameters of another
function, `var` in a block, two components in one file where only one sets the ref true, the ref passed to a call
(not reported), the `amount`/`first` names, `require("react")` (neither reports it, because the original's React
object is never a `require`), and an assignment in a function nested inside the cleanup (not reported).

Gaps that the original has too, so they are not translation defects:
- `isMounted.current = !isMounted.current` in a handler counts as "set true", and so does an assignment to a different
  variable with the same name in a nested function (`const isMounted = { current: false }; isMounted.current = true`).
  The check compares names, not bindings.
- A cleanup that the effect returns by name (`const cleanup = () => { isMounted.current = false }; return cleanup;`) is
  not seen, although then-without-stale-guard's cleanup does follow that form.

## react-practice-request-beside-data-library: ok

Agrees on every case. Reported: `.then` after `.catch`, a `.then(r => r.json()).then(setX)` chain, an async IIFE with
`await`, a destructured `const { items } = await …`, `setX(await …)`, Relay's `subscribe({ next })` as a pair and as a
method, a request in a `setTimeout` inside the effect, `require("@tanstack/react-query")`, and
`require("swr").default`. Not reported: `saveOwner`, `refetch`, a `.then(() => setX(null))` that ignores the result,
`expo-location`, a file that imports only setup or only types (`import { type UseQueryResult }`,
`import { QueryClient as QC }`, `import "swr"`, `import {} from …`, `export { useQuery } from …`).

## react-practice-selector-returns-new-collection: ok

Agrees on every case: object, array, `as const`, `!`, and comment-wrapped selectors; `map`/`filter`/`slice()`/
`concat([…])`/`Object.keys`/`Array.from`; `slice(1)` and `concat(x)` (not reported); a local `Object` (not reported);
selectors through a `const`, a hoisted `function` or `var`, `useCallback`, `React.useCallback` and
`const { useCallback } = React`; type arguments and a typed return; a second argument; a selector with no parameter;
`react-redux`'s `useSelector`, renamed and aliased, including `const useAppSelector: TypedUseSelectorHook<…> =
useSelector` (none reported); `useShallowStore`, `useEqualStore` and `useStoreWithEqualityFn` (none reported); a member
callee, `store.useStore` (reported).

Gaps that the original has too:
- `useStore((s) => s.list ?? [])` (and `|| []`) is not reported. When `list` is missing, that selector returns a new array
  on every call, which is the re-render the rule describes. On Zustand 5 it is the "getSnapshot should be cached" loop.
- RTK 2's `export const useAppSelector = useSelector.withTypes<RootState>()` is a call, not an alias, so the
  react-redux exclusion does not follow it. A typical `useAppSelector`, imported from `./hooks`, is reported.

## react-practice-side-effect-in-render: ok

Agrees on every case:
- Navigation calls as a statement, after `&&`, under `void`, in both `?:` branches, as a JSX child, and as
  `useRouter().push`.
- `const { push, replace: rep } = useRouter()` and `navigate`/`history`/`navigation` from Hooks and from parameters,
  including `props.navigation`, `navigation?.navigate` and `router?.push`.
- `navigation.dispatch` through `useNavigation`, and `useDispatch`/`useAppDispatch` imported from a local `./hooks`.
- Calls inside the initialisers of `useMemo`, `useState` and `useReducer`.
- `memo`, `React.forwardRef`, a HOC, a `connect(…)(…)` chain under `export default`, and a Hook in a `.ts` file.
- Not reported: an assigned value, handlers, `useCallback`, `useEffect`, async components, "use server" files,
  `vue-router`, a react-tracked `track` import renamed, a lowercase function, a component that returns `null` and calls
  no Hook, a HOC call cast with `as FC`, a class component's `render`, and `(/* c */ useRouter())` (the original's
  `nthChild: 1` counts the comment, and the translation reproduces that on purpose).

Gap that the original has too: a file importing `solid-js` (`import { createSignal } from "solid-js"`) is still treated as
React, and its `trackEvent(…)` and a store selector are reported. A `preact` file is skipped by both.

## react-practice-then-without-stale-guard: 1 defect

### Defect: a member dependency is matched by its source text, so a line break inside it changes the result

`tws_dep_ref` (and through it `tws_dep_arg`, `tws_keyed_arg`) matches a dependency to a request argument with
`text(e, x) … text(n, x)`: equal source text. The original uses `pattern: "$D"`, which compares syntax trees and
ignores whitespace. A member expression that is split across lines in either the request or the dependency array has
different text, so the translation finds no request that reads a dependency and reports nothing.

```tsx
// tws-multiline-dep.tsx
import { useEffect, useState } from "react";
export function Profile({ session }: { session: Session }) {
  const [user, setUser] = useState(null);
  useEffect(() => {
    load(session.user.id).then(setUser);
  }, [session.user.id]);
  useEffect(() => {
    load(session.user
      .id).then(setUser);
  }, [session.user.id]);
  useEffect(() => {
    load(session.user.id).then(setUser);
  }, [session.user
    .id]);
  useEffect(() => {
    load(session.user/* the account */.id).then(setUser);
  }, [session.user.id]);
  return <div>{String(user)}</div>;
}
```

| line | original (product-run) | translation (run-policies) |
|---|---|---|
| 5 | THEN_WITHOUT_STALE_GUARD | THEN_WITHOUT_STALE_GUARD |
| 8 | THEN_WITHOUT_STALE_GUARD | — |
| 12 | THEN_WITHOUT_STALE_GUARD | — |
| 16 | — | — |

The third effect reads its dependency exactly as the first one does. Only the formatting of its dependency array
differs, so the original is right on lines 8 and 12.

The keyed-write exception (`tws_keyed`) has the same cause, the other way round: a computed key split across lines no
longer matches the dependency, so the translation reports a write that the original recognises as keyed and lets
through:

```tsx
// tws-multiline-keyed.tsx
import { useEffect, useState } from "react";
export function Pages({ session }: { session: Session }) {
  const [cache, setCache] = useState({});
  useEffect(() => {
    load(session.user.id).then((r) => setCache((c) => ({ ...c, [session.user.id]: r })));
  }, [session.user.id]);
  useEffect(() => {
    load(session.user.id).then((r) => setCache((c) => ({ ...c, [session.user
      .id]: r })));
  }, [session.user.id]);
  return <div>{String(cache)}</div>;
}
```

Original: no findings. Translation: `tws-multiline-keyed.tsx:8 THEN_WITHOUT_STALE_GUARD`.

A fix needs to compare structure: for example, compare token text with whitespace and comments removed, or match each
member chain's object and property recursively.

Everything else agrees:
- Reported: a setter passed directly, a callback that writes its argument, a `function` callback with a local derived
  from the result, `async (r) => setUser(await r.json())`, `.then(set, onError)`, an alias `const set = setU`,
  `useReducer`'s dispatch, `React.useEffect` with a `props.id` dependency, an effect-local `url` built from the
  dependency, `{ id }` shorthand, a request in a nested function, `.then(setUser).catch(…)`, a `let active = true` that
  no cleanup clears, comments before the callback and the dependency array.
- Not reported: `[]` dependencies, `Promise.resolve`, a callback that ignores the result, the `ignore` flag (read in the
  callback or in the request, cleanup inline or by name), `AbortController` (`{ signal: c.signal }` and
  `const { signal } = c`), a `.current` guard in the callback, the `if (cancelRef.current) return; … = true` ref
  guard, `if (!user)`, the state as a dependency, a keyed write, an effect-local guard `if (u.id === requested)`, and a
  setter shadowed by a local function.

Gap that the original has too: in
`load(id).then((r) => { setUser(r); return r.x; }).then((x) => setUser(x))` both `.then` calls are reported (two
findings on one line). The header says "in a chain only a `.then` with no state-writing `.then` inside its receiver
counts", but the receiver here *is* the state-writing `.then`, and `has` is strict on both sides. The comment
describes the code accurately only if "inside" is read literally.

## The author's result file

`react-practice-b.result.json` marks all five rules `same` and has no `different` entries, so there is no `right` side to
check. My ptcheck run reproduces `same` for all five on examples, fixtures and samples-react. For
then-without-stale-guard, `same` holds on the corpora only: the defect above is a difference the corpora do not contain.
`NOTES-react-practice-b.md` says the translation "follows the JSON" for that rule, and it does except in how it compares
dependencies.
