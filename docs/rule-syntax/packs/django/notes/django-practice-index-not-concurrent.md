# django-practice-index-not-concurrent

Invariant: on PostgreSQL, a migration builds an index on an existing table with `AddIndexConcurrently` (in a
migration with `atomic = False`), not with a plain CREATE INDEX.

## Sources

- Django docs, `docs/ref/contrib/postgres/operations.txt`, "Concurrent index operations"
  (https://github.com/django/django/blob/main/docs/ref/contrib/postgres/operations.txt,
  https://docs.djangoproject.com/en/dev/ref/contrib/postgres/operations/#concurrent-index-operations):
  > PostgreSQL supports the ``CONCURRENTLY`` option to ``CREATE INDEX`` and ``DROP INDEX`` statements to add and
  > remove indexes without locking out writes. This option is useful for adding or removing an index in a live
  > production database.

  > The ``CONCURRENTLY`` option is not supported inside a transaction (see :ref:`non-atomic migration
  > <non-atomic-migrations>`).
- PostgreSQL docs, CREATE INDEX, CONCURRENTLY (https://www.postgresql.org/docs/current/sql-createindex.html):
  > When this option is used, PostgreSQL will build the index without taking any locks that prevent concurrent
  > inserts, updates, or deletes on the table; whereas a standard index build locks out writes (but not reads) on the
  > table until it's done.
- Squawk, require-concurrent-index-creation (prior art; it lints SQL files, not Django migrations)
  (https://squawkhq.com/docs/require-concurrent-index-creation,
  https://github.com/sbdchd/squawk/blob/master/docs/docs/require-concurrent-index-creation.md):
  > During a normal index creation, table updates are blocked, but reads are still allowed. `CONCURRENTLY` avoids
  > locking the table against writes during index creation.

  > This rule ignores indexes added to tables created in the same transaction.

## Linter coverage

**django-migration-linter has this check**: code `CREATE_INDEX`, "(Postgresql specific) Creating an index without
the concurrent keyword will lock the table and may generate downtime", a Warning
(https://github.com/3YOURMIND/django-migration-linter/blob/main/docs/incompatibilities.md). Its implementation
(`src/django_migration_linter/sql_analyser/postgresql.py`, `has_create_index`) runs `sqlmigrate` against a configured
database, searches the SQL for `CREATE (UNIQUE )?INDEX` without `INDEX CONCURRENTLY`, and skips a table created
earlier in the same migration: the same check as this rule, on the generated SQL. So criterion 2 of the bar fails.
Differences: it needs Django set up and a database connection; this rule reads the migration files and the settings
statically. Ruff, flake8-django, pylint-django, Bandit, django-upgrade and Django's system checks have nothing on
migrations' index operations. Squawk lints raw SQL, not Django migration files.

## What it matches

In a migration (dj_migration_doc, not test code) of a project on PostgreSQL, an operation that builds an index on a
model whose CreateModel is in an earlier-numbered migration of the same directory (and not in this one):
`AddIndex`; `AddField` with db_index=True, unique=True, OneToOneField, or a ForeignKey without db_index=False;
`AlterField` with such a field when no earlier migration gave that field the same kind of index (unique or plain).
Skipped: operations under `SeparateDatabaseAndState(state_operations=…)`, migrations whose class overrides
`apply`, unmanaged models (`options={"managed": False}`), tables created in the same migration.

PostgreSQL is detected across the run: a non-test string naming a PostgreSQL backend (`*backends.postgresql*`,
`*backends.postgis`), a `postgres://`/`postgresql://`/`postgis://` URL, the `"django.contrib.postgres"` app, or an
import of `django.contrib.postgres.*` (fields, operations, search). The corpus uses all of these: ENGINE strings in
settings (most repos), `dj_database_url.config(default="postgres://…")`, a project backend
`"posthog.db_backends.postgresql"`, and pontoon (whose settings default is a mysql:// URL) only through its
migrations' `django.contrib.postgres.fields` imports. addons-server (MySQL) has none and gets no match.

Cross-file facts: the settings module (or any file) for PostgreSQL; the earlier migration's CreateModel for "the
table exists"; earlier migrations' field definitions for AlterField's previous state.

Misses: AddConstraint(UniqueConstraint), AlterUniqueTogether, AlterIndexTogether (each builds an index); models
renamed by RenameModel (the CreateModel has the old name); a project that is PostgreSQL only through an environment
variable. One run is one project: the cases cannot hold a non-PostgreSQL negative beside the PostgreSQL project.

## Corpus

Final revision, full run (31 repos; PostHog is in the corpus SKIP list): 1519 matches in 27 repos
(`/tmp/djxf-inc-full.log`; results `~/.cache/rerail-django-corpus/results/django-practice-index-not-concurrent.jsonl`).
The independent re-derivation agrees with all 1519 (AddField 849, AddIndex 381, AlterField 289, every AlterField
going from no index, or the other kind, to an index); the previous revision's 30 disagreements are gone. netbox timed
out at 3600 s (machine at load 50+); its result is missing.
Every match was re-derived by an independent AST script (`/tmp/djxf/inc_verify.py`: earlier CreateModel, index-bearing
field, latest prior definition of an altered field), and a round-robin sample of 90 across all repos was read.
- Read 90 (spread over 22 repos), plus all 30 the script disagreed with. Real (pattern present: an index built on a
  table an earlier migration created): 90/90 of the sample after the fixes below.
- Examples: `WeblateOrg__weblate:weblate/trans/migrations/0068_unit_source_tm_index.py:16` (GIN index on `unit`),
  `DefectDojo__django-DefectDojo:dojo/db_migrations/0203_alter_finding_options_finding_epss_percentile_and_more.py:28`
  (AddIndex on `finding`), `healthchecks__healthchecks:hc/api/migrations/0103_check_badge_key.py:12` (unique UUID on `check`).
- Zero-match repos: addons-server (MySQL), Kiwi (MySQL default; PostgreSQL only in test settings), mathesar.

False-positive classes found and fixed (each has a negative case):
1. `Migration.apply` overridden to skip the operations on PostgreSQL (label-studio io_storages 0018): `inc_own_apply`.
2. `initial = True` split initial migrations (nautobot `0002_initial_part_2` … `0004_initial_part_4`, 113 matches):
   their tables are new and empty: `inc_initial`.
3. AlterField of a renamed field (no prior definition under the new name; nautobot `*_rename_foreignkey_fields`,
   arches `11869_rename_resxres_fields`, 28 matches): AlterField now needs a prior definition (`inc_prior`).
4. AlterField after an AddField earlier in the same squashed migration: same-file prior definitions count (`earlier`).
Recall fixes: a migration named `0301_…_test.py` was dropped by dj_test_doc's `*_test.py` file-name test
(DefectDojo 0301/0302); migrations now skip only tests/ and test/ directories. Projects whose PostgreSQL backend is
a subclass (awx `profiled_pg`, InvenTree) are detected through `from django.db.backends.postgresql… import`.
Also skipped up front: unmanaged models, state_operations.

## Mode

`log`, and **DROP**: criterion 2 fails, django-migration-linter's `CREATE_INDEX` makes the same check (on sqlmigrate
output). Precision alone would allow steer, but table size is unknown: most matches are years-old migrations on tables
that were small when they ran, so the finding is a review prompt, not a defect, in history. Kept for the integrator to
decide; a "new migrations only" variant (diff-scoped) would be the useful form.

## Helpers worth a library

- `inc_any_op(c, d, op)`, `inc_model(c, m)`, `inc_mig(d, dir, n)`, `inc_fielddef(d, m, x, fc)`, `inc_state_only(c)`:
  a migration-operations vocabulary (python/django-migrations.rr): every migration-safety rule (NOT NULL without
  default, RemoveField without SeparateDatabaseAndState, RunPython without reverse) needs the operation, its model,
  the migration order and the field's previous definition.
- `inc_pg(v)`: the project's database is PostgreSQL; `dj_postgres()` in python/django.rr.
