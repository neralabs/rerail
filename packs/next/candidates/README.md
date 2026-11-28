# Rerail rule pack: Next.js, candidates

Rules of Rerail's own for the App Router that left the pack on 2026-11-26 and have not passed the bar since, and the
Next fact model's probes. Nothing here is published or listed in `pack.json`; the pack's [README](../README.md) covers
the rules it ships, and [NOTES-next.md](../NOTES-next.md) every match read for every rule, candidates included.

| Path | What |
|---|---|
| `rules/<id>.rerail` | A candidate practice rule, as it was when it left the pack; its `.test cases exactly` reads `../cases/<id>/`, so `rerail rules test candidates/rules/<id>.rerail` runs its cases from here |
| `cases/<id>/` | Its cases, a small repository: a line ending in `// caught` (`# caught` in text files) must be reported, no other line may be |
| `probes/` | The Next fact model's probes (`rules/`) and the labelled fixtures they are scored on (`labels/`) |

## Candidates

A rule ships on only when more than 90% of its matches on apps it was never tuned or fixed on are real, and moves out
of here only with at least 5 such matches judged in at least 2 apps. The practice rules were tuned on the 33 corpus
apps ([README.md](../README.md#the-corpus)), generalized against 8 held-out apps (teableio/teable,
Openpanel-dev/openpanel, haydenbleasel/next-forge, nextjs/saas-starter, dubinc/dub, jitsucom/jitsu, Dokploy/dokploy,
midday-ai/midday) and then run on three test sets of frozen apps nobody used to write or tune them: 20, 37 and 41 apps
([How the rules were measured](../README.md#how-the-rules-were-measured)). Test set #2 counts only for the rules not
fixed against its findings; unvalidated-input and unstable-cache-closure-key were. After test set #3, redirect-in-try
and revalidate-unknown-target met the bar and moved to the pack; the rules below have not: too few unseen matches,
or too many false. Each keeps the mode its rule file sets; its `pack.json` entry (title, `on`, examples) is in git
history and goes back with it.

| Rule | Mode | Catches | Corpus, 33 apps (tuning) | Held-out, 8 apps | Test set #1, 20 apps | Test set #2, 37 apps | Test set #3, 41 apps |
|---|---|---|---|---|---|---|---|
| `next-practice-server-action-unvalidated-input` | Steer | a Server Function argument used unchecked in a Prisma filter, directly or through the helpers it is passed to | 41 (5), 41 of 41 real; 33 after the second test set's fixes (below) | 0 | 0 | 42, 32 real; 32 of 32 after the fixes (tuning data now) | 1 of 1 real (under the floor) |
| `next-practice-public-env-secret` | Steer | a secret's name with the `NEXT_PUBLIC_` prefix, or in next.config's `env` | 1 (1), 1 of 1 real | 0 | 0 | 0 | 2, none real (1 app) |
| `next-practice-unstable-cache-closure-key` | Steer | an `unstable_cache` function reading a per-request value (or a field of one) of the function around it that keyParts leave out | 0 | 0 | 0 | 6, none real; 0 after the fixes (tuning data now) | 0 |
| `next-practice-revalidate-unknown-tag` | Log | `revalidateTag`/`updateTag` of a tag nothing assigns | 3 (2), 2 of 3 real | 5 false before the `Vercel-Cache-Tag` change, 0 after (dub only) | 0 | 25 of 25 real (2 apps) | 6, none real (1 app): 25 of 31 unseen (81%) |
| `next-practice-request-api-in-cache-scope` | Log | `cookies()`, `headers()`, `connection()`, an auth library's session read or a page's `searchParams` in a `'use cache'` or `unstable_cache` scope, directly or through a helper | 0 | 0 | 0 | 0 | 0 |
| `next-practice-non-serializable-client-prop` | Log | a function or class instance passed from a Server Component to a Client Component (Next.js's `Link` and `Image` included) or a DOM element | 0 | 0 | 0 (before the generalization pass: 98, none real) | 0 | 0 |
| `next-practice-edge-imports-node-only` | Log | an Edge middleware or edge route that loads a Prisma client, a native addon, a Node built-in or a TCP client | 0 | 0 | 0 | 0 | 0 |
| `next-practice-client-reaches-server-module` | Log, off | a database client, Node-only package or `server-only` module imported by code a 'use client' file loads | 0 (2 real in a script estimate with `.ts` and `.tsx` joined) | 0 | not run (off) | 0 | 0 |

Matches are the engine's findings, with the number of apps in parentheses; every one read, with its verdict, is in
[NOTES-next.md](../NOTES-next.md#matches-read), the held-out run in
[NOTES-next.md](../NOTES-next.md#generalization-2026-11-25), test set #1 in
[NOTES-next.md](../NOTES-next.md#frozen-test-set-2026-11-26), test set #2's fixes in
[NOTES-next.md](../NOTES-next.md#fixes-after-the-first-test-set) and test set #3 in
[NOTES-next.md](../NOTES-next.md#test-set-3-2026-11-27). Beyond the numbers:

- non-serializable-client-prop's only test-set matches came from its text before the generalization pass: 89 in
  Mail-0/Zero (a React Router app) and 9 in webstudio-is/webstudio (Remix), none of them a Next.js Server Component.
  The pass's Next.js package gate leaves such apps alone, and the rule as it is now has matched no test-set app.
- revalidate-unknown-tag logs because a tag assigned in a `.tsx` file is unseen from a `.ts` call (the lane gap,
  [ENGINE-GAPS.md](../ENGINE-GAPS.md) 2); its corpus false report and all 6 of test set #3's are that shape.
- public-env-secret's 2 test-set-#3 matches are one obsolete Compose variable no source reads, reported once for
  each side of its assignment.
- client-reaches-server-module was off before it left: its main shape (a `.tsx` component importing a `.ts` module)
  is invisible until the engine joins the two, and what it can see already fails `next build`.
- unvalidated-input has 1 unseen match since its fixes, real, under the floor; unstable-cache-closure-key,
  request-api-in-cache-scope and edge-imports-node-only have never matched a real app.

## Rule details

What each candidate matches beyond its one-line description: exact matching, edge cases and gaps, with its sources and
corpus numbers. Each moves to the pack's README when the rule ships.

### `next-practice-server-action-unvalidated-input`

Rerail's own rule, not a port. A Server Function is an exported function of a file whose first statement is
`"use server"` (`export async function x`, `export const x = async (…) => …`, `export { x }`, `export default`), or a
function whose own body starts with `"use server"` (an inline action, `.bind` arguments included). Its parameters hold
whatever the client encodes: React's wire format carries plain objects, so a parameter typed `string` can arrive as
an object with an operator key such as `not`, which Prisma reads as a filter. The rule follows each parameter, the
names bound from it without a call (`const { id } = data`, `const id = data.id as string`) and the parameters of the
workspace functions it is passed to (same-file helpers, imported data-access functions, as deep as the calls go), and
reports where one is used, unchecked, in the `where` of a Prisma method that accepts filter operators (`findFirst`,
`findFirstOrThrow`, `findMany`, `count`, `aggregate`, `groupBy`, `updateMany`, `updateManyAndReturn`, `deleteMany`) on
a `….model.method({ where: … })` call:
- `FilterValue`: the value a field is compared with: `where: { token }`, `where: { id: input.id as string }`, inside
  `AND`/`OR`/`NOT` and relation filters too; once per query and value;
- `WholeFilter`: the whole filter: `where: filters`, `where: { ...input, archived: false }`;
- `FilterValueVia`: at the action's call `findOpenInvite(token)` when the called function (or one it calls) puts its
  parameter in such a filter; the message names the query's file and line.

The message says what a client can send, and the fix: parse the arguments with a schema of their real type
(`z.string()`, `z.number().int()`) or test `typeof` before the query, in the action or in the helper; a helper the
rule does not recognise that already checks the value can stay.

It leaves alone: a value checked anywhere in the function (or what it was read from: `schema.parse(input)` covers
`input.id`): an argument of a call named `parse`, `safeParse`, `parseAsync`, `safeParseAsync`, `spa`, `validate…` or
`assert…`, the only argument of a type test named `isX…`, `check…`, `ensure…` or `verify…` (`isUuid(id)`,
`Number.isInteger(n)`; not `verifyPassword(id, pw)`, which compares two values), the operand of `typeof`, or one side
of a `!==` against a value the client did not send in the condition of an `if` that returns or throws, alone or in an
`||` chain (`if (!session || userId !== session.user.id) return`: past it, the value is that one); a value that was
first a unique key of a unique lookup's `where` (`findUnique`, `findUniqueOrThrow`, `update`, `delete`, `upsert`), in
the function or in a helper called earlier, since Prisma rejects an object where a unique field goes and the code
after it never runs: the where's only key, whatever the field (`where: { id }`, `where: { email }`: a unique lookup
must name a unique field), or a field the repository's `schema.prisma` marks `@id` or `@unique` on the call's model,
beside an `AND`, a relation filter or other fields (`where: { uid: id, AND: { userUid } }`,
`where: { uid, roadmap: { userUid } }`); without the schema, a key beside another (`where: { id, projectId }`) proves
nothing, as the other key may be the unique field; a read (`findFirst`, `findMany`, `count`, …) followed by a unique
write (`update`, `delete`, `upsert`) keyed on the same value, both plain statements of one block (no branch, loop or
nested function around either, and no `return` around the read), since the write throws before the read's rows are
returned or kept; a bulk write followed so inside one interactive `$transaction(async (tx) => …)`, which rolls it back;
a query batched in one `$transaction([…])` with a unique call keyed on the same value, which fails the whole batch;
unique methods themselves; an
operator's operand (`{ title: { contains: q } }`, `{ id: { in: ids } }`), which Prisma types and rejects as an object;
a call's result (`String(id)`, `formData.get("id")`, `Number(x)`), so FormData actions never match; functions not
exported from a "use server" file; functions wrapped by a call (`export const a = authed(async (session, id) => …)`,
next-safe-action's `action(schema, fn)`), whose parameters the rule cannot map to the client's; `data:` (the prisma
pack's prisma-practice-request-body-as-data); test code.

**Gaps:** filters built in a variable before the call (`const where = { id }; findMany({ where })`); helpers called as
members (`repo.find(id)`, `getInteractor().invoke(data)`), default exports and re-exports; nested writes' filters;
wrapper-made actions that do not validate; a check counts wherever it is in the function, and a check of `id` does not
cover `data.id`; a one-argument helper named like a type test that only looks the value up (`isMember(id)`) counts as a
check; what the code does with a read's rows before the unique write that follows throws (an email sent, a notification
created) is not read; the schema is read from `.prisma` files six folders down from the root at most, and a compound
key (`@@unique([a, b])`) is not used. An inline action in a `.tsx` page that passes its argument to a `.ts` helper is
missed (the lane gap).

### `next-practice-public-env-secret`

A name reads as a secret when it ends in SECRET (CLIENT_SECRET, SECRET_KEY, SECRET_ACCESS_KEY, SECRET_TOKEN),
PRIVATE_KEY/PRIVATE_TOKEN, PASSWORD/PASSWD/PASS/PASSPHRASE, SERVICE_ROLE(_KEY), SERVICE_KEY, DATABASE_URL, DB_URL,
DIRECT_URL, POSTGRES_URL, MONGODB_URI, CONNECTION_STRING, SIGNING_KEY/SIGNING_SECRET, ADMIN_KEY/ADMIN_TOKEN,
ENCRYPTION_KEY, MASTER_KEY, a server-only provider's API key (OPENAI_API_KEY, ANTHROPIC_API_KEY, GEMINI_API_KEY,
GOOGLE_GENERATIVE_AI_API_KEY, OPENROUTER, FIREWORKS, RESEND_API_KEY, SENDGRID_API_KEY, POSTMARK, MAILGUN, REPLICATE,
GROQ, MISTRAL, COHERE, DEEPSEEK, ELEVENLABS, PERPLEXITY, TOGETHER, XAI), AUTH_TOKEN (TWILIO_AUTH_TOKEN,
SENTRY_AUTH_TOKEN), REST_TOKEN (Upstash's read-write Redis token) or a provider's ACCESS_TOKEN (GITHUB, GITLAB,
VERCEL, NETLIFY, CLOUDFLARE, SHOPIFY_ADMIN, NOTION, SLACK, HUBSPOT, DIGITALOCEAN, FIGMA, LINEAR, AIRTABLE, DROPBOX,
PERSONAL), with an optional _BASE64/_B64. The provider lists are platform names, not a shape: `_API_KEY` and
`_ACCESS_TOKEN` alone are not enough, since many vendors issue browser keys under those names (Firebase, Google Maps,
Algolia search, Mapbox, Contentful delivery, Rollbar client). It matches:
- `process.env.NEXT_PUBLIC_SUPABASE_SERVICE_ROLE_KEY!`, `process.env["NEXT_PUBLIC_OPENAI_API_KEY"]`,
  `const { NEXT_PUBLIC_UPLOAD_SIGNING_KEY } = process.env` in code;
- an env schema's key: `client: { NEXT_PUBLIC_STRIPE_SECRET_KEY: z.string() }` (t3-env and the like);
- `NEXT_PUBLIC_DATABASE_URL=…` (also `export …=` and commented-out `# …=`) in the `.env*` files of the repository root
  and of every folder below it (dot, dependency and build folders aside) that holds a next.config (`web/`,
  `apps/web/`, `clients/acme/portal/web/`);
- the name in a Dockerfile (`ARG NEXT_PUBLIC_JWT_SECRET`), YAML (CI workflows, compose files, charts) or turbo.json;
- next.config `env: { JWT_SECRET: process.env.JWT_SECRET }`: that block inlines any key, prefix or not.

It leaves alone:
- public-by-design keys: `NEXT_PUBLIC_SUPABASE_ANON_KEY`, `NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY`,
  `NEXT_PUBLIC_POSTHOG_KEY`, `NEXT_PUBLIC_SENTRY_DSN`, `NEXT_PUBLIC_RECAPTCHA_SITE_KEY`,
  `NEXT_PUBLIC_MAPBOX_ACCESS_TOKEN`, `NEXT_PUBLIC_UPSTASH_REDIS_REST_READ_ONLY_TOKEN`: KEY, TOKEN or DSN alone is no
  secret (`NEXT_PUBLIC_GITHUB_TOKEN` is left alone too, a known miss);
- a secret word that is not the name's end: `NEXT_PUBLIC_SECRET_SHARING_ENABLED`,
  `NEXT_PUBLIC_DISABLE_EMAIL_PASSWORD_SIGNUP`, `NEXT_PUBLIC_SIGNING_TRANSPORT_IS_CSC`;
- flags and demo values: a name with an ENABLE, DISABLE, ALLOW, SHOW, HIDE, RESET, USE, HAS, IS, REQUIRE, NO,
  SKIP, WITH(OUT), MIN, MAX, FEATURE, FF, FLAG, DEMO, EXAMPLE, PLACEHOLDER, MOCK, DUMMY, FAKE or SAMPLE
  segment (`NEXT_PUBLIC_ENABLE_PASSWORD`, `NEXT_PUBLIC_DEMO_PASSWORD`);
- server names without the prefix (`STRIPE_SECRET_KEY`, `process.env.JWT_SECRET`) outside next.config's `env`, and
  next.config `env` keys that are no secret (`APP_VERSION`);
- comments in code, tests, stories and fixtures.

**Gaps:** `.env*` files are read only in the root and in a next.config's folder (at most twelve folders down, a bound
the engine needs to see the walk end); hidden files
other than `.env*` (`kubernetes/.secrets_sample`) are never read (the engine makes no document of a hidden file). A
secret under a name that does not say so (`NEXT_PUBLIC_STRIPE_KEY` holding `sk_live_…`), a secret inlined by webpack's
DefinePlugin, and names built at run time (`` process.env[`NEXT_PUBLIC_${x}`] ``, which Next.js does not inline
either). Generic `NEXT_PUBLIC_API_KEY`/`NEXT_PUBLIC_*_API_KEY` of providers not in the list: many are public by design
(Chatwoot, PostHog, Builder.io), so the rule names only providers whose keys are server-only. A finding in a `.env*`
file, Dockerfile or YAML file is reported once by each language run (up to three times) until the engine merges them.

### `next-practice-unstable-cache-closure-key`

Rerail's own rule, not a port. `unstable_cache` keys an entry on keyParts, the JSON of the arguments the cached
function is called with, and the function's source text. A variable the cached function reads from the enclosing
function (`userId`, `orgId`, a session, an awaited route param) is in none of them: the result computed for the first
request is served to every later request, another user's or tenant's included, until it revalidates. `'use cache'`
binds the variables it reads into its key; `unstable_cache` does not.

It matches, where the callee is Next's `unstable_cache` (imported from `next/cache` under any name, or as a member of
`import * as nc from "next/cache"`) and the cached function is written inline or is a function declared in the
enclosing function:
- `unstable_cache(async () => db.invoice.findMany({ where: { userId } }), ["invoices"])()` inside a page or a data
  function, `userId` from `await requireUser()`, a parameter, or `await params`;
- the same with no keyParts at all;
- `const load = async () => db.auditLog.findMany({ where: { teamId } }); unstable_cache(load, ["audit"])()`;
- a derived value whose roots are not all keyed: `const where = { orgId, currency }` read by the cached function with
  keyParts `` [`total-${orgId}`] `` (currency is not keyed);
- a field path keyParts do not cover (code `ClosureKeyField`): the cached function reads `session.user.orgId` while
  keyParts hold only `session.user.id`. A read `a.b.c` is covered by a keyPart reading `a`, `a.b`, `a.b.c` or a
  longer path below it (`a.b.c.toString()`), not by one reading only `a.b.d`: a user who switches organization would
  get the first one's row.

It leaves alone: values passed as arguments (`unstable_cache(async (id) => …, ["org-name"])(orgId)`); values keyParts
read (`["members", teamId]`, `["p", user.orgId]` for a read of `user.orgId`, `[JSON.stringify(session.user)]` for a
read of `session.user.id`; `[cacheKey]` where
`` const cacheKey = `inv-${userId}` ``; `String(skip)`); derived values whose every request root is keyed
(`const where = { orgId, id }` with `["i", orgId, id]`); module-level `unstable_cache(...)` calls (nothing to capture
but module constants and imports); local values with no request in them (`const take = 10`,
`const orderBy = { updatedAt: "desc" }`: no call, await or `new` in the initial value, and no parameter behind it);
functions and classes the cached function reads, and names it only calls (a wrapper `cache(fn, keys)` whose inner
`async (...a) => fn(...a)` is cached under the caller's `keys`); a captured function called with the cached
function's own parameters and nothing else
(`unstable_cache(async (...args) => opts.fn(...args), [...opts.keys])(...opts.args)`: the call's arguments are keyed);
the parameters of a wrapper, a function one of whose parameters is typed as a function
(`fetcher: () => Promise<T>`) that the cached function reads (`cachedQuery(key, fetcher, options)` caching
`() => run(key, fetcher, options)` under `[key]`: its callers choose the computation and build the key; values the
wrapper reads for itself, `await getSession()`, stay reported); `'use cache'` functions; test, story and generated
files.

**Gaps:** what a captured helper function itself reads is not followed; a value that is the same for every request but
built by a call in the function (a client, a config) is reported; a project's own wrapper around `unstable_cache` (a
workspace module that wraps or re-exports it) is not followed to its callers, and in a function taking a function-typed
parameter the cached function reads, a data parameter beside it is not reported; a credential with a deployment
fallback (`tokens.get(entry.id) ?? process.env.TOKEN`) is reported, though it may only pick who pays for the call;
`this` is not read; only initial values are traced, not reassignments.

**Sources:**
- Next.js docs, [`unstable_cache`](https://nextjs.org/docs/app/api-reference/functions/unstable_cache): "keyParts: This is an extra array of keys that further adds identification to the cache. By default, `unstable_cache` already uses the arguments and the stringified version of your function as the cache key. It is optional in most cases; the only time you need to use it is when you use external variables without passing them as parameters. However, it is important to add closures used within the function if you do not pass them as parameters." Its own example caches `async () => { return { id: userId } }` with `[userId], // add the user ID to the cache key`. Same page: "The function passed to `unstable_cache` cannot depend on the incoming request. Read values from `cookies()` or `headers()` before calling the cached function, then pass those values as arguments so they become part of the cache key."
- Next.js source, [`spec-extension/unstable-cache.ts`](https://github.com/vercel/next.js/blob/canary/packages/next/src/server/web/spec-extension/unstable-cache.ts): ``const fixedKey = `${cb.toString()}-${Array.isArray(keyParts) && keyParts.join(',')}` `` and ``const invocationKey = `${fixedKey}-${JSON.stringify(args)}` ``: nothing else enters the key.
- Next.js docs, [`'use cache'`](https://nextjs.org/docs/app/api-reference/directives/use-cache), for contrast: "When a cached function references variables from outer scopes, those variables are automatically captured and bound as arguments, making them part of the cache key."

### `next-practice-revalidate-unknown-tag`

Rerail's own rule, not a port; the tag half of
[`next-practice-revalidate-unknown-target`](#next-practice-revalidate-unknown-target), split off so it can log while
the engine keeps `.ts` and `.tsx` apart. It matches, outside test files, calls of next/cache's `revalidateTag`,
`updateTag` and `unstable_expireTag` (imported by name, renamed, or through `import * as`) whose first argument is a
static string, or a template with a non-empty static start, that no cached read is given. Assigned tags are the
elements of any `tags: [...]` property (fetch's `next.tags`, unstable_cache's options, a wrapper's),
`cacheTag()`/`unstable_cacheTag()` arguments, and the value of a `Vercel-Cache-Tag` response header (or the strings
the function it calls returns), which Vercel's CDN caches under and revalidateTag purges; static or template
prefixes. When an assignment is computed
(`{ tags }`, `tags: list`, `tags: [t]`, `cacheTag(t)`, or `unstable_cache(fn, keys, opts)` with options that are not
an object literal), a tag also counts as known when the same string, or a template with a matching static start, is
written in the file that computes it, or in an argument of a call to the function whose parameter the computed tag is
(`cached(fn, ["teams"], ["teams"])` for `function cached(fn, keys, tags)`). A string written anywhere else (a label, a
route) does not make a tag known.

It leaves alone: tags that are not static (`revalidateTag(tag)`, `` `${prefix}-x` ``); same-named functions that are
not next/cache's; test files and browser-test folders (`e2e/`, `playwright/`, `cypress/`); a route folder named
`test/` is checked.

**Gaps:** a tag assigned only in a `.tsx` file is unseen from a `.ts` call, a false report (the lane gap; the rule's
`lanes` test records it). The common shape is a cached read in `page.tsx` revalidated from a sibling `actions.ts`,
which is why the rule logs until the engine joins the lanes. Tags read through constants
(`revalidateTag(TAGS.posts)`) are not checked.

**Sources:**
- Next.js docs, [revalidateTag](https://nextjs.org/docs/app/api-reference/functions/revalidateTag): "Tags must first be assigned to cached data. You can do this in two ways: Using the `next.tags` option with `fetch` … Using `cacheTag` inside cached functions or components with the `'use cache'` directive" / "Tags are case-sensitive".

### `next-practice-request-api-in-cache-scope`

Rerail's own rule, not a port. A cache scope is a function whose own directive is `'use cache'` or
`'use cache: <handler>'` (`remote`, a custom handler), an exported function of a file whose top-level directive is one
of those, or the function passed to `unstable_cache` from `next/cache` (written inline, or a function it names, local
or imported). Code runs in the scope when it sits in the scope's body or in a function nested in it, other than a
nested `'use server'` function. It reports:
1. a call of `cookies()` or `headers()` from `next/headers`, or `connection()` from `next/server` (named or namespace
   import), that runs in a cache scope; or of an auth library's session read, which calls them inside `node_modules`:
   Auth.js's `auth()` (imported from the workspace file that does `export const { auth } = NextAuth(…)`),
   next-auth's `getServerSession()`, Clerk's `auth()` and `currentUser()` (`@clerk/nextjs/server`), Kinde's
   `getKindeServerSession()`;
2. a call, running in a cache scope, of a workspace function that runs such a call itself or through further calls,
   resolved across files through relative imports, `@/`/`~/` aliases, `index` files, `export { x as y } from` and
   `export * from`, and through React's `cache()` wrapper (`const getSession = cache(async () => …)`); the message
   names the request call it reaches (path and line);
3. the default export of an `app/**/page.*` cached with `'use cache'` that reads its `searchParams` prop
   (`({ searchParams })`, `({ searchParams: sp })`, `props.searchParams`).

It leaves alone: `'use cache: private'` (made for request data); `draftMode()` (reading `isEnabled` is allowed); a
`'use server'` action defined inside a cached component (it runs in its own request); a helper that reads `cookies()`
called from code outside every cache scope, which is the documented fix; a function of the same name from another
module (a local `headers()` helper); `params` in a cached page; a call from one cache scope into another (the inner
scope's own reads are reported where they are); test and story files.

**Gaps:** calls through object methods, callbacks passed as values, dependency injection and package code other than
the auth libraries above (another library's session read inside `node_modules`, a workspace package such as
`@acme/auth`) are not followed; a project's own wrapper of `unstable_cache` (a workspace module wrapping it) is
not a cache scope; `draftMode().enable()`/`disable()` in a cache scope and request data passed unresolved into a
cached function (the prerender stalls) are not reported; a nested function inside a cache scope is taken to run there
even when it is only returned; a local name that shadows an import or a top-level function is not checked. A call from
a `.tsx` file into a `.ts` file (or back), the usual `app/**/page.tsx` → `lib/session.ts` shape, is not followed (the
lane gap).

**Sources:**
- Next.js docs, [`use cache`, Request-time APIs](https://nextjs.org/docs/app/api-reference/directives/use-cache#request-time-apis): "Cached functions and components **cannot** access runtime APIs like `cookies()`, `headers()`, or `searchParams`, and the restriction follows the call stack: a helper the cached function calls that reads one of these fails the same way, with the `next-request-in-use-cache` error. On a dynamically rendered route this surfaces when the route runs, so it can pass `next build` and fail under `next start`. Read these values outside the cached scope and pass them as arguments."
- Next.js docs, [Cannot access request data in `use cache`](https://nextjs.org/docs/messages/next-request-in-use-cache): "APIs such as `cookies()`, `headers()`, `searchParams`, and `connection()` depend on the request and cannot be called inside a function that uses the `use cache` directive." / "You can read draft mode inside a cached function, but enable or disable it outside."
- Next.js docs, [`unstable_cache`](https://nextjs.org/docs/app/api-reference/functions/unstable_cache): "The function passed to `unstable_cache` cannot depend on the incoming request. Read values from `cookies()` or `headers()` before calling the cached function, then pass those values as arguments so they become part of the cache key."
- Next.js docs, [`use cache: private`](https://nextjs.org/docs/app/api-reference/directives/use-cache-private), via the `use cache` page: "For compliance requirements or when you can't refactor to pass runtime data as arguments to a `use cache` scope, see `'use cache: private'`."

### `next-practice-non-serializable-client-prop`

Rerail's own rule, not a port. A Server Component's props to a Client Component, and the props of the DOM elements it
renders, travel as React's Flight data; a function (other than a Server Function or a client reference) or a class
instance (other than Date, Map, Set, typed arrays, ArrayBuffer, DataView, Promise, FormData, Blob, ReadableStream,
Error) cannot, and the route answers 500 at render.

It matches, in a file that renders only as a Server Component, a prop of an element imported from a 'use client'
module (directly, through an `@/`/`~/` alias, an `index.*` file or an `export … from` barrel), of Next.js's own
Client Components (`Link`, `Image`, `Script`, `Form`: the default export of `next/link`, `next/image`, `next/script`,
`next/form`, so `<Link onClick={…}>` from a page), or of a lowercase DOM element, whose value is: an inline arrow,
function expression or object method without a 'use server' body; a name of the file or a named export of another
module that is a plain function (`function f`, `const f = () => …`) in a module that is neither 'use client' nor
'use server'; `new X(…)` or `new a.X(…)` (`new Intl.NumberFormat()`) of any other class (Client Components only); any
of these inside an object or array literal, a ternary, `as`/`satisfies`/`!`, or a `const` of the file holding such a
literal; a render-prop child `<C>{(x) => …}</C>`. A file renders only as a Server Component when it is an App Router
page, layout, template, default, loading, not-found, forbidden or unauthorized file under an `app` directory that is
not itself inside a `pages` directory (`app/docs/pages/page.tsx` is an App Router route; `pages/app/page.tsx` is not),
without 'use client', or such a file imports it (transitively, `import type` aside, never through a 'use client'
file), and no 'use client' module imports it.

It leaves alone: Server Functions (an import from a 'use server' file, an inline `async () => { 'use server'; … }`),
`action.bind(null, id)`, exports of 'use client' modules passed as props (client references), `new Date()`/`new Map()`
and the other serializable built-ins, props between Server Components (`<ServerList renderItem={…}/>`), the same JSX in
a 'use client' file, in a module any 'use client' file imports (shared components may render on the client), under
`pages/`, and in Expo Router's `app/_layout.tsx`.

**Gaps:** Prisma `Decimal` and other class instances that arrive in a query result (no types); values bound to
parameters or call results; spread props; `.bind` on a function that is not a Server Function; Client Components from
other npm packages and workspace packages, `next/dynamic`, or a barrel some client file imports; a shared module
imported from both sides; tsconfig `paths` other than `@/` and `~/`. Any import between a `.tsx` and a `.ts` file is
not followed (the lane gap): a plain function from `lib/*.ts` is missed, and some server-only components reached
through `.ts` barrels are not recognised (fewer matches, not wrong ones).

**Sources:**
- React, [`'use client'`, Serializable types](https://react.dev/reference/rsc/use-client#serializable-types): "Notably, these are not supported: Functions that are not exported from client-marked modules or marked with 'use server' · Classes · Objects that are instances of any class (other than the built-ins mentioned) or objects with a null prototype".
- Next.js docs, [Server and Client Components](https://nextjs.org/docs/app/getting-started/server-and-client-components): "Props passed to Client Components need to be serializable by React."
- Prisma, [issue #25960](https://github.com/prisma/prisma/issues/25960): "Only plain objects can be passed to Client Components from Server Components. Decimal objects are not supported … Query a record from the db where a property is Decimal … Pass that from a server side component to a client side component."

### `next-practice-edge-imports-node-only`

Rerail's own rule, not a port. Next.js runs `middleware.ts` on the Edge runtime (before Next.js 15.5 always, and after
it unless the file exports `config` with `runtime: 'nodejs'`), and so is every route that exports `runtime = 'edge'`.
The Edge runtime has no TCP sockets, file system, native addons or Node built-ins beyond a few polyfills, and
middleware runs on every matched request, so one module it loads that cannot run there takes the site down. The usual
way in is indirect: `middleware.ts` → `auth.ts` → `PrismaAdapter(prisma)` → `lib/db.ts`, where `new PrismaClient()`
throws as the module loads ("PrismaClient is not configured to run in Vercel Edge Functions or Edge Middleware").

An edge file is `middleware.{ts,js,mjs,tsx,jsx}` at the root or in `src/` of a Next.js app package (its own
package.json, the nearest above the file, lists `next` in dependencies, devDependencies or peerDependencies, and it
has `app/` or `pages/`), without `runtime: 'nodejs'` in its exported `config`; or a file under `app/` or `pages/` (or
`src/app`, `src/pages`) of such a package that
exports `runtime = 'edge'` or `config = { runtime: 'edge' | 'experimental-edge' }`, unless its package's `next` range
is 16.3 or later (routes run on Node.js there). From the edge file the rule follows value imports and `export … from`
through the repository's files: relative paths with the `.ts/.tsx/.js/.jsx/.mjs/.mts` and `index.*` candidates (and
`.js` written for a `.ts` file), the `@/` and `~/` aliases to the package's `src/` or root, and workspace packages by
their package.json `exports` (conditions and a trailing `*` pattern included), `main`/`module`, or `src/index`. It
reports, at the edge file's import:
- a module anywhere on the path whose top level runs `new PrismaClient(…)` without an `adapter` option, unless that file
  imports `@prisma/client/edge`, `/wasm` or `@prisma/extension-accelerate` or uses `withAccelerate`;
- a module anywhere on the path that imports a package that fails as it loads: `bcrypt`, `argon2`, `better-sqlite3`,
  `sqlite3`, `sharp` (native addons), `jsonwebtoken` (its `jwa` reads Node's `crypto` as it loads);
- in the edge file itself, a Node built-in the runtime does not polyfill (`fs`, `fs/promises`, `child_process`, `net`,
  `tls`, `dns`, `crypto`, `os`, `path`, `http(s)`, `zlib`, `stream`, … with or without `node:`), or a TCP
  database/cache/mail client (`pg`, `postgres`, `mysql`, `mysql2`, `ioredis`, `redis`, `mongodb`, `mongoose`,
  `nodemailer`, `@prisma/adapter-pg`, `-mariadb`, `-better-sqlite3`);
- a top-level `new PrismaClient()` in the edge file itself.

One finding per import, listing every node-only thing it loads and the file it sits in. The message offers the fix for
the file's Next.js version: for middleware, edge-safe imports (Auth.js split config, `jose`, Web Crypto) or
`runtime: 'nodejs'` (15.5+) or renaming to `proxy.ts` (16); for a route, removing the edge `runtime` setting.

It leaves alone: `import type`, `import { type A }` and `export type … from`; enum or type imports from
`@prisma/client` (`import { Role } from "@prisma/client"`, which loads no engine); `new PrismaClient({ adapter })`;
`new PrismaClient()` inside a function or class (a lazy getter: it throws only when called); Node built-ins and TCP
drivers imported deeper than the edge file (they throw only when called, and the rule cannot tell whether the edge path
calls them); dynamic `import()` (the `process.env.NEXT_RUNTIME` guard pattern); `proxy.ts` (Node.js only); middleware
with `runtime: 'nodejs'`; routes with `runtime = 'nodejs'` or none; edge routes in a Next.js 16.3+ package; a module
merely named `middleware.ts` elsewhere (`lib/middleware.ts`) or in a library package with no `next` of its own (a
monorepo's `packages/auth/src/middleware.ts`); test, story and generated files.

**Gaps:** tsconfig `paths` other than `@/*` and `~/*`, and `baseUrl`-relative bare imports, are not resolved; a `next`
version written `catalog:` or `workspace:*` is unknown, so its edge routes are read as edge; a package's own
`edge-light`/`worker` export condition is not read (only the named packages count); `instrumentation.ts`, which runs on
both runtimes, is not an entry; `require()` and dynamic `import()` are not followed. A `.tsx` edge route that imports a
`.ts` module is missed, and `.mjs`/`.js` files are a third lane (the lane gap).

**Sources:**
- Next.js docs, [Edge Runtime](https://nextjs.org/docs/app/api-reference/edge): "The Edge Runtime does not support all Node.js APIs. Some packages may not work as expected." / "Native Node.js APIs **are not supported**. For example, you can't read or write to the filesystem." / "`node_modules` *can* be used, as long as they implement ES Modules and do not use native Node.js APIs."
- Next.js docs, [Using Node.js Modules in Edge Runtime](https://nextjs.org/docs/messages/node-module-in-edge-runtime): "The code in your Proxy or your Edge API Routes is using a feature from Node.js runtime. However, the Edge Runtime does not support Node.js APIs and globals." / "For example, you might replace the Node.js `crypto` module with the Web Crypto API."
- Next.js docs, [proxy.js](https://nextjs.org/docs/app/api-reference/file-conventions/proxy): "Proxy defaults to using the Node.js runtime." Version history: "`v16.0.0` Middleware is deprecated and renamed to Proxy. Proxy defaults to the Node.js runtime"; "`v15.5.0` Middleware can now use the Node.js runtime (stable)".
- Next.js docs, [Upgrading to version 16](https://nextjs.org/docs/app/guides/upgrading/version-16): "The `edge` runtime is **NOT** supported in `proxy`. The `proxy` runtime is `nodejs`, and it cannot be configured. If you want to continue using the `edge` runtime, keep using `middleware`."
- Next.js docs, [Edge Runtime Deprecated](https://nextjs.org/docs/messages/edge-runtime-deprecated): "Remove the `runtime` export from your route files … The Node.js runtime is the default, so no replacement is needed."
- Vercel docs, [Edge Runtime](https://vercel.com/docs/functions/runtimes/edge): "Starting in Next.js 16.3, setting `runtime = 'edge'` is no longer supported. Routes and pages run on Node.js."
- Auth.js, [Edge Compatibility](https://authjs.dev/guides/edge-compatibility): "Database clients are troublesome in that they often leverage TCP sockets to communicate directly with the database server. One such common database which does this is PostgreSQL." / "Raw TCP sockets are one of those Node.js features that are generally not available to edge runtimes." / "For older versions of Next.js, Middleware code always runs in an edge runtime." / "We would need a separate “version” of next-auth without the database settings for the edge environment and another one with the database for everywhere else."
- Prisma, [prisma/prisma#25234](https://github.com/prisma/prisma/issues/25234) and [discussion #23724](https://github.com/prisma/prisma/discussions/23724): the error "PrismaClient is not configured to run in Vercel Edge Functions or Edge Middleware" from Next.js middleware that reaches a Prisma client.

### `next-practice-client-reaches-server-module`

Rerail's own rule, not a port. It walks the import graph from every 'use client' module outside test code: imports
that load their module at run time (not `import type`, not `import { type X }`, not an import whose names are only used
as types, `ReturnType<typeof f>` included, since TypeScript and SWC drop those) and `export … from` re-exports
(barrels), resolving relative specifiers and the `@/` and `~/` aliases Next.js scaffolds (by file layout: the nearest
directory above the importer that holds the path, or holds it under `src/`), with the `.ts/.tsx/.js/.jsx/.mts/.mjs`
and `index.*` candidates. It does not walk into a 'use server' file (its exports reach the client as Server Action
references) or past a module that imports `server-only`. In every module the walk reaches it reports the import of:
`server-only` (the build fails there; the message names the client chain); a Node.js built-in Next.js does not
polyfill for the browser (`fs`, `fs/promises`, `child_process`, `net`, `tls`, `dns`, `dgram`, `cluster`,
`worker_threads`, `readline`, `module`, `v8`, `inspector`, `async_hooks`, `http2`, `perf_hooks`, with or without
`node:`); a Node-only package (`pg`, `pg-pool`, `postgres`, `mysql2`, `mysql`, `ioredis`, `redis`, `mongodb`,
`mongoose`, `better-sqlite3`, `sqlite3`, `knex`, `bullmq`, `bcrypt`, `argon2`, `nodemailer`, `sharp`, Drizzle's Node
drivers); a Prisma driver adapter (`@prisma/adapter-*`); and `PrismaClient` imported as a value from any specifier
naming prisma (`@prisma/client`, a generated client). The message names one 'use client' module that reaches the file
and its import that leads there.

It leaves alone: the same imports in modules no 'use client' module reaches; type-only imports; 'use server' files
and what they import; `import { Role, Prisma } from "@prisma/client"` (enums and error classes are in Prisma's browser
build); `path` and the other built-ins Next.js polyfills; modules behind a `server-only` module; tests, stories,
fixtures and generated code.

**Gaps:** a `.tsx` component importing a `.ts` module is not followed (the lane gap): the rule sees `.ts` → `.ts` and
`.tsx` → `.tsx` chains only, and both real corpus cases found are `.tsx` → `.ts`. Workspace packages imported by name
(`@acme/db`) and `paths` aliases other than `@/`/`~/` are not followed (a check of five monorepos found only type
imports of their database packages in 'use client' files); dynamic `import()` and `next/dynamic` are not followed;
`sideEffects: false` (which can let the bundler skip an unused barrel member) is not read; secret `process.env` reads
in client modules are not checked; packages are judged by name.

**Sources:**
- Next.js docs, [Server and Client Components](https://nextjs.org/docs/app/getting-started/server-and-client-components): "Once a file is marked with `"use client"`, **all of its imports and the components it directly renders are included in the client bundle**." Same page, Preventing environment poisoning: "JavaScript modules can be shared between both Server and Client Components modules. This means it's possible to accidentally import server-only code into the client." and "To prevent accidental usage in Client Components, you can use the [`server-only` package](https://www.npmjs.com/package/server-only)."
- Next.js blog, [How to Think About Security in Next.js](https://nextjs.org/blog/security-nextjs-server-components-actions): "Verify that database packages and environment variables are not imported outside the Data Access Layer."
- Next.js docs, [Module Not Found](https://nextjs.org/docs/messages/module-not-found): "The module you're trying to import uses Node.js specific modules, for example `dns`, outside of `getStaticProps` / `getStaticPaths` / `getServerSideProps`".
- Prisma source, [the browser stub](https://github.com/prisma/prisma/blob/6.19.3/packages/client-generator-js/src/TSClient/TSClient.ts) (`index-browser`): "This is a stub Prisma Client that will error at runtime if called." … "PrismaClient is unable to run in this browser environment, or has been bundled for the browser".

**Off by default.** Its main shape (a `.tsx` component → a `.ts` module → Prisma) is invisible until the engine joins
`.ts` and `.tsx`, and what it can see today (a `.tsx` → `.tsx` chain to a Node.js built-in) already fails `next build`
with "Module not found"; so it would spend its run time for close to nothing. Turn it on, in log, once the engine
joins the lanes, and re-measure the two cases below.
