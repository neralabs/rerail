# Alembic pack: mining notes

What a team running Alembic migrations against a live PostgreSQL database flags in review, which of it a linter
already checks, and what this pack builds. Squawk checks the same hazards on the SQL `alembic upgrade head --sql`
renders (`alembic-squawk` and PandaDoc's `migration-lint` package that pipeline), and django-migration-linter on Django
migrations. This pack adds the edit window (no offline render, no database), revisions that cannot render offline
(inspector calls, data migrations), and the cross-file checks no SQL linter can make: the models a migration drops a
column from, the models it imports, the revision graph, and the `env.py` that configures the run (corrected after the
independent review, REVIEW-alembic.md 3.7). Ruff and Bandit (the `python` pack's ports) check none of these shapes;
S608 is the one partial overlap (below).

## 1. Sources read

| # | Source | URL | What it contributed |
|---|---|---|---|
| S1 | PostgreSQL: CREATE INDEX | https://www.postgresql.org/docs/current/sql-createindex.html | "a standard index build locks out writes (but not reads) on the table until it's done"; CONCURRENTLY "cannot" run in a transaction block; a failed concurrent build leaves an INVALID index. |
| S2 | PostgreSQL: ALTER TABLE (Notes) | https://www.postgresql.org/docs/current/sql-altertable.html | Type changes rewrite the table and its indexes unless binary coercible; SET NOT NULL scans the table unless a valid CHECK proves it; NOT VALID for FK and CHECK; ADD FOREIGN KEY takes SHARE ROW EXCLUSIVE on both tables; VALIDATE CONSTRAINT takes SHARE UPDATE EXCLUSIVE; ADD CONSTRAINT ... USING INDEX; volatile defaults rewrite. |
| S3 | PostgreSQL: ALTER TYPE (Notes) | https://www.postgresql.org/docs/current/sql-altertype.html | A value added by ADD VALUE in a transaction cannot be used until it commits. |
| S4 | PostgreSQL: client connection defaults (lock_timeout) | https://www.postgresql.org/docs/current/runtime-config-client.html | lock_timeout aborts a statement waiting too long for a lock; default 0 (off). |
| S5 | Alembic: Operation Reference | https://alembic.sqlalchemy.org/en/latest/ops.html | `add_column`: server defaults use `server_default`, "not `default` which only specifies Python-side defaults"; UNIQUE and `index=` on an added Column are ignored (1.18.2+); `execute` coerces strings to `text()`. |
| S6 | Alembic: runtime, `MigrationContext.autocommit_block` | https://alembic.sqlalchemy.org/en/latest/api/runtime.html | For DDL that "must be run outside of a transaction block"; the transaction before the block is committed unconditionally; `transaction_per_migration` recommended with it. |
| S7 | Alembic: Auto Generating Migrations, "What does Autogenerate Detect (and what does it not detect?)" | https://alembic.sqlalchemy.org/en/latest/autogenerate.html | Table and column renames "come out as an add/drop", "not at all the same as a name change"; enum caveats. |
| S8 | Alembic: Working with Branches | https://alembic.sqlalchemy.org/en/latest/branches.html | `upgrade head` fails with "Multiple head revisions are present"; fix with `alembic merge`. |
| S9 | Alembic: Cookbook | https://alembic.sqlalchemy.org/en/latest/cookbook.html | Data migrations with `sa.table()`/`column()` and `op.bulk_insert`; migrations should go to the finished schema "and back again". |
| S10 | Alembic issue #679 (zzzeek) | https://github.com/sqlalchemy/alembic/issues/679 | ORM models in a migration "would no longer work in this migration once it's older"; use a model local to the migration. |
| S11 | Alembic issue #278 | https://github.com/sqlalchemy/alembic/issues/278 | PG enum types must be created and dropped explicitly; enum operations run outside a transaction block. |
| S12 | SQLAlchemy: PostgreSQL dialect (Indexes with CONCURRENTLY; Constraint options, NOT VALID) | https://docs.sqlalchemy.org/en/21/dialects/postgresql.html | `postgresql_concurrently` needs autocommit; `postgresql_not_valid=True` on CheckConstraint and ForeignKeyConstraint, with an Alembic example (sqlalchemy/sqlalchemy#7600, 1.4 series). |
| S13 | SQLAlchemy: Column INSERT/UPDATE defaults | https://docs.sqlalchemy.org/en/21/core/defaults.html | `Column.default` is applied by SQLAlchemy at INSERT; only `server_default` reaches the DDL. |
| S14 | Squawk rules index and rule pages | https://squawkhq.com/docs/rules | require-concurrent-index-creation, ban-concurrent-index-creation-in-transaction, adding-required-field, adding-not-nullable-field, adding-foreign-key-constraint, constraint-missing-not-valid, disallowed-unique-constraint, changing-column-type, ban-drop-column, renaming-column/table, require-lock-timeout. SQL only; each page shows the Alembic fix. |
| S15 | Squawk: Applying migrations safely | https://squawkhq.com/docs/safe_migrations | A DDL waiting for a lock makes "all other reads and writes sent after" wait; short lock_timeout plus retries. |
| S16 | strong_migrations README | https://github.com/ankane/strong_migrations | The canonical checklist: non-concurrent index, NOT NULL, column type changes (safe-change table), removing and renaming columns, FK and check without validation, unique constraints, backfilling. |
| S17 | django-migration-linter: incompatibilities | https://github.com/3YOURMIND/django-migration-linter/blob/main/docs/incompatibilities.md | Django equivalents: NOT_NULL, DROP_COLUMN, RENAME_COLUMN, ADD_UNIQUE, CREATE_INDEX, RUNPYTHON_MODEL_IMPORT, RUNPYTHON_REVERSIBLE; "default is not a database default" is a "common misconception". |
| S18 | GoCardless: Zero-downtime Postgres migrations, the hard parts | https://gocardless.com/blog/zero-downtime-postgres-migrations-the-hard-parts | 15 s outage from the lock queue; "set lock_timeout in your migration scripts"; don't rename columns in use. |
| S19 | PostgresAI: lock_timeout and retries | https://postgres.ai/blog/20210923-zero-downtime-postgres-schema-migrations-lock-timeout-and-retries | A blocked `SET NOT NULL` makes plain SELECTs hang behind it; low lock_timeout plus retries. |
| S20 | Xata: Schema changes and the Postgres lock queue | https://xata.io/blog/migrations-and-exclusive-locks | "DDL statements in migration sessions should always set lock_timeout". |
| S21 | pytest-alembic quickstart | https://pytest-alembic.readthedocs.io/en/latest/quickstart.html | test_single_head_revision (merge conflicts that "lazily break during deployment"), test_up_down_consistency. |
| S22 | alembic-postgresql-enum README | https://github.com/Pogchamp-company/alembic-postgresql-enum | Autogenerate misses enum value changes; `drop_table` does not drop the enum type. |
| S23 | Ruff S608 / Bandit B608 | https://docs.astral.sh/ruff/rules/hardcoded-sql-expression/ , https://bandit.readthedocs.io/en/latest/plugins/b608_hardcoded_sql_expressions.html | String-built SQL anywhere, `op.execute` included. |
| S24 | The corpus's own practice: polar `server/migrations` | `polarsource__polar` at 872272f | 179 of 205 migrations set `lock_timeout`; concurrent indexes always inside `autocommit_block()`, with a `drop_index(..., if_exists=True)` first; `env.py` keeps `UNMAPPED_COLUMNS` for columns removed from models before they are dropped (expand/contract). |

## 2. Candidates

Corpus: the 8 repositories with Alembic migrations (dispatch 158 files, superset 325, full-stack-fastapi-template 5,
jupyterhub 20, dify 246, mealie 66, open-webui 72, polar 203: 1,095 migration files). Counts below come from a Python
`ast` survey of each `upgrade()` (an "existing" table is one the same `upgrade()` does not create with `op.create_table`),
before any rule was written; the writers' engine runs give the final numbers.

| id | Catches | Why it is a bug | Sources | Files | Linter overlap | Detection | Corpus probe | Verdict |
|---|---|---|---|---|---|---|---|---|
| index-not-concurrent | `op.create_index` / `batch_op.create_index` on an existing table without `postgresql_concurrently=True`; `create_unique_constraint` on an existing table | CREATE INDEX (and the index behind ADD UNIQUE) locks out writes for the whole build | S1, S14, S16, S17 | single (plus the batch table) | Squawk require-concurrent-index-creation and disallowed-unique-constraint, on SQL only | op call resolved through `from alembic import op`; table from the literal or `batch_alter_table(...)`; skip tables created in the same `upgrade()` | 328 non-concurrent on existing tables (dify 133, mealie 136, open-webui 24, superset 21, polar 11, dispatch 3); 89 create_unique_constraint | build |
| concurrent-index-outside-autocommit | `postgresql_concurrently=True` on create/drop_index outside `with op.get_context().autocommit_block():` | PostgreSQL refuses CONCURRENTLY in a transaction block: the deploy's migration fails | S1, S6, S12, S14 | cross-file (env.py `transactional_ddl=False` runs without a transaction) | Squawk ban-concurrent-index-creation-in-transaction (SQL) | with-statement ancestry; env.py configure kwargs | 0 of 268 outside (polar, dify always wrap): every team that uses it got it right after the first failure; the guard for the steer of index-not-concurrent | build |
| add-column-required-no-server-default | `add_column` of a NOT NULL (or primary-key) column with no `server_default` on an existing table; `default=` alone named as the misconception | `ALTER TABLE ADD COLUMN ... NOT NULL` fails on any table with rows; `default=` is Python-side only | S5, S13, S14, S16, S17 | single | Squawk adding-required-field (SQL); none for Alembic | Column kwargs in add_column; non-PostgreSQL branches (`if _is_pg(conn): ... else:`) left alone | 19 (dify 15, superset 3, open-webui 1), 9 of them with `default=` | build |
| drop-mapped-column | `drop_column`, `drop_table`, `rename_table`, `alter_column(new_column_name=)` of a column or table a SQLAlchemy model in the repository still maps | The app's queries name the column: they fail as soon as the migration runs (expand/contract) | S14, S16, S17, S18, S24 | cross-file (migration ↔ models) | Squawk ban-drop-column / renaming-* flag every drop, without knowing whether code uses it | model classes by `__tablename__` and Column/mapped_column across files | 0 at HEAD (models and migrations agree at a release); the shape an edit creates when the migration is written before the model change | build |
| rename-as-drop-add | One `upgrade()` drops column a and adds column b of the same type on the same table, with no data statement between | Autogenerate renders a rename as drop + add: the column's data is lost | S7, S16 | single | none | per-function grouping of drop/add by table | 20 drop+add pairs on one table (dispatch 7, dify 4, superset 3, open-webui 3, template 2, jupyterhub 1); most copy data first (to be excluded) | build (log first) |
| alter-column-type-rewrite | `alter_column(type_=...)` on an existing table other than a safe widening (VARCHAR length increase, VARCHAR to TEXT, MySQL-only types) | ACCESS EXCLUSIVE and a full rewrite of the table and its indexes | S2, S14, S16 | single | Squawk changing-column-type (SQL; flags every change) | type_ and existing_type expressions compared | 80 type changes; about 40 not a widening (INTEGER→BigInteger on dify's workflow_runs, JSON→JSONB, Integer→Float, DateTime→BigInteger) | build |
| set-not-null-scan | `alter_column(nullable=False)` on an existing table | SET NOT NULL scans the whole table under ACCESS EXCLUSIVE; the safe path is CHECK ... NOT VALID, VALIDATE, then SET NOT NULL | S2, S14, S16, S19 | single | Squawk adding-not-nullable-field (SQL) | kwarg on alter_column; table not created here | 89 (polar 26, dify 22, superset 20, dispatch 15, mealie 3, template 3) | build (log first) |
| constraint-without-not-valid | `create_foreign_key` / `create_check_constraint` on an existing table without `postgresql_not_valid=True` | Validates every row while holding SHARE ROW EXCLUSIVE (FK: on both tables), blocking writes | S2, S12, S14, S16 | single | Squawk adding-foreign-key-constraint, constraint-missing-not-valid (SQL) | source table not created in the same `upgrade()` | 361 FK (dispatch 285), 31 check; polar uses NOT VALID 21 times | build |
| model-import-in-migration | A migration imports a mapped ORM model class of the application | The model follows today's schema, not the schema at that revision: old migrations break or write wrong columns | S9, S10, S17 | cross-file (migration ↔ model module) | django-migration-linter RUNPYTHON_MODEL_IMPORT (Django only) | import resolves to a class with `__tablename__` or a declarative base; column types and enums left alone | mealie 3 (Group, IngredientFoodModel, MultiPurposeLabel), dispatch 1 (Feedback), superset 1 (SqlaTable); GUID/StringUUID/JSONDict type imports must stay silent | build |
| multiple-heads | Two revision files with the same `down_revision` and no merge revision joining them, without branch labels | `alembic upgrade head` refuses to run: the deploy fails | S8, S21 | cross-file (every revision file) | pytest-alembic test_single_head_revision (a test, not a linter) | `revision`/`down_revision` literals over all files of the migration directory | 0 at HEAD (to be confirmed by the engine run); the shape an agent creates with a stale `down_revision` | build |
| missing-lock-timeout | A locking DDL op in a migration when neither the migration nor `env.py` sets `lock_timeout` | A DDL waiting for a lock queues every later read and write on the table: an outage | S4, S14, S15, S18, S19, S20, S24 | cross-file (migration ↔ env.py) | Squawk require-lock-timeout (per SQL file; cannot see env.py) | env.py `SET lock_timeout` / connect args, migration `op.execute("SET ... lock_timeout")` | polar sets it in 179/205; the other 7 repositories nowhere | build (log: repositories that never set it would match every migration) |
| empty-downgrade | `downgrade()` that is only `pass`/docstring while `upgrade()` changes the schema, in a repository whose other migrations implement downgrade | `alembic downgrade` reports success and moves the version back while the schema stays: the next upgrade fails | S9, S17, S21 | cross-file (repository convention) | django-migration-linter RUNPYTHON_REVERSIBLE (Django only) | function body; ratio of the repository's downgrades | 82 (superset 52, dispatch 13, mealie 6, open-webui 4, polar 4, dify 3) | build (log first) |
| execute-interpolated-sql | `op.execute(f"...")` / `.format()` | Injection or quoting breakage | S5, S23 | single | Ruff S608 / Bandit B608 already flag string-built SQL, `op.execute` included; migration values are constants in the corpus | — | — | drop: S608 covers it; no context-aware value |
| add-column-unique-ignored | `add_column(sa.Column(..., unique=True / index=True))` | Assumed from Alembic's doc note ("ignored: unique ... index"); the review found Alembic 1.20.1 renders `ADD UNIQUE` and `CREATE INDEX` after the ADD COLUMN, so the real hazard is a non-concurrent index build | S5 | single | none | Column kwargs | 0 in 1,095 files | dropped as its own rule; the shape is now reported by index-not-concurrent (and an inline `ForeignKey`/`CheckConstraint` by constraint-without-not-valid) |
| volatile-default-rewrite | `add_column(server_default=gen_random_uuid()/clock_timestamp()/random())` on an existing table | Rewrites the table under ACCESS EXCLUSIVE even on PG 11+ | S2, S16 | single | Squawk adding-field-with-default (SQL) | server_default text | 0 on existing tables (all 120 volatile defaults are in `create_table`) | drop: no occurrence |
| enum-add-value-in-transaction | `op.execute("ALTER TYPE ... ADD VALUE ...")` outside `autocommit_block` | Before PG 12 it fails in a transaction; since 12 the value is unusable until commit | S3, S6, S11 | single | Squawk ban-add-enum-value (opt-in, different concern) | string text | 2 (mealie) | drop: PG 12+ runs it; a later use in the same migration fails loudly in CI |
| enum-type-not-created | `add_column` with a native `sa.Enum` whose type no `create()` makes | "type does not exist": the migration fails | S11, S22 | single | none | Column type and `.create(` calls | 4 `sa.Enum` in migrations | drop: fails loudly on the first run in CI, rare |
| unbatched-backfill | `op.execute("UPDATE ...")` over a whole table in the migration's transaction | Long row locks and a long-held lock from earlier DDL | S16, S10 | single | none | SQL text | — | drop: the engine cannot tell a large table from a small one, nor a bounded UPDATE from an unbounded one, from the text |

## 3. Build list, ranked

Outage and failed-deploy first, then data loss, then silent wrong results.

1. `alembic-practice-index-not-concurrent`: write lock for the length of an index build on a live table (the most
   common hazard in the corpus).
2. `alembic-practice-concurrent-index-outside-autocommit`: the migration fails at deploy; guards the steer of 1.
3. `alembic-practice-add-column-required-no-server-default`: the migration fails on any populated table; the
   `default=` misconception.
4. `alembic-practice-drop-mapped-column`: the running app breaks the moment the migration runs (cross-file).
5. `alembic-practice-constraint-without-not-valid`: write lock on both tables for a full validation scan.
6. `alembic-practice-alter-column-type-rewrite`: ACCESS EXCLUSIVE and a full table rewrite.
7. `alembic-practice-set-not-null-scan`: ACCESS EXCLUSIVE for a full scan.
8. `alembic-practice-missing-lock-timeout`: any of the above queues all traffic behind it (cross-file, env.py).
9. `alembic-practice-multiple-heads`: `upgrade head` refuses to run (cross-file).
10. `alembic-practice-rename-as-drop-add`: the column's data is lost.
11. `alembic-practice-model-import-in-migration`: old migrations break or write the wrong columns (cross-file).
12. `alembic-practice-empty-downgrade`: a rollback that reports success and leaves the schema.

`create_unique_constraint` on an existing table is part of rule 1 (same lock, same hazard, its own message), not a
rule of its own.

## 4. The library and its probe

`rules/lib/alembic.rerail` was proven with a throwaway probe rule that reported every interface relation, run three ways:

- On a case repository (env.py, a revision with `op.*`, batch, autocommit, dialect branch, helper function, module
  constants, `op.f`; a merge revision with branch labels; models with a mixin in another module, an explicit column name,
  a relationship, a subclass, and a sqlmodel table class inheriting its fields): every relation gave the expected rows and
  none other (`rerail-engine logic`, 34 rows).
- On `fastapi__full-stack-fastapi-template`: 5 revisions, 2 sqlmodel models (`user`, `item`) with their 12 inherited
  columns, 24 operations on existing tables, both `context.configure` calls.
- On `mealie-recipes__mealie`: 66 revisions, 65 down_revision links, 83 mapped models, 596 model columns, 257 operations
  on existing tables, and the 4 operations of the `else:` branch of `if is_postgres():` in
  `2024-03-18-02.28.15_d7c6efd2de42_...py` (lines 188-191) as non-PostgreSQL.

The probe rule and its cases were deleted after the run.

## 5. Outcome

All 12 candidates on the build list shipped; each writer's corpus run replaced the survey counts of section 2.

| Rule | Mode, on | Corpus matches (repos) | Read precision |
|---|---|---|---|
| index-not-concurrent | steer, on | 296 (6) | 296/296 |
| concurrent-index-outside-autocommit | steer, on | 0 | n/a (224 wrapped concurrent calls silent) |
| not-null-without-server-default | steer, on | 13 (3) | 13/13 |
| drop-mapped-column | steer, on | 0 | n/a (edit-window test on a mealie copy) |
| constraint-without-not-valid | steer, on | 221 (8) | 221/221 |
| column-type-rewrite | steer, on | 34 (6) | 34/34 (46 widenings left alone) |
| set-not-null-scan | log, on | 77 (6) | 74/77 |
| missing-lock-timeout | log, on | 579 (8) | 220/220 hold the pattern; 155/220 where a lock queue can form |
| multiple-heads | steer, on | 0 | n/a (edit-window tests on polar and superset copies) |
| rename-as-drop-add | log, off | 2 (1) | 1/2 |
| model-import-in-migration | steer, on | 13 (4) | 13/13 |
| empty-downgrade | log, on | 3 (2) | 3/3 |

Corrections to section 2's survey: empty-downgrade's 82 candidates were mostly merges, data-only upgrades and idempotent
repairs that read the live schema (16 real candidates; 13 exempted by evidence); rename-as-drop-add's 20 pairs mostly
copy data (13) or replace a column on purpose. Writers' helpers folded into the library because two rules needed them:
the dialect reading of al_non_pg (lowercased tests, negated other-database tests, early-return guards), a project's own
`create_table` wrapper in al_creates_table, the revision graph (al_parent, al_ancestor, al_head) and the per-tree env.py
pairing (al_env_tree, al_in_some_tree). Proposed new hazard ids: `migrations.multiple-heads`, `orm.model-in-migration`.
