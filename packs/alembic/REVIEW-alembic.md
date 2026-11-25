# Alembic pack: independent review

This review audits the 12 `alembic-practice-*` rules, the library `rules/lib/alembic.rerail`, the cases, the README's
rule details and `NOTES-alembic.md`. Nothing in the rules, cases or `pack.json` was changed: the verdicts are the
output. The scratch counterexamples are in `/tmp/review-alembic/` (not committed). Every command is listed in section 4
with its result.

Each rule gets two precision figures, because the pack's README uses a lenient one:

- **Code precision**: the file holds the shape the message describes. This is the README's "real": "a match is real
  when the file holds the pattern, even when the table is small enough that the lock is harmless there".
- **Actionable precision**: a reviewer of that repository would ask for the change. A match is not actionable when the
  project's own deployment removes the hazard, when the table cannot hold rows yet at deploy, or when the shape is a
  no-op. Two deployments remove the lock hazards. jupyterhub's upgrade guide says "Shut down JupyterHub" before
  `upgrade-db`. mealie runs `command.upgrade(alembic_cfg, "head")` in `mealie/db/init_db.py:119` before its one process
  serves. The BRIEF's bar ("a rule whose matches are mostly harmless is cut or starts in log") is about this second
  figure. The README applies it to `missing-lock-timeout` (70%) but not to the other lock rules, whose matches in the
  same two repositories it counts as real.

## 1. Summary

Verdicts: **5 keep, 7 rework, 0 cut.**

| Rule | Verdict | Proposed mode, on | Corpus matches (repos) | Read: code precision | Read: actionable | Reason |
|---|---|---|--:|--:|--:|---|
| `index-not-concurrent` | rework | steer, on | 296 (6) | 78/78 | 62/78 (79%); 62/62 outside mealie | Misses `add_column(unique=True/index=True)` and `create_primary_key` (both build indexes, verified offline) and superset's 20 helper-built indexes; 97 findings for one mealie revision |
| `concurrent-index-outside-autocommit` | keep | steer, on | 0 (0) | n/a | n/a | Precise guard for the index steer; edit-window test passes; minor: an env.py with AUTOCOMMIT isolation is reported, a constant `True` is missed |
| `not-null-without-server-default` | keep | steer, on | 13 (3) | 13/13 | 4/13 tag-to-tag, 13/13 deploying main | Prevents a failed deploy; the fix is always safe |
| `drop-mapped-column` | rework | steer, on | 0 (0) | n/a | n/a | It catches "the models still map the column", not the expand/contract order its text promises; models of another service in a monorepo count |
| `constraint-without-not-valid` | rework | steer, on | 221 (8) | 65/65 | 40/65 (62%); 40/53 outside jupyterhub/mealie | Misses `add_column(Column(..., ForeignKey(...)))`, which Alembic renders as a validated ADD FOREIGN KEY; 13 dify matches are on tables created hours earlier |
| `column-type-rewrite` | rework | steer, on after the fix (log until then) | 34 (6) | 34/34 | 32/34 | Reports same-type changes spelled two ways (TIMESTAMP/DateTime, UUID/Uuid, INTEGER/Integer, BOOLEAN/Boolean, VARCHAR/AutoString) as rewrites |
| `set-not-null-scan` | keep | log, on | 77 (6) | 53/56 | 50/56 (89%) | Correct; the fix spans two deploys and half the tables stay small: log is right |
| `missing-lock-timeout` | rework | log, on | 579 (8) | 121/121 | 98/121 (81%) | One env.py decision is reported once per revision (576 of 579 matches are in trees that never set it); misses `'0s'`; an engine from another module is not seen |
| `multiple-heads` | rework | steer, on | 0 (0) | n/a | n/a | Silent on a second base and on a `depends_on`-only fork; with Alembic 1.20.1 both make `upgrade head` fail |
| `rename-as-drop-add` | rework | log, off | 2 (1) | 2/2 | 1/2 | Any `op.execute`, even `SET LOCAL lock_timeout`, counts as a data copy and silences it; a rename with a new length is missed |
| `model-import-in-migration` | keep | steer, on | 13 (4) | 13/13 | 13/13 | Precise and cross-file; Flask-SQLAlchemy `db.Model` gap |
| `empty-downgrade` | keep | log, on | 3 (2) | 3/3 | 3/3 | Small sample; a downgrade of `return` or a `# TODO` comment passes as implemented |

All 12 rules' own `.test` blocks pass (`node dist/cli/cli.js rules test`). The engine's match counts on the 8
repositories equal the README's for every rule.

## 2. Rules

### 2.1 `alembic-practice-index-not-concurrent`: rework (steer, on)

**Value.** This is the most common PostgreSQL migration outage. A plain `CREATE INDEX` "locks out writes (but not
reads) on the table until it's done" (PostgreSQL, CREATE INDEX). strong_migrations, Squawk
(`require-concurrent-index-creation`, `disallowed-unique-constraint`) and django-migration-linter (CREATE_INDEX,
ADD_UNIQUE) all flag it. The sources hold up.

**Correctness.** The logic is `al_on_existing`, no `postgresql_concurrently` other than a literal False, and not in a
non-PostgreSQL branch. Counterexamples are in `/tmp/review-alembic/index-not-concurrent/`; the run
`review-alembic-index-not-concurrent-cx` gave 2 findings:

| Shape | Expected | Result |
|---|---|---|
| `op.create_index("ix_users_age", "users", ["age"])` | report | reported (x1_shapes.py:24) |
| An index in a project whose `alembic.ini` URL is SQLite and whose env.py uses `render_as_batch=True` (a SQLite-only app) | silent | **reported** (x1:12): nothing reads the project's dialect |
| `op.add_column("users", sa.Column("code", sa.String(8), unique=True))` | report | **missed** (x1:14) |
| `op.add_column("users", sa.Column("ref", sa.String(8), index=True))` | report | **missed** (x1:16) |
| `op.create_primary_key("pk_users", "users", ["id"])` (a unique index built under ACCESS EXCLUSIVE) | report | **missed** (x1:20) |
| `_idx("ix", "users", [...])`, a helper whose table is a parameter | report | missed (x1:30; a documented gap) |
| `op.execute("CREATE INDEX ...")`, `sa.Index(...).create(op.get_bind())` | report | missed (documented for `execute`) |

The `add_column(unique=True / index=True)` misses matter. `NOTES-alembic.md` dropped the candidate
"add-column-unique-ignored" because "Alembic ignores both", and Alembic's own `add_column` docs say "As of 1.18.2, the
following Column parameters are ignored: unique ... index". Alembic 1.20.1 does not ignore them. Rendered offline with
`uv run --with alembic --with sqlalchemy python /tmp/review-alembic/offline/probe.py` (alembic 1.20.1, SQLAlchemy
2.1.9), the ADD COLUMN is followed by `ALTER TABLE orders ADD UNIQUE (code);` and
`CREATE INDEX ix_orders_ref ON orders (ref);`. Both build an index on an existing table without CONCURRENTLY, and
nothing in the pack sees them. `create_primary_key` on an existing table occurs in the corpus:
full-stack-fastapi-template `d98dd8ec85a3_edit_replace_id_integers_in_all_models_.py:54`
(`op.create_primary_key('item_pkey', 'item', ['id'])`) is not reported.

**Corpus.** 296 matches in 6 repositories: dispatch 5, open-webui 29, polar 17, mealie 151, dify 73, superset 21;
jupyterhub and the template have none. 78 were read:

| Repository | Read | Code-real | Actionable |
|---|--:|--:|--:|
| Netflix/dispatch | 5 of 5 | 5 | 5 |
| polarsource/polar | 17 of 17 | 17 | 17 |
| open-webui/open-webui | 10 of 29 (every 3rd) | 10 | 10 |
| mealie-recipes/mealie | 16 of 151 (every 10th) | 16 | 0 (migrates before serving) |
| langgenius/dify | 19 of 73 (every 4th) | 19 | 19 |
| apache/superset | 11 of 21 (every 2nd) | 11 | 11 |

Real, on tables that grow with use:

- dify `2026_01_12_1729-3334862ee907_feat_add_created_at_id_index_to_messages.py:23` (`messages`) and
  `2025_08_29_1534-b95962a3885c_add_workflow_app_log_run_id_index.py:23` (`workflow_app_logs`);
- open-webui `9f6c2a8d1b43_add_channel_lookup_indexes.py:17` (`message`) and
  `d4c1a8e37b62_add_chat_timer_at_and_chat_indexes.py:48` (`chat`);
- dispatch `tenant/versions/2023-10-23_f2605bfc1f59.py:25` (a GIN index on `signal_instance`);
- superset `2017-01-24_12-40_f18570e03440_add_query_result_key_index.py:33` (`query`).

Real but cheap:

- 7 of superset's 11 read matches are on configuration tables that stay small (`dashboards`, `tables`, `clusters`,
  `datasources`, `report_schedule`, `row_level_security_filters`, `task_subscribers`).
- polar's 17 run under `SET LOCAL lock_timeout = '5s'`. Its reviewers accepted them on tables of about 282K rows
  (`2026-07-06-1024_add_subscription_pause_columns.py`), and on the new all-NULL `token_v2` columns of
  `2026-09-17-1658_add_user_sessions_token_v2.py:29`.

The message's "Fine for a table that stays small" covers these.

Recall gap measured on the corpus: superset builds indexes in 20 revisions only through
`superset.migrations.shared.utils.create_index(table, name, columns)`, which calls `op.create_index` without
CONCURRENTLY. An example is `2026-08-24_15-50_a6c21e5b4d93_index_purge_audit_pruning.py` on `purge_audit_log`. None of
the 20 is reported.

Volume: mealie's `2023-02-07-20.57.21_ff5f73b01a7a_add_missing_foreign_key_and_order_.py` alone yields 97 findings. An
agent writing a revision like it gets 97 steers of 385 characters each.

**Overlap.** Squawk's `require-concurrent-index-creation` reads SQL and "ignores indexes added to tables created in the
same transaction" (squawkhq.com). Squawk documents `alembic upgrade head --sql | squawk` as its Alembic integration, and
`alembic-squawk`, `alembic-squawk-action` and PandaDoc's `migration-lint` package that pipeline. So the README's
claimed advantage ("knows which tables the same revision creates, and follows batch_alter_table") is no advantage over
Squawk on offline SQL. Offline SQL renders batch operations as plain `ALTER TABLE t`, and Squawk tracks tables created
in the same transaction.

What this rule does add:

- it runs in the edit window, with no offline render;
- it reads revisions that cannot render offline, those that call `op.get_bind()` or an inspector (common in dify,
  superset and open-webui).

Ruff, mypy, Bandit and pylint have nothing on this shape.

**Text.** The description is fine (158 characters). The messages are 385 and 438 characters against the 200-character
cap. UniqueNotConcurrent packs three alternatives into one sentence.

**Verdict: rework.**

- Report `add_column(sa.Column(..., unique=True))` and `index=True` on an existing table, with a message that names
  the separate concurrent index.
- Report `create_primary_key` on an existing table, with the fix: build the unique index concurrently, then
  `ADD CONSTRAINT ... PRIMARY KEY USING INDEX`.
- Follow a project helper in another module one call deep. superset's `create_index` is the corpus case.
- Group findings per revision and table.

Keep steer, on, with the pack-level deployment caveat of section 3.1.

### 2.2 `alembic-practice-concurrent-index-outside-autocommit`: keep (steer, on)

**Value.** It prevents a failed deploy: PostgreSQL refuses CONCURRENTLY in a transaction block (CREATE INDEX). It also
guards 2.1's steer, because an agent told to pass `postgresql_concurrently=True` adds the keyword and stops. The
sources hold up.

**Correctness.** Counterexamples are in `/tmp/review-alembic/concurrent-index-outside-autocommit/`; the run
`review-alembic-concurrent-index-outside-autocommit-cx` gave 2 findings:

| Shape | Expected | Result |
|---|---|---|
| `op.create_index(..., postgresql_concurrently=True)` in a transactional tree | report | reported (other/.../z1_concurrent.py:12) |
| The same under an env.py that connects with `execution_options(isolation_level="AUTOCOMMIT")`, so no transaction | silent | **reported** (y1_concurrent.py:12): only `transactional_ddl=False` is read |
| `postgresql_concurrently=CONCURRENT`, with the module constant `CONCURRENT = True` | report | **missed** (z1:14): only a literal True counts |
| A helper two calls deep (`upgrade -> _outer -> _inner`) | report | missed (y1:24; documented) |
| `op.execute("CREATE INDEX CONCURRENTLY ...")` in the transaction | report | missed (documented) |
| Inside `autocommit_block()`, under `if dialect.name == "postgresql":` | silent | silent |

Edit-window test on a copy of full-stack-fastapi-template (`/tmp/review-alembic/editwin-template-bad`, run
`review-alembic-editwin-template-bad`):

- A new revision with `create_index(..., postgresql_concurrently=True)` in `upgrade()` and
  `drop_index(..., postgresql_concurrently=True)` in `downgrade()` is reported on both lines.
- The same calls inside `autocommit_block()` (`editwin-template-good`) give 0 findings.

**Corpus.** 0 matches in all 8 repositories, superset included. There is no precision to measure: polar's 213 and
dify's 11 concurrent calls are all wrapped.

**Overlap.** Squawk's `ban-concurrent-index-creation-in-transaction` on offline SQL. Nothing in Ruff, mypy, Bandit or
pylint.

**Text.** The description names a keyword argument (`postgresql_concurrently=True`) and `autocommit_block()`, which is
borderline against criterion 3 ("no ... config keys"). "Run concurrent index builds in Alembic migrations inside an
autocommit block; PostgreSQL refuses CONCURRENTLY inside a transaction." says the same thing. The message is 368
characters.

**Verdict: keep**, steer, on. Two small follow-ups:

- read a module constant bound to True as True (`static_value`);
- treat an env.py whose connection sets `isolation_level="AUTOCOMMIT"` like `transactional_ddl=False`.

### 2.3 `alembic-practice-not-null-without-server-default`: keep (steer, on)

**Value.** It prevents a deploy that fails on any populated table, and it catches the `default=` misconception that
Alembic's docs call out (`add_column`: a server default uses `server_default`, "not `default` which only specifies
Python-side defaults"; checked on the 1.20 ops page). The value is high. It is rare at release commits and common in
an agent's first draft.

**Correctness.** Counterexamples are in `/tmp/review-alembic/not-null-without-server-default/`; the run
`review-alembic-not-null-without-server-default-cx` gave 5 findings:

| Shape | Expected | Result |
|---|---|---|
| `sa.Column("score", sa.BigInteger(), nullable=False)` | report | reported (n1_shapes.py:18) |
| `sa.Column("uid", sa.Uuid(), primary_key=True)`, which gets no SERIAL | report | reported (n1:20) |
| `nullable=False`, with `server_default` set only by a later `alter_column` | report | reported (n1:24) |
| `sa.Column("seq", BigInt, primary_key=True)` with `BigInt = sa.BigInteger`, which renders BIGSERIAL | silent | **reported** (n1:22): `nn_serial` matches the type's source text |
| `op.execute("DELETE FROM sessions")`, then a NOT NULL add on `sessions` | silent | reported (n1:16; rare) |
| `col = sa.Column(..., nullable=False); op.add_column("users", col)` | report | missed (n1:13; documented) |
| `server_default=sa.false()` | silent | silent |

**Corpus.** 13 matches in 3 repositories: open-webui 1, superset 3, dify 9. All 13 were read, and all 13 are
code-real:

- open-webui `2f1211949ecc_update_message_and_channel_member_table.py:68` adds `joined_at BigInteger NOT NULL` to
  `channel_member`. It sits behind an `if 'joined_at' not in cm_cols:` guard and still fails on a populated table.
- superset `2020-10-05_18-10_af30ca79208f_collapse_alerting_models_into_a_single_.py:128,130,133` passes `default=`
  only; these get the second message.
- dify's 9 add NOT NULL columns to tables created 1 to 4 revisions earlier in the same release. Examples:
  `00bacef91f18_rename_api_provider_description.py:24`, and `161cadc1af8d_add_dataset_permission_tenant_id.py:25`,
  whose own comment says "Step 1: Add column without NOT NULL constraint" above `nullable=False`. They fail only on a
  database that ran the earlier revision and got rows.

Actionable: 4 of 13 counting only tag-to-tag upgrades, 13 of 13 for teams that deploy main. Either way the fix
(`server_default=`) is safe, so steer is right.

**Overlap.** Squawk's `adding-required-field` on offline SQL. Nothing in Ruff, mypy, Bandit or pylint. The rule's own
value is the `default=` message, which offline SQL cannot explain.

**Text.** The messages are 424 and 421 characters. The first remedy is right. The second remedy ("add it nullable,
backfill, and set `nullable=False` with alter_column") walks straight into `set-not-null-scan` (2.7) when it is written
in one revision, which is what an agent does. `set-not-null-scan`'s counterexample A reports exactly that sequence. The
message should name the order: `server_default` first, and the nullable route only with the NOT NULL in a later
revision behind a validated CHECK.

**Verdict: keep**, steer, on. Optional: resolve the type through `static_value` before the SERIAL test.

### 2.4 `alembic-practice-drop-mapped-column`: rework (steer, on)

**Value.** Dropping or renaming a column the running code still reads is the canonical expand/contract outage
(strong_migrations, Squawk's `ban-drop-column`, GoCardless). But the rule sees less than it claims. It fires only while
the repository's models still map the column. Once the agent also removes the attribute in the same change, the rule
is silent, yet that is exactly the expand/contract violation:

- The deploy runs `alembic upgrade` before the new code replaces the old processes. The template does this in its
  `prestart` script, and release jobs do it too.
- The old processes' queries still name the dropped column, and they fail.

The rule catches "the migration and the models disagree". That state also breaks every query after the deploy, and
CI against a migrated database or `alembic check` catches it too. The rule does not catch "dropped in the same release
as the model change".

**Correctness.** Counterexamples are in `/tmp/review-alembic/drop-mapped-column/`; the run
`review-alembic-drop-mapped-column-cx` gave 2 findings:

| Shape | Expected | Result |
|---|---|---|
| Monorepo: `accounts/migrations` drops `users.nick`, while `billing/models.py` (another service, with its own database and Alembic tree) maps `users.nick` | silent | **reported** (accounts/.../a1_users.py:10): models are matched across the whole repository, not to the tree's env.py or metadata |
| `Profile` mapped from `__table__` with `__mapper_args__ = {"exclude_properties": ["bio"]}`; the migration drops `profiles.bio` | silent | **reported** (a1:12): following the message's own "exclude it from the mapper" keeps the finding |
| Flask-SQLAlchemy `class Widget(db.Model)` (generated table name `widget`); drop `widget.color` | report | missed (a1:14; documented gap) |
| Model attribute removed in the same change, column dropped | report (the real hazard) | silent (a1:16) |

Edit-window test (`editwin-template-bad`): `op.drop_column("user", "full_name")` in a new revision is reported while
`User` (sqlmodel, with the field inherited from `UserBase`) maps it. Dropping an unmapped column
(`editwin-template-good`) is silent. Both match the README.

**Corpus.** 0 matches in 8 repositories, by design, as claimed.

**Overlap.**

- Squawk's `ban-drop-column` and `renaming-column` flag every drop on offline SQL.
- `alembic check` (Alembic 1.9+) fails when the models and the migrated schema differ, which is the state this rule
  detects.
- No linter ties a drop to the deploy order.

**Text.** The description ("Stop mapping ... and deploy that first; drop or rename it in a later Alembic revision")
promises a deploy ordering the rule cannot see (criterion 6, accuracy). The DropMappedColumn message's "or exclude it
from the mapper" is not honoured by the rule (the `exclude_properties` row above). The messages are 355 to 448
characters.

**Verdict: rework.**

- Restrict models to the Alembic tree the migration runs under: the models its env.py's `target_metadata` imports, or
  at least the same top-level package.
- Honour `exclude_properties`.
- Re-word the description and messages to the check the rule makes. For example: "a model still maps this column:
  remove the attribute (ship that first if migrations run before the new code is live)".

Keep steer, on: in the edit window the rule is precise for what it checks.

### 2.5 `alembic-practice-constraint-without-not-valid`: rework (steer, on)

**Value.** The hazard is real. A validated FOREIGN KEY or CHECK add scans the table while holding a lock that blocks
writes, and ADD FOREIGN KEY also takes SHARE ROW EXCLUSIVE on the referenced table (PostgreSQL ALTER TABLE;
strong_migrations; Squawk). The sources hold up, including SQLAlchemy's `postgresql_not_valid` on
`ForeignKeyConstraint` and `CheckConstraint`.

**Correctness.** Counterexamples are in `/tmp/review-alembic/constraint-without-not-valid/`; the run
`review-alembic-constraint-without-not-valid-cx` gave 4 findings:

| Shape | Expected | Result |
|---|---|---|
| `op.create_foreign_key("fk_orders_shop", "orders", "shops", ...)` | report | reported (c1_shapes.py:16) |
| `batch_op.create_check_constraint("ck_total", "total >= 0")` | report | reported (c1:24) |
| `op.add_column("orders", sa.Column("user_id", sa.Integer(), sa.ForeignKey("users.id")))` | report | **missed** (c1:12) |
| `create_foreign_key(..., **PG_NOT_VALID)` with `PG_NOT_VALID = {"postgresql_not_valid": True}` | silent | reported (c1:18; rare) |
| `op.execute("CREATE TABLE audit ...")`, then an FK on `audit` | silent | reported (c1:21; documented) |
| `op.execute("ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY ...")` | report | missed (documented) |

The `add_column` with an inline `ForeignKey` is the hand-written form of the same hazard. Alembic 1.20.1 renders it as
`ALTER TABLE orders ADD COLUMN user_id INTEGER;` followed by
`ALTER TABLE orders ADD FOREIGN KEY(user_id) REFERENCES users (id);` (offline probe above): a validated add.

Alembic's docs add that `add_column(..., inline_references=True)` renders the reference inline, which "can improve
performance on large tables since the constraint is marked as valid immediately for nullable columns" (ops page; not
verified on a database here). That would be the cheaper fix for the commonest corpus shape, an FK on a column the same
`upgrade()` just added:

- dispatch `tenant/versions/2022-09-02_f9384b873721.py:22`;
- dispatch `tenant/versions/2024-01-17_d4bbb234d0bc.py:21`;
- dispatch `tenant/versions/2022-08-02_cec9ab5c5c8e.py:23`.

The message does not mention it.

**Corpus.** 221 matches in 8 repositories: template 2, jupyterhub 2, mealie 10, open-webui 2, dispatch 163, polar 5,
superset 20, dify 17. 65 were read:

| Repository | Read | Code-real | Actionable |
|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 2 of 2 | 2 | 2 |
| jupyterhub/jupyterhub | 2 of 2 | 2 | 0 (Hub stopped for `upgrade-db`) |
| mealie-recipes/mealie | 10 of 10 | 10 | 0 (migrates before serving) |
| open-webui/open-webui | 2 of 2 | 2 | 2 |
| Netflix/dispatch | 17 of 163 (every 10th) | 17 | 17 |
| polarsource/polar | 5 of 5 | 5 | 5 |
| apache/superset | 10 of 20 (every 2nd) | 10 | 10 |
| langgenius/dify | 17 of 17 | 17 | 4 |

13 of dify's 17 are CHECKs on `knowledge_fs_*` tables created by `2026_07_21_1400-c8e31b7d52a4` and constrained by it
and by two revisions the same day (`d4f6e8a1c305`, `e5a7c9b2d416`). Those tables are empty when the release deploys.
This is the library's "earlier revision of the same release" gap. The real matches include:

- dispatch `tenant/versions/2021-06-15_8a558baeef05.py`, which drops and re-adds 93 FKs with `ondelete="CASCADE"`;
- superset `2022-03-16_21-07_8b841273bec3_sql_lab_models_database_constraint_updates.py:58` (`tab_state`);
- template `1a31ce608336_add_cascade_delete_relationships.py:26` (`item`).

Actionable precision is 62% overall and 75% on repositories that migrate under traffic. Following the steer costs
little even on an empty table, so steer stays, but the figure is below the BRIEF's 90% and the README should say so.

**Overlap.** Squawk's `adding-foreign-key-constraint` and `constraint-missing-not-valid` on offline SQL; Squawk's
`adding-foreign-key-constraint` page shows the Alembic fix. Nothing in Ruff, mypy, Bandit or pylint.

**Text.** The description says "validate them in a later revision", while the message also allows an autocommit block
in the same revision; align the two. The messages are 495 and 453 characters.

**Verdict: rework.**

- Report the `add_column(sa.Column(..., sa.ForeignKey(...)))` shape on an existing table, and an inline
  `sa.CheckConstraint` in an added Column.
- For an FK on a column the same `upgrade()` adds, name `inline_references=True` in the message.

Keep steer, on.

### 2.6 `alembic-practice-column-type-rewrite`: rework (steer, on after the fix; log until then)

**Value.** The hazard is real. ALTER COLUMN TYPE "will normally cause the entire table and its indexes to be
rewritten" under ACCESS EXCLUSIVE unless the types are binary coercible (PostgreSQL ALTER TABLE notes; Squawk's
`changing-column-type`; strong_migrations). dify's `workflow_runs.total_tokens` going from INTEGER to BigInteger is the
textbook case.

**Correctness.** Types are compared by callee name and source text, with no synonym table. So a same-type change
spelled two ways is reported as a rewrite. Counterexamples are in `/tmp/review-alembic/column-type-rewrite/`; the run
`review-alembic-column-type-rewrite-cx` gave 8 findings:

| Shape | What PostgreSQL does | Result |
|---|---|---|
| `existing_type=sa.TIMESTAMP()`, `type_=sa.DateTime()` | offline probe: `TYPE TIMESTAMP WITHOUT TIME ZONE`, the same type, no rewrite | **reported** (t1_shapes.py:13) |
| `existing_type=postgresql.UUID()`, `type_=sa.Uuid()` | offline probe: `TYPE UUID`, the same type | **reported** (t1:15) |
| `existing_type=sa.INTEGER()`, `type_=sa.Integer()`, next to `nullable=True` | same type | **reported** (t1:17) |
| `existing_type=sa.VARCHAR(length=255)`, `type_=sqlmodel.sql.sqltypes.AutoString(length=255)` | VARCHAR(255) both ways | **reported** (t1:19) |
| `existing_type=sa.BOOLEAN()`, `type_=sa.Boolean()` | same type | **reported** (t1:21) |
| INTEGER to BigInteger | rewrite | reported (t1:23) |
| `ARRAY(VARCHAR(50))` to `ARRAY(String(100))` | not established here | reported (t1:27) |
| `DateTime()` to `DateTime(timezone=True)` | rewrite unless the session time zone is UTC (documented gap) | reported (t1:29) |
| `op.execute("ALTER TABLE ... TYPE numeric(12, 2)")` | rewrite | missed (documented) |

None of the five same-type spellings occurs in an `upgrade()` of the corpus. A grep of every
`alter_column(..., type_=...)` finds them only in downgrades, so the corpus precision is unaffected. But agents that
write an `alter_column` for a nullability change routinely restate the type. A steer that sends them to "add a new
column, backfill, switch, drop" for a no-op is the worst kind of false positive.

**Corpus.** 34 matches in 6 repositories: template 4, mealie 2, open-webui 3, dispatch 2, superset 12, dify 11. All 34
were read, and all 34 are code-real type changes. 32 are actionable; mealie's 2 are not, because mealie migrates before
it serves.

- Real, on tables that grow: dify `2025_01_01_2000-a91b476a53de_change_workflow_runs_total_tokens_to_.py:23`
  (INTEGER to BigInteger), `2025_02_27_0917-d20049ed0af6_add_metadata_function.py:95` (JSON to JSONB) and
  `e35ed59becda_modify_quota_limit_field_type.py:21,25`; open-webui `c29facfe716b_update_file_table_path.py:34`
  (Text to JSON); dispatch `tenant/versions/2024-11-04_928b725d64f6.py:29` (BOOLEAN to String).
- Real but cheap: superset's 11 `VARCHAR(256)`/`String(512)` to `String(255)` shrinks
  (`2016-05-11_17-28_956a063c52b3_adjusting_key_length.py`, `2018-03-20_19-47_f231d82b9b26_.py`, `7f2635b51f5d`,
  `e9df189e5c7e`), all on Druid and metadata tables that stay small, and the template's four `String()` to
  `String(255)` changes on `user` and `item`.
- dify's `2025_11_15_2102-09cfdda155d1_mysql_adaptation.py` matches sit in the `if _is_pg(conn):` branch: correctly
  reported.

**Overlap.** Squawk's `changing-column-type` flags every type change on offline SQL; it cannot tell a widening either,
and it has no SQLAlchemy type names. Nothing in Ruff, mypy, Bandit or pylint.

**Text.** The description is fine. The messages are 401 and 449 characters.

**Verdict: rework.** Normalise SQLAlchemy type names to the PostgreSQL type before comparing, and treat equal
normalised types as safe:

- TIMESTAMP and DateTime without a time zone;
- UUID and Uuid;
- INTEGER and Integer;
- BOOLEAN and Boolean;
- AutoString and String;
- TEXT and Text.

Keep steer, on, once that lands. Until then, log is the honest mode for agent edits.

### 2.7 `alembic-practice-set-not-null-scan`: keep (log, on)

**Value.** The hazard is real. SET NOT NULL scans the table under ACCESS EXCLUSIVE unless a valid CHECK proves the
column holds no NULL (PostgreSQL ALTER TABLE). polar's own newest revisions switched to the CHECK pattern, with the
comment the README quotes.

**Correctness.** Counterexamples are in `/tmp/review-alembic/set-not-null-scan/`; the run
`review-alembic-set-not-null-scan-cx` gave 1 finding:

| Shape | Expected | Result |
|---|---|---|
| `add_column(nullable=True)`, `UPDATE`, `alter_column(nullable=False)`: the not-null rule's second remedy | report | reported with the after-add message (s1_shapes.py:15) |
| `VALIDATE CONSTRAINT fk_users_email_domain` (an unrelated FK whose name contains `email`), then `alter_column("users", "email", nullable=False)` | report | **silent** (s1:18): a VALIDATE whose name contains the column exempts it (documented gap) |
| `op.execute("ALTER TABLE users ALTER COLUMN name SET NOT NULL")` | report | missed (s1:20; documented) |
| `create_check_constraint(..., "age IS NOT NULL", postgresql_not_valid=True)`, then VALIDATE and SET NOT NULL in `autocommit_block()` | silent | silent |

**Corpus.** 77 matches in 6 repositories: template 3, mealie 3, dispatch 15, polar 19, superset 19, dify 18. 56 were
read:

| Repository | Read | Code-real | Actionable |
|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 3 of 3 | 3 | 3 |
| mealie-recipes/mealie | 3 of 3 | 3 | 0 (migrates before serving) |
| Netflix/dispatch | 15 of 15 | 12 | 12 |
| polarsource/polar | 19 of 19 | 19 | 19 |
| apache/superset | 7 of 19 (every 3rd) | 7 | 7 |
| langgenius/dify | 9 of 18 (every 2nd) | 9 | 9 |

dispatch's 3 that are not real are `tenant/versions/2022-08-18_86ec96cd14f8.py:25,34,43`. The same `upgrade()` runs
`ALTER TABLE ... ADD COLUMN id SERIAL PRIMARY KEY`, which rewrites the table anyway. Real matches include:

- dispatch `tenant/versions/2022-08-29_479024506e05.py:52-55` (`case` and `incident`, after an ORM backfill);
- dispatch `tenant/versions/2023-06-06_0a19c5a4c2ba.py:19`, which renames a column and sets it NOT NULL in one call;
- dify `2024_11_01_0623-09a8d1878d9b_update_workflows_graph_features_and_.py:35` (`messages.inputs`);
- polar `2026-11-03-1201_make_subscription_started_at_non_nullable.py`.

**Overlap.** Squawk's `adding-not-nullable-field` on offline SQL. Nothing in Ruff, mypy, Bandit or pylint.

**Text.** The description embeds SQL (`CHECK (c IS NOT NULL) NOT VALID`, against criterion 3), and "behind a validated
... NOT VALID constraint" reads as a contradiction. Suggested: "Make an existing column NOT NULL in Alembic migrations
behind a CHECK constraint validated beforehand; a plain SET NOT NULL scans the table under an exclusive lock." The
messages are 447 and 455 characters.

**Log mode is right.** The fix spans two deploys, and about half the matches are on tables that stay small (the
template's `user` and `item`, superset's metadata tables). A steer would push a four-step dance onto a 20-row table.
Log keeps the data without that cost.

### 2.8 `alembic-practice-missing-lock-timeout`: rework (log, on)

**Value.** The hazard is real and well sourced: Squawk's `require-lock-timeout`, GoCardless, PostgresAI and Xata. But
in 7 of 8 repositories the omission is one decision in env.py, and the rule reports it once per revision.

**Correctness.** Counterexamples are in `/tmp/review-alembic/missing-lock-timeout/`; the run
`review-alembic-missing-lock-timeout-cx` gave 1 finding:

| Shape | Expected | Result |
|---|---|---|
| env.py does `from app.db import engine`, and `app/db.py` builds it with `connect_args={"options": "-c lock_timeout=5000"}` | silent | **reported** (migrations/versions/l1_add.py:12): only strings in env.py itself count |
| env.py holds `log.info("remember to set lock_timeout one day")` | report | **silent** (other/.../m1_add.py): any string that mentions lock_timeout protects the tree |
| `op.execute("SET LOCAL lock_timeout = '0s'")`, which turns it off | report | **silent** (third/.../k1_add.py): only `= 0`, `=0`, `to 0` and `= '0'` are read as off |
| `SET LOCAL lock_timeout` above an `autocommit_block()` that holds a non-concurrent `add_column` | report | silent (k2_add.py; documented gap) |

**Corpus.** 579 matches in 8 repositories: template 4, jupyterhub 16, mealie 48, open-webui 49, dispatch 128, polar 3,
superset 153, dify 178. 121 were read:

| Repository | Read | Code-real | Actionable |
|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template | 4 of 4 | 4 | 4 |
| jupyterhub/jupyterhub | 16 of 16 | 16 | 0 |
| mealie-recipes/mealie | 6 of 48 (every 8th) | 6 | 0 |
| open-webui/open-webui | 9 of 49 (every 6th) | 9 | 9 |
| Netflix/dispatch | 16 of 128 (every 8th) | 16 | 16 |
| polarsource/polar | 3 of 3 | 3 | 2 |
| apache/superset | 31 of 153 (every 5th) | 31 | 31 |
| langgenius/dify | 36 of 178 (every 5th) | 36 | 36 |

The polar match that is not actionable drops an index of `pledge_transactions`, a table nothing queries any more
(`2026-08-12-1000_drop_pledge_transactions_table.py:22`). The other two polar matches,
`2026-10-23-1000_add_agents_claim_polled_at.py:22` and `2026-11-03-1201_make_subscription_started_at_non_nullable.py:21`,
are the rule at its best: slips in a tree where 182 of 205 revision files set lock_timeout. The other 576 matches
restate one omission per tree, dispatch's two env.py files and dify's, superset's, open-webui's and mealie's single
env.py, once per revision.

**Overlap.** Squawk's `require-lock-timeout` and `require-timeout-settings` on offline SQL. Squawk sees a `SET` only
when the offline render emits it. Nothing in Ruff, mypy, Bandit or pylint.

**Text.** The message is 528 characters.

**Verdict: rework.**

- Report a tree's missing setting once, at env.py's `context.configure(...)`, when no revision of the tree sets
  lock_timeout.
- Report per revision only in trees where most revisions set it (polar), where a missing one is a slip. `empty-downgrade`
  already reads a tree convention this way.
- Count a lock_timeout set in the module env.py imports its engine from.
- Read `'0s'` and `'0ms'` as off.
- Ignore strings that are not SQL, such as a log call's argument.

Log mode is right. Steer only in the per-revision form for convention trees.

### 2.9 `alembic-practice-multiple-heads`: rework (steer, on)

**Value.** It prevents a failed deploy, and it catches the merge conflict that two agents or two branches create.
Alembic's and pytest-alembic's sources hold up.

Edit-window test (`editwin-template-bad`): a revision written on `1a31ce608336`, the parent of the head, is reported
both at its own `down_revision` and at the current head `fe56fa70289e`. The same revision written on the head
(`editwin-template-good`) is silent.

**Correctness.** Two of the rule's deliberate exemptions are the exact failure it exists to catch. Both were verified
with Alembic 1.20.1 (`alembic heads` and `alembic upgrade head --sql` in `/tmp/review-alembic/offline/heads/`):

- **Two bases in one directory**: a1 <- a2, plus b1 with `down_revision = None` and no branch labels. `alembic heads`
  lists `a2 (head)` and `b1 (head)`, and `upgrade head` fails with "Multiple head revisions are present for given
  argument 'head'". The rule leaves this alone and documents it as a gap. An agent that writes `down_revision = None`
  in a new revision creates this shape.
- **`depends_on` without `branch_labels`**: p1 <- p2, and p1 <- p3 with `depends_on = "p2"`. `alembic heads` lists
  `p2 (effective head)` and `p3 (head)`, and `upgrade head` fails the same way. The rule treats `depends_on` as an
  intentional branch and stays silent; the README concedes that Alembic "still counts both heads".

Counterexamples are in `/tmp/review-alembic/multiple-heads/` (one tree per subdirectory); the run
`review-alembic-multiple-heads-cx` gave 4 findings:

| Tree | Expected | Result |
|---|---|---|
| a: two bases in one directory | report | **silent** |
| b: `depends_on` only | report | **silent** |
| c: plain fork (q2 and q3 on q1) | report | reported (q2.py:6, q3.py:6) |
| d: `down_revision = ("r1",)` (a tuple of one) and `"r1"` | report | reported (r2.py:6, r3.py:6). r2 gets the merge-heads message ("if one of the two is a merge revision ... delete it") although it is no merge |
| e: `down_revision = PARENT`, with the constant `PARENT = "s1"` | report | **silent** (documented: non-literal) |

**Corpus.** 0 matches in 8 repositories. Polar was re-run on a complete copy of its revision files
(`review-alembic-heads-polar-trim2`): 0.

**Overlap.** pytest-alembic's `test_single_head_revision`, Alembic's own error at deploy, and `alembic heads` in CI.
No linter.

**Text.** The messages are 531 and 482 characters.

**Verdict: rework.**

- Drop the `depends_on` exemption. Keep only `branch_labels`, which lets `upgrade <label>@head` address the branch.
- Report two heads in one version directory that share no ancestor when neither line carries branch labels.
- Resolve a `down_revision` bound to a module constant.
- Read a tuple of one as a single parent.

Keep steer, on.

### 2.10 `alembic-practice-rename-as-drop-add`: rework (log, off)

**Value.** The hazard is real data loss. It is the shape autogenerate produces when an agent renames a model attribute
(Alembic, "What does Autogenerate Detect": renames come out "as a column add/drop pair"). It is rare at release
commits.

**Correctness.** Counterexamples are in `/tmp/review-alembic/rename-as-drop-add/`; the run
`review-alembic-rename-as-drop-add-cx` gave 1 finding:

| Shape | Expected | Result |
|---|---|---|
| An autogenerated rename through `batch_alter_table` (`teams.name` to `title`, both VARCHAR(80)) | report | reported (r2_rename.py:12) |
| The same autogenerated rename (`users.nick` to `handle`) with `op.execute("SET LOCAL lock_timeout = '5s'")` as the first line | report | **silent** (r1_rename.py): any `.execute(...)` counts as moving data |
| A rename that also widens (`orgs.label` VARCHAR(40) to `display` String(80)) | report | **silent** (r3_rename.py): two different literal lengths read as different types |

The first silent case means the fix `missing-lock-timeout` asks for, which is also polar's first line in 182
revisions, switches this rule off.

**Corpus.** 2 matches in 1 repository (dify); both were read:

- real: `00bacef91f18_rename_api_provider_description.py:25` drops `description_str` and adds `description`, both
  `LongText`, so every provider's description is deleted;
- not a rename: `2e9819ca5b28_add_tenant_id_in_api_token.py:26` swaps `dataset_id` for `tenant_id`, a different
  reference.

1 of 2.

**Overlap.** None: Squawk sees an ADD COLUMN and a DROP COLUMN as unrelated statements.

**Text.** The description contains code (`op.alter_column(new_column_name=...)`, against criterion 3). Suggested: "Rename
a column in place, or copy its data first, instead of dropping it and adding a new one of the same type in one Alembic
upgrade." The message is 457 characters.

**Verdict: rework.**

- Count as data movement only an UPDATE or INSERT (in SQL text, or as an `update()`/`insert()` construct) or a
  session/ORM write, not any `execute`.
- Let a widened length still pair, as a rename with a widening.

Keep log, off until a larger sample exists.

### 2.11 `alembic-practice-model-import-in-migration`: keep (steer, on)

**Value.** zzzeek's comment on sqlalchemy/alembic#679 reads as quoted ("you want this migration to remain functional
in perpetuity, and your ORM models may have changed so would no longer work in this migration once it's older"). He
also lists acceptable alternatives, such as a model local to the migration or rolling up old migrations; the message
names the local model. jupyterhub's `651f5419b74d_api_token_scopes.py` shows the cost: it has to
`defer(orm.Spawner.display_name)`.

**Correctness.** Counterexamples are in `/tmp/review-alembic/model-import-in-migration/`; the run
`review-alembic-model-import-in-migration-cx` gave 2 findings:

| Shape | Expected | Result |
|---|---|---|
| `from app.models import User` inside `upgrade()`, then `session.query(User)` | report | reported (m1_shapes.py:12) |
| A Flask-SQLAlchemy `class Widget(db.Model)` imported and queried | report | missed (m1; documented gap) |
| `from app.models import User`, used only as `op.add_column(User.__tablename__, ...)` | debatable | reported (m2_tablename.py:5) |

The last row is debatable. No query names a column, so the message's "this migration's queries name columns that do
not exist yet" is wrong for it. The import still loads the whole application at migration time.

**Corpus.** 13 matches in 4 repositories, all read, all real and actionable:

- jupyterhub 4: `651f5419b74d_api_token_scopes.py:83,89,121,136`;
- mealie 7: `04ac51cbe9a4`, `0341b154f79a`, `dded3119c1fe`;
- dispatch 1: `tenant/versions/2024-10-16_3c49f62d7914.py:12`;
- superset 1: `2021-08-09_17-32_07071313dd52_change_fetch_values_predicate_to_text.py:36`.

mealie's `GUID` imports and dify's `StringUUID` and `LongText` imports stay silent.

**Overlap.** django-migration-linter's RUNPYTHON_MODEL_IMPORT is Django-only. There is nothing for Alembic.

**Text.** The description's "never import" is absolute, while the message permits enums, constants and local models.
The messages are 542 and 549 characters.

**Verdict: keep**, steer, on. Optional: a shorter message ("use the table name literal") for a model used only for
`__tablename__` or `__table__.name`.

### 2.12 `alembic-practice-empty-downgrade`: keep (log, on)

**Value.** Moderate: a silent `alembic downgrade` moves the version table without reversing the schema. The sources
(the Alembic cookbook, pytest-alembic) hold up as general practice. No source calls an empty downgrade an outage.

**Correctness.** Counterexamples are in `/tmp/review-alembic/empty-downgrade/` and `empty-downgrade2/`. The run
`review-alembic-empty-downgrade-cx` gave 0 findings and `review-alembic-empty-downgrade-cx2` gave 1:

| Shape | Expected | Result |
|---|---|---|
| `def downgrade(): pass` after `op.add_column`, in a tree of 6 implemented revisions | report | reported (e10.py:13, "7 of the 8 revisions") |
| `def downgrade(): return` | report | **silent**, and counted as implemented (e7) |
| A `downgrade()` with only `# TODO` and `pass` | report | **silent** (e8): any comment excuses it |
| An upgrade of raw DDL (`op.execute("ALTER TABLE ... ADD COLUMN ...")`) with an empty downgrade | report | silent (e9; documented) |
| A second empty downgrade (a `drop_table`) in the same 9-revision tree | report both | **both silent** (first run): 7 of 9 is under 80% |

The last row is by design, but worth knowing: two empty downgrades in a young tree switch the rule off for the whole
tree.

**Corpus.** 3 matches in 2 repositories, all read and all real:

- dispatch `tenant/versions/2023-11-29a_bdaeabba3e53.py:26`;
- superset `2016-03-13_21-30_18e88e1cc004_making_audit_nullable.py:104`;
- superset `2020-12-23_11-34_c878781977c6_alert_reports_shared_uniqueness.py:101`.

**Overlap.** None for Alembic.

**Text.** The message is 657 characters, the longest in the pack.

**Verdict: keep**, log, on. Read a bare `return` as filler, and accept a comment as an excuse only when it is not
`TODO` or `FIXME`.

## 3. Pack-level findings

### 3.1 Precision is measured leniently, and inconsistently across the lock rules

The README counts a match as real "even when the table is small enough that the lock is harmless there". It then
applies the deployment test to `missing-lock-timeout` alone, where jupyterhub and mealie score 0 real. For three other
rules it counts the same two repositories' matches as real:

- `index-not-concurrent`: mealie 151 of 296;
- `constraint-without-not-valid`: mealie 10, jupyterhub 2;
- `set-not-null-scan`: mealie 3.

Both repositories migrate with no other client, by design. The pack's `match` recommends it to every repository with
`alembic` installed, so these teams get it. A team like mealie would see 151 index findings on its history, and one
per index on every new revision, for a hazard it does not have.

The rules cannot see the deployment. But the README should report actionable precision for every lock rule, and the
pack should say up front that its five lock rules (index, constraint, type, set-not-null, lock-timeout) assume
migrations run against a database that serves traffic. Either of these would let such projects turn the five off
together:

- a single pack-level switch for "migrations run with the app stopped";
- detecting a startup `command.upgrade(...)` in application code. This is weaker: open-webui migrates at startup too,
  yet runs multi-replica.

### 3.2 Shapes the existing rules should own (not new rules)

`add_column` emits extra DDL, verified with Alembic 1.20.1's offline output (`/tmp/review-alembic/offline/probe.py`).
Three of the four missed shapes below come from that; the fourth is `create_primary_key`.

| Shape | DDL emitted | Owner rule | Today |
|---|---|---|---|
| `add_column(t, Column(..., unique=True))` | `ALTER TABLE t ADD UNIQUE (c)` | index-not-concurrent | missed |
| `add_column(t, Column(..., index=True))` | `CREATE INDEX ix_t_c ON t (c)` | index-not-concurrent | missed |
| `add_column(t, Column(..., ForeignKey(...)))` | `ALTER TABLE t ADD FOREIGN KEY(c) REFERENCES ...` | constraint-without-not-valid | missed |
| `create_primary_key(name, t, cols)` on an existing table | `ADD CONSTRAINT ... PRIMARY KEY` (a unique index build under ACCESS EXCLUSIVE) | index-not-concurrent | missed (template `d98dd8ec85a3:54`) |

The NOTES' dropped candidate "add-column-unique-ignored" rests on Alembic's documentation note ("As of 1.18.2 ...
ignored: unique ... index"), which the 1.20.1 renderer contradicts. The real hazard is the opposite: the constraint
and the index are built, and nothing sees them.

### 3.3 Missing rules, with evidence

- **DROP INDEX without CONCURRENTLY** on an existing table: the drop takes ACCESS EXCLUSIVE on the table (PostgreSQL
  DROP INDEX). Squawk has `require-concurrent-index-deletion`. The pack's concurrent-index rule handles
  `drop_index(postgresql_concurrently=True)`, but the plain drop is unflagged. It is small to add to 2.1. dify's
  `2025_06_19_1633-0ab65e1cc7fa` and `2026_01_16_1715-288345cd01d1` drop indexes on `workflow_runs` and
  `workflow_node_executions` this way.
- **A volatile `server_default` on `add_column`** (`gen_random_uuid()`, `random()`, `clock_timestamp()`; `now()` is
  stable and fine): a table rewrite even on PostgreSQL 11+. NOTES dropped it for "0 on existing tables" in the corpus.
  It is exactly what an agent writes for "add a UUID column to users", and Squawk's `adding-field-with-default` covers
  it. It is worth a rule in log mode.
- **Raw SQL in `op.execute`.** Every lock rule documents "raw DDL in op.execute is not read". One shared library
  relation that classifies `op.execute` strings would close the gap for all of them at once: CREATE INDEX without
  CONCURRENTLY, ADD CONSTRAINT without NOT VALID, SET NOT NULL, and ALTER COLUMN ... TYPE. The corpus has instances:
  dispatch's `ALTER TABLE ... ADD COLUMN id SERIAL PRIMARY KEY` (`86ec96cd14f8`), and superset's raw `ALTER TABLE` in
  `a39867932713` and `07f9a902af1b`.
- `ALTER TYPE ... ADD VALUE` in the transaction: I agree with the NOTES' drop (PostgreSQL 12+ runs it, and a use in the
  same revision fails loudly in CI).

### 3.4 Rules that belong elsewhere

None. All twelve are Alembic-specific. `model-import-in-migration` and `drop-mapped-column` read SQLAlchemy models, but
users would look for them here (BRIEF: "SQLModel and Flask-SQLAlchemy code counts as SQLAlchemy code"). Both have the
Flask-SQLAlchemy gap: `db.Model` subclasses without `__tablename__` are invisible to both, which matters for
Flask-SQLAlchemy applications.

### 3.5 The library `rules/lib/alembic.rerail`

Good:

- one resolution of `op` through imports, batch handles, `op.f` and module constants;
- a revision graph with directory-first id resolution;
- env.py trees;
- a dialect-branch reading (`al_non_pg`) that the corpus forced to be careful: negated tests, early-return guards,
  dialect classes.

The interface comment matches what the counterexamples exercised.

Weak spots:

- **`al_on_existing`.** It means "a table that already holds rows", but tests only "not created in this `upgrade()`".
  Every lock rule inherits its blind spots:
  - a table created by an earlier revision that has not shipped yet: dify's 13 `knowledge_fs_*` checks, and any agent
    branch with two revisions;
  - a table created by `op.execute("CREATE TABLE ...")` (counterexample 2.5);
  - a table emptied first (counterexample 2.3).
- **`al_non_pg` reads lowercase substrings** of the `if` test (`"sqlite"`, `"_pg"`, `"not "`).
  - `if not settings.USE_SQLITE_FOR_TESTS:` reads as a negated other-database test.
  - `if "postgres" in url and not readonly:` reads as a negated PostgreSQL test, so PostgreSQL's own branch is marked
    non-PostgreSQL.

  Neither shape occurs in the corpus, but the reading is fragile.
- **No project-level dialect.** A SQLite-only application (an `alembic.ini` URL of `sqlite:`, `render_as_batch=True`)
  gets every PostgreSQL lock finding (counterexample 2.1).
- **Dead interface.** `al_sets_lock_timeout` and `al_model_column` are used by no rule. `missing-lock-timeout` uses its
  own `mlt_setting`, and `drop-mapped-column` its own `dm_model_column`.
- **`al_model` matches by `__tablename__` across the whole repository.** So two services with a `users` table in one
  monorepo share models (counterexample 2.4). `al_env_tree` already pairs revisions with env.py; the models side has no
  equivalent.
- **Helpers in another module are not followed.** superset's `migrations/shared/utils.py` (`create_index`,
  `add_columns`, `drop_columns`, `create_table`) is used by most of its recent revisions. Only `create_table` is
  special-cased, by name.

### 3.6 Text across the pack

All 22 messages exceed the 200-character cap of `docs/pack-format.md` (criterion 6): they run from 355 to 657
characters, with a median of about 450. The published django pack exceeds the cap too (25 of 34 messages, median 300),
so the cap is not enforced in practice. But this pack is the most verbose of the Python packs measured (sqlalchemy
median 312, celery 273, fastapi 391). In a steer block that lists several findings, a 400-character message for each
index of a 97-index revision is unreadable.

Three descriptions embed code, against criterion 3:

- `postgresql_concurrently=True` (concurrent-index-outside-autocommit);
- `CHECK (c IS NOT NULL) NOT VALID` (set-not-null-scan);
- `op.alter_column(new_column_name=...)` (rename-as-drop-add).

### 3.7 Overlap, pack-wide

The README and NOTES say Squawk and django-migration-linter "check the same hazards, but on SQL files ...; neither
reads Alembic's Python, so neither sees a batch_alter_table block, which tables the same revision creates". But Squawk's
own docs give `alembic upgrade head --sql | squawk` as its Alembic integration. `alembic-squawk`,
`alembic-squawk-action` and PandaDoc's `migration-lint` (Django, Alembic, raw SQL) package that pipeline. Offline SQL
renders batch operations as plain ALTERs, and Squawk ignores indexes on tables created in the same transaction, so two
of the three claimed advantages do not hold.

The real advantages are these, and the README should state them instead:

- no offline render step and no database;
- revisions that cannot render offline (inspector calls, data migrations) are still read;
- the cross-file rules that no SQL linter can do: drop-mapped-column, model-import-in-migration, multiple-heads,
  empty-downgrade, and the env.py part of missing-lock-timeout.

Ruff (the `python` pack's ports), mypy, Bandit and pylint have nothing on any of the 12 shapes. Ruff S608 and Bandit
B608 cover string-built SQL in `op.execute`, as NOTES says.

### 3.8 Can the pack be pitched to a team on its own?

Yes, for a team that deploys Alembic migrations against a live PostgreSQL database.

- Four failed-deploy and fresh-install rules are precise: not-null-without-server-default,
  concurrent-index-outside-autocommit, multiple-heads (after its fix) and model-import-in-migration. Each prevents a
  broken deploy or a broken fresh install.
- The lock rules are the standard strong_migrations and Squawk checklist, brought into the edit loop.

It is a weaker pitch in two cases:

- self-hosted single-process apps (mealie, jupyterhub, open-webui's single-container installs), where the lock rules
  are noise;
- teams that already run Squawk on `alembic upgrade --sql`, where the lock rules duplicate it and the pack's value is
  the cross-file rules.

## 4. Corpus and commands

### Repositories

The 8 Alembic repositories of BRIEF.md, at the `MANIFEST.tsv` commits:

| Repository | Commit | How it was run | Files | Findings (all 12 rules) |
|---|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a7` | whole repository | 43 | 13 |
| jupyterhub/jupyterhub | `3ca37442` | whole repository | 158 | 22 |
| mealie-recipes/mealie | `8dbb9280` | whole repository | 723 | 221 |
| open-webui/open-webui | `0666e59d` | whole repository | 330 | 84 |
| Netflix/dispatch | `dd2837e8` | whole repository | 717 | 315 |
| polarsource/polar | `872272fb` | trimmed copy (below) | 831 | 44 |
| langgenius/dify | `724ac151` | trimmed copy (below) | 316 | 308 |
| apache/superset | `fdf61721` | trimmed copy (below) | 981 | 231 |

The total is 1,238 findings. Per rule: index 296, constraint 221, lock-timeout 579, set-not-null 77, type 34,
not-null 13, model-import 13, empty-downgrade 3, rename 2, and 0 for the other three. Every count equals the README's.

**Trimmed copies.** polar, dify and superset ran on copies under `/tmp/review-alembic/corpus/`, as the lead allowed,
because 60 to 85 `corpus-run` jobs were queued machine-wide.

- polar and superset: every non-test file that mentions `sqlalchemy`, `sqlmodel`, `__tablename__` or `alembic`, every
  non-test `__init__.py` and `alembic.ini`, plus every file with a module-level `down_revision`. That last addition
  restored 2 polar revisions and 68 superset revisions that mention none of the words.
- dify (`dify-small`): `api/migrations/**`, `api/models/**`, every `api/` file that declares `__tablename__`,
  `DeclarativeBase`, `declarative_base` or a `Table(...)`, and their packages' `__init__.py`.

polar's main run predates the revision-file addition, so `multiple-heads` and `empty-downgrade` were re-run on the
complete copy (833 files, run `review-alembic-heads-polar-trim2`): 0 findings, as before. On these copies the engine's
counts equal the README's for every rule. The dify index, not-null and set-not-null findings are identical, line for
line, to the writers' whole-repository runs.

### Commands

All runs used `~/rerail-bench/python-arch/bin/corpus-run`. The findings are under
`~/rerail-bench/python-arch/runs/<run>/`.

- Corpus, all 12 rules per run:
  - `review-alembic-all-fastapi__full-stack-fastapi-template`
  - `review-alembic-all-jupyterhub__jupyterhub`
  - `review-alembic-all-mealie-recipes__mealie`
  - `review-alembic-all-open-webui__open-webui`
  - `review-alembic-all-Netflix__dispatch`
  - `review-alembic-all-polar-trim`
  - `review-alembic-all-dify-small`
  - `review-alembic-all-superset-trim`
- Counterexamples, one run per rule: `review-alembic-<rule>-cx` on `/tmp/review-alembic/<rule>/`, plus
  `review-alembic-empty-downgrade-cx2` on `/tmp/review-alembic/empty-downgrade2/`.
- Edit-window tests on copies of full-stack-fastapi-template, with multiple-heads, drop-mapped-column and
  concurrent-index-outside-autocommit:
  - `review-alembic-editwin-template-bad`: 5 findings, all expected;
  - `review-alembic-editwin-template-good`: 0 findings.
- Alembic behaviour checks (alembic 1.20.1, SQLAlchemy 2.1.9, through `uv run --with alembic --with sqlalchemy`):
  - `/tmp/review-alembic/offline/probe.py`: offline DDL for `add_column` with `ForeignKey`, `unique=True`,
    `index=True` and `primary_key=True`; for TIMESTAMP to DateTime and UUID to Uuid; and for `create_primary_key`;
  - `/tmp/review-alembic/offline/heads/`: `alembic heads` and `alembic upgrade head --sql` for two bases, and for a
    `depends_on`-only fork.
- Case tests: `nice -n 19 node dist/cli/cli.js rules test packs/alembic/rules/<rule>.rerail` for all 12 rules: all
  passed.

Sources opened: PostgreSQL CREATE INDEX and ALTER TABLE; the Alembic ops page (`add_column` notes); Squawk's rule
index, its `require-concurrent-index-creation`, `adding-foreign-key-constraint`, `changing-column-type` and
`require-timeout-settings` pages, and its web-frameworks page (the Alembic usage); sqlalchemy/alembic#679; and the
PandaDoc `migration-lint` README. The Squawk-Alembic packaging projects (`alembic-squawk`, `alembic-squawk-action`) were
seen in search results.

## Applied

The fixes follow the review's findings, generalised to their root causes, most of them in the library
(`rules/lib/alembic.rerail`), so every rule that leaned on the same heuristic got the same fix.

**Library.**

- Types are resolved, not spelled (root cause of 2.6's same-type false positives, 2.3's `BigInt` alias and 2.10's
  type families): `al_type_class`/`al_pg_type` follow imports, module aliases (of the same file or imported),
  TypeDecorator `impl`, SQLAlchemy subclasses and functions that return a type (`Text().with_variant(...)`), and map the
  class to the PostgreSQL type it renders. Used by column-type-rewrite, not-null-without-server-default and
  rename-as-drop-add.
- Flags are read by value (2.2's `CONCURRENT = True`, 2.5's `PG_NOT_VALID`): `al_bool`, `al_true_kw`, `al_false_kw`,
  `al_kw_on` follow module constants of the same file or imported. Used by every rule that reads a keyword flag.
- Dialect tests are read by structure (3.5's `al_non_pg` substring reading): comparisons with a database name literal,
  `in` sets, `isinstance` on SQLAlchemy dialect classes resolved through imports, `not`/`and`/`or`, test functions of
  the run (through imports, with a docstring), and names assigned a test. `if not settings.USE_SQLITE_FOR_TESTS:` and
  `if "postgres" in url and not readonly:` no longer read as other-database branches.
- What `add_column` emits (3.2): `al_added_ddl` reports the ADD UNIQUE, CREATE INDEX, ADD FOREIGN KEY and ADD CHECK
  Alembic 1.20 renders after the ADD COLUMN, with the constraint classes resolved through imports (a project class
  named `ForeignKey` is not one) and `inline_references=True` honoured.
- Helpers in another module (3.5): `al_forwarded` follows a call into a function of the run, through its import, that
  runs an operation on the table its parameter names (superset's `create_index`, one call deep).
- Existing tables (3.5's `al_on_existing`): a table the step creates with `op.execute("CREATE TABLE ...")`, or empties
  first (DELETE without WHERE, TRUNCATE), no longer counts as holding rows.
- A Column bound to a name before `add_column` is read (`al_added_column`).
- env.py and the module it imports its engine from (`al_env_reads`), for AUTOCOMMIT isolation and lock_timeout.
**Rules.**

- `index-not-concurrent` (2.1): reports `add_column(Column(..., unique=True))` and `index=True` (al_added_ddl),
  `create_primary_key` with the USING INDEX fix, and calls of project helpers that forward the table to
  `op.create_index` (superset's shared `create_index`, through its import); groups findings per revision, table and
  kind, with a count in the message; four messages of at most 200 characters. Cases add aliases (`op as alembic_op`,
  `Column as Col`, sqlmodel's Column), constants of this and another module, a re-imported helper, and the dialect-test
  class (c9). Steer, on.
- `concurrent-index-outside-autocommit` (2.2): `postgresql_concurrently` and `transactional_ddl` read by value
  (constants, imported too); an env.py whose engine runs with `isolation_level="AUTOCOMMIT"`, in env.py or the module it
  imports the engine from, counts as no transaction. Description without code; message 196 characters. Steer, on.
- `not-null-without-server-default` (2.3): the message names `server_default=` first and the nullable route only with
  NOT NULL in a later revision; the SERIAL test resolves the type (`BigInt = sa.BigInteger`, `Integer as Int`); a
  Column bound to a name and a table emptied first are read. Steer, on.
- `constraint-without-not-valid` (2.5): reports `add_column(Column(..., ForeignKey(...)))` (unless
  `inline_references=True`) and an inline `CheckConstraint`, with the classes resolved (a project class named
  `ForeignKey` is not one); an FK on a column the same upgrade adds gets the `inline_references=True` message;
  `**kwargs` splats and constants count as not known to be off; a table made by `op.execute("CREATE TABLE ...")` is
  new. Description aligned with the message (validate once the add has committed). Steer, on; the README states the
  62% actionable precision.
- `column-type-rewrite` (2.6): types compared by the PostgreSQL type their resolved class renders, so TIMESTAMP and
  DateTime, UUID and Uuid, INTEGER and Integer, BOOLEAN and Boolean, VARCHAR and AutoString, TEXT and Text are one;
  project TypeDecorators, subclasses, aliases and type-returning functions resolve too. Steer, on.
- `set-not-null-scan` (2.7): a VALIDATE exempts a column only when the validated constraint is shown, anywhere in the
  run, to be a NOT NULL check of that column (the name heuristic is gone); description and messages rewritten. Log, on.
- `rename-as-drop-add` (2.10): data movement is an UPDATE/INSERT in SQL text (literal or module constant), SQLAlchemy's
  `update()`/`insert()` resolved through imports, a table object's `.update()`/`.insert()`, `op.bulk_insert`, or an
  ORM session resolved to sqlalchemy.orm; `SET LOCAL lock_timeout`, `dict.update` and a project's own `insert()` move
  nothing; a widened length still pairs, a shrunk one does not; types through the library. Log, off.
- `model-import-in-migration` (2.11): Flask-SQLAlchemy `db.Model` subclasses (for `db = SQLAlchemy()`, resolved) are
  models; a model used only for `__tablename__` or `__table__.name` gets its own message; description no longer
  absolute; messages under 200 characters. Steer, on.
- `empty-downgrade` (2.12): a bare `return` is filler; a TODO, FIXME or XXX comment excuses nothing; message 199
  characters. Log, on.
- `drop-mapped-column` (2.4): a migration sees only its own tree's models: the modules env.py's imports reach
  (transitive, star and relative imports, package `__init__`s, the other modules of each imported package), else
  env.py's top-level directory; `__mapper_args__` `exclude_properties`/`include_properties` are honoured (dict or
  `dict()`, strings or Columns, inherited from bases); `__table__ = Table(...)` classes link to their Table (inline,
  same file or imported); description and four messages state the check the rule makes (no deploy-order promise,
  under 200 characters). The review's two counterexamples give 0 findings (was 2); corpus 0, as before; edit-window
  copies as expected. The Flask-SQLAlchemy and "attribute removed in the same change" gaps stay documented. Steer, on.
- `missing-lock-timeout` (2.8): judged per tree: one finding at env.py's `context.configure(connection=...)` unless more
  than half the tree's locking revisions set lock_timeout, where the per-revision slips stay; a setting is parsed as SQL
  `SET`/`ALTER ... SET`/`set_config`, a libpq `-c` option or an asyncpg `server_settings` key, with zero in any unit and
  DEFAULT read as off, so a log call, a comment or a redis `lock_timeout` sets nothing; env.py's engine module counts
  (`al_env_reads`, now also a module imported whole). Corpus 579 to 11 matches, all read: code 11/11, actionable 8/11.
  Log, on.
- `multiple-heads` (2.9): the `depends_on` exemption is gone (only `branch_labels` on a head's own line marks an intentional branch); two heads in one version directory pair even with no common ancestor (a second base); `revision` and `down_revision` bound to a constant are resolved; a tuple or list of one is one parent and gets the rebase message; a line whose parent is unknown (imported name, absent id) is left alone so the wider pairing cannot invent a second base. Messages 196 and 182 characters, description without code. The review's trees a to d are reported as expected; tree e binds `PARENT` after its use (a NameError at import) and stays silent, while a constant bound before its use is reported. Corpus 0, as before. Steer, on.

**Pack level.**

- 3.1: the README gives code and actionable precision for every rule, and says up front that the five lock rules
  assume migrations run against a live database and should be turned off together where the app is stopped to
  migrate. No pack-level switch exists in the pack format, so none was added.
- 3.2: all four shapes are owned (index-not-concurrent: `unique=True`, `index=True`, `create_primary_key`;
  constraint-without-not-valid: an inline `ForeignKey`); NOTES-alembic.md's "Alembic ignores both" corrected.
- 3.5: `al_on_existing` now skips raw `CREATE TABLE` and emptied tables; `al_non_pg` is structural; helpers in another
  module are followed one call deep; `al_env_doc` reads `import alembic.context`; the dead `al_sets_lock_timeout` and
  `al_model_column` are removed. Tree-scoped models live in drop-mapped-column, the only rule that needs them.
  Not done: a project-level dialect (`alembic.ini` URL, `render_as_batch`), because rules read Python files only; it is
  a documented gap of the lock rules.
- 3.6: every description is one sentence without code, at most 200 characters; every message is at most 200
  characters.
- 3.7: README and NOTES state the real advantages over Squawk on offline SQL.
- 3.3 (missing rules: DROP INDEX without CONCURRENTLY, a volatile `server_default`, a shared reading of `op.execute`
  SQL) is out of this fix's scope and is not built.

**Re-measured on the 8 repositories** (runs `alembic-fix-*`; the README has the per-repository tables):

| Rule | Matches before | After | Code precision (read) | Actionable (read) | Mode, on |
|---|--:|--:|---|---|---|
| index-not-concurrent | 296 | 242 | 99 of 99 | 83 of 99 (84%) | steer, on |
| concurrent-index-outside-autocommit | 0 | 0 | n/a | n/a | steer, on |
| not-null-without-server-default | 13 | 13 | 13 of 13 | 13 of 13 deploying main | steer, on |
| drop-mapped-column | 0 | 0 | n/a | n/a | steer, on |
| constraint-without-not-valid | 221 | 223 | 67 of 67 | 42 of 67 (63%) | steer, on |
| column-type-rewrite | 34 | 34 | 34 of 34 | 32 of 34 | steer, on |
| set-not-null-scan | 77 | 77 | 53 of 56 | 50 of 56 | log, on |
| missing-lock-timeout | 579 | 11 | 11 of 11 | 8 of 11 | log, on |
| multiple-heads | 0 | 0 | n/a | n/a | steer, on |
| rename-as-drop-add | 2 | 2 | 1 of 2 | 1 of 2 | log, off |
| model-import-in-migration | 13 | 13 | 13 of 13 | 13 of 13 | steer, on |
| empty-downgrade | 3 | 3 | 3 of 3 | 3 of 3 | log, on |
