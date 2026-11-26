# Rerail rule pack: SQLAlchemy

Rerail's own SQLAlchemy rules, from the SQLAlchemy 2.0 docs and migration guide, for safe sessions, transactions, queries and relationship loading.

## Rules

In reading order: by the harm each prevents and the trust its corpus numbers earn.

| Rule | Mode | Starts | Catches |
|---|---|---|---|
| `sqlalchemy-practice-session-reused-after-failed-commit` | Steer | on | SQLAlchemy practice: an except handler swallows a failed commit() or flush() and the session is used again without rollback() |
| `sqlalchemy-practice-bulk-write-skips-hooks` | Log | off | SQLAlchemy practice: a bulk UPDATE, DELETE or INSERT on a model whose mapper events, attribute events or validators it skips |
| `sqlalchemy-practice-lazy-load-in-loop` | Steer | on | SQLAlchemy practice: a loop over ORM rows that reads a lazily loaded collection of every row |
| `sqlalchemy-practice-lazy-many-to-one-in-loop` | Log | off | SQLAlchemy practice: a loop over ORM rows that reads a lazily loaded many-to-one relationship of every row |
| `sqlalchemy-practice-connect-without-commit` | Log | on | SQLAlchemy practice: writes on a connection from engine.connect() are committed, or the connection comes from engine.begin() |

The corpus figures below were measured with `corpus-run` on the pinned commits of the benchmark's `MANIFEST.tsv`
(run names `sqlalchemy-fix-*`). superset was run over its `superset/` package and dify over `api/` without its tests,
migrations, providers and dev tools (whole-repository runs hit the engine's content-store limit); polar over
`server/polar` without `backoffice/`, `observability/` and `organization_review/`, which hold no model hook and no
bulk write of a hooked model.

## Rule details

### `sqlalchemy-practice-session-reused-after-failed-commit`

Rerail's own rule, not a port. When `Session.flush()` (or the flush inside `commit()`) fails, SQLAlchemy rolls the
database transaction back but leaves the session's transaction "inactive": until the code calls `rollback()` or
`close()`, every later use of that session raises PendingRollbackError ("This Session's transaction has been rolled
back due to a previous exception during flush"). An except handler that swallows the error and lets the session live
on turns one failed write into a run of failures: every remaining loop iteration, the rest of the request, or the
caller's next query. In a task queue the handler's own error-status write fails too, so the row stays "processing".

It matches an `except` clause of a `try` whose body calls `s.commit()` or `s.flush()` (awaited or not) on a session
`s`, where the clause catches the error (bare `except:`, `Exception`, `BaseException`, or SQLAlchemy's
`SQLAlchemyError`, `DBAPIError`, `IntegrityError`, `OperationalError`, `DataError`, ... from `sqlalchemy.exc` or
`sqlalchemy.orm.exc`, resolved through imports), neither re-raises on every path nor calls `rollback()` (any
`….rollback()`, or a helper named for it) or `s.close()`/`invalidate()`/`reset()`/`remove()`, and the session lives on,
the first of:

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

What a session is comes from lib/sqlalchemy's `sa_session_like`: a session-typed parameter or variable, a name bound to
a session factory's result, `db.session`, a name spelled like a session in a file importing SQLAlchemy, and, resolved
through the call graph, a parameter of any name that a caller of the run fills with a session (FastAPI's unannotated
`db`, called from a route whose `db: Session = Depends(get_db)`). The message names the reason. One finding per handler
and session, at the `except` line.

It leaves alone: a handler that rolls back, closes or re-raises (through an if/else, or a last call to a def of the
same file that always ends in `raise`, such as dify's `self._handle_integrity_error(e, ...)`); a commit inside a with
block in the try body that binds the session (`with Session() as s:` / `async with maker() as s:` closes it, which
rolls back, before the handler runs: polar's event mover); a commit or flush inside a `with s.begin():` or
`begin_nested():` block in the try body (the block's exit rolls back: superset's permalink savepoint, open-webui's
reactions); a commit in an inner try of the body (dispatch's per-instance loop rolls back in its own handler); a
session the function creates that is neither reused in a loop nor used after the handler; a parameter no caller is
found to reuse (dify's `_mark_run_failed`, whose callers leave the `with` that closes the session; dify's
`RagPipelineDslService`, whose controller rolls back when the import returns FAILED); a parameter whose callers pass
something other than a session; `self.<session>` attributes, whose owner's callers are not followed (mealie's OIDC
provider returns None and its route answers 401); handlers that catch other errors (`ValueError`, Django's
`IntegrityError`); test files and Alembic revisions (a CLI module that imports `alembic.command` is checked).

A name rebound by `with helper(param) as db:` is the function's own session, not the caller's: open-webui's
`get_async_db_context(db)` hands back the caller's session only when `DATABASE_ENABLE_SESSION_SHARING` is on (off by
default) and otherwise opens a fresh session that the `async with` closes. Its ~90 `except Exception: return None`
handlers around `await db.commit()` stay silent; the loop in `create_groups_by_group_names` is reported (the loop
reason does not depend on who owns the session).

**Gaps:** a session whose name does not look like one and that no caller of the run passes a known session
(`self.db`, a `db` parameter whose callers are themselves unannotated, two calls away); a later use through a helper
the session is passed to (`flows.do_x(db_session=db_session)`) rather than a method call on it, in the function
itself; a rollback done by a helper whose name does not say "rollback"; a `self.<session>` whose owner's callers do
reuse it; callers more than one call away, or reached only through calls python/calls leaves open; a session stored
in a container or passed through `**kwargs`.

**Overlap:** Bandit B110/B112 (`except: pass` / `except: continue`, in the python pack) and Ruff's BLE001/S110 flag
the swallowing on any try, whatever it guards; pylint W0718 flags the broad catch. None of them knows the try commits
a session, that the handler leaves it un-rolled-back, or that the session is used again; this rule fires on
`except IntegrityError: log(...)` too, which none of them flags. SQLAlchemy's own signal is the PendingRollbackError,
at run time, after the damage.

**Corpus:** every match read. Rows marked "not re-run" are the independent review's figures: their repository copies
hold 1,000 Python files or more, which the corpus runner no longer takes.

| Repository (commit) | Run | Matches | Real |
|---|---|--:|--:|
| open-webui/open-webui (`0666e59`) | this version | 1 | 1 |
| mealie-recipes/mealie (`8dbb928`), fastapi-users/fastapi-users (`e9c57ff`), fastapi/full-stack-fastapi-template (`77e550a`), jupyterhub/jupyterhub (`3ca3744`) | this version | 0 | - |
| Netflix/dispatch (`dd2837e`) | this version | 2 | 2 |
| langgenius/dify (`724ac15`, `api/`) | not re-run | 11 | 11 |
| apache/superset (`fdf6172`, `superset/`) | not re-run | 2 | 2 |
| **Total** | | **16** | **16 (100%)** |

Real matches: open-webui `backend/open_webui/models/groups.py:654` (a failed group insert is logged and the loop goes
on committing on the failed session); dify `api/tasks/add_document_to_index_task.py:144`,
`create_segment_to_index_task.py:98`, `deal_dataset_index_update_task.py:98` and `:199`,
`deal_dataset_vector_index_task.py:94` and `:195`, `remove_document_from_index_task.py:98`,
`regenerate_summary_index_task.py:158` (each handler writes an error status with the failed session, so the status
write raises PendingRollbackError and the document stays "indexing"), `regenerate_summary_index_task.py:271`,
`api/commands/system.py:130`, `api/schedule/create_tidb_serverless_task.py:66`; dispatch
`src/dispatch/case_cost/scheduled.py:81` and `src/dispatch/ai/service.py:444`; superset
`superset/db_engine_specs/clickhouse.py:878` and `impala.py:629`. Read and correctly silent: dispatch
`src/dispatch/cli.py:1010` (rolls back).

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

### `sqlalchemy-practice-bulk-write-skips-hooks`

Rerail's own rule, not a port. SQLAlchemy's ORM-enabled UPDATE, DELETE and INSERT statements, the legacy
`Query.update()`/`Query.delete()` and the `bulk_*` session methods write rows without the unit of work: no mapper
event (`before_`/`after_` `insert`, `update`, `delete`) fires, and neither do attribute `set` events nor
`@validates` methods. A model's audit rows, cache invalidation, timestamps and derived fields are then silently left
stale. It is the twin of `django-practice-bulk-write-skips-hooks`.

It matches, outside test code and Alembic revisions:

- `session.execute(update(M)…)`, `session.execute(delete(M)…)`, `session.execute(insert(M)…)` and
  `session.execute(insert(M), rows)`, on any receiver (`connection.execute(...)` skips the hooks the same way), with
  the statement written inline, bound to a name first (`stmt = update(M).where(...).values(...)`;
  `session.execute(stmt)`), or returned by a function of the run (`session.execute(build_update(new))`, across
  files, through further names and calls);
- the same on the model's own table, `update(M.__table__)`, `delete(M.__table__)`, `insert(M.__table__)`: a Core
  write on the mapped table skips the same hooks;
- `update(self.model)` in a repository class that binds `model = M`, or whose subclasses do (one finding per model);
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

The message names every skipped hook with its path and line, and says to do the hook's work at the write or to change
the rows through the session, and to leave the write alone when skipping the hooks is deliberate.

It leaves alone a hook the write makes up for, read from the listener's own code:

- a listener whose only effect is assigning columns of its target (`target.updated_at = now()`,
  `target.revision += 1`, under ifs too; local assignments and docstrings aside, no call made for its effect, no
  `raise`), when the write sets every one of those columns: polar's `expire_open_checkouts` sets `status=expired`,
  all that its `check_expiration` listener would do;
- a mapper listener that never reads its target (a cache clear, a version bump), when the same function later changes,
  adds or deletes an instance of the model through the session (`obj.col = v`, `session.add(obj)`,
  `session.delete(obj)`; `obj` resolved as an instance: a local, a parameter, or `self._theme` annotated
  `Optional[Theme]`) for an operation the same listener is registered for: the flush runs it once, which is all it
  does (superset's `SetSystemDefaultThemeCommand`).

It also leaves alone: models with no hook for that operation (a validator or `set` event is no delete hook; a `delete`
listener is no update hook); a validator or `set` event on a column the write does not set, when the set columns can
be read; a listener on a base class without `propagate=True`; a write inside one of the model's own hooks (the
listener def, a method the registration names such as superset's
`sa.event.listen(SqlaTable, "before_update", SqlaTable.before_update)`, a validator), which writes in bulk on purpose;
writes whose operation a session-level `do_orm_execute` listener inspects (`is_update`, `is_delete`, `is_insert`)
when the listener names the model or names no model (it may run the hooks itself); tests; Alembic revisions (a CLI
module importing `alembic.command` is checked); Core tables of no model (`update(table)`, `sa.table(...)`).

**Gaps:** a model held in an attribute set at run time (`self.model = model`, mealie's generic repository) or
`type(obj)`; relationship-based dynamic queries (`obj.items.filter(...).delete()`); `event.listen(Mapper, ...)` or a
listener registered on a variable target; a `do_orm_execute` listener that delegates to helpers; a hook whose work the
rows make moot (a listener that would leave the written rows alone, as polar's expiry check does for checkouts that
are already expired) is still reported, since that is data.

**Overlap:** none. Ruff, Bandit, pylint, mypy, flake8-sqlalchemy(2) and the Semgrep registry have no check that
relates a bulk write to the model's events; SQLAlchemy warns about nothing.

**Corpus:** every match read; "real" means the skipped hook's work is missing where the write stands.

| Repository (commit) | Run | Matches | Real |
|---|---|--:|--:|
| apache/superset (`fdf6172`, `superset/`) | this version | 4 | 3 |
| mealie-recipes/mealie (`8dbb928`) | this version | 4 | 3 |
| polarsource/polar (`872272f`, `server/polar` reduced) | this version | 1 | 0 |
| open-webui/open-webui (`0666e59`), fastapi/full-stack-fastapi-template (`77e550a`), jupyterhub/jupyterhub (`3ca3744`) | this version | 0 | - |
| Netflix/dispatch (`dd2837e`) | this version | 0 | - |
| langgenius/dify (`724ac15`, `api/`) | not re-run | 0 | - |
| freqtrade/freqtrade (`7fcb509`), fastapi-users/fastapi-users (`e9c57ff`) | review's run | 0 | - |
| **Total** | | **9** | **6 (67%)** |

The review measured 12 matches, 6 real (50%). The three it read as harmless and this version leaves alone: superset
`superset/commands/theme/set_system_theme.py:44` and `:75` (the Set commands change and add a Theme right after, whose
flush runs the target-free `clear_bootstrap_cache` listener) and polar `polar/checkout/repository.py:131` (the write
sets `status=expired`, all its `check_expiration` listener assigns). Real: superset `set_system_theme.py:99` and `:116`
(the Clear commands flush nothing afterwards, so the bootstrap cache serves the old theme),
`superset/views/sql_lab/views.py:199` (`Query.update(fields)` skips `ExtraJSONMixin.ensure_extra_json_is_not_none`);
mealie `mealie/repos/repository_factory.py:230`, `repository_foods.py:143`, `repository_units.py:130`
(`update(ShoppingListItem)` skips the listener that bumps the shopping lists' `updated_at`). Not real: superset
`superset/semantic_layers/models.py:297` (a cache-version bump the permission-sync listener does not need), mealie
`mealie/repos/repository_recipes.py:165` (the rating recompute is for a recipe deleted in the same method), polar
`polar/checkout/repository.py:183` (anonymizes expired checkouts the expiry listener would leave alone). At 67% the
rule stays in `log` and starts off.

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

### `sqlalchemy-practice-lazy-load-in-loop`

Rerail's own rule, not a port; the SQLAlchemy twin of `django-practice-select-related-in-loop`. A relationship whose
loader is `lazy="select"` (the default) emits a SELECT the first time it is read on each instance. For a collection
(one-to-many or many-to-many) that is one SELECT per row of a loop, whatever the data: N+1 queries. A loader option on
the statement (`selectinload(M.rel)`) loads them all with the rows. Many-to-one reads are
`sqlalchemy-practice-lazy-many-to-one-in-loop`'s, since their cost depends on how many rows share a parent.

It matches a loop over ORM rows of model M that reads `x.rel` of each row, where `rel` is a collection of M with the
lazy "select" strategy. The relationship, its strategy and its direction are read from the models, across files
(lib/sqlalchemy's `sa_lazy_rel` and `sa_rel_dir`): declared on M, on a base class of the run, or given to M by another
model's `backref=`; a collection when it is `Mapped[list[...]]` (or set, Sequence, ...), `uselist=True`,
`secondary=` (keyword or second positional argument), a relationship whose foreign key sits on the target
(`foreign_keys=[B.a_id]`, `foreign_keys="B.a_id"`, or only the target has a ForeignKey to this table, table names read
from `__tablename__` or, without one, the class name in snake case), or the backref of a many-to-one. A relationship
whose direction cannot be decided is left to the many-to-one rule.

The loops and rows come from lib/sqlalchemy_loops:

- a `for` statement or a comprehension over rows of a statement in the same function: `session.scalars(select(M)…)`,
  `session.execute(select(M)).scalars()`, `session.query(M).filter(...).all()`, `(await s.scalars(select(M))).all()`,
  rows or a statement held in a name, and a legacy query over `cls` in a classmethod of M;
- `for i, x in enumerate(rows)`, and `for x, y in session.execute(select(M, N))` (each name is its entity);
- rows a function of the run returns, one hop, resolved through imports or the call graph:
  `for incident in incident_service.get_all(db_session=db_session)`; the loader check is made on the statement inside
  that function, and the message says that adding the option there changes its other callers' query too;
- a read is `x.rel` in the loop's own per-row code (not in a nested def, lambda or class, and not in the
  comprehension's iterable), or a call there that hands `x` to a function of the run reading `p.rel` on that parameter
  (`[describe(o) for o in rows]`). One finding per loop and relationship, at its first read.

It leaves alone: loader options naming the relationship anywhere in a chain (`selectinload(M.a).joinedload(A.b)` for
`x.a`, `Load(M).joinedload(M.x)`, `joinedload("x")`), a wildcard loader (`selectinload("*")`, `raiseload("*")`),
options splatted from a name (`.options(*OPTS)` with `OPTS = [selectinload(M.x)]` in the module or imported), an
option list it cannot read (`.options(*options)` with `options` a parameter: taken as loading everything), and an
`.options(...)` naming it on another statement of the function that builds the rows; relationships whose `lazy=` is
anything but "select"; many-to-one reads; columns, the key column (`x.rel_id`) and `x.rel = v`; single rows
(`.first()`, `.one()`, `session.get()`); loops over a parameter; `yield_per` iterations, which are written to stream
and restrict eager loading of collections; tests and Alembic revisions (a CLI importing `alembic.command` is checked).

**Gaps:** a loop that stops at the first row it wants (`if o.lines: return o`) is still reported, since it reads every
row until then; the message says to leave a loop that stops after a row or two. Callees more than one call away,
generic repositories (`select(self.model)`, mealie's repository layer) and models of other packages (flask_appbuilder's
`PermissionView`) are not followed; an `.options(...)` naming the relationship on another statement of the same
function is taken as covering the loop.

**Overlap:** none. Ruff, pylint, Bandit and flake8-sqlalchemy(2) have no SQLAlchemy loading checks; Semgrep's registry
has only `len-all-count` and `batch-import`. The relationship's strategy and direction sit on the models, usually in
another file. SQLAlchemy's own answer is opt-in (`lazy=` per relationship, `raiseload("*")`); profilers such as
nplusone find it at test time.

**Corpus:** every match read.

| Repository (commit) | Run | Matches | Real |
|---|---|--:|--:|
| Netflix/dispatch (`dd2837e`) | this version | 8 | 8 |
| mealie-recipes/mealie (`8dbb928`) | this version | 2 | 2 |
| open-webui/open-webui (`0666e59`), fastapi/full-stack-fastapi-template (`77e550a`), jupyterhub/jupyterhub (`3ca3744`) | this version | 0 | - |
| apache/superset (`fdf6172`, `superset/`), langgenius/dify (`724ac15`), polarsource/polar (`872272f`) | not re-run | 0 | - |
| freqtrade/freqtrade (`7fcb509`), fastapi-users/fastapi-users (`e9c57ff`) | review's run | 0 | - |
| **Total** | | **10** | **10 (100%)** |

An earlier run of this version also reported dispatch `src/dispatch/workflow/scheduled.py:114` (`sync_workflow`
rebinds its `instance` parameter before reading `instance.artifacts`); a read through a helper now needs the
parameter never rebound (a case covers it), and the final run leaves it alone. The review's ten collection reads are
all still reported and all real: dispatch `src/dispatch/entity_type/service.py:98` (`signal_instance.entities`),
`src/dispatch/feedback/service/scheduled.py:129` and `:135` (`participants` per incident and case),
`src/dispatch/incident/scheduled.py:129` and `src/dispatch/notification/service.py:142` (`notification.filters`),
`src/dispatch/incident_role/service.py:193`, `:195`, `:198` (three collections per role); mealie
`mealie/db/fixes/fix_migration_data.py:86` (`recipe.recipe_ingredient` for every recipe) and `:110`
(`shopping_list.label_settings`). freqtrade and polar declare `lazy="selectin"`/`"raise"` on what their loops read.

**Sources:**
- SQLAlchemy 2.0 docs, [Relationship Loading Techniques](https://docs.sqlalchemy.org/en/20/orm/queryguide/relationships.html):
  lazy loading has "the N plus one problem ... for any N objects loaded, accessing their lazy-loaded attributes means
  there will be N+1 SELECT statements emitted"; `selectinload()` is the loader to use for collections, and `raiseload`
  / `lazy="raise"` prevent unwanted lazy loads.
- SQLAlchemy 2.0 docs, [ORM Querying Guide: Fetching Large Result Sets with Yield Per](https://docs.sqlalchemy.org/en/20/orm/queryguide/api.html#fetching-large-result-sets-with-yield-per):
  the yield_per option "is not compatible with “subquery” eager loading loading or “joined” eager loading when using
  collections. It is potentially compatible with “select in” eager loading, provided the database driver supports
  multiple, independent cursors."
- SQLAlchemy FAQ, [Performance: Query Profiling](https://docs.sqlalchemy.org/en/20/faq/performance.html#query-profiling):
  SQL logging shows "if dozens/hundreds of queries are being issued which could be better organized into much fewer
  queries"; the ORM's "eager loading" feature automates this.

### `sqlalchemy-practice-lazy-many-to-one-in-loop`

Rerail's own rule, not a port; the many-to-one half of `sqlalchemy-practice-lazy-load-in-loop`. A lazily loaded single
object (many-to-one or one-to-one) read on each row of a loop emits a SELECT for each parent the session's identity map
has not seen yet: one per row when every row has its own parent (an incident's commander), one in all when they share
it (a handful of priorities or tag types). The rule cannot see the data, so it ships in `log` and off.

It matches the same loops, rows and reads as the collection rule, for a relationship that is a single object
(`Mapped["B"]`, `Optional[...]`, `uselist=False`, a foreign key on the declaring model: `b_id =
Column(ForeignKey("b.id"))`, `foreign_keys=[b_id]`, `remote_side=`; or the backref of a one-to-many) or whose direction
cannot be decided.

It leaves alone a statement that keeps the rows to one parent: a `where`/`filter` comparing one of the relationship's
foreign key columns with `==` (`select(Line).where(Line.order_id == order_id)`), or `filter_by(order_id=...)`, in the
loop's function or in the callee that builds the rows; collections (the other rule); and everything the collection
rule leaves alone (loader options, streams, single rows, tests, Alembic revisions).

**Gaps:** how many distinct parents the rows have is data; a filter on the parent through a join
(`.join(M.rel).where(P.id == v)`) is not read as one parent; a loop that stops after a few rows is reported.

**Overlap:** none, as for the collection rule.

**Corpus:** every match read; "real" means the rows point at a distinct parent each.

| Repository (commit) | Run | Matches | Real |
|---|---|--:|--:|
| Netflix/dispatch (`dd2837e`) | this version | 17 | 12 |
| mealie-recipes/mealie (`8dbb928`), open-webui/open-webui (`0666e59`), fastapi/full-stack-fastapi-template (`77e550a`), jupyterhub/jupyterhub (`3ca3744`) | this version | 0 | - |
| **Total** | | **17** | **12 (71%)** |

Real (a distinct parent per row): dispatch `feedback/service/scheduled.py:128` and `:134` (`commander`, `assignee`),
`incident/scheduled.py:278` and `:301`, `report/flows.py:116`, `task/scheduled.py:39`, and, read through the
function each loop hands its row to, `case/scheduled.py:46`, `:72`, `:92` (`case.assignee` in the reminder helpers),
`incident/scheduled.py:232`, `report/scheduled.py:66`, `:87` (`incident.commander`). Not real: `ai/service.py:558`
(`tag.tag_type`, a handful of types), `cli.py:919`, `:924`, `:1173` (one organization, a few plugins, a loop that
stops at its first match) and `report/scheduled.py:49` (`incident.incident_priority`, a few priorities). The
shared-parent reads are data the rule cannot see, so it ships in `log`, off.

**Sources:**
- SQLAlchemy 2.0 docs, [Relationship Loading Techniques: Lazy Loading](https://docs.sqlalchemy.org/en/20/orm/queryguide/relationships.html#lazy-loading):
  "The one case where SQL is not emitted is for a simple many-to-one relationship, when the related object can be
  identified by its primary key alone and that object is already present in the current Session. For this reason,
  while lazy loading can be expensive for related collections, in the case that one is loading lots of objects with
  simple many-to-ones against a relatively small set of possible target objects, lazy loading may be able to refer to
  these objects locally without emitting as many SELECT statements as there are parent objects."
- SQLAlchemy 2.0 docs, [Relationship Loading Techniques: Select IN loading](https://docs.sqlalchemy.org/en/20/orm/queryguide/relationships.html#select-in-loading):
  selectin loading emits a SELECT with "the primary key values of the parent object, or in the case of a many-to-one
  relationship to those of the child objects, inside of an IN clause".

### `sqlalchemy-practice-connect-without-commit`

Rerail's own rule, not a port. A Core connection from `engine.connect()` begins a transaction on its first statement
and rolls it back when it is closed, unless the code calls `conn.commit()` ("commit as you go") or opens the
connection with `engine.begin()` ("begin once"). An INSERT, UPDATE, DELETE or DDL statement run on such a connection
and never committed is lost without an error: the code looks right, the statement runs, and nothing is stored.

It matches a connection the function opens itself: `with E.connect() as conn:`, `async with E.connect() as conn:`,
`conn = E.connect()` or `conn = await E.connect()` (connect() with no arguments, maybe chained with
`.execution_options(...)`, in a file importing SQLAlchemy). It reports each write on it:

- `conn.execute(s)` where `s` is `insert(...)`, `update(...)` or `delete(...)` (sqlalchemy, sqlmodel and dialect
  inserts such as `postgresql.insert`, resolved through imports), Core's `table.insert()`/`.update()`/`.delete()`, a
  DDL element (`CreateTable`, `DropIndex`, `CreateSchema`, ...), a chain from any of them (`.values()`, `.where()`,
  `.on_conflict_do_update()`), or a name the function binds to one;
- `conn.execute(text(sql))` and `conn.exec_driver_sql(sql)` where `sql` is a string literal (an f-string,
  `"…" % x`, `"…".format(...)`, or a name bound to one) whose statement starts with INSERT, UPDATE, DELETE, MERGE,
  UPSERT, REPLACE, CREATE, ALTER, DROP, TRUNCATE, GRANT or REVOKE, or starts with WITH and holds a data-modifying
  statement (`WITH old AS (…) DELETE FROM …`, `WITH gone AS (DELETE FROM … RETURNING id) SELECT …`);
- `metadata.create_all(conn)` and `metadata.drop_all(bind=conn)`;
- a call that hands the connection to a function of the run (resolved through the call graph, in any file) that makes
  such a write on that parameter and neither commits nor begins on any parameter: reported at the call, since the
  helper rightly leaves the transaction to its caller and the caller never commits.

The message leads with `engine.begin()`, which is right on every SQLAlchemy version, and offers `conn.commit()` only
where connections have it: a 1.x Connection (without `future=True`) has no `commit` attribute.

It leaves alone:

- a write followed, anywhere later in the with block or function, by `conn.commit()` (or `conn.connection.commit()`),
  even in only one branch (precision first);
- a connection the block or function opens a transaction on: `with E.connect() as conn, conn.begin():` (dify's relyt
  and TiDB vector stores), `with conn.begin():`, `trans = conn.begin()`, `begin_nested()`;
- autocommit isolation on the connection (`conn.execution_options(isolation_level="AUTOCOMMIT")`,
  `E.connect().execution_options(isolation_level=...)`, a DBAPI `.autocommit = True`) or on the engine. What the
  engine expression evaluates to is followed through a local name, module variable, imported module variable,
  `self.<attr>` the same class assigns, and the return value of a function of the run that makes it
  (`get_engine().connect()`, `eng = get_engine()`, an `lru_cache` factory, through further calls and the names they
  return); it is autocommit when that value is `create_engine(..., isolation_level="AUTOCOMMIT")`,
  `….execution_options(isolation_level="AUTOCOMMIT")`, `execution_options={"isolation_level": ...}` or
  `connect_args={"isolation_level": None}`. An isolation level that is not a literal counts as autocommit;
- CREATE, ALTER and DROP text on an engine made from a `sqlite…` URL literal (pysqlite runs DDL outside a transaction,
  so it persists; DML there is still reported);
- a connection passed to a function of the run that commits or begins on one of its parameters;
- a statement name rebound before the call: a name reads its last assignment in the function, so superset's metadb
  `insert_row` (`query = table.insert()…` run in `engine.begin()`, then `query = select(func.count())…` on
  `engine.connect()`) is a read;
- `engine.begin()`, reads (`select(...)`, `text("SELECT …")`, `WITH … SELECT …`), connections the function receives as
  a parameter (the caller owns the transaction), sessions, `sqlite3.connect(path)`, test files and Alembic revisions
  (`op.get_bind()` runs inside the migration's transaction; a CLI importing `alembic.command` is checked).

**Gaps:** the rule assumes SQLAlchemy 2.0 (or 1.4 with `future=True`). A 1.x engine autocommitted DML, so in a project
pinned below 2.0 (the pack matches any `sqlalchemy` dependency) the rule's premise fails, which it cannot see; its
first fix, `engine.begin()`, is still right there. SQL it cannot read is not a write: a parameter (`text(sql)`), a
string built in another function, a file's contents (`text(open(path).read())`), `str(ddl_object)`; so dispatch's
`init_database`, which hands an `engine.connect()` connection to `setup_fulltext_search` (DDL from a `.sql` file and
sqlalchemy-searchable's trigger DDL via `exec_driver_sql(str(sql))`) and never commits, is not reported. Any later
commit counts, even on one branch. A connection stored on `self` or returned is not followed; an engine made autocommit
where the rule cannot see it (Flask-SQLAlchemy's `SQLALCHEMY_ENGINE_OPTIONS`, a `connect` event listener, an engine
passed in as a parameter) is treated as transactional; a helper is followed one call deep.

**Overlap:** none. Ruff, Bandit, pylint and Semgrep's registry have no transaction checks; the python pack has nothing
on SQLAlchemy connections. On 1.4, `SQLALCHEMY_WARN_20=1` warned about implicit autocommit; on 2.0 nothing warns.

**Corpus:** no match. This version was run silent on dispatch (`dd2837e`), open-webui (`0666e59`), mealie (`8dbb928`),
fastapi-users (`e9c57ff`), the full-stack FastAPI template (`77e550a`) and jupyterhub (`3ca3744`); the review's runs
are 0 on superset, dify, polar, freqtrade, flaskbb, lemur, letta, we-mp-rss and fastapi-react. The sites
it reads and leaves alone are the safe idioms: dify's relyt and TiDB vector stores (`with self.client.connect() as
conn, conn.begin():`), polar's `_grant_read_user_permissions` (GRANT, then `conn.commit()`), flaskbb's `drop_all`
(then `connection.commit()`), superset's version-retention prune (`conn.begin()`) and read-only `engine.connect()`
blocks. The rule ships in `log`, on: the 2.0 docs state the hazard outright, the cases and counterexamples (an
autocommit engine from a factory, CTE-led writes) read right, and the corpus gives no false match, but no measured
precision either.

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
