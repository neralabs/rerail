# Rerail rule pack: Postgres migrations

The `postgres-migrations-practice-*` rules are Rerail's own; none is a port of a linter rule. Each is advice for SQL
migrations run on PostgreSQL that the PostgreSQL docs give and that migration linters and their authors give again
(Squawk, strong_migrations, the Prisma and Drizzle issue trackers), aimed at what a Prisma or Drizzle team commits next
to a TypeScript app: `prisma/migrations/<dir>/migration.sql`, drizzle-kit's `drizzle/*.sql` and any
`**/migrations/**/*.sql`. A rule was kept when at least two independent sources gave the advice, one of them the
PostgreSQL docs or the migration tool's own docs or maintainers; when public apps held real violations of it; and when
no lint rule Rerail ships makes the same check. Every rule reads the migrations as SQL trees.

The pack ships four rules, each against a lock that stops the app, and all four start on: `set-not-null-scans-table`,
a `SET NOT NULL` that scans an existing table under an ACCESS EXCLUSIVE lock; `constraint-on-existing-table-not-valid`,
a foreign key or CHECK added to an existing table without `NOT VALID`; `index-on-existing-table-blocks-writes`, an
index built on an existing table without `CONCURRENTLY` (or a UNIQUE or PRIMARY KEY constraint that builds one); and
`lock-timeout`, any DDL that can queue behind a long transaction with no `lock_timeout` set. Each ships on because more
than 9 in 10 of its matches on apps it was not tuned or fixed on were real ([How the rules were
measured](#how-the-rules-were-measured)).

## Which migrations a rule reads

Every SQL rule reads its migrations through the pack's library,
[`rules/lib/postgres-migrations.rerail`](rules/lib/postgres-migrations.rerail), so the rules agree on three things:

- **Pending.** A finding is only useful in a migration that has not run yet. With a Git base (a session or a pull
  request check, `--base`), a migration is pending when the base lacks it. Without one (a whole-repository audit),
  only the newest migration of each migrations folder counts as pending, the one a coding agent has just written:
  the older ones have run, and reporting every applied migration would list hundreds of findings on a mature
  repository that nobody can act on. A migrations folder is Prisma's (and drizzle-kit 1.0's) `<g>/<name>/migration.sql`
  (or `up.sql`), ordered by folder name as Prisma sorts them; else the folder of `<g>/<name>.sql` files, ordered by
  version: the leading number of the name (after a Flyway `V`, leading zeros dropped), fewer digits first, then the
  bytes of the name, so `9_x.sql` runs before `10_x.sql`. Down migrations are none, and so is any other `.sql` beside a `migration.sql` or
  `up.sql` (a `rollback.sql` or `backfill.sql` kept there, run by hand if at all).
- **PostgreSQL only.** Every rule reads a migrations folder only when it holds PostgreSQL migrations. The ORM's own
  declaration decides first: the folder's Prisma `migration_lock.toml` (`provider = "postgresql"`), the datasource
  `provider` of the Prisma schema beside the folder (`prisma/*.prisma` or `prisma/schema/**/*.prisma` for
  `prisma/migrations/`; a schema deeper, such as `prisma/postgresql/schema.prisma`, declares only its own
  `prisma/postgresql/migrations/`), or the `dialect` (drizzle-kit 0.20's `driver`) of a Drizzle config
  (`drizzle.config.ts`, found by the checkout walk) whose `out` folder (default `drizzle`) is the folder. A folder
  declared PostgreSQL or CockroachDB is read; one declared MySQL, SQLite/libSQL/Turso/D1, SQL Server, SingleStore or
  MongoDB is skipped. With no PostgreSQL declaration, a folder is also skipped when its path names another database
  (`prisma/sqlite/migrations/…`, `clickhouse/migrations/…`) or a statement has a table `ENGINE = …` clause or
  backquoted names (MySQL, and SQLite as drizzle-kit writes it; comments and strings aside). A folder nothing
  declares (plain SQL, Supabase, dbmate) is read as PostgreSQL unless one of those guesses says otherwise.
- **An existing table.** A table can hold rows when the statement runs unless it was created (`CREATE TABLE`, `CREATE
  MATERIALIZED VIEW`) or emptied (`TRUNCATE`, `DELETE` without `WHERE`) earlier in the same file, or, with a Git base,
  created by a pending migration of the same folder; or unless the same file first adds it a `NOT NULL` column with no
  default, which PostgreSQL runs on an empty table only. Names are folded as PostgreSQL folds them (`"Order"` and
  `public."Order"` are one table, `order` another). `lock-timeout` takes only creation (an emptied table still has
  others waiting on its lock).

**The `sql` pack.** Its ports of sqlfluff PG01 (`sqlfluff-pg01-postgres-excessive-locks`, an index built or dropped
without `CONCURRENTLY`) and PG02 (`sqlfluff-pg02-postgres-not-valid-foreign-key`, a foreign key without `NOT VALID`)
read every `.sql` file and fire on every Prisma init migration, where each table is created and then indexed and given
its foreign keys in the same file. `index-on-existing-table-blocks-writes` and `constraint-on-existing-table-not-valid`
report the same statements only on tables that already existed, and only in pending migrations; they also cover what
PG01 and PG02 do not (`CREATE UNIQUE INDEX`, `ADD CONSTRAINT … UNIQUE`/`PRIMARY KEY`, CHECK constraints) and give a fix
that follows the migration runner. For migration folders install this pack rather than PG01/PG02; keep those for
hand-written SQL elsewhere. This pack does not report a `DROP INDEX` without `CONCURRENTLY`, which PG01 does.

## How the rules were measured

Each rule is proven by its cases in `cases/<id>/`, where a line ending in `-- caught` must be reported and no other line
may be, run by the rule file's own test (`.test cases exactly`, which reads the case directory as one repository;
`rerail rules test <file>.rerail`). The cases carry a one-commit `.git` (`cases/_git/`, shared) because the rules read
Git; a test has no Git base, so the cases test gives the library's `pgm_applied` no rows and reads every case
migration. Its precision was then measured in steps, each on apps the steps before had not seen:

1. **Tuned on the corpus.** The rules were written and fixed against runs on the public apps of [the
   corpus](#the-corpus) with the largest Prisma and Drizzle migration histories, whose matches were read and judged
   real or not, three ways (the pack review's runs, local optimized engine, and the re-runs after its fixes):
   - **Audit**: no Git base, so only each migrations folder's newest migration is pending, as a whole-repository
     audit reads it. All 27 of the 33 corpus apps that keep SQL migrations (karakeep and openstatus keep SQLite ones
     only).
   - **Base**: a session or pull request reads only the migrations its change adds. In clones of shelf.nu,
     documenso, karakeep, inbox-zero and cal.diy, a base commit without each folder's last 10 migrations and a head
     that adds them back: 50 pending migrations read with `--base`, including the "table created by a pending
     migration" exemption the cases cannot show.
   - **Whole history**: every migration counted as pending (a copy of the library without the newest-only clause),
     so that precision is measured on hundreds of real statements: the review's runs on papermark, comp and
     customermates, and the writers' earlier runs on 6 to 8 other migration-heavy apps (named in each rule's details).
2. **Generalized against 8 held-out apps** never in the corpus (teableio/teable, Openpanel-dev/openpanel,
   haydenbleasel/next-forge, nextjs/saas-starter, dubinc/dub, jitsucom/jitsu, Dokploy/dokploy, midday-ai/midday), one
   run per app with no Git base. An overfit audit then replaced every patch fitted to one corpus app with a
   structural rule (the database a folder declares, version order, explicit `LOCK TABLE`, a timeout set in the
   migrations' own package), each with a case ([NOTES](NOTES-postgres-migrations.md#generalization-pass-held-out-apps)).
3. **Accepted on 20 frozen apps** that nobody used to write, tune or generalize the rules: AmruthPillai/Reactive-Resume,
   Mail-0/Zero, baptisteArno/typebot.io, boxyhq/saas-starter-kit, dittofeed/dittofeed, hatchet-dev/hatchet,
   ixartz/SaaS-Boilerplate, latitude-dev/latitude-llm, lobehub/lobe-chat, mickasmt/next-saas-stripe-starter,
   onlook-dev/onlook, polarsource/polar, sadmann7/skateshop, shadcn-ui/taxonomy, sourcebot-dev/sourcebot,
   steven-tey/precedent, t3-oss/create-t3-app, usertour/usertour, vercel/ai-chatbot and webstudio-is/webstudio, at
   the commits in [NOTES](NOTES-postgres-migrations.md#frozen-test-set-2026-11-26). One run per app with every rule of
   the pack on, as a whole-repository audit (no Git base), on the profiling engine; every match was read by a reader
   who did not write the rules.
4. **Measured again on two more test sets** that nobody used to write, tune or fix the rules: test set #2, 37
   open-source apps (2026-11-27), and test set #3, 41 more. One run per app with the pack's rules and candidates on, as
   a whole-repository audit, every match read by a reader who did not write the rules
   ([NOTES](NOTES-postgres-migrations.md#test-set-3)).

The bar: a rule ships on only when more than 90% of its matches on apps it was never tuned or fixed on are real. A
test set counts for a rule only if the rule was not fixed against its findings: `lock-timeout` was fixed after test
sets #1 and #2, so only test set #3 counts for it. Promoting a candidate or keeping a rule on takes at least 5 such
matches in at least 2 apps; a rule under that floor with no false match stays where it is. A rule below the bar ships
off, with its measured precision, its false-match shapes and the fix each needs in its details. A rule without such
matches has no measured precision on unseen apps, so it is not part of the pack ([the
candidates](candidates/README.md)).

Not run: for this pack, nothing; every rule ran on every app of the three test sets (the runs that did not finish
were other packs'). Not measured on the test sets: a Git base (a session's or pull request's pending migrations),
which only the corpus's base runs measure, and any app's migration other than each folder's newest.

| Rule | Starts | Mode | Catches | Test set #1: 20 apps | Test set #2: 37 apps | Test set #3: 41 apps | Held-out: 8 apps | Tuning, audit: 27 apps | Tuning, base: 50 pending | Tuning, whole history: review (3 apps) + writers | Tuning, precision of read matches |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `postgres-migrations-practice-constraint-on-existing-table-not-valid` | On | Steer | a foreign key or CHECK added to an existing table without `NOT VALID` | 1 of 1 real (usertour) | 2 of 2 real, in 2 apps | 3 of 3 real, in 1 app | 0 matches | 3 (1) | 5 (2) | 107 + 320 (6) | 100% (21 of 21; 320 of 320) |
| `postgres-migrations-practice-index-on-existing-table-blocks-writes` | On | Steer | `CREATE [UNIQUE] INDEX` without `CONCURRENTLY`, or `ADD UNIQUE`/`PRIMARY KEY`, on an existing table | 0 matches | 5 of 5 real, in 3 apps | 0 matches | 3, none real (one operator-run migration that locks the table first); 0 after the explicit-lock fix | 9 (4) | 0 | 204 + 527 (7) | 100% (30 of 30 read by the review; 439 of 442 by the writers, the 3 since fixed) |
| `postgres-migrations-practice-set-not-null-scans-table` | On | Steer | `SET NOT NULL` on an existing table with no validated `CHECK (c IS NOT NULL)` | 10 of 10 real (one usertour migration) | 0 matches | 0 matches | 0 matches | 0 | 2 statements (1; 4 findings before one per statement) | 6 + 135 (6) | 100% (10 of 10; 135 of 135) |
| `postgres-migrations-practice-lock-timeout` | On | Log | a migration that locks an existing table with no `lock_timeout` set | 6 of 8 real (75%), in 7 apps; fixed against them | 14 of 17 real, in 16 apps; fixed against them | 9 of 9 real, in 9 apps | 5 of 6 real; 5 of 5 after the explicit-lock fix | 12 (11) | 29 of the 50 migrations | 440 (3) | 100% (14 of 14) |

A match is real when the statement does what the rule names on a table that existed before the migration, even if the
table is small; the messages say when to keep the statement (a table known to stay small, or one still empty because
the migration that creates it has not been deployed). The three rules that steer: every match read on the three test
sets was real (the index rule's false matches, 3 in the writers' corpus runs and 3 on the held-out apps, were fixed
before them). `lock-timeout` logs, because a timeout set on the database role outside the repository cannot be seen
and it fires on 60 to 90% of the migrations of any repository that sets none. It shipped off after test set #1 (6 of
8 real) and was fixed against the false matches of test sets #1 and #2; on test set #3 all 9 of its matches were
real, so it ships on ([its details](#postgres-migrations-practice-lock-timeout)).

## What an audit reports

A whole-repository audit has no Git base, so it reads only the newest migration of each migrations folder: the one a
coding agent has just written, and the one still worth changing. On the three corpus repositories with the longest
histories (local optimized engine, one run per rule and repository), the pack's matches against the writers'
whole-history matches on the same repositories:

| Rule | langfuse (500 migrations) | trigger.dev (894) | documenso (170) |
|---|--:|--:|--:|
| `constraint-on-existing-table-not-valid` | 0 (was 41) | 0 (was 65) | 0 (was 52) |
| `index-on-existing-table-blocks-writes` | 1 (was 93) | 0 (not in the writers' run) | 0 (was 73) |
| `set-not-null-scans-table` | 0 (was 27) | 0 (was 16) | 0 (was 36) |
| `lock-timeout` | 1 (was 1) | 2 (was 1) | 0 (was 0) |

On those three, 4 matches where whole-history runs gave 405, all real. Three are `lock-timeout`'s: langfuse's newest
migration (`20261120140000_add_gateway_spend_limit_warning_thresholds`) alters `gateway_spend_limits` with no
`lock_timeout` (line 3); trigger.dev's `run-ops-database` folder adds a column to `TaskRun` with no `lock_timeout`, as
does its `dashboard-agent-db` Drizzle folder to `chats` (a folder newer than the writer's run). The fourth,
`index-on-existing-table-blocks-writes`, is in the same langfuse migration: it rebuilds a unique index on
`gateway_spend_limit_alerts`, a table an earlier migration created, without `CONCURRENTLY` (line 10). The other newest
migrations lock nothing the rules name: trigger.dev's main folder ends with a `CREATE INDEX CONCURRENTLY` alone in its
migration, documenso's with an `UPDATE`.

Over all 27 corpus apps with SQL migrations (the review's audit), the four rules report 24 findings, all real:
`constraint-on-existing-table-not-valid` 3 (papermark's `20260820000000_add_multiple_team_brands`),
`index-on-existing-table-blocks-writes` 9 in 4 apps (the same papermark migration alone 5; one on a small table,
airbroke's `accounts`) and `lock-timeout` 12 in 11 apps.

## Starting values

All four rules start on. `constraint-on-existing-table-not-valid`, `index-on-existing-table-blocks-writes` and
`set-not-null-scans-table` steer; `lock-timeout` logs. To turn one on or off or change its mode, use its switch in the
Postgres migrations pack's page in the dashboard, or `repository_settings_set` (`PUT /api/repository/settings`) for a
repository or a checkout ([rule scoping](../../docs/rule-scoping.md#management-surfaces)).

The rules are written in `.rerail`, so they run apart from JSON rules: in the background, once the coding agent's writes
have settled, and in audits and `rerail scan`. They read only migration files (and `migration_lock.toml`, the
`*.prisma` schema files and Drizzle configs for the database a folder declares, and the configuration files
`lock-timeout` searches for a timeout). On the largest histories in the corpus one rule's run took 2 to 17 seconds on
a quiet machine (langfuse 500 migrations, trigger.dev 894, documenso 170) and 25 to 56 seconds on civitai (1,218
migration files) on a loaded one.

## Rule details

What each rule matches beyond its one-line description: exact matching, what it leaves alone, gaps and sources. Every
rule reads `**/migrations/**/*.sql` and `**/drizzle/**/*.sql` outside test, fixture and build folders, only in pending
PostgreSQL migrations (see [above](#which-migrations-a-rule-reads)); the lists below do not repeat that.

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
warning); `UNIQUE`, `PRIMARY KEY` and `EXCLUDE` constraints (index builds, which this rule does not read);
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

Test set #1, 20 unseen apps: 1 match, real. usertour's
`20261012000000_required_content_step_theme_segment_columns` drops `Content_environmentId_fkey` and adds it back
(`FOREIGN KEY ("environmentId") REFERENCES "Environment"("id") ON DELETE RESTRICT`, line 40) without `NOT VALID`, so
every `Content` row is checked again under SHARE ROW EXCLUSIVE; the comment above it is about the `onDelete` change
only. Held-out, 8 apps: 0 matches.

Test set #2, 37 apps: 2 matches in 2 apps, both real at low impact: kan's foreign key from `card` to `user` on a
column the same migration has just added (all NULL, so the scan is short), and devtrack's locale CHECK re-added to
`users`, probably a small table. Test set #3, 41 apps: 3 matches in one migration, all real: aural-oss's
`005_account_delete_and_answer_bank.sql` adds foreign keys to `webhooks`, `projects` and `interviews`, tables its
initial migration created and its app writes to, without `NOT VALID`. On every unseen app, 6 of 6 real in 4 apps.

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
  PRIMARY KEY`, Prisma's primary-key swap): the rewrite holds the lock anyway, and the candidate
  column-type-change rule owns it;
- a statement on a table the migration has already locked itself with `LOCK TABLE t` (ACCESS EXCLUSIVE, the default
  mode; for a `CREATE INDEX`, any mode that stops writes: SHARE, SHARE ROW EXCLUSIVE, EXCLUSIVE): the lock holds to
  the end of the transaction, so the build stalls nothing the migration did not choose to stop (an operator-run
  migration in a maintenance window).

Gaps: a table known to stay small is reported (nothing in the repository says how big a table is). `EXCLUDE`
constraints, and `UNIQUE`/`PRIMARY KEY` constraints on a partitioned table, also block and are not reported. A Drizzle
`IF NOT EXISTS` index is trusted to have been built by hand. The Prisma version is not read, so the Prisma fix is the
one that works on every version (alone in its own migration), though Prisma 7.4+ also accepts it beside other
statements outside `BEGIN … COMMIT`. A column added in the same migration is all NULL, so its index builds fast; the
rule still reports it (three of its five test-set #2 matches, all real at low impact).

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
useSend, langfuse, shelf.nu, inbox-zero): 527 matches in 7. 442 read (every match of six repositories, every fourth of
inbox-zero's 118): 439 real, 3 not, all three Prisma's primary-key swap with `SET DATA TYPE` in the same `ALTER TABLE`
(since left to the column-type-change rule). Every reported statement builds an index on a table an earlier migration
created. The corpus also shows the fix in use: langfuse writes each `CREATE INDEX CONCURRENTLY` alone in its own
Prisma migration (`20240226165118_add_observations_index` and 20 more), useSend does the same
(`20261117010000_email_team_created_at_index`), and shelf.nu's own comments make the size call the message asks for
(`20260525110348_add_trigram_indexes_for_simple_search`: "Each statement takes an ACCESS EXCLUSIVE lock on its table
for the duration of the index build. On the largest known tenant the affected tables are small … If a much larger
workspace is onboarded later, run this migration during a low-traffic window or split it across multiple
CONCURRENTLY-built migrations.").

Held-out, 8 apps: 3 matches, none real, all in one jitsu migration run by an operator that takes `LOCK TABLE … IN
ACCESS EXCLUSIVE MODE` (writes stopped first) before its index builds; the explicit-lock fix leaves them alone (0 on
the re-run). Test set #1, 20 apps: 0 matches, so the rule waited as a candidate.

**Ships on.** Test set #2, 37 apps: 5 matches in 3 apps, all real, the first unseen evidence. kan's partial index on
the existing `card` (`AddCardArchive`, a Drizzle migration), snagtime's three `CREATE UNIQUE INDEX IF NOT EXISTS` on
the existing `Booking` (a Prisma migration, where `IF NOT EXISTS` still builds under the lock), each on a column the
same file adds (all NULL, so the builds are short: real at low impact), and quenti's `Highscore` primary key dropped
and re-added, which rebuilds its index under ACCESS EXCLUSIVE on a table an earlier migration created. Test set #3, 41
apps: 0 matches. 5 of 5 real in 3 apps clears the bar and the floor. Promoted from the candidates with its cases and
no change to what it matches; for the split test, the partitioned-parent fact (which any migration of the folder may
give) is keyed by the document and the table's name instead of by the statement
([NOTES](NOTES-postgres-migrations.md#test-set-3)).

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
`UPDATE`, `SET NOT NULL`: the usual way to add a required column to a table with rows, and 6 of the 10 matches the
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

Test set #1, 20 unseen apps: 10 matches, all real, all in one usertour migration
(`20261012000000_required_content_step_theme_segment_columns`, lines 10 to 35): `SET NOT NULL` on the existing
`Content`, `Step`, `Theme` and `Segment` tables with no CHECK proof, several right after an `UPDATE` backfill, one on a
column the file's own comment says may still hold NULLs (the migration then fails). The two on `Theme` and `Segment`,
probably small tables, are real at low impact. Held-out, 8 apps: 0 matches. Test sets #2 (37 apps) and #3 (41 apps):
0 matches. Its unseen evidence stays 10 of 10 in one app, under the floor of 2 apps with no false match, so it stays
on.

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
- `ALTER TABLE "User" …` (any action but those below that take SHARE UPDATE EXCLUSIVE) on a table the migration did
  not create earlier, `SET (user_catalog_table = …)` included;
- a foreign key, in `CREATE TABLE … REFERENCES "User"` or `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY … REFERENCES
  "User"`, even from a brand-new table: it locks `"User"` (SHARE ROW EXCLUSIVE);
- `CREATE [UNIQUE] INDEX` without CONCURRENTLY on an existing table (SHARE: queues writers);
- `DROP TABLE`, `CREATE TRIGGER … ON` an existing table;
- with `SET lock_timeout = 0` before the statement (it turns the timeout off), or a `SET lock_timeout` placed after it;
- after a `LOCK TABLE` in a weaker mode than ACCESS EXCLUSIVE (`IN ROW EXCLUSIVE MODE`): the ALTER still waits.

Leaves alone:
- `SET [LOCAL|SESSION] lock_timeout = '5s'` (or `'500ms'`, `'0.5s'`) or `SELECT set_config('lock_timeout', '5s', …)`
  before the statement;
- tables created earlier in the same migration, or by a migration of the same folder missing from the Git base
  (nobody else holds a lock on them; a table only emptied, or given a required column, still has); Prisma's init
  migration and any migration whose foreign keys only point at tables it creates;
- `CREATE INDEX CONCURRENTLY`, and an `ALTER TABLE` whose every action takes SHARE UPDATE EXCLUSIVE, which blocks no
  read or write (PostgreSQL's ALTER TABLE reference): `VALIDATE CONSTRAINT`; `SET (…)`/`RESET (…)` of autovacuum
  (`autovacuum_*`), TOAST (`toast.*`, `toast_tuple_target`), `fillfactor` or `parallel_workers` storage parameters;
  `ALTER COLUMN … SET STATISTICS` and a column's `SET (n_distinct = …)`; `CLUSTER ON`, `SET WITHOUT CLUSTER`. Beside
  any other action in the same statement it is reported;
- statements on a table the migration has already locked itself with `LOCK TABLE t` (ACCESS EXCLUSIVE, the default
  mode): they wait for no other session, and the migration stops the table on purpose;
- statements in a function body or DO block;
- every migration when the repository sets the timeout elsewhere: a migration with `ALTER ROLE|USER|DATABASE … SET
  lock_timeout`; or a non-zero `lock_timeout=`, `lock_timeout:` or `lock_timeout TO` (a URL's `lock_timeout%3D5s`, an
  environment variable such as `DB_LOCK_TIMEOUT=5s`) in package.json, `.env*`, `*.config.{ts,js,mjs,cjs}`
  (prisma.config.ts, drizzle.config.ts), CI workflows (GitHub, GitLab), `*.sh`, Dockerfiles, compose files, platform
  deploy files (Procfile, render.yaml, railway.toml/json, fly.toml, vercel.json, netlify.toml, app.yaml), a Makefile,
  or a JS/TS script named for migrations (`migrate.mjs`, `run-migrations.ts`), four directory levels deep, past
  node_modules, tests and build output. Such a file counts for a migrations folder unless a package.json sits between
  them: `apps/web/.env` sets the web app's connection, not `packages/db`'s;
- every migration of a folder whose runner sets the timeout on its connection: a JS/TS source file (not a test, spec,
  story or declaration file, within the same four-level walk) that sends a non-zero `SET lock_timeout` in a string or
  template literal (`client.query("SET lock_timeout = '5s'")`, `` `SET LOCAL lock_timeout = '${LOCK_TIMEOUT}'` ``)
  and names the folder in another: by a path that resolves to it, wherever the file lies (a one-line `join(…)` or
  `resolve(…)` of a name such as `__dirname` or `HERE` and string literals, or a literal starting with `./` or `../`,
  read from the file's folder; a path of two segments or more without `..` also from the repository root:
  `join(HERE, "..", "..", "..", "apps", "web", "drizzle")` in `packages/store/scripts/migrate.mjs` runs
  `apps/web/drizzle`), or by its last segment (`migrate(db, { migrationsFolder: "./drizzle" })`,
  `resolve("migrations")`) when it lies inside the folder's package (no package.json above the folder that is not
  above the file too). Startup code in `apps/server/src/…` running the root `migrations/` counts; runtime code that
  sets a timeout for its own transactions names no migrations folder and does not;
- migrations of an example project (`examples/`, `example/`), out of scope like test and fixture folders: nobody
  deploys them from the repository.

Gaps: a timeout set on the database role outside the repository (managed Postgres consoles, infrastructure repos) is
unknowable; the rule logs for that reason. `SET LOCAL` outside a transaction block is accepted, though under Prisma
>= 7.4 (statements run one by one) it has no effect. `DROP INDEX` without CONCURRENTLY locks its table, which the
statement does not name; not counted. `LOCK TABLE` itself waits in the queue like any statement but is not counted:
a migration that takes it has planned the lock. A runner in another package than its migrations folder that names it
only by its last segment (an app's server running `packages/db/migrations` as `"migrations"`) or through a variable,
a `join(…)` split over lines, a path read from the working directory (`resolve(process.cwd(), "drizzle")`), a runner
deeper than four levels, or one that names no folder in a string literal is not read; a runner that names a folder
by its last segment counts for every folder of that name in its package.

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

Test set #1, 20 unseen apps: 6 of 8 matches real (75%), below the 90% bar, so it shipped off. Real: typebot.io's
`20260821120000_remove_inactive_workspace_cleanup` (`DROP COLUMN` on the hot `Workspace`), usertour's
`20261012000000_required_content_step_theme_segment_columns` (the first of ten ALTERs, on `Content`), and four at low
impact: Zero's and dittofeed's foreign keys from new tables to the existing `mail0_user` and `Workspace`,
saas-starter-kit's `VARCHAR(256)` widening on `jackson_store` (no rewrite, but ACCESS EXCLUSIVE), latitude-llm's `ADD
COLUMN` on `trace_deletion_requests`, a table a migration of the same day created. The two not real, each fixed since
by what the code does:
- **A timeout set by the app's own migration runner.** Reactive-Resume applies its migrations from TypeScript startup
  code (`apps/server/src/startup/checks.ts`), which sends `SET lock_timeout = '5s'` before them, with a comment on the
  lock queue. The rule looked for a timeout only in migrations, config, CI and deploy files and scripts named for
  migrations. Fixed: a JS/TS source file of the package that holds the folder that sends a non-zero `SET
  lock_timeout` and names the folder counts as its runner (`lt_runner_timeout`; cases `runner/`, `runner-zero/`,
  `runner-runtime/`).
- **An example app's migration.** dittofeed's `examples/supabase/supabase/migrations/20221017024722_init.sql`, the init
  migration of an example Supabase project nobody deploys from the repository, creates `profiles` with a foreign key
  to Supabase's `auth.users`. Fixed: `**/examples/**` and `**/example/**` are out of the rule's scope (its `.scope
  exclude`), as test and fixture folders already are (case `examples/blog/supabase/…`).

Re-run on the branch head's engine (one `run-rr` per app): test set 9 → 7 matches, the two above gone; the 7 left are
the six real ones and lobe-chat's `0230_custom_emoji_and_reactions.sql:54`, a foreign key from the new
`channel_message_reactions` to the existing `channels` (real, low impact), which the frozen run's engine did not report.
Held-out 5 → 5, unchanged. Corpus, 33 apps: 24 → 15; the 9 gone are all in supabase/supabase's `examples/` (example
Supabase projects' init and auth migrations), none real by the same reading as dittofeed's; no runner fix removed a
corpus match.

Held-out, 8 apps: 6 matches, 5 real; the sixth, an operator-run migration that takes `LOCK TABLE … IN ACCESS EXCLUSIVE
MODE` before altering the table, is left alone since the explicit-lock fix (5 of 5 on the re-run).

Test set #2, 37 apps: 17 matches in 16 apps, 14 real. The 3 not real were each fixed with a case
([NOTES](NOTES-postgres-migrations.md#fixes-after-the-first-test-set)): a runner in another package than the folder,
naming it by a resolved path, that sends `SET LOCAL lock_timeout`; a SQLite Prisma folder that a PostgreSQL schema
deeper in the tree was read as declaring (the library now reads only the schema beside the folder); and an `ALTER
TABLE … SET (autovacuum_…)`, which takes SHARE UPDATE EXCLUSIVE and blocks no read or write. After the fixes, 14 of
14 there, and no change on test set #1, the held-out apps or the corpus.

**Ships on.** Test set #3, 41 apps, the first the rule was not fixed against: 9 matches in 9 apps, all real. Seven
are an `ADD COLUMN` or another strong ALTER on a table an earlier migration created (aural-oss's `webhooks`, refref's
`product`, next16-conferences' `Talk`, voicehub's `SystemSettings`, nextacular's `domains`, uxie's `Document`,
v0.diy's `chat_ownerships`, whose own runner sets no timeout), one is a foreign key from a new table to the existing
`user` (codu), and one adds a `SERIAL` column to next-admin's `Category`, a demo app's table (low impact). None of the
nine apps sets `lock_timeout` anywhere the rule looks or in a runner. Several of the migrations are dated months or
years back and are probably applied already: an audit reads each folder's newest migration, applied or not, and the
finding is real by the statement. 9 of 9 in 9 apps clears the bar and the floor.

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
| `rules/postgres-migrations-practice-<id>.rerail` | The 4 rules, written by hand. Each opens with a comment on what it matches and leaves alone, with an example of each |
| [`rules/lib/postgres-migrations.rerail`](rules/lib/postgres-migrations.rerail) | `lib/postgres-migrations`, the shared gating: migrations, their order and which are pending, other databases' folders, runners, tables and whether they may hold rows, column helpers |
| `cases/<id>/` | Each rule's cases, a small repository: a line ending in `-- caught` must be reported, no other line may be (the rule file's `.test cases`). `cases/_git/` is the one-commit `.git` every case repository uses, `cases/_other-databases/` the other databases' migrations every rule's `other_databases` test reads. Not published |
| `candidates/` | Rules written for the pack without enough matches on apps they were not tuned or fixed on to ship, so not part of it: their files, cases and details ([`candidates/README.md`](candidates/README.md)). Not published |
| [`NOTES-postgres-migrations.md`](NOTES-postgres-migrations.md) | The batch notes: dropped candidates, the library's history and open proposals, every corpus match read with its verdict, and the three test sets' matches with theirs |
| [`ENGINE-GAPS.md`](ENGINE-GAPS.md) | Engine gaps found while writing and measuring these rules |
