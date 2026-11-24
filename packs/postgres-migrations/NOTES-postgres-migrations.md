# Notes: batch postgres-migrations (Rerail's own Postgres migration practice rules)

10 rules, written by hand as `.rerail`, 9 over SQL trees through one library,
`rules/lib/postgres-migrations.rerail`, and one (`dropped-column-still-used`) a TypeScript policy that reads the
migrations and `*.prisma` files as text. None is a port, so there is no upstream to score against: each rule is checked
by its cases (`cases/<id>/`, run by the rule file's own test) and by runs on the apps of
[README.md](README.md#the-corpus) with the largest Prisma and Drizzle migration histories, whose matches were read. What
each rule matches, leaves alone and misses, with its corpus numbers and sources, is in README.md's "Rule details"; the
verdict on every match read is at the end of this file.

## Dropped candidates

No rule written for this batch was dropped: all 10 are in pack.json. Ideas dropped while mining, before any rule was
written:

- A blanket "no DROP COLUMN/TABLE": drops are usually intended; replaced by `rename-as-drop-and-add` and
  `dropped-column-still-used`, which report the drops that lose data or break the app.
- "A multi-statement Prisma ≥ 7.4 migration without `BEGIN … COMMIT`" (a partial apply on failure): real, but
  `BEGIN` holds locks longer and blocks `CONCURRENTLY`; the advice conflicts with `index-on-existing-table-blocks-writes`
  and `concurrently-inside-transaction`.
- "The migration still carries Prisma's `Warnings:` block": the block stays after a reviewed migration; used only as
  evidence in `required-column-without-default` and `rename-as-drop-and-add`.
- `prefer-bigint` for SERIAL/INT primary keys: a real outage risk, but a `schema.prisma` decision (the `prisma` pack's).
- `prisma db push` / `migrate reset` in deploy scripts: a data-loss footgun, but it lives in `package.json`/CI (the
  `prisma` pack's).
- `json` vs `jsonb`: Prisma defaults to `jsonb`; low severity.
- `IF NOT EXISTS` robustness (Squawk prefer-robust-stmts): can mask an INVALID index; wrong advice for concurrent
  builds.
- Enum value ordering, identifier length, schema qualification, char fields: style.
- Client-compatibility opt-in bans (add column, new write restriction, add enum value): noisy for an app that ships its
  own client.
- RLS/role/grant bans: rare in Prisma/Drizzle migrations; no precise mechanical footgun.
- Squawk's `require-concurrent-index-deletion`/`reindex`: already in the `sql` pack's sqlfluff PG01.
- A reminder for Prisma 8's `operationClass: "destructive"`: Prisma 8 already prompts at plan time.

Mined and ranked but not written in this batch (next in line): `not-valid-validated-in-same-transaction` (a `VALIDATE
CONSTRAINT` in the same transaction as its `ADD … NOT VALID`), `new-enum-value-used-in-same-transaction`,
`backfill-holds-ddl-lock` (an UPDATE in the same transaction as an ALTER of its table), `rename-breaks-running-app`
(`RENAME COLUMN`/`RENAME TO` while old instances still select the old name), `applied-migration-edited` (needs the
base's file contents) and `unique-on-existing-data-without-dedupe`.

## Library: `rules/lib/postgres-migrations.rerail`

Each rule was first written with its own copy of the gating, under its own prefix (`iotb_`, `cit_`, `cnv_`, `snn_`,
`rcwd_`, `vdr_`, `ctc_`, `lt_`, `rad_`). The copies agreed in intent and differed in detail; one copy of each now sits in
the library (`pgm_`), every SQL rule includes it, and the copies are gone. Where the variants differed, the library took
the most complete one:

- **Migrations and their order** (`pgm_mig`, `pgm_newer`): from `lt_mig`, which knew `up.sql` and left down migrations
  out; `ctc_mig`/`rad_mig` knew only `<dir>/migration.sql` and `<dir>/<name>.sql`. `ctc_pos`/`rad_pos` (a node's place
  in the run order) stay in their rules, now over `pgm_mig`: each is bounded by the rule's own nodes, and a library
  relation over every node would cost more than it shares.
- **Pending** (`pgm_applied`, `pgm_pending`): with a Git base every rule already read "in the base" alike
  (`iotb_applied` = `snn_applied` = `rcwd_applied` = `vdr_applied` = `cnv_applied` = `ctc_applied` = `rad_applied` =
  `cit_applied`). Without a base, eight rules read every migration and `lock-timeout` only each folder's newest. The
  library takes `lock-timeout`'s policy for all of them: a whole-repository audit of a mature repository otherwise
  reports hundreds of statements in migrations that ran years ago (index-on-existing-table-blocks-writes: 527 in 7
  repositories). `pgm_pending` also leaves out other databases' folders, which before only three rules did.
- **Other databases** (`pgm_other_db`): the union of three variants. `iotb_other_db` read folder names (sqlite, mysql,
  mariadb, mssql); `lt_other_group` a longer list of names (adding ClickHouse, libSQL/Turso, SQL Server, PlanetScale)
  plus a table `ENGINE = …` clause or backquoted names in any statement of the folder; `cnv_other_dialect` Prisma's
  `migration_lock.toml` provider and backquoted identifiers, per file. The lock-file clause never worked: `file_line`
  on a path computed from a document gives no rows ([ENGINE-GAPS.md](ENGINE-GAPS.md), gap 3), and `cnv`'s MySQL case
  passed only through its backticks. The library reads the lock file as a document instead (every rule adds
  `**/migrations/migration_lock.toml` to its `.scope include` and `.scope text`), and a new case
  (`index-on-existing-table-blocks-writes`, `apps/edge/`) proves it. A statement's text holds the comments inside it,
  so `lt_other_group`'s backtick test took langfuse's whole Prisma folder for MySQL once a migration quoted
  ``` `projects` ``` in a comment (`20260821121500_backfill_evaluator_v2`); the library strips comments and string
  literals first, and a case (`db/migrations/006_audit.sql`) covers it.
- **Runner** (`pgm_runner`): `iotb_runner` as it was; `concurrently-inside-transaction` used the same tests inline and
  now reads it too.
- **Tables** (`pgm_ref`, `pgm_target`): every copy folded the name with `ident_pg` on the object reference's `name`
  field. They differed in which object reference: `iotb_word`, `cnv_word`, `rcwd_table`, `ctc_tbl` and `rad_tbl` took
  any child object reference, `lt_target` the first. The library takes the first (a `CREATE TABLE t PARTITION OF p`
  names `p` second), plus each table of a `TRUNCATE`.
- **An existing table** (`pgm_new_table`, `pgm_empty_table`, `pgm_fresh_table`): the copies ranged from "created
  earlier in the file or by a pending migration" (`snn_new`, `ctc_new_table`, `rad_new_table`, `lt_created`) through
  "or materialized or truncated" (`iotb_fresh`) and "or truncated or wholly deleted" (`rcwd_empty`, `vdr_empty`) to "or
  given a NOT NULL column without a default earlier in the file or the same statement" (`cnv_fresh` with
  `cnv_required`). The library has three layers, and each rule takes the most complete one that is right for it:
  `pgm_fresh_table` (all of them) for the index, constraint, set-not-null, volatile-default and column-type rules;
  `pgm_empty_table` (no required-column proof) for `required-column-without-default`, where that column is the finding
  itself, and for `rename-as-drop-and-add`, whose main case is Prisma's `DROP COLUMN "a", ADD COLUMN "b" TEXT NOT
  NULL`; `pgm_new_table`'s facts (creation only) for `lock-timeout`, since an emptied table still has others waiting
  on its lock. `ctc_new_table` also required the creating migration to sort before the altering one; the library does
  not, as `iotb_fresh` did not.
- **Required columns** (`pgm_required`, `pgm_filled`, `pgm_serial`): from `rcwd_required`/`rcwd_filled` (NOT NULL as two
  adjacent keywords; DEFAULT, GENERATED, identity, serial types read from keyword or type nodes, not from a column
  named `serial`). `cnv_required` read `has(…, "keyword_not")` anywhere below the definition and globbed its text for
  "serial"/"generated", so a `CHECK (x IS NOT NULL)` inside a column definition counted; `vdr`'s serial test also
  matched a column named `serial`.
- **Columns** (`pgm_ident`, `pgm_colname`): `ctc_col` = `rad_col`, `vdr_col` = `rcwd_col`.
- **Volatile functions** (`pgm_volatile`): `vdr_volatile` as it was.
- **The case repository**: the nine identical copies of the one-commit `.git` (`cases/<id>/git/`) are one,
  `cases/_git/`, which each rule's `.fixtures repo` maps to `.git/…`.

Every rule's cases give the same caught lines as before the library. test-rr runs with no Git base, so under the new
policy the cases would be read as an audit reads them (newest migration only); each `cases` test gives `pgm_applied`
no rows (`.given pgm_applied`) to read every case migration, and `required-column-without-default`'s `no_base` test
reads the same files without it: its older `20261120093000_post_slug` migration, three findings in `cases`, gives none.

Open library candidates, from the writers' notes, not moved because one rule uses them or they read TypeScript:

- `dropped-column-still-used`'s text readers: migrations as text (`dcsu_mig`, `dcsu_alter_at`/`dcsu_action`,
  `dcsu_back`/`dcsu_later`/`dcsu_live`), `schema.prisma` models and fields with `@map`/`@@map`/`@ignore`/`/// @view`
  (`dcsu_open`…`dcsu_column`, `dcsu_view`; the `prisma` pack's sensitive-fields rule has the same parser as `sfc_*`),
  the schema files of a migrations folder (`dcsu_schema_for`), Drizzle `pgTable` columns (`dcsu_dz_table`,
  `dcsu_dz_col`), and raw SQL sent through Prisma or Drizzle (`dcsu_raw`). The rule now includes the SQL library for
  `pgm_pending` (its tree relations find no SQL nodes in a TypeScript policy and stay empty; the path, lock-file and
  Git ones work), so these could move into it or into a TypeScript-side library of their own.
- `concurrently-inside-transaction`'s Prisma version reader (`cit_prisma_74`) and its transaction unit (`cit_units`,
  `cit_in_tx`): worth sharing with `not-valid-validated-in-same-transaction`,
  `new-enum-value-used-in-same-transaction` and `backfill-holds-ddl-lock` when they are written.
- `set-not-null-scans-table`'s `CHECK (c IS NOT NULL)` facts (`snn_check`, `snn_validated`), which
  `not-valid-validated-in-same-transaction` would read too.
- `lock-timeout`'s strong-lock statements (`lt_cand_w`) and "lock_timeout is set" (`lt_timeout_before`,
  `lt_repo_timeout`): a statement_timeout sibling would reuse them. (The checkout walk, `lt_dir`/`lt_config`, is now
  the library's `pgm_dir`/`pgm_config`, which `concurrently-inside-transaction` also reads for `prisma migrate
  deploy` in Dockerfiles, shell scripts and CI.)
- `column-type-change-rewrites`' and `rename-as-drop-and-add`'s column history (`ctc_def`/`ctc_old`/`ctc_desc`,
  `rad_def`/`rad_old`/`rad_desc`): "the column's type as of a point in the run order", for any rule that asks what a
  column was; `rad_fk` (a foreign key's column and referenced table) and `rad_renamed` (a table renamed).
- `index-on-existing-table-blocks-writes`' `iotb_retype` (an ALTER TABLE that also changes a column's type).

## Audits after the library

Re-run of each rule with the optimized engine (`run-rr`, one rule and repository at a time, no Git base, so the newest
migration of each folder only) on the three repositories with the longest histories in the corpus, against the
whole-history numbers the writers measured on the same repositories:

- 4 matches where the same repositories' whole histories gave 498 (index-on-existing-table-blocks-writes 1 vs 166,
  constraint-on-existing-table-not-valid 0 vs 158, set-not-null-scans-table 0 vs 79, required-column-without-default
  0 vs 18, column-type-change-rewrites 0 vs 36, rename-as-drop-and-add 0 vs 39, lock-timeout 3 vs 2; the other three
  rules 0 either way). The per-repository table is in README.md's "What an audit reports".
- Every zero was checked against the folder's newest migration (trigger.dev's main folder ends with a `CREATE INDEX
  CONCURRENTLY` alone in its migration, documenso's with an `UPDATE`), and a probe on langfuse confirmed its newest
  migration is pending: the first run gave 0 for every rule there, which led to the comment-stripping fix above.
- lock-timeout's third match, trigger.dev `internal-packages/dashboard-agent-db/drizzle/0008_chat_kind.sql:1`
  (`ALTER TABLE "trigger_dashboard_agent"."chats" ADD COLUMN "kind" text;`, no lock_timeout in the folder or its
  `migrate.mjs`), is real; that Drizzle folder was not in the writer's run.

## Fixes after the pack review

The review read every rule on the full corpus three ways (an audit of the 27 apps with SQL migrations, 50 pending
migrations read with a Git base in five apps, the whole histories of papermark, comp and customermates) plus four
adversarial repositories; its numbers are in README.md's table. What changed after it, re-measured with the same
setup (`run-rr` audits through `corpus-local.sh`, `logic --base HEAD~1` on the same base clones, whole-history runs
with the newest-only clause removed from a copy of the library):

- **Library.** `pgm_runner` reads drizzle-kit 1.0's layout (`drizzle/<ts>_x/migration.sql` with a `snapshot.json`
  beside it, no journal) as Drizzle (`pgm_drizzle`): before, the index rule gave it the plain/Prisma text and
  `concurrently-inside-transaction` did not report a CONCURRENTLY there. `pgm_mig` no longer reads another `.sql`
  beside a `migration.sql`/`up.sql` as a one-file migrations folder (civitai's `rollback.sql`, `backfill.sql`), which
  `required-column-without-default` and `lock-timeout` reported as pending. `pgm_partitioned` names tables created
  `PARTITION BY`. The checkout walk moved from `lock-timeout` into the library (`pgm_dir`/`pgm_config`).
- **index-on-existing-table-blocks-writes** (steer, kept: re-measured). A Drizzle `CREATE [UNIQUE] INDEX IF NOT
  EXISTS`, the form its own Drizzle text (and `concurrently-inside-transaction`'s) prescribes, was reported again with
  the same text: a steering loop. It is left alone now, and the text says so. An index on a partitioned parent was
  steered to a CONCURRENTLY PostgreSQL refuses; it gets the `ON ONLY` + per-partition CONCURRENTLY + `ATTACH
  PARTITION` text, and a UNIQUE/PRIMARY KEY constraint on one (no `USING INDEX` there) is left alone. The Prisma texts
  say to write the migration with `prisma migrate dev --create-only` instead of editing one `migrate dev` has applied.
  Audit 9 → 9, whole history 204 → 204, base 0 → 0; adversarial: `advB/…/0002_name_idx.sql:1` no longer reported,
  `advA/…/partition_idx:2` reported with the partition text, `advC` with the Drizzle text.
- **concurrently-inside-transaction** (steer). Catches drizzle-kit 1.0 (`advC/…/20250201000000_idx/migration.sql:2`,
  missed before). `prisma migrate deploy`/`migrate dev` is also read from Dockerfiles, shell scripts, compose files
  and CI workflows (dossier, crawlseo and customermates run it only there), not from JS/TS files: civitai's
  `scripts/prisma-migrate-with-views-workaround.mjs` wraps `migrate dev` for local work and would bring back its 45
  hand-applied matches. Whole history of civitai, dossier, customermates, crawlseo: 0 → 0. A `deploy_script` test.
- **constraint-on-existing-table-not-valid** (steer). Message: VALIDATE in a later migration file; with Prisma the next
  file suffices even in the same deploy, only Drizzle needs a later deploy.
- **set-not-null-scans-table** (steer, re-measured). One finding per statement naming all its columns (documenso base 4
  → 2). When every column was added earlier in the same file (6 of the 10 read), the message leads with `ADD COLUMN …
  NOT NULL DEFAULT <constant>` and otherwise NOT NULL in a later migration, instead of the three-deploy CHECK recipe.
  Whole history 6 → 6 statements (papermark's are now reported at the statement's first line, one above the action).
- **required-column-without-default** (steer). Only the library's rollback.sql fix: civitai audit 0.
- **column-type-change-rewrites** (steer, re-measured). `TIMESTAMPTZ` → `TIMESTAMP` is left alone like its reverse
  (both skip the rewrite only under TimeZone UTC): comp whole history 16 → 14, the two removed being
  `20260121144812_alter_fleet_policy_result_timestamps:2,4`.
- **lock-timeout** (log). The "rerun the deploy" advice follows the runner: Prisma needs `prisma migrate resolve
  --rolled-back <name>` (P3009 otherwise), and from 7.4 a `BEGIN … COMMIT` so no statement is left applied; Drizzle
  rolls back and redeploys. civitai audit 0 (its rollback.sql no longer read).
- **rename-as-drop-and-add** (log). The message asks first whether the new column holds the same facts and says to
  keep the drop and add when the meaning changed (`enabled` → `disabled`), before offering RENAME or `@map`.
- **dropped-column-still-used** (log, on: re-measured). Every migration regex now skips `--` and `/* */` comments and
  string literals (an alternative the scan consumes first), a `DROP TABLE` name list stops at a `--`, only pending
  migrations are read (the rule includes the SQL library for `pgm_pending`), models documented `/// @view` are not
  tables, and an identifier right after a literal or `)` is an output alias. civitai audit 7 → 0 (273 s, was 332 s);
  cal.diy, trigger.dev, documenso, shelf.nu, inbox-zero, langfuse, papermark, comp, customermates 0; the rallly probe
  (a drop of `polls.location`, `polls.updated_at` added as the newest migration) still reported at both columns.
  `advD`'s commented-out drop is not reported.

## Open fixes

From the review, not done here:

- `lock-timeout`: one statement that locks two tables (a foreign key from an existing table) gives two findings at one
  line (`advA/…/20240901000000_fk_existing:1`, `order_items` and `Order`); the header says one finding per migration.
- `index-on-existing-table-blocks-writes`: one finding per migration listing its statements would read better than 5
  index + 3 foreign-key findings for papermark's one migration (`20260820000000_add_multiple_team_brands`).
- `column-type-change-rewrites`: the "from" type does not follow `ALTER TYPE … RENAME` (customermates
  `…dynamic_record_model:2556` says `from "Resource_new"` for a type renamed to `"Resource"` long before). Cosmetic.
- `dropped-column-still-used`: a column qualified by an alias of its table (`SELECT u."legacy" FROM "users" u` in a
  `$queryRaw` template, `advD/src/q.ts`) is missed; still 273 s on civitai.
- `set-not-null-scans-table`: CHECK NOT VALID + VALIDATE + SET NOT NULL in one Prisma 6 file (one transaction holding
  ACCESS EXCLUSIVE, `advA/…/20240601000000_notnull_same_tx`) is not reported: the unwritten
  `not-valid-validated-in-same-transaction` rule's.
- The `sql` pack's README should say to keep sqlfluff PG01/PG02 off where this pack is on (outside this pack).
- Next rules the review ranked: `applied-migration-edited` (needs the base's file content),
  `new-enum-value-used-in-same-transaction`, `drop-in-same-change-as-code-removal`, `backfill-holds-ddl-lock`,
  `not-valid-validated-in-same-transaction`; and a repository fact for large tables so the lock rules could steer
  there and log elsewhere.

## Every match read, with its verdict

The writers' runs read each repository's whole migration history (every migration counted as pending, before the
library's newest-only audit policy). Each list is the rule's final round; the verdicts and the fixes they led to are
summarised in README.md's rule details.

### `postgres-migrations-practice-index-on-existing-table-blocks-writes`

- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230223205820_add_firstname_and_lastname_make_username_unique/migration.sql:16 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230224170332_add_unique_constraint_to_user/migration.sql:8 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230627160243_adding_image_model_and_changing_relationship/migration.sql:26 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230804145414_adding_customer_id_to_user/migration.sql:11 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230811130821_adding_tier_limit_table/migration.sql:22 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231003170230_add_image_to_organization/migration.sql:11 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231121144115_making_name_of_category_unique_within_organization/migration.sql:14 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231121173002_make_name_unique_for_tag_asset_loc_custom/migration.sql:42 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231121173002_make_name_unique_for_tag_asset_loc_custom/migration.sql:45 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231121173002_make_name_unique_for_tag_asset_loc_custom/migration.sql:48 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:12 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:16 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:19 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:22 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:26 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:3 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:30 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:34 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:38 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:42 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:6 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241121134146_add_indexes_for_asset_query_optimization/migration.sql:9 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241218134155_add_indexes_for_better_asset_search_performance/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241218134155_add_indexes_for_better_asset_search_performance/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250114124237_add_reminder_indexes_for_optimization/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250114124237_add_reminder_indexes_for_optimization/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250114124237_add_reminder_indexes_for_optimization/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250114124237_add_reminder_indexes_for_optimization/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250114124237_add_reminder_indexes_for_optimization/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250114124237_add_reminder_indexes_for_optimization/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250114124237_add_reminder_indexes_for_optimization/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250127174933_upgrade_prisma/migration.sql:14 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250127174933_upgrade_prisma/migration.sql:2 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250127174933_upgrade_prisma/migration.sql:20 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250127174933_upgrade_prisma/migration.sql:26 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250127174933_upgrade_prisma/migration.sql:8 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:101 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:104 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:107 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:110 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:113 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:116 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:119 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:122 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:125 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:26 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:32 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:35 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:38 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:41 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:44 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:47 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:50 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:53 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:56 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:59 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:62 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:65 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:68 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:71 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:74 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:77 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:80 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:83 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:86 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:89 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:92 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:95 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250528122114_added_missing_indexes_for_foreign_keys/migration.sql:98 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250716153532_add_kit_category_indexes/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250716153532_add_kit_category_indexes/migration.sql:4 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250716153532_add_kit_category_indexes/migration.sql:6 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250716153532_add_kit_category_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250818111026_add_sequential_id_column/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20250818111026_add_sequential_id_column/migration.sql:14 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20251014111701_add_deleted_at_to_custom_field_model/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20251113141031_add_parent_and_children_location_fields/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20251216201234_add_audit_asset_id_to_audit_note/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260211084408_add_last_selected_organization_id/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260331101357_add_asset_model_and_quantity_based_fields/migration.sql:76 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260401113301_add_index_to_asset_models/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260401113301_add_index_to_asset_models/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260401113301_add_index_to_asset_models/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260403135852_add_quantity_based_custody_fields/migration.sql:14 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260407140322_add_status_index_to_booking_table/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260409100000_rename_asset_to_booking_to_booking_asset/migration.sql:24 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260409100000_rename_asset_to_booking_to_booking_asset/migration.sql:28 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260409100000_rename_asset_to_booking_to_booking_asset/migration.sql:33 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260430100759_add_kit_custody_id_to_custody/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260521133643_assetlocation_kit_discriminator/migration.sql:31 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260521133643_assetlocation_kit_discriminator/migration.sql:51 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260521133643_assetlocation_kit_discriminator/migration.sql:55 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260522103000_add_preferred_barcode_and_extend_qr_id_display_pref/migration.sql:16 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525110348_add_trigram_indexes_for_simple_search/migration.sql:20 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525110348_add_trigram_indexes_for_simple_search/migration.sql:26 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525110348_add_trigram_indexes_for_simple_search/migration.sql:29 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525110348_add_trigram_indexes_for_simple_search/migration.sql:32 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525110348_add_trigram_indexes_for_simple_search/migration.sql:38 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525110348_add_trigram_indexes_for_simple_search/migration.sql:42 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525131507_bookingasset_kit_discriminator/migration.sql:109 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525131507_bookingasset_kit_discriminator/migration.sql:113 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260525131507_bookingasset_kit_discriminator/migration.sql:27 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260526130441_add_trgm_indexes_for_code_search/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260526130441_add_trgm_indexes_for_code_search/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260527150000_consumptionlog_bookingasset_attribution/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260603100000_custody_partial_uniques_split_operator_kit/migration.sql:54 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260603100000_custody_partial_uniques_split_operator_kit/migration.sql:58 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260605000000_add_calendar_token_to_user_organization/migration.sql:5 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260617091901_add_location_description_address_trgm_idx/migration.sql:18 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260617091901_add_location_description_address_trgm_idx/migration.sql:21 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260622134742_add_user_mobile_activity/migration.sql:10 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260629120000_add_asset_archived_at/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260706120000_add_asset_org_createdat_id_index/migration.sql:18 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260706180000_add_booking_and_scan_pagination_indexes/migration.sql:25 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260706180000_add_booking_and_scan_pagination_indexes/migration.sql:28 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260805120000_bookingasset_source_kit_provenance/migration.sql:32 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260810120000_add_booking_asset_model_request_provenance/migration.sql:22 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260814171814_add_asset_custom_field_value_searchable_trgm_idx/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260827090000_booking_asset_checkout_state/migration.sql:170 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260831150000_add_booking_approval/migration.sql:13 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260921100000_add_user_email_lower_index/migration.sql:15 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260925100000_custody_source_location/migration.sql:21 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260925100000_custody_source_location/migration.sql:24 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260925100000_custody_source_location/migration.sql:77 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260925120000_user_business_intel_signup_attribution/migration.sql:10 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260925120000_user_business_intel_signup_attribution/migration.sql:13 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260925140000_bookingasset_source_location/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260929120000_scan_record_every_code/migration.sql:41 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260929120000_scan_record_every_code/migration.sql:44 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260929120000_scan_record_every_code/migration.sql:47 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260929120000_scan_record_every_code/migration.sql:50 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260930120000_add_asset_model_name_trgm_idx/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20261006100000_custody_source_unknown/migration.sql:39 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20261015120100_asset_import_one_active_per_workspace/migration.sql:5 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20261019120000_add_audit_asset_asset_id_index/migration.sql:1 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20230505085908_update_unique_constraint/migration.sql:11 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20230621131348_add_document_id_and_email_index/migration.sql:8 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20230912011344_reverse_document_data_relation/migration.sql:20 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20230918111438_update_subscription_constraints_and_columns/migration.sql:17 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20230920124941_fix_documentmeta_relation/migration.sql:8 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20230922121421_fix_document_meta_schema/migration.sql:49 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231030055821_add_database_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231206073509_add_multple_subscriptions/migration.sql:28 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231206073509_add_multple_subscriptions/migration.sql:31 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231221101005_add_templates/migration.sql:52 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231221101005_add_templates/migration.sql:55 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20231221101005_add_templates/migration.sql:58 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20240205040421_add_teams/migration.sql:144 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20240209023519_add_document_audit_logs/migration.sql:34 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20240220115435_add_public_profile_url_bio/migration.sql:22 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20240227003622_migrate_to_cuids_for_webhooks/migration.sql:11 not (fixed: exempt now): the same ALTER TABLE changes the key column type, the rewrite holds the lock anyway
- documenso/documenso packages/prisma/migrations/20240227111633_rework_user_profiles/migration.sql:31 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- documenso/documenso packages/prisma/migrations/20240227111633_rework_user_profiles/migration.sql:34 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20240327074701_add_secondary_verification_id/migration.sql:18 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20240606060033_add_public_profiles/migration.sql:43 not (fixed: exempt now): the same ALTER TABLE changes the key column type, the rewrite holds the lock anyway
- documenso/documenso packages/prisma/migrations/20240606060033_add_public_profiles/migration.sql:59 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250318072444_add_document_folders/migration.sql:27 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250324135010_model_folder_update_id_type_and_add_pinned_property/migration.sql:22 not (fixed: exempt now): the same ALTER TABLE changes the key column type, the rewrite holds the lock anyway
- documenso/documenso packages/prisma/migrations/20250404111643_add_folder_type_and_template_folders/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:268 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:442 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:445 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:448 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:451 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250807011918_add_organisation_sso_portal/migration.sql:66 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250918070643_remove_template_metadata/migration.sql:54 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:425 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:428 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:431 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:434 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:26 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:32 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:35 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:38 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:41 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251112224336_add_database_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:26 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20251126053402_add_insights_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260113022858_add_envelope_id_index_to_document_audit_log_table/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260219091107_add_recipient_expirations/migration.sql:18 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260302223702_optimize_recipient_indexes/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260302223702_optimize_recipient_indexes/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260302223702_optimize_recipient_indexes/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260302223702_optimize_recipient_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260302223702_optimize_recipient_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- documenso/documenso packages/prisma/migrations/20260401000000_add_reminder_settings/migration.sql:16 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:151 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230626095337_external_trace_id/migration.sql:2 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230707132314_traces_project_id_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230707133415_user_add_email_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230710105741_added_indices/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230710105741_added_indices/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230710105741_added_indices/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230710114928_traces_add_user_id/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711104810_traces_add_index/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711110517_memberships_add_userid_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711112235_fix_indices/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711112235_fix_indices/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711112235_fix_indices/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711112235_fix_indices/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711112235_fix_indices/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230711112235_fix_indices/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230907225604_api_keys_publishable_to_public/migration.sql:8 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230918180320_add_indices/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230918180320_add_indices/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230922030325_add_observation_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230924232620_datasets_continued/migration.sql:20 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20230924232620_datasets_continued/migration.sql:23 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231004005909_add_parent_observation_id_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231005064433_add_release_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231019094815_add_additional_secret_key_column/migration.sql:11 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231019094815_add_additional_secret_key_column/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231019094815_add_additional_secret_key_column/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231019094815_add_additional_secret_key_column/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231019094815_add_additional_secret_key_column/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231030184329_events_add_index_on_projectid/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104004529_scores_add_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104004529_scores_add_index/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104005403_fkey_indicies/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104005403_fkey_indicies/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104005403_fkey_indicies/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104005403_fkey_indicies/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104005403_fkey_indicies/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104005403_fkey_indicies/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231104005403_fkey_indicies/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231110012829_observation_created_at_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231112095703_observations_add_unique_constraint/migration.sql:8 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231116005353_scores_unique_id_projectid/migration.sql:8 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20231130003317_trace_session_input_output/migration.sql:28 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240105170551_index_tags_in_traces/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240118235424_add_index_to_models/migration.sql:11 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240118235424_add_index_to_models/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240124140443_session_composite_key/migration.sql:15 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20240214232619_prompts_add_indicies/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240214232619_prompts_add_indicies/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240424150909_add_job_execution_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240604133338_scores_add_index_name/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240718004923_datasets_tables_add_projectid_composite_key/migration.sql:43 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20240718004923_datasets_tables_add_projectid_composite_key/migration.sql:48 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20240718004923_datasets_tables_add_projectid_composite_key/migration.sql:53 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20240718004923_datasets_tables_add_projectid_composite_key/migration.sql:58 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:118 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:119 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:160 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:35 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:87 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:89 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20250310100328_add_api_key_to_audit_log/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20250403153555_membership_invitations_no_duplicates/migration.sql:17 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20250410145712_add_organization_scoped_api_keys/migration.sql:10 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20251118162943_add_idx_dataset_item_events/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20251127105316_add_pricing_tiers/migration.sql:61 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20251215233903_dataset_items_add_idx_project_id_valid_to/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20260203220623_pending_deletions_object_id_idx/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20260507120000_share_pending_verified_domains/migration.sql:12 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20260507120000_share_pending_verified_domains/migration.sql:9 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20260527110000_monitor_wallclock_lifecycle/migration.sql:9 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20260617000000_slack_integrations_pending_columns/migration.sql:15 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20260706000000_add_api_key_created_by/migration.sql:6 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20260722070000_add_in_app_agent_run_lifecycle_fields/migration.sql:28 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:137 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:138 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:139 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:140 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:141 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261012134000_add_unique_gateway_connection_names/migration.sql:1 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261013120000_add_organization_llm_connections/migration.sql:13 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261014132400_update_gateway_connection_model_routing/migration.sql:12 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261014132400_update_gateway_connection_model_routing/migration.sql:9 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261020134835_add_api_key_user_binding/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261030120000_add_topic_rule_scoped_runs/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261030120000_add_topic_rule_scoped_runs/migration.sql:6 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261114120000_add_role_assignment_expires_at_and_tags/migration.sql:6 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261117120000_add_annotation_queue_session_view_ownership/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261117120000_add_gateway_user_spend_limits/migration.sql:23 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261117120000_add_gateway_user_spend_limits/migration.sql:25 real: plain CREATE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261118120000_add_gateway_default_spend_limits/migration.sql:14 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261118120000_add_gateway_default_spend_limits/migration.sql:7 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- langfuse/langfuse packages/shared/prisma/migrations/20261120140000_add_gateway_spend_limit_warning_thresholds/migration.sql:10 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220511180705_naming_convention_update/migration.sql:107 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220511180705_naming_convention_update/migration.sql:110 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220511180705_naming_convention_update/migration.sql:113 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220511180705_naming_convention_update/migration.sql:116 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220511180705_naming_convention_update/migration.sql:119 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220511180705_naming_convention_update/migration.sql:122 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220623175037_remove_links_model/migration.sql:23 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220623175037_remove_links_model/migration.sql:26 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20220624111614_rename_poll_id/migration.sql:21 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20221026220835_add_indexes/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20221026220835_add_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20230117103853_update_indexes/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20230117103853_update_indexes/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20230117103853_update_indexes/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20230117103853_update_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20230118192253_bring_back_comment_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20230118200546_add_unqiue_participant/migration.sql:8 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20230823084154_stripe_subscriptions/migration.sql:27 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20231118134458_add_account_user_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20231205043530_poll_status/migration.sql:18 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20240309051319_add_option_vote_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20240315104329_index_votes_by_poll/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20241224103150_guest_id_column/migration.sql:12 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20241224103150_guest_id_column/migration.sql:15 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20241224103150_guest_id_column/migration.sql:18 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20241228093234_update_relational_model/migration.sql:94 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250227110115_make_subscription_user_required/migration.sql:27 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250614062855_create_spaces_model/migration.sql:25 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:26 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250615114214_update_indexes/migration.sql:32 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250715153525_update_space_member_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250715153525_update_space_member_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20250901121851_add_ics_columns/migration.sql:10 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20251202151504_better_auth_joins/migration.sql:11 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20251202151504_better_auth_joins/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20251202151504_better_auth_joins/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260423144927_add_instance_id/migration.sql:11 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:38 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:41 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:44 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:47 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:50 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:53 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:56 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:59 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260514165732_migrate_hash_indexes_to_btree_small_tables/migration.sql:62 real: author documents the table as small and the plain lock as accepted, the message's small-table out
- lukevella/rallly packages/database/prisma/migrations/20260518184939_add_sheet_and_extend_scheduled_event/migration.sql:60 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260518184939_add_sheet_and_extend_scheduled_event/migration.sql:63 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260518184939_add_sheet_and_extend_scheduled_event/migration.sql:66 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260616131914_unique_scheduled_event_invite_email/migration.sql:35 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260703161929_add_scheduled_event_space_status_start_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260710120000_add_license_idempotency_key/migration.sql:5 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260719000000_add_user_deleted_at/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260723000000_add_orphaned_anonymous_user_indexes/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260723000000_add_orphaned_anonymous_user_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260723000000_add_orphaned_anonymous_user_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20260802224313_add_poll_invites_and_activities/migration.sql:57 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20261002130000_index_queued_email_retention/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- lukevella/rallly packages/database/prisma/migrations/20261019120000_poll_activities_user_id_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:14 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:20 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:26 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:32 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:35 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:38 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:41 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:44 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:47 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:50 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/03_metric_performance_index/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/04_team_redesign/migration.sql:26 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/04_team_redesign/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/05_add_visit_id/migration.sql:19 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/05_add_visit_id/migration.sql:22 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/06_session_data/migration.sql:12 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/06_session_data/migration.sql:15 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/06_session_data/migration.sql:18 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/07_add_tag/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/09_update_hostname_region/migration.sql:21 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/09_update_hostname_region/migration.sql:22 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/22_add_2fa/migration.sql:73 real: plain CREATE INDEX on a table created in an earlier migration
- umami-software/umami prisma/migrations/23_update_session_data/migration.sql:15 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20250201051912_add_indexes/migration.sql:2 real: plain CREATE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20250201051912_add_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20250201051912_add_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20250202115917_add_payments/migration.sql:31 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20250809231000_add_team_id_to_email_events/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20251006185736_campaign_batching/migration.sql:30 real: plain CREATE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20251006185736_campaign_batching/migration.sql:33 real: plain CREATE INDEX on a table created in an earlier migration
- usesend/useSend apps/web/prisma/migrations/20251006185736_campaign_batching/migration.sql:36 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20230804105315_rule_name/migration.sql:5 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20240513103627_mark_not_cold_email/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250112081255_pending_invite/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250210224905_summary_indexes/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250417135524_writing_style/migration.sql:30 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:238 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:250 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:262 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:274 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:286 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:298 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250506025728_stripe/migration.sql:49 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20250804163003_better_auth/migration.sql:35 real: ADD PRIMARY KEY on an existing table builds the index under ACCESS EXCLUSIVE
- elie222/inbox-zero apps/web/prisma/migrations/20251021123040_drop_executed_rule_unique/migration.sql:8 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20251030010539_indexes/migration.sql:11 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20251030010539_indexes/migration.sql:23 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20251030010539_indexes/migration.sql:35 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20251030010539_indexes/migration.sql:47 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20251223000001_rename_notification_token_to_message_id/migration.sql:6 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260219024141_automation_jobs/migration.sql:57 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260319090000_reply_memory_learning_rework/migration.sql:70 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260407153000_add_apple_iap/migration.sql:15 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260503110000_add_query_performance_indexes/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260503110000_add_query_performance_indexes/migration.sql:17 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260503110000_add_query_performance_indexes/migration.sql:29 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260505095800_document_filing_attachment_unique/migration.sql:31 real: plain CREATE UNIQUE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260621140000_booking_availability_schedules/migration.sql:92 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260802010000_add_last_inbox_health_email_at/migration.sql:5 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260828013000_meeting_recording_account_ownership/migration.sql:57 real: plain CREATE INDEX on a table created in an earlier migration
- elie222/inbox-zero apps/web/prisma/migrations/20260911190000_upgrade_better_auth_17/migration.sql:415 real: plain CREATE INDEX on a table created in an earlier migration

### `postgres-migrations-practice-concurrently-inside-transaction`

Final rule: no matches in the 10 repositories.

Before the `migrate deploy` script gate (round 1 matcher, run on a copy of civitai's migrations), all
`ConcurrentlyInPrismaBatch`, all `not: applied by hand, statement by statement (CONTRIBUTING.md "We do not use prisma
migrate deploy"); no package.json script runs migrate deploy` (now exempt):
civitai/civitai `packages/civitai-db-schema/prisma/migrations/` +
- 20251117182613_post_metrics_denormalization/migration.sql:32, :36, :40 not: hand-applied (ALTER, trigger function, three concurrent builds)
- 20260703120000_add_modelfile_replacedat/migration.sql:8 not: hand-applied; its comment says "apply this statement on its own"
- 20260707120000_block_scope_invocation_nullable_appblock/migration.sql:26 not: hand-applied
- 20260715120000_w13_app_listing_comments/migration.sql:74 not: hand-applied
- 20260805120000_mod_activity_append_only/migration.sql:18, :22 not: hand-applied
- 20260813120000_placement_metric_counted_at/migration.sql:57 not: hand-applied
- 20260814120000_chat_member_filtered_at/migration.sql:12 not: hand-applied
- 20260816120000_drop_permanent_home_block_clones/migration.sql:28 not: hand-applied (its later BEGIN is a PL/pgSQL body)
- 20260819010000_model_file_hash_redundancy_cleanup/migration.sql:28 not: hand-applied
- 20260824140000_comment_user_id_indexes/migration.sql:34, :36 not: hand-applied
- 20260901220000_placement_submission_uniqueness_and_undelivered_idx/migration.sql:32, :59 not: hand-applied
- 20260911160000_model_associations_unique_index/migration.sql:111 not: hand-applied; comment "Cannot run inside a transaction block"
- 20260917230000_post_block_published_app_id_index_predicate/migration.sql:73, :75 not: hand-applied
- 20260918120000_user_cosmetic_shop_purchases_shop_item_id_index/migration.sql:67 not: hand-applied
- 20260921120000_model_version_generator_loaded_boolean/migration.sql:14 not: hand-applied
- 20261006210000_bounty_payout_markers/migration.sql:16 not: hand-applied
- 20261010120000_model_hot_score/migration.sql:365, :369 not: hand-applied, after `SET lock_timeout = 0`
- 20261011120000_feedback_app_listing/migration.sql:162 not: hand-applied (after its `BEGIN … COMMIT`, not inside)
- 20261012120000_crucible_comments/migration.sql:21 not: hand-applied
- 20261015120000_event_points/migration.sql:19 not: hand-applied
- 20261022120000_article_scheduled_at/migration.sql:13 not: hand-applied
- 20261101120000_thread_entity_fk_cascade/migration.sql:117 not: hand-applied
- 20261122120000_crucible_event_tags/migration.sql:11 not: hand-applied
- 20261125120000_comment_v3_created_at_indexes/migration.sql:9, :11 not: hand-applied; comment "run each statement on its own"
- 20261126120000_cosmetic_shop_item_event_tags/migration.sql:11 not: hand-applied
- 20261127120000_storyline_thread/migration.sql:33 not: hand-applied (before its `BEGIN … COMMIT`)
- 20261130120000_worker_identity/migration.sql:81 not: hand-applied
- 20261206120000_model_created_updated_at_indexes/migration.sql:16, :18 not: hand-applied
- 20261211120000_shop_artwork_key_indexes/migration.sql:8, :10, :12 not: hand-applied
- 20261218120000_app_block_storage_deleted_at/migration.sql:32 not: hand-applied
- 20261227140000_evidence_hold_owner_held_indexes/migration.sql:8, :11 not: hand-applied
- 20261231130000_resource_review_post_id/migration.sql:39, :43 not: hand-applied

Each of these would fail under `prisma migrate deploy` with Prisma ^6.3.0 (civitai's root package.json), so the
matcher read the runner semantics right; the repository simply does not use that runner. The case
`manual/` (a Prisma repository with no migrate script) pins the exemption.

### `postgres-migrations-practice-constraint-on-existing-table-not-valid`

Each is `real`: an ALTER TABLE adding a foreign key or CHECK without NOT VALID to a table created (or renamed into, as
inbox-zero `20260621140000:101` `BookingWindow` → `AvailabilityWindow`) by an earlier migration. Listed as
`<migration timestamp>:<lines>` under each repository's migrations folder; a line listed twice holds a CHECK and a
foreign key in one statement (langfuse `20261013120000:6`) or two foreign keys (langfuse `20261008120000:112`,
inbox-zero `20260404103000:40`). Possibly-same-deploy tables (created hours or a day earlier, which a Git base would
exempt when both are pending): trigger.dev `20230425122235:9`, `20230606120402:5`, `20240428150144:16`, langfuse
`20261117120000:15`.

- documenso/documenso `packages/prisma/migrations`: 20230411134605:5; 20230912011344:23; 20230920052232:14; 20230922121421:52; 20231013012902:5; 20231205000309:5; 20231220124343:5; 20231221101005:19,30,61,64,67; 20240205040421:147,150,176; 20240206051948:5; 20240220115435:25; 20240221055920:6; 20240226035048:5; 20240227023747:5; 20240424072655:14,17,20,23; 20240530055436:42; 20240627050809:16,19; 20250318072444:39; 20250324135010:26,29; 20250404111643:20; 20250407124054:8,11; 20250522054050:454,457,496,499; 20250710050651:82,85; 20250714060452:8,11; 20250807011918:69; 20250918070643:57; 20250918070645:455,458,461,464,467,470; 20260408052212:8; 20260604143030:25,28
- triggerdotdev/trigger.dev `internal-packages/database/prisma/migrations`: 20230102175304:8,11; 20230216184843:5; 20230303131806:5; 20230306122326:5; 20230306150053:5; 20230421171344:5; 20230425122235:9; 20230427131511:5; 20230428092422:86,89,92,95; 20230516121549:21; 20230517153909:5; 20230517154918:5; 20230525110458:5; 20230525110838:5; 20230525123241:5; 20230525124804:5; 20230602155844:5; 20230605123159:16; 20230605155714:12; 20230606120402:5; 20230615152126:203; 20230616083056:32; 20230705151109:18; 20230707105750:5; 20230707122412:5; 20230712132529:5; 20231102143530:6,9; 20231109122305:5; 20231122210707:24,30; 20231122212600:14,17; 20240123193905:91; 20240126130139:5; 20240126204658:5; 20240311135706:5; 20240312131122:5; 20240322172035:21,24; 20240412151157:14,17; 20240415134559:17; 20240417142604:8,11; 20240424210540:5; 20240428142050:25; 20240428150144:16; 20240810090402:24; 20240815123344:9; 20240920085046:10,13,16,19; 20250103152909:222,225; 20250109131442:10,13; 20250109173506:8; 20250320085824:10; 20250705082744:23
- langfuse/langfuse `packages/shared/prisma/migrations`: 20230809132331:22; 20231009095917:17,20,23,26,29; 20231130003317:34; 20231230151856:26; 20240124140443:18; 20240326114211:6; 20240404203640:5; 20240512155022:11; 20240523142610:2; 20240718004923:62,64,66,68; 20240807111359:33,126,141,153; 20250108220721:13; 20250410145712:13; 20250519093328:18,21; 20250523120545:3; 20250711105322:6; 20251127105316:55; 20260706000000:9,12; 20260707170000:5; 20261008120000:112,112; 20261013120000:6,6; 20261020134835:8; 20261116120000:14; 20261117120000:15,17; 20261120120001:5,12
- Shelf-nu/shelf.nu `packages/database/prisma/migrations`: 20230405165021:21; 20230425074445:5; 20230428101832:8; 20230428104507:5; 20230519100223:45; 20230620122326:18; 20230627161509:8; 20230628140147:5; 20230628141128:5; 20230718155504:8; 20230809145816:5; 20230811130821:25; 20230906120138:8; 20230919162512:47; 20231002153143:29; 20231003170230:14; 20231004120609:5; 20231004124134:14; 20231004133839:14,17,44,45; 20231006131408:12; 20231009125120:5,21; 20231016163353:41,50; 20231024014450:4,20; 20231025012338:8; 20231120193948:9,12; 20231122141222:5; 20240425131533:9; 20240426172829:19; 20240504033325:44,47,50; 20240529142530:19; 20240705125333:5; 20240710071325:6; 20240711140019:8; 20240816120345:38,41,44,47,50,53,56,59,62,65,68,71; 20240816121348:5; 20240816131518:23,26,29,32,35,38; 20240924110940:6; 20250703121610:5; 20250726104744:5; 20251113141031:8; 20251216201234:11,14; 20260211084408:8; 20260331101357:79; 20260430100759:17; 20260521133643:23; 20260522103000:17; 20260525131507:22; 20260527150000:24; 20260702120000:18; 20260805120000:27; 20260831150000:16; 20260925100000:27,30; 20260925140000:24; 20260929120000:53,56,59,62; 20261029130000:11; 20261104120000:6,9
- papermark/papermark `prisma/migrations`: 20231105152632:71,74; 20240313100203:27; 20240327102407:70,73; 20240408000000:24; 20240611000000:44; 20240720000000:22,25,28,31; 20240911000000:79,82,85; 20240921000000:15; 20241020000000:14,17; 20250612000000:45; 20251209000000:5; 20260212000000:8; 20260316000000:9; 20260418000000:86; 20260705150000:101,104; 20260709071336:8,17; 20260820000000:29,34,39
- elie222/inbox-zero `apps/web/prisma/migrations`: 20230804140051:8,11; 20240116235134:35; 20240424111051:48,51; 20240507211259:5; 20241023204900:23; 20250128141602:5; 20250417135524:33; 20250420131728:52,126; 20250429192105:307,310,313,316,319,322,325,328,331,334,337,340,343,346; 20250504061506:101; 20250612142528:45; 20250616122919:91; 20250904131746:78; 20250918194235:37,40; 20251013003655:8,11; 20260311120000:18; 20260325043000:21,26; 20260404103000:40,40; 20260505120000:4; 20260607120000:61; 20260621140000:98,101; 20260630000000:2; 20260828013000:60,66; 20260922120000:14

### `postgres-migrations-practice-set-not-null-scans-table`

Every match was read. "Real" means: the statement runs on a table an earlier migration created (or renamed into
place), so on production data it scans the whole table under ACCESS EXCLUSIVE. All are in applied history (no Git
base in the corpus run); none of the six repositories uses the CHECK … NOT VALID pattern anywhere. Notable: Shelf's
`20261016120000_scan_organization_required` takes `LOCK TABLE "Scan" IN SHARE ROW EXCLUSIVE MODE` and then
`SET NOT NULL` inside `BEGIN … COMMIT`, a deliberate choice the comment explains; the scan under ACCESS EXCLUSIVE is
still real, and the message's escape ("only when the table is known to be small") is the judgement call it asks for.
Many matches add a nullable column, backfill it with `UPDATE`, then `SET NOT NULL` in the same file: the backfill
does not avoid the scan, so these are real too. Two raw `SET NOT NULL` text hits are SQL comments (langfuse
`20240807111359_add_organizations_main_migration/migration.sql:30`, Shelf
`20261016120000_scan_organization_required/migration.sql:9`) and are correctly not matched.

How they were read: each match's statement was located in the file and its table traced to the migration that created (or renamed) it, by script over all 135, and about 20 were read by hand in full (the Shelf, rallly, langfuse and trigger.dev ones quoted above, the multi-line `ALTER TABLE\n  "Project"\nALTER COLUMN …` forms, and the four commented-out hits). The engine also matched two multi-line statements a text grep missed (trigger.dev `20240115160657_add_external_ref_to_projects:15`, `20260115154554_taskscheduleinstance_required_projectid:19`).

- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20221216125550_added_runtime_environment_title/migration.sql:10 real: "RuntimeEnvironment" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20221228101710_add_service_for_external_source/migration.sql:13 real: "ExternalSource" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230103145652_add_idempotency_key_to_workflow_run_steps/migration.sql:15 real: "WorkflowRunStep" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230105173643_add_timestamp_to_steps/migration.sql:14 real: "WorkflowRunStep" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230109195958_response_changed_output_context/migration.sql:6 real: "IntegrationResponse" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230515151445_add_trigger_sources/migration.sql:47 real: "HttpSource" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230616102332_make_trigger_source_integrations_required/migration.sql:8 real: "TriggerSource" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230725161151_pk_api_key_unique_and_non_null/migration.sql:9 real: "RuntimeEnvironment" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20231025091939_trigger_http_endpoint_environment_created/migration.sql:10 real: "RuntimeEnvironment" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240115160657_add_external_ref_to_projects/migration.sql:15 real: "Project" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240319121124_what_migration_is_this/migration.sql:8 real: "TaskEvent" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240428150144_org_integration_non_optional_fields/migration.sql:12 real: "OrganizationIntegration" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240428150144_org_integration_non_optional_fields/migration.sql:13 real: "OrganizationIntegration" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240517164246_bulk_action_item_source_run_id_is_required/migration.sql:8 real: "BulkActionItem" created in an earlier migration; nullable-to-required change
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20260115154554_taskscheduleinstance_required_projectid/migration.sql:19 real: "TaskScheduleInstance" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20260426190818_sessions_as_run_manager/migration.sql:6 real: "Session" created in an earlier migration; nullable-to-required change
- langfuse/langfuse packages/shared/prisma/migrations/20230720172051_tokens_non_null/migration.sql:18 real: "observations" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20230720172051_tokens_non_null/migration.sql:19 real: "observations" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20230720172051_tokens_non_null/migration.sql:20 real: "observations" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- langfuse/langfuse packages/shared/prisma/migrations/20230810191453_project_id_not_null/migration.sql:3 real: "observations" created in an earlier migration; nullable-to-required change
- langfuse/langfuse packages/shared/prisma/migrations/20230924232620_datasets_continued/migration.sql:14 real: "dataset_items" created in an earlier migration; nullable-to-required change
- langfuse/langfuse packages/shared/prisma/migrations/20240326115136_dataset_run_item_traceid_non_null/migration.sql:8 real: "dataset_run_items" created in an earlier migration; nullable-to-required change
- langfuse/langfuse packages/shared/prisma/migrations/20240405124810_prompt_to_json/migration.sql:16 real: "prompts" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240512155020_rename_enum_membership_role_to_project_role/migration.sql:32 real: "audit_logs" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240512155020_rename_enum_membership_role_to_project_role/migration.sql:33 real: "membership_invitations" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240512155020_rename_enum_membership_role_to_project_role/migration.sql:34 real: "project_memberships" existed before (renamed into place or created under another spelling in an earlier migration); column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240512155022_scores_non_null_and_add_fk_project_id/migration.sql:8 real: "scores" created in an earlier migration; nullable-to-required change
- langfuse/langfuse packages/shared/prisma/migrations/20240611105521_llm_api_keys_custom_endpoints/migration.sql:13 real: "llm_api_keys" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240611105521_llm_api_keys_custom_endpoints/migration.sql:22 real: "eval_templates" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240718004923_datasets_tables_add_projectid_composite_key/migration.sql:46 real: "dataset_items" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240718004923_datasets_tables_add_projectid_composite_key/migration.sql:51 real: "dataset_runs" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240718004923_datasets_tables_add_projectid_composite_key/migration.sql:56 real: "dataset_run_items" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:115 real: "audit_logs" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:116 real: "audit_logs" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:157 real: "membership_invitations" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:158 real: "membership_invitations" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:31 real: "projects" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20251127105316_add_pricing_tiers/migration.sql:52 real: "prices" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20260619120000_revert_slack_pending_install_columns/migration.sql:18 real: "slack_integrations" created in an earlier migration; nullable-to-required change
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:77 real: "role_assignments" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:78 real: "role_assignments" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20261008120000_reshape_role_assignments/migration.sql:79 real: "role_assignments" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- langfuse/langfuse packages/shared/prisma/migrations/20261116120000_add_model_scope/migration.sql:21 real: "models" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230510135044_make_raw_qr_id_required/migration.sql:8 real: "Scan" created in an earlier migration; nullable-to-required change
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230919162512_expand_custom_fields/migration.sql:44 real: "AssetCustomFieldValue" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231002153143_org_id_assets/migration.sql:26 real: "Asset" existed before (renamed into place or created under another spelling in an earlier migration); backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231004124134_make_organization_required_for_location/migration.sql:11 real: "Location" created in an earlier migration; nullable-to-required change
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231004133839_cat_tag_add_org_id/migration.sql:40 real: "Tag" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231004133839_cat_tag_add_org_id/migration.sql:41 real: "Category" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231006131408_org_id_custom_field/migration.sql:11 real: "CustomField" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231009125120_org_id_qr/migration.sql:18 real: "Qr" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231016163353_add_org_roles/migration.sql:17 real: "Invite" created in an earlier migration; nullable-to-required change
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231024014450_org_to_img/migration.sql:17 real: "Image" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231118003409_bookings/migration.sql:12 real: "Asset" existed before (renamed into place or created under another spelling in an earlier migration); column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231122141222_fix_team_member_organization_relation/migration.sql:13 real: "TeamMember" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231204175427_booking_name_required/migration.sql:3 real: "Booking" created in an earlier migration; nullable-to-required change
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20251120080159_make_booking_from_and_to_required_fields/migration.sql:17 real: "Booking" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20251120080159_make_booking_from_and_to_required_fields/migration.sql:18 real: "Booking" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260409100000_rename_asset_to_booking_to_booking_asset/migration.sql:23 real: "BookingAsset" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20261016120000_scan_organization_required/migration.sql:34 real: "Scan" created in an earlier migration; backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- documenso/documenso packages/prisma/migrations/20230907080056_add_created_at_and_updated_at_columns/migration.sql:15 real: "Document" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20230907080056_add_created_at_and_updated_at_columns/migration.sql:19 real: "Document" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20230912011344_reverse_document_data_relation/migration.sql:14 real: "Document" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20230918111438_update_subscription_constraints_and_columns/migration.sql:14 real: "Subscription" created in an earlier migration; nullable-to-required change
- documenso/documenso packages/prisma/migrations/20230922121421_fix_document_meta_schema/migration.sql:46 real: "DocumentMeta" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20231206073509_add_multple_subscriptions/migration.sql:20 real: "Subscription" created in an earlier migration; nullable-to-required change
- documenso/documenso packages/prisma/migrations/20231206073509_add_multple_subscriptions/migration.sql:21 real: "Subscription" created in an earlier migration; nullable-to-required change
- documenso/documenso packages/prisma/migrations/20240209023519_add_document_audit_logs/migration.sql:15 real: "Field" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20240327074701_add_secondary_verification_id/migration.sql:15 real: "VerificationToken" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20240418140819_remove_impossible_field_optional_states/migration.sql:11 real: "Field" created in an earlier migration; nullable-to-required change
- documenso/documenso packages/prisma/migrations/20240530055436_add_template_direct_links/migration.sql:18 real: "Document" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:216 real: "Folder" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:219 real: "Webhook" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:222 real: "ApiToken" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:225 real: "Document" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:267 real: "TeamGlobalSettings" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:271 real: "Template" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:63 real: "Subscription" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:884 real: "Team" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:885 real: "Team" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:886 real: "Subscription" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:91 real: "User" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250710050651_add_email_domains/migration.sql:31 real: "OrganisationGlobalSettings" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250807011918_add_organisation_sso_portal/migration.sql:72 real: "Organisation" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:394 real: "DocumentAuditLog" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:398 real: "DocumentShareLink" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:403 real: "Field" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:404 real: "Field" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:409 real: "Recipient" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20250918070645_add_envelopes/migration.sql:413 real: "TemplateDirectLink" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20251023021214_add_envelope_item_limits/migration.sql:19 real: "SubscriptionClaim" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20251023021214_add_envelope_item_limits/migration.sql:20 real: "OrganisationClaim" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20260922002213_add_envelope_contents/migration.sql:18 real: "OrganisationClaim" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20260922002213_add_envelope_contents/migration.sql:19 real: "OrganisationClaim" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20260922002213_add_envelope_contents/migration.sql:22 real: "SubscriptionClaim" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- documenso/documenso packages/prisma/migrations/20260922002213_add_envelope_contents/migration.sql:23 real: "SubscriptionClaim" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20220623175037_remove_links_model/migration.sql:13 real: "polls" existed before (renamed into place or created under another spelling in an earlier migration); column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20220623175037_remove_links_model/migration.sql:14 real: "polls" existed before (renamed into place or created under another spelling in an earlier migration); column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20230329173551_options_refactor/migration.sql:34 real: "options" existed before (renamed into place or created under another spelling in an earlier migration); column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20231205043530_poll_status/migration.sql:40 real: "polls" existed before (renamed into place or created under another spelling in an earlier migration); column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20240224033808_option_start_time/migration.sql:26 real: "options" existed before (renamed into place or created under another spelling in an earlier migration); column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20250217103625_make_subscription_fields_required/migration.sql:13 real: "subscriptions" created in an earlier migration; nullable-to-required change
- lukevella/rallly packages/database/prisma/migrations/20250217103625_make_subscription_fields_required/migration.sql:14 real: "subscriptions" created in an earlier migration; nullable-to-required change
- lukevella/rallly packages/database/prisma/migrations/20250217103625_make_subscription_fields_required/migration.sql:15 real: "subscriptions" created in an earlier migration; nullable-to-required change
- lukevella/rallly packages/database/prisma/migrations/20250217103625_make_subscription_fields_required/migration.sql:16 real: "subscriptions" created in an earlier migration; nullable-to-required change
- lukevella/rallly packages/database/prisma/migrations/20250221111248_add_subscription_interval_enum/migration.sql:16 real: "subscriptions" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20250223160003_set_subscription_status_enum/migration.sql:20 real: "subscriptions" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20250227110115_make_subscription_user_required/migration.sql:21 real: "subscriptions" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20250614115818_migrate_events_to_spaces/migration.sql:15 real: "scheduled_events" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20250730115902_subscription_item_id_non_nullable/migration.sql:8 real: "subscriptions" created in an earlier migration; nullable-to-required change
- lukevella/rallly packages/database/prisma/migrations/20250901121851_add_ics_columns/migration.sql:9 real: "scheduled_events" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20251014054941_better_auth/migration.sql:26 real: "accounts" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20251110113931_non_nullable_space_id_susbcription/migration.sql:5 real: "subscriptions" created in an earlier migration; nullable-to-required change
- lukevella/rallly packages/database/prisma/migrations/20260518184939_add_sheet_and_extend_scheduled_event/migration.sql:9 real: "scheduled_event_invites" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- lukevella/rallly packages/database/prisma/migrations/20260905170000_require_participant_token/migration.sql:8 real: "participants" existed before (renamed into place or created under another spelling in an earlier migration); backfill UPDATE just before, still a full scan under ACCESS EXCLUSIVE
- elie222/inbox-zero apps/web/prisma/migrations/20241027173153_category_filter/migration.sql:11 real: "Category" created in an earlier migration; nullable-to-required change
- elie222/inbox-zero apps/web/prisma/migrations/20250209113928_non_null_email/migration.sql:8 real: "User" created in an earlier migration; nullable-to-required change
- elie222/inbox-zero apps/web/prisma/migrations/20250420131728_email_account_settings/migration.sql:123 real: "CleanupJob" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:222 real: "Category" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:223 real: "CleanupJob" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:224 real: "CleanupThread" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:225 real: "ColdEmail" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:226 real: "EmailMessage" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:227 real: "EmailToken" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:228 real: "ExecutedRule" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:229 real: "Group" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:230 real: "Knowledge" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:231 real: "Label" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:232 real: "Newsletter" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:233 real: "Rule" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:234 real: "ThreadTracker" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20250429192105_mutli_email/migration.sql:50 real: "EmailAccount" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20260404103000_enforce_messaging_channel_ownership/migration.sql:28 real: "Action" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20260621140000_booking_availability_schedules/migration.sql:59 real: "BookingLink" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table
- elie222/inbox-zero apps/web/prisma/migrations/20260828013000_meeting_recording_account_ownership/migration.sql:55 real: "MeetingRecording" created in an earlier migration; column added and backfilled earlier in this file, SET NOT NULL still scans the whole table

### `postgres-migrations-practice-required-column-without-default`

Each was read at the statement (`ALTER TABLE` and the `ADD COLUMN` line) and checked against the migration that
creates the table (earlier migration directory, by Prisma timestamp) and the same file (no earlier CREATE, TRUNCATE
or unconditional DELETE of the table).

- lukevella/rallly packages/database/prisma/migrations/20230615111329_events/migration.sql:8 real (pair): "option_id" NOT NULL, no DEFAULT, on "events" created by 20230614141138_v3 (0d21h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- trycompai/comp packages/db/prisma/migrations/20250403201319_tasks_use_asignee/migration.sql:13 real (pair): "assigneeId" NOT NULL, no DEFAULT, on "Task" created by 20250403163138_init (0d3h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- trycompai/comp packages/db/prisma/migrations/20250403205655_vendor_assignee/migration.sql:16 real (pair): "assigneeId" NOT NULL, no DEFAULT, on "Vendor" created by 20250403163138_init (0d4h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- trycompai/comp packages/db/prisma/migrations/20250417205625_update_task_table_and_comments_and_attachments/migration.sql:47 real: "entityType" NOT NULL, no DEFAULT, on "Comment" created by 20250406004306_added_common_comment_table (11d20h earlier); Prisma's own warning says it fails on a non-empty table
- trycompai/comp packages/db/prisma/migrations/20250417205625_update_task_table_and_comments_and_attachments/migration.sql:53 real: "entityId" NOT NULL, no DEFAULT, on "Task" created by 20250403163138_init (14d4h earlier); Prisma's own warning says it fails on a non-empty table
- trycompai/comp packages/db/prisma/migrations/20250417205625_update_task_table_and_comments_and_attachments/migration.sql:54 real: "entityType" NOT NULL, no DEFAULT, on "Task" created by 20250403163138_init (14d4h earlier); Prisma's own warning says it fails on a non-empty table
- trycompai/comp packages/db/prisma/migrations/20250417205625_update_task_table_and_comments_and_attachments/migration.sql:55 real: "frequency" NOT NULL, no DEFAULT, on "Task" created by 20250403163138_init (14d4h earlier); Prisma's own warning says it fails on a non-empty table
- trycompai/comp packages/db/prisma/migrations/20250509162708_init_framework_editor_tables/migration.sql:8 real (pair): "version" NOT NULL, no DEFAULT, on "FrameworkEditorFramework" created by 20250509160401_added_framework_editor (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- trycompai/comp packages/db/prisma/migrations/20251215033455_add_org_id/migration.sql:30 real: "organizationId" NOT NULL, no DEFAULT, on "BrowserbaseContext" created by 20251211172746_add_browserbase_context (3d10h earlier); Prisma's own warning says it fails on a non-empty table
- elie222/inbox-zero apps/web/prisma/migrations/20230804105315_rule_name/migration.sql:2 real: "name" NOT NULL, no DEFAULT, on "Rule" created by 20230730073019_init (5d3h earlier)
- elie222/inbox-zero apps/web/prisma/migrations/20250203174037_reply_tracker_sent_at/migration.sql:2 real: "sentAt" NOT NULL, no DEFAULT, on "ThreadTracker" created by 20250202092329_reply_tracker (1d8h earlier)
- elie222/inbox-zero apps/web/prisma/migrations/20250227142620_payment_tax/migration.sql:10 real (pair): "taxInclusive" NOT NULL, no DEFAULT, on "Payment" created by 20250227135610_payments (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- elie222/inbox-zero apps/web/prisma/migrations/20250227142620_payment_tax/migration.sql:9 real (pair): "tax" NOT NULL, no DEFAULT, on "Payment" created by 20250227135610_payments (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- elie222/inbox-zero apps/web/prisma/migrations/20250521132820_message_parts/migration.sql:10 real (pair): "parts" NOT NULL, no DEFAULT, on "ChatMessage" created by 20250521104911_chat (0d2h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- elie222/inbox-zero apps/web/prisma/migrations/20250804163003_better_auth/migration.sql:22 real: "updatedAt" NOT NULL, no DEFAULT, on "Session" created by 20230730073019_init (736d8h earlier)
- elie222/inbox-zero apps/web/prisma/migrations/20250804163003_better_auth/migration.sql:37 real: "updatedAt" NOT NULL, no DEFAULT, on "VerificationToken" created by 20230730073019_init (736d8h earlier)
- elie222/inbox-zero apps/web/prisma/migrations/20250918194235_update_org_tables_to_email_account/migration.sql:24 real: "emailAccountId" NOT NULL, no DEFAULT, on "Member" created by 20250904131746_add_sso_and_organizations (14d6h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20251211145507_add_normalized_title/migration.sql:8 real: "normalized_title" NOT NULL, no DEFAULT, on "courses" created by 20251130210506_add_courses (10d17h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20251228144106_simplify_chapters_and_lessons/migration.sql:35 real: "course_id" NOT NULL, no DEFAULT, on "chapters" created by 20251226181942_add_chapters (1d20h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20251228144106_simplify_chapters_and_lessons/migration.sql:36 real: "language" NOT NULL, no DEFAULT, on "chapters" created by 20251226181942_add_chapters (1d20h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20251228144106_simplify_chapters_and_lessons/migration.sql:37 real: "position" NOT NULL, no DEFAULT, on "chapters" created by 20251226181942_add_chapters (1d20h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20251228144106_simplify_chapters_and_lessons/migration.sql:40 real (pair): "chapter_id" NOT NULL, no DEFAULT, on "lessons" created by 20251227183114_add_lessons (0d20h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- zoonk/zoonk packages/db/src/prisma/migrations/20251228144106_simplify_chapters_and_lessons/migration.sql:41 real (pair): "language" NOT NULL, no DEFAULT, on "lessons" created by 20251227183114_add_lessons (0d20h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- zoonk/zoonk packages/db/src/prisma/migrations/20251228144106_simplify_chapters_and_lessons/migration.sql:42 real (pair): "position" NOT NULL, no DEFAULT, on "lessons" created by 20251227183114_add_lessons (0d20h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- zoonk/zoonk packages/db/src/prisma/migrations/20260112212254_refactor_course_suggestions/migration.sql:18 real: "description" NOT NULL, no DEFAULT, on "course_suggestions" created by 20251128215522_init (44d23h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20260112212254_refactor_course_suggestions/migration.sql:19 real: "slug" NOT NULL, no DEFAULT, on "course_suggestions" created by 20251128215522_init (44d23h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20260112212254_refactor_course_suggestions/migration.sql:20 real: "title" NOT NULL, no DEFAULT, on "course_suggestions" created by 20251128215522_init (44d23h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114012512_add_day_of_week_to_daily_progress/migration.sql:8 real: "day_of_week" NOT NULL, no DEFAULT, on "daily_progress" created by 20251231003222_add_activities_steps_progress (14d0h earlier); Prisma's own warning says it fails on a non-empty table
- zoonk/zoonk packages/db/src/prisma/migrations/20260116032149_step_kind_enum/migration.sql:12 real: "kind" NOT NULL, no DEFAULT, on "steps" created by 20251231003222_add_activities_steps_progress (16d2h earlier)
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:261 real: "user_id" NOT NULL, no DEFAULT, on "accounts" created by 20251128215522_init (138d4h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:271 real: "lesson_id" NOT NULL, no DEFAULT, on "activities" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:279 real: "user_id" NOT NULL, no DEFAULT, on "activity_progress" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:281 real: "activity_id" NOT NULL, no DEFAULT, on "activity_progress" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:289 real: "user_id" NOT NULL, no DEFAULT, on "chapter_completions" created by 20260411151357_add_durable_completion_tables (4d11h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:291 real: "chapter_id" NOT NULL, no DEFAULT, on "chapter_completions" created by 20260411151357_add_durable_completion_tables (4d11h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:301 real: "course_id" NOT NULL, no DEFAULT, on "chapters" created by 20251226181942_add_chapters (110d8h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:309 real: "entity_id" NOT NULL, no DEFAULT, on "content_reviews" created by 20260304200616_rename_content_reviews_table (42d6h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:311 real: "user_id" NOT NULL, no DEFAULT, on "content_reviews" created by 20260304200616_rename_content_reviews_table (42d6h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:319 real: "course_id" NOT NULL, no DEFAULT, on "course_alternative_titles" created by 20251227152403_add_alternative_titles (109d11h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:327 real: "course_id" NOT NULL, no DEFAULT, on "course_categories" created by 20251227193851_add_course_categories (109d7h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:335 real: "user_id" NOT NULL, no DEFAULT, on "course_completions" created by 20260411151357_add_durable_completion_tables (4d11h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:337 real: "course_id" NOT NULL, no DEFAULT, on "course_completions" created by 20260411151357_add_durable_completion_tables (4d11h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:351 real: "course_id" NOT NULL, no DEFAULT, on "course_users" created by 20251229032633_add_course_users (107d23h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:353 real: "user_id" NOT NULL, no DEFAULT, on "course_users" created by 20251229032633_add_course_users (107d23h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:371 real: "user_id" NOT NULL, no DEFAULT, on "daily_progress" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:379 real: "organization_id" NOT NULL, no DEFAULT, on "invitations" created by 20251128215522_init (138d4h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:381 real: "inviter_id" NOT NULL, no DEFAULT, on "invitations" created by 20251128215522_init (138d4h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:395 real: "user_id" NOT NULL, no DEFAULT, on "lesson_completions" created by 20260411151357_add_durable_completion_tables (4d11h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:397 real: "lesson_id" NOT NULL, no DEFAULT, on "lesson_completions" created by 20260411151357_add_durable_completion_tables (4d11h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:405 real: "lesson_id" NOT NULL, no DEFAULT, on "lesson_sentences" created by 20260121142124_add_word_sentence_models (84d12h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:407 real: "sentence_id" NOT NULL, no DEFAULT, on "lesson_sentences" created by 20260121142124_add_word_sentence_models (84d12h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:415 real: "lesson_id" NOT NULL, no DEFAULT, on "lesson_words" created by 20260121142124_add_word_sentence_models (84d12h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:417 real: "word_id" NOT NULL, no DEFAULT, on "lesson_words" created by 20260121142124_add_word_sentence_models (84d12h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:427 real: "chapter_id" NOT NULL, no DEFAULT, on "lessons" created by 20251227183114_add_lessons (109d8h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:435 real: "organization_id" NOT NULL, no DEFAULT, on "members" created by 20251128215522_init (138d4h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:437 real: "user_id" NOT NULL, no DEFAULT, on "members" created by 20251128215522_init (138d4h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:457 real: "search_prompt_id" NOT NULL, no DEFAULT, on "search_prompt_suggestions" created by 20260112212254_refactor_course_suggestions (93d5h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:459 real: "course_suggestion_id" NOT NULL, no DEFAULT, on "search_prompt_suggestions" created by 20260112212254_refactor_course_suggestions (93d5h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:473 real: "organization_id" NOT NULL, no DEFAULT, on "sentences" created by 20260121142124_add_word_sentence_models (84d12h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:481 real: "user_id" NOT NULL, no DEFAULT, on "sessions" created by 20251128215522_init (138d4h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:493 real: "user_id" NOT NULL, no DEFAULT, on "step_attempts" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:495 real: "step_id" NOT NULL, no DEFAULT, on "step_attempts" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:503 real: "activity_id" NOT NULL, no DEFAULT, on "steps" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:515 real: "reference_id" NOT NULL, no DEFAULT, on "subscriptions" created by 20251128215522_init (138d4h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:523 real: "user_id" NOT NULL, no DEFAULT, on "user_learning_profiles" created by 20260218030833_personalized_learning_schema (56d23h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:531 real: "user_id" NOT NULL, no DEFAULT, on "user_progress" created by 20251231003222_add_activities_steps_progress (106d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:551 real: "word_id" NOT NULL, no DEFAULT, on "word_pronunciations" created by 20260327000000_scope_translations_to_lessons (20d2h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:559 real: "organization_id" NOT NULL, no DEFAULT, on "words" created by 20260121142124_add_word_sentence_models (84d12h earlier); int-to-uuid key change written as DROP + ADD
- zoonk/zoonk packages/db/src/prisma/migrations/20260821174540_add_lesson_question_idempotency/migration.sql:10 real (pair): "request_fingerprint" NOT NULL, no DEFAULT, on "lesson_questions" created by 20260821164929_add_lesson_questions (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- zoonk/zoonk packages/db/src/prisma/migrations/20260821174540_add_lesson_question_idempotency/migration.sql:11 real (pair): "request_id" NOT NULL, no DEFAULT, on "lesson_questions" created by 20260821164929_add_lesson_questions (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- documenso/documenso packages/prisma/migrations/20230605164015_expire_password_reset_token/migration.sql:8 real (pair): "expiry" NOT NULL, no DEFAULT, on "PasswordResetToken" created by 20230605122017_password_reset (0d4h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- documenso/documenso packages/prisma/migrations/20230829180915_recipient_id_for_user_id/migration.sql:13 real (pair): "recipientId" NOT NULL, no DEFAULT, on "Share" created by 20230829165148_user_sharing_link (0d1h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- documenso/documenso packages/prisma/migrations/20240227031228_add_event_to_webhook_call_model/migration.sql:8 real (pair): "event" NOT NULL, no DEFAULT, on "WebhookCall" created by 20240227015420_add_webhook_call_table (0d1h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- documenso/documenso packages/prisma/migrations/20240227111633_rework_user_profiles/migration.sql:30 real: "id" NOT NULL, no DEFAULT, on "UserProfile" created by 20240220115435_add_public_profile_url_bio (6d23h earlier); Prisma's own warning says it fails on a non-empty table
- documenso/documenso packages/prisma/migrations/20240516030441_add_more_timestamp_columns_for_background_jobs/migration.sql:9 real (pair): "updatedAt" NOT NULL, no DEFAULT, on "BackgroundJob" created by 20240516022236_add_background_job_models (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- documenso/documenso packages/prisma/migrations/20240516034546_update_background_job_task_model/migration.sql:9 real (pair): "name" NOT NULL, no DEFAULT, on "BackgroundJobTask" created by 20240516022236_add_background_job_models (0d1h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- documenso/documenso packages/prisma/migrations/20250217120859_add_session_fields/migration.sql:15 real: "expiresAt" NOT NULL, no DEFAULT, on "Session" created by 20230404095503_initial_migration (685d2h earlier); Prisma's own warning says it fails on a non-empty table
- documenso/documenso packages/prisma/migrations/20250217120859_add_session_fields/migration.sql:17 real: "updatedAt" NOT NULL, no DEFAULT, on "Session" created by 20230404095503_initial_migration (685d2h earlier); Prisma's own warning says it fails on a non-empty table
- documenso/documenso packages/prisma/migrations/20250404111643_add_folder_type_and_template_folders/migration.sql:11 real: "type" NOT NULL, no DEFAULT, on "Folder" created by 20250318072444_add_document_folders (17d3h earlier); Prisma's own warning says it fails on a non-empty table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230223205820_add_firstname_and_lastname_make_username_unique/migration.sql:13 real: "username" NOT NULL, no DEFAULT, on "User" created by 20220318181539_init (342d2h earlier); Prisma's own warning says it fails on a non-empty table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230421135210_adding_relationship_between_note_and_user/migration.sql:8 real (pair): "userId" NOT NULL, no DEFAULT, on "Note" created by 20230421134426_adding_note_model_to_db (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230505115249_add_email_field_to_report/migration.sql:8 real (pair): "email" NOT NULL, no DEFAULT, on "ReportFound" created by 20230505115144_add_report_found_model (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230519100223_changing_item_to_asset_in_schema/migration.sql:25 real: "assetId" NOT NULL, no DEFAULT, on "Note" created by 20230421134426_adding_note_model_to_db (27d20h earlier); Prisma's own warning says it fails on a non-empty table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230519100223_changing_item_to_asset_in_schema/migration.sql:33 real: "assetId" NOT NULL, no DEFAULT, on "ReportFound" created by 20230505115144_add_report_found_model (13d22h earlier); Prisma's own warning says it fails on a non-empty table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230627160243_adding_image_model_and_changing_relationship/migration.sql:11 real: "imageId" NOT NULL, no DEFAULT, on "Location" created by 20230620122326_adding_location_table (7d3h earlier); Prisma's own warning says it fails on a non-empty table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230627172213_adding_relationship_between_user_and_image/migration.sql:8 real (pair): "userId" NOT NULL, no DEFAULT, on "Image" created by 20230627160243_adding_image_model_and_changing_relationship (0d1h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20241217152549_add_organization_in_asset_reminder/migration.sql:8 real (pair): "organizationId" NOT NULL, no DEFAULT, on "AssetReminder" created by 20241217150402_create_schema_for_asset_reminders (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- langfuse/langfuse packages/shared/prisma/migrations/20230518193521_changes/migration.sql:14 real (pair): "type" NOT NULL, no DEFAULT, on "observations" created by 20230518193415_add_observaionts_and_traces (0d0h earlier); Prisma's own warning says it fails on a non-empty table; table only hours old, probably in the same unreleased change: a Git base at review time exempts it, else the message's empty-table clause covers it
- langfuse/langfuse packages/shared/prisma/migrations/20230522092340_add_metrics_and_observation_types/migration.sql:12 real: "type" NOT NULL, no DEFAULT, on "observations" created by 20230518193415_add_observaionts_and_traces (3d13h earlier)
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:47 real: "user_id" NOT NULL, no DEFAULT, on "Account" created by 20230518191501_init (12d1h earlier); Prisma's own warning says it fails on a non-empty table
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:53 real: "updated_at" NOT NULL, no DEFAULT, on "Example" created by 20230518191501_init (12d1h earlier); Prisma's own warning says it fails on a non-empty table
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:58 real: "session_token" NOT NULL, no DEFAULT, on "Session" created by 20230518191501_init (12d1h earlier); Prisma's own warning says it fails on a non-empty table
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:59 real: "user_id" NOT NULL, no DEFAULT, on "Session" created by 20230518191501_init (12d1h earlier); Prisma's own warning says it fails on a non-empty table
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:63 real: "trace_id" NOT NULL, no DEFAULT, on "observations" created by 20230518193415_add_observaionts_and_traces (12d1h earlier); Prisma's own warning says it fails on a non-empty table
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:69 real: "trace_id" NOT NULL, no DEFAULT, on "scores" created by 20230523084523_rename_to_score (7d11h earlier); Prisma's own warning says it fails on a non-empty table
- langfuse/langfuse packages/shared/prisma/migrations/20230530204241_auth_api_ui/migration.sql:72 real: "project_id" NOT NULL, no DEFAULT, on "traces" created by 20230518193415_add_observaionts_and_traces (12d1h earlier); Prisma's own warning says it fails on a non-empty table

### `postgres-migrations-practice-volatile-default-rewrites`

- calcom/cal.diy packages/prisma/migrations/20230629124638_membership_autoinc_id/migration.sql:10 real: "Membership" gets ADD COLUMN "id" SERIAL NOT NULL as its new primary key; full rewrite of an existing table.
- elie222/inbox-zero apps/web/prisma/migrations/20250804163003_better_auth/migration.sql:34 real: "VerificationToken" ADD COLUMN "id" TEXT NOT NULL DEFAULT gen_random_uuid(), then ADD PRIMARY KEY; rewrite of an existing table.
- lukevella/rallly packages/database/prisma/migrations/20260423144927_add_instance_id/migration.sql:8 real: instance_settings (created 20250602) ADD COLUMN instance_id UUID NOT NULL DEFAULT gen_random_uuid(); rewrites, but instance_settings is a one-row table, so the message's keep-if-small clause applies (harmless finding).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:259 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:267 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:277 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:287 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:297 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:307 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:317 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:325 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:333 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:343 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:349 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:359 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:369 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:377 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:387 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:393 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:403 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:413 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:423 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:433 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:443 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:449 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:455 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:465 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:471 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:479 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:491 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:501 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:513 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:521 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:529 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:537 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:543 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:549 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:557 real: Prisma's id-type change, DROP COLUMN "id", ADD COLUMN "id" UUID NOT NULL DEFAULT uuidv7() on an existing table; uuidv7() is volatile, full rewrite (the same statement also drops data, which other rules cover).
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:96 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:104 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:130 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:137 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:145 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:157 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:167 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:175 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai packages/civitai-db-schema/prisma/migrations/20221013194408_int_keys/migration.sql:189 real: Prisma's String-to-Int id change, DROP COLUMN "id", ADD COLUMN "id" SERIAL NOT NULL on an existing table (Account, Image, Model, ModelVersion, Review, ReviewReaction, Session, Tag, User); rewrite.
- civitai/civitai …/20221018202627_add_kv_store/migration.sql:17 real: ModelMetric ADD COLUMN "id" SERIAL NOT NULL as new primary key.
- civitai/civitai …/20221214181035_add_modelfile_id/migration.sql:12 real: ModelFile ADD COLUMN "id" SERIAL NOT NULL.
- civitai/civitai …/20230212204953_api_key_tweaks/migration.sql:5 real: ApiKey ADD COLUMN "id" SERIAL NOT NULL.
- civitai/civitai …/20230613205927_model_article_association/migration.sql:3 real: ModelAssociations ADD COLUMN "id" SERIAL NOT NULL.
- civitai/civitai …/20231130141155_add_club_membershi_id/migration.sql:4 real: ClubMembership ADD COLUMN "id" SERIAL NOT NULL inside BEGIN … COMMIT.
- civitai/civitai …/20240118214315_muted_at/migration.sql:19 real: "User" ADD COLUMN muted boolean GENERATED ALWAYS AS ("mutedAt" IS NOT NULL) STORED; rewrites the User table.
- civitai/civitai …/20260304123827_comic_chapter_early_access/migration.sql:2 real: ComicChapter ADD COLUMN "id" SERIAL (nullable, but the nextval default still fills every row); the file then backfills and sets NOT NULL, which the SERIAL already made pointless.
- civitai/civitai …/20260715120000_w13_app_listing_comments/migration.sql:48 real: app_listings ADD COLUMN IF NOT EXISTS serial_id SERIAL; the authors' comment says it "backfills every existing row via its DEFAULT".
- civitai/civitai …/20261128120000_scan_label_claim_token/migration.sql:8 real: ScanLabelClaim ADD COLUMN token uuid DEFAULT gen_random_uuid(); the authors' comment confirms the rewrite and accepts it because the table is small: exactly the message's keep-if-small clause.

### `postgres-migrations-practice-column-type-change-rewrites`

Old and new types are as the rule's message gives them (old type = latest earlier definition in the history).
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20231215142649_add_checkout_status_2_asset/migration.sql:11 real: "status" on "Asset" from "AssetStatus" to "AssetStatus_new" rewrites the table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20240718142031_remove_team_member_organization_role/migration.sql:10 real: "roles" on "UserOrganization" from "OrganizationRoles"[] to "OrganizationRoles_new"[] rewrites the table
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20240718142031_remove_team_member_organization_role/migration.sql:11 real: "roles" on "Invite" from "OrganizationRoles"[] to "OrganizationRoles_new"[] rewrites the table
- documenso/documenso packages/prisma/migrations/20230621133446_migrate_field_position_to_float/migration.sql:3 real: "positionX" on "Field" from INTEGER to DECIMAL(65,30) rewrites the table
- documenso/documenso packages/prisma/migrations/20230621133446_migrate_field_position_to_float/migration.sql:5 real: "positionY" on "Field" from INTEGER to DECIMAL(65,30) rewrites the table
- documenso/documenso packages/prisma/migrations/20230621133446_migrate_field_position_to_float/migration.sql:7 real: "height" on "Field" from INTEGER to DECIMAL(65,30) rewrites the table
- documenso/documenso packages/prisma/migrations/20230621133446_migrate_field_position_to_float/migration.sql:9 real: "width" on "Field" from INTEGER to DECIMAL(65,30) rewrites the table
- documenso/documenso packages/prisma/migrations/20240227003622_migrate_to_cuids_for_webhooks/migration.sql:10 real: "id" on "Webhook" from SERIAL to TEXT rewrites the table
- documenso/documenso packages/prisma/migrations/20240606060033_add_public_profiles/migration.sql:42 real: "id" on "UserProfile" from INTEGER to TEXT rewrites the table
- documenso/documenso packages/prisma/migrations/20250324135010_model_folder_update_id_type_and_add_pinned_property/migration.sql:14 real: "folderId" on "Document" from INTEGER to TEXT rewrites the table
- documenso/documenso packages/prisma/migrations/20250324135010_model_folder_update_id_type_and_add_pinned_property/migration.sql:20 real: "id" on "Folder" from SERIAL to TEXT rewrites the table
- documenso/documenso packages/prisma/migrations/20250324135010_model_folder_update_id_type_and_add_pinned_property/migration.sql:21 real: "parentId" on "Folder" from INTEGER to TEXT rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20231229221011_remove_summarize_action/migration.sql:10 real: "type" on "Action" from "ActionType" to "ActionType_new" rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20231229221011_remove_summarize_action/migration.sql:11 real: "actions" on "ExecutedRule" from "ActionType"[] to "ActionType_new"[] rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20250804163003_better_auth/migration.sql:11 real: "expires_at" on "Account" from INTEGER to TIMESTAMP(3) rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20250804163003_better_auth/migration.sql:25 real: "emailVerified" on "User" from TIMESTAMP(3) to BOOLEAN rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20251010143722_remove_track_thread_action/migration.sql:19 real: "type" on "Action" from its old type to "ActionType_new" rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20251010143722_remove_track_thread_action/migration.sql:20 real: "type" on "ExecutedAction" from "ActionType" to "ActionType_new" rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20251010143722_remove_track_thread_action/migration.sql:21 real: "actionType" on "ScheduledAction" from "ActionType" to "ActionType_new" rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20260114000000_follow_up_days_to_float/migration.sql:2 real: "followUpAwaitingReplyDays" on "EmailAccount" from INTEGER to DOUBLE PRECISION rewrites the table
- elie222/inbox-zero apps/web/prisma/migrations/20260114000000_follow_up_days_to_float/migration.sql:3 real: "followUpNeedsReplyDays" on "EmailAccount" from INTEGER to DOUBLE PRECISION rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20230620181114_restructure/migration.sql:9 real: "type" on "observations" from "ObservationType" to "ObservationType_new" rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20230731162154_score_value_float/migration.sql:2 real: "value" on "scores" from INTEGER to DOUBLE PRECISION rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20240512155020_rename_enum_membership_role_to_project_role/migration.sql:27 real: "user_project_role" on "audit_logs" from its old type to "ProjectRole" rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20240512155020_rename_enum_membership_role_to_project_role/migration.sql:28 real: "role" on "membership_invitations" from its old type to "ProjectRole" rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20240512155020_rename_enum_membership_role_to_project_role/migration.sql:29 real: "role" on "project_memberships" from its old type to "ProjectRole" rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20240524165931_scores_source_enum_drop_review/migration.sql:10 real: "source" on "scores" from "ScoreSource" to "ScoreSource_new" rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20241015110145_prompts_config_to_JSON/migration.sql:5 real: "config" on "prompts" from JSONB to JSON rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20260521000000_monitor_severity_status_reorder_drop_message/migration.sql:6 real: "severity" on "monitors" from "MonitorSeverity" to "MonitorSeverity_new" rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20260521000000_monitor_severity_status_reorder_drop_message/migration.sql:18 real: "status" on "monitors" from "MonitorStatus" to "MonitorStatus_new" rewrites the table
- langfuse/langfuse packages/shared/prisma/migrations/20260522113010_add_paused_monitor_severity/migration.sql:19 real: "severity" on "monitors" from "MonitorSeverity_new" to "MonitorSeverity" rewrites the table
- lukevella/rallly packages/database/prisma/migrations/20250226173250_remove_paddle_subscriptions/migration.sql:7 real: "status" on "subscriptions" from its old type to "subscription_status_new" rewrites the table
- lukevella/rallly packages/database/prisma/migrations/20250722083454_remove_owner_role/migration.sql:12 real: "role" on "space_members" from "SpaceMemberRole" to "SpaceMemberRole_new" rewrites the table
- lukevella/rallly packages/database/prisma/migrations/20250731100421_update_subscription_interval_enum/migration.sql:10 real: "interval" on "subscriptions" from its old type to "subscription_interval_new" rewrites the table
- lukevella/rallly packages/database/prisma/migrations/20260117124004_update_poll_status_enum/migration.sql:11 real: "status" on "polls" from "poll_status" to "poll_status_new" rewrites the table
- lukevella/rallly packages/database/prisma/migrations/20261015120000_queued_email_kind_to_text/migration.sql:2 real: "kind" on "queued_emails" from "queued_email_kind" to TEXT rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20221227191746_renamed_processed_to_dispatch_for_events/migration.sql:12 real: "status" on "TriggerEvent" from "TriggerEventStatus" to "TriggerEventStatus_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230105115145_remove_interruption_type/migration.sql:13 real: "type" on "WorkflowRunStep" from "WorkflowRunStepType" to "WorkflowRunStepType_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230105115719_remove_interrupted_state_from_runs/migration.sql:14 real: "status" on "WorkflowRun" from "WorkflowRunStatus" to "WorkflowRunStatus_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230105175909_make_ts_a_string/migration.sql:2 real: "ts" on "WorkflowRunStep" from INTEGER to TEXT rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230115194010_add_scheduler_source_model/migration.sql:13 real: "type" on "ExternalSource" from "ExternalSourceType" to "ExternalSourceType_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230119183126_removed_archived_status/migration.sql:11 real: "status" on "Workflow" from "WorkflowStatus" to "WorkflowStatus_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230213154802_simplify_org_template_status/migration.sql:11 real: "status" on "OrganizationTemplate" from "OrganizationTemplateStatus" to "OrganizationTemplateStatus_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230312132000_more_kv_types/migration.sql:10 real: "type" on "WorkflowRunStep" from its old type to "WorkflowRunStepType_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230330132602_use_string_instead_of_big_int/migration.sql:2 real: "ts" on "Task" from BIGINT to TEXT rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230503172156_rename_create_execution_to_create_run/migration.sql:11 real: "action" on "JobEventRule" from "JobEventAction" to "JobEventAction_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230606122534_improve_run_execution_model/migration.sql:12 real: "reason" on "JobRunExecution" from "JobRunExecutionReason" to "JobRunExecutionReason_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230609150822_add_graphile_job_id_string/migration.sql:2 real: "graphileJobId" on "JobRunExecution" from INTEGER to TEXT rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20230614135014_change_endpoint_source_enum/migration.sql:11 real: "source" on "EndpointIndex" from "EndpointIndexSource" to "EndpointIndexSource_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240517105224_remove_test_alert_type/migration.sql:10 real: "alertTypes" on "ProjectAlertChannel" from "ProjectAlertType"[] to "ProjectAlertType_new"[] rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240517105224_remove_test_alert_type/migration.sql:11 real: "type" on "ProjectAlert" from "ProjectAlertType" to "ProjectAlertType_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20240517105224_remove_test_alert_type/migration.sql:12 real: "alertType" on "ProjectAlertStorage" from "ProjectAlertType" to "ProjectAlertType_new" rewrites the table
- triggerdotdev/trigger.dev internal-packages/database/prisma/migrations/20250116115746_rename_blocked_by_waitpoints_to_suspended/migration.sql:10 real: "executionStatus" on "TaskRunExecutionSnapshot" from "TaskRunExecutionStatus" to "TaskRunExecutionStatus_new" rewrites the table
- trycompai/comp packages/db/prisma/migrations/20250403165218_archived_on_its_own/migration.sql:12 real: "status" on "Policy" from "PolicyStatus" to "PolicyStatus_new" rewrites the table
- trycompai/comp packages/db/prisma/migrations/20250417205625_update_task_table_and_comments_and_attachments/migration.sql:33 real: "status" on "Task" from "TaskStatus" to "TaskStatus_new" rewrites the table
- trycompai/comp packages/db/prisma/migrations/20250423154259_make_role_string/migration.sql:6 real: "role" on "Member" from "Role" to TEXT rewrites the table
- trycompai/comp packages/db/prisma/migrations/20250424170518_remove_evidence_artifact_type/migration.sql:10 real: "type" on "Artifact" from "ArtifactType" to "ArtifactType_new" rewrites the table
- trycompai/comp packages/db/prisma/migrations/20250516161852_cast_framework_id_and_requirement_id_to_string/migration.sql:2 real: "frameworkId" on "FrameworkInstance" from "FrameworkId" to TEXT rewrites the table
- trycompai/comp packages/db/prisma/migrations/20250516161852_cast_framework_id_and_requirement_id_to_string/migration.sql:5 real: "requirementId" on "RequirementMap" from "RequirementId" to TEXT rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260121144812_alter_fleet_policy_result_timestamps/migration.sql:2 not: the table was created by the migration one second earlier (20260121144811), so both ship in one deploy and the table is empty; only a Git base shows that
- trycompai/comp packages/db/prisma/migrations/20260121144812_alter_fleet_policy_result_timestamps/migration.sql:4 not: the table was created by the migration one second earlier (20260121144811), so both ship in one deploy and the table is empty; only a Git base shows that
- trycompai/comp packages/db/prisma/migrations/20260217190000_convert_evidence_form_types_to_enum/migration.sql:70 real: "formType" on "EvidenceSubmission" from TEXT to "EvidenceFormType" rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260217190000_convert_evidence_form_types_to_enum/migration.sql:75 real: "evidenceFormType" on "Finding" from TEXT to "EvidenceFormType" rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260305100005_change_organization_role_permissions_to_text/migration.sql:5 real: "permissions" on "organization_role" from JSONB to TEXT rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260602120000_policy_department_to_string/migration.sql:2 real: "department" on "Policy" from "Departments" to TEXT rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260602120000_policy_department_to_string/migration.sql:6 real: "visibleToDepartments" on "Policy" from "Departments"[] to TEXT[] rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260602130000_member_risk_task_department_to_string/migration.sql:3 real: "department" on "Member" from "Departments" to TEXT rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260602130000_member_risk_task_department_to_string/migration.sql:7 real: "department" on "Risk" from "Departments" to TEXT rewrites the table
- trycompai/comp packages/db/prisma/migrations/20260602130000_member_risk_task_department_to_string/migration.sql:11 real: "department" on "Task" from "Departments" to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260113000000_add_generation_status_enum/migration.sql:7 real: "generation_status" on "activities" from VARCHAR(20) to "GenerationStatus" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260113000000_add_generation_status_enum/migration.sql:13 real: "generation_status" on "chapters" from VARCHAR(20) to "GenerationStatus" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260113000000_add_generation_status_enum/migration.sql:19 real: "generation_status" on "course_suggestions" from VARCHAR(20) to "GenerationStatus" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260113000000_add_generation_status_enum/migration.sql:25 real: "generation_status" on "courses" from VARCHAR(20) to "GenerationStatus" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260113000000_add_generation_status_enum/migration.sql:31 real: "generation_status" on "lessons" from VARCHAR(20) to "GenerationStatus" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260113170834_add_kind_enums/migration.sql:14 real: "kind" on "activities" from VARCHAR(30) to "ActivityKind" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260113170834_add_kind_enums/migration.sql:22 real: "kind" on "lessons" from VARCHAR(20) to "LessonKind" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:22 real: "id" on "activities" from SERIAL to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:27 real: "id" on "activity_progress" from SERIAL to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:28 real: "activity_id" on "activity_progress" from INTEGER to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:33 real: "id" on "daily_progress" from SERIAL to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:38 real: "id" on "step_attempts" from SERIAL to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:39 real: "step_id" on "step_attempts" from INTEGER to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:44 real: "id" on "steps" from SERIAL to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260114010522_use_bigint/migration.sql:45 real: "activity_id" on "steps" from INTEGER to BIGINT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260121142124_add_word_sentence_models/migration.sql:11 real: "kind" on "activities" from "ActivityKind" to "ActivityKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260303161226_on_demand_review/migration.sql:18 real: "kind" on "activities" from "ActivityKind_new" to "ActivityKind" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260303225204_remove_mechanics_activity_kind/migration.sql:7 real: "kind" on "activities" from "ActivityKind" to "ActivityKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260312000000_rename_story_to_practice/migration.sql:5 real: "kind" on "activities" from its old type to "ActivityKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260312100000_remove_background_examples_activities/migration.sql:7 real: "kind" on "activities" from its old type to "ActivityKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260317000000_remove_language_practice/migration.sql:7 real: "kind" on "activities" from its old type to "ActivityKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260330160639_remove_challenge/migration.sql:12 real: "kind" on "activities" from its old type to "ActivityKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:88 real: "id" on "accounts" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:89 real: "user_id" on "accounts" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:94 real: "organization_id" on "activities" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:97 real: "user_id" on "activity_progress" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:100 real: "user_id" on "chapter_completions" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:103 real: "organization_id" on "chapters" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:106 real: "user_id" on "content_reviews" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:109 real: "user_id" on "course_completions" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:112 real: "user_id" on "course_users" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:115 real: "organization_id" on "courses" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:116 real: "user_id" on "courses" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:119 real: "user_id" on "daily_progress" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:124 real: "id" on "invitations" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:125 real: "organization_id" on "invitations" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:126 real: "inviter_id" on "invitations" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:133 real: "id" on "jwks" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:138 real: "user_id" on "lesson_completions" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:141 real: "organization_id" on "lessons" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:146 real: "id" on "members" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:147 real: "organization_id" on "members" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:148 real: "user_id" on "members" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:155 real: "id" on "organizations" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:162 real: "id" on "rate_limits" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:167 real: "organization_id" on "sentences" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:172 real: "id" on "sessions" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:173 real: "user_id" on "sessions" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:178 real: "user_id" on "step_attempts" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:183 real: "id" on "subscriptions" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:188 real: "user_id" on "user_learning_profiles" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:191 real: "user_id" on "user_progress" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:196 real: "id" on "users" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:203 real: "id" on "verifications" from SERIAL to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260416000743_auth_string_ids/migration.sql:208 real: "organization_id" on "words" from INTEGER to TEXT rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260427175822_remove_applied_activities/migration.sql:15 real: "kind" on "activities" from its old type to "ActivityKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260427175822_remove_applied_activities/migration.sql:25 real: "kind" on "steps" from "StepKind" to "StepKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260428052713_move_content_to_lessons/migration.sql:5 real: "kind" on "lessons" from "LessonKind" to "LessonKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260616170301_update_course_mode_enum/migration.sql:11 real: "mode" on "courses" from "CourseMode" to "CourseMode_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260714144021_remove_product_course_format/migration.sql:9 real: "format" on "courses" from "CourseFormat" to "CourseFormat_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260714144021_remove_product_course_format/migration.sql:10 real: "course_format" on "course_prompts" from "CourseFormat" to "CourseFormat_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20260822192802_optimize_lesson_questions/migration.sql:23 real: "context_kind" on "lesson_questions" from "LessonQuestionContextKind" to "LessonQuestionContextKind_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20261027220000_simplify_live_calls/migration.sql:8 real: "end_reason" on "live_call_connections" from "LiveCallEndReason" to "LiveCallEndReason_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20261027220000_simplify_live_calls/migration.sql:18 real: "state" on "live_call_connections" from "LiveCallState" to "LiveCallState_new" rewrites the table
- zoonk/zoonk packages/db/src/prisma/migrations/20261105025438_certificates_per_level_and_plus/migration.sql:7 real: "kind" on "certificates" from "CertificateKind" to "CertificateKind_new" rewrites the table

### `postgres-migrations-practice-lock-timeout`

- triggerdotdev/trigger.dev internal-packages/run-ops-database/prisma/migrations/20260829120000_add_task_run_gates/migration.sql:2 real: `ALTER TABLE "TaskRun" ADD COLUMN IF NOT EXISTS "gates" JSONB;` on the hottest table of the app with no lock_timeout; the repository's other migration set (internal-packages/database) starts such migrations with "-- Fail fast instead of queueing behind a long txn/VACUUM for the ACCESS EXCLUSIVE lock." / `SET lock_timeout = '5s';`, so this one missed its own convention. No lock_timeout in config (only in runtime code and tests).
- langfuse/langfuse packages/shared/prisma/migrations/20261120140000_add_gateway_spend_limit_warning_thresholds/migration.sql:3 real: `ALTER TABLE "gateway_spend_limits" ADD COLUMN …` on a table created by 20261116120000_add_gateway_spend_limits; no timeout, while the repository's 20260904151000_add_ai_gateway_control_plane migration writes `SET LOCAL lock_timeout = '5s';` with the comment "we fail fast instead of parking a SHARE ROW EXCLUSIVE request at the head of the lock queue, where every subsequent writer would pile up behind it." (The first statement, `ALTER TYPE … ADD VALUE`, locks no table and is rightly skipped.)
- elie222/inbox-zero apps/web/prisma/migrations/20261123120000_add_mail_group_by_sender/migration.sql:2 real: `ALTER TABLE "EmailAccount" ADD COLUMN "mailGroupBySender" BOOLEAN NOT NULL DEFAULT true;` on a table created by an earlier migration; the rule's checkout walk found no lock_timeout in the config files, scripts or migrations.
- lukevella/rallly packages/database/prisma/migrations/20261120120000_add_space_usage/migration.sql:18 real: the new table "space_usage" is created without being reported (line 5), and the finding is at its FK `REFERENCES "spaces"("id")`, which takes SHARE ROW EXCLUSIVE on the existing "spaces" table; the rule's checkout walk found no lock_timeout in the config files, scripts or migrations.

### `postgres-migrations-practice-rename-as-drop-and-add`

Final round (revision with every exemption). `old → new [type] table` as the message names them.

Shelf-nu/shelf.nu (packages/database/prisma/migrations)
- 20230519100223_changing_item_to_asset_in_schema/migration.sql:24 itemId → assetId (Note) real: the Item model became Asset (table renamed one migration earlier); every note loses its asset link
- …:28 itemId → assetId (Qr) real: same rename
- …:32 itemId → assetId (ReportFound) real: same rename
- 20230223205820_add_firstname_and_lastname_make_username_unique/migration.sql:10 name → firstName (User) real: every user's name is dropped while firstName/lastName are added empty; a backfill (the message's UPDATE option) keeps it
- 20230510134953_replace_deleted_qr_id_with_raw_qr_id/migration.sql:8 deletedQrId → rawQrId (Scan) not: a different value ("replace"), not a rename

documenso/documenso (packages/prisma/migrations)
- 20241017035042_rename_typed_signature_option/migration.sql:8 enabledTypedSignature → typedSignatureEnabled (DocumentMeta) real: the migration's own name says rename; every document's setting resets to false
- 20250217120859_add_session_fields/migration.sql:13 expires → expiresAt (Session) real: rename; sessions lose their expiry
- 20240227111633_rework_user_profiles/migration.sql:22 profileURL → url (User) real: profile URLs dropped
- …:27 profileBio → bio (UserProfile) real: bios dropped

elie222/inbox-zero (apps/web/prisma/migrations)
- 20260911190000_upgrade_better_auth_17/migration.sql:21 type → applicationType (OauthClient) not: better-auth upgrade replaces the field; values differ
- 20250324222007_skipconversation/migration.sql:2 skipConversations → skipConversation (CleanupJob) real: plural → singular rename

trycompai/comp (packages/db/prisma/migrations)
- 20250417205625_update_task_table_and_comments_and_attachments/migration.sql:50 dueDate → lastCompletedAt (Task) not: different meaning
- …:51 relatedId → entityId (Task) real: relatedType → entityType changes with it; the task's link is the same value
- 20250424165055_update_onboarding_to_use_tasks/migration.sql:8 evidence → tasks (Onboarding) real: the onboarding step was renamed; progress resets

langfuse/langfuse (packages/shared/prisma/migrations)
- 20230620181114_restructure/migration.sql:26 attributes → metadata (traces) real: attributes became metadata
- 20230530204241_auth_api_ui/migration.sql:46 userId → user_id (Account) real: camelCase → snake_case switch
- …:50 createdAt → created_at (Example) real: same
- …:51 updatedAt → updated_at (Example) real: same
- …:56 sessionToken → session_token (Session) real: same
- …:57 userId → user_id (Session) real: same
- …:62 traceId → trace_id (observations) real: same
- …:66 observationId → observation_id (scores) real: same
- …:67 traceId → trace_id (scores) real: same
- 20230522131516_default_timestamp/migration.sql:8 createdAt → timestamp (metrics) real: the metric time column renamed; old times lost
- 20230518193521_changes/migration.sql:12 parentObservationId → parent_observation_id (observations) real: snake_case rename

calcom/cal.diy (packages/prisma/migrations)
- 20240307200336_rename_dsync_org_id_to_organization_id/migration.sql:15 orgId → organizationId (DSyncData) real: rename in the migration's name
- 20240307203026_rename_team_group_mapping_org_id_to_organization_id/migration.sql:9 orgId → organizationId (DSyncTeamGroupMapping) real: same
- 20220113145333_rename_column_sc_address_to_smart_contract_address/migration.sql:2 scAddress → smartContractAddress (EventType) real: same

triggerdotdev/trigger.dev (internal-packages/database/prisma/migrations)
- 20240611113047_change_schedules_limit_to_maximum_schedules_limit/migration.sql:8 maximumScheduleInstancesLimit → maximumSchedulesLimit real: rename (each org's custom limit lost)
- 20230504152611_rename_shadow_to_internal/migration.sql:8 shadow → internal real
- 20230614110359_rename_deploy_to_index/migration.sql:8 deployHookIdentifier → indexingHookIdentifier real
- 20230605123159_renamed_credentials_reference_to_custom_client_reference/migration.sql:11 credentialsReferenceId → customClientReferenceId real
- 20230329133809_rename_endpoint_name_to_slug/migration.sql:13 name → slug real
- 20230425162613_add_action_identifier_to_event_rule/migration.sql:20 taskId → actionIdentifier not: a new key, not the task id
- 20221227191746_renamed_processed_to_dispatch_for_events/migration.sql:20 processedAt → dispatchedAt real
- 20230330125238_rename_finished_at/migration.sql:8 finishedAt → completedAt (Task) real
- 20250623141255_bulk_action_group_counts_completed_at/migration.sql:14 reason → name not: unrelated
- 20240111143407_encrypt_the_personal_access_token_in_the_database/migration.sql:13 token → encryptedToken not: deliberate (plaintext tokens dropped; SQL cannot encrypt them)
- 20221216140358_rename_data_to_input/migration.sql:9 data → input real
- 20230330125349_rename_finished_at_on_executions/migration.sql:8 finishedAt → completedAt (Execution) real
- 20230306132827_added_more_columns_to_deployment/migration.sql:11 imageIdentifier → imageId real
- 20221227182358_rename_rule_to_filter/migration.sql:9 rule → filter real
- 20230609095323_rename_elements_to_properties/migration.sql:10 elements → properties (JobRun) real
- …:14 elements → properties (JobVersion) real
- …:18 elements → properties (Task) real
- 20230517105823_rename_connection_metadata/migration.sql:9 connectionMetadata → metadata real
- 20230210111519_change_access_token_url_column_name/migration.sql:9 accessTokensUrls → accessTokensUrl real
- 20260928120000_webhook_shared_endpoints/migration.sql:14 routingTarget → routingTargets real: singular → plural of the same JSONB
- 20230329132829_changed_dispatched_at_to_delivered_at/migration.sql:8 dispatchedAt → deliveredAt real
- 20230404095457_rename_display_properties_to_elements/migration.sql:8 displayProperties → elements real
- 20250320152806_waitpoint_renamed_tags/migration.sql:9 waitpointTags → tags real
- 20230615152126_revamp_integration_schema/migration.sql:71 apiConnectionId → connectionId not: the revamp points it at a new table (no FK constraint read for the old column)

zoonk/zoonk (packages/db/src/prisma/migrations)
- 20260327192910_rename_distractor_unsafe_metadata/migration.sql:10 alternative_translations → distractor_unsafe_translations (lesson_sentences) real: rename in the migration's name
- …:14 same (lesson_words) real
- …:18 alternative_sentences → distractor_unsafe_sentences (sentences) real
- 20261027220000_simplify_live_calls/migration.sql:44 provider_started_at → started_at real: the start time kept under a shorter name
- 20260328075924_simplify_language_distractors/migration.sql:7 distractor_unsafe_translations → distractors (lesson_words) real: the list carried under a new name

customermates/customermates (prisma/migrations)
- 20260625161605_unipile_v2_messaging/migration.sql:13 backfillCheckpoint → folders not: unrelated JSONB
- 20260515065222_add_messaging/migration.sql:396 emailsText → avatarUrl not: unrelated TEXT, paired across statements

civitai/civitai (packages/civitai-db-schema/prisma/migrations)
- 20240229155733_vault_item_update_field/migration.sql:1 detailsSizeKB → detailsSizeKb real: a case-only rename, Prisma's drop+add
- …:2 imagesSizeKB → imagesSizeKb real
- …:3 modelSizeKB → modelSizeKb real
- 20240409202625_add_shop_item_unit_amount/migration.sql:2 unitValue → unitAmount (CosmeticShopItem) real: prices lost; NOT NULL add fails on a non-empty table anyway
- …:6 unitValue → unitAmount (UserCosmeticShopPurchases) real
- 20241016185054_announcement_disabled/migration.sql:8 enabled → disabled not: inverted meaning; a rename would be wrong (the message's "unrelated column" clause applies)
- 20241002212355_resupport_stripe_connect_on_payment_configuration/migration.sql:9 chargesEnabled → stripePaymentsEnabled not: new provider flag
- 20241003192438_model_flag_poi_name_column/migration.sql:2 nameNsfw → poiName not: unrelated flag

lukevella/rallly, AienTech/dossier: 0 matches.

Removed by the rounds (each now a near-miss case): documenso `profileURL` → `bio` (claimed by profileBio); `userId` →
`recipientId`/`emailAccountId`, comp `ownerId`/`userId` → `assigneeId`, `connectionId` → `organizationId` (foreign key
retargeted); trigger.dev `queueName` → `queueId`, `versionId` → `version`, `icon` → `definitionId`, `key` →
`triggerSourceId`, `connectionKey` → `runConnectionId` (id vs name); zoonk `simplify_live_calls`, trigger.dev
`schema_redesign`/`restructure_task_runs` many → one (no one-to-one match); inbox-zero `public` →
`backchannelLogoutSessionRequired`.

### `postgres-migrations-practice-dropped-column-still-used`

Round 1 (8 repos, before the fixes; distinct lines; the engine's .ts/.tsx/.js lanes repeat each finding):
- documenso/documenso packages/prisma/migrations/20250522054050_add_organisations/migration.sql:878 not: "teamId" in get-users-stats.ts:32 is `"Envelope"."teamId"`, another table's column (fixed: qualified by another name)
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230627160243_adding_image_model_and_changing_relationship/migration.sql:10 not: `am.image` (AssetModel alias) and a comment in query.server.ts (fixed: qualifier, comments)
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20230223205820_add_firstname_and_lastname_make_username_unique/migration.sql:10 not: `'name'` JSON keys and `bk.name` in query.server.ts (fixed: string literals, qualifier)
- Shelf-nu/shelf.nu packages/database/prisma/migrations/20260519143054_add_asset_location_pivot/migration.sql:135 not: `al."locationId"` and `AS "locationId"` in home.tsx:344 (fixed: qualifier, AS output name)
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:82 not: `DROP COLUMN "role",\nADD COLUMN "role" "Role"` type change (fixed: same-statement re-add)
- langfuse/langfuse packages/shared/prisma/migrations/20240807111359_add_organizations_main_migration/migration.sql:149 not: same, "project_role"
- langfuse/langfuse packages/shared/prisma/migrations/20260521000000_monitor_severity_status_reorder_drop_message/migration.sql:26 not: "message" only in a `-- … into a message queue event` comment in scheduler.ts (fixed: comments)
- lukevella/rallly packages/database/prisma/migrations/20250402100733_remove_closed_column/migration.sql:8 not: `SET status = 'closed'` string literal in mutations.ts:425 (fixed: string literals)
- elie222/inbox-zero apps/web/prisma/migrations/20250227135758_processor_type_enum/migration.sql:11 not: type change, drop + add in one statement (fixed)
- elie222/inbox-zero apps/web/prisma/migrations/20260907190000_mail_split_multi_value/migration.sql:5 not: "value" is MailSplitFilter's (`INSERT INTO "MailSplitFilter" ("id", "kind", "value", …)` beside `FROM "MailSplit"`), `filter."value"` (fixed: qualifier; a bare name in a query that also names another table declaring that column is skipped)
- elie222/inbox-zero apps/web/prisma/migrations/20260910000000_split_inbox_filters/migration.sql:43 not: same, "kind"
- zoonk/zoonk packages/db/src/prisma/migrations/20260416024906_use_uuid_v7_auth_ids/migration.sql:258, 260, 296, 324, 326, 358, 360, 362, 368, 370, 376, 378, 380, 422, 426, 432, 434, 436, 442, 448, 470, 472, 478, 480, 482, 484, 512, 514, 520, 522, 528, 530, 536, 542, 548, 550, 556, 558 not (38): each `DROP COLUMN "x",\nADD COLUMN "x" UUID …` in one statement, the TEXT→UUID id change (checked by script: every one has its ADD COLUMN within the statement) (fixed)
- zoonk/zoonk packages/db/src/prisma/migrations/20260116032149_step_kind_enum/migration.sql:11 not: `DROP COLUMN "kind",\nADD COLUMN "kind" "StepKind"` (fixed)
- trycompai/comp packages/db/prisma/migrations/20250423154259_make_role_string/migration.sql:2 not: `DROP COLUMN "role",\nADD COLUMN "role" TEXT` (fixed)
- openstatusHQ/openstatus: no match.

Round 2 (8 repos) after the same-statement, qualifier, literal, comment and AS fixes: 2 (inbox-zero MailSplit "kind"/"value",
both the MailSplitFilter INSERT above), fixed by the other-table check. Round 3 (13 repos, adds karakeep, useSend, formbricks,
papermark, umami): 0 matches.

Positive check (integration): a copy of rallly's `packages/database/prisma` with a new migration
`ALTER TABLE "polls" DROP COLUMN "location",\nDROP COLUMN "updated_at";` → 2 findings, at lines 2 and 3, citing
`packages/database/prisma/models/poll.prisma:39 declares Poll.location` and `:35 declares Poll.updatedAt`.
