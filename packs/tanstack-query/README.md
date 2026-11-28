# Rerail rule pack: TanStack Query

The three `tanstack-query-practice-*` rules are Rerail's own; none is a port of a linter rule. Each is advice that the
TanStack Query docs give, or that its maintainers (TkDodo above all) give again and again in issues, discussions and
on their blogs, and that no linter checks. A rule was kept when the TanStack Query docs or a maintainer gave the
advice, with a second source beside it (another docs page, a maintainer's discussion answer, react.dev, MDN); when no
existing lint rule makes the same check; and when public apps were checked for it at pinned commits. The lint rules
checked were `@tanstack/eslint-plugin-query`'s (exhaustive-deps, no-rest-destructuring, stable-query-client,
no-unstable-deps, infinite-query-property-order, no-void-query-fn, mutation-property-order, prefer-query-options);
where one comes close, the rule's details say how far, and the candidates it covers were dropped
([NOTES-tanstack-query.md](NOTES-tanstack-query.md)). The pack ships only rules that were measured on apps they were
not tuned on ([How the rules were measured](#how-the-rules-were-measured)).

All three read across files: fetch-without-ok-check follows query functions into their fetchers,
query-data-copied-to-state follows a custom query hook, and server-action-as-query-fn follows imported Server Actions
and one helper call. The first two use bounded relative and conventional-alias resolution; the Server Action rule
uses tsx/names and tsconfig paths. Shared-library opportunities are in [the notes](NOTES-tanstack-query.md#library-candidates).

**The `.ts`/`.tsx` lane split.** The engine runs a TypeScript rule once per dialect, so a `.tsx` file and a `.ts` file
never meet in one cross-file join on their trees. The commonest layout, a `.tsx` component using a hook or fetcher in
a `.ts` module, is therefore missed: a custom query hook in `use-todos.ts` whose data a `.tsx` component copies into
state, a `.tsx` `queryFn` whose fetcher sits in a `.ts` module, a `'use client'` `.tsx` component calling a Server
Action from a `.ts` module, a pair of query definitions split across the two. The cases mark each line the split
hides `// cross-lane` and do not expect it. [ENGINE-GAPS.md](ENGINE-GAPS.md) has the repro and what closes it.

**Where the rules fire.** Each requires a TanStack Query import or, for tRPC option shapes, a relevant package
dependency. The Server Action rule also requires Next.js in the calling file's package.

Each rule is proven by its cases in `cases/<id>/`, a small repository where a line ending in `// caught` must be
reported and no other line may be, run by the rule file's own test (`.test cases exactly`; `rerail rules test
<file>.rerail`), and measured on five sets of open-source apps whose every match was read and judged real or not
([How the rules were measured](#how-the-rules-were-measured)). The unseen-apps column is the acceptance number: the
three frozen test sets, each counted for a rule only if the rule was not tuned or fixed on it. The corpus columns are
the tuning numbers: an independent review's per-rule sweep of the 21 repositories of [the corpus](#the-corpus) that
depend on TanStack Query (2026-11-24), with the rules changed after it re-run on every repository that had a match or
that the review named.

## Starting values

All three start on: query-data-copied-to-state and server-action-as-query-fn steer; fetch-without-ok-check logs.
The conflicting-query-function rule has no matches on frozen sets #2 and #3 after fixes. Its held-out findings
informed those fixes and are tuning evidence, not independent acceptance evidence. It and its cases remain
unpublished in [candidates/](candidates/README.md).

## How the rules were measured

A 33-app tuning corpus, eight held-out apps and frozen sets of 20, 37 and 41 apps were measured separately.

The bar: a rule ships on only if it is right in more than 90% of its matches on apps it was not tuned or fixed on
(test set #1 counts for a rule not tuned on it, test set #2 for a rule not fixed against its findings, test set #3 for
every rule). Below the bar a rule ships off, with its measured precision, its false-match shapes and the fix each
needs ([Starting values](#starting-values)). The evidence floor: a candidate joins the pack, and a rule stays on, only
with at least 5 judged matches in at least 2 of those apps; a candidate under it stays a candidate.

## Rule details

What each rule matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources.

### `tanstack-query-practice-query-data-copied-to-state`

TanStack Query re-renders a component whenever a refetch, an invalidation or a `setQueryData` changes the cached data.
A `useState` copy of that data is a second source of truth: `useState(data)` keeps the first render's value (undefined
while the query loads), and an effect that copies data into state renders a frame behind the cache and costs a render
per change. The rule finds the query as `useQuery`, `useSuspenseQuery`, `useInfiniteQuery` or
`useSuspenseInfiniteQuery` imported from `@tanstack/react-query` (or `react-query` 3, renamed or through `import *
as`), tRPC's `api.a.b.useQuery()` family, or a custom hook, in the same file or imported through a relative path, an
`@/` or `~/` alias, an `index` file or `export *`/`export { … } from` barrels, that returns such a query (`() =>
useQuery(…)`, `return query`) or its data (`return data`). The data is the result's `data` (`{ data }`, `{ data: todos
}`, `{ data = [] }`, `q.data`, tRPC's `const [data] = …useSuspenseQuery()`) and any member, `??`/`||` fallback, cast or
const alias of it. It reports:

- `useState(data)` or `useState(() => data)` when the state is never set (no setter, or a setter nobody reads); the
  message's way to keep a first value on purpose is a child component seeded from a prop, which is not reported;
- a setter call with query data in the own code of a `useEffect`/`useLayoutEffect` callback (not inside a `.then`, a
  timer or another function nested in it), when every other use of that setter is inside such a copying effect (the
  state is a pure mirror of the query).

It leaves alone: values derived during render; a state whose setter is also called or handed on elsewhere (an event
handler, a prop, a custom hook): an edited draft seeded from the query, which TkDodo names as the acceptable case;
updater calls (`setRows((prev) => [...prev, ...data])`), which accumulate rather than copy; a setter called in a
callback inside the effect (`identify(user.id).then(() => setReadyUserId(user.id))`), which records that something
happened after the data arrived; a child seeded from a prop
(`<EditForm initial={data} />`); form libraries (`reset(data)`, `defaultValues: data`); `useQuery` from other libraries
(Apollo, urql, Convex) and RTK Query's `api.endpoints.x.useQuery()`; custom hooks that return local state, also from
another file: an effect that fills the state from an imported hook copies query data only when that hook, followed
through imports and barrels, returns a query or its data, and one that fills it from any other hook
(`setRows(saved)` with `saved = useSavedFilter()`) sets the state elsewhere; tests, stories and generated code.

**Sources:**
- TkDodo (TanStack Query maintainer), [Practical React Query](https://tkdodo.eu/blog/practical-react-query), "Keep server and client state separate": "If you get data from useQuery, try not to put that data into local state. The main reason is that you implicitly opt out of all background updates that React Query does for you, because the state “copy” will not update with it." and, on the exception: "This is fine if you want to e.g. fetch some default values for a Form, and render your Form once you have data."
- TkDodo, [Breaking React Query's API on purpose](https://tkdodo.eu/blog/breaking-react-querys-api-on-purpose): "todos will be an Array of length 5 and todoCount will be 0 … This is wrong because values are not in-sync."
- react.dev, [You Might Not Need an Effect](https://react.dev/learn/you-might-not-need-an-effect), "Updating state based on props or state": "When something can be calculated from the existing props or state, don't put it in state. Instead, calculate it during rendering."

### `tanstack-query-practice-fetch-without-ok-check`

`fetch()` resolves on a 404 or a 500, and TanStack Query only sees an error when the query or mutation function throws
or rejects; a response read without a status check turns a failed request into a success: a query caches the error
body as data (no `error` state, no retry, no error boundary), a mutation runs `onSuccess` and its invalidations. The
rule reports the `fetch()` call (the global one, or `window.`/`globalThis.`/`self.fetch`) whose response is read
(`.json()`, `.text()`, `.blob()`, `.arrayBuffer()`, `.formData()`, directly, through a variable, `await`, or a
`.then(r => …)` callback) or dropped (`await fetch(…);`), when the function it sits in runs for a `queryFn` or
`mutationFn` option (a pair or a method) in a file that imports `@tanstack/*query*` or v3's `react-query`: the option's
own function, inline or named, and every function it calls by name, transitively, in the same file or another one
(relative imports, the `@/`, `~/`, `#/` aliases by layout, `index.*` files, barrel re-exports, default exports,
namespace imports, and members of object literals such as `projectService.getAll()`). The message cites the option
the fetch feeds (`queryFn` at path:line), so a shared fetcher in `lib/` is reported once, where the fix goes.

**Gaps:** class methods and `this.m()`; a function passed as a value (`queryFn: () => request(getTodos)`); a wrapper
hook in a file that does not import TanStack Query (`useApiQuery({ queryFn })`); workspace packages (`@acme/api`) and
other tsconfig `paths` keys; a Response held in an array (`Promise.all([...])`); an object member more than 3 deep
(`api.a.b.c.d`), and a member taken past a local object's imported member (`{ todos: importedApi }.todos.list`). Any
`.ok`/`.status` name in those functions counts as a check, even when it reads something else (`session.status`). The
lane split hides a `.tsx` component whose `queryFn` is a fetcher in a `.ts` module, and the reverse.

**Sources:**
- TanStack Query, [Query Functions](https://tanstack.com/query/latest/docs/framework/react/guides/query-functions): "For TanStack Query to determine a query has errored, the query function **must throw** or return a **rejected Promise**." and "While most utilities like `axios` or `graphql-request` automatically throw errors for unsuccessful HTTP calls, some utilities like `fetch` do not throw errors by default. If that's the case, you'll need to throw them on your own."
- MDN, [Window: fetch() method](https://developer.mozilla.org/en-US/docs/Web/API/Window/fetch): "A `fetch()` promise _does not_ reject if the server responds with HTTP status codes that indicate errors (`404`, `504`, etc.). Instead, a `then()` handler must check the `Response.ok` and/or `Response.status` properties."

### `tanstack-query-practice-server-action-as-query-fn`

In a file that imports TanStack Query (`@tanstack/react-query`, `@tanstack/query-core`, another `@tanstack/*-query*`
package, or v3's `react-query`) and whose package (its nearest package.json) depends on `next`, it reports a Server
Action inside a `queryFn` (a `queryFn:` property, a `queryFn() {}` method, or the shorthand `{ queryFn }`). A Server
Action is a name exported from a module whose first statement is `'use server'` (`export async function`, or `export
const x = actionClient…action(…)` as next-safe-action writes them), reached through an import tsx/names resolves (a
relative path, or an alias the tsconfig's `paths` map), `import * as actions`, or an `index.ts` that re-exports it.
It reports the action passed by reference (`queryFn: getProjects`), a call or reference anywhere in the query
function's body (`queryFn: async () => { const r = await getUsersAction(…) … }`), and a workspace function the queryFn
names whose own body references an action (`queryFn: loadProjectCount`, one call deep; the message names both). It
covers `useQuery`, `useSuspenseQuery`, `useInfiniteQuery`, `useQueries` entries, `queryOptions`/`infiniteQueryOptions`
(options a client hook will use), and `prefetchQuery`/`fetchQuery`/`ensureQueryData`/`prefetchInfiniteQuery`/
`fetchInfiniteQuery`/`ensureInfiniteQueryData` in a `'use client'` file or inside a `use*` function.

It leaves alone: `useMutation({ mutationFn: action })` (mutations are what actions are for); a
`queryClient.prefetchQuery({ queryFn: () => action() })` in server code (a file without `'use client'`, outside any
`use*` function: a Server Component prefetch is a plain call on the server); query options in a module that imports
`'server-only'` or starts with `'use server'`; a `queryFn` that only calls `fetch` or a helper that calls no action; a
`queryFn` key in a file with no TanStack Query import (RTK Query's `build.query({ queryFn })`); test and story files.

**Sources:**
- TanStack Query, [Advanced Server Rendering](https://tanstack.com/query/latest/docs/framework/react/guides/advanced-ssr): "We do **not** recommend using Next.js Server Actions to _fetch_ data in a `queryFn`. When called from the client, Server Actions run serially, not in parallel … This can leave queries stuck in a pending state or cause the action to never run at all".
- React, [`'use server'`](https://react.dev/reference/rsc/use-server) caveats: "Server Functions are designed for mutations that update server-side state; they are not recommended for data fetching."
- TkDodo in [TanStack/query#7934](https://github.com/TanStack/query/issues/7934): "if you come here wanting to use a server action inside a query function, please be advised that server actions are NOT RECOMMENDED to be used within query functions."
- TkDodo in [TanStack/query#6264](https://github.com/TanStack/query/issues/6264): "By passing the function as a reference, you're basically doing: `queryFn: (...args) => getProjects(args)` … one of our arguments being an `AbortSignal`, which is not a plain object".


## The corpus

App identities, source commits and finding locations are retained privately for coordinated disclosure. Aggregate evidence is given above.

## Files

| Path | What it is |
|---|---|
| `rules/tanstack-query-practice-<id>.rerail` | Three hand-written rules with matching notes, exclusions and fixtures |
| `cases/<id>/` | Each rule's cases, a small repository: a line ending in `// caught` must be reported, no other line may be (the rule file's `.test cases`); a `// cross-lane` line is a real violation the lane split hides. Not published |
| `candidates/` | Rules written for this pack that it does not ship (under the evidence floor or below the bar on apps they were not tuned on), with their cases and details. Not published |
| [`NOTES-tanstack-query.md`](NOTES-tanstack-query.md) | Aggregate measurement evidence, decisions and generic matching limitations |
| [`ENGINE-GAPS.md`](ENGINE-GAPS.md) | Engine gaps found while writing and measuring these rules, with repros and the rules each affects |

