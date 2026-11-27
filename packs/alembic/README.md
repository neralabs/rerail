# Rerail rule pack: Alembic

The `alembic-practice-*` rules are Rerail's own; none is a port of a linter rule. They catch the Alembic migrations
that take a live PostgreSQL database down, fail at deploy, or lose data: index builds and constraint checks that lock
out writes, NOT NULL columns that cannot be added to a populated table, type changes that rewrite a table, columns
dropped while the models still map them, revisions that fork the history, and data migrations that import the
application's models. Each rule is advice that the PostgreSQL, Alembic or SQLAlchemy documentation gives, or that
Squawk, strong_migrations, django-migration-linter and well-known migration post-mortems give, from at least two
independent sources ([NOTES-alembic.md](NOTES-alembic.md)).

**What it adds over a SQL linter.** Squawk reads the SQL that `alembic upgrade head --sql` renders, and projects such
as `alembic-squawk` and PandaDoc's `migration-lint` package that pipeline; on that SQL it sees batch operations as plain
ALTERs and ignores indexes on tables the same transaction creates. This pack adds three things that pipeline lacks:

- it runs in the edit window, on the Python the agent just wrote, with no offline render and no database;
- it reads revisions that cannot render offline (inspector calls, data migrations, `op.get_bind()`), common in dify,
  superset and open-webui;
- it has the cross-file rules no SQL linter can have: `drop-mapped-column` (the migration's own models still map the
  column), `model-import-in-migration`, `multiple-heads`, `empty-downgrade`, and the env.py side of
  `missing-lock-timeout` and `concurrent-index-outside-autocommit`.

Ruff, mypy, Bandit and pylint check none of these shapes.

**Who it is for.** The five lock rules (`index-not-concurrent`, `constraint-without-not-valid`, `column-type-rewrite`,
`set-not-null-scan`, `missing-lock-timeout`) ship off, in Log mode, because whether they apply depends on how a
project deploys, which the rules cannot see. Their advice is right when migrations run against a database that serves
traffic, and fixes nothing when a project stops the application to migrate (jupyterhub's upgrade guide says "Shut down
JupyterHub" before `upgrade-db`; mealie runs `command.upgrade(alembic_cfg, "head")` in `mealie/db/init_db.py` before
its one process serves). A team that migrates a live PostgreSQL database turns the five on in its repository settings.
For the same reason the corpus figures below give two precisions: **code** (the file holds the shape the message
describes) and **actionable** (a reviewer of that repository would ask for the change: not on a database that migrates
with the app stopped, not a no-op).

Each rule reads the repository's Python files through the pack's library [`rules/lib/alembic.rerail`](rules/lib/alembic.rerail),
which resolves what it reads rather than matching spellings: `op.*` and `batch_op.*` operations through `from alembic
import op`; the table each acts on (a literal, `op.f(...)` or a module constant); the tables the same `upgrade()`
creates or empties; project helpers that forward a table to an operation (one call deep, across modules); flags by
value (`postgresql_concurrently=CONCURRENT` with `CONCURRENT = True`, imported too); types by the SQLAlchemy class they
resolve to (aliases, TypeDecorators, subclasses, functions that return a type); dialect tests by their structure
(comparisons with a database name, `isinstance` on SQLAlchemy dialect classes, `not`/`and`/`or`, test functions of the
run); `autocommit_block()` regions; SQLAlchemy and sqlmodel model classes with their columns; revision ids; env.py and
the module it imports its engine from.

Each rule is proven three ways: by its cases in `upstream/alembic-practice/authored/<id>/` (pack test data in S3, pulled
by `npm run fixtures:pull -- --pack alembic`, not in git), where a line ending in `# caught` must be reported and no
other line may be (`rerail rules test <file>.rerail`, which reads the case directory as one repository); by a run on the
8 open-source repositories of the corpus that use Alembic ([the corpus](#the-corpus)), whose matches were read and judged
(every match, or a spread sample covering every repository); and by an independent review
([REVIEW-alembic.md](REVIEW-alembic.md)), whose findings were applied (its "Applied" section). Each rule was then
benchmarked on #1120's engine over further repositories (lemur, pyaleph, letta, flaskbb, HaloWebUI, notify-server,
remnawave-admin, themecp-backend-v2, siema_tech_fastapi); each rule's details give the figures. A rule starts on only
when at least 90% of its judged matches are actionable, the corpus and the benchmark counted together; the others ship
in Log mode, off.

Figures are after the review's fixes (runs `alembic-fix-*`), on the 8 Alembic repositories; "read" is every match or
the spread sample the rule's details name. The last column adds the benchmark's judged matches.

| Rule | Mode | Catches | Matches (repos) | Code precision (read) | Actionable (read) | Actionable, with the benchmark |
|---|---|---|---|---|---|---|
| `alembic-practice-index-not-concurrent` | Log, starts off | an index, unique constraint or primary key built on an existing table without CONCURRENTLY (also through `add_column(unique=/index=True)` and project helpers), once per table | 242 (7) | 100% (99 of 99) | 84% (83 of 99); 100% outside mealie | 83% (107 of 129) |
| `alembic-practice-concurrent-index-outside-autocommit` | Steer | a concurrent index build or drop run inside the migration's transaction | 0 (0) | no matches; 224 wrapped concurrent calls left alone | n/a | n/a (no matches) |
| `alembic-practice-not-null-without-server-default` | Log, starts off | a NOT NULL column added to an existing table with no `server_default` (and `default=` mistaken for one) | 13 (3) | 100% (13 of 13) | 100% deploying main; 4 of 13 tag to tag | 72% (18 of 25) |
| `alembic-practice-drop-mapped-column` | Log, starts off | a column or table dropped or renamed while a model of the migration's own tree still maps it | 0 (0) | no matches at release commits | n/a | 0% (0 of 1) |
| `alembic-practice-constraint-without-not-valid` | Log, starts off | a foreign key or CHECK added validated to an existing table (also through `add_column`) | 223 (8) | 100% (67 of 67) | 63% (42 of 67); 76% outside jupyterhub and mealie | 64% (62 of 97) |
| `alembic-practice-column-type-rewrite` | Log, starts off | a column type change that rewrites or scans the table (widenings and same-type spellings left alone) | 34 (6) | 100% (34 of 34) | 94% (32 of 34) | 67% (38 of 57) |
| `alembic-practice-set-not-null-scan` | Log, starts off | `alter_column(nullable=False)` on an existing table not proven NOT NULL by a validated check | 77 (6) | 95% (53 of 56) | 89% (50 of 56) | 74% (64 of 86) |
| `alembic-practice-missing-lock-timeout` | Log, starts off | a migration tree whose connection has no lock_timeout (once, at env.py), or a revision that slips in a tree that sets it per revision | 11 (8) | 100% (11 of 11) | 73% (8 of 11) | 72% (13 of 18) |
| `alembic-practice-multiple-heads` | Steer | revisions that leave one migration directory with two heads: `alembic upgrade head` refuses to run | 0 (0) | no matches at release commits | n/a | n/a (no matches) |
| `alembic-practice-rename-as-drop-add` | Log, starts off | one upgrade that drops a column and adds one of the same type on the same table with no data copy | 2 (1) | 50% (1 of 2) | 50% | 57% (4 of 7) |
| `alembic-practice-model-import-in-migration` | Steer | a migration that imports an application ORM model instead of declaring the table it needs | 13 (4) | 100% (13 of 13) | 100% | 100% (15 of 15) |
| `alembic-practice-empty-downgrade` | Log | a downgrade() that does nothing while upgrade() changes the schema, in a tree whose other revisions implement downgrade | 3 (2) | 100% (3 of 3) | 100% | 100% (3 of 3; no new matches) |

Each rule details its gaps and the shapes it leaves alone below. Rules in Log mode record matches without steering
the agent; the details give the reason.

## Starting values

4 of 12 rules start on: 3 steer (`concurrent-index-outside-autocommit`, `multiple-heads`, `model-import-in-migration`)
and 1 logs (`empty-downgrade`, on a sample of 3). The other 8 ship in Log mode, off, each below 90% actionable with the
corpus and the benchmark counted together (the table's last column):

- the five lock rules (`index-not-concurrent`, `constraint-without-not-valid`, `column-type-rewrite`,
  `set-not-null-scan`, `missing-lock-timeout`): their misses are mostly projects that migrate with the app stopped or
  before they serve (jupyterhub and mealie in the corpus; flaskbb, themecp-backend-v2 and siema_tech_fastapi in the
  benchmark), which the rules cannot see. A team that migrates a live PostgreSQL database turns them on.
  `set-not-null-scan`'s message for a column just added also advises one `server_default=` where the backfill is per
  row;
- `not-null-without-server-default`: right where the table holds rows when the migration runs; the benchmark's misses
  add columns to tables created hours earlier;
- `drop-mapped-column`: its one benchmark match drops a table that only a model nothing imports still maps;
- `rename-as-drop-add`: 3 of its 7 matches are deliberate replacements.

`concurrent-index-outside-autocommit` and `multiple-heads` have no matches on real code, in the corpus or the
benchmark, by design: at a release commit every repository already gets these right, and the bug exists only between
writing a migration and finishing the change, which is when Rerail checks an edit. They keep their starting state;
each was proven by an edit-window test on a copy of a corpus repository (see its details). On the five corpus
repositories the benchmark re-ran (the template, jupyterhub, mealie, open-webui, dispatch), #1120's engine reports
exactly the corpus findings for every rule.

## The corpus

The 8 open-source repositories with Alembic migrations among the 20 of the Python team corpus, shallow clones at pinned
commits. Each rule ran over every one of them (with `corpus-run`, or with the engine directly on the repository's
migration directory where a rule reads one revision at a time; each rule's details say which).

| Repository | Commit | Alembic revision files |
|---|---|--:|
| fastapi/full-stack-fastapi-template | `77e550a7` | 5 |
| jupyterhub/jupyterhub | `3ca37442` | 20 |
| mealie-recipes/mealie | `8dbb9280` | 66 |
| open-webui/open-webui | `0666e59d` | 72 |
| Netflix/dispatch | `dd2837e8` | 158 |
| polarsource/polar | `872272fb` | 203 |
| langgenius/dify | `724ac151` | 246 |
| apache/superset | `fdf61721` | 325 |

## Rule details

### `alembic-practice-index-not-concurrent`

Rerail's own rule, not a port. A plain `CREATE INDEX` "locks out writes (but not reads) on the table until it's done",
and ADD CONSTRAINT ... UNIQUE or PRIMARY KEY builds the same index under the same or a stronger lock. In an Alembic
revision's `upgrade()` (or a module-level function it calls), on a table `upgrade()` neither creates nor empties, it
reports, once per table and kind (the first operation, with a count in the message):

- `op.create_index(...)` and `batch_op.create_index(...)` without `postgresql_concurrently` on (read by value:
  `postgresql_concurrently=OFF` with `OFF = False` counts as off), with the table given positionally, as
  `table_name=`, through `op.f("...")` or as a module constant;
- `op.create_unique_constraint(...)` and the batch form, with its own message: build the unique index concurrently, then
  add the constraint `USING INDEX`;
- `op.add_column(t, sa.Column(..., unique=True))` and `index=True`: Alembic 1.20 emits `ALTER TABLE t ADD UNIQUE (c)`
  and `CREATE INDEX ix_t_c ON t (c)` after the ADD COLUMN (checked with its offline render, although its docs say
  these are ignored), each with its own message;
- `op.create_primary_key(...)`, with the fix: build a unique index concurrently, then `ADD CONSTRAINT ... PRIMARY KEY
  USING INDEX`;
- a call of a project helper, in another module (through its import) or the same file, whose `op.create_index` takes
  the table from a parameter: superset's `superset.migrations.shared.utils.create_index(table, name, columns)`, which
  most of its recent revisions use, reported at the call.

The messages name the fix (`postgresql_concurrently=True` inside an autocommit block, which commits the steps before
it) and when to leave the code alone: a table known to stay small. `postgresql_concurrently` is ignored on SQLite and
MySQL, so the fix is safe for projects that also run there (mealie, dify, open-webui, superset).

It leaves alone: indexes on a table the same `upgrade()` creates (`op.create_table` in any branch, a project's own
`create_table("t", ...)` wrapper, or `op.execute("CREATE TABLE t ...")`) or empties first; `postgresql_concurrently`
on (whether the call sits inside `autocommit_block()` is `concurrent-index-outside-autocommit`'s concern); code in a
branch that runs only on another database, read by the test's structure (the else of `if _is_pg(conn):`, the body of
`if dialect_name in ("sqlite", "mysql"):`, the else of `if not isinstance(bind.dialect, SQLiteDialect):`, code after
`if not _is_mysql(): return`, a test function imported from another module); `downgrade()`; revisions under
`tests/`; files without module-level `revision` and `down_revision`; an `op` that is not `alembic.op`. A test that
only mentions a database name (`if not settings.USE_SQLITE_FOR_TESTS:`) is no dialect branch.

**Gaps:** `downgrade()` (a rollback re-creates indexes on the rare path); helpers two calls deep; raw
`op.execute("CREATE INDEX ...")` and `sa.Index(...).create(bind)`; a plain `DROP INDEX` (ACCESS EXCLUSIVE too; not
reported); a table created by an earlier revision of the same release counts as existing; a project that runs only on
SQLite (its `alembic.ini` URL is not read); the size of a table is not known, so the message says when to leave it.

**Corpus** (BRIEF commits; polar, dify and superset on the review's trimmed copies; runs `alembic-fix-mine-*`):

| Repository | Commit | Before the review's fixes | After | Change |
|---|---|--:|--:|---|
| fastapi/full-stack-fastapi-template | 77e550a | 0 | 2 | `create_primary_key` on `user` and `item` (`d98dd8ec85a3:51,52`) |
| jupyterhub/jupyterhub | 3ca3744 | 0 | 0 | |
| Netflix/dispatch | dd2837e | 5 | 5 | |
| open-webui/open-webui | 0666e59 | 29 | 21 | grouped per table; a `batch_op.create_primary_key` on `tag` (`3ab32c4b8f59:45`) added |
| polarsource/polar | 872272f | 17 | 15 | grouped per table |
| mealie-recipes/mealie | 8dbb928 | 151 | 96 | grouped per table (the 97-index revision `ff5f73b01a7a` no longer gives 97) |
| langgenius/dify | 724ac15 | 73 | 64 | grouped per table |
| apache/superset | fdf6172 | 21 | 39 | 18 calls of the shared `create_index` helper added |
| Total | | 296 | 242 | |

Read: the review's sample of 78 (every repository) and all 21 new matches. Code precision 99 of 99. Actionable: 83 of
99 (84%); every read match outside mealie (83 of 83) is one a reviewer would ask for, and mealie's migrate before it
serves. The 21 new matches build indexes or keys on tables created by earlier revisions: superset's `logs`, `query`,
`report_execution_log`, `dashboard_slices_version`, `tasks`, `version_transaction` and configuration tables
(`2026-07-28_09-00_e7d93a524ff6_add_purge_audit_log.py:77` indexes `dashboard_slices_version` and leaves the
`purge_audit_log` it creates alone), the template's primary keys, open-webui's `tag` key.

Real matches, for example: open-webui `backend/open_webui/migrations/versions/9f6c2a8d1b43_add_channel_lookup_indexes.py`
(`message`), `d4c1a8e37b62_add_chat_timer_at_and_chat_indexes.py` (`chat`); dify
`api/migrations/versions/2026_01_12_1729-3334862ee907_feat_add_created_at_id_index_to_messages.py` (`messages`);
polar `server/migrations/versions/2026-07-06-1024_add_subscription_pause_columns.py` (`subscriptions`, about 282K
rows); dispatch `src/dispatch/database/revisions/tenant/versions/2023-10-23_f2605bfc1f59.py` (a GIN index on
`signal_instance`); superset `2017-01-24_12-40_f18570e03440_add_query_result_key_index.py` (`query`). Real but cheap:
superset's configuration tables, polar's small tables under `SET LOCAL lock_timeout = '5s'`; the message's "fine for a
small table" covers them.

**On #1120's engine (benchmark).** 118 matches on 6 repositories, 30 judged: code 29 of 30, actionable 24 of 30. With
the corpus: code 128 of 129 (99%), actionable 107 of 129 (83%). The misses are mostly real index builds in projects
that stop the app or migrate before serving (flaskbb
`flaskbb/migrations/202607102150_1783713047_settings_refactoring.py:30`, themecp-backend-v2), plus letta's
`f55542f37641:30`, where the team commented out the concurrent build on purpose. One false positive: pyaleph
`deployment/migrations/versions/0059_b9c4f1e6a2d7_persist_node_metrics.py:168`, whose upgrade creates the table with
`op.execute(text("CREATE TABLE ..."))`, which the rule does not read as creating it (it reads only a bare string). It
starts off (Log): below 90%; turn it on where the project migrates against a live database.

**Sources:** PostgreSQL, [CREATE INDEX](https://www.postgresql.org/docs/current/sql-createindex.html) ("locks out
writes (but not reads) on the table until it's done"; CONCURRENTLY cannot run in a transaction block; a failed
concurrent build leaves an INVALID index); Alembic,
[`MigrationContext.autocommit_block`](https://alembic.sqlalchemy.org/en/latest/api/runtime.html) and
[`add_column`](https://alembic.sqlalchemy.org/en/latest/ops.html); Squawk
[require-concurrent-index-creation](https://squawkhq.com/docs/require-concurrent-index-creation) and
[disallowed-unique-constraint](https://squawkhq.com/docs/disallowed-unique-constraint);
[strong_migrations](https://github.com/ankane/strong_migrations) "Adding an index non-concurrently" and "Adding a unique
constraint"; django-migration-linter CREATE_INDEX and ADD_UNIQUE (Django only).

### `alembic-practice-concurrent-index-outside-autocommit`

Rerail's own rule, not a port. PostgreSQL refuses `CREATE INDEX CONCURRENTLY` and `DROP INDEX CONCURRENTLY` inside a
transaction block, and Alembic runs each revision in a transaction on PostgreSQL, so a concurrent index operation left
in it fails the deploy's migration with "CREATE INDEX CONCURRENTLY cannot run inside a transaction block". In an
Alembic revision's `upgrade()` or `downgrade()`, or in a module-level function of the same file that a step calls from
outside an autocommit block, it reports:

- `op.create_index(..., postgresql_concurrently=True)` and `op.drop_index(..., postgresql_concurrently=True)`, and the
  same on the handle of `with op.batch_alter_table("t") as batch_op:`;
- such a call after an autocommit block in the same function (the revision's transaction is open again there), and
  one in PostgreSQL's own branch of a dialect test.

The message names the fix, `with op.get_context().autocommit_block():` around the call, and its cost: the block first
commits what the revision ran before it, so the concurrent work goes last. This rule also guards the advice of
`alembic-practice-index-not-concurrent` (where a team turns it on): an agent told to build an index concurrently often
adds only the keyword.

`postgresql_concurrently` is read by value: the literal `True`, or a module constant of the same file or one imported
from another module (`postgresql_concurrently=CONCURRENT`).

It leaves alone: calls inside `with <ctx>.autocommit_block():` at any depth (an `if` or a `batch_alter_table` in it,
a helper called from it, a MigrationContext held in a local); `postgresql_concurrently` False, absent, or an
expression the rule cannot evaluate; code in a branch that runs only on another database (read by the test's
structure, lib/alembic's `al_non_pg`); revisions under `tests/`; files without module-level `revision` and
`down_revision`; an `op` that is not `alembic.op`.

**Across files.** An env.py that runs its revisions with no transaction around them makes CONCURRENTLY work as
written: `context.configure(..., transactional_ddl=False)` (by value), or an engine or connection with
`isolation_level="AUTOCOMMIT"` set in env.py or in the module env.py imports it from (`from app.db import engine`).
The rule reports nothing in the revisions under that env.py's directory, nor in revisions under no env.py's directory
of the run when such an env.py exists. Other Alembic trees of the same repository, with their own env.py, are still
read (Netflix/dispatch's core and tenant schemas). `transaction_per_migration=True` still opens a transaction per
revision: still reported.

**Gaps:** raw SQL (`op.execute("CREATE INDEX CONCURRENTLY ...")`) is not read (Squawk reads SQL); the old
`op.execute("COMMIT")` workaround is not recognised (none in the corpus); helpers in another module, or two calls
deep, are not followed; an env.py outside the run, or `transactional_ddl` passed through a `**kwargs` dict
(superset's env.py, for SQLite and MySQL only), is not seen.

**Corpus** (BRIEF commits; polar and superset on the review's trimmed copies):

| Repository | Commit | Concurrent index calls | Inside an autocommit block | Matches |
|---|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 0 | – | 0 |
| jupyterhub/jupyterhub | 3ca3744 | 0 | – | 0 |
| mealie-recipes/mealie | 8dbb928 | 0 | – | 0 |
| open-webui/open-webui | 0666e59 | 0 | – | 0 |
| Netflix/dispatch | dd2837e | 0 | – | 0 |
| polarsource/polar | 872272f | 213 | 211, and 2 in helpers called only from inside one | 0 |
| langgenius/dify | 724ac15 | 11 | 11 | 0 |
| apache/superset | fdf6172 | 0 | – | 0 |

No match before or after the review's fixes, so no precision to measure: every repository that uses concurrent index
operations wraps them, which is the practice the rule enforces. polar's
`server/migrations/versions/2026-11-12-1405_add_manual_benefit_grant_timestamps.py` is the shape that needs the
helper reading: its `_create_index` and `_drop_index` call `op.create_index(..., postgresql_concurrently=True)`
outside any `with`, and `upgrade()` and `downgrade()` call them only inside `autocommit_block()`. The review's
edit-window test on a copy of the template (a new revision with both calls in `upgrade()` and `downgrade()`) reports
both lines, and the wrapped version is silent.

**On #1120's engine (benchmark).** No matches on the benchmark repositories, and no new ones on the corpus repositories
it re-ran.

**Sources:** PostgreSQL, [CREATE INDEX](https://www.postgresql.org/docs/current/sql-createindex.html) ("a regular
CREATE INDEX command can be performed within a transaction block, but CREATE INDEX CONCURRENTLY cannot") and
[DROP INDEX](https://www.postgresql.org/docs/current/sql-dropindex.html); SQLAlchemy's PostgreSQL dialect,
["Indexes with CONCURRENTLY"](https://docs.sqlalchemy.org/en/21/dialects/postgresql.html); Alembic,
[`MigrationContext.autocommit_block`](https://alembic.sqlalchemy.org/en/latest/api/runtime.html); Squawk
[ban-concurrent-index-creation-in-transaction](https://squawkhq.com/docs/ban-concurrent-index-creation-in-transaction)
(SQL files only). No Ruff or Bandit rule reads Alembic operations; this rule adds what Squawk cannot see in Python:
the autocommit block around the call (or around the helper call), the batch handle, and the env.py that decides
whether revisions run in a transaction at all.

### `alembic-practice-not-null-without-server-default`

Rerail's own rule, not a port. `ALTER TABLE t ADD COLUMN c ... NOT NULL` with no `DEFAULT` fails on PostgreSQL as soon
as `t` holds a row ("column c contains null values"): the migration passes on an empty development database and fails
on the production deploy. SQLAlchemy's `default=` does not help: SQLAlchemy applies it when it inserts a row, and it
never reaches the database; only `server_default=` fills the existing rows.

It matches, in `upgrade()` of an Alembic revision (or a module-level helper `upgrade()` calls), `op.add_column(t,
sa.Column(...))` and `batch_op.add_column(sa.Column(...))` inside `with op.batch_alter_table(t) as batch_op:`
(positional or `table_name=`/`column=` keywords, or a Column bound to a name first), on a table `upgrade()` neither
creates nor empties, where the Column is NOT NULL (`nullable=False`, or `primary_key=True` on a type that gets no
SERIAL) and passes no `server_default` (or `server_default=None`). Flags are read by value (a module constant counts),
and types by the class they resolve to. A Column with `default=` gets the second message, which explains that
`default=` is Python-side only. The first message names `server_default=` first; its other route, a nullable column
made NOT NULL in a later revision, is the one `set-not-null-scan` then checks.

It leaves alone: any `server_default=` value (`""`, `"0"`, `sa.text(...)`, `sa.false()`); nullable columns (omitted or
`nullable=True`); tables the same `upgrade()` creates, or empties first (`DELETE FROM t` without WHERE, TRUNCATE); the
branch of a dialect test that runs only on another database (`else:` of dify's `if _is_pg(conn):`; MySQL fills a NOT
NULL column with an implicit default); generated columns (`sa.Computed(...)`, `sa.Identity(...)`); an integer
`primary_key=True` column without `autoincrement=False`, however its type is spelled (`sa.BigInteger`, `BigInt =
sa.BigInteger`, `from sqlalchemy import Integer as Int`), which SQLAlchemy renders as `SERIAL`/`BIGSERIAL` on
PostgreSQL (Alembic's offline SQL: `ADD COLUMN id2 SERIAL NOT NULL`), so the sequence numbers the existing rows;
`downgrade()`; test files; files that are not Alembic revisions; an `op` that is not `alembic.op`.

**Gaps:** `nullable=` given as an expression the rule cannot evaluate; a table created by an earlier revision that no
database has run yet is still empty, but is reported (the message says when the column is fine as written).

**Sources:** Squawk, [adding-required-field](https://squawkhq.com/docs/adding-required-field) ("will fail
immediately upon running for any populated table"); Alembic, [`add_column`](https://alembic.sqlalchemy.org/en/latest/ops.html)
(a server default uses `server_default`, "and not `default` which only specifies Python-side defaults"); SQLAlchemy,
[Column INSERT/UPDATE Defaults](https://docs.sqlalchemy.org/en/21/core/defaults.html); django-migration-linter's
NOT_NULL check, which calls the framework-default-is-a-database-default belief a "common misconception".

**Overlap:** Squawk's adding-required-field reads the SQL; nothing in Ruff or Bandit. This rule reads the migration
before any SQL exists: the Column's keywords (`default=` against `server_default=`), the dialect branch, and which
tables the same `upgrade()` creates.

**Corpus:** after the review's fixes, the same 13 matches as before them, in the 8 Alembic repositories (runs
`alembic-fix-mine-*`), every one read. The 6 dify `else:` branches of `if _is_pg(conn):` that pass only `default=`
(e.g. `api/migrations/versions/2024_08_13_0633-63a83fcf12ba_support_conversation_variables.py:58`) are left alone.

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a7` | 0 | |
| jupyterhub/jupyterhub | `3ca37442` | 0 | |
| mealie-recipes/mealie | `8dbb9280` | 0 | |
| open-webui/open-webui | `0666e59d` | 1 | 1 |
| Netflix/dispatch | `dd2837e8` | 0 | |
| polarsource/polar | `872272fb` | 0 | |
| langgenius/dify | `724ac151` | 9 | 9 (latent, see below) |
| apache/superset | `fdf61721` | 3 | 3 |

Real, failing on a populated production table: open-webui
`backend/open_webui/migrations/versions/2f1211949ecc_update_message_and_channel_member_table.py:68`
(`joined_at BigInteger NOT NULL` on `channel_member`, a table released in v0.5.3; the column came in v0.6.41); superset
`superset/migrations/versions/2020-10-05_18-10_af30ca79208f_collapse_alerting_models_into_a_single_.py:128,130,133`
(`alerts.database_id`, `sql`, `validator_type` with `default=` only, in the non-SQLite branch that PostgreSQL runs;
`alerts` dates from May 2020).

dify's 9 (`00bacef91f18`, `161cadc1af8d`, `714aafe25d39` ×2, `8ec536f3c800`, `ad472b61a054`, `f2a6fc85e260`,
`97e2e1a644e8`, `0b2f2c8a9d1e`) add NOT NULL columns to tables created 1 to 4 revisions earlier, and each pair first
shipped in the same dify release (0.3.34, 0.5.0, 0.6.14, 1.15.0, checked against the release tags). An install that
upgrades from tag to tag had no rows there yet; the DDL fails on any database that ran the earlier revision and got
rows (a deployment of the main branch, a development database with data). `161cadc1af8d` shows the intent it
missed: its comment says "Step 1: Add column without NOT NULL constraint" above `nullable=False`. Precision: code 13 of
13; actionable 13 of 13 for a team that deploys main, 4 of 13 counting only tag-to-tag upgrades. The fix
(`server_default=`) is safe either way.

**On #1120's engine (benchmark).** 12 matches, all judged: code 12 of 12, actionable 5 of 12 (letta's 3 and pyaleph's
2, such as `deployment/migrations/versions/0004_82e0f9c7a952_error_codes.py:32` on `rejected_messages`). With the
corpus: code 25 of 25, actionable 18 of 25 (72%). The other 7 add NOT NULL columns to tables created hours earlier the
same day, which could hold no rows yet (themecp-backend-v2
`alembic/versions/a81e39f8ee47_updated_ddl_contest_session.py:24`, an autogenerated rename; siema_tech_fastapi's
`products`). It starts off (Log): below 90%; turn it on where the table holds rows when the migration runs.

### `alembic-practice-drop-mapped-column`

Rerail's own rule, not a port. It checks that a migration and its own tree's models agree: in `upgrade()` (or a
module-level helper it calls by name) of an Alembic revision outside test code, it reports

- `op.drop_column("users", "nick")` / `batch_op.drop_column("nick")` (table from `op.batch_alter_table("users")`) while
  a model of the migration's tree still maps `users.nick`;
- `op.alter_column("users", "nick", new_column_name="handle")` (or the batch form) while such a model still maps `nick`;
- `op.drop_table("users")` / `op.rename_table("users", "people")` while such a model still maps `users`.

**The tree's models.** A migration sees only the models of the application its env.py runs: the modules env.py's
imports reach, transitively (every import form: `import a.b as c`, `from a import b`, star and relative imports, and
the packages above each module), plus the other modules of each imported package (a models package's `__init__`
often imports only the base). When that reaches no model, the documents under env.py's top-level directory count. So
in a monorepo, `accounts/migrations` dropping `users.nick` is silent while only `billing/models.py`, another service
with its own Alembic tree, maps `users.nick`.

A table name is a literal, `op.f("...")`, or a module constant. A model maps `users` when an application class (not in
a migration, `env.py` or test file) has `__tablename__ = "users"`, is a sqlmodel `table=True` class named `User`
(table `user`), or maps a `__table__ = Table("users", ...)` (inline, a module-level name, or one imported from another
module). It maps `nick` when it, or a mixin or base class, declares `nick = Column(...)`, `nick: Mapped[str] =
mapped_column(...)`, a bare `nick: Mapped[str]`, a sqlmodel field, a Column whose explicit name is `nick`, or a Column
wrapped in `deferred(...)`/`column_property(...)`, and its mapper does not leave it out: `__mapper_args__`'s
`exclude_properties` and `include_properties` are honoured (a dict display or `dict(...)`, strings or Column objects,
inherited from bases as Python looks them up). A Table no class maps counts in full. The messages say what the rule
checks: a model still maps what the upgrade removes, so remove the attribute or model (ship that first if migrations
run before the new code is live), or keep the column.

It leaves alone:

- a column or table no application model maps (the normal end state of expand/contract);
- a rebuild: a later operation of the same `upgrade()` adds the same column back (`add_column` of a Column with that
  name, `create_table` listing it, `alter_column(..., new_column_name=...)` to that name) or creates or renames a table
  to that name, as full-stack-fastapi-template's `d98dd8ec85a3` does when it swaps `item.id` for a UUID column;
- a historical revision whose descendant revision (following `down_revision`, merge tuples included) adds the column
  or table back: at the repository's head the model maps the re-added column, not the dropped one;
- a column the model unmaps by setting the attribute to None (`client_secret = None` over a mixin's Column, polar's
  expand/contract step; an annotated `power: int | None = None` is a field default and still maps);
- a table the same revision drops in `downgrade()` too: a reset that something else rebuilds (jupyterhub's
  `833da8570507_rbac.py` drops `api_tokens` both ways, and the hub's `Base.metadata.create_all()` recreates it, with
  the hub stopped for `upgrade-db`);
- `downgrade()`; migrations in test trees; models in test files; a model class declared inside a migration file.

**Gaps:** a `__tablename__` computed by `declared_attr` (Netflix/dispatch derives every table name from the class name,
so the rule sees none of its models); Flask-SQLAlchemy `db.Model` subclasses without `__tablename__` (the table name
is generated); imperative mappings (`registry.map_imperatively`); a table or column named by a loop variable or other
non-literal expression; `schema=`; a model that maps a column only through `__table_args__` or reflection; a mixin
from an installed library; a re-add by a revision whose `down_revision` is not a literal; the attribute removed in
the same change as the drop (the models and the migration then agree, and the rule cannot see the deploy order).

**Sources:** strong_migrations, "Removing a column" and "Renaming a column" (https://github.com/ankane/strong_migrations);
Squawk ban-drop-column ("Update your application code to no longer read or write the column",
https://squawkhq.com/docs/ban-drop-column) and renaming-column; GoCardless, "Zero-downtime Postgres migrations: the
hard parts" (https://gocardless.com/blog/zero-downtime-postgres-migrations-the-hard-parts); django-migration-linter
DROP_COLUMN / RENAME_COLUMN
(https://github.com/3YOURMIND/django-migration-linter/blob/main/docs/incompatibilities.md); polar's
`server/migrations/env.py`, whose `UNMAPPED_COLUMNS` holds columns removed from the models before they are dropped.

**Overlap:** Squawk and django-migration-linter flag every drop and rename (on SQL files and Django migrations), without
knowing whether code still uses the column; Ruff and Bandit check nothing here. This rule's value is the cross-file
check against the live models: it stays silent on the contract step of an expand/contract, which is the normal way
columns go.

**Corpus** (8 Alembic repositories at the `MANIFEST.tsv` commits; "removals" counts the `drop_column`, `drop_table`,
`rename_table` and `alter_column` operations in `upgrade()` whose table the rule resolved, from a diagnostic copy of the
rule before the review; the reworked rule was re-run on all 8, runs `alembic-fix-drop-mapped-column-*`):

| Repository | Commit | Removals read | Reported | Silenced as a rebuild | Table not resolved |
|---|---|--:|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 16 | 0 | 3 (`d98dd8ec85a3`: `item.owner_id`, `user.id`, `item.id` swapped for UUID columns) | 0 |
| jupyterhub/jupyterhub | 3ca3744 | 9 | 0 | 0 (`api_tokens` in `833da8570507_rbac.py` is a reset, dropped in both directions) | 0 |
| mealie-recipes/mealie | 8dbb928 | 9 | 0 | 0 | 0 |
| open-webui/open-webui | 0666e59 | 18 | 0 | 3 (`242a2047eae0` `chat.chat`, `374d2f66af06` `prompt`, `3ff2c63645b8` `config`) | 2 |
| Netflix/dispatch | dd2837e | 85 | 0 | 0 (no model resolvable: `declared_attr` table names) | 0 |
| polarsource/polar | 872272f | 60 | 0 | 0 | 0 |
| langgenius/dify | 724ac15 | 112 | 0 | 0 | 0 |
| apache/superset | fdf6172 | 141 | 0 | 0 | 7 |

0 matches at the heads, before and after the rework, as expected: at a release the models and the migrations agree.
Tree resolution after the rework: env.py's imports reach every model module of the template, jupyterhub, mealie
(through `_all_models`' star imports), polar (117 of 118) and dify, and 14 of 15 of dispatch's; superset falls back to
its top-level directory (its metadata comes through `flask_appbuilder`, outside the run). The review's two
counterexamples (the monorepo, `exclude_properties`) give 0 findings, down from 2. The bug lives in the window
between writing the migration and editing the model, which is when Rerail checks an edit. No historical drop of a
column that a later revision adds back occurs in these repositories (an independent `ast` survey agrees); the
descendant exemption is proven on the cases only. The first version reported jupyterhub's `api_tokens` reset
(harmless: the hub is stopped for `upgrade-db` and `create_all()` rebuilds the table); the both-directions exemption
removed it. polar's eight
`drop_column(..., "token")`-style revisions of September 2026 stay silent because the models map the replacements under
explicit names (`mapped_column("token_v2", ...)`). Edit-window check: a new revision added to a copy of mealie that drops
`shopping_list_items.is_food` (batch), `shopping_list_items.created_at` (declared on the `SqlAlchemyBase` base class)
and the table `shopping_list_recipe_reference` is reported on all three lines, and a drop of an unmapped column is not;
after `is_food` is removed from the model, its line is silent. The same check on dify reports `op.drop_column("apps",
"name")`. dify was read as the 306 files the rule needs (`api/models`, `api/migrations`, and every other non-test file
that declares `__tablename__` or a `Table(...)`), because the shared corpus queue held 90+ runs.

**On #1120's engine (benchmark).** 1 match, judged: code 1 of 1, actionable 0 of 1. letta
`alembic/versions/567e9fe06270_create_new_runs_table_and_remove_legacy_.py:67` drops `job_messages`, which
`letta/orm/job_messages.py` still maps; but no module imports that model, so nothing reads the table. The rule counts
every module of an imported package as the tree's models, which brought the dead model in. It starts off (Log): below
90%; turn it on where the models of the migration's tree are the ones the application loads.

### `alembic-practice-constraint-without-not-valid`

Rerail's own rule, not a port. In an Alembic revision's `upgrade()` (or a module-level function of the same file that
`upgrade()` calls), it reports a FOREIGN KEY or CHECK constraint added validated to a table that may hold rows (one
`upgrade()` neither creates nor empties):

- `op.create_foreign_key(...)` and `batch_op.create_foreign_key(...)`, with the table given positionally, as
  `source_table=`, through `op.f("...")`, as a module constant, or from the batch. A source table that exists with a
  referent table the same `upgrade()` creates is still reported: the existing rows are what the add checks.
- `op.create_check_constraint(...)` and `batch_op.create_check_constraint(...)`.
- `op.add_column(t, sa.Column(..., sa.ForeignKey(...)))` and an inline `sa.CheckConstraint(...)` in the added Column:
  Alembic 1.20 renders them as a separate, validated `ALTER TABLE t ADD FOREIGN KEY` / `ADD CHECK` after the ADD COLUMN
  (offline render). The constraint classes resolve through imports, so `ForeignKey as FK` counts and a project class
  of the same name does not.
- `postgresql_not_valid` is read by value: False, or a constant bound to False, counts as missing.

Adding either constraint checks every existing row, and "other updates to the table are locked out until the ALTER
TABLE ADD CONSTRAINT command is committed"; ADD FOREIGN KEY also takes a SHARE ROW EXCLUSIVE lock on the referenced
table (PostgreSQL, ALTER TABLE). With NOT VALID the add skips the scan; a later VALIDATE CONSTRAINT scans under SHARE
UPDATE EXCLUSIVE, which lets writes through. The messages name both steps: pass `postgresql_not_valid=True`, then
VALIDATE the constraint once the add has committed, inside an autocommit block or in a later revision. A foreign key
on a column the same `upgrade()` adds (dispatch's commonest shape), and a ForeignKey inside an added Column, get the
cheaper fix: `add_column(..., inline_references=True)` renders the reference inside the ADD COLUMN, which PostgreSQL
marks valid at once for a nullable column with no default (Alembic's `add_column` docs; not verified here on a
database). Every message says when to leave the code alone: a table known to stay small.

It leaves alone: constraints on a source table the same `upgrade()` creates (with `op.create_table` in any branch, a
project's own `create_table("t", ...)` wrapper, or `op.execute("CREATE TABLE t ...")`) or empties first; constraints
declared inside `op.create_table(...)`; `postgresql_not_valid` on, an expression the rule cannot evaluate, or a
`**kwargs` splat that may carry it; `add_column(..., inline_references=True)`; code in a branch that runs only on
another database (lib/alembic's structural `al_non_pg`); `downgrade()`; revisions under `tests/`; files without
module-level `revision` and `down_revision`; an `op` that is not `alembic.op`.

**Gaps:** `downgrade()`; a table name the rule cannot resolve (a loop variable or a function parameter: jupyterhub
`833da8570507_rbac.py`, superset `2017-10-03_14-37_4736ec66ce19_.py`, polar's `_ensure_foreign_key` helpers); a table
created by an earlier revision of the same release counts as existing (dify's `knowledge_fs_*` tables, created by
`2026_07_21_1300-b7f2a9d41c60` and constrained by three revisions the same day); constraints added through
`op.execute("ALTER TABLE ... ADD CONSTRAINT ...")` or a helper in another module are not read; `inline_references=True`
on a NOT NULL column or one with a default is left alone although PostgreSQL then validates; the size of a table is
not known, so the message says when to leave it.

**Corpus** (BRIEF commits; polar, dify and superset on the review's trimmed copies; runs `alembic-fix-mine-*`; a match
is code-real when the revision adds that validated constraint to a table an earlier revision created):

| Repository | Commit | Before the review's fixes | After | Code-real (read) | Actionable (read) |
|---|---|--:|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 2 FK | 2 | 2 of 2 | 2 |
| jupyterhub/jupyterhub | 3ca3744 | 2 FK | 2 | 2 of 2 | 0 (Hub stopped for `upgrade-db`) |
| mealie-recipes/mealie | 8dbb928 | 10 FK | 10 | 10 of 10 | 0 (migrates before serving) |
| open-webui/open-webui | 0666e59 | 2 (1 FK, 1 check) | 4 | 4 of 4 | 4 |
| Netflix/dispatch | dd2837e | 163 FK | 163 | 17 of 17 (every 10th) | 17 |
| polarsource/polar | 872272f | 5 FK | 5 | 5 of 5 | 5 |
| langgenius/dify | 724ac15 | 17 (1 FK, 16 check) | 17 | 17 of 17 | 4 |
| apache/superset | fdf6172 | 20 (16 FK, 4 check) | 20 | 10 of 10 (every 2nd) | 10 |
| Total | | 221 | 223 | 67 of 67 | 42 of 67 (63%) |

The two new matches are open-webui's `add_column` with an inline `sa.ForeignKey(...)` in a batch
(`81cc2ce44d79_update_channel_file_and_knowledge_table.py:30`, `channel_file.message_id`;
`b8e4f0a3c752_add_group_hierarchy.py:18`, `group.parent_group_id`), which Alembic renders as a validated ADD FOREIGN
KEY. Actionable precision is 63% overall and 76% (42 of 55) outside the two repositories that migrate with the app
stopped, below the 90% bar: dify's 13 CHECKs on `knowledge_fs_*` tables created hours earlier in the same release are
on empty tables. The review proposed steering anyway, since following the message costs little even on an empty
table; with the benchmark below it stays under the bar and starts off. The commonest corpus shape (an FK on a column
the same `upgrade()` adds, dispatch's) gets the cheaper `inline_references=True` fix.

Every match outside dispatch was read in its whole `upgrade()`; in dispatch, each of the 44 files was read for its
`upgrade()`'s table creations, column additions and constraint operations (93 matches sit in one autogenerated
revision, `tenant/versions/2021-06-15_8a558baeef05.py`, that drops and re-adds foreign keys with `ondelete="CASCADE"`).
A Python `ast` cross-check of every `create_foreign_key`/`create_check_constraint` in the same `upgrade()` steps found
no report on a table the step creates or with `postgresql_not_valid=True`, and 3 silent calls with an unresolvable
table (the gaps above). The 4 foreign keys of polar's squash revision `2026-06-17-1455_initial_migration_june_2026_squash.py`
are on tables it creates, and stay silent.

Real matches, for example: dispatch `src/dispatch/database/revisions/tenant/versions/2022-11-09_eb348d46afd0.py`
(re-adds `signal_instance`'s FK to `case`), `2025-02-02_8790ff1c8d6d.py` (FK on `event`); superset
`superset/migrations/versions/2023-03-27_12-30_7e67aecbf3f1_chart_ds_constraint.py` and
`2026-10-13_00-00_dfa683969f48_allow_saved_query_chart_datasource.py` (CHECK on `slices`), `2022-03-16_21-07_8b841273bec3_sql_lab_models_database_constraint_updates.py`
(FKs on `tab_state`); polar `server/migrations/versions/2026-09-14-0902_add_backoffice_users_and_nullable_actor_.py`
(four FKs on support tables, while polar's later revisions use `postgresql_not_valid=True` plus VALIDATE); open-webui
`backend/open_webui/migrations/versions/c92da83b017f_add_apps.py` (CHECK on `channel_member`); fastapi template
`backend/app/alembic/versions/1a31ce608336_add_cascade_delete_relationships.py` (FK on `item`). Code-real but not
actionable: dify's 13 checks on `knowledge_fs_*` tables created hours earlier in the same series (likely empty when
the release deploys), and mealie's and jupyterhub's, which migrate with the app stopped. polar's 5 run under
`SET LOCAL lock_timeout = '5s'` and still scan; the message's "fine for a small table" covers the cheap ones.

**On #1120's engine (benchmark).** 78 matches on 5 repositories, 30 judged: code 30 of 30, actionable 20 of 30. With
the corpus: code 97 of 97, actionable 62 of 97 (64%). No false positive; the misses are deployment again: flaskbb's
autogenerated drop and re-add of foreign keys to change `ondelete`
(`flaskbb/migrations/201802021027_af3f5579c84d_add_cascades.py:100`), in a project that stops FlaskBB to migrate, and
siema_tech_fastapi's, which migrates before serving; outside those, 20 of 20. It starts off (Log): below 90%; turn it
on where the project migrates against a live database.

**Sources:** PostgreSQL, [ALTER TABLE](https://www.postgresql.org/docs/current/sql-altertable.html) (ADD
table_constraint NOT VALID, VALIDATE CONSTRAINT's SHARE UPDATE EXCLUSIVE lock, ADD FOREIGN KEY's SHARE ROW EXCLUSIVE
lock on both tables, and the Notes on NOT VALID); SQLAlchemy,
[PostgreSQL Constraint Options](https://docs.sqlalchemy.org/en/21/dialects/postgresql.html) (`postgresql_not_valid`
on `CheckConstraint` and `ForeignKeyConstraint`, with an Alembic example; sqlalchemy/sqlalchemy#7600, 1.4 series);
Alembic, [`MigrationContext.autocommit_block`](https://alembic.sqlalchemy.org/en/latest/api/runtime.html); Squawk
[adding-foreign-key-constraint](https://squawkhq.com/docs/adding-foreign-key-constraint) and
[constraint-missing-not-valid](https://squawkhq.com/docs/constraint-missing-not-valid) (SQL files only);
[strong_migrations](https://github.com/ankane/strong_migrations) "Adding a foreign key" and "Adding a check
constraint". No Ruff or Bandit rule reads Alembic operations; this rule adds what Squawk cannot see in Python: the
tables the same revision creates, and the table of a `batch_alter_table` block.

### `alembic-practice-column-type-rewrite`

Rerail's own rule, not a port. In an Alembic revision's `upgrade()` (or a module-level function of the same file that
`upgrade()` calls), it reports `op.alter_column(t, c, type_=NEW, ...)` and `batch_op.alter_column(c, type_=NEW, ...)`
inside `with op.batch_alter_table(t) as batch_op:` on a table `upgrade()` does not create, unless PostgreSQL can make
the change in its catalog alone. It judges the change against `existing_type=`.

`ALTER COLUMN ... TYPE` takes an ACCESS EXCLUSIVE lock (no reads or writes) and "will normally cause the entire table
and its indexes to be rewritten", unless the old type is binary coercible to the new one (PostgreSQL, ALTER TABLE
notes). Shrinking or adding a length limit (`VARCHAR(256)` to `VARCHAR(255)`, `String()` or `Text` to `String(255)`)
is no binary coercion either: PostgreSQL re-reads every row against the limit under the same lock. Two messages:

- a type change (`sa.INTEGER()` to `sa.BigInteger()`, `postgresql.JSON` to `postgresql.JSONB`, `sa.Integer()` to
  `sa.Float()`/`sa.Numeric()`, `sa.DateTime()` to `sa.BigInteger()` with `postgresql_using=`, `sa.Text()` to
  `sa.JSON()`/`sa.LargeBinary()`, `sa.BOOLEAN()` to `sa.String()`, `Numeric(10, 2)` to `Numeric(12, 4)`): add a new
  column of the new type, backfill it in batches, switch reads and writes, then drop the old column;
- a length limit added or shrunk: keep the column's type and add a NOT VALID length CHECK, validated in a later
  revision, or go through a new column.

Both say when to keep the change as written: a table that stays small.

Types are compared by the PostgreSQL type their class renders, not by spelling (lib/alembic's `al_pg_type_t`): the
class is resolved through imports and aliases (`from sqlalchemy import Integer as Int`, `Int32 = sa.Integer` in
another module), a project TypeDecorator's `impl` (dify's `StringUUID`), a project subclass of a SQLAlchemy type, and
a project function that returns a type (superset's `MediumText()`, `Text().with_variant(MEDIUMTEXT(), "mysql")`, is
TEXT on PostgreSQL). So the same type spelled two ways is no change: TIMESTAMP and DateTime (with or without a time
zone on both sides), `postgresql.UUID` and `sa.Uuid`, INTEGER and Integer, BIGINT and BigInteger, BOOLEAN and Boolean,
TEXT and Text, VARCHAR(255) and sqlmodel's `AutoString(length=255)`. Autogenerate restates the type next to a
nullability change, so these come up in agent edits.

It leaves alone the changes PostgreSQL makes without touching the rows (Squawk, strong_migrations): VARCHAR/String(n)
to a length m >= n, to an unlimited `String()` or to `Text`/`UnicodeText`; `Text` to `String()` without a length;
`Numeric(p, s)` to `Numeric(p2, s)` with p2 >= p, or to an unconstrained `Numeric()`; `LargeBinary` to
`LargeBinary(length=...)` (both BYTEA); the same PostgreSQL type on both sides. Also left alone: a target from
`sqlalchemy.dialects.mysql` (`mysql.LONGTEXT`); a length that is not an integer literal where it decides (superset's
`sa.String(current_length)`); an `existing_type` that is not a type expression (a conditional); a call without
`existing_type` whose target is a string, text or numeric type, which may be a widening the rule cannot judge; a table
the same `upgrade()` creates or empties; the branch of a dialect test that runs only on another database; a call in an
`except` handler (superset's non-Postgres fallback after a `try:` that made the change); `alter_column` without
`type_=`; `downgrade()`; test files; an `op` that is not `alembic.op`.

**Gaps:** a class outside the run that is not SQLAlchemy's is read by its last name; a table named by a variable is
not seen; `DateTime()` to `DateTime(timezone=True)` is reported although PostgreSQL 12+ skips that rewrite when the
session time zone is UTC; an ARRAY of a widened VARCHAR is reported; a `postgresql_using=` on an otherwise safe
widening is left alone with it; raw `op.execute("ALTER TABLE ... TYPE ...")` is not read; a table created by an
earlier revision that no database has run yet is still empty, but is reported.

**Sources:** PostgreSQL, [ALTER TABLE](https://www.postgresql.org/docs/current/sql-altertable.html) (notes on
`SET DATA TYPE`); Squawk, [changing-column-type](https://squawkhq.com/docs/changing-column-type) (VARCHAR to TEXT and
VARCHAR(5) to VARCHAR(10) are safe, INT to BIGINT is not); strong_migrations, "Changing the type of a column" (the safe
changes: increasing or removing a string limit, string to text, increasing numeric precision at the same scale).

**Overlap:** Squawk's changing-column-type flags every type change in SQL; this rule reads the migration before any
SQL exists, compares `type_=` with `existing_type=`, and leaves the safe widenings, the MySQL-only branches and the
tables the same `upgrade()` creates alone. Nothing in Ruff or Bandit.

**Corpus:** 34 matches in the 8 Alembic repositories before and after the review's fixes (runs `alembic-fix-mine-*`),
every one read. None of the same-type spellings the rework leaves alone occurs in a corpus `upgrade()` (they occur in
downgrades), so the count is unchanged; they come up in agent edits. An independent Python `ast` survey found 80
`alter_column(type_=)` calls in `upgrade()`; the 46 left alone are 36 widenings (26 in dify, such as VARCHAR(40) to
255; 10 in superset, such as VARCHAR(50) to 1000, Text to `MediumText`, `LargeBinary` to `LargeBinary(length=...)`), 9
in MySQL or SQLite branches or superset's `except TypeError:` fallback, and superset's `x2s8ocx6rto6` with a computed
`existing_type`.

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a7` | 4 | 4 |
| jupyterhub/jupyterhub | `3ca37442` | 0 | |
| mealie-recipes/mealie | `8dbb9280` | 2 | 2 |
| open-webui/open-webui | `0666e59d` | 3 | 3 |
| Netflix/dispatch | `dd2837e8` | 2 | 2 |
| polarsource/polar | `872272fb` | 0 | |
| langgenius/dify | `724ac151` | 11 | 11 |
| apache/superset | `fdf61721` | 12 | 12 |

Every match rewrites (or re-reads) a populated table under ACCESS EXCLUSIVE on PostgreSQL. Examples on tables that grow
with use: dify `api/migrations/versions/2025_01_01_2000-a91b476a53de_change_workflow_runs_total_tokens_to_.py:23`
(`workflow_runs.total_tokens` INTEGER to BigInteger), `api/migrations/versions/2025_02_27_0917-d20049ed0af6_add_metadata_function.py:95`
(`documents.doc_metadata` JSON to JSONB); mealie
`mealie/alembic/versions/2022-03-23-17.43.34_263dd6707191_convert_quantity_from_integer_to_float.py:30`
(`recipes_ingredients.quantity` Integer to Float); open-webui
`backend/open_webui/migrations/versions/c29facfe716b_update_file_table_path.py:34` (`file.meta` Text to JSON with
`postgresql_using`); dispatch `src/dispatch/database/revisions/tenant/versions/2024-11-04_928b725d64f6.py:29`
(`entity.source` BOOLEAN to String). About 12 of the 34 are on configuration tables that stay small (superset's
`clusters`, `datasources`, `dbs`; dify's `datasource_providers`, `tool_oauth_tenant_clients`), where the rewrite is
short and the message's "keep it for a table that stays small" applies. Precision: code 34 of 34; actionable 32 of 34
(mealie's 2 migrate before it serves).

**On #1120's engine (benchmark).** 23 matches, all judged: code 23 of 23, actionable 6 of 23 (HaloWebUI's 3 and
letta's 3, such as letta's BIGINT to Integer at `f7507eab4bb9:57`). With the corpus: code 57 of 57, actionable 38 of
57 (67%). 17 of the 23 are flaskbb's TIMESTAMP to TIMESTAMPTZ changes
(`flaskbb/migrations/201606210939_d9530a529b3f_add_timezone_awareness_for_datetime.py:100`), which PostgreSQL 12+ does
not rewrite when the session time zone is UTC (the gap above), in a project that stops the app to migrate. It starts
off (Log): below 90%; turn it on where the project migrates against a live database and the change is one PostgreSQL
rewrites.

### `alembic-practice-set-not-null-scan`

Rerail's own rule, not a port. `ALTER TABLE t ALTER COLUMN c SET NOT NULL` checks every row of `t` ("by scanning the
entire table") while ALTER TABLE holds ACCESS EXCLUSIVE on it, so reads and writes queue behind it for the whole scan,
and every query that arrives meanwhile queues behind them. Since PostgreSQL 12 the scan is skipped when a valid CHECK
constraint already proves the column holds no NULL. The safe sequence: `op.create_check_constraint("ck_t_c_not_null",
"t", "c IS NOT NULL", postgresql_not_valid=True)` (no scan), `VALIDATE CONSTRAINT` once that has committed (it scans
under SHARE UPDATE EXCLUSIVE, which lets reads and writes through), then `alter_column(..., nullable=False)` (no scan)
and drop the check. `lock_timeout` does not help: it bounds the wait for the lock, not how long the scan holds it.

It matches, in `upgrade()` of an Alembic revision (or a module-level helper `upgrade()` calls),
`op.alter_column(t, c, nullable=False)` and `batch_op.alter_column(c, nullable=False)` inside
`with op.batch_alter_table(t) as batch_op:` (positional or `table_name=`/`column_name=` keywords, `op.f(...)`, module
constants, together with any other keyword such as `existing_type=`, `type_=` or `server_default=`), on a table
`upgrade()` does not create. When the same `upgrade()` adds the column earlier (Alembic's `add_column`, or a project's
own `add_columns("t", sa.Column("c", ...))` wrapper as in superset), it gets a second message: the SET NOT NULL scans
just the same, but when every existing row gets one value (mealie's `on_hand = FALSE`, dispatch's
`scope = 'multiple'`) the cheaper fix is one `add_column(..., server_default=..., nullable=False)`, which PostgreSQL 11+
applies without a rewrite or a scan.

It leaves alone: an `alter_column` whose column a VALIDATE in the same `upgrade()` proves NOT NULL, that is, an
`op.execute(...)` running `VALIDATE CONSTRAINT n` where `n` is shown, anywhere in the run, to be a `c IS NOT NULL`
check: added by SQL (polar's `ADD CONSTRAINT {NOT_NULL_CHECK} CHECK (c IS NOT NULL) NOT VALID` with
`VALIDATE CONSTRAINT {NOT_NULL_CHECK}`) or by `create_check_constraint(N, "t", "c IS NOT NULL", ...)`, in this
revision or an earlier one (the safe sequence's first revision); a `create_check_constraint` on `t` whose condition
says `c IS NOT NULL` added validated in the same file (its own scan is `constraint-without-not-valid`'s concern);
`existing_nullable=False` with `nullable=False` (the column is already NOT NULL; flags are read by value);
`nullable=True` or no `nullable=`; tables the same `upgrade()` creates or empties; the branch of a dialect test that
runs only on another database; `downgrade()`; test files; files that are not Alembic revisions; an `op` that is not
`alembic.op`. A VALIDATE of another constraint exempts nothing, even one whose name contains the column's
(`fk_users_email_domain` for `email`): the name is no longer read as evidence.

**Gaps:** `op.execute("ALTER TABLE ... SET NOT NULL")` is not read; a table created by an earlier revision of the same
release counts as existing; a NOT NULL check added outside the run is not seen, so its column is reported; a column
already NOT NULL that the call does not mark `existing_nullable=False` is reported (a no-op on PostgreSQL); a table
that the same transaction already rewrites through raw SQL (dispatch's `ADD COLUMN id SERIAL PRIMARY KEY`) is
reported, though the safe sequence does not shorten that lock. Whether a table stays small cannot be read from the
migration, so the message says to leave a small table as it is.

**Sources:** PostgreSQL, [ALTER TABLE](https://www.postgresql.org/docs/current/sql-altertable.html) (whether the
column holds a NULL "is checked during the ALTER TABLE by scanning the entire table ...; however, if a valid CHECK
constraint exists ... which proves no NULL can exist, then the table scan is skipped"; PostgreSQL 12+; PostgreSQL 18
also takes a `NOT NULL ... NOT VALID` constraint, validated the same way); Squawk,
[adding-not-nullable-field](https://squawkhq.com/docs/adding-not-nullable-field) (the same sequence, with Alembic
code); strong_migrations, "Setting NOT NULL on an existing column"; PostgresAI,
[Zero-downtime Postgres schema migrations need this: lock_timeout and retries](https://postgres.ai/blog/20210923-zero-downtime-postgres-schema-migrations-lock-timeout-and-retries)
(a blocked `set not null` hangs every SELECT queued behind it). polar's own history makes the case: its 19 plain
`alter_column(nullable=False)` revisions from June to November 2026 run under `SET LOCAL lock_timeout = '5s'`, and its
8 newest `enforce_*_not_null` revisions switch to `CHECK ... NOT VALID`, `VALIDATE`, SET NOT NULL inside an
autocommit block, with the comment "A plain SET NOT NULL scans the whole table under ACCESS EXCLUSIVE"
(`server/migrations/versions/2026-11-03-1400_make_webhook_delivery_organization_id_non_nullable.py`).

**Overlap:** Squawk's adding-not-nullable-field reads the SQL; nothing in Ruff or Bandit. This rule reads the
migration before any SQL exists: which tables the same `upgrade()` creates, the dialect branch, the batch handle,
whether a VALIDATE of the column's check runs first, and whether the column was added in the same revision.

**Corpus:** 77 matches in the 8 Alembic repositories before and after the review's fixes (runs `alembic-fix-mine-*`;
the corpus has no VALIDATE whose name merely contains a column's). The engine's matches equal an independent Python `ast`
survey of `alter_column(..., nullable=False)` in every `upgrade()` (89), less the 7 polar revisions behind a validated
check, 4 dify calls in the MySQL `else:` branch, and 1 superset call with `existing_nullable=False`.

| Repository | Commit | Matches | After add | Real |
|---|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a7` | 3 | 2 | 3 |
| jupyterhub/jupyterhub | `3ca37442` | 0 | | |
| mealie-recipes/mealie | `8dbb9280` | 3 | 2 | 3 |
| open-webui/open-webui | `0666e59d` | 0 | | |
| Netflix/dispatch | `dd2837e8` | 15 | 1 | 12 |
| polarsource/polar | `872272fb` | 19 | 0 | 19 |
| langgenius/dify | `724ac151` | 18 | 4 | 18 |
| apache/superset | `fdf61721` | 19 | 5 | 19 |

Precision, on the review's read sample of 56 (all of the template, mealie, dispatch and polar; every 3rd of superset;
every 2nd of dify): code 53 of 56; actionable 50 of 56 (89%; mealie's 3 migrate before it serves). The 3 that are
not: dispatch `src/dispatch/database/revisions/tenant/versions/2022-08-18_86ec96cd14f8.py:25,34,43`, where the same
`upgrade()` runs `ALTER TABLE ... ADD COLUMN id SERIAL PRIMARY KEY` on each table, which rewrites it under the same
lock anyway. Real matches on large, live tables: polar
`server/migrations/versions/2026-09-03-0700_make_webhookendpoint_webhookevent_api_.py:46` (`webhook_events`),
`2026-09-18-0919_enforce_user_sessions_token_v2_not_null.py:26` (`user_sessions`) and
`2026-11-05-1103_require_organization_catalog_limits.py:31,34` (`organizations`); dify
`api/migrations/versions/2024_11_01_0623-09a8d1878d9b_update_workflows_graph_features_and_.py:30,35`
(`conversations.inputs`, `messages.inputs`) and `2025_10_30_1518-669ffd70119c_introduce_trigger.py:312`
(`celery_taskmeta`); dispatch `tenant/versions/2022-08-29_479024506e05.py:52-55` (`case`, `incident`). About half of
the matches are on tables that stay small (superset's metadata tables such as `dbs`, `clusters`, `themes`; the
template's `user` and `item`), where the scan takes milliseconds and the message's "fine for a table that stays small"
applies; that, and a fix that spans two revisions, is why the rule logs rather than steers.

**On #1120's engine (benchmark).** 66 matches on 6 repositories, 30 judged: code 21 of 30 (29 of 30 for the scan
alone), actionable 14 of 30. With the corpus: code 74 of 86, actionable 64 of 86 (74%). The main false-positive shape
is the message for a column the same upgrade just added: it advises one `server_default=` where the backfill is per
row (8 of the 14 such rows, e.g. letta `alembic/versions/74e860718e0d_add_archival_memory_sharing.py:374`, filled per
agent from `archives_agents`), where the NOT VALID check route is the right fix; most other misses are flaskbb's and
themecp-backend-v2's, which do not migrate a live database. It starts off (Log): below 90%; turn it on where the
project migrates against a live database.

### `alembic-practice-missing-lock-timeout`

Rerail's own rule, not a port. A DDL statement waiting for its lock queues every later query on the table behind it,
and `lock_timeout` defaults to 0, wait forever: one long transaction on the table turns the migration into an outage.
Whether migrations run with a lock_timeout is decided once, for the connection env.py opens, so the rule judges the
tree, not each revision. A tree is the revisions under one env.py's directory (the deepest one, so a nested tenant
env.py owns its revisions; a revision under no env.py's directory belongs to every env.py of the run). Its locking
revisions are those whose `upgrade()` (or a helper of the same file it calls) takes a write-blocking lock on a table
it neither creates nor empties, on PostgreSQL: `add_column`, `drop_column`, `alter_column`, `create_foreign_key`,
`create_check_constraint`, `create_unique_constraint`, `create_primary_key`, `drop_constraint`, `rename_table`,
`drop_table`, and `create_index` / `drop_index` without `postgresql_concurrently` on.

- When neither env.py nor a module it reads settings from sets lock_timeout, and at most half the tree's locking
  revisions set it themselves, the rule reports once, at env.py's `context.configure(connection=...)`, with the
  tree's count ("0 of 178 revisions here set it"): set it once for the migration connection; leave it when no traffic
  runs during migrations.
- When more than half set it (a convention tree: polar, 110 of 113), a locking revision that does not set it before
  its first lock is a slip, reported at that operation: set it first in `upgrade()`; leave it when the table gets no
  traffic.

A setting is read by what it means, not by a mention: a string that parses as `SET [LOCAL|SESSION] lock_timeout = v` /
`TO v` (also among several statements, triple-quoted or in an f-string), `ALTER ROLE|USER|DATABASE ... SET
lock_timeout`, `set_config('lock_timeout', v, ...)`, a libpq options string `-c lock_timeout=v` (or URL-encoded in a
DSN), or the `lock_timeout` key of an asyncpg `server_settings` dict. A zero value in any unit (`0`, `'0s'`, `'0ms'`)
and DEFAULT turn it off; RESET is no setting; a log call, a comment, a docstring or a redis lock's `{"lock_timeout":
30}` set nothing. env.py's own settings count, and those of a module it reads them from (lib/alembic's
`al_env_reads`: `from app.db import engine`, a re-export through a package `__init__`, a module imported whole, a
`@event.listens_for(engine, "connect")` hook in the engine's module).

It leaves alone: operations on a table the same `upgrade()` creates or empties; concurrent index builds; code in a
branch that runs only on another database; data statements; `downgrade()` (a lock_timeout set only there does not
protect `upgrade()`); revisions under `tests/`; files without module-level `revision` and `down_revision`.

**Gaps:** raw DDL in `op.execute("ALTER TABLE ...")` is not read; a foreign key from a new table also locks its
existing referent (SHARE ROW EXCLUSIVE), which is not reported; a `SET LOCAL` at the top of `upgrade()` ends at the
commit of a later `autocommit_block()`, so non-concurrent operations inside that block run without it, unseen; a
setting two imports away from env.py, passed in a variable, split across adjacent literals, or made by the deploy (a
role's or database's default, `PGOPTIONS`) is not seen; a setting the imported module makes for another engine
counts; a run with revisions but no env.py reports nothing.

**Mode: log, off.** The finding is one decision per tree, and two of the eight corpus repositories (jupyterhub, mealie)
and two of the seven benchmark ones migrate with no other traffic by design, which the message's "leave it" covers but
the rule cannot see. Steering the per-revision slips of convention trees is the next step once more convention trees
than polar have been read.

**Corpus** (BRIEF commits; polar, dify and superset on the review's trimmed copies; runs
`alembic-fix-missing-lock-timeout-*`; every match read):

| Repository | Before the review's fixes | After | Where | Code-real | Actionable |
|---|--:|--:|---|--:|--:|
| fastapi/full-stack-fastapi-template | 4 | 1 | `backend/app/alembic/env.py:75` (0 of 4 set it) | 1 | 1 |
| jupyterhub/jupyterhub | 16 | 1 | `jupyterhub/alembic/env.py:101` (0 of 16) | 1 | 0 (Hub stopped for `upgrade-db`) |
| mealie-recipes/mealie | 48 | 1 | `mealie/alembic/env.py:107` (0 of 48) | 1 | 0 (migrates before serving) |
| open-webui/open-webui | 49 | 1 | `backend/open_webui/migrations/env.py:103` (0 of 49) | 1 | 1 |
| Netflix/dispatch | 128 | 2 | `revisions/core/env.py:51` (0 of 5), `revisions/tenant/env.py:63` (0 of 123) | 2 | 2 |
| polarsource/polar | 3 | 3 | per revision; the tree sets it in 110 of 113 | 3 | 2 |
| langgenius/dify | 178 | 1 | `api/migrations/env.py:99` (0 of 178) | 1 | 1 |
| apache/superset | 153 | 1 | `superset/migrations/env.py:123` (0 of 153) | 1 | 1 |
| Total | 579 | 11 | | 11 of 11 | 8 of 11 (73%) |

Each env.py-level count equals the repository's match count before the fixes, so every former restatement is now
inside one finding. polar keeps its three slips: `2026-10-23-1000_add_agents_claim_polled_at.py:22` (`add_column` on
`agents`) and `2026-11-03-1201_make_subscription_started_at_non_nullable.py:21` (`alter_column(nullable=False)` on
`subscriptions`) are real; `2026-08-12-1000_drop_pledge_transactions_table.py:22` drops an index of a table nothing
reads. Without jupyterhub and mealie, 8 of 9 are actionable. The review's counterexamples: the engine built in
`app/db.py` with `-c lock_timeout=5000` is silent; the log call and the `'0s'` trees are reported once at their env.py.

**On #1120's engine (benchmark).** 7 matches, all judged, one per env.py: code 7 of 7, actionable 5 of 7. With the
corpus: code 18 of 18, actionable 13 of 18 (72%). No false-positive shape; the 2 misses migrate before serving, in one
container (themecp-backend-v2 `alembic/env.py:83`, siema_tech_fastapi `alembic/env.py:38`). It starts off (Log): below
90%; turn it on where the project migrates against a live database.

**Sources:** Squawk, [Safe migrations](https://squawkhq.com/docs/safe_migrations) and
[require-lock-timeout](https://squawkhq.com/docs/require-lock-timeout) ("all other reads and writes sent after will
wait for this statement too"); PostgreSQL,
[Client connection defaults](https://www.postgresql.org/docs/current/runtime-config-client.html) (`lock_timeout`,
default 0); GoCardless, [Zero-downtime Postgres migrations: the hard
parts](https://gocardless.com/blog/zero-downtime-postgres-migrations-the-hard-parts) (a 15-second API outage; "set
lock_timeout in your migration scripts"); PostgresAI, [lock_timeout and
retries](https://postgres.ai/blog/20210923-zero-downtime-postgres-schema-migrations-lock-timeout-and-retries); Xata,
[Migrations and exclusive locks](https://xata.io/blog/migrations-and-exclusive-locks). Squawk checks each SQL file
and cannot see env.py; no Ruff or Bandit rule reads Alembic operations. This rule adds the env.py of the revision's
tree, a shared helper or constant the revision imports, the tables the same revision creates, and the order of the
`SET` and the first lock.

### `alembic-practice-multiple-heads`

Rerail's own rule, not a port. It reads every Alembic revision file of the repository (module-level `revision = ...` and
`down_revision = ...`, annotated or not, outside test code) as one graph. A revision's parents are the ids its
`down_revision` names: a string, a name bound to one in the same file (`down_revision = PARENT`), or each element of a
tuple or list; a tuple or list of one is a single parent, not a merge. An id resolves first among the revisions of the
same directory and only then elsewhere, so two version directories that reuse ids (`"0001"`, `"0002"`) stay apart. A
head is a revision no other revision names in its `down_revision` (`depends_on` orders an upgrade but joins nothing:
Alembic's `RevisionMap` counts heads by `down_revision` alone). Two heads that share an ancestor, or that sit in one
version directory (a second base, `down_revision = None`, next to the existing line), make `alembic upgrade head`
refuse to run: "Multiple head revisions are present for given argument 'head'" (checked with Alembic 1.20.1 by the
review). The rule reports at each head's `down_revision = ...`, once per other head:

- two revisions written on the same parent, with no merge revision joining them: the usual result of two branches
  each adding a migration on the head they saw (both heads are reported, also when one line has grown further);
- a fork whose one side only carries `depends_on`;
- a second base in the same version directory, the shape an agent creates by writing `down_revision = None`;
- a merge revision that is one head while a revision written on a pre-merge revision is the other;
- two branches that each ran `alembic merge heads` on the same heads (two merge heads).

The message names the other head and the revision that starts each line: if it has not run on any shared database,
point its `down_revision` at the other head; otherwise add a merge revision with `alembic merge heads`. For two merge
heads it says to merge again or delete the merge that has not run. It says when to leave it: an intentional parallel
branch, which should carry `branch_labels`.

It leaves alone:

- forks joined by a merge revision (`down_revision = ("b2", "b3")`, also as a list or through constants);
- heads of trees in separate directories that share no ancestor (Netflix/dispatch's `core` and `tenant` trees, two
  services that reuse sequential ids);
- a head whose own line (between the fork, or its own base, and the head) sets `branch_labels`: an intentional branch,
  migrated with `alembic upgrade heads` or `branch@head`. Labels on the shared base or trunk do not silence a fork;
- a line holding a revision whose parent is unknown (an imported name, a call, or an id no revision of the run has: a
  squashed history, a library's version location), which would otherwise look like a second base;
- revisions in test trees.

**Gaps:** a history spread over several directories (`version_locations`, recursive version directories) whose lines
share no ancestor is read as separate trees, because which directories a script location reads is in `alembic.ini`;
a parent imported from another module is unknown and its line is left alone.

**Sources:** Alembic, "Working with Branches" (https://alembic.sqlalchemy.org/en/latest/branches.html): the multiple
heads error, `alembic merge heads`, `branch_labels`; Alembic's `RevisionMap` (`alembic/script/revision.py`), whose
`heads` drop only revisions named in a `down_revision` (not in `depends_on`); pytest-alembic's
`test_single_head_revision`, for merge conflicts that "lazily break during deployment"
(https://pytest-alembic.readthedocs.io/en/latest/quickstart.html).

**Overlap:** pytest-alembic (an opt-in test) and Alembic's own error at deploy; no linter (Ruff, Bandit, Squawk) reads
the revision graph. This rule's value is catching it while the revision is written, before the deploy fails.

**Corpus** (8 Alembic repositories at the `MANIFEST.tsv` commits). "Engine" is the rule's own count before and after
the review's fixes (runs `alembic-fix-multiple-heads-*`, polar, dify and superset on the review's trimmed copies);
"heads" and "merges" come from an independent Python `ast` survey of every revision file:

| Repository | Commit | Revisions | Trees | Heads | Merge revisions | Engine matches |
|---|---|--:|--:|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 5 | 1 | 1 | 0 | 0 |
| jupyterhub/jupyterhub | 3ca3744 | 19 | 1 | 1 | 0 | 0 |
| mealie-recipes/mealie | 8dbb928 | 66 | 1 | 1 | 0 | 0 |
| open-webui/open-webui | 0666e59 | 72 | 1 | 1 | 0 | 0 |
| Netflix/dispatch | dd2837e | 162 | 2 (`core`, `tenant`) | 2 | 3 | 0 |
| polarsource/polar | 872272f | 205 | 1 | 1 | 0 | 0 |
| langgenius/dify | 724ac15 | 253 | 1 | 1 | 4 | 0 |
| apache/superset | fdf6172 | 411 | 1 | 1 | 48 | 0 |

0 matches at the pinned commits, as expected: a release has one head per tree. A diagnostic copy of the rule confirmed
the engine resolves the graphs (polar 205, dify 253, superset 411 revisions, one head each; dispatch 162 revisions, one
head in `core` and one in `tenant`). The 55 merge
revisions are real past forks the rule sees as joined. Edit-window checks: a new revision added to a copy of polar's
migrations with `down_revision` on the head's parent (`f3238e76d766`) is reported together with the current head
`3c1e7a9d52f4`, and adding `alembic merge heads`' revision silences both; deleting superset's merge revision
`3740c7bcb2c6` from a copy reports the two heads it joined (`875472b9c016`, itself a merge, and `d623a0cb6bb0`).
After the rework, a text count of each version directory finds one base and one head in every tree, so the second-base
and unknown-parent readings change nothing at release commits. The review's counterexamples: two bases and a
`depends_on`-only fork are now reported (both lines each); `("r1",)` gets the rebase message; the review's
`down_revision = PARENT` with `PARENT` bound after its use (a NameError at import, which Alembic cannot load either)
is left alone, while a constant bound before its use is resolved. The review's edit-window copies: the fork is
reported on both lines, the revision written on the head is silent.

**On #1120's engine (benchmark).** No matches on the benchmark repositories, and no new ones on the corpus repositories
it re-ran.

### `alembic-practice-rename-as-drop-add`

Rerail's own rule, not a port. Mode `log`, starts off. In an Alembic revision's `upgrade()` (or a module-level function
of the same file that `upgrade()` calls), it reports `op.drop_column(t, "a")` and `batch_op.drop_column("a")` inside
`with op.batch_alter_table(t) as batch_op:` when the same `upgrade()` adds a column `b` to the same existing table and
the pair reads as a rename that loses the data:

- `upgrade()` neither creates nor drops `t`, and adds as many columns to `t` as it drops;
- the dropped column's type, read from `downgrade()`'s `add_column(t, Column("a", T))`, is the added column's type,
  or `downgrade()` does not show it. Types compare by the PostgreSQL type their class renders (lib/alembic's
  `al_pg_type_t`, through imports, aliases and project TypeDecorators): VARCHAR, String and sqlmodel's AutoString are
  one, TIMESTAMP and DateTime one, INTEGER and Integer one. A widened length still pairs (`VARCHAR(40)` to
  `String(80)`: a rename with a widening); a shorter one does not;
- the pair is unique: no other add or drop on `t` in that `upgrade()` matches either side;
- nothing in `upgrade()` (or the helpers it calls) moves rows. Data movement is read by what it resolves to: an UPDATE
  or INSERT in SQL text (a literal, or a module constant the step names); SQLAlchemy's `update()`/`insert()` constructs
  resolved through imports (`sa.update`, `sqlalchemy.dialects.postgresql.insert`); a table object's
  `.update()`/`.insert()` (bound to `sa.table(...)`/`sa.Table(...)`, or a model's `__table__`); `op.bulk_insert`; an
  ORM session resolved to `sqlalchemy.orm`. Any other `execute`, such as `op.execute("SET LOCAL lock_timeout =
  '5s'")` (the line `missing-lock-timeout` asks for), moves nothing, nor does `dict.update(...)` or a project's own
  `insert()`.

Autogenerate cannot detect a rename: "Changes of column name ... are detected as a column add/drop pair, which is not
at all the same as a name change" (Alembic, What does Autogenerate Detect). Run as generated, the migration deletes
the column's data on the first deploy. The message names `alter_column(..., new_column_name=...)` for a rename, or
copying the data first, and says to keep the migration only when the old column's data is meant to go.

It leaves alone the copy pattern (add the new column, `UPDATE t SET b = a`, drop the old one) and any other data
statement in the same `upgrade()`; types that differ (`JSON` dropped, `LargeBinary` added); a drop and an add of the
same name (a column recreated); tables with more drops than adds or the reverse (Netflix/dispatch's
`2022-08-01_391f7c3e6992.py` replaces `stable_at` with `triage_at` and `escalated_at`; its `2022-10-27_01aa49ca0470.py`
adds `window` and drops five columns, one of them the INTEGER `creator_id`); two adds and two drops of one type,
where no pairing can be named; tables the same `upgrade()` creates or drops; the branch of a dialect test that runs
only on another database; `downgrade()`; test files; an `op` that is not `alembic.op`.

**Gaps:** a replacement whose new column means something else but has the old one's type (langgenius/dify's
`2e9819ca5b28_add_tenant_id_in_api_token.py`: `dataset_id` dropped, `tenant_id` added, both `StringUUID`) is
reported, which is why the rule starts in `log` and off; the old type is read only from `downgrade()`'s `add_column`;
a data copy in a function of another module, or through SQL in a variable the rule cannot resolve, is not seen; two
renames of one type on one table are not reported; a table named by a variable is not seen.

**Sources:** Alembic, [What does Autogenerate Detect (and what does it not
detect?)](https://alembic.sqlalchemy.org/en/latest/autogenerate.html#what-does-autogenerate-detect-and-what-does-it-not-detect);
strong_migrations, "Renaming a column" (rename through a new column, a backfill and a code switch);
Squawk, [renaming-column](https://squawkhq.com/docs/renaming-column).

**Overlap:** none. Squawk sees `ADD COLUMN` and `DROP COLUMN` as two unrelated statements and its renaming-column rule
sees only `RENAME COLUMN`; nothing in Ruff or Bandit.

**Corpus:** before the review's fixes, the five smaller repositories ran with `corpus-run` over the repository root and
dify, polar and superset with `rerail-engine logic` over their migration directory; after them, all 8 with
`corpus-run` (polar, dify and superset on the review's trimmed copies; runs `alembic-fix-mine-*`), with the same
matches.

| Repository | Commit | Run over | Matches | Real |
|---|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a` | root (43 files) | 0 | 0 |
| jupyterhub/jupyterhub | `3ca3744` | root (158 files) | 0 | 0 |
| open-webui/open-webui | `0666e59` | root (330 files) | 0 | 0 |
| Netflix/dispatch | `dd2837e` | root (717 files) | 0 | 0 |
| mealie-recipes/mealie | `8dbb928` | root (723 files) | 0 | 0 |
| polarsource/polar | `872272f` | `server/migrations` (206 files) | 0 | 0 |
| langgenius/dify | `724ac15` | `api/migrations` (254 files) | 2 | 1 |
| apache/superset | `fdf6172` | `superset/migrations` (425 files) | 0 | 0 |

Precision 1 of 2 (50%, code and actionable), hence `log`, off until a larger sample exists. The real match: langgenius/dify
`api/migrations/versions/00bacef91f18_rename_api_provider_description.py`, titled "rename api provider description":
the autogenerated batch adds `description` (`LongText`, `nullable=False`) and drops `description_str` (`LongText`),
so every provider's description is deleted. The other: `2e9819ca5b28_add_tenant_id_in_api_token.py` swaps
`dataset_id` for `tenant_id`, a different reference whose old values are meant to go. Of the 20 (table, `upgrade()`)
pairs in these repositories that drop and add columns on the same existing table, 13 move data (`op.execute`,
`UPDATE`, `sa.update`, an ORM `Session`), 5 add and drop different numbers of columns, and the 2 above are reported.

**On #1120's engine (benchmark).** 5 matches, all judged: code and actionable 3 of 5; with the corpus, 4 of 7 (57%).
The 2 false are letta `567e9fe06270:73,79`, a deliberate replacement (dify `2e9819ca5b28`'s shape); it stays off.

### `alembic-practice-model-import-in-migration`

Rerail's own rule, not a port. It reads a migration against the application's models. In an Alembic revision outside
test code (module level, `upgrade()`, `downgrade()` or a helper), it reports

- each name of `from app.models.group import Group` (or `from app.models import Group`, a re-export followed to the
  class) that is a mapped ORM class of the application, one finding per imported name;
- a model read through its module, `from jupyterhub import orm` then `db.query(orm.APIToken)`: the first use of each
  model in the file, outside type annotations.

A mapped class is a class of the repository that declares `__tablename__`, is a sqlmodel `table=True` class,
subclasses either (`al_model_q`), or subclasses a declarative base (`class Base(DeclarativeBase)`, a module-level
`Base = declarative_base()`, or Flask-SQLAlchemy's `db.Model` for a module-level `db = SQLAlchemy(...)`, each resolved
through imports) without `__abstract__ = True`: Netflix/dispatch names every table with a `declared_attr` on its base,
so its models declare no `__tablename__`. A class of another library that merely has a `Model` attribute is no base.
An import whose module sits in a namespace package (a directory without `__init__.py`, as dispatch's
`dispatch/feedback/`) is matched to the class by file path. The message says why (the model follows today's schema,
not this revision's) and names the replacement: declare the table in the migration with `sa.table()` and
`sa.column()`; it says that enums, types and constants may stay. A model the migration uses only for its table name
(`User.__tablename__`, `User.__table__.name`) gets its own message: nothing queries its columns, but the import still
loads the application when the migration runs, so write the table name as a string.

It leaves alone:

- column types and TypeDecorators (`GUID`, `StringUUID`, `LongText`, `JSONDict`, `NaiveDateTime`), enums and constants
  (`ObjectType`, `TagType`, `AppTriggerStatus`, `ADDITIVE_METRIC_TYPES_LOWER`), pydantic schemas, sqlmodel classes
  without `table=True`, plain helper classes;
- the declarative base and mixins (`SqlAlchemyBase`, `Base`, `TimestampMixin`). Evidence: the one migration in the
  corpus that imports an application base (mealie `c7427796f7b6`) calls its static `SqlAlchemyBase.normalize()`, and
  `Base.metadata` carries no column a query names. Querying the base itself is not something code does;
- a model class declared inside the migration file (mealie's later revisions declare their own `SqlAlchemyBase` and
  `RecipeModel`, `IngredientFoodModel`, ... locally; superset declares `declarative_base()` models in its migrations),
  and a model of a module under a `migrations/` or `alembic/` directory (superset's `migrations/shared/` keeps frozen
  copies for the migrations);
- `if TYPE_CHECKING:` imports and annotations (jupyterhub's `def access_scopes(oauth_client: orm.OAuthClient, ...)`);
- `env.py` (it imports `Base.metadata` on purpose), revisions in test trees, and test files.

**Gaps:** a model module bound to a local name another way (`importlib.import_module(...)`); a base assigned from
`declarative_base()` in the same module as its models (the base reference does not resolve there); models from an
installed package; a class whose base chain crosses a namespace package (the path fallback covers only the imported
name itself).

**Sources:** zzzeek on sqlalchemy/alembic#679 ("your ORM models may have changed so would no longer work in this
migration once it's older", https://github.com/sqlalchemy/alembic/issues/679); the Alembic cookbook's data migrations,
which declare tables with `sa.table()`/`sa.column()` (https://alembic.sqlalchemy.org/en/latest/cookbook.html);
django-migration-linter's RUNPYTHON_MODEL_IMPORT error
(https://github.com/3YOURMIND/django-migration-linter/blob/main/docs/incompatibilities.md). jupyterhub's own
`651f5419b74d_api_token_scopes.py` shows the failure: it has to `defer(orm.Spawner.display_name)` because "the new
display_name column isn't added until a subsequent db upgrade script runs".

**Overlap:** django-migration-linter checks Django migrations only; Ruff and Bandit check nothing here. This rule's value
is the cross-file resolution: it follows the import (re-exports and namespace packages included) to a class and decides
from the class's definition whether it is mapped, so it tells `Group` from `GUID` in the same models package.

**Corpus** (8 Alembic repositories at the `MANIFEST.tsv` commits; every match read). Before the review's fixes, six
were run whole through `corpus-run` and polar and dify with the engine on the files the rule needs; after them, all 8
through `corpus-run` (polar, dify and superset on the review's trimmed copies; runs `alembic-fix-mine-*`), with the
same 13 matches, all with the query message (none uses a model only for its table name).

| Repository | Commit | Matches | Real | Files |
|---|---|--:|--:|---|
| fastapi/full-stack-fastapi-template | 77e550a | 0 | 0 | |
| jupyterhub/jupyterhub | 3ca3744 | 4 | 4 | `651f5419b74d_api_token_scopes.py` (`orm.APIToken`, `orm.Role`, `orm.OAuthClient`, `orm.Spawner`, queried and monkeypatched with `relationship()`) |
| mealie-recipes/mealie | 8dbb928 | 7 | 7 | `04ac51cbe9a4_added_group_slug.py` (`Group`), `0341b154f79a_added_normalized_unit_and_food_names.py` (`IngredientFoodModel`, `IngredientUnitModel`), `dded3119c1fe_added_unique_constraints.py` (`MultiPurposeLabel`, `IngredientFoodModel`, `IngredientUnitModel`, `RecipeIngredientModel`) |
| open-webui/open-webui | 0666e59 | 0 | 0 | |
| Netflix/dispatch | dd2837e | 1 | 1 | `tenant/versions/2024-10-16_3c49f62d7914.py` (`Feedback`, `session.query(Feedback).all()`) |
| polarsource/polar | 872272f | 0 | 0 | |
| langgenius/dify | 724ac15 | 0 | 0 | |
| apache/superset | fdf6172 | 1 | 1 | `2021-08-09_17-32_07071313dd52_change_fetch_values_predicate_to_text.py` (`SqlaTable`, `session.query(SqlaTable).filter(...)`) |

Every match is a migration that queries or writes through the imported model (`session.query(Model)`,
`select(Model)`, `load_only(Model.col)`), or reads its `__tablename__` next to such a query. An independent `grep` of
the corpus's migration imports finds the same set and no other import of a mapped class; dispatch's plugin
configuration imports (pydantic models) and superset's `ObjectType`/`TagType`/`ReportState` enums stay silent, as do
mealie's ten `GUID` imports, dify's `StringUUID`/`LongText` imports and jupyterhub's `JSONDict` imports. The first
version missed dispatch's `Feedback`: `dispatch/feedback/` has no `__init__.py`, so the class's qualified name lacks the
`dispatch.feedback` prefix the import names; the file-path fallback fixed it (a case covers it).

Precision on the corpus: code and actionable 13 of 13 (100%). The 13 sit in 6 revisions, because projects that learned this
moved to local models (mealie's revisions from 2024 on declare their own `SqlAlchemyBase` and model classes, which stay
silent). Edit-window check: a new revision added to a copy of polar that imports `Organization` (through the
`polar.models` re-export) and `polar.models.user.User` is reported on both lines and `polar.config.settings` is not; in
a copy of dify, `from models.model import App` and `models.Account` (through `import models`) are reported, and
`StringUUID` and `AppTriggerStatus` are not.

**On #1120's engine (benchmark).** 2 matches (pyaleph `0016:17`, `0032:18`), both real and actionable; with the
corpus, 15 of 15.

### `alembic-practice-empty-downgrade`

Rerail's own rule, not a port. It reports, at `def downgrade():`, an Alembic revision whose `upgrade()` changes the
schema while its `downgrade()` does nothing (`pass`, `...`, a bare `return`, or those, with at most Alembic's
autogenerate markers `# ### commands auto generated by Alembic - please adjust! ###`), in a migration tree that
implements its downgrades.
`alembic downgrade` runs the empty function, reports success and moves the version table back while the schema keeps
the change: the database no longer matches the revision it records, the next `alembic upgrade` can fail on what is
already there (a column that exists, a constraint already dropped), and the rollback path the team relies on is broken.

A schema change is any Alembic operation (`op.m(...)`, or `b.m(...)` on a `with op.batch_alter_table(...) as b:`
handle, also inside a module-level helper `upgrade()` calls) other than `execute`, `bulk_insert`, `f`, `get_bind`,
`get_context`, `batch_alter_table`, `inline_literal`, `invoke`, `run_async` and `implementation_for`.

The convention is read across files. A migration tree is the revision files of one directory (Netflix/dispatch's
`core/versions` and `tenant/versions` are two trees). Its counted revisions have one parent, a schema change in
`upgrade()`, and are not excused (below). The rule reports only when at least 4 in 5 of a tree's counted revisions have
a `downgrade()` that does something. So a tree needs 5 counted revisions before one empty downgrade can be reported,
and a tree where 3 of 4 downgrades are implemented (75%) stays silent. Why 80%: every tree of the 8 Alembic repositories
is at 98% or more once the excused revisions are set aside (superset 169 of 171, dispatch's `tenant` 136 of 137), so
the threshold only matters for small or young trees where empty downgrades are the habit. The message gives the tree's
count ("136 of the 137 revisions ... implement theirs").

The message says to reverse `upgrade()` there, or raise when the change cannot be undone, so a rollback stops instead
of leaving the schema behind; and to leave it in a project that never runs downgrades. No primary source prefers
`raise` over `pass`; the message offers both.

It leaves alone (and does not count):

- merge revisions (`down_revision` a tuple or list);
- revisions whose `upgrade()` is empty or data-only (`op.execute`, `op.bulk_insert`, ORM sessions);
- a `downgrade()` that does anything, including one that raises or calls a helper;
- an empty `downgrade()` when `upgrade()` reads the live schema first (`sa.inspect`, `Inspector.from_engine`,
  `get_table_names`, `get_columns`, `has_table`, `get_pk_constraint`, superset's `table_has_column` and
  `get_table_column`, ...): an idempotent repair, which a second upgrade after the empty downgrade reruns without error
  (polar's `restore_event_sequences`, open-webui's `add_missing_primary_keys_to_legacy_`);
- an empty `downgrade()` with its author's own comment below the `def` line, or a docstring: a recorded decision, such
  as dify's "The corrected down revision already defines nullable columns". In these the advice to raise would break a
  downgrade that is correct as it is. A comment that starts `TODO`, `FIXME` or `XXX` records no decision and excuses
  nothing;
- revisions in test code.

**Gaps:** schema changes made outside Alembic's `op` (raw DDL in `op.execute`, a project's own `add_columns` wrapper,
sqlalchemy-searchable's `sync_trigger`) are not schema changes here, so such a revision is neither reported nor counted.
A `downgrade()` made of other no-op statements (`print(...)`, a log call) counts as implemented. Any other explaining
comment silences the rule, whatever it says. In a tree of fewer than 10 counted revisions, two empty downgrades put it
under 80% and silence the rule for the whole tree (by design: the habit is then the tree's own). The tree is a
directory, not the revision graph, so one tree spread over several `version_locations` is judged per directory.

**Sources:** Alembic cookbook, which expects migrations to reach the finished schema "and back again"
(https://alembic.sqlalchemy.org/en/latest/cookbook.html); pytest-alembic's `test_up_down_consistency`, which checks only
that every downgrade runs, so an empty one passes (https://pytest-alembic.readthedocs.io/en/latest/quickstart.html);
django-migration-linter's `RUNPYTHON_REVERSIBLE` and `RUNSQL_REVERSIBLE`, which warn on Django migrations without reverse
code.

**Overlap:** none for Alembic. Ruff, Bandit and Squawk do not read Alembic's steps; pytest-alembic's round trip passes
an empty downgrade. The rule's value over a per-file check is the tree's convention: it stays silent in projects whose
downgrades are mostly empty.

**Corpus** (8 Alembic repositories at the `MANIFEST.tsv` commits). "Candidates" are revisions with one parent, an
Alembic schema change in `upgrade()` and a downgrade of only `pass`/`...`/comments/docstring, from an independent Python
`ast` survey (Python 3.15). "Excused" are the candidates the reflection and explanation exemptions set aside.

| Repository | Commit | Tree(s): implemented / counted | Candidates | Excused | Matches | Real |
|---|---|---|--:|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 5 / 5 | 0 | 0 | 0 | - |
| jupyterhub/jupyterhub | 3ca3744 | 18 / 18 | 0 | 0 | 0 | - |
| mealie-recipes/mealie | 8dbb928 | 59 / 59 | 0 | 0 | 0 | - |
| open-webui/open-webui | 0666e59 | 68 / 68 | 2 | 2 | 0 | - |
| Netflix/dispatch | dd2837e | `core` 7 / 7, `tenant` 136 / 137 | 2 | 1 | 1 | 1 |
| polarsource/polar | 872272f | 186 / 186 | 1 | 1 | 0 | - |
| langgenius/dify | 724ac15 | 235 / 235 | 3 | 3 | 0 | - |
| apache/superset | fdf6172 | 169 / 171 | 8 | 6 | 2 | 2 |

3 matches, all real (precision 3 of 3; a small sample):

- `Netflix__dispatch/src/dispatch/database/revisions/tenant/versions/2023-11-29a_bdaeabba3e53.py`: `upgrade()` drops
  the index `signal_instance_search_vector_idx` and the column `signal_instance.search_vector`; `downgrade()` is only
  autogenerate markers and `pass`. After a downgrade the next upgrade fails dropping an index that no longer exists.
- `apache__superset/superset/migrations/versions/2020-12-23_11-34_c878781977c6_alert_reports_shared_uniqueness.py`:
  `upgrade()` drops `uq_report_schedule_name` and creates `uq_report_schedule_name_type`; `downgrade()` is `pass`, so a
  downgrade leaves the new constraint and the re-upgrade fails dropping the old one.
- `apache__superset/superset/migrations/versions/2016-03-13_21-30_18e88e1cc004_making_audit_nullable.py`: `upgrade()`
  drops four columns of `columns` and relaxes NOT NULL on a dozen more; `downgrade()` is `pass`.

The 13 excused candidates were read: each is an inspector-guarded repair (open-webui 2, polar 1, dify 1, dispatch
`928b725d64f6`, superset 6) or explains its empty downgrade (dify 2), and none would fail a re-upgrade. Reporting them
would have made precision 3 of 16. An estimate given before the rule existed counted 82 empty downgrades with schema
operations (superset 52, dispatch 13); the survey above finds 16. Of dispatch's 18 empty downgrades, 3 are merges, 2
follow an empty `upgrade()`, 7 follow data-only upgrades (ORM sessions, `op.execute`), 4 follow sqlalchemy-searchable's
`sync_trigger` (a schema change outside `op`: a gap, and idempotent on re-upgrade) and 2 are candidates. Of superset's
104 empty downgrades, 47 are merges, 4 follow an empty `upgrade()`, 45 change no schema through `op` (43 run ORM
sessions or data SQL over chart and dashboard JSON; `a39867932713` and `07f9a902af1b` run raw `ALTER TABLE` through
`execute`: a gap) and 8 are candidates.

An edit-window check: a new revision added to a copy of polar's `server/migrations` (`op.add_column` with `pass` in
`downgrade()`) is reported with "186 of the 187 revisions ... implement theirs".

Counts: before the review's fixes, the five smaller repositories were run whole with `corpus-run` and polar, dify and
superset with the engine on their migration directories; after them (a bare `return` is filler, a TODO comment excuses
nothing), all 8 with `corpus-run` (polar, dify and superset on the review's trimmed copies; runs `alembic-fix-mine-*`):
the same 3 matches. The per-tree figures come from the engine (a copy of the rule that also reports the tree counts
and the excused revisions).

Mode: `log`. The read matches are all real (code and actionable 3 of 3), but there are only 3, and the exemptions that
remove the 13 deliberate no-ops are heuristics (a reflection call, a comment).

**On #1120's engine (benchmark).** No new matches; dispatch's match is reported the same.
