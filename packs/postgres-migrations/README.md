# Rerail rule pack: Postgres migrations

The 10 `postgres-migrations-practice-*` rules are Rerail's own; none is a port of a linter rule. Each is advice for SQL
migrations run on PostgreSQL that the PostgreSQL docs give and that migration linters and their authors give again
(Squawk, strong_migrations, the Prisma and Drizzle issue trackers), aimed at what a Prisma or Drizzle team commits next
to a TypeScript app: `prisma/migrations/<dir>/migration.sql`, drizzle-kit's `drizzle/*.sql` and any
`**/migrations/**/*.sql`. A rule was kept when at least two independent sources gave the advice, one of them the
PostgreSQL docs or the migration tool's own docs or maintainers; when public apps held real violations of it, or the
failure it prevents is a failed deploy that blocks every later one; and when no lint rule Rerail ships makes the same
check. Every rule reads the migrations as SQL trees, except `dropped-column-still-used`, which reads `schema.prisma`,
Drizzle `pgTable` declarations and raw SQL in the app's TypeScript to find a dropped column's live uses.

Seven rules guard against a lock that stops the app: an index build, a constraint check, a `SET NOT NULL` scan, a table
rewrite (a volatile default, a type change) or any DDL queued behind a long transaction (`lock-timeout`). Two guard
against a failed deploy (`concurrently-inside-transaction`, `required-column-without-default`), which with Prisma also
blocks every later `migrate deploy` until someone resolves it by hand, and two against data loss or a broken app
(`rename-as-drop-and-add`, `dropped-column-still-used`).

## Which migrations a rule reads

Every SQL rule reads its migrations through the pack's library,
[`rules/lib/postgres-migrations.rerail`](rules/lib/postgres-migrations.rerail), so the rules agree on three things:

- **Pending.** A finding is only useful in a migration that has not run yet. With a Git base (a session or a pull
  request check, `--base`), a migration is pending when the base lacks it. Without one (a whole-repository audit),
  only the newest migration of each migrations folder counts as pending, the one a coding agent has just written:
  the older ones have run, and reporting every applied migration would list hundreds of findings on a mature
  repository that nobody can act on. A migrations folder is Prisma's (and drizzle-kit 1.0's) `<g>/<name>/migration.sql`
  (or `up.sql`), else the folder of `<g>/<name>.sql` files, ordered by name; down migrations are none, and so is any
  other `.sql` beside a `migration.sql` or `up.sql` (civitai keeps `rollback.sql` and `backfill.sql` there, run by hand
  if at all).
- **Another database.** A folder holds another database's migrations, and is skipped, when its path names one
  (`prisma/sqlite/migrations/…`, `clickhouse/migrations/…`), its Prisma `migration_lock.toml` names another provider,
  or a statement has a table `ENGINE = …` clause or MySQL's backquoted names (comments and strings aside).
- **An existing table.** A table can hold rows when the statement runs unless it was created (`CREATE TABLE`, `CREATE
  MATERIALIZED VIEW`) or emptied (`TRUNCATE`, `DELETE` without `WHERE`) earlier in the same file, or, with a Git base,
  created by a pending migration; or unless the same file first adds it a `NOT NULL` column with no default, which
  PostgreSQL runs on an empty table only. Names are folded as PostgreSQL folds them (`"Order"` and `public."Order"` are
  one table, `order` another). `required-column-without-default` and `rename-as-drop-and-add` do not take the last
  proof (that column is the first's own finding, and Prisma's rename form), and `lock-timeout` takes only creation
  (an emptied table still has others waiting on its lock).

**The `sql` pack.** Its ports of sqlfluff PG01 (`sqlfluff-pg01-postgres-excessive-locks`, an index built or dropped
without `CONCURRENTLY`) and PG02 (`sqlfluff-pg02-postgres-not-valid-foreign-key`, a foreign key without `NOT VALID`)
read every `.sql` file and fire on every Prisma init migration, where each table is created and then indexed and given
its foreign keys in the same file. `index-on-existing-table-blocks-writes` and `constraint-on-existing-table-not-valid`
report the same statements only on tables that already existed, and only in pending migrations; they also cover what
PG01 and PG02 do not (`CREATE UNIQUE INDEX`, `ADD CONSTRAINT … UNIQUE`/`PRIMARY KEY`, CHECK constraints) and give a fix
that follows the migration runner. For migration folders install this pack rather than PG01/PG02; keep those for
hand-written SQL elsewhere.

## How the rules are proven

Each rule is proven two ways: by its cases in `cases/<id>/`, where a line ending in `-- caught` must be reported and no
other line may be, run by the rule file's own test (`.test cases exactly`, which reads the case directory as one
repository; `rerail rules test <file>.rerail`); and by runs on the public apps of [the corpus](#the-corpus) with the
largest Prisma and Drizzle migration histories, whose matches were read and judged real or not. The cases carry a
one-commit `.git` (`cases/_git/`, shared) because the rules read Git; a test has no Git base, so the cases test gives
the library's `pgm_applied` no rows and reads every case migration, and two tests (`required-column-without-default`'s
and `dropped-column-still-used`'s `no_base`) show a repository audit reading only each folder's newest migration.

The numbers below come from the pack review's runs on the corpus (local optimized engine), three ways, and from the
re-runs after the review's fixes:

- **Audit**: no Git base, so only each migrations folder's newest migration is pending, as a whole-repository audit
  reads it. All 27 of the 33 corpus apps that keep SQL migrations (karakeep and openstatus keep SQLite ones only).
- **Base**: a session or pull request reads only the migrations its change adds. In clones of shelf.nu, documenso,
  karakeep, inbox-zero and cal.diy, a base commit without each folder's last 10 migrations and a head that adds them
  back: 50 pending migrations read with `--base`, including the "table created by a pending migration" exemption the
  cases cannot show.
- **Whole history**: every migration counted as pending (a copy of the library without the newest-only clause), so
  that precision is measured on hundreds of real statements: the review's runs on papermark, comp and customermates,
  and the writers' earlier runs on 6 to 13 other migration-heavy apps (named in each rule's details).

| Rule | Mode | Catches | Audit, 27 apps | Base, 50 pending | Whole history: review (3 apps) + writers | Precision of read matches |
|---|---|---|---|---|---|---|
| `postgres-migrations-practice-index-on-existing-table-blocks-writes` | Steer | `CREATE [UNIQUE] INDEX` without `CONCURRENTLY`, or `ADD UNIQUE`/`PRIMARY KEY`, on a table that already exists | 9 (4 apps) | 0 | 204 + 527 (7) | 100% (30 of 30 read by the review, one on a small table; 439 of 442 by the writers, the 3 since fixed) |
| `postgres-migrations-practice-concurrently-inside-transaction` | Steer | `CREATE`/`DROP INDEX`, `REINDEX` or `DETACH PARTITION … CONCURRENTLY` where the runner wraps the file in a transaction | 0 | 0 | 0 + 0 (10) | n/a (2 adversarial files caught; 45 read before the deploy-script gate, none real) |
| `postgres-migrations-practice-constraint-on-existing-table-not-valid` | Steer | a foreign key or CHECK added to an existing table without `NOT VALID` | 3 (1) | 5 (2) | 107 + 320 (6) | 100% (21 of 21; 320 of 320) |
| `postgres-migrations-practice-set-not-null-scans-table` | Steer | `SET NOT NULL` on an existing table with no validated `CHECK (c IS NOT NULL)` | 0 | 2 statements (1; 4 findings before one per statement) | 6 + 135 (6) | 100% (10 of 10; 135 of 135) |
| `postgres-migrations-practice-required-column-without-default` | Steer | `ADD COLUMN … NOT NULL` with no default on an existing table | 0 | 0 | 10 + 96 (7) | 100% (10 of 10; 96 of 96) |
| `postgres-migrations-practice-volatile-default-rewrites` | Steer | `ADD COLUMN` with a volatile default, a serial type, an identity or a stored generated expression on an existing table | 0 | 0 | 3 + 56 (5) | 100% (3 of 3; 56 of 56) |
| `postgres-migrations-practice-column-type-change-rewrites` | Steer | a column type change that rewrites an existing table | 1 (1) | 4 (1) | 20 + 134 (8) | 100% (20 of 20 after `TIMESTAMPTZ` → `TIMESTAMP` joined its reverse as left alone; 132 of 134) |
| `postgres-migrations-practice-lock-timeout` | Log | a migration that locks an existing table with no `lock_timeout` set | 12 (11) | 29 of the 50 migrations | 440 (3) | 100% (14 of 14) |
| `postgres-migrations-practice-rename-as-drop-and-add` | Log | a `DROP COLUMN` plus an `ADD COLUMN` of the same type in one migration: a rename that deletes the data | 0 | 0 | 4 + 67 (10) | 50% (2 of 4); 82% (55 of 67) |
| `postgres-migrations-practice-dropped-column-still-used` | Log | a column or table dropped while `schema.prisma`, a Drizzle `pgTable` or raw SQL still uses it | 0 in 10 apps (7 before the review's fixes, all civitai, all false) | — | 0 (13) | n/a (7 of 7 false before the fixes; a drop added to a copy of rallly is reported) |

A match is real when the statement does what the rule names on a table that existed before the migration, even if the
table is small; the messages say when to keep the statement (a table known to stay small, or one still empty because
the migration that creates it has not been deployed). A rule steers where nearly every read match was real;
`lock-timeout` logs because a timeout set on the database role outside the repository cannot be seen and it fires on 60
to 90% of the migrations of any repository that sets none, `rename-as-drop-and-add` because a third of its matches are
deliberate replacements an agent could wrongly turn into a RENAME, and `dropped-column-still-used` because the corpus
shows none of its matches (a repository whose history applied cleanly never drops a column it still declares).

## What an audit reports

A whole-repository audit has no Git base, so it reads only the newest migration of each migrations folder: the one a
coding agent has just written, and the one still worth changing. On the three corpus repositories with the longest
histories (local optimized engine, one run per rule and repository), the pack's matches against the writers'
whole-history matches on the same repositories (— where the writer did not run that repository):

| Rule | langfuse (500 migrations) | trigger.dev (894) | documenso (170) |
|---|--:|--:|--:|
| `index-on-existing-table-blocks-writes` | 1 (was 93) | 0 (—) | 0 (was 73) |
| `concurrently-inside-transaction` | 0 (was 0) | 0 (was 0) | 0 (was 0) |
| `constraint-on-existing-table-not-valid` | 0 (was 41) | 0 (was 65) | 0 (was 52) |
| `set-not-null-scans-table` | 0 (was 27) | 0 (was 16) | 0 (was 36) |
| `required-column-without-default` | 0 (was 9) | 0 (—) | 0 (was 9) |
| `volatile-default-rewrites` | 0 (was 0) | 0 (was 0) | 0 (was 0) |
| `column-type-change-rewrites` | 0 (was 10) | 0 (was 17) | 0 (was 9) |
| `lock-timeout` | 1 (was 1) | 2 (was 1) | 0 (was 0) |
| `rename-as-drop-and-add` | 0 (was 11) | 0 (was 24) | 0 (was 4) |
| `dropped-column-still-used` | 0 (was 0) | 0 (was 0) | 0 (was 0) |

On those three, 4 matches where whole-history runs gave 498, all 4 real: langfuse's newest migration
(`20261120140000_add_gateway_spend_limit_warning_thresholds`) rebuilds a unique index on `gateway_spend_limit_alerts`,
a table an earlier migration created, without `CONCURRENTLY` (line 10), and alters `gateway_spend_limits` with no
`lock_timeout` (line 3); trigger.dev's `run-ops-database` folder adds a column to `TaskRun` with no `lock_timeout`, as
does its `dashboard-agent-db` Drizzle folder to `chats` (a folder newer than the writer's run). The other newest
migrations lock nothing the rules name: trigger.dev's main folder ends with a `CREATE INDEX CONCURRENTLY` alone in its
migration, documenso's with an `UPDATE`.

Over all 27 corpus apps with SQL migrations (the review's audit), the nine SQL rules report 25 findings, all real (one
on a small table, airbroke's `accounts`): `index-on-existing-table-blocks-writes` 9 in 4 apps (papermark's
`20260820000000_add_multiple_team_brands` alone gives 5 index and 3 foreign-key findings),
`constraint-on-existing-table-not-valid` 3, `column-type-change-rewrites` 1 (cal.diy's enum recreation on `Webhook`)
and `lock-timeout` 12 in 11 apps. `dropped-column-still-used`, which before the review read every migration as raw text, gave 7 findings on
civitai, all false (commented-out drops, a `DROP TABLE` name list that ran into the next comment, a `/// @view` model,
an output alias without AS); it now reads only pending migrations and skips comments, and gives 0 on civitai and on
the nine other apps re-run.

## Starting values

All 10 rules start on, in the mode the table gives them: 7 steer and 3 log. To turn one off or change its mode, use
its switch in the Postgres migrations pack's page in the dashboard, or `repository_settings_set`
(`PUT /api/repository/settings`) for a repository or a checkout ([rule scoping](../../docs/rule-scoping.md#management-surfaces)).

The rules are written in `.rerail`, so they run apart from JSON rules: in the background, once the coding agent's writes
have settled, and in audits and `rerail scan`. The SQL rules read only migration files (and `migration_lock.toml`,
`package.json` for `concurrently-inside-transaction`, and the configuration files `lock-timeout` searches for a
timeout, which `concurrently-inside-transaction` also searches for `prisma migrate deploy`); `dropped-column-still-used`
also reads the app's TypeScript and `*.prisma` files. On the largest histories in the corpus one SQL rule's run took 2
to 17 seconds on a quiet machine (langfuse 500 migrations, trigger.dev 894, documenso 170) and 25 to 56 seconds on
civitai (1,218 migration files) on a loaded one; `dropped-column-still-used` took 18 to 47 seconds, and 273 seconds on
civitai (332 before it read only pending migrations).

## Rule details

What each rule matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources. Every
SQL rule reads `**/migrations/**/*.sql` and `**/drizzle/**/*.sql` outside test, fixture and build folders, only in
pending PostgreSQL migrations (see [above](#which-migrations-a-rule-reads)); the lists below do not repeat that.

### `postgres-migrations-practice-index-on-existing-table-blocks-writes`

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
  PRIMARY KEY`, Prisma's primary-key swap): the rewrite holds the lock anyway, and the column-type-change rule owns it.

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
  Prisma 7.4 or later and some `package.json` script, Dockerfile, shell script, compose file or CI workflow runs
  `prisma migrate deploy` or `migrate dev`: Prisma before 7.4 sends the file as one batch, which PostgreSQL runs as one
  transaction;
- in a Drizzle migration (a `.sql` file beside `meta/_journal.json`, or drizzle-kit 1.0's `<ts>_x/migration.sql`
  beside a `snapshot.json`): drizzle-kit `migrate` and drizzle-orm's `migrate()` run all pending migrations in one
  transaction.

Leaves alone:
- the statement alone in its Prisma migration (comments do not count);
- a Prisma migration of a package whose `prisma` range is entirely `>=7.4.0` (Prisma 7.4 sends each statement on its
  own), unless the statement sits in `BEGIN … COMMIT`;
- a Prisma migration in a repository where no `package.json` script, Dockerfile, shell script, compose file or CI
  workflow runs `migrate deploy`/`migrate dev`: the team applies migrations another way (civitai's CONTRIBUTING.md:
  "We do not use `prisma migrate deploy`. Migrations are applied by hand, per environment."; its JS helper around
  `migrate dev` for local schema work does not count), unless the statement sits in `BEGIN … COMMIT`;
- `REFRESH MATERIALIZED VIEW CONCURRENTLY`, which PostgreSQL allows inside a transaction;
- `.sql` migrations of other runners (no Prisma layout, no Drizzle journal) unless in `BEGIN … COMMIT`.

Gaps: a Drizzle project applying its `.sql` files with its own non-transactional script is reported. A Prisma project
that runs `prisma migrate deploy` only from a JS/TS script, or from a file more than four folders deep, is not
reported. A Prisma version written as `catalog:`, `workspace:` or a
dist-tag is read as before 7.4. Prisma 7.5's shadow database was reported to still reject a mixed file on `migrate
dev`; 7.4+ files are not reported. Formbricks' `packages/database/migration/` folder is outside the pack's
`**/migrations/**` scope. Prisma 8 `ops.json` migrations and other runners' wrapping (node-pg-migrate, Supabase CLI,
golang-migrate) are not read.

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

### `postgres-migrations-practice-constraint-on-existing-table-not-valid`

Matches `ALTER TABLE t ADD [CONSTRAINT c] FOREIGN KEY …` and `… CHECK (…)` without `NOT VALID` when `t` may already
hold rows. Such a statement checks every existing row before it commits, holding SHARE ROW EXCLUSIVE on `t` and on the
referenced table (a foreign key: writes to both wait) or ACCESS EXCLUSIVE on `t` (a CHECK: reads wait too). The
message steers to `NOT VALID` now and `VALIDATE CONSTRAINT` in a later migration file (in the same file the
validation runs in the same transaction and keeps the lock): with Prisma the next file is enough, even in the same
deploy, since each file runs on its own; with Drizzle, which runs every pending migration in one transaction, it has
to ship in a later deploy. It reports a
foreign key (ForeignKeyScan) and a CHECK (CheckScan) separately, at the ALTER statement. A constraint re-added after
`DROP CONSTRAINT` (Prisma's form for an `onDelete` change) is reported: it re-checks every row. So is a constraint on a
column the same migration has just added as nullable or with a default: the check still reads the whole table.

Leaves alone: `NOT VALID` constraints and `VALIDATE CONSTRAINT`; a table that is not an existing one (Prisma's and
drizzle-kit's `CREATE TABLE` followed by its foreign keys, pg_dump's `ALTER TABLE ONLY` after its `CREATE TABLE`, a
table first given a `NOT NULL` column without a default: Prisma's "This is not possible if the table is not empty"
warning); `UNIQUE`, `PRIMARY KEY` and `EXCLUDE` constraints (index builds: `index-on-existing-table-blocks-writes`);
statements inside an ERROR node.

Gaps: without a Git base a table created by an earlier migration of the same change counts as existing. A column-level
`ADD COLUMN x int REFERENCES t` is not reported. A table known to stay small cannot be told from a large one; the
message says to leave it then. On PostgreSQL 17 and older a partitioned table refuses `NOT VALID` foreign keys; the
message says so. Formbricks keeps its migrations in `packages/database/migration/` (singular), outside the pack's
match, so its unguarded foreign keys are not read.

Overlap: sqlfluff PG02 (`packs/sql`) reports every foreign key without `NOT VALID` in any `.sql` file, including every
Prisma init migration, and never a CHECK ([above](#which-migrations-a-rule-reads)).

Sources:
- PostgreSQL, [ALTER TABLE](https://www.postgresql.org/docs/current/sql-altertable.html): "Normally, this form will
  cause a scan of the table to verify that all existing rows in the table satisfy the new constraint. But if the NOT
  VALID option is used, this potentially-lengthy scan is skipped."; "Although most forms of ADD table_constraint
  require an ACCESS EXCLUSIVE lock, ADD FOREIGN KEY requires only a SHARE ROW EXCLUSIVE lock. Note that ADD FOREIGN
  KEY also acquires a SHARE ROW EXCLUSIVE lock on the referenced table"; "Scanning a large table to verify new
  foreign-key, check, or not-null constraints can take a long time, and other updates to the table are locked out
  until the ALTER TABLE ADD CONSTRAINT command is committed."
- Squawk, [adding-foreign-key-constraint](https://squawkhq.com/docs/adding-foreign-key-constraint): "add the
  constraint as `NOT VALID` in one transaction, then `VALIDATE CONSTRAINT` in another."; "Since the newly created table
  has no rows, you don't need to add a foreign key with `NOT VALID`."
- strong_migrations, [Adding a check constraint](https://github.com/ankane/strong_migrations#adding-a-check-constraint):
  "Adding a check constraint blocks reads and writes in Postgres … while every row is checked."
- PostgreSQL commit [Allow NOT VALID foreign key constraints on partitioned tables](https://www.postgresql.org/message-id/E1taygE-0036kE-Fh%40gemulon.postgresql.org)
  (PostgreSQL 18): "This feature was intentionally omitted when FKs were first implemented for partitioned tables".

Corpus, review: audit of the 27 apps with SQL migrations, 3 in 1 (papermark's foreign keys from `Team`, `Link`,
`Dataroom` to `Brand`); base, 5 in 2 (shelf.nu's CHECKs on `CustomField` and `AuditAsset`, documenso's foreign keys
from the existing `SubscriptionClaim`/`OrganisationClaim`); whole history of comp, papermark and customermates, 107
(53, 29, 25), 13 read; all 21 read real. Two of comp's are on tables created minutes to hours earlier, almost surely
the same deploy: a Git base exempts that, an audit cannot. Writers' run (documenso, trigger.dev, langfuse, formbricks,
shelf.nu, papermark, umami, inbox-zero), whole history: 320 matches in 6 of 8 (documenso 52, trigger.dev 65, langfuse 41, shelf.nu 87, papermark 29, inbox-zero 46; formbricks and
umami 0). Every match was checked against its statement, its table and the earlier migration that created (or renamed
into) that table; about 25 were read in full. All 320 are real: a foreign key or CHECK added without `NOT VALID` to a
table an earlier migration created, many on large tables (trigger.dev `TaskRun`, langfuse `observations`/`traces`,
documenso `Document`/`Field`). trigger.dev and formbricks later began writing `NOT VALID` by hand (trigger.dev
`20260810130446_add_cron_spread_fields`, formbricks `20261027120000_allow_response_outcome_deletion_cleanup`). The
required-column proof of an empty table came from trigger.dev
`20250319110754_add_org_and_project_to_execution_snapshots`, which adds `NOT NULL` columns without a default and then
their foreign keys (the table must be empty there).

### `postgres-migrations-practice-set-not-null-scans-table`

`ALTER TABLE t ALTER COLUMN c SET NOT NULL` takes an ACCESS EXCLUSIVE lock on `t` (reads and writes wait) and scans
every row to prove no `c` is NULL, unless a valid `CHECK (c IS NOT NULL)` already proves it; the migration also fails
if a NULL is left. Prisma writes exactly this statement whenever a field goes from `String?` to `String`. The rule
steers to the three-step pattern: `ADD CONSTRAINT … CHECK (c IS NOT NULL) NOT VALID`, then `VALIDATE CONSTRAINT` in a
later, separately deployed migration (SHARE UPDATE EXCLUSIVE: reads and writes go on), then `SET NOT NULL`, which skips
the scan.

Catches, once per `ALTER TABLE` statement, naming every column it sets NOT NULL:
- `ALTER TABLE "User" ALTER COLUMN "phone" SET NOT NULL;` on an existing table;
- a multi-action statement (`ALTER COLUMN "a" SET NOT NULL, ALTER COLUMN "b" SET NOT NULL`): one finding;
- drizzle and hand-written SQL: unquoted names, `ALTER c SET NOT NULL` without `COLUMN`;
- a `SET NOT NULL` whose only `CHECK (c IS NOT NULL)` is `NOT VALID` and never validated.

When every column the statement sets NOT NULL was added earlier in the same file (add nullable, backfill with
`UPDATE`, `SET NOT NULL`: the shape `required-column-without-default` itself suggests, and 6 of the 10 matches the
review read), the three-step CHECK recipe is the wrong lever: the backfill already rewrote every row. The message then
leads with `ADD COLUMN … NOT NULL DEFAULT <constant>` (catalog-only since PostgreSQL 11) when the backfill is a
constant, and otherwise with the NOT NULL in a later migration.

Leaves alone: a table that is not an existing one (Prisma's and drizzle's init migrations); a column some migration
proves non-null with `CHECK (c IS NOT NULL)` on that table, valid or later validated; `DROP NOT NULL`, `SET DEFAULT` and
the other `ALTER COLUMN` actions; statements inside a function body or DO block.

Gaps: table size is unknown, so a small lookup table is reported (the message says to keep the statement then).
Without a Git base a table created in an earlier migration of the same pending change counts as existing. The CHECK
is matched in any migration, not only an earlier one, and only in its exact form `CHECK (c IS NOT NULL)`; a CHECK
inside `CREATE TABLE` is not read. PostgreSQL 18's `ADD CONSTRAINT … NOT NULL c NOT VALID` + `VALIDATE CONSTRAINT` is
not recognised. A CHECK validated in the same transaction as the `SET NOT NULL` is accepted, though it holds the lock.
Prisma 8 `ops.json` (`setNotNull`) is not read.

Sources:
- PostgreSQL, ALTER TABLE (https://www.postgresql.org/docs/current/sql-altertable.html): "SET NOT NULL may only be
  applied to a column provided none of the records in the table contain a NULL value for the column. Ordinarily this
  is checked during the ALTER TABLE by scanning the entire table, unless NOT VALID is specified; however, if a valid
  CHECK constraint exists (and is not dropped in the same command) which proves no NULL can exist, then the table scan
  is skipped." / "An ACCESS EXCLUSIVE lock is acquired unless explicitly noted."
- strong_migrations (https://github.com/ankane/strong_migrations#setting-not-null-on-an-existing-column): "In Postgres,
  setting `NOT NULL` on an existing column blocks reads and writes while every row is checked."
- Squawk, adding-not-nullable-field (https://squawkhq.com/docs/adding-not-nullable-field): "`ALTER TABLE` also requires
  an `ACCESS EXCLUSIVE` lock which will disable reads and writes while this statement is running."
- prisma-engines, `unexecutable_step_check.rs`
  (https://github.com/prisma/prisma-engines/blob/main/schema-engine/connectors/sql-schema-connector/src/sql_destructive_change_checker/unexecutable_step_check.rs):
  "Made the column `{column}` on table `{table}` required. This step will fail if there are existing NULL values in
  that column."

Corpus, review: 0 in the audit of the 27 apps with SQL migrations; base, documenso
`20260922002213_add_envelope_contents` (2 statements, 4 findings before one finding per statement); whole history of
papermark (3) and customermates (3), comp 0; all real scans under ACCESS EXCLUSIVE. 6 of the 10 statements read
(documenso's two, customermates' `channelClass` and `slug`) are "add nullable, backfill, SET NOT NULL" in one file,
which now get the constant-DEFAULT text; the re-run gives the same 6 whole-history statements, those four with it.
Writers' run (the six repositories with the most migration SQL that use `SET NOT NULL`), whole history: 135 matches in 6
(trigger.dev 16, langfuse 27, shelf.nu 17, documenso 36, rallly 19, inbox-zero 20), all 135 checked, 135 real. None of
the six uses the CHECK … NOT VALID pattern; langfuse once left the statement commented out "To be applied in separate
migration after application release to minimize ingestion downtime" (`20230809132331_add_project_id_to_observations`),
the concern this rule steers on.

### `postgres-migrations-practice-required-column-without-default`

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

### `postgres-migrations-practice-lock-timeout`

A DDL statement that needs a strong lock on a table waits in the lock queue behind any open transaction that touched
the table, and every later read and write on that table queues behind the DDL: a millisecond `ALTER TABLE` behind one
slow query or VACUUM stops the table for as long as that query runs. `SET lock_timeout` makes the migration fail fast
instead; PostgreSQL's default is 0 (no timeout). Squawk's `require-lock-timeout` and strong_migrations' lock timeout
give the same advice. One finding per migration, at its first statement that locks an existing table before any `SET
lock_timeout`. What to do after a timeout follows the runner: a Drizzle run rolls back and is deployed again; Prisma
records the migration as failed and refuses every later deploy (P3009) until `prisma migrate resolve --rolled-back
<name>`, and from 7.4 (statements sent one by one) the message also asks for `BEGIN … COMMIT` around the statements so
none is left applied.

Catches:
- `ALTER TABLE "User" …` (any action but VALIDATE CONSTRAINT) on a table the migration did not create earlier;
- a foreign key, in `CREATE TABLE … REFERENCES "User"` or `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY … REFERENCES
  "User"`, even from a brand-new table: it locks `"User"` (SHARE ROW EXCLUSIVE);
- `CREATE [UNIQUE] INDEX` without CONCURRENTLY on an existing table (SHARE: queues writers);
- `DROP TABLE`, `CREATE TRIGGER … ON` an existing table;
- with `SET lock_timeout = 0` before the statement (it turns the timeout off), or a `SET lock_timeout` placed after it.

Leaves alone:
- `SET [LOCAL|SESSION] lock_timeout = '5s'` or `SELECT set_config('lock_timeout', '5s', …)` before the statement;
- tables created earlier in the same migration, or by a migration missing from the Git base (nobody else holds a lock
  on them; a table only emptied, or given a required column, still has); Prisma's init migration and any migration
  whose foreign keys only point at tables it creates;
- `CREATE INDEX CONCURRENTLY`, `ALTER TABLE … VALIDATE CONSTRAINT` (neither blocks reads or writes);
- statements in a function body or DO block;
- every migration when the repository sets the timeout elsewhere: a migration with `ALTER ROLE|USER|DATABASE … SET
  lock_timeout`, or `lock_timeout` on a line of package.json, `.env*`, `*.config.{ts,js,mjs,cjs}` (prisma.config.ts,
  drizzle.config.ts), `.github/workflows/*.yml`, `*.sh`, Dockerfiles, compose files, or a `*migrat*.{ts,js}` runner
  script (four directory levels deep, past node_modules, tests and build output).

Gaps: a timeout set on the database role outside the repository (managed Postgres consoles, infrastructure repos) is
unknowable; the rule logs for that reason. `SET LOCAL` outside a transaction block is accepted, though under Prisma
>= 7.4 (statements run one by one) it has no effect. `DROP INDEX` without CONCURRENTLY locks its table, which the
statement does not name; not counted. Without a Git base, names order bytewise (`10_x` before `9_x`). A runner that
sets the timeout in a TypeScript file outside the listed names, or deeper than four levels, is not read.

Sources:
- GoCardless, Zero-downtime Postgres migrations: the hard parts
  (https://gocardless.com/blog/zero-downtime-postgres-migrations-the-hard-parts/): "As `AccessExclusive` locks
  conflict with every other type of lock, having one sat in the queue blocks all other operations on that table." /
  "Set `lock_timeout` in your migration scripts to a pause your app can tolerate. It's better to abort a deploy than
  take your application down."
- Squawk, safe migrations (https://squawkhq.com/docs/safe_migrations): "You must configure a `lock_timeout` within
  Postgres while running your migrations. Without this your migration could block other writes while trying to grab a
  lock on the table."
- strong_migrations (https://github.com/ankane/strong_migrations#lock-timeout): "It’s extremely important to set a
  short lock timeout for migrations. This way, if a migration can’t acquire a lock in a timely manner, other
  statements won’t be stuck behind it."
- PostgreSQL, lock_timeout (https://www.postgresql.org/docs/current/runtime-config-client.html#GUC-LOCK-TIMEOUT):
  "Abort any statement that waits longer than the specified amount of time while attempting to acquire a lock on a
  table, index, row, or other database object." / "A value of zero (the default) disables the timeout."
- Prisma, pgfence guide (https://www.prisma.io/docs/guides/integrations/pgfence): "Missing `lock_timeout` settings
  (risk of lock queue death spirals)".
- In the corpus, trigger.dev's own migrations start with "-- Fail fast instead of queueing behind a long txn/VACUUM for
  the ACCESS EXCLUSIVE lock." / "SET lock_timeout = '5s';"
  (`internal-packages/database/prisma/migrations/20260626140000_drop_bulk_action_item_group_fk/migration.sql`).

Corpus, review: audit of the 27 apps with SQL migrations, 12 in 11 (dossier, cal.diy, crawlseo, inbox-zero,
airbroke, langfuse, rallly, openship, papermark, customermates, trigger.dev ×2), every one an ALTER, foreign key or
index on an existing table with no lock_timeout anywhere the rule looks: real (civitai, which sets it, 0); base, 29 of
the 50 pending migrations (shelf.nu 9 of 10, documenso 7, inbox-zero 7, cal.diy 6, karakeep 0: SQLite); whole history
of comp 250 of 292 migrations, papermark 128 of 133, customermates 62. A statement that locks two tables (a foreign key
from an existing table to another) gives two findings at one line. The adversarial `rollback.sql` is no longer read
(re-run on civitai: 0). Writers' run (8 repositories with the most Prisma migration SQL, newest migration of each
folder): 4 matches in 4, all 4 read, all real. Two of the matched repositories already write `SET [LOCAL] lock_timeout = '5s'` in some of their own
migrations, with comments naming the lock queue, and the matched migration is one that left it out (trigger.dev's
`run-ops-database` folder; langfuse's `20261120140000_add_gateway_spend_limit_warning_thresholds`). shelf.nu,
documenso, comp and zoonk: 0 (their newest migrations are an UPDATE backfill or `ALTER TYPE … ADD VALUE`, which lock
no table). Log: a timeout set on the role outside the repository cannot be seen, and four matches is a small sample.

### `postgres-migrations-practice-rename-as-drop-and-add`

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
  names compare case-folded without underscores, so `updatedAt` → `updated_at` pairs); with no name alike, only a
  one-to-one match counts (five TEXT columns dropped and one added is a restructure);
- an id replacing a name or the reverse (`queueName` → `queueId`, `versionId` → `version`);
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
an applied migration, a down migration, a `rollback.sql` beside a `migration.sql`; a model documented `/// @view` (a
view that replaced the table); a column or table already gone from the schema, every pgTable and raw SQL; a column of
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

## The corpus

The open-source apps the rules were run on, cloned shallow at these commits (the default branch's HEAD on 2026-11-24
or just before). Each rule's details name the ones it ran on.

| Repository | Commit | Licence |
|---|---|---|
| [AienTech/dossier](https://github.com/AienTech/dossier) | `f0f70f51fcd717604348bc5181bff07d5e80cf80` | AGPL-3.0 |
| [calcom/cal.diy](https://github.com/calcom/cal.diy) | `ae027bb70e694e2d2c44569f398d6cbcee0389b1` | MIT |
| [civitai/civitai](https://github.com/civitai/civitai) | `02be75c24d80f2a000d7a3a165b3ba04a1d7fcce` | Apache-2.0 |
| [customermates/customermates](https://github.com/customermates/customermates) | `fdc9bfd0626d3f6685950e02895b7925a4dd712c` | Sustainable Use License |
| [documenso/documenso](https://github.com/documenso/documenso) | `e745c9ec339f1b16d2f8d181b6457323fb33ebe5` | AGPL-3.0 |
| [elie222/inbox-zero](https://github.com/elie222/inbox-zero) | `cc2c4c1871883b1513392a50b70cce28adcb0234` | AGPL-3.0 |
| [formbricks/formbricks](https://github.com/formbricks/formbricks) | `eae381e3372eeea70d7f5e2a2349cb195f18645e` | AGPL-3.0 (ee/ commercial) |
| [karakeep-app/karakeep](https://github.com/karakeep-app/karakeep) | `28ae67526755479b13a91ac89d34eb711638841b` | AGPL-3.0 |
| [KunMoe/kun-touchgal-next](https://github.com/KunMoe/kun-touchgal-next) | `5a24d22fcfbd21006bb72f14503a5ed946772aa6` | AGPL-3.0 |
| [langfuse/langfuse](https://github.com/langfuse/langfuse) | `ce1049696d188e1e6cfcca104c26f95be7751df6` | MIT (ee/ commercial) |
| [lukevella/rallly](https://github.com/lukevella/rallly) | `0a7558f97c0250d79e0bcbbea9481f41d7b7e3b7` | AGPL-3.0 |
| [openstatusHQ/openstatus](https://github.com/openstatusHQ/openstatus) | `51a822354a831597496a7c11b4e9ff232f056cc1` | AGPL-3.0 |
| [papermark/papermark](https://github.com/papermark/papermark) | `ed19717ec02a1ac79aecf5569159aa9d2d869312` | AGPL-3.0 (ee/ commercial) |
| [Shelf-nu/shelf.nu](https://github.com/Shelf-nu/shelf.nu) | `36fefc023fed32d28b3da7c4280c57cf2de5f0d9` | AGPL-3.0 |
| [triggerdotdev/trigger.dev](https://github.com/triggerdotdev/trigger.dev) | `59f5aebe8d857359a3cc5cea240346b0a98258e5` | Apache-2.0 |
| [trycompai/comp](https://github.com/trycompai/comp) | `712ee1b3d722ca1b4f09eb4e5b67cd3bde8713ed` | AGPL-3.0 |
| [umami-software/umami](https://github.com/umami-software/umami) | `dbb8d606fbd4e99220fb83bcb4abad3e0f97f42a` | MIT |
| [usesend/useSend](https://github.com/usesend/useSend) | `10d0134c738cf7ad8615319ecdb19bc38086e42f` | AGPL-3.0 |
| [zoonk/zoonk](https://github.com/zoonk/zoonk) | `c3e83e723314caff586ca3ad4d75d0e37c8e5348` | MIT |

## Files

| Path | What it is |
|---|---|
| `rules/postgres-migrations-practice-<id>.rerail` | The 10 rules, written by hand. Each opens with a comment on what it matches and leaves alone, with an example of each |
| [`rules/lib/postgres-migrations.rerail`](rules/lib/postgres-migrations.rerail) | `lib/postgres-migrations`, the shared gating: migrations, their order and which are pending, other databases' folders, runners, tables and whether they may hold rows, column helpers |
| `cases/<id>/` | Each rule's cases, a small repository: a line ending in `-- caught` must be reported, no other line may be (the rule file's `.test cases`). `cases/_git/` is the one-commit `.git` every case repository uses. Not published |
| [`NOTES-postgres-migrations.md`](NOTES-postgres-migrations.md) | The batch notes: dropped candidates, the library's history and open proposals, and every corpus match read with its verdict |
| [`ENGINE-GAPS.md`](ENGINE-GAPS.md) | Engine gaps found while writing and measuring these rules |
