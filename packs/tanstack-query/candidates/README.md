# Rerail rule pack: TanStack Query, candidates

Rules written and tested for this pack that it does not ship. A candidate is not listed in `pack.json`, so it never
runs for anyone and is not published (`package.json`'s `files` leaves `candidates/` out). Each keeps its rule file in
`candidates/rules/` and its cases in `candidates/cases/<id>/`; the rule's `.test` block reads them from there
(`../cases/<id>/…`), so `rerail rules test candidates/rules/<id>.rerail` still proves it.

**Why they are candidates.** A rule ships on only when it is right in more than 90% of its matches on apps it was not
tuned or fixed on, with at least 5 judged matches in at least 2 such apps (the evidence floor). The pack's rules were
tuned on the 21 corpus apps that depend on TanStack Query (the pack README's corpus), generalized against 8 held-out
apps, and measured on three frozen test sets of open-source apps nobody used to write, tune or fix them: test set #1,
20 apps (2026-11-26; the apps and every finding are in
[NOTES-tanstack-query.md](../NOTES-tanstack-query.md#frozen-test-set-2026-11-26)), and test sets #2, 37 apps, and #3,
41 apps (2026-11-27; [NOTES-tanstack-query.md](../NOTES-tanstack-query.md#test-sets-2-and-3-2026-11-27)). Each rule
below is under the floor or below the bar on those apps: most matched nothing, so their precision on unseen apps is
unmeasured, however sound their corpus numbers; set-query-data-mutates-cache has 1 match, real;
cache-key-matches-no-query is 2 of 7 real. A candidate moves into the pack (its rule to `rules/`, its cases to
`cases/`, an entry in `pack.json`, its details to the README) when runs over apps it was not tuned on give it enough
matches, more than 90% real; server-action-as-query-fn did so on test set #2 (6 of 6 real in 3 apps) and is in the
pack now.

same-key-different-query-fn also stays here: no matches on the 37- and 41-app frozen sets after its fixes. The earlier
held-out findings informed tuning and do not qualify as unseen evidence. Its unresolved wildcard-key ambiguity
remains documented in the rule header.


## Rule details

What each candidate matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources.
They were the pack README's "Rule details" while the rules shipped; the `.ts`/`.tsx` lane split, the gate on a
package that depends on TanStack Query, and the corpus are described in the [pack README](../README.md).
### `tanstack-query-practice-same-key-different-query-fn`

A query definition is an object literal with a `queryKey` and a `queryFn` pair (the options of `useQuery`,
`useSuspenseQuery`, `queryOptions`, `useQueries` entries, `prefetchQuery`, `fetchQuery`, `ensureQueryData`, or a
wrapper hook), outside test, story and generated code. Its key template is the key array literal (through parentheses,
`as const`, `satisfies`) with string, number and keyword elements kept, an object element read as its sorted property
names (`{boardId, modified}`, `...` for a spread; the key hash sorts an object's keys, so `['boards', { teamId }]` and
`['boards', { boardId }]` never meet) and every other element read as `*` (`['user', *]`; a template literal's `${…}`
parts become `*`); or a key factory call by the declaration it calls, with its arguments read the same way
(`caKeys.getCaById(*)`): the factory's root is the symbol tsx/names gives it, through imports, re-exports and `const a =
b` aliases, so two files' own `listKey` functions are two factories and an import of one is that one. A definition
inside a `createQueryKeyStore` or `createQueryKeys` object of `@lukemorales/query-key-factory` has the key the library
caches: its property path in front of the key it returns (`users: { detail: (id) => ({ queryKey: [id], … }) }` is
`['users', 'detail', *]`), so two entries returning one key never meet, and a plain key that spells out an entry's
whole key meets it. Its fetchers are the callees its `queryFn` calls or names, by their dotted text with an
aliased import read as the name it imports (`getSettings as loadSettings` is `getSettings`), the global `fetch`, and
the callees of a function declared in the same file that the `queryFn` calls (one hop: `fetchWeeklyLimit` calling the
service a sibling query calls directly); a call whose first argument is a string also records that string. The rule
reports both definitions of a pair in one package (the nearest package.json) that are both plain or both infinite,
have the same key template, and have no fetcher in common, where a common fetcher called with strings on both sides
and no string in common (`fetch('/api/todos')`, `fetch('/api/todos?done=1')`) does not count. The message names the
other definition's file and line, and suggests appending a key element (so invalidations of the shared prefix still
reach both) or one `queryOptions` with `select`.

It leaves alone: a definition with no `queryFn` (an observer of an entry another query fills) or written as
shorthand; a key that is a variable, has a spread, or is an array with no string element (`[id]`); keys whose object
elements or factory arguments have other property names; calls of two factories of one name (each file's own `const
listKey = …`); two entries of a query-key-factory store that return one key; two definitions sharing a fetcher (the
same function with a different `select` or other arguments, a reference and a call, another import name for it, a
same-file helper that calls it); a `queryFn` that calls a parameter of an enclosing function (a wrapper hook's
fetcher) or calls only locals and globals other than `fetch`; a plain query and an infinite one (a different failure,
a mixed cache entry); two definitions in different packages of a monorepo, which have different QueryClients; and a
prefetch (`fetchQuery`, `prefetchQuery`, `ensureQueryData` and their infinite forms) in a file that calls TanStack
Query's `dehydrate`: a server render filling the cache it hands to the browser, whose fetcher is the server's way to
the data the browser's query fetches (`ssrApi.getDashboard(…)` beside `getDashboard(…).then((r) => r.data)`), so the
two have different fetchers by design.

**Sources:**
- TkDodo, maintainer, in [TanStack/query discussion #3776](https://github.com/TanStack/query/discussions/3776): "queryKeys and queryFn come in pairs … So you can't really have the same key with different query functions that produce different results. If you produce a different result, please use a separate key, as keys have to be unique."
- TkDodo in [TanStack/query discussion #4020](https://github.com/TanStack/query/discussions/4020): "'the first observer' will be picked, so the one that mounted first I guess. It's really not a good idea to do different Query Function for the same key!"
- @tanstack/eslint-plugin-query, [prefer-query-options](https://tanstack.com/query/latest/docs/eslint/prefer-query-options): "Separating `queryKey` and `queryFn` can cause unexpected runtime issues when the same query key is accidentally used with more than one `queryFn`."


### `tanstack-query-practice-module-query-client-in-ssr`

It reads only files whose nearest `package.json` lists a server-rendering framework among its dependencies,
devDependencies or peerDependencies: `next`; Remix (`@remix-run/react`, `/node`, `/server-runtime`, `/cloudflare`,
`/express`, `/serve`, `@vercel/remix`); React Router's framework mode (`@react-router/dev`, `/node`, `/serve`,
`/express`, `/cloudflare`); TanStack Start (`@tanstack/react-start`, `@tanstack/start`, `@tanstack/solid-start`);
`@solidjs/start`, `@sveltejs/kit`, `nuxt`, `astro`, `waku`, `vike`, `vike-react` (plain `react-router` is library
mode, a browser router, and is not one). The package must also put data into a query cache while the server renders,
which is what makes a shared cache leak: TanStack's hydration (`dehydrate`, `HydrationBoundary`, v4's
`<Hydrate>`/`useHydrate`), a suspense hook (`useSuspenseQuery`, TanStack's or tRPC's) or `suspense: true`, tRPC's
server-side helpers, or a QueryClient `prefetchQuery`/`fetchQuery`/`ensureQueryData` (or the infinite forms) in a
file that has a server data entry (`export function loader`, `getServerSideProps`, `getStaticProps`, a route's
`loader:`/`beforeLoad:`/`preload:`). This is read from the lines of every file of the package, so a `.tsx` page's
`HydrationBoundary` counts for a `.ts` client module across the lane split. A package whose queries are all plain
`useQuery`, or that prefetches only from browser code (an effect, a click handler), leaves the server's copy without
data: its queries fetch in the browser. In
those packages it matches TanStack Query's `QueryClient` (imported from `@tanstack/react-query`, `@tanstack/query-core`,
any `@tanstack/*-query` or `@tanstack/angular-query-experimental`, or v3's `react-query`, renamed or through a
namespace import `RQ.QueryClient`) made, or a factory of it called, in two shapes:

- in module scope, with no function or class around it: `export const queryClient = new QueryClient()`,
  `export default new RQ.QueryClient()`, `const client = makeQueryClient()`, unless the top-level statement itself
  tests where it runs;
- stored from inside a function in a module-level variable: `client ??= new QueryClient()`,
  `if (!c) c = makeQueryClient()`, unless the getter's own top-level statement tests where it runs: the TanStack docs'
  `getQueryClient()` makes a new client when `isServer` and caches one only in the browser. An unrelated `typeof
  window` elsewhere in the file does not hide an unguarded getter.

Both shapes read the test from code, not comments: a name `isServer`, `isBrowser`, `isClient`, `isSSR` (or the
SCREAMING_CASE form), `canUseDOM`; `typeof window`/`document`/`self`/`globalThis.window`; `"window" in globalThis`;
`import.meta.env.SSR`; `process.server`/`process.client`/`process.browser`; or a module const that holds one
(`const onServer = typeof window === "undefined"`).

A factory is a function declared at the top of its file (`function f()`, `const f = () => …`, `const f = function
() {…}`, exported or not) that returns a new client (directly, through a variable of its own, `const c = new
QueryClient(); return c`, or a branch of a returned `?:`) or another factory's result, so the docs' `getQueryClient()`
is one; it is followed into the file an import names, by a relative path or a tsconfig/jsconfig `paths` alias, with
TypeScript's extensions and `index` files. The message names the shape and the framework, gives the three fixes (a
client provider's `useState(() => new QueryClient())`, the docs' `isServer` getQueryClient, React's `cache()` in Server
Components) and says to leave it when the module never runs on the server, or when the client comes from an
`isServer` getter and only browser code reads it (then keep the export the browser relies on).

It leaves alone: `useState(() => new QueryClient())` and any client made inside a component, hook or function per
call; `cache(() => new QueryClient())` and calls of such a getter; factories themselves; the docs' `getQueryClient()`
and any getter that tests where it runs; a top-level `typeof window === "undefined" ? undefined : new
QueryClient()`; class fields (one client per instance); a `QueryClient` class of another library; packages with no
server-rendering dependency (Vite SPAs, React Native/Expo apps, library-mode React Router, browser extensions), or
none of whose code fills a cache on the server; tests, stories, fixtures, test setup files (`*.setup.*` such as
`vitest.setup.ts`, `setupTests.*`), `testing/`, `.ladle`/`.storybook`.

**Gaps:** a module client in a shared package without its own framework dependency, imported by an app that has one
(a monorepo `packages/ui` provider); a module client only ever loaded in the browser (`dynamic(…, { ssr: false })`),
which is reported; caches in an object or Map property, on `globalThis`, or in a class's static field; a guard that
lives only in another file (`if (isServer())` imported), or one spelled another way (`isNode`, `IS_WEB`); re-exported
factories (`export { makeQueryClient } from "./x"`) and default-exported factories; a tsconfig `baseUrl` other than
the config's folder, and `extends`. The fill test is a text search, so a comment naming one of its APIs counts, and a
prefetch that only runs in the browser (on hover) counts too. The lane split hides a `.tsx` file calling a factory
defined in a `.ts` file: `export const trpcClient = getQueryClient()` in a `.tsx` module, with the getter in a `.ts`
one, would be reported once the lanes join (a client made at import on the server and held for the process, though
nothing reads that export).

**Sources:**
- TanStack Query docs, [Server Rendering & Hydration](https://tanstack.com/query/latest/docs/framework/react/guides/ssr): "NEVER DO THIS: // const queryClient = new QueryClient() // Creating the queryClient at the file root level makes the cache shared between all requests and means _all_ data gets passed to _all_ users. // Besides being bad for performance, this also leaks any sensitive data."
- Same page: "it's important to create the `queryClient` instance **inside of your app** … **This ensures that data is not shared between different users and requests**".
- TanStack Query docs, [Advanced Server Rendering](https://tanstack.com/query/latest/docs/framework/react/guides/advanced-ssr): "Server: always make a new query client"; and for Server Components, "cache() is scoped per request, so we don't leak data between requests".
- TkDodo (TanStack Query maintainer), [TanStack/query#4920](https://github.com/TanStack/query/discussions/4920): "If you create the QueryClient instance in the top level module scope, it can share data between users as modules are evaluated once, meaning the QueryClient instance can persist across pre-renders on the server."
- Overlap: `@tanstack/eslint-plugin-query`'s `stable-query-client` shows `const queryClient = new QueryClient()` above `function App()` as correct; it is single-file and does not read the framework.

### `tanstack-query-practice-set-query-data-mutates-cache`

An updater that `setQueryData` (or `setQueriesData`, or tRPC's `utils.<path>.setData` and `setInfiniteData`) calls gets
the cached value itself, `getQueryData` (tRPC: `getData`, `getInfiniteData`) returns it, and a query's `select` option
receives it. If the code changes that value in place, the cache keeps the same reference. Structural sharing then
finds nothing new, so components may not re-render. The snapshot that `onMutate` took for rollback is the same object,
so `onError` "restores" the value that was already changed.

**Catches** (the first change in place is reported, once per updater, `select` function or `getQueryData` value; the
message counts the further changes of the same value, since they are one fix):
- `old.push(x)`, `.splice`, `.sort`, `.reverse`, `.unshift`, `.pop`, `.shift`, `.fill`, `.copyWithin`, and Map/Set
  `.set/.add/.delete/.clear` on the updater's first parameter or a part of it (these four are read as Map/Set
  changes, so an Immutable.js value or a class whose method of that name returns a copy would be a false report);
- `old.title = x`, `old.count += 1`, `old.count++`, `delete old.draft`, `Object.assign(old, patch)`;
- parts reached through member reads, `!`, `as`, `?? fallback`, `.find()`/`.findLast()`/`.at()`, `const { items } =
  old`, `for (const t of old.items)`, and the element parameter of an `old.forEach/map/filter/find/some/every/flatMap`
  callback (`old.map((t) => { t.done = true; return t; })`);
- an updater written inline, or a `const` or `function` in the same file passed by name;
- the same changes to a `const` bound to `qc.getQueryData(...)` (also `?? []`, `!`, `as T`);
- the same changes to the first parameter of a `select` option in an object that also has `queryKey` or `queryFn`
  (inline, a same-file `const`/`function`, or `useCallback(fn, deps)`): `select: (data) => data.sort(cmp)` sorts the
  cache in place whenever `select` runs.

**Leaves alone:** updaters that build a new value (`[...(old ?? []), x]`, `{ ...old, done: true }`, `old.map((t) =>
({ ...t, done: true }))`, `old.filter(...)`, `old.slice().sort(...)`, `old.toSorted(...)`, and in `select`
`[...data].sort(...)`); a `select` key of an object that is not query options; a local copy changed after
copying (`const next = [...old]; next.push(x)`, `structuredClone(old)`); immer (`produce(old, (draft) => { draft.push(x)
})`, `setData(input, produce((draft) => { ... }))`); a parameter assigned again before it is changed (`old =
structuredClone(old)`); a one-argument `setData(points)` on a plain receiver (chart and table libraries).

**Gaps:** a shallow copy whose nested arrays are then changed (`const n = { ...old }; n.items.push(x)` still changes
the cached `items`); an updater defined in another file; `useQuery`'s `data`, `fetchQuery` or `ensureQueryData`
results changed in place; a helper that mutates the value it is passed (`addTo(old, x)`). The rule reads one file at a
time, so the lane split does not touch it.

**Sources:**
- TanStack Query, [Updates from Mutation Responses](https://tanstack.com/query/latest/docs/framework/react/guides/updates-from-mutation-responses), "Immutability": "Updates via `setQueryData` must be performed in an _immutable_ way. **DO NOT** attempt to write directly to the cache by mutating data (that you retrieved from the cache) in place. It might work at first but can lead to subtle bugs along the way."
- TanStack Query, [Render Optimizations](https://tanstack.com/query/latest/docs/framework/react/guides/render-optimizations): "React Query will keep the original reference if _nothing_ changed in the data."

### `tanstack-query-practice-dehydrated-state-json-stringify`

It matches a `JSON.stringify(<state>)` or `superjson.stringify(<state>)` call (superjson by default, namespace or named
import) whose text is written into HTML as it is, outside test, story and generated files. `<state>` is the dehydrated
query cache: a `dehydrate(…)` call imported from a TanStack Query package (`@tanstack/react-query`,
`@tanstack/query-core`, the Vue/Solid/Svelte adapters, or `react-query` v3), named, renamed or through a namespace
import (`RQ.dehydrate(qc)`); a name bound to such a call (`const state = dehydrate(qc)`); or a name or property called
`dehydratedState`, `dehydrated`, `…DehydratedState` or `trpcState` (tRPC's SSR helpers); or an object literal holding
one of those as a property value (`{ queries: state, user }`, `{ dehydratedState }`). Parentheses, `as`,
`satisfies` and `!` are seen through. "Written into HTML" means the call's text is carried, through template
substitutions, `+` concatenations, parentheses, conditional branches, type assertions and a `{{ __html: … }}` object,
to one of: a template literal or `+` chain whose text holds an HTML tag or comment (`<script`, `</div`, `<!--`; a `<`
in script code such as `i<keys.length` is no tag); the `dangerouslySetInnerHTML` prop of any element; an expression
child of next/script's `<Script>`, which Next.js writes into the page as the inline script; or a `const` initialised
with the call (or a template around it) that is then used in one of those places. Anything else ends the climb: a call
the text is passed to (an escaper), a member it is the object of (`.replace(/</g, '\\u003c')`), a comparison, a JSX
text child, any other attribute.

It leaves alone: `JSON.stringify(state).replace(/</g, "\\u003c")`; `htmlEscapeJsonString(JSON.stringify(state))` (any
function between the call and the HTML); `serialize(state, { isJSON: true })` and devalue; a stringify with no markup
around it (`res.type("json").send(JSON.stringify(dehydrate(qc)))`, `JSON.parse(JSON.stringify(dehydrate(qc)))`, a
script-code template with `i<keys.length`); `<HydrationBoundary state={dehydrate(qc)}>` and `getServerSideProps`'
`{ dehydratedState }` (the framework serialises and escapes them); `<div data-state={JSON.stringify(state)} />` and
`<pre>{JSON.stringify(state, null, 2)}</pre>` (React escapes attribute values and text); `JSON.stringify(state) !==
JSON.stringify(prev)`; `JSON.stringify` of anything else, JSON-LD included; a `dehydrate` imported from the app's own
module; test files.

**Gaps:** markup that sits in another file (`index.html`'s `<!--state-->` filled by
`html.replace("__RQ_STATE__", JSON.stringify(state))` with no `<` around the call); state handed in under another name
or built in another function; tRPC's `helpers.dehydrate()` (a method, not an import) unless it is read back as
`trpcState`; a template that reaches the HTML through more than one `const` or through a return value; template
engines (EJS, Handlebars); `dehydrate` re-exported through the app's own module (imports are resolved in the file
only). The rule reads one file at a time, so the lane split does not touch it.

**Sources:**
- TanStack Query, [Server Rendering & Hydration](https://tanstack.com/query/latest/docs/framework/react/guides/ssr): "If you are using a custom SSR setup, you need to take care of this step yourself. Your first instinct might be to use `JSON.stringify(dehydratedState)`, but because this doesn't escape things like `<script>alert('Oh no..')</script>` by default, this can easily lead to **XSS-vulnerabilities** in your application. superjson also **does not** escape values and is unsafe to use by itself in a custom SSR setup (unless you add an extra step for escaping the output). Instead we recommend using a library like Serialize JavaScript or devalue which are both safe against XSS injections out of the box."
- Next.js, [JSON-LD](https://nextjs.org/docs/app/guides/json-ld): "The following snippet uses `JSON.stringify`, which does not sanitize malicious strings used in XSS injection. To prevent this type of vulnerability, you can scrub `HTML` tags from the `JSON-LD` payload, for example, by replacing the character, `<`, with its unicode equivalent, `\u003c`."

### `tanstack-query-practice-query-fn-input-hidden-from-key`

A query site is an object literal with both `queryKey` and `queryFn` (the options of `useQuery`, `useSuspenseQuery`,
`useInfiniteQuery`, a `useQueries` entry, `queryOptions`, `prefetchQuery`, `fetchQuery`, `ensureQueryData`, or a
project's wrapper), outside test, story and generated files. It reports three shapes that
`@tanstack/eslint-plugin-query`'s `exhaustive-deps` cannot see, because that rule checks only an inline queryFn
against the identifiers written in the key and counts any identifier passed to a key-factory call as covered:

- **FactoryDropsArgument**: the queryKey is a call of a key factory (`keys.members(projectId)`, `todosKey(ws, f)`,
  `queryKeys.a.b(x)`), the queryFn is an inline function that reads the argument (the same declaration, or the same
  member chain), and the factory never reads its parameter at that position, or has no parameter there.
- **FactoryDropsField**: the factory takes an object (an identifier read only as `p.<field>`, or a destructuring
  pattern without a rest element) and keeps no `<field>`, while the queryFn reads it: through an object-literal
  argument (`keys.byOwner({ ownerId, archived })` with `archived` read in the queryFn) or as `y.<field>` of the
  argument `y` (not a method call).
- **QueryFnByNameReadsMissing**: the queryFn is a name (`queryFn: load`) for a function declared in the same
  component or hook (`const load = () => …`, `useCallback(() => …, deps)`, `function load() {}`) that reads a
  parameter or local of that component, not as a callee, not a function, not a `useRef` value, and the key (an array
  or a call written there, or a local const holding one) contains no identifier of that name.

The factory is found in the same file or through imports: relative specifiers (with `.ts/.tsx/.js/.jsx/.mts` and
`index.*`), the `@/`, `~/` and `#/` aliases by file layout, `export { a as b }`, `export { a as b } from` and `export *`
re-exports, a `const keys = { … }` object, nested `keys.a.b`, methods `list(p) { return […] }`, and `as const` /
`satisfies` wrappers.

It leaves alone: a factory that keeps the field or the whole argument (`[…, f]`, `{ ...f }`, `f.join()`, `f[k]`); a
queryFn that does not read the dropped value; a literal argument (`keys.list({ done: false })`); a destructuring rest
(`({ cursor, ...rest }) => [rest]`, a deliberate exclusion); a factory with a rest parameter or one reading
`arguments`; a call with a spread argument; a queryFn by name that is module-level, imported, or reads only refs,
callees and functions; a key that names the read value anywhere (also inside a factory call, as exhaustive-deps
counts it); a key the rule cannot see (a parameter, a wrapper's `queryKey`); a credential or request plumbing passed
or read by a name like `token`, `accessToken`, `auth`, `session`, `userSession`, `jwt`, `csrf`, `secret`, `password`,
`apiKey`, `accessKey`, `privateKey`, `signature`, `cookies`, `credentials`, `header(s)`, `signal` or `*_token` (the
whole name or its last camelCase word; the last property of a member chain counts: `session.accessToken`; `nonce` and
`otp` are not listed), which does not say which data is fetched and must not go into a key, where it would show in
devtools and persisted caches and refetch on every token refresh. The messages say so too.

**Sources:**
- TanStack Query, [Query Keys](https://tanstack.com/query/latest/docs/framework/react/guides/query-keys): "If your query function depends on a variable, include it in your query key" … "they should include any variables you use in your query function that change."
- TkDodo (TanStack Query maintainer), [The Query Options API](https://tkdodo.eu/blog/the-query-options-api): "Separating QueryKey from QueryFunction was a mistake. The queryKey defines the dependencies to our queryFn - everything we use inside it must go into the key."
- TanStack Query, [exhaustive-deps](https://tanstack.com/query/latest/docs/eslint/exhaustive-deps): "Query keys should contain the serializable values that identify the data returned by your queryFn." Its source (`packages/eslint-plugin-query/src/rules/exhaustive-deps/exhaustive-deps.rule.ts`) returns unless the queryFn is an arrow, a function expression or a conditional, which is why a queryFn passed by name is this rule's.

### `tanstack-query-practice-query-client-factory-called-in-render`

It matches a call, outside test files, written directly in the body of a React component or Hook (a function named in
PascalCase or `useX`, or one passed to `memo`/`forwardRef`; not in a nested callback), whose callee is a function that
returns a new TanStack Query `QueryClient` every time it is called. The callee is followed to its declaration in the
same file (any scope) or across files: through an import, a tsconfig `paths` alias (`@/lib/query-client`), a relative
path, an `index.ts` barrel and `export … from` / `export *` re-exports. It counts as a factory when every value it
returns (an arrow's expression body, or each `return`, a returned local never assigned again read as its value: `const
c = new QueryClient(); return c`) is `new QueryClient(…)` of `@tanstack/*-query`,
`@tanstack/query-core` or v3's `react-query`, or a call of such a factory (one wrapper deep: `makeAppQueryClient = ()
=> makeQueryClient()`). The message names the factory and the line of its `new QueryClient`, and gives both fixes:
`useState(() => factory())`, or, where the first render can suspend with no Suspense boundary below (TanStack's own
caveat), the docs' `getQueryClient()` with an `isServer` branch.

It leaves alone: a factory called inside `useState(() => …)`, `useMemo(() => …)` or any other callback;
`useState(factory())` and any call that is an argument of a Hook (the Hook keeps the first client); `useState(factory)`;
lazy refs and singletons (`if (!ref.current) ref.current = factory()`, `client ??= factory()`, any assignment); calls
under a branch in the component (`isServer ? factory() : browserClient`, `if`, `&&`, `||`, `??`, `switch`); getters
that can return a kept client (`getQueryClient()` returning `browserQueryClient`, `() => (client ??= new
QueryClient())`): one non-fresh `return` disqualifies the function; `async` components (Server Components render once
per request, the upstream rule's own exception) and components that call `dehydrate()` (the server-prefetch pattern
with `HydrationBoundary`); calls at module level or in plain functions; a class named `QueryClient` that is not
TanStack Query's; test helpers in `test-utils.*`, `testUtils.*`, `test-helpers.*`, `testing/`, `mocks/`, `.storybook/`
(plus the scope's test and story globs), where a fresh client per test is the point.

**Gaps:** a factory reached through an object or class (`clients.make()`, `static create()`), through `export
default`, through a workspace package imported by name (`@acme/api`), or more than one wrapper deep; a call inside a
JSX ternary or `&&` is left alone with the lazy-init branches; components not named in PascalCase; a non-async Server
Component that never calls `dehydrate()` would be reported (none in the corpus). The lane split hides the commonest
layout, a factory in `lib/query-client.ts` and a provider in `providers.tsx`, so the pack's example keeps both in
`.tsx` files. `@tanstack/eslint-plugin-query`'s `stable-query-client` catches only a `new QueryClient` written in the
component itself; this rule is its one-hop extension into the factory.

**Sources:**
- TanStack Query docs, [ESLint: Stable Query Client](https://tanstack.com/query/latest/docs/eslint/stable-query-client): "The QueryClient contains the QueryCache, so you'd only want to create one instance of the QueryClient for the lifecycle of your application - _not_ a new instance on every render." And: "Exception: It's allowed to create a new QueryClient inside an async Server Component, because the async function is only called once on the server."
- TanStack Query docs, [Advanced Server Rendering](https://tanstack.com/query/latest/docs/framework/react/guides/advanced-ssr), in `getQueryClient()`: "This is very important, so we don't re-make a new client if React suspends during the initial render. This may not be needed if we have a suspense boundary BELOW the creation of the query client". And in `Providers`: "NOTE: Avoid useState when initializing the query client if you don't have a suspense boundary between this and the code that may suspend because React will throw away the client on the initial render if it suspends and there is no boundary".

### `tanstack-query-practice-infinite-and-plain-share-key`

TanStack Query keeps one cache entry per query key. An infinite query stores `{ pages, pageParams }` there and a plain
query stores the fetched value, so when an infinite query and a plain query use the same key, whichever runs second
reads the other's shape: `data.pages` is undefined in the infinite one (getNextPageParam throws), or the plain one
renders the wrapper object. The rule reads every query site of a package across its files: the options object of
`useInfiniteQuery`, `useSuspenseInfiniteQuery`, `infiniteQueryOptions`, `prefetchInfiniteQuery`,
`fetchInfiniteQuery`, `ensureInfiniteQueryData`, or any object literal with a `queryKey` and a page param option
(`initialPageParam`, `getNextPageParam`, `getPreviousPageParam`: a project's own wrapper); of `useQuery`,
`useSuspenseQuery`, `queryOptions`, `prefetchQuery`, `fetchQuery`, `ensureQueryData` and `useQueries` entries; and
the v4 positional key, `useInfiniteQuery(['k', id], fn)`. The hooks are names imported from `@tanstack/react-query`
(or query-core, vue/solid/svelte/angular-query, v3's `react-query`), renamed or through a namespace import; the client
methods are on any receiver in a file that imports one of those packages, or on a receiver named like a client. Two
keys are the same when their string and number elements have the same values and every other element the same
tokens; a name imported or declared at the top level reads as its file and export, so `projectKeys.list(orgId)` is one
key in every file that imports `projectKeys`, by an `@/` alias or a relative path; a key held in a const, in the file
or imported from another, is read through it. It reports the infinite query's key. It also reports
`queryClient.setQueryData(key, value)` where key is an infinite query's key that no plain query shares, and value is
written inline (a function, object or array) whose call names no `pages` or `pageParams` property and no
`InfiniteData` type (a key such as `['homepage']` or a variable `pageSize` does not count).

It leaves alone: keys that differ in any element (`['todos', 'infinite', f]` against `['todos', f]`;
`projectKeys.feed(orgId)` against `projectKeys.list(orgId)`); elements written differently (`['users', orgId]`
against `['users', userId]`), taken as different values; two packages (folders with their own `package.json`), which
run separate QueryClients, unless a file of one imports the other by its package.json `name` (`@acme/ui-kit` from
`packages/ui`), which joins them; tRPC's `trpc.x.useInfiniteQuery`/`.useQuery` (tRPC keys infinite and plain
procedures apart) and other libraries' `useQuery` (Apollo); setQueryData whose call names `pages`, `pageParams` or
`InfiniteData`, or whose value is a named function; test and story files.

**Gaps:** an options object held in a variable or spread into the call (`useInfiniteQuery({ ...opts(f), ... })`); key
factories are compared by call text, not by the array they return, so a factory call on one side and the same literal
array on the other are missed, and two factory methods returning the same array are missed; re-exports through a
barrel file; aliases other than `@/`, `~/`, `#/` to the package's `src/` or root; two apps sharing one package.json
are one package; elements written differently but holding the same value at run time
(`['users', id]` against `['users', userId]`) are missed; a key held in `useMemo(() => key, deps)` or chosen by `cond
? a : b` is compared as written, not as its value. The lane split hides a pair whose two sites are in a `.ts` and a
`.tsx` file (a `.tsx` component reading a key a `.ts` hook also uses).

**Sources:**
- TkDodo (TanStack Query maintainer), [Effective React Query Keys](https://tkdodo.eu/blog/effective-react-query-keys): "Please also be aware that you cannot use the same key for useQuery and useInfiniteQuery. There is, after all, only one Query Cache, and you would share the data between these two. That is not good because infinite queries have a fundamentally different structure than “normal” queries."
- TkDodo in [TanStack/query discussion #1255](https://github.com/TanStack/query/discussions/1255): "useQuery and useInfiniteQuery have different underlying data structures, so by using the same key, you are overriding the data. This is not a good idea". In the same thread tannerlinsley (TanStack's author) quotes a user's suggestion, "something that'd greatly help is if react-query can throw an error if it sees you are using both useQuery and useInfiniteQuery with the same query key.", and answers "I like this idea."
- [TanStack/query discussion #7455](https://github.com/TanStack/query/discussions/7455) (a `getNextPageParam` crash report), TkDodo: "you probably use that key already for non-infinite queries"; a commenter: "it is used more than once during manual cache updates using the setQueryData method."

### `tanstack-query-practice-cache-key-matches-no-query`

TanStack Query matches the key given to `invalidateQueries` (and `refetchQueries`, `resetQueries`, `removeQueries`,
`cancelQueries`, `setQueryData`, `setQueriesData`, `getQueryData`, `getQueriesData`, `getQueryState`) against the
cached queries by prefix. A key whose first element no query uses matches nothing: the invalidation after a mutation
silently does nothing and the screen keeps stale data, and `setQueryData` writes an entry no component reads. The
queries live in other files, so a single-file linter cannot see it.

It catches a cache call whose key (positional, as in v4 and `setQueryData`, or `{ queryKey }`) is an array literal
whose first element is a string literal that occurs nowhere else in the repository as a string:

```ts
// features/todos/queries.ts
export const todoKeys = { all: ["todos"] as const };
// features/todos/use-add-todo.ts
onSuccess: () => queryClient.invalidateQueries({ queryKey: ["todo"] }),   // caught: no code spells 'todo'
qc.setQueryData(["projects", id], next);                                  // caught: queries use ["project", id]
qc.invalidateQueries(["projcet"]);                                        // caught (v4 positional key)
```

It leaves alone: a root that any other code spells as a string (a `queryKey`, a key factory, a constant, a hook
argument such as `useApi("/api/settings")` for a hook that puts its argument in the key); a root that starts with the
literal head of a built string that ends at a separator (`` `/api/${name}` `` or `"/api/" + name` makes
`["/api/invoices"]` known; `` `t${x}` `` makes nothing known); a root quoted on a line of a `.ts` file read from a
`.tsx` call, or the reverse (the other lane's files are read as text); keys that
are not array literals with a string first element (`todoKeys.all`, `[...todoKeys.all, "x"]`, tRPC's `[["post",
"list"]]`, no key at all); calls in tests, stories, mocks, fixtures and generated files (they are still read, so a
generated client's keys count as known).

**Gaps:** a key root built from strings that never occur whole and have no literal head; queries defined in a
dependency (wagmi, a published SDK) or in `.vue`/`.svelte` files, which the rule does not read; a root string that
occurs anywhere else (a label, a type, and in the other lane's files a comment) hides a typo; only the first element
is compared (`["todos", "lsit"]` passes); a key built with `` `/api/${x}` `` in the other lane only is not read as a
prefix.

**Sources:**
- TanStack Query, [Query Invalidation](https://tanstack.com/query/latest/docs/framework/react/guides/query-invalidation): "// Invalidate every query with a key that starts with `todos`" and "When using APIs like `invalidateQueries` and `removeQueries` (and others that support partial query matching), you can match multiple queries by their prefix, or get really specific and match an exact query."
- TkDodo, [Effective React Query Keys](https://tkdodo.eu/blog/effective-react-query-keys): "In the examples above, you can see that I’ve been manually declaring the Query Keys a lot. This is not only error-prone, but it also makes changes harder in the future, for example, if you find out that you’d like to add another level of granularity to your keys."
- @tanstack/eslint-plugin-query, [prefer-query-options](https://tanstack.com/query/latest/docs/eslint/prefer-query-options): "The rule also enforces reusing `queryKey` from a `queryOptions` result instead of typing it manually in `QueryClient` methods or filters." It asks for the style, in one file; it never checks that the key exists.
