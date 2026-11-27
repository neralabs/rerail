# Rerail rule pack: Prisma, candidates

The rules here were written for the pack, pass their cases, and do not ship: `pack.json` does not list them. This file
is not published; the pack's [README](../README.md) covers its five shipped rules, and
[NOTES-prisma.md](../NOTES-prisma.md) records aggregate measurement evidence.

Promotion requires more than 90% real findings on unseen apps, with at least five judged matches across two apps.
Rules below that threshold or without enough evidence remain candidates. The latest sets contain 37 and 41 apps;
global-client-in-transaction has 3 of 3 real findings in one app, read-modify-write 2 of 2 in one app,
sensitive-fields-to-client 4 of 7, bulk-write-without-where 0 of 1, and dev-migration-in-deploy 0 of 2.

Each rule file keeps its `.test` blocks; their case paths (`../cases/<id>/…`) resolve to `candidates/cases/<id>/`, and
their `#include`s are the engine's own `tsx/` libraries. Run one with `rerail rules test candidates/rules/<id>.rerail`.

## Candidates

The corpus is the 28 Prisma apps the rules were tuned on ([README](../README.md#the-corpus)); the held-out apps are 8
apps used to generalize them, measured before the generalization round's changes. "Real" is the reader's verdict.

| Rule | Catches | Test set, 16 apps | Held-out, 8 apps | Corpus, 28 apps |
|---|---|---|---|---|
| `prisma-practice-global-client-in-transaction` | a query through the outer Prisma client, or a helper that uses it, inside an interactive `$transaction` callback | 0 matches | 0 matches | 7 in 1 app, 7 real |
| `prisma-practice-unawaited-transaction-query` | a `tx` query nothing awaits, or one started from `forEach(async …)` or a timer, inside a `$transaction` callback | 0 matches | 0 matches | 0 matches (6 of 6 adversarial shapes right) |
| `prisma-practice-bulk-write-without-where` | a `deleteMany`/`updateMany` on a schema model with no filter, or one whose filter rests on a value that may be undefined | 0 matches | 0 matches | 1, not real |
| `prisma-practice-sensitive-fields-to-client` | a whole row of a model with a password hash, token or secret column sent to the browser | 0 matches | 1 of 2 real | 20 in 5 apps, 10 real |
| `prisma-practice-dev-migration-in-deploy` | `prisma migrate dev`, `migrate reset` or a data-losing `db push` in a command a deploy runs | 0 matches | 0 of 1 real (debatable) | 0 matches |
| `prisma-practice-read-modify-write` | a counter, stock or balance written back as arithmetic on the value read earlier | 0 matches | 2 of 2 real | 7 in 4 apps, 5 real |
| `prisma-practice-find-first-without-order-by` | a `findFirst` whose `where` pins no unique field and that has no `orderBy` | not run | not run | 86 in 3 of 8 apps queued, 9 real (10%) |

Held-out after the generalization round (final rule logic, the runs that finished before local runs stopped):
read-modify-write and side-effect-in-transaction 0 matches on Dokploy and Openpanel; one-client 0 on Dokploy,
Openpanel and dub (dub's driver-adapter client, the one held-out match, is now left alone); the whole pack 0 on
next-forge and Dokploy. A rule here ships once it has matches on apps it was not tuned on and more than 90% of them are
real.

## Rule details

What each rule matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources.

### `prisma-practice-global-client-in-transaction`

Rerail's own rule, not a port. An interactive transaction, `<client>.$transaction(async (tx) => …)`, runs its queries on
one connection, through `tx`. A query the callback makes through the outer client goes out on another pooled
connection, outside the transaction: a write commits even when the callback throws and the transaction rolls back, a
read misses the transaction's uncommitted writes, and a write to a row the transaction has locked waits for it until
the interactive transaction times out (P2028). Inside the body of a function written inline as the first argument of
`<r>.$transaction(…)`, or of a callback an array method there runs at once (`ids.map((id) => …)` under `Promise.all`,
forEach, flatMap, filter, reduce, some, every, find), it reports:

- a query through the same client r: `r.<model>.<op>(…)` (findUnique … groupBy and their OrThrow forms, the
  createManyAndReturn/updateManyAndReturn forms), `r.$queryRaw`/`$executeRaw` (tagged, Unsafe or Typed),
  `r.$runCommandRaw(…)`, and a nested `r.$transaction(…)`, which is a transaction of its own. "The same client" is the
  same binding (`prisma`, an import of it), the same member chain on the same binding (`ctx.db`, `this.prisma`), or the
  same global name;
- a call `g(…)` of a project function (same file, or imported one hop through a relative or `@/`/`~/` import, barrels
  included) whose body queries that same client by its module binding (matched by the exported symbol across files; a
  client re-exported under another name or as `export default prisma` counts), when the call passes nothing that names
  the callback's parameters;
- both of the above for a callback that takes no parameter (`$transaction(async () => …)`), where every query runs
  outside the transaction.

The message names the query (or the helper and the query inside it, with its file and line), says what goes wrong,
and gives the fix: call it on the callback's `tx` (adding the parameter if missing), or give the helper a client
parameter (`client: Prisma.TransactionClient = prisma`) and pass `tx`. A write that must survive a rollback on purpose
(an audit or failure record) belongs after the `$transaction` call or in its catch.

**Gaps:** a callback passed by name (`$transaction(work)`); helpers reached through a method (`this.svc.f()`), an
object (`repo.f()`), two hops or more, or a default-exported function; a helper defaulting to the client and called
without `tx` (`recordEvent("x")`); the outer client passed as an argument (`f(prisma)`); a workspace package imported
by name in one file and by a relative path in the other (`@repo/db` vs `./client`); a second client of the same
database under another name; tsconfig `paths` other than `@/` and `~/`; a `.tsx` file calling a helper in a `.ts`
module, until the engine joins its `.ts` and `.tsx` lanes ([ENGINE-GAPS.md](../ENGINE-GAPS.md)); the direct form is
single-file and unaffected; a query in a `.then(…)` callback of an awaited promise, or in a callback a wrapper runs at
once (`withRetries(() => …)`), which does run inside the transaction but is not followed. Prisma 8's
`db.transaction(async (tx) => …)` and `tx.orm.<schema>.<Model>` are not matched yet; its docs keep the same contract
("Pass `tx` down instead").

**Sources:**
- Prisma's agent skill, [prisma-client-api: transactions](https://github.com/prisma/skills/blob/main/prisma-client-api/references/transactions.md): "The `tx` parameter is a Prisma Client scoped to the transaction … `// Use tx instead of prisma`".
- [prisma/prisma#17215](https://github.com/prisma/prisma/issues/17215): "Despite throwing the error inside the interactive transaction, the post is still created, because `createPost()` references the global `PrismaClient` object (`prisma`) instead of accepting a `TransactionClient` (`tx`) as an argument. … (b) it's easy to make this mistake; (c) it's hard to catch this error in code reviews."
- Prisma ORM 8 docs, [Transactions](https://www.prisma.io/docs/orm/fundamentals/transactions) (same contract): "Do not call `db.transaction(...)` inside the helper: it does not nest, so the inner call is a separate transaction that commits or rolls back on its own, and the outer one cannot roll it back. Pass `tx` down instead."

### `prisma-practice-unawaited-transaction-query`

Rerail's own rule, not a port. Inside an interactive transaction, `<client>.$transaction(async (tx) => …)`, Prisma
commits when the callback's promise settles, so every query on `tx` must finish before the callback does. A Prisma
query is a lazy thenable: one that nothing awaits or `.then()`s never runs at all; one that is started but not waited
for runs after the commit and fails with P2028 "Transaction already closed", or its write is lost while the caller
sees success. The transaction client is the first parameter of a function literal passed first to `.$transaction(…)`
(on any receiver: the client, an extended client, `this.prisma`), or a parameter whose type annotation mentions
`TransactionClient` (`tx: Prisma.TransactionClient`, `PrismaTransactionClient`) in a helper. A query is
`tx.<model>.<op>(…)` with op one of Prisma's model operations (findUnique, findFirst, findMany and their OrThrow forms,
create, createMany, createManyAndReturn, update, updateMany, updateManyAndReturn, upsert, delete, deleteMany, count,
aggregate, groupBy), or `tx.$queryRaw`/`$executeRaw` (Unsafe and Typed forms too), tagged or called. It reports, with a
message per shape:

- a query whose promise nothing reads: an expression statement (`tx.cart.delete({…});`), `void tx…`, a `.then()`/
  `.catch()`/`.finally()` chain on it, a `Promise.all|allSettled|race|any` over it, or an array `.map()`/`.flatMap()`
  whose callback is async and runs it or is a concise arrow returning it, when that expression is itself a statement;
- `xs.forEach(cb)` whose callback is async and runs a tx query, or is a concise arrow returning one;
- `setTimeout`, `setImmediate`, `queueMicrotask` or `process.nextTick` whose callback runs a tx query.

It leaves alone: `await tx…`, `return tx…`, a concise arrow body; `await`ed or returned `Promise.all(xs.map(…))`; a
`for…of` loop that awaits; a query kept in a variable (`const p = tx.a.create(…)`) or pushed into an array for a later
`Promise.all`; forEach callbacks that do not query tx; the array form `$transaction([prisma.a.create(…), …])`; the
outer client used after the transaction (`forEach(async …)` on `prisma.…` once `$transaction` has resolved); a
parameter named `tx` whose type is not Prisma's; tests, stories and generated code.

**Gaps:** a helper called without `await` and handed `tx` (`audit(tx, "paid");`), since without types the rule cannot
tell an async helper from a sync one; helper parameters not typed `…TransactionClient` (`tx: any`, `tx: typeof db`);
tx aliased to another name; a promise kept in a variable that is never awaited; a nested parameter shadowing `tx` is
still read as tx; Next.js `after()` and other deferring APIs; model methods added by a client extension; Prisma 8,
where queries are `tx.orm.<schema>.<Model>.<op>(…)` and a second call on tx while one is in flight is refused, so
`await Promise.all` over tx queries is wrong there too. typescript-eslint's `no-floating-promises` and
`no-misused-promises` (`checksVoidReturn`) catch the floating and `forEach(async …)` shapes with type information, and
are often off in Next.js projects for speed; this rule needs no types and is scoped to the transaction, where the bug
loses data.

Why Steer with no matches to read: each reported shape is wrong by Prisma's documented semantics whatever the code
around it, the client is recognised only through Prisma's own names, and the message's fix (await it, return it,
`for…of`, `await Promise.all`) is right for every shape.

**Sources:**
- Prisma Client API reference, [`PrismaPromise` behavior](https://www.prisma.io/docs/orm/v7/reference/prisma-client-reference#prismapromise-behavior): "All Prisma Client queries return an instance of `PrismaPromise`. This is a "thenable", meaning a `PrismaPromise` only executes when you call `await` or `.then()` or `.catch()`. This behavior is different from a regular JavaScript `Promise`, which starts executing immediately."
- Prisma maintainer on [prisma/prisma#13713](https://github.com/prisma/orm/issues/13713#issuecomment-2160467713): "`// DO NOT DO THIS deal.activities.forEach(async (activity) => { const dealActivity = await tx.crmDealActivity.create() …` is just plainly wrong and doesn't do what you think it does. The transaction callback will finish and commit the transaction before the operations have a chance to execute. You need to await the operations **inside** the transaction callback and not outside of it." and "Don't forget `await` before `Promise.all` or you will ultimately run into the same issue as with `forEach`."
- Prisma ORM 8 docs, [Transactions](https://www.prisma.io/docs/orm/fundamentals/transactions): "Send the calls on `tx` one at a time, and await each before you send the next. … other work on the same `tx`, such as a second call in `Promise.all`, is refused with `RUNTIME.TRANSACTION_SUSPENDED`."

### `prisma-practice-bulk-write-without-where`

Rerail's own rule, not a port. It matches `<x>.<model>.deleteMany(...)`, `<x>.<model>.updateMany(...)` and
`<x>.<model>.updateManyAndReturn(...)`, outside tests, seed files and `scripts/`, where `<model>` is a model that a
`.prisma` file of the repository declares (`model Draft {` gives `draft`, compared without case; the schema files are
found by walking the directories up to six levels down, so `prisma/schema.prisma`, `prisma/schema/*.prisma` and
`packages/db/src/prisma/schema/*.prisma` all count). `<x>` may be anything: `prisma`, `db`, `tx`, `this.prisma`,
`prisma()`. Two shapes:

- the filter is missing or empty as written: `deleteMany()`, `deleteMany({})`, an object argument with no `where` and
  no spread (`updateMany({ data: { read: true } })`), or `where: {}`;
- a value of the `where` object may be `undefined` (the identifier `undefined`, an optional chain
  `session?.user?.id`, or a name bound to an optional parameter `userId?: string` or one typed `T | undefined`), and
  every other value is a literal (`role: "MEMBER"`, `deleted: false`, `Role.MEMBER`) or may be undefined too.
  Prisma leaves an undefined field out of the query, so the filter becomes `{}`, or only the constants, which match
  the rows of every owner and tenant: `deleteMany({ where: { teamId, role: "MEMBER" } })` with `teamId` missing
  deletes every team's members. Not reported when the chain's root or the parameter is read earlier in the same
  function (a guard such as `if (!userId) return`), when another value the caller chose always filters (`kind`, a
  date bound, an id list), or when a schema's generator enables `strictUndefinedChecks` (Prisma then throws instead).

The message names the model and the shape; for the empty filter it says to put the filter the change is for in
`where` and, when emptying the table is the intent, to keep it with a `rerail-ignore` comment; for the undefined value
it says to check the value first or enable `strictUndefinedChecks`, and to leave it when the value is always set.

It leaves alone: a receiver that is not a Prisma client (the pack's Prisma client gate, under query-in-loop in the
[README](../README.md#prisma-practice-query-in-loop)); a
`where` with a value that is always set (`session.user.id`, `{ expiresAt: { lt: now } }`); a filter not written in the
call (`deleteMany(args)`, `deleteMany({ where })`, `deleteMany({ ...args })`); `deleteMany` on anything that is not a
schema model (MongoDB's `db.collection("x").deleteMany({})`, `this.items.deleteMany({})`), and a `data` that is a list
of per-item writes (`data: [{ where, data }]`, `data: ids.map(…)`: KeystoneJS's list API, whose generated schema.prisma
names its lists as models; Prisma's updateMany takes one object); a deleteMany followed in the same function by a
create of the same model (replace the rows: `$transaction([db.license.deleteMany(), db.license.create(...)])`, an
import that refills the table); an unfiltered updateMany followed by a filtered update of the same model that sets one
of the same fields (move a flag to one row: every currency `isDefault: false`, then one `isDefault: true`); a function
whose unfiltered bulk writes empty two or more models (a "delete all my data" reset); tests, `**/seed.*`,
`**/seed/**`, `**/seeds/**`, `**/scripts/**`.

**Gaps:** a `where` built in a variable or a helper; values undefined for reasons the syntax does not show (a function
returning `string | undefined`, a destructured optional field such as `{ bookingUid }: { bookingUid?: string }`, a
variable assigned from `searchParams.get(...) ?? undefined`); a delegate stored in a variable
(`const d = prisma.draft; d.deleteMany()`); a schema outside the repository or deeper than six directories; one
repository-wide `strictUndefinedChecks` decision, though a monorepo with two schemas may enable it in only one; Prisma 8,
which reaches models as `tx.orm.<schema>.<Model>`. Deliberate single-table clears are still reported: no syntax tells
a one-row table from a large one.

Why Log: existing code clears tables only on purpose; the rule is for an agent that drops a filter in new code, which
the corpus cannot show.

**Sources:**
- Prisma docs, [CRUD: Delete all records](https://www.prisma.io/docs/orm/v7/prisma-client/queries/crud#delete-all-records): "The following query uses `deleteMany()` to delete all `User` records: `const deleteUsers = await prisma.user.deleteMany({});`"
- Prisma docs, [Null and undefined](https://www.prisma.io/docs/orm/v7/prisma-client/special-fields-and-types/null-and-undefined): "In Prisma ORM, if `undefined` is passed as a value, it is not included in the generated query. This behavior can lead to unexpected results and data loss. We strongly recommend enabling the `strictUndefinedChecks` preview feature described below."
- Same page, under strict undefined checks: "This change helps prevent accidental deletions or updates, such as: // Before: This would delete all users `prisma.user.deleteMany({ where: { id: undefined } })`"
- Same page: "Using `undefined` as the value of any key in a Prisma Client query's parameter object will cause Prisma ORM to act as if that key was not provided at all." and "the same concepts apply to any function that can affect multiple records, such as `updateMany` and `deleteMany`."

### `prisma-practice-sensitive-fields-to-client`

Rerail's own rule, not a port. It reads every `*.prisma` file as text (models and views, multi-file schemas too). A
query reads the schemas of its own package (the nearest package.json) and of the workspace packages its package.json
depends on; a query whose package reaches no schema that way reads the root package's and those of packages some
workspace package depends on, never another app nobody depends on (an example app whose `User` shares the name but
not the columns). It takes as secret a `String` or `Bytes` column whose name, lowercased
without `_`, contains `password`, or ends in `salt`, `secret`, `secretkey`, `token`, `apikey`, `privatekey`,
`backupcode(s)`, `recoverycode(s)`, `keyhash`, `tokenhash`, `secrethash`, or starts with
`hashedkey`/`hashedtoken`/`hashedsecret`/`hashedapikey`; not one whose name holds a display word at a camelCase or
snake_case boundary (`display`, `masked`, `partial`, `hint`, `last4`, `prefix`, `redacted`: `displaySecretKey`,
`keyPrefix`, but not `prefixedSecret` or `previewToken`), nor a column with a literal string default (`token String
@default("usdt")` is a currency name: a credential is not one value shared by rows). A query is
`<receiver>.<accessor>.<op>(...)`, the receiver a Prisma client (the pack's gate, under query-in-loop in the
[README](../README.md#prisma-practice-query-in-loop)), where
the accessor is a schema model's name with its first letter lowercased and op
returns rows: `findUnique`, `findFirst`, `findMany` (and the `OrThrow` forms), `create`, `update`, `upsert`, `delete`,
`createManyAndReturn`, `updateManyAndReturn`. It reports the query (at its first line) when it returns a whole row of a
model with secret columns: no arguments, or an argument object with no `select` and an `omit` that leaves a secret
column in, or `include`/`select` of a relation as `true` or as an object without `select` (followed to any depth, so
`prisma.post.findMany({ include: { author: true } })` leaks `author.passwordHash`). And only when that result is sent
to the browser, as the call itself or a `const` bound to it in the same function and block, placed anywhere in an
object, array, spread, conditional or `??`/`||` of:

- a JSON response: `NextResponse.json(v)`, `Response.json(v)`, `res`/`response`/`reply`/`c` `.json(v)` or `.send(v)`
  (also after `.status(n)`), `json`/`typedjson`/`data` imported from Remix or React Router,
  `new Response(JSON.stringify(v))`;
- a Server Action's return value: a top-level function inside an export of a `'use server'` module (next-safe-action's
  `export const a = client.action(async () => ...)` too), or a function whose body starts with `'use server'`, in a
  file whose nearest package.json lists `next` (the root's when none is nearer);
- a tRPC procedure's result: the resolver of `<...procedure...>.query(fn)` / `.mutation(fn)`, unless the chain has
  `.output(schema)`, which parses the result and strips every key the schema does not name;
- loader data: an exported `getServerSideProps` / `getStaticProps` (in a package that lists `next`), or `loader` /
  `action` in a file under `routes/`;
- a prop (attribute or `{...spread}`) of a component imported from a `'use client'` module, in a file that is not
  one, in a package that lists `next`. Imports resolve relatively (`./x`, with `.tsx/.ts/.jsx/.js` and `index.*`), by
  path suffix for `@/x`, `~/x`, `#/x`, and for `@scope/pkg/x` to a file ending in `/pkg/x` or `/pkg/src/x`.

The message names the model and op, the secret columns (with their relation path), where the result goes, and the
three fixes (`select`, per-query `omit`, the client's global `omit`), with the case where the fix does not apply.

It leaves alone: a query with `select` (also `select: { password: true }`: an explicit choice); an `omit` that drops
every secret column, or `omit: CONST` (not visible here); a global `new PrismaClient({ omit: { user: { passwordHash:
true } } })` anywhere in the repository; `findMany(args)`, `findMany({ ...args })`; models without secret columns, and
columns of other types (`inputTokens Int`, `hasPassword Boolean`); a row used only on the server (`bcrypt.compare(pw,
user.passwordHash)`) or reshaped before it is sent (`users.map(toPublic)`, `{ id: user.id }`, destructuring); a
non-exported helper of a `'use server'` module; a prop of a server component or of a component in the same file;
Drizzle's `db.query.users.findMany()`; tests, stories, fixtures and generated code.

Why Log: half the matches are by design or go to a server, and the message cannot tell a settings page that shows its
own secret from a leak; it is kept for the severity of the real ones (a password hash, a 2FA secret and backup codes,
an encrypted mailbox password in a JSON body).

**Sources:**
- Prisma docs, [Excluding fields](https://www.prisma.io/docs/orm/v7/prisma-client/queries/excluding-fields): "Use `omit` when the default result is mostly correct and you only want to remove a few sensitive or noisy fields." with `prisma.user.findUnique({ where: { id: 1 }, omit: { password: true } })` and the global form `new PrismaClient({ omit: { user: { password: true } } })`.
- Prisma changelog, [v6.2.0: omit API is now GA](https://www.prisma.io/changelog/2025-01-09): "In this release we are moving the omit API (our most requested feature) to Generally Available." (Preview `omitApi` from [5.13.0](https://github.com/prisma/orm/releases/tag/5.13.0), global omit from [5.16.0](https://www.prisma.io/changelog/2024-06-27).)
- OWASP API Security Top 10 2023, [API3: Broken Object Property Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa3-broken-object-property-level-authorization/): "Avoid using generic methods such as to_json() and to_string(). Instead, cherry-pick specific object properties you specifically want to return."
- Next.js docs, [Data security](https://nextjs.org/docs/app/guides/data-security): "// only return the data relevant for this query and not everything".

### `prisma-practice-dev-migration-in-deploy`

Rerail's own rule, not a port. It reads the commands a deploy runs and reports one that runs `prisma migrate dev`,
`prisma migrate reset`, `prisma db push --accept-data-loss` or `prisma db push --force-reset` (through `npx`, `pnpm`,
`yarn`, `bunx`, `dotenv --`, an exec-form array, or `node node_modules/prisma/build/index.js`). A deploy command is:

- a package.json script named `build`, `start`, `install` or `prepare` (with their `pre`/`post` hooks), `postinstall`,
  `vercel-build`, `heroku-prebuild`/`heroku-postbuild`, or with a `deploy`, `release`, `prod` or `production` segment
  in its name (`db:deploy`, `start:prod`);
- vercel.json `buildCommand`/`installCommand`; railway.json `build.buildCommand`, `deploy.startCommand`,
  `deploy.preDeployCommand`;
- any line of a Dockerfile, Procfile, `fly.toml`, `render.yaml`, `railway.toml`, `nixpacks.toml`, `netlify.toml`, or a
  compose file named for `prod` or `production`;
- a GitHub Actions workflow line in a job whose id names deploy/release/prod, or in a workflow whose file name does,
  outside a test/CI/check/lint job (job ids read by word: `deploy-web` is a deploy job, `product-checks` is not);
- any line of a `.sh` script one of those commands names.

It follows the scripts a command calls, transitively: `npm run x`, `npm start`, `pnpm x`, `yarn x`, `bun run x` in the
same package.json, `pnpm --filter <name> x`, `yarn workspace <name> x` and `npm run x -w <name>` in the package so
named, `turbo run x` in every package; the message names the script that holds the command. The file walk reads
directories six deep from the root, skipping dot directories (but `.github`), `node_modules`, build output and test
and fixture folders. The message names the command, why it is destructive, and the fix (`prisma migrate deploy`; for
`db push`, committed migrations or at least dropping the flag).

It leaves alone: the same commands in dev scripts (`dev`, `db:migrate`, `db:reset`, `test:e2e`, `start:dev`); a
command that also starts a dev server or watcher (`next dev`, `pnpm dev`, `tsx watch`, `nodemon`, `--watch`); a
command guarded on its line (`if [ "$VERCEL_ENV" = "preview" ]; then …`, `[ -n "$CI" ] && …`, `!= "production"`) or
inside an `if` block still open there (no `fi` at its own indentation in between) whose condition names
preview/test/e2e/CI/dev/local or rules out production (a non-production word after `!=` is no guard);
`Dockerfile.dev`, `Dockerfile.local`,
`Dockerfile.test`, Dockerfiles under `dev/` or `local/`; `docker-compose.yml` and other compose files not named for
production; a workflow's test job; a `.sh` script that no deploy command names, or that starts a dev server; plain
`prisma db push` (it stops rather than lose data); package.json files under test, e2e and fixture folders;
`prisma migrate deploy` anywhere.

**Gaps:** commands assembled in variables or Makefiles; `cd dir && npm run x` resolves x in the package.json files
beside and above the deploy file, not in dir; a guard that names a non-production word in a negation (`!=
"development"`) is still taken as a guard; workflows whose jobs are not indented two spaces; Heroku `app.json`,
Kubernetes manifests and platform files not listed above; a `.sh` script is found by file name only, so two scripts
with one name are both taken. Findings in Dockerfiles, Procfiles, workflows and shell scripts are reported at files
outside the rule's document scope (the rule reads them as lines); whether a session that edits only such a file
triggers the rule depends on when core runs it ([ENGINE-GAPS.md](../ENGINE-GAPS.md)). Prisma 8 replaces these commands
with `prisma db migrate` and `prisma db update --delete <subject>`; a later revision can report the latter.

Why Log: no read matches yet to back a steer. It is kept because the footgun is production data loss, and Prisma's own
CLI refuses `migrate reset --force` when it detects a coding agent, for this reason.

**Sources:**
- Prisma docs, [Development and production (v7)](https://www.prisma.io/docs/orm/v7/prisma-migrate/workflows/development-and-production): "`migrate dev` is a development command and should never be used in a production environment."; "`migrate reset` is a development command and should never be used in a production environment."; "In production and testing environments, use the `migrate deploy` command to apply migrations"; "The `migrate dev` command will prompt you to reset the database in the following scenarios: … The database schema has drifted away from the end-state of the migration history".
- Prisma CLI reference, [`migrate reset`](https://www.prisma.io/docs/orm/v7/reference/prisma-cli-reference#migrate-reset): "Drops the database/schema if possible, or performs a soft reset if the environment does not allow deleting databases/schemas"; `--force`: "Skip the confirmation prompt"; and of `prisma migrate reset --force` run by a coding agent: "You are attempting a highly dangerous action that can lead to devastating consequences if it is incorrectly executed against a production database."
- Prisma CLI reference, [`db push`](https://www.prisma.io/docs/orm/v7/reference/prisma-cli-reference#db-push): `--force-reset`: "Resets the database and then updates the schema"; `--accept-data-loss`: "Ignore data loss warnings. This option is required if as a result of making the schema changes, data may be lost."
- Prisma docs, [Prototyping your schema (v7)](https://www.prisma.io/docs/orm/v7/prisma-migrate/workflows/prototyping-your-schema): "if `db push` anticipates that changes will result in data loss, you can either accept data loss with the `--accept-data-loss` option or stop the process."

### `prisma-practice-read-modify-write`

Rerail's own rule, not a port. It matches, outside tests, stories, fixtures and generated code, a write
`<x>.<model>.update(…)`, `updateMany(…)` or `upsert(…)` (the `update` object of an upsert) whose `data` sets a field
`f` to arithmetic on `<row>.f`, where `<row>` holds a row of the same model read earlier in the file by
`<x>.<model>.findUnique|findUniqueOrThrow|findFirst|findFirstOrThrow(…)` (`const row = await …`), or is the variable of
a `for (const row of rows)` loop over a `findMany` result. Arithmetic is `+ - * /` with `<row>.f` as an operand at any
depth (`row.f - a - b`), also through `row.f ?? 0`, `row?.f`, `row.f || 0` and `Number(row.f)`, a Decimal's
`row.f.plus|minus|add|sub|times|mul(…)`, and a `const` holding such an expression (`const next = row.stock - qty; …
data: { stock: next }`, or the shorthand `{ stock }`). The write must name the row read: its `where` reads `<row>`
(`{ id: row.id }`), or shares a variable with the read's `where` (`{ id }` in both, `{ id: input.id }`), or is the
same text. The client is any receiver (`prisma`, `db`, `tx`, `ctx.db`, `this.prisma`): there is no type checker, but
the `<x>.<model>.<op>({ data })` shape with these op names is Prisma's (Drizzle writes `.update(t).set()`, TypeORM
`update(criteria, partial)`). The message names the row variable, the field and the model, and gives the atomic
operator fix, the conditional-write fix (`where: { id, f: row.f }` or a version field, with the retry), and when to
leave it.

It leaves alone: a receiver that is not a Prisma client (the pack's gate, under query-in-loop in the
[README](../README.md#prisma-practice-query-in-loop)); atomic operators
(`{ decrement: qty }`); a write whose `where` also tests the field itself (`{ id, stock: { gte: qty } }`,
`{ id, sold: product.sold }`), a version, revision, `updatedAt` or etag column (the name alone or ending a camelCase
or snake_case name: `rowVersion`, `lock_version`), or a claim column (a column that some write of the model in the
file clears, `lockId: null`); a write inside `$transaction(fn, { isolationLevel: Serializable or RepeatableRead })`
(Postgres then aborts one of two racing writers instead of losing a write); a function that takes a row or advisory
lock through raw SQL (`FOR UPDATE`, `pg_advisory_xact_lock`), or that first passes the write's client to a function
of the run whose body does so (`await lockAssetForQuantityUpdate(tx, id)`: what the helper runs, not its name); a
write whose `data` releases a claim the function checked on the row (`if (job.lockId !== lockId) return` … `data:
{ lockId: null }`: the claim holder is the row's only writer), or whose `where` pins, beside the id, a column its own
`data` sets (`where: { id, armedCouponId: couponId }, data: { armedCouponId: null, … }`: a compare-and-set claim, so a
second writer matches no row); `+` with a string or template operand (concatenation); a field computed from another
field (`price: p.sold * qty`); another model; a row the request just created (`create()` is not a read); a write to
another row than the one read; `create` of a new version row (`version: doc.version + 1` in `create`); test files.

**Gaps:** the read and the write in different functions (a helper taking the row), a row passed in as a parameter or
read from a session (`ctx.user.credits - cost`), destructured reads (`const { stock } = await …`), `Promise.all` reads,
`rows.map(r => db.x.update(…))`, interactive transactions on an extended client that renames `$transaction`, and Prisma
8's `tx.orm.<schema>.<Model>` delegates. A row with a single writer (a per-user draft only its owner edits, a cron job
with one instance) is reported though it cannot race; the message says to leave it then. Serializable isolation set
globally (a `transactionOptions` default on the client) or passed in a variable (`$transaction(fn, opts)`) is not
seen. A lock helper is found by its name among the functions the run declares (not through the import), so a
same-named function elsewhere stands in for it, and one in a file another lane reads is not seen. A claim released
without a check in the same function, on a column no write of the file clears, is reported. The rule does not read
the schema, so it cannot tell a numeric field from a string one; it leaves string concatenation alone by its syntax.

Why Log: 71% of matches real; the two harmless ones are single-writer rows, which no syntax tells from shared ones.

**Sources:**
- Prisma Client reference, [Atomic number operations](https://www.prisma.io/docs/orm/reference/prisma-client-reference#atomic-number-operations): "This feature allows you to update a field based on its **current** value (such as *subtracting* or *dividing*) without risking a race condition."
- Prisma docs, [Transactions: Optimistic concurrency control](https://www.prisma.io/docs/orm/prisma-client/queries/transactions#optimistic-concurrency-control): "we **optimistically** assume that a record will remain unchanged in between reading and writing, and use a concurrency token (a timestamp or version field) to detect changes to a record."
- PostgreSQL docs, [Transaction Isolation: Read Committed](https://www.postgresql.org/docs/current/transaction-iso.html#XACT-READ-COMMITTED): "If the first updater commits, the second updater will ignore the row if the first updater deleted it, otherwise it will attempt to apply its operation to the updated version of the row." (The update re-reads the row, but a value the application computed from its earlier read is written as it is: the lost update a `$transaction` does not prevent.) And under Repeatable Read: "a repeatable read transaction cannot modify or lock rows changed by other transactions after the repeatable read transaction began." (Why Serializable and RepeatableRead transactions are left alone.)

### `prisma-practice-find-first-without-order-by`

Rerail's own rule, not a port. Prisma runs `findFirst` as a `findMany` limited to one row after it has decided the
ordering, so without `orderBy` (and without `take`, `skip` or `cursor`, after which Prisma orders by the primary key)
the SQL ends in `LIMIT 1` with no `ORDER BY`. When several rows match, Postgres returns whichever its plan reaches
first, and that row changes after updates, VACUUM or a new index. It matches, outside tests, seed files and
`scripts/`, `<client>.<m>.findFirst(…)` and `findFirstOrThrow(…)` on a model a `.prisma` file of the repository
declares, whose `where` is an object literal without a spread that names no `@id`/`@unique` field of the model with
an equality value and not every field of one of its `@@id`/`@@unique` keys, and that passes no `orderBy`, `take`,
`skip` or `cursor`. The message says to add the `orderBy` that picks the row meant, ending in a unique field, or to
declare a business-unique column with `@unique`/`@@unique`.

It leaves alone: presence checks (`if (await prisma.x.findFirst(…))`, `const e = await …; if (e) throw`, `!!e`,
`e === null`), where any row answers; a call with no `where` written in it (`findFirst()`, `findFirst(args)`,
`findFirst({ where })`); a filter that pins a unique field (`{ email }` with `email @unique`, `{ id, ownerId }`, an
`equals` with `mode: "insensitive"`); `contains` or `not` on a unique field is no pin and is reported; a model the
schema does not declare; `findMany({ take, skip })` without `orderBy`, where Prisma adds `ORDER BY <primary key>`
itself.

**Gaps:** a `where` built in a variable or another function; a filter unique by a business rule the schema does not
declare; a unique filter through a relation (`where: { user: { id } }` on a one-to-one); a schema outside the
repository or more than six directories down; a delegate stored in a variable; Prisma 8's
`db.orm.<schema>.<Model>.where(…).first()`. An engine disagreement between two `deparen` definitions made the rule key
its check on the key, not the value ([ENGINE-GAPS.md](../ENGINE-GAPS.md), gap 16).

**Sources:**
- PostgreSQL docs, [SELECT: ORDER BY](https://www.postgresql.org/docs/current/sql-select.html#SQL-ORDERBY): "If
  `ORDER BY` is not given, the rows are returned in whatever order the system finds fastest to produce."
- prisma-engines `query_arguments.rs` on `take`/`skip`/`cursor`: "We simply take the primary identifier as a default
  order-by" ([prisma/prisma#21032](https://github.com/prisma/prisma/issues/21032)).
