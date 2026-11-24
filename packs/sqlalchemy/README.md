# Rerail rule pack: SQLAlchemy

Rerail's own SQLAlchemy rules, from the SQLAlchemy 2.0 docs and migration guide, for safe sessions, transactions, queries and relationship loading.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/sqlalchemy`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `sqlalchemy-practice-bulk-write-skips-hooks` | Steer | SQLAlchemy practice: a bulk UPDATE, DELETE or INSERT on a model whose mapper events, attribute events or validators it skips |
| `sqlalchemy-practice-connect-without-commit` | Log | SQLAlchemy practice: writes on a connection from engine.connect() are committed, or the connection comes from engine.begin() |
| `sqlalchemy-practice-lazy-load-in-loop` | Steer | SQLAlchemy practice: a loop over ORM rows that reads a lazily loaded relationship of every row |
| `sqlalchemy-practice-session-reused-after-failed-commit` | Steer | SQLAlchemy practice: an except handler swallows a failed commit() or flush() and the session is used again without rollback() |
| `sqlalchemy-practice-text-interpolation` | Log | SQLAlchemy practice: a text() SQL string built by interpolating values instead of binding them |

## Rule details

### `sqlalchemy-practice-bulk-write-skips-hooks`

Rerail's own rule, not a port. SQLAlchemy's ORM-enabled UPDATE, DELETE and INSERT statements, the legacy
`Query.update()`/`Query.delete()` and the `bulk_*` session methods write rows without the unit of work: no mapper
event (`before_`/`after_` `insert`, `update`, `delete`) fires, and neither do attribute `set` events nor
`@validates` methods. A model's audit rows, cache invalidation, timestamps and derived fields are then silently left
stale. It is the twin of `django-practice-bulk-write-skips-hooks`.

It matches, outside test code and Alembic migrations:

- `session.execute(update(M)…)`, `session.execute(delete(M)…)`, `session.execute(insert(M)…)` and
  `session.execute(insert(M), rows)`, with the statement written inline or bound to a name first
  (`stmt = update(M).where(...).values(...)`; `session.execute(stmt)`), on any receiver (`connection.execute(...)`
  skips the hooks the same way);
- `session.query(M)….update({...})`, `….delete()` and Flask-SQLAlchemy's `M.query.filter(...).delete()`;
- `session.bulk_insert_mappings(M, rows)`, `session.bulk_update_mappings(M, rows)`, and
  `session.bulk_save_objects([...])` when the objects are a list or list comprehension of `M(...)`;

when model M has a hook for that operation in any non-test file of the run:

- a mapper event for the operation (`before_update`/`after_update` for an update, and so on), registered with
  `@event.listens_for(M, "...")` or `event.listen(M, "...", fn)`; on a base class of M in the run when it passes
  `propagate=True`; or `event.listen(cls, ...)` inside `__declare_last__`/`__declare_first__` of M or a base of M (it
  runs once for every mapped subclass, as Netflix/dispatch's `TimeStampMixin` registers its `updated_at` bump);
- for an update or insert, a `set` event on `M.<col>` (`propagate=True` for a base's column) or a `@validates("<col>")`
  method of M or a base class (a mixin included), for a column the write sets: `.values(col=...)`,
  `.values({"col": ...})`, `.values({M.col: ...})`, the `Query.update({...})` dict. When the columns cannot be read
  (`insert(M), rows`, `.values(**d)`, a values dict held in a name, `bulk_*_mappings`, an upsert's
  `on_conflict_do_update`, `from_select`), every column counts. `bulk_save_objects` builds its objects with `M(...)`,
  so their validators and `set` events already ran; only its insert mapper events count.

The message names every skipped hook with its path and line, and says to do the hook's work at the write or to load
the objects and change them through the session, and to leave the write alone when skipping the hooks is deliberate.

It leaves alone: models with no hook for that operation (a validator or `set` event is no delete hook; a `delete`
listener is no update hook); a validator or `set` event on a column the write does not set, when the set columns can
be read; a listener on a base class without `propagate=True`; a write inside one of the model's own hooks (the
listener def, a method the registration names such as superset's
`sa.event.listen(SqlaTable, "before_update", SqlaTable.before_update)`, a validator), which writes in bulk on purpose;
writes whose operation a session-level `do_orm_execute` listener inspects (`is_update`, `is_delete`, `is_insert`)
when the listener names the model or names no model (it may run the hooks itself); tests; Alembic revisions; Core
tables (`update(table)`, `sa.table(...)`), which have no ORM hooks.

**Gaps:** models reached through a variable (`self.model` in generic repositories, `type(obj)`); relationship-based
dynamic queries (`obj.items.filter(...).delete()`); `event.listen(Mapper, ...)` or a listener registered on a variable
target; a `do_orm_execute` listener that delegates to helpers; a write whose values hold the hook's work already (polar
sets `status=expired` in bulk, which is what its `before_update` listener would have done) is still reported.

**Overlap:** none. Ruff, Bandit, pylint, flake8-sqlalchemy(2) and the Semgrep registry have no check that relates a
bulk write to the model's events.

**Corpus:** 13 matches in 3 of the 10 SQLAlchemy repositories, all read; all 13 hold the pattern (100%).

| Repository (commit) | Matches | Real |
|---|--:|--:|
| mealie-recipes/mealie (`8dbb928`) | 4 | 4 |
| apache/superset (`fdf6172`) | 7 | 7 |
| polarsource/polar (`872272f`) | 2 | 2 |
| Netflix/dispatch (`dd2837e`), freqtrade/freqtrade (`7fcb509`), jupyterhub/jupyterhub (`3ca3744`), open-webui/open-webui (`0666e59`), fastapi-users/fastapi-users (`e9c57ff`), fastapi/full-stack-fastapi-template (`77e550a`), langgenius/dify (`724ac15`) | 0 | – |

The seven silent repositories have no hook a bulk write of theirs skips: dispatch's 21 `Query.delete()` calls hit
models whose only hook is TimeStampMixin's `before_update`; freqtrade's, dify's and open-webui's validators sit on
columns no bulk write sets. Examples: mealie `mealie/repos/repository_factory.py`, `repository_foods.py` and
`repository_units.py` (`update(ShoppingListItem)` skips the after_update listener that bumps the shopping list's
`updated_at`) and `repository_recipes.py` (`delete(UserToRecipe)` skips the after_delete listener that recomputes the
recipe's rating); superset `superset/commands/theme/set_system_theme.py` (four `update(Theme)` calls skip the
cache-clearing after_update listener; in the two Clear commands no object is flushed afterwards, so the bootstrap
cache stays stale), `superset/views/sql_lab/views.py` (`Query.update(fields)` with request fields that include
`extra_json` skips `ExtraJSONMixin.ensure_extra_json_is_not_none`, so a JSON `null` is stored as NULL) and
`superset/commands/theme/delete.py` (dashboards' `theme_id` nulled in bulk skips the thumbnail listener). Five of the
13 are harmless where they stand though the pattern holds: polar's two `update(Checkout)` calls (they expire or
scrub checkouts the `before_update` expiry check would leave alone), superset's `SemanticLayer.clear_metadata_cache`
(a cache-version bump the permission-sync listener does not need) and two `SetSystem*Theme` commands that flush a
Theme right after, which clears the cache anyway.

**Sources:**
- SQLAlchemy docs, [ORM-Enabled INSERT, UPDATE, and DELETE statements: Important Notes and Caveats for ORM-Enabled
  Update and Delete](https://docs.sqlalchemy.org/en/20/orm/queryguide/dml.html#important-notes-and-caveats-for-orm-enabled-update-and-delete):
  "The ORM-enabled UPDATE and DELETE features bypass ORM unit of work automation in favor of being able to emit a
  single UPDATE or DELETE statement that matches multiple rows at once without complexity"; they do not run
  in-Python relationship cascades, and "In order to intercept ORM-enabled UPDATE and DELETE operations with event
  handlers, use the SessionEvents.do_orm_execute() event."
- SQLAlchemy docs, [ORM Events: Execute Events](https://docs.sqlalchemy.org/en/20/orm/session_events.html#execute-events):
  `do_orm_execute` is "a comprehensive hook designed to intercept all SELECT statements made on behalf of the ORM as
  well as bulk UPDATE and DELETE statements"; the mapper events (`before_update` and the rest) run inside the flush.
- SQLAlchemy GitHub issue [#9339 "before_update is not triggered for update statement"](https://github.com/sqlalchemy/sqlalchemy/issues/9339):
  the maintainer's answer: "before_update applies only to ORM flush operations. for all use at [the ORM DML page], the
  only event hook to use is do_orm_execute()".
- SQLAlchemy docs, [Simple Validators](https://docs.sqlalchemy.org/en/20/orm/mapped_attributes.html#simple-validators):
  "Validators, like all attribute extensions, are only called by normal userland code": they run when an attribute
  of an object is set, which a bulk statement never does.

### `sqlalchemy-practice-connect-without-commit`

Rerail's own rule, not a port. SQLAlchemy 2.0 removed library-level autocommit: a Core connection from
`engine.connect()` begins a transaction on its first statement and rolls it back when it is closed, unless the code
calls `conn.commit()` ("commit as you go") or opens the connection with `engine.begin()` ("begin once"). An INSERT,
UPDATE, DELETE or DDL statement run on such a connection and never committed is lost without an error: the code
looks right, the statement runs, and nothing is stored.

It matches a connection the function opens itself: `with E.connect() as conn:`, `async with E.connect() as conn:`,
`conn = E.connect()` or `conn = await E.connect()` (connect() with no arguments, maybe chained with
`.execution_options(...)`, in a file importing SQLAlchemy). It reports each write on it:

- `conn.execute(s)` where `s` is `insert(...)`, `update(...)` or `delete(...)` (sqlalchemy, sqlmodel and dialect
  inserts such as `postgresql.insert`, resolved through imports), Core's `table.insert()`/`.update()`/`.delete()`, a
  DDL element (`CreateTable`, `DropIndex`, `CreateSchema`, ...), a chain from any of them (`.values()`, `.where()`,
  `.on_conflict_do_update()`), or a name the function binds to one;
- `conn.execute(text(sql))` and `conn.exec_driver_sql(sql)` where `sql` is a string literal (an f-string,
  `"…" % x`, `"…".format(...)`, or a name bound to one) whose statement starts with INSERT, UPDATE, DELETE, MERGE,
  UPSERT, REPLACE, CREATE, ALTER, DROP, TRUNCATE, GRANT or REVOKE;
- `metadata.create_all(conn)` and `metadata.drop_all(bind=conn)`;
- a call that hands the connection to a function of the run (resolved through the call graph, in any file) that makes
  such a write on that parameter and neither commits nor begins on any parameter: reported at the call, since the
  helper rightly leaves the transaction to its caller and the caller never commits.

It leaves alone:

- a write followed, anywhere later in the with block or function, by `conn.commit()` (or `conn.connection.commit()`),
  even in only one branch (precision first);
- a connection the block or function opens a transaction on: `with E.connect() as conn, conn.begin():` (dify's relyt
  and TiDB vector stores), `with conn.begin():`, `trans = conn.begin()`, `begin_nested()`;
- autocommit isolation on the connection (`conn.execution_options(isolation_level="AUTOCOMMIT")`,
  `E.connect().execution_options(isolation_level=...)`, a DBAPI `.autocommit = True`) or on the engine: a local name,
  module variable, imported module variable, or `self.<attr>` the same class assigns, whose value is
  `create_engine(..., isolation_level="AUTOCOMMIT")`, `….execution_options(isolation_level="AUTOCOMMIT")`,
  `execution_options={"isolation_level": ...}` or `connect_args={"isolation_level": None}`. An isolation level that is
  not a literal counts as autocommit;
- CREATE, ALTER and DROP text on an engine made from a `sqlite…` URL literal (pysqlite runs DDL outside a transaction,
  so it persists; DML there is still reported);
- a connection passed to a function of the run that commits or begins on one of its parameters;
- a statement name rebound before the call: a name reads its last assignment in the function, so superset's metadb
  `insert_row` (`query = table.insert()…` run in `engine.begin()`, then `query = select(func.count())…` on
  `engine.connect()`) is a read;
- `engine.begin()`, reads (`select(...)`, `text("SELECT …")`), connections the function receives as a parameter (the
  caller owns the transaction), sessions, `sqlite3.connect(path)`, test files and Alembic revisions (`op.get_bind()`
  runs inside the migration's transaction).

**Gaps:** the rule assumes SQLAlchemy 2.0 (or 1.4 with `future=True`). A 1.x engine autocommitted DML, so in a project
pinned below 2.0 the rule's premise fails, which it cannot see. SQL it cannot read is not a write: a string built in
another function, a file's contents (`text(open(path).read())`), `str(ddl_object)`; so dispatch's `init_database`,
which hands an `engine.connect()` connection to `setup_fulltext_search` (DDL from a `.sql` file and
sqlalchemy-searchable's trigger DDL via `exec_driver_sql(str(sql))`) and never commits, is not reported. A connection
stored on `self` or returned is not followed; an engine made autocommit where the rule cannot see it
(Flask-SQLAlchemy's `SQLALCHEMY_ENGINE_OPTIONS`, a `connect` event listener, an engine passed in as a parameter) is
treated as transactional; a helper is followed one call deep.

**Overlap:** none. Ruff, Bandit, pylint and Semgrep's registry have no transaction checks; the python pack has nothing
on SQLAlchemy connections.

**Corpus:** 0 matches in 13 repositories, all read silent by the final rule:
full-stack-fastapi-template (77e550a7), fastapi-users (e9c57ff7), jupyterhub (3ca37442), freqtrade (7fcb509a),
open-webui (0666e59d), mealie (8dbb9280), dispatch (dd2837e8), polar (872272fb), superset (fdf61721), dify
(724ac151), flaskbb (63afaa41), MemoLite (e5621191) and reactpy-table (3c597011). The sites it reads and leaves alone
are the safe idioms: dify's relyt and TiDB vector stores (`with self.client.connect() as conn, conn.begin():`
around inserts and deletes), polar's `_grant_read_user_permissions` (GRANT/ALTER DEFAULT PRIVILEGES, then
`conn.commit()`), flaskbb's `drop_all` (DropTable and `drop_all`, then `connection.commit()`), superset's version
retention prune (`conn.begin()`), and read-only `engine.connect()` blocks everywhere. Two shapes found during the runs
were false and are now cases: reactpy-table's `CREATE VIRTUAL TABLE` on a `create_engine(url="sqlite:///…")` engine
(pysqlite persists DDL without a commit) and superset's metadb `insert_row`, whose `query` is rebound from an insert to
a `select(func.count())` before the `engine.connect()` block. The one real lost write seen, dispatch's
`init_database` handing an `engine.connect()` connection to `setup_fulltext_search` on SQLAlchemy 2.0.8, writes SQL
read from a file and built by objects, which the rule cannot read (Gaps). The rule ships in `log`: the standard states
the hazard outright, but zero corpus matches give no measured precision.

**Sources:**
- SQLAlchemy 2.0 migration guide, [Library-level (but not driver level) "Autocommit" removed from both Core and
  ORM](https://docs.sqlalchemy.org/en/20/changelog/migration_20.html#library-level-but-not-driver-level-autocommit-removed-from-both-core-and-orm):
  "SQLAlchemy 2.0 will do away with all attempts to implicitly commit transactions"; DML on a connection now needs
  `conn.commit()` or `engine.begin()`.
- SQLAlchemy Unified Tutorial, [Working with Transactions and the
  DBAPI](https://docs.sqlalchemy.org/en/20/tutorial/dbapi_transactions.html): "when the connection is released, a
  ROLLBACK is emitted to end the transaction. The transaction is not committed automatically; if we want to commit
  data we need to call Connection.commit()"; the "commit as you go" and "begin once" styles.
- SQLAlchemy Core, [Setting Transaction Isolation Levels including DBAPI
  Autocommit](https://docs.sqlalchemy.org/en/20/core/connections.html#setting-transaction-isolation-levels-including-dbapi-autocommit):
  the `isolation_level="AUTOCOMMIT"` forms the rule accepts.
- SQLAlchemy SQLite dialect, [Serializable isolation / Savepoints / Transactional
  DDL](https://docs.sqlalchemy.org/en/20/dialects/sqlite.html#serializable-isolation-savepoints-transactional-ddl):
  pysqlite does not begin a transaction before DDL, the reason SQLite DDL is left alone.

### `sqlalchemy-practice-lazy-load-in-loop`

Rerail's own rule, not a port; the SQLAlchemy twin of `django-practice-select-related-in-loop`. A relationship whose
loader is `lazy="select"` (the default) emits one SELECT the first time it is read on each instance, so a loop over N
rows that reads it runs N+1 queries. A loader option on the statement (`selectinload(M.rel)`, or `joinedload` for a
many-to-one) loads them all with the rows.

It matches a loop (a `for` statement or a comprehension) over ORM rows of model M that reads `x.rel` of each row, where
`rel` is a relationship of M with the lazy "select" strategy: declared on M in another file, on a base class of the
run, or given to M by another model's `backref=`. The rows come from:

- a statement in the same function: `for o in session.scalars(select(M)…)`, `session.execute(select(M)).scalars()`,
  `orders = session.query(M).filter(...).all(); for o in orders`, `(await s.scalars(select(M))).all()`, the same over
  a statement held in a name (`stmt = select(M).where(...); session.scalars(stmt)`), and a legacy query over `cls` in
  a classmethod of M (`db.session.query(cls)`, `cls.query`);
- a function of the run the loop calls, one hop, resolved through imports or the call graph, that returns such rows
  or the legacy query itself: `for incident in incident_service.get_all(db_session=db_session)` where `get_all`
  returns `db_session.query(Incident).all()`. The loader check is made on the statement inside that function.

A read is `x.rel` evaluated in the loop's own per-row code (not in a nested def, lambda or class, and not in the
comprehension's iterable). One finding per loop and relationship, at its first read.

It leaves alone: loader options naming the relationship anywhere in a chain (`selectinload(M.a).joinedload(A.b)` for
`x.a`, `Load(M).joinedload(M.x)`, `joinedload("x")`), a wildcard loader (`selectinload("*")`, `raiseload("*")`), and an
`.options(...)` naming it on another statement of the function that builds the rows (a query assembled over several
assignments); relationships whose `lazy=` is anything but "select" (`"selectin"`, `"joined"`, `"raise"`, `"dynamic"`,
...); loops that read only columns or the key column (`x.rel_id`), and `x.rel = v`; single rows (`.first()`,
`.one()`, `session.get()`); loops over a parameter or another value whose statement is not visible;
`yield_per` iterations (`.yield_per(n)`, `execution_options(yield_per=n)`), which are written to stream and restrict
eager loading of collections; tests, Alembic revisions and `env.py` (a file importing `op` or `context` from alembic,
or under `migrations/versions`; a CLI that imports `alembic.command` is checked).

**Gaps:** a many-to-one whose target is already in the session's identity map emits no SELECT, so when most rows share
a few parents the N+1 is small: 4 of the 22 corpus matches are such (dispatch `incident.incident_priority`,
`tag.tag_type`, `project.organization`; superset `datasource.database`), one SELECT per distinct parent rather than
per row. Loop targets that unpack (`for i, o in enumerate(rows)`), callees more than one call away, generic
repositories (`self.model`, mealie's repository layer), relationships read inside a function the loop calls (jupyterhub
passes each user to `authenticator.add_user(user)`), and models of other packages (flask_appbuilder's
`PermissionView`, so superset's `pvm.view_menu` loop) are not followed; an `.options(...)` naming the relationship on
another statement of the same function is taken as covering the loop; a loop that stops after the first row or two is
still reported.

**Overlap:** none. Ruff, pylint, Bandit and flake8-sqlalchemy(2) have no SQLAlchemy loading checks; Semgrep's registry
has only `len-all-count` and `batch-import`. The relationship's strategy sits on the model, usually in another file.

**Corpus:** 22 matches on the 10 SQLAlchemy repositories, all read, 22 real (100%):

| Repository | Commit | Matches | Real |
|---|---|---|---|
| Netflix/dispatch | `dd2837e8` | 19 | 19 |
| mealie-recipes/mealie | `8dbb9280` | 2 | 2 |
| apache/superset | `fdf61721` | 1 | 1 |
| fastapi/full-stack-fastapi-template, fastapi-users/fastapi-users, jupyterhub/jupyterhub, freqtrade/freqtrade, open-webui/open-webui, langgenius/dify, polarsource/polar | (MANIFEST.tsv) | 0 | — |

Real matches include dispatch `src/dispatch/incident_role/service.py:193` (`p.incident_priorities` for every
incident role a service returns), `src/dispatch/feedback/service/scheduled.py:129` (`incident.participants` over
`get_all_last_x_hours`), `src/dispatch/notification/service.py:142`, `src/dispatch/entity_type/service.py:98`,
`src/dispatch/cli.py:924`, and mealie `mealie/db/fixes/fix_migration_data.py:86` (`recipe.recipe_ingredient` for every
recipe). freqtrade and polar declare `lazy="selectin"`/`"raise"` (polar: on nearly every relationship) on what their
loops read, jupyterhub reads relationships in the functions its loops call, and dify and open-webui declare few
relationships, which is why they have none.

**Sources:**
- SQLAlchemy 2.0 docs, [Relationship Loading Techniques](https://docs.sqlalchemy.org/en/20/orm/queryguide/relationships.html):
  lazy loading has "the N plus one problem ... for any N objects loaded, accessing their lazy-loaded attributes means
  there will be N+1 SELECT statements emitted"; `selectinload()` is the loader to use for collections, `joinedload()`
  for many-to-one, and `raiseload` / `lazy="raise"` prevent unwanted lazy loads.
- SQLAlchemy 2.0 docs, [ORM Querying Guide: Fetching Large Result Sets with Yield Per](https://docs.sqlalchemy.org/en/20/orm/queryguide/api.html#fetching-large-result-sets-with-yield-per):
  the yield_per option "is not compatible with “subquery” eager loading loading or “joined” eager loading when using
  collections. It is potentially compatible with “select in” eager loading, provided the database driver supports
  multiple, independent cursors."
- SQLAlchemy FAQ, [Performance: Query Profiling](https://docs.sqlalchemy.org/en/20/faq/performance.html#query-profiling):
  SQL logging shows "if dozens/hundreds of queries are being issued which could be better organized into much fewer
  queries"; the ORM's "eager loading" feature automates this.

### `sqlalchemy-practice-session-reused-after-failed-commit`

Rerail's own rule, not a port. When `Session.flush()` (or the flush inside `commit()`) fails, SQLAlchemy rolls the
database transaction back but leaves the session's transaction "inactive": until the code calls `rollback()` or
`close()`, every later use of that session raises PendingRollbackError ("This Session's transaction has been rolled
back due to a previous exception during flush"). An except handler that swallows the error and lets the session live
on turns one failed write into a run of failures: every remaining loop iteration, the rest of the request, or the
caller's next query.

It matches an `except` clause of a `try` whose body calls `s.commit()` or `s.flush()` (awaited or not) on a session
`s` (lib/sqlalchemy's `sa_session_like`: a session-typed parameter or variable, a name bound to a session factory's
result, `db.session`, or a name spelled like a session in a file importing SQLAlchemy), where the clause catches the
error (bare `except:`, `Exception`, `BaseException`, or SQLAlchemy's `SQLAlchemyError`, `DBAPIError`,
`IntegrityError`, `OperationalError`, `DataError`, ... from `sqlalchemy.exc` or `sqlalchemy.orm.exc`, resolved through
imports), neither re-raises on every path nor calls `rollback()` (any `….rollback()`, or a helper named for it) or
`s.close()`/`invalidate()`/`reset()`/`remove()`, and the session lives on, the first of:

- the handler itself calls a method on `s` (dify's tasks write an error status with `session.execute(update(...))`
  and `session.commit()` in the handler);
- the try sits in a `for`/`while` body, `s` is bound outside that loop, and the handler falls through or `continue`s,
  so the next iteration commits on the failed session (open-webui's `create_groups_by_group_names`);
- a statement after the try, in the same function and on a path from the handler (after the loop, for a handler that
  `break`s or `continue`s), calls a method on `s`;
- a caller passes the session in and goes on using it (cross-file, through python/calls' call graph): `s` is a
  parameter the function never rebinds, and some call of the function passes it a name, by keyword or position, from
  a loop that name is bound outside of, or is followed in the caller by a statement that mentions the name
  (dispatch's weekly report calls `ai_service.generate_incident_summary(db_session=db_session, ...)` once per
  incident);
- the session is shared beyond the function: `<module>.session` (Flask-SQLAlchemy's request- and app-scoped
  `db.session`) or a module-level name (a scoped_session); not in a CLI command (`@click.command`,
  `@app.cli.command`), a web route (`@router.post`, `@app.route`) or a Slack Bolt listener (`@app.action`), whose
  session is closed or removed when it returns.

The message names the reason. One finding per handler and session, at the `except` line.

It leaves alone: a handler that rolls back, closes or re-raises (through an if/else, or a last call to a def of the
same file that always ends in `raise`, such as dify's `self._handle_integrity_error(e, ...)`); a commit inside a with
block in the try body that binds the session (`with Session() as s:` / `async with maker() as s:` closes it, which
rolls back, before the handler runs: polar's event mover); a commit or flush inside a `with s.begin():` or
`begin_nested():` block in the try body (the block's exit rolls back: superset's permalink savepoint, open-webui's
reactions); a commit in an inner try of the body (dispatch's per-instance loop rolls back in its own handler); a
session the function creates that is neither reused in a loop nor used after the handler; a parameter no caller is
found to reuse (dify's `_mark_run_failed`, whose callers leave the `with` that closes the session; dify's
`RagPipelineDslService`, whose controller rolls back when the import returns FAILED); `self.<session>` attributes,
whose owner's callers are not followed (mealie's OIDC provider returns None and its route answers 401); handlers that
catch other errors (`ValueError`, Django's `IntegrityError`); test files and Alembic revisions.

A name rebound by `with helper(param) as db:` is the function's own session, not the caller's: open-webui's
`get_async_db_context(db)` hands back the caller's session only when `DATABASE_ENABLE_SESSION_SHARING` is on (off by
default) and otherwise opens a fresh session that the `async with` closes. Its ~90 `except Exception: return None`
handlers around `await db.commit()` stay silent; the loop in `create_groups_by_group_names` is reported (the loop
reason does not depend on who owns the session).

**Gaps:** a session whose name does not look like one (`self.db`, an unannotated `db` parameter); a later use through
a helper the session is passed to (`flows.do_x(db_session=db_session)`) rather than a method call on it, in the
function itself; a rollback done by a helper whose name does not say "rollback"; a `self.<session>` whose owner's
callers do reuse it; callers more than one call away, or reached only through calls python/calls leaves open; a
session stored in a container or passed through `**kwargs`.

**Overlap:** Bandit B110/B112 (`except: pass` / `except: continue`, in the python pack) and Ruff's BLE001/S110 flag
the swallowing on any try, whatever it guards; pylint W0718 flags the broad catch. None of them knows the try commits
a session, that the handler leaves it un-rolled-back, or that the session is used again; this rule fires on
`except IntegrityError: log(...)` too, which none of them flags.

**Corpus:** run with `corpus-run` over the pinned commits of `MANIFEST.tsv`; every match read. Whole repositories,
except superset and dify: their whole-repository runs were killed at the engine's 15 minutes on a loaded machine, so
they were run over the directories that hold their SQLAlchemy code.

| Repository (commit) | Run over | Matches | Real | Precision |
|---|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template (`77e550a`) | whole, 43 files | 0 | 0 | - |
| fastapi-users/fastapi-users (`e9c57ff`) | whole, 88 files | 0 | 0 | - |
| jupyterhub/jupyterhub (`3ca3744`) | whole, 158 files | 0 | 0 | - |
| freqtrade/freqtrade (`7fcb509`) | whole, 484 files | 0 | 0 | - |
| mealie-recipes/mealie (`8dbb928`) | whole, 723 files | 0 | 0 | - |
| Netflix/dispatch (`dd2837e`) | whole, 717 files | 2 | 2 | 100% |
| open-webui/open-webui (`0666e59`) | whole, 330 files | 1 | 1 | 100% |
| apache/superset (`fdf6172`) | `superset/{db_engine_specs,commands,views,utils,tasks,daos,models,sqllab,mcp_service}`, 798 files | 2 | 2 | 100% |
| langgenius/dify (`724ac15`) | `api/{tasks,schedule,commands,events,services,controllers,core}`, 1,862 files | 11 | 11 | 100% |
| **Total** | | **16** | **16** | **100%** |

Real matches, by file: open-webui `backend/open_webui/models/groups.py:654` (a failed group insert is logged and
`continue`d; every later group of the loop commits on the failed session); dispatch
`src/dispatch/case_cost/scheduled.py:81` (one case's failed commit fails every later case of the scheduled run) and
`src/dispatch/ai/service.py:444` (the weekly report in `incident/scheduled.py` calls it once per incident with the
same `db_session`); dify `api/tasks/add_document_to_index_task.py:144`, `create_segment_to_index_task.py:98`,
`deal_dataset_index_update_task.py:98` and `:199`, `deal_dataset_vector_index_task.py:94` and `:195`,
`remove_document_from_index_task.py:98`, `regenerate_summary_index_task.py:158` (each handler writes an error status
with the failed session, so after a failed commit the status write raises PendingRollbackError and the document stays
"indexing"), `regenerate_summary_index_task.py:271` (the next document of the loop), `api/commands/system.py:130` (a
CLI loop over apps on `db.session`), `api/schedule/create_tidb_serverless_task.py:66` (`db.session`; its caller calls
it again in a loop); superset `superset/db_engine_specs/clickhouse.py:878` and `impala.py:629` (a failed
`db.session` flush/commit during query execution is swallowed, and SQL Lab goes on to save the query's state on the
same scoped session).

Matches of earlier drafts (or of an `ast` probe of the same shapes) that the final rule leaves alone, each by a
matcher change, each now a case: mealie `core/security/providers/openid_provider.py:135` and dify
`services/rag_pipeline/rag_pipeline_dsl_service.py:364`, `:511` (`self.<session>`; mealie's route answers 401 and
dify's controller rolls back on FAILED); dify `tasks/evaluation_task.py:1991` (a parameter whose only callers do not
reuse the session after the call: the call is followed by `return`, or ends the `with` that closes it); dify
`commands/plugin.py:385` (a click command); dispatch `plugins/dispatch_slack/incident/interactive.py:3150` (a Slack
Bolt listener whose middleware closes the session); dify `services/tools/mcp_tools_manage_service.py:329` (probe only:
the handler's last call always raises). open-webui's ~90 `except Exception: return None` handlers around
`await db.commit()` in `get_async_db_context(db)` blocks never matched (see above). polarsource/polar was not run (2,522
files, over the engine's 15 minutes for its peers); the `ast` probe found its only swallowing handlers around a commit
catch polar's own exceptions (`UnsupportedTarget`, `SkipEvent`, `_SavepointRollback`), which the rule leaves alone.

**Sources:**
- SQLAlchemy FAQ, ["This Session's transaction has been rolled back due to a previous exception during flush." (or
  similar)](https://docs.sqlalchemy.org/en/20/faq/sessions.html#this-session-s-transaction-has-been-rolled-back-due-to-a-previous-exception-during-flush-or-similar):
  the error "occurs when a Session.flush() raises an exception, rolls back the transaction, but further commands upon
  the Session are called without an explicit call to Session.rollback() or Session.close()"; it "usually corresponds
  to an application that catches an exception upon Session.flush() or Session.commit() and does not properly handle
  the exception"; its example is `except: pass` followed by another `commit()`.
- SQLAlchemy error reference, [This Session's transaction has been rolled back due to a previous exception during
  flush](https://docs.sqlalchemy.org/en/20/errors.html#this-session-s-transaction-has-been-rolled-back-due-to-a-previous-exception-during-flush):
  after a failed flush "the session's transaction is now “inactive” and must be explicitly rolled back by the calling
  application".
- SQLAlchemy, [Session Basics: Framing out a begin / commit / rollback
  block](https://docs.sqlalchemy.org/en/20/orm/session_basics.html#framing-out-a-begin-commit-rollback-block): the
  `try: ... commit() except: rollback(); raise` shape.

### `sqlalchemy-practice-text-interpolation`

Rerail's own rule, not a port. `text()` is SQLAlchemy's way into hand-written SQL, and everything in its string goes to
the database as SQL. A value spliced into that string with an f-string, `%`, `.format()`, `.replace()`, `.join()` or
`+` is SQL injection when the value can come from outside; values belong in bound parameters (`:name` in the SQL, with
`.bindparams(name=...)` or the execute parameters), which the driver sends apart from the SQL. Identifiers (table and
column names) cannot be bound: they are quoted with the dialect's identifier preparer or picked from a fixed
allow-list.

It matches SQLAlchemy's `text(...)` (resolved through imports: `from sqlalchemy import text`, `sa.text`, `text as
sql_text`, sqlmodel's re-export) and a connection's `exec_driver_sql(...)` whose SQL string is built from a value that
is not a constant. The SQL argument and each spliced value are followed to where they come from: names the scope binds
(`sql = f"…"; sql += f"…"; session.execute(text(sql))`, loop variables, `a, b = x, y` item by item), module variables
in the same or an imported module, `self.x`/`cls.x` assigned by the class or a base of the run, the return values of
the run's functions, and a parameter's default and the arguments the run's calls pass it. A value is not a constant
when a path ends in something else: a parameter no call of the run passes (a route's, a public helper's), an
attribute, a call outside the run, a subscript of a request. It reports the call and names the spliced expression:

- `session.execute(text(f"SELECT * FROM recipe WHERE name = '{name}'"))`
- `select(Recipe).order_by(text(f"{col} {direction}"))`, `select(M).where(sa.text(f"name = '{where}'"))`
- `text("… WHERE id = %s" % item_id)`, `text("… slug = '{}'".format(slug))`, `text("… WHERE " + clause)`
- `conn.exec_driver_sql(f"VACUUM ANALYZE {table}")`
- `text(f'SELECT count(*) FROM "{self._collection_name}"')` with the name set from a constructor parameter

It leaves alone constants: literals and values that end only in literals by the paths above (`direction = "DESC" if
desc else "ASC"`, `SORTS[key]` for a dict of literals, `match_sql("chat", key)` for a helper every caller passes a
literal, `cls._threshold` for `_threshold: ClassVar[float] = 0.5`), `int()`/`float()`/`len()`/`bool()` results,
parameters annotated `int`/`float`/`bool`/`Decimal`, `M.__tablename__`, and bind-parameter names (an interpolation right
after `:`, `f"… = :{param}"`, `", ".join(f":id_{i}" for i in range(n))`). It leaves alone vetted identifiers: a name
its scope binds to a quoting call (`table = preparer.quote(table)`, `quote_identifier(...)`) or tests against an
allow-list (`if col not in ("name", "created_at"): raise`, or a setting reset to a default when it is not in a tuple of
literals, in the module it is imported from). It also leaves alone a SQL string a function receives and runs unchanged
(`text(query)`: nothing is built there), other libraries' `text` (tagflow, htpy, a local `def text`), tests, Alembic
revisions and hand-written migration modules (`migrations.py`, a `migrations/` directory), whose DDL splices table,
index and sequence names read from the schema.

**Gaps:** a name quoted or allow-listed anywhere in its scope counts as vetted at every read in that scope; a helper
whose callers the call graph resolves only in part is judged by those callers; an identifier read from the schema or
the model's metadata (`column.name`) is reported; SQL built in another function and passed in as a parameter is not
followed; `session.execute("…")` with a plain string (SQLAlchemy 1.x) is not a sink.

**Overlap:** Ruff S608 and Bandit B608 match SQL keywords (`SELECT … FROM`, `UPDATE … SET`, `INSERT INTO`, `DELETE
FROM`) in any formatted string. They miss fragments given to `text()` (`order_by(text(f"{col} {direction}"))`,
`where(text(f"name = '{x}'"))`, `SET search_path TO {schema}`), flag constants and strings that never reach a database,
and S608 is not ported in the `python` pack. Semgrep's `python.sqlalchemy.security.audit.avoid-sqlalchemy-text` flags
every non-literal `text()` argument, constants included. `python-security-sql-injection` reports request data reaching
SQL through the call graph; this rule reports the construct whatever the value's source, resolves `text` to SQLAlchemy
(polar's many `text(f"…")` calls are tagflow's and stay silent) and follows values far enough to leave constants alone.

**Corpus:** CORPUS_PLACEHOLDER

**Sources:**
- SQLAlchemy 2.0 Major Migration Guide, [execute() method more strict, execution options are more prominent](https://docs.sqlalchemy.org/en/20/changelog/migration_20.html#execute-method-more-strict-execution-options-are-more-prominent):
  "the string SQL use case is becoming more subject to scrutiny from a security perspective, and the text() construct
  has come to represent an explicit boundary into the textual SQL realm where attention to untrusted user input must be
  given."
- SQLAlchemy 2.0 docs, [`text()`](https://docs.sqlalchemy.org/en/20/core/sqlelement.html#sqlalchemy.sql.expression.text):
  "Bind parameters are specified by name, using the format :name", e.g. `text("SELECT * FROM users WHERE
  id=:user_id")` executed with `{"user_id": 12}`; `TextClause.bindparams()` attaches values and types to them.
- Semgrep rule [`python.sqlalchemy.security.audit.avoid-sqlalchemy-text`](https://semgrep.dev/r/python.sqlalchemy.security.audit.avoid-sqlalchemy-text)
  and Bandit [B608: hardcoded_sql_expressions](https://bandit.readthedocs.io/en/latest/plugins/b608_hardcoded_sql_expressions.html):
  both flag SQL built by string formatting.
