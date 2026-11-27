# Rerail rule pack: Next.js

**Linter-equivalent to @next/eslint-plugin-next 16.3.8.** Each rule flags what the original flags, bar declared differences. [What this means](../../docs/rule-packs.md#linter-equivalent).

21 code rules for Next.js projects, ported from `@next/eslint-plugin-next` 16.3.8. Each is scored against the
plugin's own findings on the plugin's rule tests and on their rename variants (`holds-under-rename`), and against the
labelled lines of real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's
production workspace path, by the conformance check ([conformance/README.md](../../conformance/README.md#the-gate)); no
disagreement is declared today, so the pack has no `divergences.json`.

The pack also ships 4 rules of Rerail's own for the App Router, `next-practice-*`, written in `.rerail`, 2 of them on
and 2 off; they are no ports and nothing scores them against a tool
([Rerail's own App Router rules](#rerails-own-app-router-rules)).

The pack follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack next"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; six ported rules and every practice rule carry examples |
| `rules/` | The ported rules' documents (`rerail.policy/v1`, `.json`, generated) and the practice rules (`.rerail`, written by hand; each opens with a comment on what it matches and leaves alone) |
| `cases/<id>/` | Each practice rule's cases, a small repository: a line ending in `// caught` (`# caught` in text files) must be reported, no other line may be (the rule file's `.test cases exactly`). In git, not published |
| [`NOTES-next.md`](NOTES-next.md) | Aggregate acceptance evidence and generic matching limitations; identifying disclosure evidence is private |
| [`ENGINE-GAPS.md`](ENGINE-GAPS.md) | Engine gaps found while writing and measuring the practice rules, with repros and the rules each affects |
| `conformance.json` | Each ported rule's upstream rule, how findings compare, and the capabilities it exercises, the shipped rules first and then the candidates; the pinned plugin |
| `upstream/eslint-plugin-next/` | The plugin's copied rule tests (`tests/`) and the cases extracted from them (`fixtures/`, their rename variants under `fixtures/renamed/`), its expected findings (`fixtures.expected.json`) and case list (`fixtures.cases.json`), the generator (`generate.mjs`), and the lock naming its real-code samples (`samples.lock`) with their sources and licences (`SAMPLES.md`). The tests, cases and expected findings are pack test data, derived from the pins (`npm run fixtures:derive`) and archived in S3; `npm run fixtures:pull` fetches them and the samples |
| `candidates/` | Not published. Ports that cannot pass their fixtures yet or that fail the conformance check (none today; scored through `conformance.json`, never gating), Rerail's own rules that have not passed the bar on apps they were not tuned on (`rules/`, `cases/`, with their details in `candidates/README.md`), and the Next fact model's probes (`probes/`) |
| `licenses/next.js/` | Next.js's licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |

The ported rules' documents, `conformance.json`'s rule list, the ported rules' entries in `pack.json`, the port candidates and the probes are generated, offline, by
`scripts/generate/packs/next-policies.mjs` and `next-probes.mjs` ([scripts/generate/README.md](../../scripts/generate/README.md)):

```sh
node scripts/generate/packs/next-policies.mjs
node scripts/generate/packs/next-probes.mjs
```

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/eslint-plugin-next/publish/`. The recipe
(`scripts/pack-data/recipes/next.ts`) runs `upstream/eslint-plugin-next/generate.mjs` against the pinned plugin (Node
23.6 or later):

```sh
npm run fixtures:derive -- --pack next
npm run fixtures:stage -- --pack next    # copy what changed into upstream/eslint-plugin-next/publish/
```

Scoring one rule, shipped or candidate, with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack next --rule next-no-img-element --fixtures
```

## Rerail's own App Router rules

The 4 `next-practice-*` rules are Rerail's own; none is a port of a linter rule. Each is advice for the App Router
that the Next.js docs give and that no linter checks. A rule was written when at least two sources state the advice or
the failure, one of them the Next.js or React docs (the others are the Next.js source, its issues and blog); when
breaking it causes an outage or a security hole; and when no existing lint rule makes the same check. The tools
checked were `@next/eslint-plugin-next` (whose rules this pack ports), eslint-plugin-react-server-components,
eslint-plugin-import's `no-restricted-paths`, secret scanners (gitleaks, TruffleHog), Semgrep's Next.js patterns and
Next.js's own build errors; ideas that a build error or the type checker already catches were dropped
([NOTES-next.md](NOTES-next.md#dropped-candidates)).

Each fires only in a package that uses Next.js: the nearest package.json above the file lists `next` (dependencies,
devDependencies or peerDependencies); server-action-without-auth also takes a file that imports `next` or a `next/…`
module (`next/cache`, `next/headers`) as Next.js code. A folder named `app/` is no proof (React Router 7, Remix and
Vite apps have one), and `'use server'` is React's; each rule's cases hold a React Router 7 app with the same shapes,
left alone. A shared package whose package.json does not list next is not read, unless the file imports Next.js
itself.

Each rule reads the repository's TypeScript and JavaScript files and follows the repository:
redirect-in-try from a call inside a `try` into the top-level workspace function it calls, to a `redirect()` or
`notFound()` there; revalidate-unknown-target reads the route folders on disk (every `app/` and `pages/` beside a
package.json or in its `src/`); server-action-without-auth from an action into the functions it calls, to a database
write and to an auth check; middleware-not-applied reads the file tree (where `app/` and `pages/` are) and
package.json. redirect-in-try and server-action-without-auth resolve imports through the engine's TypeScript
resolution (relative specifiers, the tsconfig's `paths`; without-auth also barrels); middleware-not-applied asks
whether another file imports the middleware file by a relative path or an alias whose rest ends its path. Each
dialect's run (`.ts`, `.tsx`, `.js`) reads the others' files as text, so a join that needs another dialect's syntax
tree is missed ([ENGINE-GAPS.md](ENGINE-GAPS.md)); each rule's details below say what it misses.

| Rule | Starts | Mode | Catches | Test set #3: 41 apps | Test set #2: 37 apps | Test set #1: 20 apps | Tuning: matches (repos) on the 33 corpus apps | Held-out: 8 apps |
|---|---|---|---|---|---|---|---|---|
| `next-practice-redirect-in-try` | on | Steer | `redirect()`, `notFound()` and the like inside a `try` whose `catch` swallows them, or a `.then()` whose `.catch()` does | 6 of 6 real (1 app) | 2 of 2 real (1 app) | 0 | 4 (2), 4 of 4 real | 0 |
| `next-practice-revalidate-unknown-target` | on | Steer | `revalidatePath` of a route the app does not have or of an untyped pattern | 19 of 19 real (2 apps) | 0 | 0 | 28 (3), 28 of 28 real | 1, unclear (a locale-less path under an i18n rewrite) |
| `next-practice-server-action-without-auth` | off | Steer | a Server Action that writes to the database and reaches no auth or session check | 13, 8 real (62%; 5 apps) | 34, 30 real (88%); tuning data since its fixes | 9 of 9 real (1 app) | 3 (1), 3 of 3 real | 1, not real (an action gated by a token it looks up) |
| `next-practice-middleware-not-applied` | off | Steer | a middleware.ts or proxy.ts where Next.js never loads it, or a proxy.ts before Next.js 16 | 1, not real | 0 | 1 of 1 real | 0 | 0 |

Matches are observed findings, not proof of exploitability. [Measurement notes](NOTES-next.md) retain aggregate
decisions and generic false-match shapes. App identities, commits and source locations are kept privately for
coordinated disclosure.

redirect-in-try and revalidate-unknown-target start on and steer: every match on apps they were not tuned on was real
(8 of 8 in 2 apps; 19 of 19 in 2 apps). server-action-without-auth and middleware-not-applied ship off, below the bar
on apps they were not tuned or fixed on: 8 of 13 real (62%) and 1 of 2 (50%). Each rule's details say which shapes
were false. To change a rule's mode or turn it on or off, use its switch in the Next.js pack's page in the dashboard,
or `repository_settings_set` ([rule scoping](../../docs/rule-scoping.md#management-surfaces)).

### How the rules were measured

Five sets were kept apart: a 33-app tuning corpus, 8 held-out generalization apps, then frozen sets of 20, 37 and
41 apps. A set becomes tuning data for a rule after its false matches inform a fix.

The bar: a rule ships on only when more than 90% of its matches on apps it was never tuned or fixed on are real. Per
rule, test set #1 counts when the rule was not tuned on it, test set #2 when it was not fixed against its findings,
and test set #3 always. A rule below the bar ships off, its precision and false shapes stated. Promoting a candidate,
or keeping a rule on, takes at least 5 judged matches in at least 2 apps; a candidate under that floor with no false
match stays a candidate. After test set #3, redirect-in-try and revalidate-unknown-target moved from `candidates/` and
start on, server-action-without-auth and middleware-not-applied fell below the bar and ship off, and the other 8 App
Router rules wait in `candidates/` ([candidates/README.md](candidates/README.md#candidates)).

Not run: lobehub/lobe-chat on test set #1, where the run stopped at the engine's content-store hard limit after 933 s
([ENGINE-GAPS.md](ENGINE-GAPS.md#17-a-large-app-fills-the-content-store)); chatbotxio/chatbotx on test set #2 and
osmosis-labs/osmosis-frontend on test set #3, stopped past the 5 GB memory guard or 50 minutes. Each set's numbers are
for its other apps.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

The 21 sections that follow first are ports of `@next/eslint-plugin-next` 16.3.8 (rule `@next/next/<name>`). Note: the rule documents are generated by `scripts/generate/packs/next-policies.mjs`, which prefixes each description with the upstream version and rule code (line ~140); re-running it overwrites the rewritten descriptions unless the generator is updated too. The 4 `next-practice-*` sections after them are Rerail's own rules.

### `next-google-font-display`

Matches a `<link>` whose literal `href` is a Google Fonts CSS URL (`https://fonts.googleapis.com/css...`) whose query has no `display` parameter, or has `display=auto`, `block` or `fallback`.

### `next-google-font-preconnect`

Matches a `<link>` whose literal `href` starts with `https://fonts.gstatic.com` and whose `rel` is not the literal `"preconnect"`.

### `next-inline-script-id`

Matches an element named by the first local name that an import from `next/script` binds, with children or `dangerouslySetInnerHTML` and no `id`. Attributes and the keys of object-literal spreads count; any other spread makes the element uncheckable (no finding).

### `next-next-script-for-ga`

Matches a `<script>` with a literal `src` containing the Google Analytics or gtag URL, or whose `dangerouslySetInnerHTML` object's first property is a template literal whose first quasi contains the GA or GTM script URL.

### `next-no-assign-module-variable`

Matches any variable declaration with a declarator whose identifier is `module`, including a `for…of` or `for…in` head such as `for (const module of mods)`, reported at its keyword as upstream does.

### `next-no-async-client-component`

Only after a top-level `"use client"` statement. Matches an `export default async function` with a capitalized name, or the first top-level declaration of the capitalized name that `export default` exports when it is an async function or a variable initialized with an async arrow.

### `next-no-before-interactive-script-outside-document`

Matches an opening element named by the default import of `next/script` with a plain `strategy="beforeInteractive"`, unless the path contains `/app/` or its segment after the first `pages` has a name starting with `_document`.

### `next-no-css-tags`

Matches a `<link>` with `rel="stylesheet"` and a string `href` that does not start with `http(s)`.

### `next-no-document-import-in-page`

Matches an import from `next/document` unless the part of the path after its last `pages` is empty or starts with `/_document`.

### `next-no-duplicate-head`

Flags every `<Head>` child of a returned element after the first, in a `return` inside a class extending the default import of `next/document`.

### `next-no-head-element`

Matches a `<head>` opening element unless the file's path contains `app/`.

### `next-no-head-import-in-document`

Matches an import from `next/head` in a file whose path, between its first and second `pages`, names a document (name starts with `_document`, or `/_document/index`).

### `next-no-img-element`

Matches an `<img>` opening element with attributes that is not a direct child of `<picture>`, unless the file is an app-dir metadata image route (path relative to the ESLint cwd matches `^(src/)?app/` and `/opengraph-image|twitter-image|icon.<ext>`). The violation message is upstream's plus Rerail additions: what `<Image />` needs (`width` and `height` or `fill`; a matching `images.remotePatterns` entry for a remote `src`) and the eslint-disable comment that keeps `<img>`.

### `next-no-location-assign-relative-destination`

Matches an assignment to `location.href` (or `window`/`globalThis`/`document`/`self` `.location.href`) or a call of its `assign()`, whose value's static string prefix is a relative URL.

### `next-no-page-custom-font`

Matches a `<link>` whose literal `href` is a Google Fonts CSS URL, unless the file is `pages/_document` (last `pages` piece starts with `/_document`) and the link is inside the default-exported component. Two clauses: outside `_document` ("only loads for a single page"), and inside `_document` but outside the default-exported component ("outside `<Head>` disables font optimization").

### `next-no-script-component-in-head`

Matches a `<Head>` element with a direct `<Script>` element child, in a file that imports `next/head`.

### `next-no-styled-jsx-in-document`

Matches a `<style>` opening element with a `jsx` attribute in a file whose path, between its first and second `pages`, names a document.

### `next-no-sync-scripts`

Matches a `<script>` opening element with a `src` attribute and neither `async` nor `defer`.

### `next-no-title-in-document-head`

Matches the first `<title>` element child of a `<Head>` element, in a file with an import from `next/document` that binds the local name `Head`.

### `next-no-typos`

Matches an exported function or variable whose name is one edit away from `getStaticProps`, `getStaticPaths` or `getServerSideProps`, in a file whose segment after the first `pages` is non-empty and not under `/api`.

### `next-no-unwanted-polyfillio`

Matches a `<script>` (or the first local name an import from `next/script` binds) with a plain string `src` on a polyfill.io host whose first `features` parameter lists a feature Next.js already polyfills.

### `next-practice-server-action-without-auth`

**Ships off:** on test set #3, the only set it was neither tuned on nor fixed against, 8 of its 13 matches were real
(62%, in 5 apps), below the 90% bar.

Rerail's own rule, not a port. Next.js makes every function a `'use server'` file exports, and every function whose
body starts with `'use server'`, an endpoint anyone can call with a direct POST request. The page's or layout's auth
check does not run for that request, and a middleware (proxy) matcher can leave the action's path out. An action that
writes rows must find out who is calling inside itself.

It matches, outside test code, a Server Action that writes to the database and reaches no auth or session check:
- a function a top-level `'use server'` file exports (`export async function f`, `export const f = async () =>`,
  `export { f }`), or an inline action (`async function save(fd) { 'use server'; … }` in a Server Component);
- an action built by an action client or wrapper (`export const f = actionClient.schema(s).action(async …)`,
  `export const f = action(schema, async …)`) whose client resolves to a workspace declaration whose own code checks
  no session either (code `ServerActionWithoutAuthWrapped`).

It writes when it, or a workspace function or declaration it calls (by name, through relative imports, tsconfig `paths`
aliases and barrels, at any depth), makes a Prisma write (`<client>.<model>.create|createMany|createManyAndReturn|
update|updateMany|updateManyAndReturn|upsert|delete|deleteMany` with an object argument holding `where`, `data`,
`create` or `update`; `$executeRaw`, `$executeRawUnsafe`) or a Drizzle write (`db.insert(t).values(…)`,
`db.update(t).set(…)`, `db.delete(t).where(…)`). One finding per action, at its name, naming one write and, when the
write is in another function, that function (code `ServerActionWithoutAuthVia`).

**Sources:**
- Next.js docs, [Data Security](https://nextjs.org/docs/app/guides/data-security): "By default, when a Server Action is created and exported, it is reachable via a direct POST request, not just through your application's UI." / "A page-level authentication check does not extend to the Server Actions defined within it. Always re-verify inside the action" / "you should still treat Server Actions as reachable via direct POST requests and verify authentication and authorization inside each one."
- React, [`'use server'`](https://react.dev/reference/rsc/use-server): "Arguments to Server Functions are fully client-controlled. … In any Server Function, make sure to validate that the logged-in user is allowed to perform that action."
- Next.js docs, [proxy.js](https://nextjs.org/docs/app/api-reference/file-conventions/proxy): "a Proxy matcher that excludes a path will also skip Server Function calls on that path. … Always verify authentication and authorization inside each Server Function rather than relying on Proxy alone."
- Next.js blog, [CVE-2025-29927](https://nextjs.org/blog/cve-2025-29927): "We do not recommend Middleware to be the sole method of protecting routes in your application."
- Next.js blog, [How to Think About Security in Next.js](https://nextjs.org/blog/security-nextjs-server-components-actions): '"use server" files. Are the Action arguments validated in the action or inside the Data Access Layer? Is the user re-authorized inside the action?'

### `next-practice-middleware-not-applied`

**Ships off:** of its 2 matches on apps it was not tuned on, 1 was real (test set #1 1 of 1, test set #3 0 of 1:
50%), below the 90% bar and under the 5-match floor.

Rerail's own rule, not a port. Next.js runs one middleware file per app (`middleware.ts`, renamed `proxy.ts` in
Next.js 16), and only from one folder: the parent of the `app`/`pages` directory it found, which is the project folder
when `app/` or `pages/` sits there and its `src/` otherwise (Next.js's `findDir` prefers `./app` and `./pages` over
`./src/app` and `./src/pages`; its build scans only that folder for `middleware.<ext>`/`proxy.<ext>`). A file anywhere
else is an ordinary module, and a `proxy.ts` on Next.js before 16 is not a convention file at all; in both cases there
is no error and no warning, and every redirect, auth check, rewrite or security header in it is missing.

A Next.js project is a folder holding `next.config.{js,mjs,cjs,ts,mts}`, or a package.json with `next` in
`dependencies`/`devDependencies`; its `app`/`pages` directories are read from the file tree. A file counts as meant for
Next.js when it is named `middleware.{ts,tsx,js,jsx}` or `proxy.{ts,tsx,js,jsx}`, exports a default or a binding of
its own name (`export function middleware`, `export const proxy =`, `export { auth as middleware } from "./auth"`),
exports `config` or imports `next/server`, `next-auth*`, `@auth/*`, `next-intl/middleware` or `@clerk/nextjs*`, and no
other file of the run imports it. The rule reports it, at that export:
- in the project folder when the app lives in `src/app` or `src/pages` (Next.js reads only `src/middleware.ts`);
- in `src/` when the app lives in `./app` or `./pages` (Next.js reads only `./middleware.ts`);
- anywhere inside the app directory (`app/middleware.ts`, `src/app/(dashboard)/middleware.ts`);
- a correctly placed `proxy.ts` whose project's package.json pins `next` to a range entirely below 16 (`^15.5.2`,
  `14.2.3`; a node-semver subset of `<16.0.0-0`; 15.6.0 canaries are left alone, since `proxy` landed there).

It leaves alone: a correctly placed file (either name on Next.js 16, which still loads the deprecated
`middleware.ts`); Pages Router apps with a root `middleware.ts`; helpers that another file imports (relative imports
resolved by path, aliases such as `@/app/middleware` by the end of the path); files that export neither a default nor
their own name (`proxy.ts` exporting `proxyToApi`); files in folders Next.js never scans (`lib/middleware.ts`,
`docker/proxy.ts` that a Dockerfile copies into place, `packages/*/src/middleware.ts`); projects whose next.config sets
`pageExtensions`; a `proxy.ts` whose `next` version is not a semver range (`catalog:`, `workspace:*`, `latest`).

**Gaps:** a non-literal `config.matcher` is not reported: Next.js 15 and later throw in dev and fail the production
build on it, so it is not silent. Custom `pageExtensions` (`middleware.page.ts`): the project is skipped, and the check
reads the next.config document, so it sees a `next.config.ts` only from a `.ts` middleware's run (beside a
`next.config.mjs` such a project could be reported); likewise a `.tsx` file importing `middleware.ts` is not seen as
an import. A project with neither next.config.* nor a package.json naming next; aliases that do not start with `@`,
`~` or `#`; dynamic `import()` of the file; a `next` version set only by a workspace catalog.

**Sources:**
- Next.js docs, [proxy.js](https://nextjs.org/docs/app/api-reference/file-conventions/proxy): "Create a proxy.ts (or .js) file in the project root, or inside src if applicable, so that it is located at the same level as pages or app." / "If you've customized pageExtensions, for example to .page.ts or .page.js, name your file proxy.page.ts or proxy.page.js accordingly." / Version history: "v16.0.0 Middleware is deprecated and renamed to Proxy."
- Next.js source, [`lib/find-pages-dir.ts`](https://github.com/vercel/next.js/blob/canary/packages/next/src/lib/find-pages-dir.ts): "// prioritize ./${name} over ./src/${name}"; and [`build/index.ts`](https://github.com/vercel/next.js/blob/canary/packages/next/src/build/index.ts): `const rootDir = path.join((pagesDir || appDir)!, '..')` … `const isAtConventionLevel = normalizedFileDir === '/' || normalizedFileDir === '/src'`.
- Next.js source, [v15.5.0 `lib/constants.ts`](https://github.com/vercel/next.js/blob/v15.5.0/packages/next/src/lib/constants.ts) defines only `MIDDLEWARE_FILENAME = 'middleware'`; `PROXY_FILENAME = 'proxy'` first appears in the 15.6.0 canaries and 16.0.0.

**Held-out and test set #1.** No match on the 8 held-out apps. On test set #1, 1 match, real: steven-tey/precedent
`app/middleware.ts`, inside the app directory, where Next.js never loads it.

### `next-practice-redirect-in-try`

Rerail's own rule, not a port. `redirect()`, `permanentRedirect()`, `notFound()`, `forbidden()` and `unauthorized()`
from `next/navigation` never return: they throw a Next.js control-flow error that Next.js itself catches to send the
redirect or the error page. A `try` around the call whose `catch` does not rethrow swallows it: a Server Action returns
its error result instead of redirecting, a page renders its error UI instead of the 404, and an auth redirect falls
through to the code after the try/catch.

It matches:
- a call of one of the five functions (imported by name, renamed, or through `import * as nav`) in a `try` body whose
  `catch` neither throws its own parameter nor names `unstable_rethrow`, `isRedirectError`, `isNotFoundError`,
  `isNextRouterError`, `isHTTPAccessFallbackError`, reads the caught error's `digest` (`e.digest`, not
  `hash.digest("hex")`), or holds a `NEXT_REDIRECT`/`NEXT_NOT_FOUND`/`NEXT_HTTP_ERROR` string;
- the same call in a callback the try runs before going on (`.map`/`.forEach`/… callbacks) or awaits
  (`await res.then((r) => notFound())`, `await prisma.$transaction(async (tx) => redirect(…))`);
- a catch that throws a different error (`throw new Error("failed")`): the redirect becomes an error page;
- a catch that returns under an `instanceof Error` guard before its rethrow (`if (e instanceof Error) return { error:
  e.message }; throw e`), or rethrows only non-Errors (`if (!(e instanceof Error)) throw e`): Next.js's control-flow
  errors are `Error`s, so they are swallowed;
- an inner try that rethrows inside an outer try that swallows;
- the same call in a `.then()` callback of a promise chain whose next `.catch()` callback (a function literal) does
  not rethrow (`await save().then(() => redirect(…)).catch(() => null)`), with its own message (code
  `RedirectInPromiseCatch`);
- `redirect()`/`permanentRedirect()` in such a try inside a callback of a `'use client'` file (an event handler such
  as `onDelete`): reported with its own message (code `RedirectInClientHandler`), which steers to `useRouter().push()`
  in the success path, since Next.js supports `redirect` in Client Components only while rendering;
- a call, in such a try body, of a top-level workspace function whose own body calls one of the five
  (`await requireUser()` where `lib/auth.ts#requireUser` redirects to /login), declared in the same file or imported
  through a relative path or the tsconfig's `paths` (the engine's TypeScript resolution, not through a re-export).

It leaves alone: `catch (e) { unstable_rethrow(e); … }`, `catch (e) { if (isRedirectError(e)) throw e; … }`, and any
catch that rethrows its parameter (`if (e instanceof ZodError) return …; throw e`); a catch that ends by calling the
same function again (`try { if (!ok) notFound() } catch { notFound() }`); a `.catch()` callback that rethrows; the
call after the try/catch, in the `catch` or `finally`, or in a `try … finally` with no catch; a closure the try only
defines (`const go = () => redirect("/")`); `NextResponse.redirect(url)` and other libraries' `redirect` (only
`next/navigation`'s throws); workspace helpers that never call the five functions; tests, stories, fixtures and
generated code.

**Gaps:** a catch that rethrows through a helper (`handleError(e)` that throws `e`) counts as swallowing; a promise
`.catch(handler)` is read only when its callback is a function literal; a rethrow anywhere in the catch counts as
rethrowing, even when it only covers other errors (the `instanceof Error` guard aside); helpers are followed one call
deep and only into top-level functions; next-auth's `signIn`/`signOut` (which also throw `NEXT_REDIRECT`) and a
`finally` that returns are not seen; a helper in a `.ts` module called from a `.tsx` file is missed (the lane gap).

**Sources:**
- Next.js docs, [`redirect`](https://nextjs.org/docs/app/api-reference/functions/redirect): "In Server Actions and Route Handlers, redirect should be called **outside** the `try` block when using `try/catch` statements." and "`redirect` throws an error so it should be called **outside** the `try` block when using `try/catch` statements."
- Next.js docs, [`unstable_rethrow`](https://nextjs.org/docs/app/api-reference/functions/unstable_rethrow): "calling the `notFound` function will throw an internal Next.js error and render the `not-found.js` component. However, if used inside the `try` block of a `try/catch` statement, the error will be caught, preventing `not-found.js` from rendering" and "This method should be called at the top of the catch block, passing the error object as its only argument. It can also be used within a `.catch` handler of a promise."
- [vercel/next.js#55586](https://github.com/vercel/next.js/issues/55586), "Next Redirect - Server action redirects do not execute when inside try / catch blocks"; [vercel/next.js#59930](https://github.com/vercel/next.js/issues/59930), "redirect inside try/catch block in server action throws NEXT_REDIRECT error".

### `next-practice-revalidate-unknown-target`

Rerail's own rule, not a port. It matches, outside test files, calls of next/cache's `revalidatePath` (imported by
name, renamed, or through `import * as`), in two shapes (the tag half, `revalidateTag`/`updateTag` of a tag nothing
assigns, is the candidate
[`next-practice-revalidate-unknown-tag`](candidates/README.md#next-practice-revalidate-unknown-tag), split off so it can log
while the engine keeps `.ts` and `.tsx` apart):
- **A path no route renders** (code `UnknownPath`). The first argument of `revalidatePath` is a static string starting
  with `/` (a file-local constant too) or a template whose substitutions hold no `/`. It is matched against the route
  files on disk: every `app/` and `pages/` folder where Next.js looks for one, beside a package.json or in that
  package's `src/`, at any depth (dot, dependency, build and `public/` folders aside), whose parent folder holds the
  calling file (private `_folders`, dot folders and `node_modules` skipped).
  A folder counts when it holds `page`, `route` (a URL renders it) or `layout`, `template`, `default`, as
  `.js/.jsx/.ts/.tsx` or `.md/.mdx`; under `pages/` every non-`_` file is a page. Without a type, or with `'page'`, the
  path must be a page or route handler; with `'layout'`, any folder holding route files or above one. `(group)` and
  `@slot` folders are no URL segment, but a pattern may name a group exactly (`/(site)/blog/[slug]`); `[x]` takes one
  segment (not a `(group)` the path names), `[...x]` one or more, `[[...x]]` none or more; a `${…}` path segment takes
  one or more route segments; a trailing `/` is ignored. Reported when nothing matches and at least one route was
  found for the call.
- **An untyped pattern** (code `UntypedPattern`). A path (static, or a template's static segments) holding a
  `[segment]` with no second argument: Next.js warns "This has no effect by default".

It leaves alone: paths that are not static (`revalidatePath(path)`, `` `${base}/x` ``, nested templates,
`revalidatePath(p, type)` with a computed type); `/blog/page` when `[slug]` may be `"page"`; a call with no `app/` or
`pages/` above it (a shared package whose app lives elsewhere); same-named functions that are not next/cache's; test
files and browser-test folders (`*.test.*`, `__tests__/`, `e2e/`, `playwright/`, `cypress/`). A route folder named
`test/` or `tests/` is app code and is checked.

**Gaps:** locale prefixes a middleware adds or rewrites (`/invoices/1` matches `app/[locale]/invoices/[id]`, `[locale]`
taking `"invoices"`; with an i18n middleware's rewrite strategy, `/teams` against `[locale]/(app)/teams` is reported);
rewrites in next.config (a source path is reported only when no route happens to match it); a `[slug]` pattern
against an `[id]` folder counts as a match; an `app/` in a folder without a package.json, or more than twelve folders
down (a bound the engine needs to see the walk end). Routes are read from disk, so this rule has no lane gap.

**Sources:**
- Next.js docs, [revalidatePath](https://nextjs.org/docs/app/api-reference/functions/revalidatePath): "Do not append `/page` or `/layout`, use the `type` parameter instead." / "If `path` contains a dynamic segment, for example `/product/[slug]`, this parameter is required." / "When using rewrites, you must pass the **destination** path (the actual route file location), not the source path that appears in the browser's address bar." / "This is because `revalidatePath` operates on the route file structure, not the URL visible to users."
- Next.js source, [spec-extension/revalidate.ts](https://github.com/vercel/next.js/blob/canary/packages/next/src/server/web/spec-extension/revalidate.ts): "Warning: a dynamic page path "${originalPath}" was passed to "revalidatePath", but the "type" parameter is missing. This has no effect by default".

## The corpus

App identities, source commits and finding locations are retained privately for coordinated disclosure. Aggregate evidence is given above.

