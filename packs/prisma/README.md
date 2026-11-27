# Rerail rule pack: Prisma

The pack ships five rules, all Rerail's own; none is a port of a linter rule. Three start on:
`prisma-practice-one-client` (a PrismaClient made per call, or at module load in a Next.js app without a `globalThis`
cache), `prisma-practice-request-body-as-data` (a request body or a Server Action's argument passed to a Prisma write as
`data`) and `prisma-practice-side-effect-in-transaction` (mail, payment, HTTP, queue or cache revalidation inside an
interactive `$transaction` callback). Two start off: `prisma-practice-raw-unsafe-interpolation` (SQL injection through
Prisma's Unsafe raw queries) and `prisma-practice-query-in-loop` (one Prisma query per item of a list). Each is advice
that the Prisma docs (for request bodies, OWASP's and the Next.js docs) give and that no linter checks, quoted in its
details below, and what each reports is wrong by documented behaviour (connection pools exhausted, columns a client
chooses, mail and charges for rolled-back writes, SQL injection, n+1 round trips), not a matter of style. A rule starts
on when more than 90% of its matches on apps it was not tuned or fixed on are real
([How the rules were measured](#how-the-rules-were-measured)); the two off rules have not met that. All five log: a
finding is recorded and steers no one until a repository switches the rule to steer. A repository turns a rule on or
off, or changes its mode, with its switch on the Prisma pack's page in the dashboard, or with `repository_settings_set`
(`PUT /api/repository/settings`) for a repository or a checkout
([rule scoping](../../docs/rule-scoping.md#management-surfaces)). Set-up recommends a pack with a rule that starts on,
so it recommends this one where Prisma is used. The ideas mined and not built are in
[NOTES-prisma.md](NOTES-prisma.md#dropped-candidates), the rules written and not shipped in
[candidates/README.md](candidates/README.md).

The rules target Prisma ORM 5 to 7 on Postgres, in a Next.js, Remix, NestJS or plain Node app. Each reads the
repository's TypeScript and JavaScript files without a type checker, one file at a time; what one file needs of
another (which module is the Prisma client, which models the schema declares, what an imported helper or client does)
crosses between files by paths and symbols. raw-unsafe-interpolation recognises Prisma by its own names
(`$queryRawUnsafe`, `$executeRawUnsafe`, and `Prisma.raw` with `Prisma` imported from Prisma); query-in-loop and
request-body-as-data match `<r>.<model>.<op>(…)` only where the receiver resolves to a Prisma client (the gate
described under query-in-loop's details), request-body-as-data only for a model of the repository's `*.prisma`
schema; one-client matches `new PrismaClient(…)` and side-effect-in-transaction the callback of `$transaction(…)`.
Prisma ORM 8, a ground-up rewrite now in release candidate, is matched by none; each rule's details say how its advice
carries over, and what it misses.

Each rule is proven two ways: by its cases in `cases/<id>/`, where a line ending in `// caught` must be reported and
no other line may be, run by the rule file's own test (`.test cases exactly`, which reads the case directory as one
repository; `rerail rules test <file>.rerail`); and by runs on open-source Prisma apps at pinned commits, whose
matches were read and judged real or not. A match is real when the file holds the pattern, in code that runs, and the
rewrite the message gives is safe there.

| Rule | Mode | Catches | Test set #1: 20 apps | Test set #2: 37 apps | Test set #3: 41 apps | Held-out: 8 apps | Corpus (tuning): 28 apps |
|---|---|---|---|---|---|---|---|
| `prisma-practice-one-client` | Log, on | `new PrismaClient()` per call in a function, or at module load in a Next.js app without a `globalThis` cache | 0 matches | 30 of 30 real, in 4 apps (26 in one) | 0 of 1 real | 0 of 1 real (now left alone) | 0 matches |
| `prisma-practice-request-body-as-data` | Log, on | a request body or a Server Action's argument passed to a Prisma write as `data` | 0 matches | 5 of 5 real, in 3 apps | 3 of 3 real, in 1 app | 0 matches | 0 matches (6 of 6 adversarial shapes right) |
| `prisma-practice-side-effect-in-transaction` | Log, on | mail, payment, HTTP, queue or cache revalidation, directly or through a workspace helper, inside a `$transaction` callback | 0 matches | 5 of 5 real, in 2 apps | 0 matches | 0 matches | 1, real |
| `prisma-practice-raw-unsafe-interpolation` | Log, off | a value pasted into raw SQL where a parameter belongs | 0 of 1 real before the fix | 0 of 3 real, in one app | 0 matches | 0 matches after fixes | 72 matches |
| `prisma-practice-query-in-loop` | Log, off | one Prisma query per item of a list | 13 of 17 real before fixes | 13 of 19 judged real; 56 additional matches unjudged | 35 of 48 judged real; 1 additional match unjudged | 39 matches after fixes | 537 matches after fixes |

The corpus numbers are tuning numbers: the rules were written and fixed against those apps. Test set #1's apps are
tuning data too now for query-in-loop and raw-unsafe-interpolation, which were fixed against its false matches
([NOTES-prisma.md](NOTES-prisma.md#fixes-after-the-first-test-set)); neither was fixed against test sets #2 or #3, and
no rule was. The three rules that start on had no match on test set #1; their evidence is test sets #2 and #3, read
before request-body-as-data and side-effect-in-transaction were rewritten to run per file. Their fixtures and split
checks pass, and re-runs reproduce all ten set-#2 findings ([NOTES-prisma.md](NOTES-prisma.md#the-promoted-rules-run-per-file)).
The apps of the corpus already follow most of this advice, so the shapes these rules report show up in new code, where
a coding agent writes them, more than in mature apps.

## How the rules were measured

The bar: a rule ships on only when more than 90% of its matches on apps it was not tuned or fixed on are real. Test
set #1 counts for a rule not tuned on it, test set #2 for a rule not fixed against its findings, test set #3 for every
rule. Promoting a candidate or keeping a rule on takes at least 5 such matches in at least 2 apps; a rule under that
floor with no false match stays where it is. A rule below the bar ships off, as query-in-loop does
(48 of 67 judged matches real, 72%; not an estimate over all findings). one-client (30 of 31, 97%, in 5 apps), request-body-as-data (8 of 8, in 4 apps) and
side-effect-in-transaction (5 of 5, in 2 apps) cleared it and moved from candidates into the pack. The rules still
under the bar or the floor wait outside the pack, unpublished ([candidates/README.md](candidates/README.md)).

## What a run costs

On the measured remote runs, a single app was stopped at 5 GB resident memory or 50 minutes. Those limits are
measurement guards, not product resource guarantees. Missing runs are excluded rather than counted as clean apps.

## Rule details

What each rule matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources.

### `prisma-practice-raw-unsafe-interpolation`

Rerail's own rule, not a port. It matches, outside test, story and generated files, an expression pasted into the
SQL text of `<any>.$queryRawUnsafe(sql, …)`, `<any>.$executeRawUnsafe(sql, …)` or `Prisma.raw(sql)` where the SQL
around it reads a value: inside a `'…'` string literal (an odd number of `'` before it in the template's fixed text,
an apostrophe on a `-- …` comment line not counted), or right after `=`, `<>`, `<`, `>`, `||`, `LIKE`, `ILIKE`,
`IN (`, `ANY (`, `VALUES (`, `LIMIT` or `OFFSET`. The SQL text is the first argument as written, a local variable
passed there (its initialiser and every `=`/`+=` to it), the pieces of a `+` chain, templates nested in a `${…}` or a
ternary's branches, and the callback of `list.map((x) => `'${x}'`).join(",")`. It reports at the pasted expression;
the message names the API and its parameter fix (`$1` with an extra argument, or the tagged `$queryRaw`/`$executeRaw`;
for `Prisma.raw`, `Prisma.sql` and `Prisma.join` for IN lists), and says to keep a table or column name pasted only
from a fixed list.

Two more shapes, each with its own fix in the message:

- a sort or group key from a `string` parameter its function never checks (no `.includes(x)`, `.has(x)`, `.test(x)`,
  `.parse(x)`, `switch (x)`, `x === "…"`, `x in o`): pasted after `ORDER BY`/`GROUP BY` (or as the direction after
  the first column) into Unsafe or `Prisma.raw` text, given to `Prisma.raw(sort)` that a tagged template pastes after
  `ORDER BY`, or given to `Prisma.raw` under a sort-key name (`sortBy`, `orderBy`, `dir`, `direction`). A name cannot
  be a parameter, so the fix is a fixed map of literals or a check against a list;
- a value quoted in `'…'` inside a `SET`, which Postgres takes no parameters in (`SET LOCAL app.tenant =
  '${tenant}'`, the usual row-level-security tenant switch): `SELECT set_config('app.tenant', $1, true)` takes the
  value as a parameter. Other statements without parameters stay alone even quoted (on the corpus, partition bounds
  computed from dates in `CREATE TABLE … FOR VALUES FROM ('${lo}')`).

It leaves alone: placeholders (`WHERE id = $1`, extra arguments); the tagged `$queryRaw\`…${v}\`` and
`Prisma.sql\`…${v}\``; other identifiers pasted where SQL names something (`SET search_path TO "${schema}"`,
`FROM ${table}`, `"${alias}"."${col}"`, `${Prisma.raw(alias)}."userId"`), and a sort key typed as a union of
literals or checked first; values in statements Postgres takes no bind parameters in, by their first word (`SET`
unless the value is quoted, `CREATE`, `ALTER`, `DROP`, `TRUNCATE`, `SAVEPOINT`, `RELEASE`, `ROLLBACK`, `COMMENT`,
`GRANT`, `LOCK`, `DO`, …);
constants: a literal, a variable this file binds to one, a SCREAMING_CASE name or a member of one, an enum member (a
SCREAMING_CASE property of a capitalised object: `Reason.CSAM`, not `req.body.ID`), a setting from the environment
(`process.env.X`, `import.meta.env.X`, `env.X`), a callback parameter over a SCREAMING_CASE array or
`Object.values/keys(…)`, a parameter typed as a union of string literals (`key: 'a' | 'b'`); coerced or escaped values:
`Number()`, `parseInt()`, `parseFloat()`, `Math.*`, `BigInt()`, `.toFixed()`, `.toISOString()`, `.length`, a variable
initialised to one of these, calls named `escape…`/`quote…` or `…Escape…`/`…Quote…` at a camelCase boundary
(`escapeLiteral`, `pgEscape`; not `unescapeHtml`), and `s.replace(/'/g, "''")`; placeholder text, recognised by what
it is rather than what it is called, in Postgres `$n` and MySQL/SQLite `?` form: a template that is only `$` and one
`${…}` (`$${i + 1}`), a string or template that is only `?`, a list of them joined
(`ids.map((_, i) => `$${i + 1}`).join(",")`, `ids.map(() => "?").join(", ")`, `Array(n).fill("?").join(",")`), `?`
marks repeated (`"?, ".repeat(n)`, also sliced or trimmed), a variable holding any of these, or a call of a function of
the same file that returns a mark (`const bind = (v) => `$${values.push(v)}``); booleans and arithmetic (`${a === b}`, `${n * 2}`);
test files. A ternary or a `+`, `??`, `||`, `&&` pasted whole is looked into: `${q + "%"}` and `${cond ? q : ""}`
report `q`. `Prisma.raw` counts only when `Prisma` is an import, and not one from another ORM's package;
`$queryRawUnsafe`/`$executeRawUnsafe` are Prisma's own names.

**Sources:**
- Prisma docs, [Raw queries](https://www.prisma.io/docs/orm/v7/prisma-client/using-raw-sql/raw-queries): "The methods with "Unsafe" in the name are a lot more flexible but are at **significant risk of making your code vulnerable to SQL injection**."
- Same page, SQL injection prevention: "Another way to make these methods vulnerable is misuse of the `Prisma.raw` function. The following examples are all vulnerable ❌ to SQL Injection:" and, on `"SELECT id, name, email FROM User WHERE name = " + inputString` passed to `$queryRawUnsafe`: "Prisma Client ❌ **cannot** escape `inputString` in this example, which makes it vulnerable to SQL injection".
- Same page, Parameterized queries: "`$queryRawUnsafe` supports standard parameterized queries where each variable is represented by a symbol (`?` for MySQL, `$1`, `$2`, and so on for PostgreSQL)" … "As with tagged templates, Prisma Client escapes all variables when they are provided in this way." and "**Note**: You cannot pass a table or column name as a variable into a parameterized query." (Why identifiers are left alone.)
- Prisma ORM 8 docs, [Raw queries reference](https://www.prisma.io/docs/orm/reference/raw-queries): "An interpolated value is sent as a query parameter, never pasted into the SQL. A column or table name cannot be interpolated as a string, because a string becomes a parameter."

### `prisma-practice-query-in-loop`

Rerail's own rule, not a port. A Prisma query per item of a list is the n+1 problem: N round trips per request, and
under `Promise.all(xs.map(…))` N queries at once, which queue for the connection pool and fail with P2024 once the list
outgrows it. A query is `<r>.<model>.<op>(…)` (findUnique … groupBy, their OrThrow and AndReturn forms) whose receiver
is a Prisma client by the pack's gate (below). It reports such a query when it runs once per item of a list and its
arguments read the item:

- in the body of `for (const x of xs)` / `for (const k in o)`, not in a function nested in it;
- in the callback of `xs.map/forEach/flatMap/reduce/filter/some/every/find(…)`, not in a function nested in it;
- the innermost loop only, once per query.

The message gives the one-query fix for the operation: reads, one `findMany` with an `in` filter before the loop, or
`include`/`select` of the relation (`_count`/`groupBy` for counts); `create`, `createMany`/`createManyAndReturn` or a
nested `create: [...]`; update, upsert and delete, `updateMany`/`deleteMany` with `where: { id: { in: ids } }` when
every item gets the same change, otherwise one `$transaction(items.map(...))` batch.

It leaves alone:

- `findUnique`/`findUniqueOrThrow` in an iteration callback: Prisma's dataloader batches those made in one tick;
- `createMany` per item (a batch per chunk), and a query that filters `in` on the item (`where: { id: { in: chunk } }`,
  `{ in: ids }` per group of a Map): each iteration already is one query for a group of rows;
- a list written in the code: an array literal (`["a", "b"] as const`, not `[...spread]`), a const bound to a
  non-empty one that is never pushed to (`REGIONS.map(…)`), or the values of an enum-like object
  (`Object.values(Provider)`, capitalised);
- a query whose arguments do not read the item (one query per iteration, but no n+1 to batch);
- a top-N or newest-row query per item, `findMany({ take })` or `findFirst({ orderBy })`: no single `in` query
  expresses a limit or an order per key;
- a for loop that stops early by `return` or its own `break` (a search for the first match);
- a query inside a `try` statement (block or catch) in the loop body: each item may fail on its own, and a batch would
  make it all-or-nothing;
- an unsent query pushed to an array (`ops.push(prisma.x.update(…))`) for a later `$transaction(ops)`, and
  `<client>.$transaction(xs.map((x) => <client>.y.update(…)))`: both build the batch the fix asks for;
- `while`, `do` and `for (;;)` loops (cursor pagination and batch drains);
- a query in a function nested in the loop body (`setTimeout(() => prisma.x.create(…))`);
- other SDKs with the same shape (`stripe.customers.create(…)`, `openai.chat.completions.create(…)`) and other ORMs
  (a Drizzle or Sequelize `db`): the receiver is not a Prisma client;
- a file that exports nothing and calls a function of its own at its top level (`main()`, `main().catch(…)`, an IIFE):
  a script run on its own; a module that exports a function `up` (the migration API of knex, Kysely, umzug, TypeORM
  and MikroORM): a data migration that runs once per database;
- seed files (`seed.*`, `seed-*`, `*-seed.*`, `seed/`, `seeds/`, `seeders/`), scripts (`scripts/`) and migrations
  (`migrations/`), by path, and a module that only they, or the script and `up` files above, import (a backfill
  exported from `prisma/migration.ts` and run from `prisma/seed.ts` on every deploy). The rule reads those files to
  see what they import, and follows one import level by path (relative, `@/`, `~/`, `#/`, `@scope/pkg/…`); a module
  an app file also imports is reported;
- a compare-and-set write: an `update`/`updateMany` whose `where` picks the item's row by a value read from the item
  (`id: row.id`, `id` destructured from it) and beside it compares another field with a value read from the item
  (`version: row.version`, `dataId: previousDataId`, `localized: { equals: row.localized }`), and whose `data` (either
  branch of a `cond ? {…} : {…}`) writes that field. Each row's write holds only if the row is unchanged since it was
  read (an optimistic lock, or a claim on the state read); one write for all the rows would drop that check. A
  `where` that compares a field the `data` leaves alone, or whose only item value is the compared field (a lookup by
  the old value, `{ envelopeId, dataId: oldId }`), is still reported;
- writes in a transaction (inside a `$transaction` callback, or on a client typed `TransactionClient`) over a list
  the code sorts first (`xs.sort(…)` before the loop, `const ys = xs.toSorted(…)`, `for (… of xs.sort(…))`): every
  transaction takes the row locks in one order so that two cannot deadlock;
- an `update`/`upsert` in a transaction (as above) whose data reads the item, after a `throw` on its own path in the
  loop body (`if (bad) throw …;` then the write, in the same block; a throw in another `if` branch or rethrown from a
  `catch` does not count): each item is checked, the whole transaction abandoned on a bad one, and its own value
  written once checked; no `updateMany` writes per-row values, and the message's `$transaction([...])` batch is not
  open inside an interactive transaction. A create or delete there, or a write before the first `throw`, is reported
  (`createMany`/`deleteMany` after the loop fits);
- tests and stories.

**The Prisma client gate.** A receiver is a Prisma client when its root name
resolves to one: the parameter of a `$transaction` callback; a parameter, variable or class member typed with a Prisma
client type (`PrismaClient`, `Prisma.TransactionClient`, `PrismaService`); a variable whose value makes or extends one
(`new PrismaClient(…)`, `.$extends(…)`); an import from `@prisma/client`, a generated client or a specifier with a
path part naming prisma (`@/lib/prisma`, `@acme/prisma`), or from a file of the run that makes a PrismaClient. It is
never one when the root resolves to another ORM: an import from Drizzle, Kysely, Knex, TypeORM, Sequelize, Mongoose,
Objection or MikroORM, a file of the run importing one, a variable made by one, or the parameter of a non-`$`
`.transaction(…)` callback. A receiver resolved neither way (`ctx.db` of a tRPC context, an import from a file read in
another lane) counts only under the client names `prisma…`, `db`, `tx`, `trx`, `database`, `_db`, and only when a
package.json of the repository lists Prisma and neither the file's package nor the root lists another ORM.
raw-unsafe-interpolation needs no gate: `$queryRawUnsafe` and `$executeRawUnsafe` are Prisma's own names, and
`Prisma.raw` counts only when `Prisma` is imported, and not from another ORM.

**Gaps:** a client under another name that does not resolve (`client` from a context, `this.repo`) or passed in as a
parameter of another name; a query in a helper the loop calls; `while`/classic `for` loops over a list; the item read
only through a variable derived from it in the loop body. Reported though not an n+1 to fix: a list small by business
rule but read from data (an input keyed by a fixed set of categories), a one-off data migration reached from a
script through more than one module or run from a script outside `scripts/` that exports something, per-row guarded
claims with a constant state (`updateMany({ where: { id, status: "PENDING" } })`, which one `updateMany` with `in`
does batch), and a write after a per-item external call. Left alone though it may be an n+1: a module the seed
imports and the app imports only through a tsconfig alias other than `@/`, `~/`, `#/` (the alias is not read); a
per-row write in a transaction over a list sorted for another reason, or a per-row update after a per-item check that
throws.
`Object.values(X)` counts as a fixed list when X is capitalised, whatever X is. Prisma 8's
`tx.orm.<schema>.<Model>.<op>(…)` delegates are not matched yet.

**Sources:**
- Prisma docs, [Query optimization](https://www.prisma.io/docs/orm/v7/prisma-client/queries/advanced/query-optimization-performance): "The n+1 problem occurs when looping through query results and performing one additional query per result." and "Prisma's dataloader automatically batches `findUnique()` queries in the same tick."
- Prisma error reference, [P2024](https://www.prisma.io/docs/orm/v7/reference/error-reference#p2024): "Timed out fetching a new connection from the connection pool."

### `prisma-practice-request-body-as-data`

Rerail's own rule, not a port. Mass assignment: Prisma writes every key of `data`, so when `data` is the object the
client sent, the client chooses the columns: it adds `role: "ADMIN"`, an owner id, `emailVerified`, a balance, or a
nested `connect`/`create` relation write, and Prisma writes it. A TypeScript type on the body checks nothing at run
time, and a zod `parse` whose result is thrown away leaves the original object as it came. A Prisma write is
`<x>.<m>.<op>(...)` with op `create`, `createMany`, `createManyAndReturn`, `update`, `updateMany`,
`updateManyAndReturn` or `upsert` and `m` a model of a `*.prisma` file of the repository (read as text; compared
without case); its row data is `data` (upsert: `create` and `update`).

It reports, after parentheses, `as`, `satisfies`, `!` and `await`:

- `data: await req.json()`; `const body = await req.json(); ... data: body` (a const, or a let nothing reassigns);
- `data: { ...body, authorId }`; `data: body.profile`; `const { id, ...rest } = body; data: rest`;
- `data: req.body` (Pages API routes, Express, Fastify); `c.req.json()` (Hono), `ctx.request.body` (Koa);
- `data: Object.fromEntries(formData)` / `Object.fromEntries(formData.entries())`, with `formData` a parameter typed
  `FormData`, `await req.formData()`, or a Server Action's argument;
- a Server Action's argument used as `data`: a parameter of a function a `'use server'` file exports, or of a function
  with its own `'use server'` (not one typed `string`, `number`, `boolean`, `bigint`, `Date`), in a file whose nearest
  package.json lists `next` (the root's when none is nearer).

The write's receiver must be a Prisma client by the pack's gate (under query-in-loop in the
[client gate](#prisma-practice-query-in-loop)). A request is a parameter named
`req`/`request`, a parameter typed `…Request` (`NextRequest`, `NextApiRequest`,
`FastifyRequest`), or `c.req`/`ctx.request` of a parameter.

It leaves alone: `data: Schema.parse(body)` and `data: parsed.data` (any call's result); fields picked one by one
(`data: { name: body.name }`); the body as one column's value (`data: { payload: body }`, a Json column); a `let`
reassigned after the read (`body = Schema.parse(body)`); tRPC's `input` (validated by `.input(schema)`) and Hono's
`c.req.valid("json")`; exported functions of files without `'use server'` and non-exported functions in one; calls
whose `<m>` is no Prisma model (`server.services.org.update({ data: req.body })`); tests, stories, fixtures.

**Gaps:** a body passed to a data-layer function in another module that writes it; an argument object built in a
variable (`const args = { data: body }`); nested writes (`data: { posts: { create: body.posts } }`);
`JSON.parse(await req.text())`; NestJS `@Body()` parameters; schemas that keep unknown keys (`.passthrough()`,
`.loose()`, `z.any()`, tRPC `.input(z.any())`); Server Actions exported through `export { f }` or built by wrappers
(`action(schema, fn)`, next-safe-action); a `.strict()` schema whose result is discarded is safe and still reported.
Prisma 8 writes through `tx.orm.<schema>.<Model>.create(...)`, one more member level: not matched.

**Corpus:** run on all 28 Prisma apps of the corpus: 0 matches. They validate before writing (zod `safeParse` then
`parsed.data`, a `cleanRule(input)`
helper, next-safe-action or a local `action(schema, fn)` wrapper, tRPC `.input(...)`). A probe reporting every Prisma
write of a schema model found 13 in spliit, so 0 means no request object reaches `data`, not that no write was seen.

**Test sets:** 5 of 5 real on set #2 and 3 of 3 on set #3, across four apps. Starts on in log mode.

**Sources:**
- OWASP API Security Top 10 2023, [API3:2023 Broken Object Property Level Authorization](https://owasp.org/API-Security/editions/2023/en/0xa3-broken-object-property-level-authorization/): "If possible, avoid using functions that automatically bind a client's input into code variables, internal objects, or object properties ("Mass Assignment")." and "Allow changes only to the object's properties that should be updated by the client."
- Next.js, [Data Security](https://nextjs.org/docs/app/guides/data-security): "You should always validate input from client, as they can be easily modified. For example, form data, URL parameters, headers, and searchParams" and, in its audit checklist, "Are the Action arguments validated in the action or inside the Data Access Layer?"
- Zod, [Defining schemas (objects)](https://zod.dev/api): "By default, unrecognized keys are stripped from the parsed result" (the parsed result, not the object passed in).

### `prisma-practice-side-effect-in-transaction`

Rerail's own rule, not a port. Inside the callback of an interactive transaction, `<client>.$transaction(async (tx)
=> …)` or `$transaction(async function (tx) { … })` (any receiver: the client, an extended client, `ctx.db`,
`this.prisma`), it reports a call that leaves the process:

- the global `fetch(…)`;
- any call on a member chain rooted at an outside-service client made at the top of a module, in the same file or
  imported (`new Stripe(…)`, `new Resend(…)`, bullmq `new Queue(…)`, `new Inngest(…)`, AWS
  `SQSClient`/`SNSClient`/`SESClient`/`SESv2Client`, Postmark `ServerClient`, Slack `WebClient`, `new OpenAI(…)`,
  `new Anthropic(…)`, PostHog's `new PostHog(…)`, `new Pusher(…)`, Twilio's `new Twilio(…)`/`twilio(…)`, Upstash
  QStash's `new Client(…)`, nodemailer `createTransport(…)`, `axios.create(…)`, also behind the
  `globalThis.x ?? new X()` singleton); the `axios`, `got`, `ky` and `@sendgrid/mail` default imports themselves,
  node-fetch's default, undici's `fetch`/`request`, and @trigger.dev/sdk's `tasks`;
- `revalidatePath` and `revalidateTag` from `next/cache`, in a file whose nearest package.json lists `next` (the
  root's when none is nearer);
- a call of a workspace function (a top-level function declaration, a top-level `const f = async () => …`, or a
  default-exported function; same file, imported by a relative path, a tsconfig `paths` alias, or a monorepo package
  by its package.json name, through `export { a as b } from` and `export *` barrels) whose body reaches one of those
  at any depth.

Calls count when they sit in the callback's body or in a callback that an array method there runs at once (`map`,
`forEach`, `flatMap`, `filter`, `reduce`, `some`, `every`, `find`). The message names the call, and for a helper the
call it reaches (`sendReceipt (reaches resend.emails.send)`).

It leaves alone: the same calls before or after the `$transaction(…)` call; any call under a client's `.webhooks`
(`stripe.webhooks.constructEvent`, `generateTestHeaderString`: local signature work) and `sgMail.setApiKey`, which stay
in the process; callbacks the transaction body only defines (`afterCommit.push(() => send(…))`, Next.js
`after(() => …)`); helpers that only query through the `tx` they are given, and pure helpers of a mail module
(`renderReceipt`); the array form `$transaction([q1, q2])`; a `fetch` that is a parameter or local; test files.

**Test sets:** 5 of 5 real on set #2, across two apps; no match on set #3. Starts on in log mode.

### `prisma-practice-one-client`

Rerail's own rule, not a port. A Prisma client is `new PrismaClient(...)` whose constructor is imported under the
name `PrismaClient` (renamed or not, from `@prisma/client`, a Prisma 7 generated folder such as
`@/generated/prisma/client`, or a workspace package that re-exports it), or destructured from a `require()` of a module
whose name mentions prisma. Outside one-shot and test code, it reports two shapes:

- **Per call:** the client is made inside a function (a route handler, server action, API route, method, callback) and
  used only there: read for a member directly (`new PrismaClient().post.create(...)`), or held in a const whose every
  use is a member read (`db.user.findMany()`, `db.$disconnect()`). Each call then opens its own connection pool.
- **Uncached in Next.js:** the client is made when the module loads (at the top level outside a class, or returned by
  a factory in the same file that the file calls at its top level), the nearest package.json lists `next`, the file
  never reads `globalThis` or Node's `global`, and it exports something (`export …`, `module.exports`; a file that
  exports nothing is a script run on its own). Every hot reload of `next dev` makes another; this bites in development
  only (a production server evaluates the module once), and the message says so.

The message for each shape names the problem, the fix (import the app's one client, or Prisma's `globalForPrisma`
pattern) and when to leave it (a function that runs once per process; a module that never runs in the Next.js server).

It leaves alone: the documented singleton (`globalForPrisma.prisma ?? new PrismaClient()`, also through a factory and
`.$extends(...)`); factories that return the new client (`return new PrismaClient()`, `const c = new …; return c`,
`return c.$extends(…)`, an arrow whose body is the client), since their caller decides how often they run; clients
stored anywhere (`if (!client) client = new PrismaClient()`, `this.prisma = …`, `clients.set(k, new PrismaClient())`,
a client passed to a function or put in an object); class fields (one per instance, which dependency injection makes
once); static blocks; a function the file calls at its top level (`main()`, `main().catch(…)`, an IIFE); files whose
path has a part `script`, `scripts`, `bin`, `cli`, `tools`, `playwright`, `cypress`, or a part holding one of the
words test(s), testing, e2e, spec(s), mock(s), seed(s), seeder(s), migrate, migration(s), bench, benchmark(s),
fixture(s) between `-`, `_`, `.` or at a camelCase boundary (`seed-users.ts`, `test-utils/`, `runOpsMigration/`; not
`latest/` or `contest.ts`), besides the scope's test, story and generated-code excludes; module-level clients in a file
that exports nothing, or in a package whose own package.json does not list `next` (the root's when none is nearer: a
worker, a NestJS API, a `packages/db` workspace package); a client built on an HTTP driver adapter or on Accelerate
(`@prisma/adapter-planetscale`, `@prisma/adapter-d1`, `@prisma/adapter-libsql`, `PrismaNeonHTTP`,
`@prisma/extension-accelerate`), which holds no connection pool; and two modules that each make their own client (a
read replica, a second schema, an admin connection are common and deliberate).

**Test sets:** 30 of 30 real on set #2, across four apps (26 in one); set #3's one finding was a separate sidecar
sharing the Next.js package, not a hot-reloaded Next.js client. Combined 30 of 31 (97%); starts on in log mode.

**Sources:**
- Prisma docs (v7), [Database connections](https://www.prisma.io/docs/orm/v7/prisma-client/setup-and-configuration/databases-connections): "✔ Create **one** instance of `PrismaClient` and re-use it across your application" and "✔ Assign `PrismaClient` to a global variable *in dev environments only* to prevent hot reloading from creating new instances"; for serverless: "Instantiate `PrismaClient` outside the scope of the function handler to increase the chances of reuse."
- Prisma docs (v7), [Next.js troubleshooting](https://www.prisma.io/docs/orm/v7/more/troubleshooting/nextjs): "Next.js's hot-reloading feature reloads modules frequently to reflect code changes instantly. However, this can lead to multiple instances of Prisma Client being created, which consumes resources and might cause unexpected behavior." The fix shown ends with `if (process.env.NODE_ENV !== "production") globalForPrisma.prisma = prisma;`.
- Prisma docs (v7), [Connection pool](https://www.prisma.io/docs/orm/v7/prisma-client/setup-and-configuration/databases-connections/connection-pool): the pool defaults table, "Pool size | `connection_limit` | `num_cpus::get_physical() * 2 + 1` | `max` | `10`" (Prisma 6's engine vs Prisma 7's pg adapter).
- Prisma docs, [Best practices](https://www.prisma.io/docs/orm/more/best-practices): "Create one global PrismaClient instance and reuse it throughout your application. Creating multiple instances creates multiple connection pools".

## The corpus

App identities, source commits and finding locations are retained privately for coordinated disclosure. Aggregate evidence is given above.

## The test set

App identities, source commits and finding locations are retained privately for coordinated disclosure. Aggregate evidence is given above.

## Files

| Path | What it is |
|---|---|
| `rules/prisma-practice-<id>.rerail` | Five hand-written rules, each with matching notes, examples and its own helpers |
| `cases/<id>/` | Each rule's cases, a small repository: a line ending in `// caught` must be reported, no other line may be (the rule file's `.test cases`). Not published |
| `candidates/` | Rules written for the pack that do not ship, with their cases and details. Not published |
| [`NOTES-prisma.md`](NOTES-prisma.md) | Aggregate measurement evidence, decisions and generic matching limitations |
| [`ENGINE-GAPS.md`](ENGINE-GAPS.md) | Engine gaps found while writing and measuring these rules |

