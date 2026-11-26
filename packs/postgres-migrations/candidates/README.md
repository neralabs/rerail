# Rerail rule pack: Postgres migrations, candidates

Rules written for the Postgres migrations pack that are not part of it: not listed in `pack.json`, not published
(`package.json` ships only `pack.json`, `rules/` and the licence files). Each was on when the pack was measured on 20
open-source apps that nobody used to write, tune or generalize the rules (the frozen test set, [How the rules were
measured](../README.md#how-the-rules-were-measured)), and none of them matched there. With no match on apps it was not
tuned on, a rule's precision on unseen apps is unmeasured, and a rule ships only when more than 90% of its matches on
such apps are real; each waits for a test set on which it matches. Its corpus and held-out numbers below are tuning
numbers.

Three guard against a lock that stops the app: an index build (`index-on-existing-table-blocks-writes`) or a table
rewrite (`volatile-default-rewrites`, `column-type-change-rewrites`). Two guard against a failed deploy
(`concurrently-inside-transaction`, `required-column-without-default`), which with Prisma also blocks every later
`migrate deploy` until someone resolves it by hand, and two against data loss or a broken app
(`rename-as-drop-and-add`, `dropped-column-still-used`). Every one reads the migrations as SQL trees, except
`dropped-column-still-used`, which reads `schema.prisma`, Drizzle `pgTable` declarations and raw SQL in the app's
TypeScript to find a dropped column's live uses.

Each keeps its rule in `rules/<id>.rerail` and its cases in `cases/<id>/` here. The engine reads an `#include` only
beside the including file, so `rules/lib/postgres-migrations.rerail` here is a copy of the pack's library
(`../../rules/lib/postgres-migrations.rerail`), kept identical to it. The tests read the shared `../../cases/_git/`
(the one-commit `.git`) and `../../cases/_other-databases/` beside the shipped rules' cases. Run one with `rerail
rules test candidates/rules/<id>.rerail`. To ship one, add its entry back to `pack.json`, move its file and cases back
to `rules/` and `cases/` (its shared fixtures back to `../cases/`), and move its details to the README; delete the
copy of the library once no candidate is left.

Columns: the test set (20 apps, every rule on, no Git base); the 8 held-out apps of the generalization pass (the nine
SQL rules, no Git base); and the corpus runs the [README](../README.md#how-the-rules-were-measured) describes: Audit
(no Git base, the 27 corpus apps with SQL migrations; matches and apps), Base (50 pending migrations in 5 clones with a
Git base), Whole history (the review's 3 apps + the writers' runs, the writers' app count in parentheses) and the
precision of the matches read.

| Candidate | Mode it had | Catches | Test set, 20 apps | Held-out, 8 apps | Audit, 27 apps | Base, 50 pending | Whole history | Precision of read matches |
|---|---|---|---|---|---|---|---|---|
| `postgres-migrations-practice-index-on-existing-table-blocks-writes` | Steer | `CREATE [UNIQUE] INDEX` without `CONCURRENTLY`, or `ADD UNIQUE`/`PRIMARY KEY`, on a table that already exists | 0 matches | 3, none real (one operator-run migration that locks the table first); 0 after the explicit-lock fix | 9 (4 apps) | 0 | 204 + 527 (7) | 100% (30 of 30 read by the review, one on a small table; 439 of 442 by the writers, the 3 since fixed) |
| `postgres-migrations-practice-concurrently-inside-transaction` | Steer | `CREATE`/`DROP INDEX`, `REINDEX` or `DETACH PARTITION … CONCURRENTLY` where the runner wraps the file in a transaction | 0 matches | 0 matches | 0 | 0 | 0 + 0 (10) | n/a (2 adversarial files caught; 45 read before the deploy-script gate, none real) |
| `postgres-migrations-practice-required-column-without-default` | Steer | `ADD COLUMN … NOT NULL` with no default on an existing table | 0 matches | 0 matches | 0 | 0 | 10 + 96 (7) | 100% (10 of 10; 96 of 96) |
| `postgres-migrations-practice-volatile-default-rewrites` | Steer | `ADD COLUMN` with a volatile default, a serial type, an identity or a stored generated expression on an existing table | 0 matches | 0 matches | 0 | 0 | 3 + 56 (5) | 100% (3 of 3; 56 of 56) |
| `postgres-migrations-practice-column-type-change-rewrites` | Steer | a column type change that rewrites an existing table | 0 matches | 0 matches | 1 (1) | 4 (1) | 20 + 134 (8) | 100% (20 of 20 after `TIMESTAMPTZ` → `TIMESTAMP` joined its reverse as left alone; 132 of 134) |
| `postgres-migrations-practice-rename-as-drop-and-add` | Log | a `DROP COLUMN` plus an `ADD COLUMN` of the same type in one migration: a rename that deletes the data | 0 matches | 0 matches | 0 | 0 | 4 + 67 (10) | 50% (2 of 4); 82% (55 of 67) |
| `postgres-migrations-practice-dropped-column-still-used` | Log | a column or table dropped while `schema.prisma`, a Drizzle `pgTable` or raw SQL still uses it | 0 matches | not run | 0 in 10 apps (7 before the review's fixes, all civitai, all false) | — | 0 (13) | n/a (7 of 7 false before the fixes; a drop added to a copy of rallly is reported) |

On the three corpus repositories with the longest histories, a whole-repository audit (each folder's newest
migration) gave, against the writers' whole-history matches: `index-on-existing-table-blocks-writes` langfuse 1 (was
93), trigger.dev 0, documenso 0 (was 73); `required-column-without-default` 0 (was 9 in langfuse and in documenso);
`column-type-change-rewrites` 0 (was 10, 17, 9); `rename-as-drop-and-add` 0 (was 11, 24, 4);
`concurrently-inside-transaction`, `volatile-default-rewrites` and `dropped-column-still-used` 0 (was 0). The one
match, langfuse's `20261120140000_add_gateway_spend_limit_warning_thresholds`, rebuilds a unique index on
`gateway_spend_limit_alerts`, a table an earlier migration created, without `CONCURRENTLY` (line 10): real. Over the
27 corpus apps with SQL migrations, `index-on-existing-table-blocks-writes` gave 9 findings in 4 apps (papermark's
`20260820000000_add_multiple_team_brands` alone 5) and `column-type-change-rewrites` 1 (cal.diy's enum recreation on
`Webhook`), all real (one on a small table, airbroke's `accounts`); `dropped-column-still-used`, which before the
review read every migration as raw text, gave 7 findings on civitai, all false (commented-out drops, a `DROP TABLE`
name list that ran into the next comment, a `/// @view` model, an output alias without AS); it now reads only pending
migrations and skips comments, and gives 0 on civitai and on the nine other apps re-run.

## Rule details

What each candidate matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources.
Every SQL candidate reads `**/migrations/**/*.sql` and `**/drizzle/**/*.sql` outside test, fixture and build folders,
only in pending PostgreSQL migrations (see [Which migrations a rule
reads](../README.md#which-migrations-a-rule-reads)); the lists below do not repeat that.

### `postgres-migrations-practice-index-on-existing-table-blocks-writes`

**Why a candidate.** 0 matches on the 20 test apps. Held-out: 3 matches, none real, all in one jitsu migration run by
an operator that takes `LOCK TABLE … IN ACCESS EXCLUSIVE MODE` (writes stopped first) before its index builds; the
explicit-lock fix leaves them alone (0 on the re-run).

A plain `CREATE INDEX` locks the table against writes for the whole build, and `ADD CONSTRAINT … UNIQUE` / `ADD
PRIMARY KEY` build the same index under an ACCESS EXCLUSIVE lock that blocks reads too. Prisma writes a plain `CREATE
UNIQUE INDEX` for every new `@unique` and `CREATE INDEX` for every `@@index`, so adding one to a model whose table is
in production stalls the app for as long as the build takes.

Catches, on a table that already exists:
- `CREATE INDEX` and `CREATE UNIQUE INDEX` without `CONCURRENTLY` (sqlfluff PG01 skips UNIQUE);
- `ALTER TABLE … ADD [CONSTRAINT …] UNIQUE (…)` and `ADD [CONSTRAINT …] PRIMARY KEY (…)`;
- `ALTER TABLE … ADD COLUMN … UNIQUE` / `… PRIMARY KEY`.

The fix in the message follows the runner, told from the layout: Prisma (`…/migrations/<dir>/migration.sql`: put
`CREATE INDEX CONCURRENTLY` alone in a migration of its own, which works on every Prisma version, written with
`prisma migrate dev --create-only` and edited before it is applied, never by editing a migration `migrate dev` has
already applied), drizzle-kit (a `meta/_journal.json` beside the file, or drizzle-kit 1.0's `snapshot.json` beside a
`migration.sql`: drizzle runs all pending migrations in one transaction, so build the index by hand before deploying
and write it in the migration as `CREATE INDEX IF NOT EXISTS`, which the rule then accepts), or plain SQL
(`CONCURRENTLY`, outside any transaction). Constraints are steered to a concurrent unique index plus `ADD CONSTRAINT …
UNIQUE USING INDEX` / `PRIMARY KEY USING INDEX`. An index on a partitioned parent (a table created `PARTITION BY …`),
where PostgreSQL refuses `CONCURRENTLY`, gets the per-partition fix instead: the index on the parent alone with `ON
ONLY`, `CREATE INDEX CONCURRENTLY` on each partition, then `ALTER INDEX … ATTACH PARTITION`.

Leaves alone:
- `CREATE [UNIQUE] INDEX CONCURRENTLY`; `ADD CONSTRAINT … UNIQUE USING INDEX …` and `PRIMARY KEY USING INDEX …`;
- a table that is not an existing one (created, materialized or emptied earlier in the file, or given a required
  column first);
- `CREATE INDEX … ON ONLY t` (a partitioned parent's index shell);
- in a Drizzle migration, `CREATE [UNIQUE] INDEX IF NOT EXISTS`: the form the Drizzle fix asks for, the index built
  by hand beforehand (before this, the rule reported the statement its own message prescribed);
- `ADD CONSTRAINT … UNIQUE`/`PRIMARY KEY` on a partitioned parent (`USING INDEX` is refused there);
- `ADD CONSTRAINT … EXCLUDE` (no `USING INDEX` form to steer to);
- an `ALTER TABLE` that also changes a column's type (`ALTER COLUMN "id" SET DATA TYPE TEXT, ADD CONSTRAINT …
  PRIMARY KEY`, Prisma's primary-key swap): the rewrite holds the lock anyway, and the column-type-change rule owns it;
- a statement on a table the migration has already locked itself with `LOCK TABLE t` (ACCESS EXCLUSIVE, the default
  mode; for a `CREATE INDEX`, any mode that stops writes: SHARE, SHARE ROW EXCLUSIVE, EXCLUSIVE): the lock holds to
  the end of the transaction, so the build stalls nothing the migration did not choose to stop (an operator-run
  migration in a maintenance window).

Gaps: a table known to stay small is reported (nothing in the repository says how big a table is). `EXCLUDE`
constraints, and `UNIQUE`/`PRIMARY KEY` constraints on a partitioned table, also block and are not reported. A Drizzle
`IF NOT EXISTS` index is trusted to have been built by hand. The Prisma version is not read, so the Prisma fix is the one that works on every version
(alone in its own migration), though Prisma 7.4+ also accepts it beside other statements outside `BEGIN … COMMIT`.

Sources:
- PostgreSQL, CREATE INDEX, "Building Indexes Concurrently"
  (https://www.postgresql.org/docs/current/sql-createindex.html#SQL-CREATEINDEX-CONCURRENTLY): "Normally PostgreSQL
  locks the table to be indexed against writes and performs the entire index build with a single scan of the table.
  Other transactions can still read the table, but if they try to insert, update, or delete rows in the table they
  will block until the index build is finished."
- PostgreSQL, ALTER TABLE (https://www.postgresql.org/docs/current/sql-altertable.html): `ADD table_constraint_using_index`
  "This form adds a new PRIMARY KEY or UNIQUE constraint to a table based on an existing unique index."
- Squawk, require-concurrent-index-creation (https://squawkhq.com/docs/require-concurrent-index-creation): "During a
  normal index creation, table updates are blocked, but reads are still allowed." / "This rule ignores indexes added
  to tables created in the same transaction."
- Squawk, disallowed-unique-constraint (https://squawkhq.com/docs/disallowed-unique-constraint): "Adding a `UNIQUE`
  constraint requires an `ACCESS EXCLUSIVE` lock which blocks reads and writes to the table while the index is
  built."
- strong_migrations (https://github.com/ankane/strong_migrations#adding-an-index-non-concurrently): "In Postgres,
  adding an index non-concurrently blocks writes." / "indexes on new tables (those created in the same migration)
  don’t require this."
- Prisma (https://github.com/prisma/prisma/issues/14456): "ERROR: CREATE INDEX CONCURRENTLY cannot run inside a
  transaction block"; "I've found that a migration script that contains ONLY the `CREATE INDEX CONCURRENTLY` command
  will succeed."
- drizzle-orm (https://github.com/drizzle-team/drizzle-orm/issues/860, open): "[BUG]: Can't create index
  concurrently due to transaction block".

Corpus, review: audit of the 27 apps with SQL migrations, 9 matches in 4 (papermark 5 on `Brand`, `Team`, `Link`,
`Dataroom`; langfuse 1; airbroke 1, on the one-row-per-login `accounts`; customermates 2), all real; base, 0 in the 50
pending migrations (all 36 plain `CREATE INDEX` there are on tables created in the same file or by another pending
migration, which the Git-base exemption leaves alone); whole history of papermark, comp and customermates, 204 (64, 60,
80), 17 read, all real. Re-run after the fixes: the same 9 audit and 204 whole-history matches, and 0 on trigger.dev
(its partitioned tables are indexed `ON ONLY` or on new tables); the adversarial Drizzle `CREATE INDEX IF NOT EXISTS`
is no longer reported, the drizzle-kit 1.0 folder gets the Drizzle text, and a partitioned parent the per-partition
one. Writers' whole-history run on 7 repositories with the most Prisma migration SQL (umami, rallly, documenso,
useSend, langfuse, shelf.nu, inbox-zero): 527 matches in 7. 442 read (every match of six repositories, every fourth of inbox-zero's
118): 439 real, 3 not, all three Prisma's primary-key swap with `SET DATA TYPE` in the same `ALTER TABLE` (since left
to the column-type-change rule). Every reported statement builds an index on a table an earlier migration created. The
corpus also shows the fix in use: langfuse writes each `CREATE INDEX CONCURRENTLY` alone in its own Prisma migration
(`20240226165118_add_observations_index` and 20 more), useSend does the same
(`20261117010000_email_team_created_at_index`), and shelf.nu's own comments make the size call the message asks for
(`20260525110348_add_trigram_indexes_for_simple_search`: "Each statement takes an ACCESS EXCLUSIVE lock on its table
for the duration of the index build. On the largest known tenant the affected tables are small … If a much larger
workspace is onboarded later, run this migration during a low-traffic window or split it across multiple
CONCURRENTLY-built migrations.").

### `postgres-migrations-practice-concurrently-inside-transaction`

**Why a candidate.** 0 matches on the 20 test apps and 0 on the 8 held-out apps, so it has matched no app it was not
tuned on.

PostgreSQL refuses `CREATE INDEX CONCURRENTLY`, `DROP INDEX CONCURRENTLY`, `REINDEX … CONCURRENTLY` and `ALTER TABLE …
DETACH PARTITION … CONCURRENTLY` inside a transaction block. A migration that puts one there fails on deploy: Prisma
records it as failed and refuses every later `migrate deploy` until someone runs `migrate resolve` by hand; Drizzle
rolls back the whole pending run. It is the usual result of adding `CONCURRENTLY` to a generated `CREATE INDEX` (the
fix `index-on-existing-table-blocks-writes` asks for) without moving it. Who adds the transaction depends on the
runner, which the rule reads from the layout and `package.json`.

Catches:
- any of the four statements between an explicit `BEGIN` and `COMMIT`, whatever the runner;
- in a Prisma migration (`…/migrations/<dir>/migration.sql`) with any other statement in the file, when the package
  that runs it (the nearest `package.json` listing `prisma` or `@prisma/client`, else the root one) does not pin
  Prisma 7.4 or later and something in the repository runs `prisma migrate deploy` or `migrate dev`: a `package.json`
  script, or a Dockerfile, shell script, compose file, CI workflow, platform deploy file (render.yaml, Procfile, …),
  Makefile or a JS/TS script named for migrations (`migrate.mjs`) found by the library's checkout walk. Prisma before
  7.4 sends the file as one batch, which PostgreSQL runs as one transaction;
- in a Drizzle migration (a `.sql` file beside `meta/_journal.json`, or drizzle-kit 1.0's `<ts>_x/migration.sql`
  beside a `snapshot.json`): drizzle-kit `migrate` and drizzle-orm's `migrate()` run all pending migrations in one
  transaction.

Leaves alone:
- the statement alone in its Prisma migration (comments do not count);
- a Prisma migration of a package whose `prisma` range is entirely `>=7.4.0` (Prisma 7.4 sends each statement on its
  own), unless the statement sits in `BEGIN … COMMIT`;
- a Prisma migration in a repository where nothing runs `migrate deploy`/`migrate dev` (`migrate dev --create-only`
  applies nothing and does not count): the team applies migrations another way, by hand, unless the statement sits in
  `BEGIN … COMMIT`;
- `REFRESH MATERIALIZED VIEW CONCURRENTLY`, which PostgreSQL allows inside a transaction;
- `.sql` migrations of other runners (no Prisma layout, no Drizzle journal) unless in `BEGIN … COMMIT`.

Gaps: a Drizzle project applying its `.sql` files with its own non-transactional script is reported. A Prisma project
that runs `prisma migrate deploy` only from a script under another name (`deploy.mjs`), or from a file more than four
folders deep, is not reported; a project that applies its migrations by hand but keeps a helper script that runs (or
prints) `prisma migrate dev`/`deploy` is reported. Any `package.json` script or configuration file of the checkout
counts for every Prisma folder. A Prisma version written as `catalog:`, `workspace:` or a
dist-tag is read as before 7.4. Prisma 7.5's shadow database was reported to still reject a mixed file on `migrate
dev`; 7.4+ files are not reported. A singular `migration/` folder (`packages/database/migration/`) is outside the
pack's `**/migrations/**` scope. Prisma 8 `ops.json` migrations and other runners' wrapping (node-pg-migrate, Supabase
CLI, golang-migrate) are not read.

Sources:
- PostgreSQL, CREATE INDEX (https://www.postgresql.org/docs/current/sql-createindex.html): "Another difference is that
  a regular CREATE INDEX command can be performed within a transaction block, but CREATE INDEX CONCURRENTLY cannot."
- Squawk, ban-concurrent-index-creation-in-transaction
  (https://squawkhq.com/docs/ban-concurrent-index-creation-in-transaction): "For migrations that are implicitly
  wrapped in a transaction, ensure that the `CREATE INDEX` command is the only command in the migration to allow
  migration tool to detect that no transaction is needed."
- prisma/prisma#14456 (https://github.com/prisma/prisma/issues/14456): "ERROR: CREATE INDEX CONCURRENTLY cannot run
  inside a transaction block"; "I've found that a migration script that contains ONLY the `CREATE INDEX CONCURRENTLY`
  command will succeed."; closed with "This has been fixed in Prisma 7.4.0".
- prisma-engines#5767 (https://github.com/prisma/prisma-engines/pull/5767): "The reason is that the Postgres simple
  protocol automatically wraps the script in a transaction, which is sometimes undesirable (e.g. when the script
  contains statements that cannot be run inside a transaction like `CREATE INDEX CONCURRENTLY`)."
- Prisma error reference (https://www.prisma.io/docs/orm/v7/reference/error-reference): P3018 "A migration failed to
  apply. New migrations cannot be applied before the error is recovered from."; P3009 "migrate found failed
  migrations in the target database, new migrations will not be applied".
- drizzle-orm `pg-core/dialect.ts`
  (https://github.com/drizzle-team/drizzle-orm/blob/main/drizzle-orm/src/pg-core/dialect.ts):
  `await session.transaction(async (tx) => { for await (const migration of migrations) { …`; drizzle-orm#860
  (https://github.com/drizzle-team/drizzle-orm/issues/860): "[BUG]: Can't create index concurrently due to
  transaction block".

Corpus, review: 0 in the audit of the 27 apps with SQL migrations, in the 50 base-pending migrations and in the whole
histories of papermark, comp and customermates; the two adversarial files (a `SET lock_timeout` beside a
`CONCURRENTLY` in a Prisma 6 file, a Drizzle journal migration) are reported. After the fixes, whole history of civitai,
dossier, customermates and crawlseo (the last three run `prisma migrate deploy` only from a Dockerfile, a shell script
or CI, now read): still 0, and civitai's JS helper around `migrate dev` does not bring back its 45 hand-applied
matches. Writers' run on 10 repositories with the most Prisma migration SQL or any `CONCURRENTLY` in a migration
(trigger.dev, langfuse, documenso, inbox-zero, shelf.nu, trycompai/comp, civitai, cal.diy, useSend, kun-touchgal-next),
whole history: 0 matches. The teams that run `prisma migrate deploy` (trigger.dev 94 `CONCURRENTLY` migrations, langfuse 104, cal.diy,
inbox-zero, useSend) all put the statement alone in its migration, the convention the message asks for. Before the
`migrate deploy` script gate, civitai gave 45 matches in 31 migrations, none real: civitai applies its migrations by
hand, statement by statement (its files say "CONCURRENTLY cannot run inside a transaction, so run each statement on
its own"). No corpus repository has a Drizzle migration with `CONCURRENTLY` or one inside an explicit `BEGIN … COMMIT`
(civitai's PL/pgSQL `BEGIN` in `$$` bodies is correctly not read as a transaction), so those two clauses are proven by
the cases only. Kept at 0 matches because the failure is severe (prisma/prisma#14456 and drizzle-orm#860: the deploy
fails and, with Prisma, blocks every later one until resolved by hand) and it is the exact mistake an agent makes when
it follows `index-on-existing-table-blocks-writes` naively.

### `postgres-migrations-practice-required-column-without-default`

**Why a candidate.** 0 matches on the 20 test apps and 0 on the 8 held-out apps, so it has matched no app it was not
tuned on.

`ALTER TABLE t ADD COLUMN c … NOT NULL` with no `DEFAULT` has no value for the rows `t` already holds, so PostgreSQL
refuses it on any table that is not empty ("column c contains null values"). A dev database is usually empty, so
`prisma migrate dev` applies it and the migration reaches production, where the deploy fails, and a failed Prisma
migration blocks every later one (P3018, then P3009) until someone resolves it by hand. Prisma writes "This is not
possible if the table is not empty." into the migration and applies it anyway. App instances still running the old
code also fail to insert while a deploy rolls out.

Catches, on a table that already exists: `ALTER TABLE … ADD [COLUMN] c <type> NOT NULL` with no `DEFAULT`, one finding
per column (Prisma's `ALTER TABLE "T" ADD COLUMN "a" … NOT NULL, ADD COLUMN "b" … NOT NULL` gives two). This includes a
column whose Prisma field has `@updatedAt`, `@default(cuid())`, `@default(uuid())` or `@default(nanoid())`: those are
Prisma-level defaults and put no `DEFAULT` in the SQL. The message steers to a constant `DEFAULT`, or to adding the
column nullable, backfilling it and making it `NOT NULL` in a later migration (in Prisma: optional field, migrate,
backfill, required field), and says the statement is fine when the table is certain to be empty in every environment.

Leaves alone:
- a column with `DEFAULT …`, `GENERATED ALWAYS AS (…) STORED`, `GENERATED … AS IDENTITY`, or a `serial` type
  (`serial`, `bigserial`, `smallserial`, `serial2/4/8`); a nullable column; columns inside `CREATE TABLE`;
- a table created, truncated or wholly deleted (`DELETE FROM t` without `WHERE`) earlier in the same file, or, with a
  Git base, created by a pending migration (an earlier required column of the same file is no proof here: it is this
  rule's own finding);
- `ADD COLUMN … PRIMARY KEY` without `NOT NULL`: `index-on-existing-table-blocks-writes` reports it, so one line does
  not get two steers.

Gaps: a table known to be empty in production for another reason (a feature shipped dark) is reported. `schema.prisma`
is not read, so the message names Prisma-level defaults in general rather than for the one field.

Sources:
- Squawk, adding-required-field (https://squawkhq.com/docs/adding-required-field): "Adding a new column that is `NOT
  NULL` and has no default value to an existing table effectively makes it required. … This will fail immediately upon
  running for any populated table. Furthermore, old application code that is unaware of this column will fail to
  `INSERT` to this table."
- prisma-engines, `unexecutable_step_check.rs`
  (https://github.com/prisma/prisma-engines/blob/main/schema-engine/connectors/sql-schema-connector/src/sql_destructive_change_checker/unexecutable_step_check.rs):
  "Added the required column `{column}` to the `{table}` table without a default value. {details}" and "The required
  column `{column}` was added to the `{table}` table with a prisma-level default value. {details} Please add this
  column as optional, then populate it before making it required."
- Prisma 8 docs, editing a migration (https://www.prisma.io/docs/orm/migrations/editing-a-migration): "`addColumn`
  adds the column as nullable, `dataTransform` holds the backfill you write, and `setNotNull` makes the column
  required".
- Prisma error reference (https://www.prisma.io/docs/orm/v7/reference/error-reference): P3009 "migrate found failed
  migrations in the target database, new migrations will not be applied".

Corpus, review: 0 in the audit of the 27 apps with SQL migrations and in the 50 base-pending migrations; whole
history of papermark (2) and comp (8), all carrying Prisma's own "Added the required column" warning, all real. The
adversarial `rollback.sql` beside a Prisma `migration.sql` (civitai keeps three, and a `backfill.sql`) was read as a
pending migration; it is no migration now (re-run on civitai: 0). Writers' run (8 repositories with the largest Prisma
migration histories), whole history: 96 matches in 7 of 8 (rallly 1,
comp 8, inbox-zero 8, zoonk 53, documenso 9, shelf.nu 8, langfuse 9, umami 0); all 96 read, all the shape the rule
names (a NOT NULL column with no default added to a table created by an earlier, separate migration; Prisma's own
warning sits above 51 of them). 22 of the 96 come hours after the migration that created the table, so they were
probably written in the same unreleased change: a Git base exempts those at review time. The DELETE exemption is
confirmed on langfuse (`DELETE FROM "project_memberships";` then `ADD COLUMN "org_membership_id" TEXT NOT NULL`, with
the comment "ok to be not null as it's a new column on a now empty table": not reported). rallly shows the practice the
rule steers to (`backfill_participant_token` then `require_participant_token`).

### `postgres-migrations-practice-volatile-default-rewrites`

**Why a candidate.** 0 matches on the 20 test apps and 0 on the 8 held-out apps, so it has matched no app it was not
tuned on.

Since PostgreSQL 11, `ADD COLUMN … DEFAULT <constant>` (or a stable expression such as `now()`) only records the
default in the catalog. A volatile default must be computed for each existing row, so PostgreSQL rewrites the whole
table and its indexes under an ACCESS EXCLUSIVE lock: every read and write to the table waits until it is done. A
serial type (a `nextval` default), an identity column and a stored generated column do the same. Prisma writes these
for a new field with `@default(dbgenerated("gen_random_uuid()"))`, `@default(autoincrement())`, or an `@id` changed to
`Int @default(autoincrement())` (`DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL`).

Catches, on `ALTER TABLE … ADD COLUMN` of an existing table:
- `DEFAULT` containing a call to `gen_random_uuid`, `uuidv4`, `uuidv7`, `uuid_generate_v1/v1mc/v4/v7`,
  `gen_random_bytes`, `gen_salt`, `random`, `random_normal`, `clock_timestamp`, `timeofday`, `nextval`,
  `txid_current` (also nested: `md5(random()::text)`);
- `SERIAL`, `BIGSERIAL`, `SMALLSERIAL`, `serial2/4/8`;
- `GENERATED … AS IDENTITY`;
- `GENERATED ALWAYS AS (…) STORED`.

Leaves alone: `DEFAULT CURRENT_TIMESTAMP`/`now()` (stable) and constants; `GENERATED ALWAYS AS (…) VIRTUAL`; `ALTER
COLUMN … SET DEFAULT gen_random_uuid()` (new rows only); `CREATE TABLE` columns; a table that is not an existing one;
a default calling a function of unknown volatility.

Gaps: table size is unknown, so a small table (a rewrite of milliseconds) is reported too; the message says to keep
the statement then. A domain type with constraints also rewrites and is not reported. User-defined volatile functions
are unknown.

Sources:
- PostgreSQL, ALTER TABLE (https://www.postgresql.org/docs/current/sql-altertable.html): "Adding a column with a
  volatile DEFAULT (e.g., clock_timestamp()), a stored generated column, an identity column, or a column with a domain
  data type that has constraints will cause the entire table and its indexes to be rewritten."
- strong_migrations (https://github.com/ankane/strong_migrations#adding-a-column-with-a-volatile-default-value):
  "Adding a column with a volatile default value to an existing table causes the entire table to be rewritten. During
  this time, reads and writes are blocked."
- Squawk, adding-field-with-default (https://squawkhq.com/docs/adding-field-with-default): "Adding a stored generated
  column in Postgres will cause a table rewrite".
- civitai's own migration comments (`packages/civitai-db-schema/prisma/migrations/20261128120000_scan_label_claim_token`):
  "The volatile default rewrites "ScanLabelClaim", which holds only the leases of the last few minutes plus unvoted
  ones, so the lock is brief." and (`20260715120000_w13_app_listing_comments`): "Adding a SERIAL column to a populated
  table backfills every existing row via its DEFAULT".

Corpus, review: 0 in the audit of the 27 apps with SQL migrations and in the 50 base-pending migrations; whole
history of customermates, 3 (`GENERATED ALWAYS AS (…) STORED` on the existing `Contact` and `CustomFieldValue`), real.
Writers' run (10 repos with the most migration SQL: cal.diy, inbox-zero, comp, zoonk, rallly, langfuse, trigger.dev,
civitai, shelf.nu, documenso; 4,700+ migration files), whole history: 56 matches in 5 (zoonk 35, civitai 18, cal.diy 1,
inbox-zero 1, rallly 1), all 56 read, 56 real (every one rewrites a table that existed before the migration). Two are
on tables their authors know are small (rallly's one-row `instance_settings`, civitai's `ScanLabelClaim`, whose
comment accepts the rewrite): the message's keep-if-small clause is the right advice there. Most matches are Prisma's
id-type swap (`DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL` / `UUID … DEFAULT uuidv7()`).

### `postgres-migrations-practice-column-type-change-rewrites`

**Why a candidate.** 0 matches on the 20 test apps and 0 on the 8 held-out apps, so it has matched no app it was not
tuned on.

Changing a column's type (`ALTER TABLE … ALTER COLUMN … [SET DATA] TYPE …`) makes PostgreSQL rewrite the whole table
and every index on it while it holds an ACCESS EXCLUSIVE lock, unless the old type is binary coercible to the new one
and no USING clause changes the values. Prisma emits these statements for a changed field type (`Int` → `BigInt`,
`@db.VarChar(100)` → `@db.VarChar(50)`, a new `Decimal` scale) and, inside `BEGIN … COMMIT`, for every enum change
that removes a value (`TYPE "E_new" USING (…::text::"E_new")`).

Catches, on an existing table:
- a type change whose old type, the column's latest earlier definition in the same migrations folder (CREATE TABLE,
  ADD COLUMN or an earlier TYPE change, in migration-name order), is not a known no-rewrite widening: `INTEGER` →
  `BIGINT`, `TEXT` → `VARCHAR(80)`, `VARCHAR(255)` → `VARCHAR(50)`, `DECIMAL(10,2)` → `DECIMAL(10,4)`, `integer` →
  `numeric(12, 2)`;
- any type change with a `USING` that converts the values (Prisma's enum recreation `USING ("x"::text::"E_new")`);
- with the old type unknown (history squashed, the column dropped and re-added or renamed into place), any `USING`:
  PostgreSQL asks for one only when no assignment cast converts the values.

Leaves alone: a table that is not an existing one; `VARCHAR(n)` → `VARCHAR(m ≥ n)`, `VARCHAR` or `TEXT`;
`NUMERIC(p,s)` → `NUMERIC(q ≥ p,s)` or `NUMERIC`; a precision increase or removal on `TIMESTAMP`, `TIMESTAMPTZ`, `TIME`,
`INTERVAL`, `VARBIT`; `TIMESTAMP` → `TIMESTAMPTZ` and `TIMESTAMPTZ` → `TIMESTAMP` alike (no rewrite on PostgreSQL 12+
when the session time zone is UTC);
`TEXT`/`VARCHAR` ↔ `CITEXT` (binary casts); the same type again, also with a `USING` that only casts the column
(`USING "x"::TEXT` on a TEXT column); a column whose old type no earlier migration shows (squashed or baselined
history, or a DROP/RENAME COLUMN since), unless the change has `USING`.

Gaps: a renamed column or table is not followed back to its old definition (unknown type: reported only with `USING`);
a schema-qualified table is matched by its last name only; the grammar drops the precision of `TIMESTAMPTZ(3)`, so a
precision decrease on it is not reported; a domain over the new type, and a value-keeping `USING` on a column of
unknown type, are reported though they need no rewrite; the table's size is unknown, so a small lookup table is
reported too (the message says the change is fine there); without a Git base, a table created by the previous, still
pending migration counts as existing.

Sources:
- PostgreSQL, ALTER TABLE (https://www.postgresql.org/docs/current/sql-altertable.html): "Changing the type of an
  existing column will normally cause the entire table and its indexes to be rewritten. As an exception, when changing
  the type of an existing column, if the USING clause does not change the column contents and the old type is either
  binary coercible to the new type or an unconstrained domain over the new type, a table rewrite is not needed."
- strong_migrations (https://github.com/ankane/strong_migrations#changing-the-type-of-a-column): "Changing the type of
  a column causes the entire table to be rewritten. During this time, reads and writes are blocked in Postgres", with
  its table of safe changes ("`string` | Increasing or removing `:limit`, changing to `text`").
- prisma-engines `warning_check.rs` (the warning Prisma prints for these migrations): "You are about to alter the
  column `{column}` on the `{table}` table … The data in that column will be cast from `{previous_type}` to
  `{next_type}`."
- Squawk, changing-column-type (https://squawkhq.com/docs/changing-column-type): "An `INT` (4 bytes wide) cannot be
  converted to a `BIGINT` (8 bytes wide) without rewriting the table."

Corpus, review: audit of the 27 apps with SQL migrations, 1 (cal.diy's enum recreation on `Webhook`); base, 4 in
cal.diy (an enum-array recreation and the same); whole history of comp (16), customermates (4) and papermark (2): enum
recreations, enum ↔ TEXT, `INTEGER` → `BIGINT`, `JSONB` → `TEXT`, real, but comp's two `TIMESTAMPTZ` → `TIMESTAMP(3)`
(`20260121144812_alter_fleet_policy_result_timestamps`) skip the rewrite under TimeZone UTC exactly as the exempted
reverse does. Both directions are left alone now; the re-run gives 20 (comp 14), all real. Writers' run (langfuse,
comp, inbox-zero, zoonk, rallly, shelf.nu, documenso, trigger.dev), whole history: 134 matches in 8,
all read (zoonk's 65 by message and a sample of its files); 132 real: Prisma enum recreations, `SERIAL`/`INTEGER` →
`BIGINT`/`TEXT` id changes, `INTEGER` → `DOUBLE PRECISION`/`DECIMAL(65,30)`, `JSONB` → `JSON`/`TEXT`, enum ↔ `TEXT`,
`TIMESTAMP(3)` → `BOOLEAN`. The 2 others alter a table created by the migration one second earlier; with a Git base
that holds neither, the rule leaves them alone. Near misses seen and left alone: rallly's two `SET DATA TYPE CITEXT`
(from TEXT), comp's two `TYPE TEXT USING "x"::TEXT` on columns already TEXT.

### `postgres-migrations-practice-rename-as-drop-and-add`

**Why a candidate.** 0 matches on the 20 test apps and 0 on the 8 held-out apps, so it has matched no app it was not
tuned on.

`prisma migrate dev` cannot tell a renamed field from a removed field plus a new one. Renaming `biograpy` to
`biography` in `schema.prisma` generates `ALTER TABLE "Profile" DROP COLUMN "biograpy", ADD COLUMN "biography" TEXT NOT
NULL`; drizzle-kit writes the same when its rename prompt is answered "create column". Deployed, it deletes every value
of the old column, with no error. The rule reports the DROP COLUMN when the same migration file adds another column to
the same table with the dropped column's type, which it reads from the column's latest earlier definition in the same
migrations folder (CREATE TABLE, ADD COLUMN, ALTER COLUMN TYPE, in migration-name order).

Catches:
- `ALTER TABLE "Profile" DROP COLUMN "biograpy", ADD COLUMN "biography" TEXT NOT NULL;` (Prisma's form, one statement);
- `ALTER TABLE "Post" DROP COLUMN "state";` with `ALTER TABLE "Post" ADD COLUMN "status" "PostState" …` later in the
  same file (an enum column renamed);
- drizzle-kit: `ALTER TABLE "users" DROP COLUMN IF EXISTS "full_name";--> statement-breakpoint` with `ALTER TABLE
  "users" ADD COLUMN "name" varchar(255);`.

Leaves alone:
- `ALTER TABLE … RENAME COLUMN a TO b` (the data stays);
- a drop and an add of different types (`BOOLEAN` dropped, `TEXT` added): a new column, not a rename;
- a column dropped and re-added under its own name (a hand-written type change);
- a foreign key moved to another table (`userId` → User dropped, `memberId` → Member added): the values cannot carry
  over; a referenced table renamed in between (`"Item" RENAME TO "Asset"`) still counts as the same table;
- several drops and one add: the add pairs only with the drop named like it (`profileBio`, not `profileURL`, → `bio`;
  names compare case-folded without underscores, so `updatedAt` → `updated_at` pairs; one name must hold the other
  and the shorter must have at least three characters, so `ip` is not like `zip`); with no name alike, only a
  one-to-one match counts (five TEXT columns dropped and one added is a restructure);
- an id replacing a name or the reverse (`queueName` → `queueId`, `versionId` → `version`; an id is `id` or a name
  ending in `Id`, `ID` or `_id`, so `externalIdentifier` → `externalRef` is a rename);
- a file that UPDATEs the table, or reads the dropped column in a query: the values are carried over before the drop;
- a table created or emptied earlier in the same file, or by a pending migration with a Git base (a required column
  added in the same file is no proof here: Prisma's rename form adds exactly that);
- a column whose type no earlier migration of its folder shows (squashed or baselined history).

Gaps: a rename split over two migration files is not seen; renames of the column or table since its definition are
not followed; a schema-qualified table is matched by its last name part; a real replacement by an unrelated column of
the same type is reported (`enabled` → `disabled`, `token` → `encryptedToken`: the message first asks whether the
new column holds the same facts, and says to keep the drop and add when the meaning changed); a
foreign key is read only from `ADD CONSTRAINT … FOREIGN KEY`; a pair with alike names is reported even when the
referenced table changed; `schema.prisma` is not read.

Sources:
- Prisma, Customizing migrations, "Example: Rename a field"
  (https://www.prisma.io/docs/orm/v7/prisma-migrate/workflows/customizing-migrations#example-rename-a-field): "By
  default, renaming a field in the schema results in a migration that will: `CREATE` a new column (for example,
  `fullname`) `DROP` the existing column (for example, `name`) and the data in that column", with the generated `ALTER
  TABLE "Profile" DROP COLUMN "biograpy", ADD COLUMN "biography" TEXT NOT NULL;`
- prisma-engines, `warning_check.rs`: "You are about to drop the column `{column}` on the `{table}` table. All the
  data in the column will be lost."
- Prisma, Database mapping (https://www.prisma.io/docs/orm/v7/prisma-schema/data-model/database-mapping): "`@map` and
  `@@map` allow you to tune the shape of your Prisma Client API by decoupling model and field names from table and
  column names in the underlying database."
- Squawk, renaming-column (https://squawkhq.com/docs/renaming-column): "If you're using an ORM (object relational
  mapper), you can rename the field in your application level struct while leaving the column name unchanged."

Corpus, review: 0 in the audit of the 27 apps with SQL migrations and in the 50 base-pending migrations; whole
history of comp (3) and customermates (1): comp's `relatedId → entityId` and `evidence → tasks` real, comp's `dueDate →
lastCompletedAt` and customermates' `emailsText → avatarUrl` replacements with a new meaning (2 of 4). An agent that
followed the old message's RENAME on such a replacement (civitai's `enabled → disabled`) would keep values that now
mean the opposite; the message now asks first whether the new column holds the same facts. Writers' run (12
migration-heavy repositories), whole history: 67 matches in 10 of 12 (trigger.dev 24, langfuse 11, civitai
8, shelf.nu 5, zoonk 5, documenso 4, cal.diy 3, comp 3, inbox-zero 2, customermates 2; rallly and dossier 0), all 67
read, 55 real: renames written by `prisma migrate dev` (trigger.dev's `rename_*` migrations, langfuse's camelCase →
snake_case switch, cal.diy's `orgId` → `organizationId`, shelf.nu's `itemId` → `assetId`, civitai's `sizeKB` →
`sizeKb`). The 12 not real are deliberate replacements by an unrelated column of the same type. Log (below 95%).

### `postgres-migrations-practice-dropped-column-still-used`

**Why a candidate.** 0 matches on the 20 test apps. Not run on the held-out apps (the generalization runs read only
the nine SQL rules), and 0 corpus matches since the review's fixes, so it has never been seen to match a real
repository: a history that applied cleanly rarely drops a column it still declares.

Catches a migration (`**/migrations/**/*.sql`, `**/drizzle/**/*.sql`) that drops a column (`ALTER TABLE "T" DROP
COLUMN "c"`, also one action of a multi-action `ALTER TABLE`, `IF EXISTS`) or a table (`DROP TABLE "T"`) while the app
still uses it. To find the uses it reads three places:

- `schema.prisma` (or `schema/*.prisma`) of the migrations' Prisma folder still declares the field mapped to `T.c`
  (`@map`, `@@map` followed), or a model mapped to `T`. Prisma Client selects every scalar field it declares, so every
  query on the model fails with P2022 once the migration runs, and the next `migrate dev` re-creates the column.
- a Drizzle `pgTable("T", { … })` (or `schema.table("T", …)`) still declares column `c` (its builder's name argument,
  or the key when there is none);
- raw SQL names both `T` and `c`, the column bare or as `"T"."c"`: a tagged `$queryRaw`/`$executeRaw`/`sql`/`Prisma.sql`
  template, the first string of `$queryRawUnsafe`/`$executeRawUnsafe`/`sql.raw`, or a TypedSQL file `prisma/sql/*.sql`
  (comment lines skipped).

The message names every place that still uses it, with path and line.

Only pending migrations are read, through the pack's library (with a Git base, the ones the base lacks; in an audit,
the newest of each folder), and in them `--` and `/* */` comments and string literals are skipped.

Leaves alone: a drop in a comment (a commented-out follow-up `-- ALTER TABLE "x" DROP COLUMN "y"`, a rollback note);
an applied migration, a down migration, a `rollback.sql` beside a `migration.sql`; a table that a view of its name
replaces (`CREATE [OR REPLACE] [MATERIALIZED] VIEW t` later in the same or a later pending migration: Prisma reads the
view as the model); a column or table already gone from the schema, every pgTable and raw SQL; a column of
the same name on another table; raw SQL that names the column without its table, qualified by another table or alias
(`"Envelope"."teamId"`, `filter."kind"`), as an output name (`AS "locationId"`, or with no AS after a literal or `)`:
`meta->>'prompt' prompt`), in a string literal or a `--` comment,
or bare in a query that also names another table declaring a column of that name (`INSERT INTO "MailSplitFilter"
("kind", …) … FROM "MailSplit"`); a column or table a later migration of the same folder, a later statement of the
same file, or the same statement brings back (`ADD [COLUMN]`, Prisma's type change `DROP COLUMN "c", ADD COLUMN "c"
<type>`, `RENAME … TO`, `CREATE TABLE`, `RENAME TO`); `ALTER COLUMN … DROP DEFAULT/NOT NULL`, `DROP CONSTRAINT`; a
`schema.prisma` outside the migrations' Prisma folder; `@ignore` fields, `@@ignore` models, `Unsupported(...)` and
relation fields.

Gaps: this is a TypeScript policy that reads the migrations as text: a drop inside a dollar-quoted function body
counts. The commoner outage, a field removed from `schema.prisma` and its column dropped in the same deploy (old
instances still select it), is out of its reach: the schema no longer declares it. Drizzle's `casing: "snake_case"` is not followed; a pgTable in any package of a monorepo counts. Raw SQL through `pg`'s
`client.query("…")`, Kysely builders and query strings built in variables or other templates is not read; a column
qualified by an alias of its own table (`u.tier`) is missed. Schema-qualified names compare by their last part. A table
renamed between the drop and the schema is not followed. Prisma 8 `ops.json` migrations are not read.

Sources:
- Prisma, Select fields (https://www.prisma.io/docs/orm/v7/prisma-client/queries/select-fields): "If you do not pass
  `select`, `include`, or `omit`, Prisma Client returns all scalar fields for the model and excludes relations from
  the result."
- Prisma error reference (https://www.prisma.io/docs/orm/v7/reference/error-reference): P2022 "The column `{column}`
  does not exist in the current database."
- Squawk, ban-drop-column (https://squawkhq.com/docs/ban-drop-column): "Dropping a table or foreign table column or a
  composite type attribute may break existing clients." / "Update your application code to no longer read or write
  the column."
- strong_migrations (https://github.com/ankane/strong_migrations#removing-a-column): "Active Record caches database
  columns at runtime, so if you drop a column, it can cause exceptions until your app reboots."

Corpus, review: audit of the 27 apps with SQL migrations, 7 findings at 5 lines, all civitai, all false: a
`"prompt"` that raw SQL names only as an output alias without AS (`i.meta->>'prompt' prompt`), `ModelHash`, now a
`/// @view` model, a `DROP TABLE` inside a `--` comment whose name list ran into the next comment lines and picked up
`"Image"`, and a commented-out follow-up dropping four `UserProfile` columns; civitai alone has 37 `--` lines naming
`DROP COLUMN`/`DROP TABLE`. Each is now a near-miss case; the rule reads only pending migrations and skips comments and
string literals. Re-run after the fixes on civitai, cal.diy, trigger.dev, documenso, shelf.nu, inbox-zero, langfuse,
papermark, comp and customermates: 0 matches (civitai 273 s, was 332 s); a drop of `polls.location` and
`polls.updated_at` added as the newest migration of a copy of rallly's Prisma folder is still reported at both
columns. Writers' run (13 repositories with Prisma or Drizzle migrations: documenso, shelf.nu, langfuse, rallly,
inbox-zero, zoonk, openstatus, comp, karakeep, useSend, formbricks, papermark, umami): 0 matches after four fixes. The first round over
eight of them gave 51 distinct matches, all false: 43 were Prisma's type change written as `DROP COLUMN "c", ADD
COLUMN "c" <new type>` in one statement (a re-add in the same statement now counts), 8 were raw SQL naming the column
as another table's, as an output name, inside a string literal or a comment (each now a near-miss case). The rule
fires when a column is dropped while still declared, which a repository whose history was applied cleanly never
shows; a drop of `polls.location` and `polls.updated_at` added to a copy of rallly's own Prisma folder (multi-file
schema, `@@map("polls")`, `@map("updated_at")`) is reported at both columns, citing `models/poll.prisma:39` and `:35`.
Kept as a real outage (Prisma P2022 on every query of the model) with 0 corpus matches; log until a wider sweep shows
its matches.
