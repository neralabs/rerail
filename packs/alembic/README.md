# Rerail rule pack: Alembic

The `alembic-practice-*` rules are Rerail's own; none is a port of a linter rule. They catch the Alembic migrations
that take a live PostgreSQL database down, fail at deploy, or lose data: index builds and constraint checks that lock
out writes, NOT NULL columns that cannot be added to a populated table, type changes that rewrite a table, columns
dropped while the models still map them, revisions that fork the history, and data migrations that import the
application's models. Each rule is advice that the PostgreSQL, Alembic or SQLAlchemy documentation gives, or that
Squawk, strong_migrations, django-migration-linter and well-known migration post-mortems give, from at least two
independent sources ([NOTES-alembic.md](NOTES-alembic.md)). Squawk and django-migration-linter check the same hazards in
SQL files and Django migrations; neither reads Alembic's Python, so neither sees a `batch_alter_table` block, which
tables the same revision creates, the models a dropped column still belongs to, or the `env.py` that runs the
migrations. Ruff and Bandit check none of these shapes.

Each rule reads the repository's Python files through the pack's library [`rules/lib/alembic.rerail`](rules/lib/alembic.rerail):
`op.*` and `batch_op.*` operations resolved through `from alembic import op`, the table each acts on (a literal,
`op.f(...)` or a module constant), the tables the same `upgrade()` creates, `autocommit_block()` regions, the branches
of a dialect test that run only on SQLite or MySQL, SQLAlchemy and sqlmodel model classes with their columns (mixins in
other modules included), revision ids, and env.py.

Each rule is proven three ways: by its cases in `cases/<id>/`, where a line ending in `# caught` must be reported and no
other line may be (`rerail rules test <file>.rerail`, which reads the case directory as one repository); by a run on the
8 open-source repositories of the corpus that use Alembic ([the corpus](#the-corpus)), whose matches were read and judged
real or not (every match, or a spread sample covering every repository); and by an independent review
([REVIEW-alembic.md](REVIEW-alembic.md)). A match is real when the file holds the pattern, even when the table is small
enough that the lock is harmless there. A rule starts on only when at least 90% of its read matches were real.

| Rule | Mode | Catches | Matches (repos) on 8 repositories | Precision of read matches |
|---|---|---|---|---|
| `alembic-practice-index-not-concurrent` | Steer | an index or unique constraint built on an existing table without CONCURRENTLY | 296 (6) | 100% (296 of 296) |
| `alembic-practice-concurrent-index-outside-autocommit` | Steer | `postgresql_concurrently=True` run inside the migration's transaction, outside `autocommit_block()` | 0 (0 of 7 run) | no matches; 224 wrapped concurrent calls left alone |
| `alembic-practice-not-null-without-server-default` | Steer | a NOT NULL column added to an existing table with no `server_default` (and `default=` mistaken for one) | 13 (3) | 100% (13 of 13) |
| `alembic-practice-drop-mapped-column` | Steer | a column or table dropped or renamed while a model in the repository still maps it | 0 (0) | no matches at release commits |
| `alembic-practice-constraint-without-not-valid` | Steer | a foreign key or CHECK added to an existing table without `postgresql_not_valid=True` | 221 (8) | 100% (221 of 221) |
| `alembic-practice-column-type-rewrite` | Steer | a column type change that rewrites or scans the table (widenings left alone) | 34 (6) | 100% (34 of 34) |
| `alembic-practice-set-not-null-scan` | Log | `alter_column(nullable=False)` on an existing table: a full scan under ACCESS EXCLUSIVE | 77 (6) | 96% (74 of 77) |
| `alembic-practice-missing-lock-timeout` | Log | a revision that takes a write-blocking lock on an existing table while neither it nor its env.py sets lock_timeout | 579 (8) | 100% of 220 read hold the pattern; 70% (155) where a lock queue can form (jupyterhub and mealie migrate with no traffic) |
| `alembic-practice-multiple-heads` | Steer | two revisions that fork one migration tree with no merge revision: `alembic upgrade head` refuses to run | 0 (0) | no matches at release commits |
| `alembic-practice-rename-as-drop-add` | Log, starts off | one upgrade that drops a column and adds one of the same type on the same table with no data copy: a rename autogenerate rendered as drop and add | 2 (1) | 50% (1 of 2) |
| `alembic-practice-model-import-in-migration` | Steer | a migration that imports an application ORM model instead of declaring the table it needs | 13 (4) | 100% (13 of 13) |
| `alembic-practice-empty-downgrade` | Log | a downgrade() that is only pass while upgrade() changes the schema, in a tree whose other revisions implement downgrade | 3 (2) | 100% (3 of 3) |

Matches are the engine's findings, every one read unless the rule's details say a spread sample was read. Each rule
details its gaps and the shapes it leaves alone below. Rules in Log mode record matches without steering the agent;
the details give the reason.

## Starting values

11 of 12 rules start on: 8 steer and 3 log. `set-not-null-scan` logs because about half its matches are on tables
that stay small and the safe path spans several revisions; `missing-lock-timeout` logs because 7 of the 8 repositories
never set `lock_timeout`, so it matches nearly every migration there, and whether a lock queue can form depends on how
the project deploys (jupyterhub and mealie migrate with no traffic); `empty-downgrade` logs on a sample of 3.
`rename-as-drop-add` starts off (`"on": false`): one of its two corpus matches was a deliberate replacement, below the
90% bar. Three rules have no corpus matches by design (`concurrent-index-outside-autocommit`, `drop-mapped-column`,
`multiple-heads`): at a release commit every repository already gets these right, and the bug exists only between
writing a migration and finishing the change, which is when Rerail checks an edit. Each was proven by an edit-window
test on a copy of a corpus repository (see its details).

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

Rerail's own rule, not a port. In an Alembic revision's `upgrade()` (or a module-level function of the same file that
`upgrade()` calls), it reports an index built on a table that already holds rows without `postgresql_concurrently=True`:

- `op.create_index(...)` and `batch_op.create_index(...)` on the handle of `with op.batch_alter_table("t") as batch_op:`,
  with or without `unique=True`, with the table given positionally, as `table_name=`, through `op.f("...")` or as a
  module constant; `postgresql_concurrently=False` counts as missing.
- `op.create_unique_constraint(...)` and `batch_op.create_unique_constraint(...)`: ADD CONSTRAINT ... UNIQUE builds its
  index under the same lock. This match has its own message: build the unique index concurrently, then attach it with
  `ALTER TABLE ... ADD CONSTRAINT ... UNIQUE USING INDEX ...` if a constraint is needed.

A table "already holds rows" when the same `upgrade()` does not create it: not with `op.create_table("t", ...)`
(in any branch, a dialect branch included) and not with a project's own `create_table("t", ...)` wrapper (superset's
`superset.migrations.shared.utils.create_table`). Both messages name the two parts of the fix and their cost:
`autocommit_block()` commits whatever the migration ran before it, so the index belongs in its own revision or after
the transactional steps; a concurrent build that fails leaves an INVALID index, so drop it first with
`op.drop_index(..., if_exists=True, postgresql_concurrently=True)`, as polarsource/polar's
`server/migrations/versions/2026-07-08-1636_add_index_on_files_path.py` does. Both say when to leave the code alone: a
table known to stay small.

It leaves alone: indexes on a table the same `upgrade()` creates; `postgresql_concurrently=True` or any expression
other than a literal `False` (whether a concurrent build sits inside `autocommit_block()` is
`alembic-practice-concurrent-index-outside-autocommit`'s concern); code in a branch that runs only on another database
(the else of `if _is_pg(conn):` / `if dialect.name == "postgresql":`, the body of `if dialect.name == "sqlite":` or
`!= "postgresql"`, the else of `if not isinstance(bind.dialect, SQLiteDialect):`; dialect classes count, case aside),
and code after an early-return guard that leaves only another database (`if not _is_mysql(): return`, dify's
`3edd6cd27f54`); `downgrade()`; revisions under `tests/`; files without module-level `revision` and
`down_revision`; an `op` that is not `alembic.op`. `postgresql_concurrently` is ignored on SQLite and MySQL, so the fix
is safe for projects that also run there (mealie, dify, open-webui, superset).

**Gaps:** `downgrade()` (a rollback re-creates indexes on the rare path); a table name the rule cannot resolve (a
function parameter, an f-string); a table created by an earlier revision of the same release, or by
`op.execute("CREATE TABLE ...")`, counts as existing; index helpers in another module (superset's
`migrations.shared.utils.create_index(table, name, columns)`, which most of its recent revisions use, is not followed);
the size of a table is not known, so the message says when to leave it.

**Corpus** (BRIEF commits; a match is real when the file builds that index on a table that already exists):

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 0 (its one index is on a table the same `upgrade()` creates) | – |
| jupyterhub/jupyterhub | 3ca3744 | 0 (no index operations) | – |
| Netflix/dispatch | dd2837e | 5 (3 index, 2 unique) | 5 |
| open-webui/open-webui | 0666e59 | 29 (27 index, 2 unique) | 29 |
| polarsource/polar | 872272f | 17 (11 index, 6 unique) | 17 |
| mealie-recipes/mealie | 8dbb928 | 151 (136 index, 15 unique) | 151 |
| langgenius/dify | 724ac15 | 73 (63 index, 10 unique) | 73 |
| apache/superset | fdf6172 | 21 (5 index, 16 unique) | 21 |
| Total | | 296 | 296 |

Every match was read (grouped by file: the whole `upgrade()`, the tables it creates, renames or reflects, and its
dialect branches). The first runs had 311 matches; three false-positive shapes were fixed and added to the cases:
superset's own `create_table(...)` wrapper (13 matches on tables the same revision creates), superset's
`if not isinstance(bind.dialect, SQLiteDialect): ... else:` (1 match in the SQLite-only branch of `c878781977c6`), and
dify's `if not _is_mysql(): return` guard (1 match on MySQL-only code).

Real matches, for example: open-webui `backend/open_webui/migrations/versions/9f6c2a8d1b43_add_channel_lookup_indexes.py`
(three plain indexes on `message`), `d4c1a8e37b62_add_chat_timer_at_and_chat_indexes.py` (five on `chat`); dify
`api/migrations/versions/2026_01_09_1630-905527cc8fd3_add_workflow_run_created_at_id_idx.py` (`workflow_runs`),
`2026_02_26_1336-e288952f2994_add_partial_indexes_on_conversations_.py` (`conversations`); polar
`server/migrations/versions/2026-07-06-1024_add_subscription_pause_columns.py` (two indexes on `subscriptions`, whose
own comment counts about 282K rows) and the six `2026-09-17-165x_*_token_v2.py` revisions (`create_unique_constraint` on
session and token tables); dispatch `src/dispatch/database/revisions/tenant/versions/2023-10-23_f2605bfc1f59.py` (a GIN
index on `signal_instance`); superset `superset/migrations/versions/2019-07-11_19-02_def97f26fdfb_add_index_to_tagged_object.py`.
Harmless in context, though real: polar's matches on small tables (`oauth2_clients`, `backoffice_users`,
`organization_emails`, created two revisions earlier) run under `SET LOCAL lock_timeout = '5s'`, and polar uses the
concurrent form where a table is large; mealie is self-hosted, and its 97-index revision `ff5f73b01a7a` runs on small
databases. The message's "fine for a table that stays small" covers these.

**Sources:** PostgreSQL, [CREATE INDEX](https://www.postgresql.org/docs/current/sql-createindex.html) ("locks out
writes (but not reads) on the table until it's done"; CONCURRENTLY cannot run in a transaction block; a failed
concurrent build leaves an INVALID index); Alembic,
[`MigrationContext.autocommit_block`](https://alembic.sqlalchemy.org/en/latest/api/runtime.html); Squawk
[require-concurrent-index-creation](https://squawkhq.com/docs/require-concurrent-index-creation) and
[disallowed-unique-constraint](https://squawkhq.com/docs/disallowed-unique-constraint) (SQL files only);
[strong_migrations](https://github.com/ankane/strong_migrations) "Adding an index non-concurrently" and "Adding a unique
constraint"; django-migration-linter CREATE_INDEX and ADD_UNIQUE (Django only). No Ruff or Bandit rule reads Alembic
operations; this rule adds what Squawk cannot see in Python: the tables the same revision creates, and the table of a
`batch_alter_table` block.

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
commits what the revision ran before it, so concurrent index work belongs in its own revision or after the other
steps. This rule also guards the steer of `alembic-practice-index-not-concurrent`: an agent told to build an index
concurrently often adds only the keyword.

It leaves alone: calls inside `with <ctx>.autocommit_block():` at any depth (an `if` or a `batch_alter_table` in it,
a helper called from it, a MigrationContext held in a local); `postgresql_concurrently=False`, absent, or any
expression other than a literal `True`; code in a branch that runs only on another database (the else of
`if _is_pg(conn):` / `if dialect.name == "postgresql":`, the body of `if dialect.name == "sqlite":`); revisions under
`tests/`; files without module-level `revision` and `down_revision`; an `op` that is not `alembic.op`.

**Across files.** An `env.py` whose `context.configure(...)` passes `transactional_ddl=False` runs its revisions
without a transaction, so CONCURRENTLY works as written: the rule reports nothing in the revisions under that env.py's
directory, nor in revisions under no env.py's directory of the run when such an env.py exists. Other Alembic trees of
the same repository, with their own env.py, are still read (a repository can hold several: Netflix/dispatch's core
and tenant schemas). `transaction_per_migration=True` still opens a transaction per revision: still reported.

**Gaps:** raw SQL (`op.execute("CREATE INDEX CONCURRENTLY ...")`) is not read (Squawk reads SQL); the old
`op.execute("COMMIT")` workaround is not recognised (none in the corpus); helpers in another module, or two calls
deep, are not followed; an env.py outside the run, or `transactional_ddl` set through a variable or a `**kwargs`
dict (superset's env.py sets it that way, for SQLite and MySQL only), is not seen.

**Corpus** (BRIEF commits):

| Repository | Commit | Concurrent index calls | Inside an autocommit block | Matches |
|---|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 0 | – | 0 |
| jupyterhub/jupyterhub | 3ca3744 | 0 | – | 0 |
| mealie-recipes/mealie | 8dbb928 | 0 | – | 0 |
| open-webui/open-webui | 0666e59 | 0 | – | 0 |
| Netflix/dispatch | dd2837e | 0 | – | 0 |
| polarsource/polar | 872272f | 213 | 211, and 2 in helpers called only from inside one | 0 |
| langgenius/dify | 724ac15 | 11 | 11 | 0 |

No match, so no precision to measure: every repository that uses concurrent index operations wraps them, which is
the practice the rule enforces. A probe of the same logic over polar's and dify's migration directories saw all 213
and 11 concurrent calls (the rule reads them) and kept every one silent. polar's
`server/migrations/versions/2026-11-12-1405_add_manual_benefit_grant_timestamps.py` is the shape that needs the
helper reading: its `_create_index` and `_drop_index` call `op.create_index(..., postgresql_concurrently=True)`
outside any `with`, and `upgrade()` and `downgrade()` call them only inside `autocommit_block()`; a reading by step
alone would have reported both. superset (fdf6172) was not run: a Python `ast` survey finds no concurrent index call
in it, and its env.py sets `transactional_ddl` through a kwargs dict, for SQLite and MySQL only.

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
(positional or `table_name=`/`column=` keywords), on a table `upgrade()` does not create, where the Column passes
`nullable=False` or `primary_key=True` and no `server_default` (or `server_default=None`). A Column with `default=`
gets the second message, which explains that `default=` is Python-side only. `sa.Column` and a bare `Column` resolve
through imports (`sqlalchemy.Column`, `sqlalchemy.schema.Column`, `sqlmodel.Column`).

It leaves alone: any `server_default=` value (`""`, `"0"`, `sa.text(...)`, `sa.false()`); nullable columns (omitted or
`nullable=True`), including the add-nullable, backfill, `alter_column(..., nullable=False)` sequence; tables the same
`upgrade()` creates; the branch of a dialect test that runs only on another database (`else:` of dify's `if
_is_pg(conn):`, the body of `if dialect.name == "sqlite":`; MySQL fills a NOT NULL column with an implicit default);
generated columns (`sa.Computed(...)`, `sa.Identity(...)`); an integer `primary_key=True` column without
`autoincrement=False`, which SQLAlchemy renders as `SERIAL`/`BIGSERIAL` on PostgreSQL (checked with Alembic's offline
SQL: `ADD COLUMN id2 SERIAL NOT NULL`), so the sequence numbers the existing rows; `downgrade()`; test files; files
that are not Alembic revisions; an `op` that is not `alembic.op`.

**Gaps:** a Column built into a variable before `add_column`; `nullable=` given as an expression; a table created by
an earlier revision that no database has run yet is still empty, but is reported (the message says the column is
safe as written only when no database that runs the migration can hold rows in the table). On MySQL before 8.0.13 a
TEXT/BLOB/JSON column takes no server default; the `default=` message names the nullable-backfill route for that case.

**Sources:** Squawk, [adding-required-field](https://squawkhq.com/docs/adding-required-field) ("will fail
immediately upon running for any populated table"); Alembic, [`add_column`](https://alembic.sqlalchemy.org/en/latest/ops.html)
(a server default uses `server_default`, "and not `default` which only specifies Python-side defaults"); SQLAlchemy,
[Column INSERT/UPDATE Defaults](https://docs.sqlalchemy.org/en/21/core/defaults.html); django-migration-linter's
NOT_NULL check, which calls the framework-default-is-a-database-default belief a "common misconception".

**Overlap:** Squawk's adding-required-field reads the SQL; nothing in Ruff or Bandit. This rule reads the migration
before any SQL exists: the Column's keywords (`default=` against `server_default=`), the dialect branch, and which
tables the same `upgrade()` creates.

**Corpus:** 13 matches in the 8 Alembic repositories, every one read; the same 13 as an independent Python `ast`
survey of every `upgrade()`. The 6 dify `else:` branches of `if _is_pg(conn):` that pass only `default=` (e.g.
`api/migrations/versions/2024_08_13_0633-63a83fcf12ba_support_conversation_variables.py:58`) are left alone.

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
missed: its comment says "Step 1: Add column without NOT NULL constraint" above `nullable=False`. Precision: 13 of 13
by the code, 4 of 13 counting only tag-to-tag upgrades.

### `alembic-practice-drop-mapped-column`

Rerail's own rule, not a port. It reads a migration against the application's models: in `upgrade()` (or a
module-level helper it calls by name) of an Alembic revision outside test code, it reports

- `op.drop_column("users", "nick")` / `batch_op.drop_column("nick")` (table from `op.batch_alter_table("users")`) while
  a model still maps `users.nick`;
- `op.alter_column("users", "nick", new_column_name="handle")` (or the batch form) while a model still maps `nick`;
- `op.drop_table("users")` / `op.rename_table("users", "people")` while a model still maps `users`.

A table name is a literal, `op.f("...")`, or a module constant. A model maps `users` when an application class (not in
a migration, `env.py` or test file) has `__tablename__ = "users"`, or is a sqlmodel `table=True` class named `User`
(table `user`). It maps `nick` when it, or a mixin or base class anywhere in the repository, declares `nick =
Column(...)`, `nick: Mapped[str] = mapped_column(...)`, a bare `nick: Mapped[str]`, a sqlmodel field, a Column whose
explicit name is `nick` (`Column("nick", ...)`, `mapped_column(..., name="nick")`) under any attribute, or a Column
wrapped in `deferred(...)`/`column_property(...)`. A `Table("users", metadata, Column("nick", ...))` in application
code (an association table, or a class's `__table__ = Table(...)`) maps the table and its columns too. The message
says which of the four it is and names the expand/contract order: stop mapping it and deploy first, drop it in a later
revision; for a rename, add the new column, write both and backfill, move the model, and drop the old one later. It
says when to leave it: the model change ships in the same change and the deploy runs migrations only after the new
code is live, or the app is stopped while migrations run.

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
so the rule sees none of its models); imperative mappings (`registry.map_imperatively`); a table or column named by a
loop variable or other non-literal expression (superset's composite-PK migration drops association tables from a
loop); `schema=` (a model of another schema with the same table name still counts); a model that maps a column only
through `__table_args__` or reflection; a mixin from an installed library (authlib's `OAuth2ClientMixin`) is outside
the run, so its columns are not known to be mapped; a re-add by a revision whose `down_revision` is not a literal.

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
rule):

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

0 matches at the heads, as expected: at a release the models and the migrations agree. The bug lives in the window
between writing the migration and editing the model, which is when Rerail steers. No historical drop of a column that a
later revision adds back occurs in these repositories (an independent `ast` survey agrees); the descendant exemption is
proven on the cases only. The first version reported jupyterhub's `api_tokens` reset (harmless: the hub is stopped for
`upgrade-db` and `create_all()` rebuilds the table); the both-directions exemption removed it. polar's eight
`drop_column(..., "token")`-style revisions of September 2026 stay silent because the models map the replacements under
explicit names (`mapped_column("token_v2", ...)`). Edit-window check: a new revision added to a copy of mealie that drops
`shopping_list_items.is_food` (batch), `shopping_list_items.created_at` (declared on the `SqlAlchemyBase` base class)
and the table `shopping_list_recipe_reference` is reported on all three lines, and a drop of an unmapped column is not;
after `is_food` is removed from the model, its line is silent. The same check on dify reports `op.drop_column("apps",
"name")`. dify was read as the 306 files the rule needs (`api/models`, `api/migrations`, and every other non-test file
that declares `__tablename__` or a `Table(...)`), because the shared corpus queue held 90+ runs.

### `alembic-practice-constraint-without-not-valid`

Rerail's own rule, not a port. In an Alembic revision's `upgrade()` (or a module-level function of the same file that
`upgrade()` calls), it reports a FOREIGN KEY or CHECK constraint added to a table that already holds rows without
`postgresql_not_valid=True`:

- `op.create_foreign_key(...)` whose source table `upgrade()` does not create, with the table given positionally, as
  `source_table=`, through `op.f("...")` or as a module constant, and `batch_op.create_foreign_key(...)` on the handle
  of `with op.batch_alter_table("t") as batch_op:`. A source table that exists with a referent table the same
  `upgrade()` creates is still reported: the existing rows are what the add checks.
- `op.create_check_constraint(...)` and `batch_op.create_check_constraint(...)` on an existing table.
- `postgresql_not_valid=False` counts as missing.

Adding either constraint checks every existing row, and "other updates to the table are locked out until the ALTER
TABLE ADD CONSTRAINT command is committed"; ADD FOREIGN KEY also takes a SHARE ROW EXCLUSIVE lock on the referenced
table (PostgreSQL, ALTER TABLE). With NOT VALID the add skips the scan; a later VALIDATE CONSTRAINT scans under SHARE
UPDATE EXCLUSIVE, which lets writes through. The message names both steps: pass `postgresql_not_valid=True`, then run
`op.execute("ALTER TABLE <t> VALIDATE CONSTRAINT <name>")` once the add has committed, either inside
`with op.get_context().autocommit_block():` (which commits the steps before it, polar's pattern in
`server/migrations/versions/2026-10-15-1418_add_reducer_bucket_billed_at_and_order_id.py`) or in a revision deployed
later. A later revision in the same `alembic upgrade` run does not help by itself: by default Alembic runs a whole
upgrade in one transaction, so the ADD's lock would still be held. VALIDATE CONSTRAINT is PostgreSQL SQL, so the
message says to run it on PostgreSQL only; `postgresql_not_valid` itself is ignored on SQLite and MySQL. The message
names the constraint when the rule can read it (`<constraint name>` for `None` under a naming convention) and says
when to leave the code alone: a table known to stay small.

It leaves alone: constraints on a source table the same `upgrade()` creates (with `op.create_table` in any branch, or a
project's own `create_table("t", ...)` wrapper, superset's); constraints declared inside `op.create_table(...)`
(`sa.ForeignKey`, `sa.CheckConstraint`), which are part of CREATE TABLE; `postgresql_not_valid=True` or any expression
other than a literal `False`; code in a branch that runs only on another database (lib/alembic `al_non_pg`: the else
of a PostgreSQL test, the body of a SQLite/MySQL test, the else of `if not isinstance(bind.dialect, SQLiteDialect):`,
code after `if not _is_mysql(): return`); `downgrade()`; revisions under `tests/`; files without module-level
`revision` and `down_revision`; an `op` that is not `alembic.op`.

**Gaps:** `downgrade()`; a table name the rule cannot resolve (a loop variable or a function parameter: jupyterhub
`833da8570507_rbac.py`, superset `2017-10-03_14-37_4736ec66ce19_.py`, polar's `_ensure_foreign_key` helpers); a table
created by an earlier revision of the same release counts as existing (dify's `knowledge_fs_*` tables, created by
`2026_07_21_1300-b7f2a9d41c60` and constrained by three revisions the same day); constraints added through
`op.execute("ALTER TABLE ... ADD CONSTRAINT ...")` or a helper in another module are not read; the size of a table is
not known, so the message says when to leave it.

**Corpus** (BRIEF commits; each repository's migration directory run with `rerail-engine logic`; a match is real when
the revision adds that validated constraint to a table an earlier revision created):

| Repository | Commit | Matches | Real |
|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a | 2 FK | 2 |
| jupyterhub/jupyterhub | 3ca3744 | 2 FK | 2 |
| mealie-recipes/mealie | 8dbb928 | 10 FK | 10 |
| open-webui/open-webui | 0666e59 | 2 (1 FK, 1 check) | 2 |
| Netflix/dispatch | dd2837e | 163 FK | 163 |
| polarsource/polar | 872272f | 5 FK | 5 |
| langgenius/dify | 724ac15 | 17 (1 FK, 16 check) | 17 |
| apache/superset | fdf6172 | 20 (16 FK, 4 check) | 20 |
| Total | | 221 | 221 |

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
`backend/app/alembic/versions/1a31ce608336_add_cascade_delete_relationships.py` (FK on `item`). Harmless in context,
though real by this definition: dify's 13 checks on `knowledge_fs_*` tables created hours earlier in the same series
(likely empty when the release deploys), mealie's foreign keys on new all-NULL columns of a self-hosted app's small
tables, and polar's 5 under `SET LOCAL lock_timeout = '5s'`. Excluding dify's 13, 208 of 221 (94%) are worth the
change; the message's "fine for a table that stays small" covers the rest.

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
- a length limit added or shrunk: keep the column's type and add `CHECK (char_length(c) <= n) NOT VALID`, then
  `VALIDATE CONSTRAINT` later, or go through a new column.

Both say when to keep the change as written: a table that stays small (or a maintenance window).

It leaves alone the changes PostgreSQL makes without touching the rows (Squawk, strong_migrations): VARCHAR/String(n)
to a length m >= n, to an unlimited `String()` or to `Text`/`UnicodeText`; `Text` to `String()` without a length;
`Numeric(p, s)` to `Numeric(p2, s)` with p2 >= p, or to an unconstrained `Numeric()`; `LargeBinary` to
`LargeBinary(length=...)` (both BYTEA); the same type text on both sides. `MediumText`/`LongText` count as TEXT: superset
defines them as `Text().with_variant(MEDIUMTEXT(), "mysql")` (`superset/utils/core.py`), dify's `models.types.LongText`
is a TypeDecorator that loads TEXT on PostgreSQL. Also left alone: a target from `sqlalchemy.dialects.mysql`
(`mysql.LONGTEXT`); a length that is not an integer literal where it decides (superset's `sa.String(current_length)`);
an `existing_type` that is not a type expression (a conditional); a call without `existing_type` whose target is a
string, text or numeric type, which may be a widening the rule cannot judge (a call without `existing_type` and any
other target is reported; the 80 corpus calls all pass `existing_type`); a table the same `upgrade()` creates; the
branch of a dialect test that runs only on another database (`else:` of `if is_postgres():`, the body of `if
isinstance(bind.dialect, MySQLDialect):`); a call in an `except` handler (superset's non-Postgres fallback after a
`try:` that made the change); `alter_column` without `type_=`; `downgrade()`; test files; an `op` that is not
`alembic.op`.

**Gaps:** types are judged by the last name of their callee, not resolved through imports, so a project type that
shadows `String` or `Text` is read as SQLAlchemy's; a table named by a variable is not seen; `DateTime()` to
`DateTime(timezone=True)` is reported although PostgreSQL 12+ skips that rewrite when the session time zone is UTC; a
`postgresql_using=` on an otherwise safe widening is left alone with it; a table created by an earlier revision that
no database has run yet is still empty, but is reported.

**Sources:** PostgreSQL, [ALTER TABLE](https://www.postgresql.org/docs/current/sql-altertable.html) (notes on
`SET DATA TYPE`); Squawk, [changing-column-type](https://squawkhq.com/docs/changing-column-type) (VARCHAR to TEXT and
VARCHAR(5) to VARCHAR(10) are safe, INT to BIGINT is not); strong_migrations, "Changing the type of a column" (the safe
changes: increasing or removing a string limit, string to text, increasing numeric precision at the same scale).

**Overlap:** Squawk's changing-column-type flags every type change in SQL; this rule reads the migration before any
SQL exists, compares `type_=` with `existing_type=`, and leaves the safe widenings, the MySQL-only branches and the
tables the same `upgrade()` creates alone. Nothing in Ruff or Bandit.

**Corpus:** 34 matches in the 8 Alembic repositories, every one read; the engine was run directly on each
repository's migration directory (the rule reads one revision at a time). An independent Python `ast` survey found 80
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
short and the message's "keep it for a table that stays small" applies. Precision: 34 of 34 by the code.

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

It leaves alone: an `alter_column` the same `upgrade()` puts behind a validated check, that is, an `op.execute(...)`
that runs `VALIDATE CONSTRAINT n` where `n` names the column (`ck_refunds_reason_not_null`) or where the same file adds
`n` as `CHECK (c IS NOT NULL)` (polar's `ADD CONSTRAINT {NOT_NULL_CHECK} CHECK (c IS NOT NULL) NOT VALID` with
`VALIDATE CONSTRAINT {NOT_NULL_CHECK}`, or a `create_check_constraint(N, "t", "c IS NOT NULL", postgresql_not_valid=True)`
validated by name); a `create_check_constraint` on `t` whose condition says `c IS NOT NULL` added validated in the same
file (its own scan is `alembic-practice-constraint-without-not-valid`'s concern); `existing_nullable=False` with
`nullable=False` (the column is already NOT NULL; PostgreSQL does nothing); `nullable=True` or no `nullable=`; tables
the same `upgrade()` creates; the branch of a dialect test that runs only on another database (dify's `else:` of
`if _is_pg(conn):`); `downgrade()`; test files; files that are not Alembic revisions; an `op` that is not `alembic.op`.
A VALIDATE of an unrelated constraint (a foreign key) does not exempt the column.

**Gaps:** `op.execute("ALTER TABLE ... SET NOT NULL")` is not read; a table created by an earlier revision of the same
release counts as existing; a column already NOT NULL that the call does not mark `existing_nullable=False` is reported
(a no-op on PostgreSQL); a table that the same transaction already rewrites through raw SQL (dispatch's
`ADD COLUMN id SERIAL PRIMARY KEY`) is reported, though the safe sequence does not shorten that lock; a VALIDATE of
another constraint whose name contains the column's name exempts it. Whether a table stays small cannot be read from
the migration, so the message says to leave a small table as it is.

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

**Corpus:** 77 matches in the 8 Alembic repositories, every one read; run with `corpus-run` on each repository's
migration directory (the rule reads one revision at a time). The engine's matches equal an independent Python `ast`
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

Precision: 74 of 77 (96%) are SET NOT NULL scans of an existing table that the safe sequence avoids. The 3 that are
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
applies; that, and a fix that spans two revisions, is why the rule starts in `log`.

### `alembic-practice-missing-lock-timeout`

Rerail's own rule, not a port. It reports an Alembic revision whose `upgrade()` (or a module-level function of the
same file that `upgrade()` calls) takes a table lock on a table that already holds rows, when nothing sets
PostgreSQL's `lock_timeout` for it. Once per revision, at the first such operation in the file:

- `add_column`, `drop_column`, `alter_column`, `create_foreign_key`, `create_check_constraint`,
  `create_unique_constraint`, `create_primary_key`, `drop_constraint`, `rename_table`, `drop_table`, and
  `create_index` / `drop_index` without `postgresql_concurrently`, as `op.m(...)` or on the handle of
  `with op.batch_alter_table("t") as batch_op:`.
- A table "already holds rows" when the same `upgrade()` does not create it (`op.create_table` or a project's own
  `create_table(...)` wrapper), as in the pack's other existing-table rules.

A DDL statement waiting for its lock queues every later query on the table behind it, and `lock_timeout` defaults to
0, wait forever: one long transaction on the table turns the migration into an outage. The message names the fix,
`op.execute("SET LOCAL lock_timeout = '5s'")` as the first statement of `upgrade()` or `SET lock_timeout` once in
env.py before `context.run_migrations()`, guarded to PostgreSQL for projects that also run on SQLite or MySQL (where
the statement fails), and a retry of the deploy when it times out; and it says when to leave it: a database that
serves no traffic while migrations run.

Nothing is reported for a revision when lock_timeout is set:

- in `upgrade()`, above the operation (a `SET` below it does not protect the operations before it, which are
  reported), or anywhere in the file outside `upgrade()` and `downgrade()` (a module-level helper or constant);
- through a call in `upgrade()` of a function of the run that sets it (`from migrations.shared.locks import
  set_lock_timeout`), or a module constant of the run that holds it (`op.execute(locks.LOCK_TIMEOUT)`): the import is
  followed by qualified name, without the call graph;
- in env.py: the env.py whose directory holds the revision (a repository can hold several Alembic trees, as Netflix
  dispatch's `core` and `tenant` do), or, for a revision under no env.py's directory (`version_locations`), any env.py
  of the run. `SET lock_timeout`, `SET LOCAL lock_timeout` and `connect_args={"options": "-c lock_timeout=..."}` all
  count.

A setting is a string that mentions `lock_timeout`; one that sets it to 0 or resets it does not count, nor does a
docstring. It leaves alone: operations on a table the same `upgrade()` creates; concurrent index builds; code in a
branch that runs only on another database (lib/alembic's `al_non_pg`); revisions whose `upgrade()` runs only data
statements (`op.execute("UPDATE ...")`, `bulk_insert`); `downgrade()` (and a lock_timeout set only there does not
protect `upgrade()`); revisions under `tests/`; files without module-level `revision` and `down_revision`.

**Gaps:** raw DDL in `op.execute("ALTER TABLE ...")` is not read; a foreign key from a new table also locks its
existing referent (SHARE ROW EXCLUSIVE), which is not reported; a `SET LOCAL` at the top of `upgrade()` ends at the
commit of a later `autocommit_block()`, so non-concurrent operations inside that block run without it, unseen; a
lock_timeout set through a package outside the run, through a variable, or by the deploy (a role's or database's
default, `PGOPTIONS`) is not seen, and the revision is reported; operations on a table named by a loop variable or
parameter are not seen (jupyterhub's `4621fec11365`); project helpers in another module that wrap the operations
(superset's `migrations.shared.utils.add_columns`, `drop_columns`, `create_index`) are not followed; a table created
by an earlier revision of the same release counts as existing.

**Mode: log.** Seven of the eight Alembic repositories in the corpus never set lock_timeout, so the rule matches
nearly every revision that alters a table there: a repository convention, not a slip an agent made in one edit. In
two of them (jupyterhub, mealie) migrations run with no other traffic by design, which the message's "leave it" covers
but which the rule cannot see.

**Corpus** (BRIEF commits; `corpus-run` over the repository root, except polar and dify: `corpus-run` over a copy of
`server/migrations` and `api/migrations`, env.py included, while the large-repository queue was full; neither tree
imports a helper from outside it; a match is real when the revision takes that lock on a table that already exists, nothing sets lock_timeout for
it, and the project's deployments can run the migration while other connections use the table):

| Repository | Commit | Matches | Real | Note |
|---|---|--:|--:|---|
| fastapi/full-stack-fastapi-template | 77e550a | 4 | 4 | `prestart` runs `alembic upgrade head` while the previous backend still serves |
| jupyterhub/jupyterhub | 3ca3744 | 16 | 0 | its upgrade guide shuts the Hub down before `upgrade-db`; the Hub is the database's only client |
| mealie-recipes/mealie | 8dbb928 | 48 | 0 | one self-hosted process runs `command.upgrade` in `init_db` before it serves |
| open-webui/open-webui | 0666e59 | 49 | 49 | migrations run at startup; multi-replica PostgreSQL deployments roll over live replicas |
| Netflix/dispatch | dd2837e | 128 | 32 of 32 read | both trees (`core`, `tenant`); neither env.py sets it |
| polarsource/polar | 872272f | 3 | 2 | 182 of its 205 revision files mention lock_timeout; the not-real match drops `pledge_transactions`, which no code reads any more |
| langgenius/dify | 724ac15 | 178 | 36 of 36 read | PostgreSQL branches only (`_is_pg`, `_is_mysql` guards honoured) |
| apache/superset | fdf6172 | 153 | 32 of 32 read | `superset db upgrade` against a live metadata database; SQLite-only branches left alone |
| Total | | 579 | 155 of 220 read | |

Read: every match in fastapi-template, jupyterhub, mealie, polar and open-webui; spread samples of 32 in dispatch and
superset (every 4th and 5th match), and 36 in dify (every 5th match). Every match read is a correct reading of the
code: the operation, the table, and no lock_timeout anywhere the rule looks; there were no matcher false positives to
fix. The not-real matches are the two repositories whose deployment model takes the hazard away, and polar's drop of
a table nothing queries. Precision over all read matches: 155 of 220 (70%); without jupyterhub and mealie, 155 of 156.

Real matches, for example: polar
`server/migrations/versions/2026-11-03-1201_make_subscription_started_at_non_nullable.py` (`alter_column(...,
nullable=False)` on `subscriptions`, a full-table scan under ACCESS EXCLUSIVE, in a repository that sets lock_timeout
everywhere else) and `2026-10-23-1000_add_agents_claim_polled_at.py` (`add_column` on `agents`); dispatch
`src/dispatch/database/revisions/tenant/versions/2023-11-29b_580a18ec4c39.py` (`drop_column` on `signal_instance`) and
`.../tenant/versions/2022-08-29_479024506e05.py` (`alter_column ... nullable=False` on `case`); open-webui
`backend/open_webui/migrations/versions/9a1b2c3d4e5f_add_current_message_id_to_chat.py` (`add_column` on `chat`) and
`374d2f66af06_add_prompt_history_table.py` (`drop_table("prompt")` before renaming the new table over it); superset
`superset/migrations/versions/2020-12-03_16-21_811494c0cc23_remove_path_from_logs.py` (`drop_column` on `logs`);
dify `api/migrations/versions/2024_11_28_0553-cf8f4fc45278_add_exceptions_count_field_to_.py` (`add_column` on
`workflow_runs`) and `2024_08_20_0455-2dbe42621d96_rename_workflow__conversation_variables_.py` (`rename_table`).

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
`down_revision` names: one string, or each string of a merge revision's tuple or list. An id resolves first among the
revisions of the same directory and only then elsewhere, so two version directories that reuse ids (`"0001"`,
`"0002"`) stay apart while a tree spread over several `version_locations` still joins up. A head is a revision no
other revision names in its `down_revision`. Two heads that share an ancestor make one tree with two heads, and
`alembic upgrade head` refuses to run: "Multiple head revisions are present for given argument 'head'". The rule
reports at each head's `down_revision = ...`, once per other head of its tree:

- two revisions written on the same parent, with no merge revision joining them: the usual result of two branches
  each adding a migration on the head they saw (both heads are reported, also when one line has grown further);
- a merge revision that is one head while a revision written on a pre-merge revision is the other;
- two branches that each ran `alembic merge heads` on the same heads (two merge heads).

The message names the other head and, for each head, the one-parent revision that starts its line: if that line is
the newer one and the revision has not run on any shared database, point its `down_revision` at the other head;
otherwise add a merge revision with `alembic merge heads`. For two merge heads it says to merge again or delete the
merge that has not run. It says when to leave it: an intentional parallel branch, which should carry `branch_labels`.

It leaves alone:

- forks joined by a merge revision (`down_revision = ("b2", "b3")`, also as a list, also with no `from alembic import
  op` and an empty `upgrade()`);
- heads of trees that share no ancestor: independent migration trees in separate directories (Netflix/dispatch's
  `core` and `tenant` trees), or with the same sequential ids;
- a head whose own line (the revisions between the fork and the head) sets `branch_labels` or `depends_on`: an
  intentional branch, migrated with `alembic upgrade heads` or `branch@head`. Labels on the shared base or trunk do
  not silence a fork, since `alembic upgrade head` still fails there;
- revisions in test trees.

**Gaps:** a second base without `branch_labels` in the same version directory (two heads with no common ancestor) is
left alone; a `revision` or `down_revision` that is not a string literal is unresolved, so its line looks like a
separate tree; revisions outside the repository (a library's `version_locations`) are unseen. `depends_on` alone
counts as an intentional branch, although Alembic still counts both heads for `upgrade head`.

**Sources:** Alembic, "Working with Branches" (https://alembic.sqlalchemy.org/en/latest/branches.html): the multiple
heads error, `alembic merge heads`, `branch_labels`; Alembic's `RevisionMap` (`alembic/script/revision.py`), whose
`heads` drop only revisions named in a `down_revision` (not in `depends_on`); pytest-alembic's
`test_single_head_revision`, for merge conflicts that "lazily break during deployment"
(https://pytest-alembic.readthedocs.io/en/latest/quickstart.html).

**Overlap:** pytest-alembic (an opt-in test) and Alembic's own error at deploy; no linter (Ruff, Bandit, Squawk) reads
the revision graph. This rule's value is catching it while the revision is written, before the deploy fails.

**Corpus** (8 Alembic repositories at the `MANIFEST.tsv` commits). "Engine" is the rule's own count; "heads" and
"merges" come from an independent Python `ast` survey of every revision file (Python 3.15: an older parser skipped
files using `match` or PEP 758 `except A, B:`, which made one phantom head each in mealie and dify):

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
Counts: the five smaller repositories were also run whole with `corpus-run` (0 findings each); all eight were run with
the final rule directly on their migration directories (the rule reads only revision files).

### `alembic-practice-rename-as-drop-add`

Rerail's own rule, not a port. Mode `log`. In an Alembic revision's `upgrade()` (or a module-level function of the
same file that `upgrade()` calls), it reports `op.drop_column(t, "a")` and `batch_op.drop_column("a")` inside
`with op.batch_alter_table(t) as batch_op:` when the same `upgrade()` adds a column `b` to the same existing table and
the pair reads as a rename that loses the data:

- `upgrade()` neither creates nor drops `t`, and adds as many columns to `t` as it drops;
- the dropped column's type, read from `downgrade()`'s `add_column(t, Column("a", T))`, is the added column's type,
  or `downgrade()` does not show it. Types compare by family, from the last name of the callee: `VARCHAR`, `String`
  and `Unicode` are one family, `TIMESTAMP` and `DateTime` another, `INTEGER` and `Integer` another; two literal
  lengths that differ (`VARCHAR(32)` and `String(64)`) are different types;
- the pair is unique: no other add or drop on `t` in that `upgrade()` matches either side;
- nothing in `upgrade()` (or the helpers it calls) moves data: no `.execute(...)` or `.exec_driver_sql(...)` on `op`,
  a connection, a bind or a session, no `op.bulk_insert`, no `update(...)`/`insert(...)` call (`sa.update`,
  `table.update()`), no string holding `UPDATE ` or `INSERT `, no `Session`/`sessionmaker`/`scoped_session`.

Autogenerate cannot detect a rename: "Changes of column name ... are detected as a column add/drop pair, which is not
at all the same as a name change" (Alembic, What does Autogenerate Detect). Run as generated, the migration deletes
the column's data on the first deploy. The message names `op.alter_column(t, "a", new_column_name="b")` for a
rename, with the expand/contract caveat (while running code still reads `a`: add `b`, copy, switch the code, drop `a`
in a later release), or an UPDATE that copies the data before the drop; it says to keep the migration only when the
old column's data is meant to be discarded.

It leaves alone the copy pattern (add the new column, `UPDATE t SET b = a`, drop the old one) and any other data
statement in the same `upgrade()`; types that differ (`JSON` dropped, `LargeBinary` added); a drop and an add of the
same name (a column recreated); tables with more drops than adds or the reverse (Netflix/dispatch's
`2022-08-01_391f7c3e6992.py` replaces `stable_at` with `triage_at` and `escalated_at`; its `2022-10-27_01aa49ca0470.py`
adds `window` and drops five columns, one of them the INTEGER `creator_id`); two adds and two drops of one type,
where no pairing can be named; tables the same `upgrade()` creates or drops; the branch of a dialect test that runs
only on another database; `downgrade()`; test files; an `op` that is not `alembic.op`.

**Gaps:** a replacement whose new column means something else but has the old one's type (langgenius/dify's
`2e9819ca5b28_add_tenant_id_in_api_token.py`: `dataset_id` dropped, `tenant_id` added, both `StringUUID`) is
reported, which is why the rule starts in `log`; the old type is read only from `downgrade()`'s `add_column`; a data
copy in a function of another module is not seen; two renames of one type on one table are not reported; a table
named by a variable is not seen.

**Sources:** Alembic, [What does Autogenerate Detect (and what does it not
detect?)](https://alembic.sqlalchemy.org/en/latest/autogenerate.html#what-does-autogenerate-detect-and-what-does-it-not-detect);
strong_migrations, "Renaming a column" (rename through a new column, a backfill and a code switch);
Squawk, [renaming-column](https://squawkhq.com/docs/renaming-column).

**Overlap:** none. Squawk sees `ADD COLUMN` and `DROP COLUMN` as two unrelated statements and its renaming-column rule
sees only `RENAME COLUMN`; nothing in Ruff or Bandit.

**Corpus:** the five smaller repositories ran with `corpus-run` over the repository root; dify, polar and superset
ran with `rerail-engine logic` over their migration directory (the rule reads one migration file at a time).

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

Precision 1 of 2 (50%), hence `log`. The real match: langgenius/dify
`api/migrations/versions/00bacef91f18_rename_api_provider_description.py`, titled "rename api provider description":
the autogenerated batch adds `description` (`LongText`, `nullable=False`) and drops `description_str` (`LongText`),
so every provider's description is deleted. The other: `2e9819ca5b28_add_tenant_id_in_api_token.py` swaps
`dataset_id` for `tenant_id`, a different reference whose old values are meant to go. Of the 20 (table, `upgrade()`)
pairs in these repositories that drop and add columns on the same existing table, 13 move data (`op.execute`,
`UPDATE`, `sa.update`, an ORM `Session`), 5 add and drop different numbers of columns, and the 2 above are reported.

### `alembic-practice-model-import-in-migration`

Rerail's own rule, not a port. It reads a migration against the application's models. In an Alembic revision outside
test code (module level, `upgrade()`, `downgrade()` or a helper), it reports

- each name of `from app.models.group import Group` (or `from app.models import Group`, a re-export followed to the
  class) that is a mapped ORM class of the application, one finding per imported name;
- a model read through its module, `from jupyterhub import orm` then `db.query(orm.APIToken)`: the first use of each
  model in the file, outside type annotations.

A mapped class is a class of the repository that declares `__tablename__`, is a sqlmodel `table=True` class,
subclasses either (`al_mapped_class`), or subclasses a declarative base (`class Base(DeclarativeBase)`, a module-level
`Base = declarative_base()`) without `__abstract__ = True`: Netflix/dispatch names every table with a `declared_attr` on
its base, so its models declare no `__tablename__`. An import whose module sits in a namespace package (a directory
without `__init__.py`, as dispatch's `dispatch/feedback/`) is matched to the class by file path. The message says why
(the model follows today's schema, so an old revision's queries name columns that do not exist yet at that revision,
and fresh installs and CI from an empty database fail, or write the wrong ones) and names the replacement: declare the
table in the migration with `sa.table("name", sa.column("col"), ...)` and run it through `op.execute()` or the
connection, or define a model class inside the migration. It says that column types, enums and constants from the same
module may stay.

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
`declarative_base()` in the same module as its models (the base reference does not resolve there); Flask-SQLAlchemy's
`db.Model` subclasses without `__tablename__` (the name is generated); models from an installed package; a class whose
base chain crosses a namespace package (the path fallback covers only the imported name itself).

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

**Corpus** (8 Alembic repositories at the `MANIFEST.tsv` commits; every match read). Six were run whole through
`corpus-run`; polar and dify, while the shared queue was full, were run with the engine directly on the files the rule
needs (every non-test `__init__.py`, and every non-test file that mentions `sqlalchemy`, `sqlmodel`, `__tablename__` or
a revision id: 828 and 1,262 files). superset gave the same single match both ways.

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

Precision on the corpus: 13 of 13 matches real (100%). The 13 sit in 6 revisions, because projects that learned this
moved to local models (mealie's revisions from 2024 on declare their own `SqlAlchemyBase` and model classes, which stay
silent). Edit-window check: a new revision added to a copy of polar that imports `Organization` (through the
`polar.models` re-export) and `polar.models.user.User` is reported on both lines and `polar.config.settings` is not; in
a copy of dify, `from models.model import App` and `models.Account` (through `import models`) are reported, and
`StringUUID` and `AppTriggerStatus` are not.

### `alembic-practice-empty-downgrade`

Rerail's own rule, not a port. It reports, at `def downgrade():`, an Alembic revision whose `upgrade()` changes the
schema while its `downgrade()` does nothing (`pass`, `...`, or both, with at most Alembic's autogenerate markers
`# ### commands auto generated by Alembic - please adjust! ###`), in a migration tree that implements its downgrades.
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

The message says to write the reverse operations of `upgrade()`, or, when the change cannot be reversed, to raise
(`raise NotImplementedError("irreversible: ...")`) so a rollback stops instead of leaving the schema behind; and to leave
it in a project that never runs downgrades. No primary source prefers `raise` over `pass`; the message offers both.

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
  downgrade that is correct as it is;
- revisions in test code.

**Gaps:** schema changes made outside Alembic's `op` (raw DDL in `op.execute`, a project's own `add_columns` wrapper,
sqlalchemy-searchable's `sync_trigger`) are not schema changes here, so such a revision is neither reported nor counted.
A `downgrade()` made of other no-op statements (`return`, `print(...)`) counts as implemented. Any explaining comment
silences the rule, whatever it says. The tree is a directory, not the revision graph, so one tree spread over several
`version_locations` is judged per directory.

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

Counts: the five smaller repositories were run whole with `corpus-run`; polar, dify and superset were run with the
engine directly on their migration directories (`server/migrations`, `api/migrations`, `superset/migrations`), as the
rule reads only revision files. The per-tree figures come from the engine (a copy of the rule that also reports the
tree counts and the excused revisions), run the same way on all eight.

Mode: `log`. The read matches are all real, but there are only 3, and the exemptions that remove the 13 deliberate
no-ops are heuristics (a reflection call, a comment).
