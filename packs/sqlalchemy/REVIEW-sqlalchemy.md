# Review: the sqlalchemy pack

Independent review of the five rules in `packs/sqlalchemy/`. Rules, cases, pack.json and README were not changed; the
verdicts are the output. Every count below was measured for this review (`corpus-run`, run names
`review-sqlalchemy-*` under `~/rerail-bench/python-arch/runs/`); none is copied from the README.

## 1. Summary

"Real" means a reviewer would want the line changed: the bug the rule names happens there, or would on the data the
code handles. A match whose pattern holds but whose effect is nil where it stands (the hook's work is redone a line
later) counts as not real.

| Rule | Verdict | Proposed mode / on | Corpus matches (repos) | Read precision | Reason |
|---|---|---|--:|--:|---|
| `session-reused-after-failed-commit` | keep | steer / on | 16 (4) | 16/16, 100% | Outage-shaped bug, every match real, cross-file caller reuse no linter has |
| `bulk-write-skips-hooks` | rework | log / on until re-measured at 90%, then steer | 12 (3) | 6/12, 50% | Premise sound and every match holds the pattern, but half are harmless where they stand: the write redoes the hook's work, or a flush right after runs the hook |
| `connect-without-commit` | rework | log / on | 0 (0 of 15) | n/a | No corpus signal; its first fix (`conn.commit()`) raises AttributeError on SQLAlchemy 1.x, and the pack's dependency match lets 1.x projects in |
| `text-interpolation` | cut | (none) | 26 (6) | 2/26, 8% | Matches are identifiers from code, settings and metadata; injection from request data is `python-security-sql-injection`'s |
| `lazy-load-in-loop` | rework | steer / on for collections; many-to-one reads log or dropped | 21 (2) | 16/21, 76% | All 5 false matches are many-to-one reads (a few shared parents, or a loop that stops early); all 10 collection reads are real |

Verdicts: 1 keep, 3 rework, 1 cut.

Read the rules in this order (by harm prevented and by trust earned), not the alphabetical order of `pack.json`:

1. `sqlalchemy-practice-session-reused-after-failed-commit`: one failed write fails every later write of the batch or
   request; precise; cross-file.
2. `sqlalchemy-practice-bulk-write-skips-hooks`: silent data drift; the hook lives in another module.
3. `sqlalchemy-practice-lazy-load-in-loop`: performance, but the most frequent real finding.
4. `sqlalchemy-practice-connect-without-commit`: silent data loss, never seen in the corpus.
5. `sqlalchemy-practice-text-interpolation`: proposed cut.

Every rule's message is over the 200-character cap of `docs/pack-format.md` ("Message", criterion 6): 227 to 328
characters before placeholders expand. That is a text fix for all five, listed per rule, and does not change a verdict.

## 2. Method

- Premises run, not only read: `/tmp/review-sqlalchemy/semantics.py`, SQLAlchemy 2.0.58 on SQLite
  (`uv run --no-project --with 'sqlalchemy>=2.0,<2.1' python semantics.py`):
  1. after a caught IntegrityError from `commit()`, the next `commit()` raises PendingRollbackError;
  2. `session.execute(update(Child).values(label=...))`, `session.execute(insert(Child), rows)` and
     `session.execute(update(Child.__table__)...)` fire none of `before_update`, `before_insert`, `@validates`;
  3. an `insert()` on `with engine.connect() as conn:` without `commit()` leaves 0 rows;
  4. 22 rows sharing one many-to-one parent, each reading `row.parent` lazily: 1 SELECT in total, not 22.
  And on SQLAlchemy 1.4.54 (`--with 'sqlalchemy>=1.4,<1.5'`): `engine.connect()` returns a Connection with no
  `commit` attribute unless the engine is `future=True`.
- Sources opened (the README's URLs). Each quoted sentence is on its page: the sessions FAQ ("catches an exception upon
  Session.flush() or Session.commit() and does not properly handle the exception"), the DML caveats ("bypass ORM unit of
  work automation", "use the SessionEvents.do_orm_execute() event"), the 2.0 migration guide ("do away with all attempts
  to implicitly commit transactions"; text() as "an explicit boundary into the textual SQL realm where attention to
  untrusted user input must be given"; 1.4's "RemovedIn20Warning: The current statement is being autocommitted using
  implicit autocommit"), the transactions tutorial ("The transaction is not committed automatically"), the loading guide
  ("the N plus one problem"), and issue #9339 (zzzeek: "before_update applies only to ORM flush operations ... the only
  event hook to use is do_orm_execute()"). The sources hold up for every rule.
- Cases: `nice -n 19 node dist/cli/cli.js rules test packs/sqlalchemy/rules/<rule>.rerail` passes for all five.
- Counterexamples: one scratch repository per rule, `/tmp/review-sqlalchemy/{bulk,connect,lazy,session,text}`, each run
  with `corpus-run /tmp/review-sqlalchemy/<dir> review-sqlalchemy-<rule>-cx packs/sqlalchemy/rules/<rule>.rerail`.
- Corpus: section 9.

## 3. `sqlalchemy-practice-session-reused-after-failed-commit`

**Value.** Real and common. The sessions FAQ has an entry for exactly this, with the rule's shape as its example
(`except: pass`, then another `commit()`), and the probe reproduced it on 2.0.58. One bad row fails the rest of a batch;
in a Celery task the handler's own error-status write fails, so the row stays "processing" forever (dify's pattern
below). Code that commits per item in loops or tasks meets it: 4 of the 15 corpus repositories have it.

**Correctness.** It needs a catch that can see a DB error, no rollback/close/re-raise on every path, and a concrete reason
the session lives on (the handler uses it, a loop, a later statement, a caller that reuses it, a shared `db.session`).
Commits inside `with Session() as s:` or `with s.begin()`/`begin_nested()` in the try are left alone, which is right.
Counterexamples (`/tmp/review-sqlalchemy/session/app/jobs.py`, run `review-sqlalchemy-session-reused-after-failed-commit-cx`):

| Line | Shape | Expected | Result |
|---|---|---|---|
| 20 | `with SessionLocal() as session:` loop, `except IntegrityError: log` | report | reported (loop) |
| 29 | the same loop on an unannotated `db` parameter | report | **missed** (`db` is not session-like; README gap) |
| 41 | `sp = session.begin_nested()` … `except IntegrityError: sp.rollback()` | silent | silent |
| 52 | `except IntegrityError: session.rollback()` | silent | silent |
| 63 | `except IntegrityError: session.expunge(tag)` in a loop | report | reported ("the handler itself uses the session") |
| 72 | `save_tag(session)` returns False; the FastAPI route calling it then runs `session.query(...)` | report | reported (caller reason) |

The miss at 29 is a recall gap, not a precision one: helpers taking an unannotated `db` are common in FastAPI code.
Treating a parameter as a session when a caller passes it a known session would close it.

**Corpus.** 16 matches in 4 repositories, all read, all real.

| Repository | Matches | Real |
|---|--:|--:|
| langgenius/dify (`api/`) | 11 | 11 |
| Netflix/dispatch | 2 | 2 |
| apache/superset (`superset/`) | 2 | 2 |
| open-webui/open-webui | 1 | 1 |
| the other 11 that finished | 0 | - |

Real: dify `tasks/add_document_to_index_task.py:144` (the handler sets `indexing_status = ERROR` and commits on the
failed session, so the status write raises too and the document stays "indexing"), the same shape in
`create_segment_to_index_task.py:98`, `deal_dataset_index_update_task.py:98` and `:199`,
`deal_dataset_vector_index_task.py:94` and `:195`, `remove_document_from_index_task.py:98`,
`regenerate_summary_index_task.py:158`; `regenerate_summary_index_task.py:271` (the next document of the loop);
`commands/system.py:130` (a loop over apps on `db.session`, `except Exception: echo`);
`schedule/create_tidb_serverless_task.py:66` (`db.session`, called again by a `while True`). Dispatch
`src/dispatch/case_cost/scheduled.py:81` (its comment says "we shouldn't fail to update all cases when one fails",
which a failed commit then does) and `src/dispatch/ai/service.py:444` (the weekly report calls it per incident with the
same `db_session`). Superset `superset/db_engine_specs/clickhouse.py:878` and `impala.py:629` (a swallowed `db.session`
flush/commit during query execution; SQL Lab then saves query state on the same scoped session). Open-webui
`backend/open_webui/models/groups.py:654` (`log.exception; continue` in a loop of inserts). Every dify handler was read
with its try body. Read and correctly silent: dispatch `src/dispatch/cli.py:1010` (rolls back).

**Overlap.** The `python` pack's `bandit-b110-try-except-pass`, Ruff S110/S112/BLE001 and pylint W0718 flag the
swallowing on any try; none knows the try commits a session, that the handler leaves it un-rolled-back, or that the
session is used again, and none fires on a narrow `except IntegrityError: log(...)`, which this rule does. SQLAlchemy's
own signal is the PendingRollbackError, at runtime, after the damage. The cross-file caller check has no counterpart.

**Text.** Description (139 characters) meets the criteria. The message is 312 characters before `{why}` expands: over
the cap. Its content is right. A form that fits: "Call `{s}.rollback()` in this handler, or re-raise: after a
failed `{s}.{m}()` the session raises PendingRollbackError until rolled back, and {why}."

**Verdict: keep**, `steer`, on. Shorten the message.

## 4. `sqlalchemy-practice-bulk-write-skips-hooks`

**Value.** Real: the DML caveats page and the maintainer's answer in #9339 say mapper events and validators never run for
ORM-enabled UPDATE/DELETE/INSERT, and the probe saw none fire for any of the three bulk forms. Silent and cross-file:
the listener sits by the model, the bulk write in a repository or command. How often a team meets it depends on how
much it puts in hooks; in the corpus, mealie, superset and polar.

**Correctness.** The column filter (a validator or `set` event counts only when the write sets its column) and the
`do_orm_execute` exemption are good precision work. Counterexamples (`/tmp/review-sqlalchemy/bulk/app/services.py`, run
`review-sqlalchemy-bulk-write-skips-hooks-cx`):

| Line | Shape | Expected | Result |
|---|---|---|---|
| 10 | `update(Item).values(label=new, updated_at=func.now())`; Item's only hook is a `before_update` that sets `updated_at` | silent: the write does the hook's work | **reported** (README gap) |
| 15 | `update(Item.__table__)…values(label=new)` | report: the probe shows the hooks are skipped the same way | **missed**: a model's own `__table__` is treated as a Core table with no hooks |
| 23 | `update(self.model)` in a generic repository | report | missed (README gap) |
| 28 | `session.query(Tag)…update({"name": new})`, Tag has `@validates("name")` | report | reported |
| 33 | `delete(Tag)`, Tag has only a validator | silent | silent |
| 42 | `session.execute(build_update(new))`, the `update(Item)` built in a helper | report | missed |

**Corpus.** 12 matches in 3 of the 15 finished repositories, all read. All 12 hold the pattern (a hook of that
operation exists and is skipped); 6 are real.

| Repository | Matches | Real |
|---|--:|--:|
| apache/superset (`superset/`) | 6 | 3 |
| mealie-recipes/mealie | 4 | 3 |
| polarsource/polar (`server/polar`, see section 9) | 2 | 0 |
| the other 12 | 0 | - |

Real: superset `superset/commands/theme/set_system_theme.py:99` and `:116` (the two Clear commands null
`is_system_default`/`is_system_dark` in bulk and flush nothing else, so `clear_bootstrap_cache`, an `after_update`
listener in `superset/models/core.py:231`, never runs and the bootstrap cache serves the old theme);
`superset/views/sql_lab/views.py:199` (`Query.update(fields)` with request fields, `extra_json` among the allowed ones,
skips `ExtraJSONMixin.ensure_extra_json_is_not_none`, so a JSON `null` is stored as NULL); mealie
`mealie/repos/repository_factory.py:230`, `repository_foods.py:143`, `repository_units.py:130` (`update(ShoppingListItem)`
on a label/food/unit merge skips `buffer_shopping_list_updates`, so the shopping lists' `updated_at` is not bumped).

Not real: superset `set_system_theme.py:44` and `:75` (the Set commands flush `self._theme` right after, which fires
the same after_update listener and clears the cache); superset `superset/semantic_layers/models.py:297` (a
cache-version bump; the skipped `before_update` is a permission sync the bump does not need); mealie
`mealie/repos/repository_recipes.py:165` (`delete(UserToRecipe)` skips the rating recompute for a recipe deleted in the
same method); polar `polar/checkout/repository.py:131` (`expire_open_checkouts` sets `status=expired` in bulk, which is
all the skipped `check_expiration` listener does) and `:183` (anonymizes already-expired checkouts, which the listener
would leave alone).

Two of the harmless shapes are visible to the rule: the write sets the column the listener assigns (counterexample line
10, polar `:131`), and the same function then adds or changes an instance of the same model, whose flush runs the
listener anyway (superset's Set commands). Excluding both turns the corpus to 6 of 9.

**Overlap.** None. Ruff, pylint, Bandit, mypy and flake8-sqlalchemy(2) have no check that relates a bulk write to a
model's events; SQLAlchemy warns about nothing (the probe saw no warning).

**Text.** Description (177 characters) meets the criteria. The message is 230 characters before `{hooks}`, which adds a
path and line per hook (mealie's findings run to about 330): over the cap. Its advice and its "leave it when skipping
them is deliberate" are right; "writes Item rows past the session's unit of work" is jargon for "runs no Item hooks".

**Verdict: rework**, then re-measure.
- Leave a mapper-event hook alone when the write sets every column the listener assigns (`target.<col> = …` in the
  listener body, matched against `.values(...)` keys).
- Leave a write alone when the same function later adds or flushes an instance of the same model (the listener then
  runs for that flush), or say in the message that the hook runs only for that object.
- Treat `update(M.__table__)`/`delete(M.__table__)`/`insert(M.__table__)` as writes on M.
- Add each as a case. Start `log`, on; move to `steer` when a re-measure reads at least 90% real.

## 5. `sqlalchemy-practice-connect-without-commit`

**Value.** The premise is right on 2.0 (the probe stored 0 rows) and is the headline change of the 2.0 migration guide;
the failure is silent data loss. But application code rarely writes Core DML on raw `engine.connect()` connections: 0
matches in 15 repositories, so its value to a team is unmeasured.

**Correctness.** Counterexamples (`/tmp/review-sqlalchemy/connect/app/jobs.py`, run `review-sqlalchemy-connect-without-commit-cx`):

| Line | Shape | Expected | Result |
|---|---|---|---|
| 12 | `with get_engine().connect() as conn:`, `get_engine()` returns `create_engine(..., isolation_level="AUTOCOMMIT")` | silent | **reported**: an engine's autocommit is read from names only, not from a factory's return |
| 18 | `text("WITH old AS (…) DELETE FROM users …")` | report | **missed**: only the first word is matched against the write verbs, so a CTE-led write is not a write |
| 24 | `conn.execute(text(sql))`, `sql` a parameter | (unknowable) | silent (README gap) |
| 30 | `conn.execute(insert(users)…)`, no commit | report | reported |
| 36 | commit only under `if not dry:` | arguably report | silent (README: any later commit counts) |
| 44 | insert, then `conn.commit()` | silent | silent |
| 54 | connection kept on `self.conn`, written in another method | report | missed (README gap) |

**Steer safety.** The pack matches any project depending on `sqlalchemy`, with no version check, and the rule cannot see
the pin. On SQLAlchemy 1.x (Netflix/lemur pins 1.3.24 in `requirements-dev.txt`) the premise fails, since a 1.x engine
autocommits DML. Worse, the message's first fix, "Call `{c}.commit()` after the writes", raises AttributeError there: on
1.4.54 a non-future Connection has no `commit` (checked above). Its second fix, `engine.begin()`, works on 1.x and 2.0.

**Corpus.** 0 matches in all 15 finished repositories (dify and polar on the reduced trees of section 9). Nothing to
read.

**Overlap.** None: no linter models Core transactions. On 1.4, `SQLALCHEMY_WARN_20=1` emitted a RemovedIn20Warning for
implicit autocommit; on 2.0 nothing warns.

**Text.** The description (157 characters) names a version ("SQLAlchemy 2.0 rolls back an uncommitted connection"):
criterion 4, no versions. "an uncommitted connection is rolled back when it closes" says the same. The message (319
characters) is over the cap, and its first sentence has no verb ("This write on `conn`, a connection from
`engine.connect()` that is never committed: …").

**Verdict: rework**, stay `log`, on.
- Lead the message with `{e}.begin()` (safe on every version), and offer `{c}.commit()` only as the 2.0 alternative.
- Read an engine's autocommit through a call of a function of the run that returns `create_engine(...)` (counterexample
  line 12).
- Treat `WITH … INSERT|UPDATE|DELETE|MERGE` as a write (counterexample line 18).
- Drop the version from the description; shorten the message. Keep `log` until some corpus shows a match.

## 6. `sqlalchemy-practice-text-interpolation`

**Value.** Injection through `text()` is real and the migration guide names `text()` as the untrusted-input boundary.
But this rule reports the construct whatever the value's source, and on the corpus that means identifiers: table,
schema and column names from code, settings or SQLAlchemy metadata. Those cannot be bound, are rarely
attacker-controlled, and the message's advice (bind it, or quote it) is noise for them. The exploitable subset, request
data reaching SQL, is what `python-security-sql-injection` reports, across files, with taint, on the same sinks
(`sqlalchemy.text`, `exec_driver_sql`).

**Correctness.** The constant-following is thorough (module constants, enums, `int()` casts, allow-lists, quoting
calls, a helper's callers). Counterexamples (`/tmp/review-sqlalchemy/text/app/repo.py`, run
`review-sqlalchemy-text-interpolation-cx`):

| Line | Shape | Expected | Result |
|---|---|---|---|
| 10 | `for fts in TABLES.values(): text(f"INSERT INTO {fts}…")`, `TABLES` a module dict of literals | silent | **reported** (`.values()` is a call outside the run; flaskbb's two matches are this shape) |
| 15 | `text(f"SELECT count(*) FROM {model.__table__.name}")` | silent | **reported** (README gap) |
| 20 | `f"… name = '{name}'"`, `name` a parameter | report | reported |
| 26 | `sql = "… '%s'" % email; stmt = text(sql)` | report | reported |
| 36 | `text("… WHERE " + where_clause(col, val))`, the helper returns `f"{col} = '{val}'"` | report | reported |
| 44 | `select(literal_column(f"{col}"))` | report | missed: `literal_column()`/`column()` are not sinks |
| 49 | bound `:n` | silent | silent |
| 55 | `limit = max(1, min(int(limit), 100))`, then `f"… LIMIT {limit}"` | silent | **reported**: only a bare `int()` counts as a cast |

**Corpus.** 26 matches in 6 of the 15 finished repositories, all read, 2 real.

| Repository | Matches | Real |
|---|--:|--:|
| apache/superset (`superset/`) | 9 | 2 |
| langgenius/dify (`api/`) | 8 | 0 |
| polarsource/polar (`server/polar`) | 4 | 0 |
| flaskbb/flaskbb | 2 | 0 |
| rachelos/we-mp-rss | 2 | 0 |
| Netflix/dispatch | 1 | 0 |
| the other 9 | 0 | - |

Real: superset `superset/db_engine_specs/hive.py:341` and `:369` (`DROP TABLE IF EXISTS {str(table)}` / `CREATE TABLE
{str(table)}` with the table of a CSV upload). Not exploitable: `Table.__str__` percent-encodes each part, and its
docstring says it "Should not be used for SQL generation". But that is why the rule's advice (quote with the dialect's
preparer) is the right change: a name with a space becomes `my%20table`.

Not real (identifiers from code, config or metadata, or SQL by design): dify
`repositories/app_statistic_query_repository.py:51`, `agent_observability_repository.py:998`, `:1149`,
`sqlalchemy_api_workflow_run_repository.py:1766`, `:1815`, `:1864`, `:1933` (`converted_created_at` is
`convert_datetime_to_date("created_at")`, an f-string of a literal column and the literal `":tz"`) and
`agent_observability_repository.py:1090` (`binding_filters`, literal fragments with bind parameters joined from a
list); superset `superset/utils/encrypt.py:369`, `:577`, `:642` (the re-encrypt command's own table and column names;
the code already carries `# noqa: S608`), `superset/utils/mock_data.py:293` (a foreign key's target from metadata),
`superset/examples/generic_loader.py:89` (the examples schema, quoted), `superset/connectors/sqla/partition_mapping.py:1910`
and `:2078` (an admin-authored SQL transform with its colons escaped, SQL by design); dispatch
`src/dispatch/search/fulltext/__init__.py:132` (`column.name`, quoted when reserved); flaskbb
`flaskbb/search/backends/sqlite.py:124`, `:130` (`_FTS_TABLES` built from a constant spec; the code comments "`fts` is
an internal constant, never user input"); we-mp-rss `data_sync.py:302`, `:305` (a schema-sync's `ALTER TABLE … ADD
COLUMN` from the model's own columns); polar `polar/event/repository.py:1087`, `polar/meter/service.py:494`,
`polar/metrics/service.py:254` (`SET LOCAL TIME ZONE '{timezone.key}'` with a `ZoneInfo`, whose key must name a real
tz database entry; SET takes no bound parameter, `set_config()` would) and `polar/models/subscription.py:139` (an index's
`postgresql_where` built from `SubscriptionStatus.billable_statuses()`, enum values, at import time).

**Overlap.** `python-security-sql-injection` covers the same sinks (`sqlalchemy.text`, `exec_driver_sql`) for request
data. None of the 26 matches is injectable request data (polar's `timezone.key` comes from a request, through a
`ZoneInfo`), and the two real ones are identifier-quoting fixes. Ruff S608 / Bandit B608 (not ported in the `python`
pack; on for teams that enable Ruff's `S` rules) flag the
same f-strings by keyword; superset's `# noqa: S608` on three of these lines shows a team already dismissing them. What
this rule adds over both is reporting non-request values, and on the corpus those are its false positives.

**Text.** Description (138 characters) meets the criteria; the message (328 characters) is over the cap.

**Verdict: cut.** 2 of 26 real, both quoting fixes rather than injection; injection from request data is
`python-security-sql-injection`'s, which covers the same sinks. If a team wants the wider audit, it belongs in
`python-security` in `log`, with the exclusions above (metadata `.name`, calls of run functions that return only
literals, literal list joins, `max`/`min` of casts) added first.

## 7. `sqlalchemy-practice-lazy-load-in-loop`

**Value.** N+1 queries are the most common ORM performance bug, and the loading guide names it. How much a match costs
depends on the relationship's direction: a collection (one-to-many, many-to-many) costs a SELECT per row; a many-to-one
costs one per distinct parent, since the identity map answers repeats (the probe: 1 SELECT for 22 rows).

**Correctness.** Counterexamples (`/tmp/review-sqlalchemy/lazy/app/reports.py`, run `review-sqlalchemy-lazy-load-in-loop-cx`):

| Line | Shape | Expected | Result |
|---|---|---|---|
| 11 | `for line in scalars(select(Line).where(Line.order_id == order_id))`, reads `line.order` | silent: one parent, 1 SELECT (measured) | **reported** |
| 20 | `select(Order).options(*OPTS)`, `OPTS = [selectinload(Order.lines)]` | silent | **reported**: options held in a name are not read |
| 26 | a loop that `return`s at the first order with lines | silent | reported (the message says to leave it) |
| 33 | `for i, o in enumerate(…)` | report | missed (README gap) |
| 42 | `[describe(o) for o in …]`, `describe` reads `o.lines` | report | missed (README gap) |
| 49 | `for o in scalars(select(Order))`, `len(o.lines)` | report | reported |
| 55 | `for o, l in session.execute(select(Order, Line))` | report | missed: row tuples are not followed |

**Corpus.** 21 matches in 2 of the 15 finished repositories, all read, 16 real.

| Repository | Matches | Real | Collections (real) | Many-to-one (real) |
|---|--:|--:|--:|--:|
| Netflix/dispatch | 19 | 14 | 8 (8) | 11 (6) |
| mealie-recipes/mealie | 2 | 2 | 2 (2) | 0 |
| the other 13 | 0 | - | - | - |

Real, collections (10): dispatch `src/dispatch/entity_type/service.py:98` (`signal_instance.entities`, up to 100 rows),
`src/dispatch/feedback/service/scheduled.py:129` and `:135` (`participants` per incident and case),
`src/dispatch/incident/scheduled.py:129` and `src/dispatch/notification/service.py:142` (`notification.filters`),
`src/dispatch/incident_role/service.py:193`, `:195`, `:198` (three collections per role); mealie
`mealie/db/fixes/fix_migration_data.py:86` (`recipe.recipe_ingredient` for every recipe, at startup) and `:110`
(`shopping_list.label_settings`).
Real, many-to-one with a distinct parent per row (6): dispatch `feedback/service/scheduled.py:128` and `:134`
(`commander`, `assignee`), `incident/scheduled.py:278` (`incident_review_document`) and `:301` (`commander`),
`src/dispatch/report/flows.py:116` (`executive_report.document`), `src/dispatch/task/scheduled.py:39` (`t.incident`).
Not real, many-to-one onto a few shared parents or a loop that stops early (5): dispatch `src/dispatch/ai/service.py:558`
(`tag.tag_type`, a handful of tag types), `src/dispatch/cli.py:919` (`project.organization`, one organization per
schema, and only on a warning path), `cli.py:924` (`plugin.plugin`, a few instances), `cli.py:1173` (`i.plugin` in a
loop that breaks at the first match), `src/dispatch/report/scheduled.py:49` (`incident.incident_priority`, a few
priorities). Polar is silent because 276 of its 281 `relationship(` calls in `polar/models/` pass `lazy=`; freqtrade's
`Trade.orders` is `lazy="selectin"` and `custom_data` `lazy="raise"`.

Every false match is a many-to-one; every collection read is real.

**Overlap.** None in Ruff, pylint, Bandit or mypy. SQLAlchemy's answer is opt-in (a `lazy=` strategy per relationship,
as polar declares on 276 of 281, or `raiseload("*")`); nothing warns by default. nplusone and similar profilers find it
at test time.

**Text.** Description (116 characters) meets the criteria. The message (227 characters) is over the cap. Its fix,
`selectinload`, is safe for any relationship (a joined collection would need `.unique()`), but when the rows come from
a shared callee ("the statement get_all() returns") adding the option there changes every caller's query, which the
message should say.

**Verdict: rework.**
- Split by direction: report collection reads (`Mapped[list[...]]`, `uselist=True`, the many side of a `backref`)
  in `steer`, on (10 of 10 real); report many-to-one reads in `log`, or not at all (6 of 11 real). Direction is
  readable from the annotation, `uselist`, or a ForeignKey column on the reading model to the target.
- Leave a many-to-one alone when the statement filters on that relationship's own foreign key (counterexample line 11).
- Read loader options through a name (`.options(*OPTS)`, counterexample line 20).
- Add each as a case; shorten the message and mention the shared-callee trade-off.

## 8. Pack-level findings

**Missing rules.** `NOTES-sqlalchemy.md` ranks nine "build" candidates; five shipped. The four left out are the pack's
gaps, and two are the async half of the stack, which the shipped rules barely touch:

- `session-shared-across-tasks` (one AsyncSession passed to `asyncio.gather`/`create_task`, or a Session to a thread):
  the asyncio docs, the error reference and maintainer answers (discussions #9312, #10687) name it; the failure is
  IllegalStateChangeError or interleaved transactions. The lead's probe found no corpus case; this review did not
  probe further. Docs-strong, corpus-empty: a `log` candidate.
- `lazy-load-in-async` (a lazy relationship read on an AsyncSession's instance: MissingGreenlet at runtime): the
  asyncio docs, "Preventing Implicit IO". Polar passes `lazy=` in 276 of the 281 `relationship(` calls of `polar/models/`, the
  practice this rule would enforce for everyone else. It reuses the library's `sa_lazy_rel` and `sa_rows`, has a louder
  failure than `lazy-load-in-loop`, and the lead ranked it above that rule.
- `session-not-closed` (`SessionLocal()` with no `with`/`close()`): pool exhaustion; the lead's probe counted 8 sites.
- `legacy-query-get`: maintainability only, and SQLAlchemy emits a LegacyAPIWarning; leaving it out was right.

**Rules in the wrong pack.** `text-interpolation` (above): SQL injection is `python-security`'s, which already reports
request data reaching `text()`; if the wider audit is kept, it goes there. The other four belong here; none is
celery's or alembic's.

**Library quality.** `lib/sqlalchemy.rerail` is well built: names resolve through imports, models through bases across
files, Flask-SQLAlchemy's `db.Model`/`db.session`, flask_appbuilder's `Model` and SQLModel `table=True` are handled, and
the interface is documented with examples. Two shared weaknesses:

- `sa_migration_doc` takes any file that imports alembic, so a CLI module that imports `alembic.command` inside one
  function (dispatch's `src/dispatch/cli.py:451`) is skipped whole by bulk-write, connect, session-reused and text.
  `lazy-load-in-loop` already narrows it (`lil_migration_doc`); move that version into the library.
- `sa_dml` needs the first argument to resolve to a model class, so `update(M.__table__)` is a Core write to every rule
  and `update(self.model)` is invisible.

**Cases.** All five pass, and they are thorough: 12 to 20 caught lines each, near misses, cross-file files, test and
migration files that must stay silent. None of the false-positive shapes found here is a case yet; each rework above
should add its shape as a silent case.

**Engine cost.** Runs failed with "content store hard limit rejects new payloads" on whole superset and polar (all five
rules; raised for the lazy-load and text rules) and on dify `api/` with tests (three rules; raised for the connect
rule); runs on polar (four rules) and indico (five rules) were killed at 15 minutes. Runs split by rule over the
application package finished (section 9).

**Can the pack be pitched on its own?** Not yet. Session-reused is the review comment a SQLAlchemy lead makes, and
bulk-write and lazy-load (after their reworks) are close. But on the corpus the pack fires a few times per repository,
the connect rule has never fired, the text rule is mostly noise, and the async footguns a FastAPI + async SQLAlchemy
team meets first (MissingGreenlet, a shared AsyncSession) are missing. With the reworks, the text rule cut and
`lazy-load-in-async` added, it covers the sessions, transactions and loading checklist the pack claims.

## 9. Corpus repositories and commits

The ten repositories of BRIEF.md's corpus whose stack lists SQLAlchemy, SQLModel or Flask, plus six SQLAlchemy
repositories already in the corpus directory (MANIFEST.tsv; five finished). All five rules ran on each, in one run
unless noted. No repository was cloned for this review.

| Repository | Commit | Run over | Run name(s) | Finished |
|---|---|---|---|---|
| fastapi-users/fastapi-users | `e9c57ff7` | whole | `review-sqlalchemy-all-fastapi-users__fastapi-users` | yes |
| fastapi/full-stack-fastapi-template | `77e550a7` | whole | `review-sqlalchemy-all-fastapi__full-stack-fastapi-template` | yes |
| jupyterhub/jupyterhub | `3ca37442` | whole | `review-sqlalchemy-all-jupyterhub__jupyterhub` | yes |
| open-webui/open-webui | `0666e59d` | whole | `review-sqlalchemy-all-open-webui__open-webui` | yes |
| freqtrade/freqtrade | `7fcb509a` | whole | `review-sqlalchemy-all-freqtrade__freqtrade` | yes |
| Netflix/dispatch | `dd2837e8` | whole | `review-sqlalchemy-all-Netflix__dispatch` | yes |
| mealie-recipes/mealie | `8dbb9280` | whole | `review-sqlalchemy-all-mealie-recipes__mealie` | yes |
| apache/superset | `fdf61721` | `superset/` package copied to `/tmp/review-sqlalchemy/superset-root` (1,616 files; tests left out) | `review-sqlalchemy-four-apache__superset` (four rules), `review-sqlalchemy-lazy-apache__superset` | yes (the whole-repository run hit the content-store limit) |
| langgenius/dify | `724ac151` | `api/` copied to `/tmp/review-sqlalchemy/dify-root/api` without `tests`, `migrations`, `providers`, `dev` (2,336 files) | `review-sqlalchemy-sb-…` (session, bulk), `review-sqlalchemy-cl-…` (connect, lazy), `review-sqlalchemy-text-…` | yes (`api/` with tests hit the content-store limit) |
| polarsource/polar | `872272fb` | `server/polar` package copied to `/tmp/review-sqlalchemy/polar-root` (1,162 files): text and session runs; the same without `backoffice/`, `observability/`, `organization_review/` (974 files, `/tmp/review-sqlalchemy/polar-small`) for bulk-write and connect + lazy | `review-sqlalchemy-text-…`, `review-sqlalchemy-session-reused-after-failed-commit-…`, `review-sqlalchemy-bulk-small-…`, `review-sqlalchemy-cl-small-…` | yes, split: the whole-tree and four-rule runs timed out or hit the content-store limit. The three left-out directories hold no model hook and write no hooked model, so the bulk-write result is unaffected; lazy-load may miss loops there |
| flaskbb/flaskbb | `63afaa41` | whole | `review-sqlalchemy-all-flaskbb__flaskbb` | yes |
| Netflix/lemur | `3a3c2c20` | whole | `review-sqlalchemy-all-Netflix__lemur` | yes (SQLAlchemy 1.3.24) |
| letta-ai/letta | `11315357` | whole | `review-sqlalchemy-all-letta-ai__letta` | yes |
| rachelos/we-mp-rss | `8fe5bf93` | whole | `review-sqlalchemy-all-rachelos__we-mp-rss` | yes |
| Buuntu/fastapi-react | `af17f2d4` | whole | `review-sqlalchemy-all-Buuntu__fastapi-react` | yes |
| indico/indico | `8e641393` | whole | `review-sqlalchemy-all-indico__indico` | no: killed at 15 minutes; not counted |

The superset and polar copies keep the package directory as the run root so imports (`superset.models…`,
`polar.models…`) resolve; a first attempt run from inside the package (`…/superset/superset`, `…/server/polar`) broke
module resolution and its zero results were discarded.

Differences from the README's own numbers: bulk-write 12 here against 13 there (superset
`superset/commands/theme/delete.py`, nulling dashboards' `theme_id`, is not reported by the merged rule on the
`superset/` tree), and lazy-load 21 against 22 (superset's one loop is not reported here). Session-reused matches the
README exactly (16). Connect-without-commit agrees (0). The README has no text-interpolation numbers
(`CORPUS_PLACEHOLDER`).

## Applied

Applied by the pack's fixer; the review above is unchanged. Corpus runs are named `sqlalchemy-fix-*`.

- **Library** (`rules/lib/sqlalchemy.rerail`): `sa_migration_doc` narrowed to Alembic revisions and env.py (a CLI
  importing `alembic.command` is checked); `sa_dml` reads its target through `sa_model_expr` (the class,
  `M.__table__`, `self.model` bound by the class or a subclass); `sa_dml_value` follows an executed statement through
  names and the return values of functions of the run; `sa_rel_dir` gives a relationship's direction (secondary,
  annotation, `uselist`, the foreign-key side, backrefs; table names from `__tablename__` or snake case);
  `sa_session_like` also takes a parameter a caller of the run fills with a session; `sa_instance` reads annotated or
  assigned `self.<a>`; the superseded `sa_loop_rows` is gone. New `rules/lib/sqlalchemy_loops.rerail` holds the
  loop/row/read machinery both lazy-load rules share (enumerate and multi-entity rows, reads through a helper the
  row is handed to, `.options(*NAME)`, rebound parameters).
- **`session-reused-after-failed-commit`**: kept, steer, on. Message shortened (148 characters). An unannotated
  parameter whose callers pass a session is now a session (counterexample line 29's shape, with a caller); cases
  added. Re-measured on dispatch (2 of 2 real), open-webui (1 of 1), mealie, fastapi-users, template, jupyterhub (0);
  dify and superset not re-run (1,000+ files; review's 13 of 13 kept).
- **`bulk-write-skips-hooks`**: rework done. A listener whose only effect is assigning target columns the write sets
  is left alone; a target-free mapper listener is left alone when the same function later changes, adds or deletes
  an instance for an operation it is registered for; `update(M.__table__)`, `update(self.model)` and statements built
  by helpers are writes. Cases for each. Corpus: 9 matches, 6 real (67%: superset 4/3, mealie 4/3, polar 1/0), so
  log, off. Message shortened (139).
- **`connect-without-commit`**: rework done. Message leads with `engine.begin()` and offers `commit()` only where
  connections have it (197); description without a version; engine autocommit read through factory functions'
  return values; CTE-led and CTE-contained writes are writes. Cases for each, the review's gaps as near misses.
  Still 0 corpus matches; log, on, as proposed.
- **`text-interpolation`**: cut (rule, cases, pack.json entry, README section, dimensions entry).
- **`lazy-load-in-loop`**: split by direction. This id now reports collections only (steer, on): dispatch 8/8 and
  mealie 2/2 (100%; a helper that rebinds its parameter is no longer read through). New
  `sqlalchemy-practice-lazy-many-to-one-in-loop` (log, off) reports single-object and undecidable reads and leaves
  alone statements filtering on the relationship's own foreign key with `==`: dispatch 17, 12 real (71%). Both read
  `.options(*OPTS)`, enumerate, multi-entity rows and helper
  reads; messages shortened, the shared-callee trade-off has its own message.
- **README and pack.json**: rules in the review's reading order, modes and `on` as above, no placeholders, corpus
  tables re-measured on every repository under 1,000 Python files (larger ones marked not re-run).
