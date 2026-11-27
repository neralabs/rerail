# Celery pack: review

Reviewer's audit of the six rules in `packs/celery/` (pack.json, rules, cases under
`upstream/celery-practice/authored/<id>/`, README "Rule details", `NOTES-celery.md`). Every number below was measured
for this review; none is copied from the README. Corpus runs used
`~/rerail-bench/python-arch/bin/corpus-run` with all six rules at once per repository (run names
`review-celery-all-<repo>`), counterexample runs used run names `review-celery-<rule>-cx[-<dir>]`, and every scratch
repository is under `/tmp/review-celery/`. The machine ran at a load average of 40 to 65 during the runs (other
reviewers' corpus runs), which matters for the one repository that hit the 15-minute limit (section 4).

## 1. Summary

| Rule | Verdict | Proposed mode / on | Corpus matches (repos) | Read precision | Reason |
|---|---|---|---|---|---|
| `celery-practice-dispatch-before-commit` | rework | steer, on | 3 (2) | 3/3 (100%, small sample) | Real, cross-file, no linter has it; reports a send on a branch that never wrote (exclusive `if`/`else`), counts RQ's and any imported object's `.delay` as Celery, message 273 chars. |
| `celery-practice-pickle-serializer` | rework (text only) | steer, on | 9 (2) | 9/9 (100%) | Precise; description names config keys and the message is 293 chars with unrelated advice ("pass ids, not objects"). |
| `celery-practice-countdown-past-visibility-timeout` | rework | steer, on | 2 (2) | 2/2 (100%) | Silent on RabbitMQ, where Celery's docs say a countdown past `consumer_timeout` (30 min) kills the worker's channel: misses lookit-api's 7-day countdown and addons-server's 2-hour backoff cap. Message omits the cost of raising `visibility_timeout`. |
| `celery-practice-result-get-in-task` | rework | steer, on | 0 (0) | n/a (no match) | Value is real (Celery raises in the worker), but `ThreadPool.apply_async(...).get()` in a task is reported as a Celery wait, and the message offers `disable_sync_subtasks=False` as the way out, which an agent will take to silence it. |
| `celery-practice-task-arg-not-serializable` | rework | steer, on (after the fix) | 0 (0) | n/a (no match) | A string or dict named `session` is reported as a SQLAlchemy session (3 of 3 counterexamples), and RQ's `@job(...).delay(set(...))` (pickle) is reported as a Celery send. `"on": true` in pack.json rests on no corpus match. |
| `celery-practice-unknown-task-name` | rework | log, off until reworked | 0 (0) | n/a (no match) | 4 of 4 false-positive counterexamples reported (factory-built app, class name from a constant, another service's `tasks.x`, a non-beat dict with `"options"`); its steer tells the agent to rename a string that may be right. |

Verdicts: keep 0, rework 6 (one of them text only), cut 0.

Reading order (by what a team loses when the rule is missing, not pack.json's alphabetical order):

1. `dispatch-before-commit` (data safety, critical; the pack's one cross-layer rule).
2. `pickle-serializer` (security, critical; config-level, near-zero noise).
3. `countdown-past-visibility-timeout` (data safety, duplicate execution).
4. `result-get-in-task` (availability; Celery already fails it at run time, the rule moves it to edit time).
5. `task-arg-not-serializable` (fails loudly at the first send under JSON; the silent case is pickle + model).
6. `unknown-task-name` (silent never-running beat job, but no real occurrence found and the most false-positive shapes).

## 2. Rules

### 2.1 `celery-practice-dispatch-before-commit`

**Value.** The Celery tasks guide ("Database transactions") states the bug and the fix; Adam Johnson's "Common Issues
Using Celery" and BaoDev's "flush vs commit" post describe the same race; all three URLs return 200 and contain the
quoted text (checked). It is the SQLAlchemy twin of `django-practice-enqueue-on-commit`, which a Django team already
has; for Flask/FastAPI + SQLAlchemy + Celery code nothing else catches it. A lead flags this every time.

**Logic read.** Four shapes: (1) write, send, commit in one function by statement order; (2) send after a write inside
`with x.begin():`; (3) a non-committing writer-sender whose caller commits after the call or calls it in a begin
block; (4) a send in a flush-time mapper/session listener. Sends are `lib/celery`'s `cel_send`: `.delay`/`.apply_async`
on `cel_task_ref` (any name that resolves through an import and is not a plain undecorated def) or any `.send_task`.
Sessions are `sa_session`: any identifier or attribute whose name contains "session". Commit and write are matched by
the session's spelling. Case 3 does not check that the caller's commit is on the session the callee wrote to.

**Counterexamples** (`/tmp/review-celery/dbc`, run `review-celery-dispatch-before-commit-cx`, 6 findings in 7 files):

| Shape | Expected | Result |
|---|---|---|
| `if is_new: session.add(o)` / `else: ship.delay(o.id)` / `session.commit()` (`shop/orders.py:15`) | silent (the send path wrote nothing) | **reported: false positive** |
| `http_session.delete(url)` then `audit.delay(...)` then `http_session.commit()` (`orders.py:24`; contrived) | silent | reported (name-based session) |
| Attribute write on a loaded row: `order.status = "paid"; ship.delay(...); session.commit()` | reported | **missed** (documented gap; the most common real shape) |
| Loop: `for o in orders: ship.delay(o.id); session.add(o)` then commit | reported | **missed** (documented gap) |
| `self.session.add(o); self.notify(o); self.session.commit()` with `notify` sending | reported | reported (`orders.py:54`) |
| `try: add; flush; ship.delay() finally: session.commit()` | reported | reported (`orders.py:63`) |
| RQ: `@job("default") def reindex`, `reindex.delay(...)` between add and commit | arguably reported, but not as Celery | reported, message says "a Celery worker" (`orders.py:71`) |
| Module-local `POOL = ThreadPool(2)`; `POOL.apply_async(...)` between add and commit | silent | silent |
| Flask-SQLAlchemy: `create()` adds and sends, `view()` calls it then `db.session.commit()` | reported | reported (`flaskapp.py:11`) |

The `.delay` misattribution follows from `cel_task_ref`: any name that resolves through an import and is not an
undecorated function counts as a task (a decorated function of another library, a module-level object, a class). In
`result-get-in-task`'s counterexamples an *imported* `ThreadPool` instance's `apply_async` counted as a Celery send
(2.4), so the same object between a write and a commit here would be reported too.

**Corpus.** 3 matches in 2 of 20 repositories, all read, all real (3/3):

- langgenius/dify `api/services/knowledge/dataset_service.py:755` and `:758` (run `review-celery-dbc-langgenius__dify`,
  this rule alone, after the six-rule run hit the 15-minute limit): `deal_dataset_vector_index_task.delay(dataset.id,
  action)` and `regenerate_summary_index_task.delay(...)` after `session.execute(update(Dataset)…)` and
  `session.flush()`; the code's own comment says the commit is left to the caller's `with Session(...)` block. Both
  tasks load the dataset the update changed.
- apache/superset `superset/models/slice.py:656`: `cache_chart_thumbnail.delay(...)` in `event_after_chart_changed`,
  registered for `after_insert`/`after_update` on `Slice` (behind the `THUMBNAILS_SQLA_LISTENERS` flag). On insert the
  task can find no chart and skip; on update it can render the old chart.

The other 18 repositories gave 0. 13 are Django projects whose documents import django (left to the django pack by
design). The 5 Flask/FastAPI + SQLAlchemy ones: a text search of lemur, flaskbb, zenodo-rdm and fastapi-react for
`.delay(`/`.apply_async(` outside tests found 10 sends; the one I read in full, lemur `certificates/service.py:574`,
follows `database.commit()` (correctly silent). For indico the README says its sends follow attribute writes the
request handler commits (a documented gap); I did not re-check that by hand. Three real matches is too few to measure
precision; the counterexamples carry the false-positive evidence.

**Overlap.** None. Ruff (full rule list checked: no rule names Celery, `apply_async` or `send_task`), Bandit, pylint
(pylint-celery only suppresses a false positive), mypy and SQLAlchemy's own warnings say nothing about dispatch inside
a transaction. The django pack's three dispatch rules read Django's ORM only; this rule skips documents that import
django, so the two never report the same line.

**Text.** Description (150 chars) is one imperative sentence, accurate. The message is 273 characters (criterion 6:
200), and "leave it if the task reads nothing this transaction writes" is the right escape. For a non-Celery `.delay`
it still says "Celery worker".

**Verdict: rework.** (a) Require the write to reach the send on one path: replace statement order (`earlier`) by the
flow library's dominance or path relation, so a write in one branch and a send in its sibling branch stay silent.
(b) Narrow `cel_task_ref` in `lib/celery` to names that resolve to a `cel_task` def or to a Celery signature/task
object, not every imported decorated name (it also fixes 2.4 and 2.5). (c) Shorten the message under 200 characters,
for example: "`{what}` runs {where}: the worker can start before the commit and miss the row. Send it after `commit()` or
from an `after_commit` event; leave it if the task reads nothing written here." Mode steer, on.

### 2.2 `celery-practice-pickle-serializer`

**Value.** Celery's security guide ("pickle is inherently insecure"), the 4.0 "What's new" (JSON default) and the
configuration reference (`accept_content` default `{'json'}`); checked. A worker that accepts pickle executes any
message on the broker. Real, and the fix is a configuration line.

**Logic read.** Reads serializer and accept-content settings in every `cel_setting` spelling, a task's
`serializer=`, a Task subclass's `serializer` attribute and `serializer=` on any `.apply_async`/`.send_task` call; a
run calling `setup_security()` outside tests is silent; test files and `test*.py`/`*_test*.py` are skipped.

**Counterexamples** (`/tmp/review-celery/pickle`, run `review-celery-pickle-serializer-cx`, 2 findings in 8 files):

| Shape | Expected | Result |
|---|---|---|
| `app.config_from_object({"task_serializer": "pickle", "accept_content": ["pickle"]})` | reported | **missed** (documented gap) |
| `app.conf.task_serializer = os.environ.get("CELERY_TASK_SERIALIZER", "pickle")` | reported | **missed** (documented gap) |
| `app.conf.result_serializer = SERIALIZER`, imported from `settings/serializers.py` | reported | **missed** (documented gap) |
| `app.conf.update(**{"accept_content": ["pickle", "json"]})` | reported | **missed** (not documented) |
| `CELERY_ACCEPT_CONTENT = ["json", "pickle"]` in `settings/dev.py` | debatable (dev broker) | reported (`settings/dev.py:2`) |
| kombu `Producer(conn).publish(body, serializer="pickle")` | silent | silent |
| `sig.set(serializer="pickle").apply_async()` | reported | **missed** (documented gap) |
| `pool.apply_async(fn, serializer="pickle")` on a `multiprocessing.Pool` parameter | silent | reported (`other.py:6`; contrived: Pool has no such keyword, but any `.apply_async` counts) |

The misses are spellings one more value-resolution pass would close (imported constants, environment defaults, dict
arguments to `config_from_object`/`update`). The one false positive needs a non-Celery `apply_async` with a
`serializer` keyword, which no library in the corpus has.

**Corpus.** 9 matches in 2 of 20 repositories, all read, all real:

- taigaio/taiga-back `settings/common.py:53`, `:54`, `:55` and `docker/config.py:112`, `:113`, `:114`
  (`CELERY_ACCEPT_CONTENT = ['pickle', ]`, `CELERY_TASK_SERIALIZER = "pickle"`, `CELERY_RESULT_SERIALIZER = "pickle"`,
  no `setup_security()`; the Docker config is the deployed one).
- indico/indico `indico/core/celery/core.py:66`, `:67`, `:68`: `self.conf['result_serializer'] = 'pickle'`,
  `task_serializer`, `accept_content = ['json', 'yaml', 'pickle']`, with the comment "Pickle isn't pretty but that way
  we can pass along all types (tz-aware datetimes, sets, etc.)". A deliberate choice, still the exposure the security
  guide names; the README's corpus list does not include indico for this rule.

paperless-ngx's `"signed-pickle"` stays silent, as intended. Precision 9/9. Both repositories pass objects to tasks by
design, so the fix is not one line for them; the message's `setup_security()` alternative is the realistic one.

**Overlap.** `bandit-b301-pickle` / Ruff S301 flag `pickle.load(s)` calls, and B403/S403 the import; neither reads
Celery's configuration. Nothing else does.

**Text.** Description (151 chars) names configuration keys (`task_serializer`, `accept_content`: criterion 3) and
reads as two instructions. Suggested: "Keep Celery off pickle for task messages and results; a worker that accepts
pickle runs code from any message on the broker." The message is 293 characters; "pass ids, not objects" is advice
for another rule. Suggested: "`{k}` enables pickle, so anyone who can write to the broker runs code in the worker.
Use json; if pickle must stay, sign messages with `app.setup_security()`." (172 chars.)

**Verdict: rework, text only.** Logic as is; steer, on.

### 2.3 `celery-practice-countdown-past-visibility-timeout`

**Value.** Celery's Redis caveats ("executed again, and again in a loop"), the SQS caveats and the calling guide;
checked. The calling guide also carries a warning the rule contradicts: "When using RabbitMQ as a message broker when
specifying a countdown over 15 minutes, you may encounter the problem that the worker terminates with an
PreconditionFailed error … since version 3.8.17 [the default consumer_timeout] was increased to 30 minutes"
(https://docs.celeryq.dev/en/stable/userguide/calling.html#eta-and-countdown). The rule treats a RabbitMQ run as safe
and goes silent.

**Logic read.** Delays from `countdown=`, `eta=now + timedelta`, `default_retry_delay` when used, and
`retry_backoff_max` when reached; constant folding of literals, constants, `timedelta`, `min`/`max`. Threshold: the
run's largest literal `visibility_timeout`, else 3600; no threshold (silent) when the value is unreadable or the
broker is RabbitMQ.

**Counterexamples** (`/tmp/review-celery/cv/{rabbit,envbroker,sqs,misc}`, runs
`review-celery-countdown-past-visibility-timeout-cx-<dir>`):

| Shape | Expected | Result |
|---|---|---|
| `Celery(broker="amqp://…")`, `countdown=2 * 60 * 60` | reported (consumer_timeout 30 min) | **missed** (0 findings) |
| `Celery(broker=os.environ["CELERY_BROKER_URL"])`, `countdown=7200` | reported | reported (`envbroker/proj/tasks.py:10`) |
| `Celery(broker="sqs://")`, `countdown=45 * 60` | reported (SQS default 30 min) | **missed** (documented gap) |
| `eta=timezone.now() + timedelta(hours=3)` | reported | reported (`misc/proj/tasks.py:23`) |
| `later = timezone.now() + timedelta(days=1)`; `eta=later` | reported | **missed** (eta read only as a literal sum) |
| `countdown=timedelta(hours=2).total_seconds()` | reported | reported (`misc/proj/tasks.py:26`) |
| `remind.s(uid).set(countdown=7200).delay()` | reported | **missed** (documented gap) |
| `self.retry(countdown=30 * 2 ** self.request.retries)` with `max_retries=None` | reported | **missed** (documented gap) |
| `countdown=3000`, `expires=7200` | silent | silent |

**Corpus.** 2 matches in 2 of 20 repositories, both read, both real:

- ecds/readux `apps/export/tasks.py:86`: `delete_download_task.apply_async((zipfile_name,), countdown=86400)`; no
  broker or `visibility_timeout` configured anywhere in the repository's Python, so whatever broker is deployed, a
  one-day hold breaks (Redis redelivers after an hour; RabbitMQ closes the channel after 30 minutes).
- breatheco-de/apiv2 `breathecode/authenticate/receivers.py:354`: `countdown=7200` on a Copilot-revocation task;
  Redis when `CLOUDAMQP_URL` is unset, with `broker_transport_options` setting no `visibility_timeout`.

Missed on the corpus because of the RabbitMQ silence (read by hand):

- lookit/lookit-api `studies/models.py:1950`: `delete_video_from_cloud.apply_async(..., countdown=60 * 60 * 24 * 7)`,
  broker `os.environ.get("BROKER_URL", "amqp://…")`; no `consumer_timeout` anywhere in the repository.
- mozilla/addons-server `src/olympia/abuse/tasks.py:86`: `retry_backoff=30`, `retry_backoff_max=2 * 60 * 60`,
  `max_retries=60`: the backoff reaches the 2-hour cap; broker default `amqp://…` (`lib/settings_base.py:687`); no
  `consumer_timeout` in the repository.

Both are the hazard the Celery docs describe for RabbitMQ. A team may have raised `consumer_timeout` in
`rabbitmq.conf`, which lives outside the repository, so those reports need their own message (below).

**Overlap.** None: no linter reads Celery delays or broker settings.

**Text.** Description (174 chars) accurate for what it reports. The message (313 chars) says "Redis and SQS" for a
broker the rule could not identify, and offers "raise visibility_timeout above it" without its cost: a larger
visibility timeout delays redelivery of every task a crashed worker held by the same amount (Redis caveats).

**Verdict: rework.** (a) For a RabbitMQ run, report delays past 1800 s (RabbitMQ's default `consumer_timeout` since
3.8.17) with a RabbitMQ message: "This task is held unacknowledged {secs} s, past RabbitMQ's default consumer_timeout
(1800 s): the broker closes the worker's channel. Keep the delay short and schedule far-off work from the database, or
raise consumer_timeout in rabbitmq.conf." (b) Use 1800 s for an `sqs://` broker. (c) Follow `eta=<name>` bound to now
plus a timedelta. (d) Name the trade-off in the Redis message ("raising visibility_timeout also delays recovery of
tasks from a crashed worker") and bring both under 200 characters. Steer, on.

### 2.4 `celery-practice-result-get-in-task`

**Value.** Tasks guide, "Avoid launching synchronous subtasks" ("may even cause a deadlock if the worker pool is
exhausted"); `celery/result.py`'s `assert_will_not_block()` raises `RuntimeError("Never call result.get() within a
task!")`. Checked. The bug is real but Celery already fails it at run time on the first call in a worker; the rule's
value is moving that to edit time, and catching the helper reached from a task across files.

**Logic read.** Waits (`get`, `join`, `join_native`, `wait`, `collect`) on a result expression (sends, `AsyncResult`
family, canvas sends, functions returning one, names last bound to one) inside a task or a function the task reaches.
Opt-outs: `disable_sync_subtasks=` not `True`, `allow_join_result()`, test files.

**Counterexamples** (`/tmp/review-celery/rget`, run `review-celery-result-get-in-task-cx`, 4 findings in 5 files):

| Shape | Expected | Result |
|---|---|---|
| `POOL.apply_async(compute, (i,)).get()` in a task, `POOL = ThreadPool(4)` imported from `proj/pools.py` | silent (not Celery) | **reported: false positive** (`tasks.py:19`, "blocks … on another task") |
| `results = [price.delay(i) for i in ids]; [r.get() for r in results]` | reported | **missed** (documented gap; the shape the Celery guide's fan-out readers write) |
| `pending = {i: price.delay(i) …}; pending[i].get()` | reported | **missed** |
| `r = AsyncResult(task_id); if r.ready(): return r.get()` | reported (raises in the worker) | reported; message says it "blocks", it raises instead |
| `child = price.apply_async((i,)); child.get(timeout=10)` | reported | reported (`tasks.py:49`) |
| `price.delay(i).get(propagate=False)` | reported | reported (`tasks.py:55`) |
| A web view's `price.delay(i).get(timeout=5)` | silent | silent |

**Corpus.** 0 matches in 20 repositories (dify through run `review-celery-rg-dify-api`, this rule alone on `api/`).
A text search over 10 Celery repositories (pretix, addons-server, DefectDojo, saleor,
superset, dify, indico, paperless-ngx, plane, apiv2) for `AsyncResult(`, `.delay(…).get(`,
`apply_async(…).get(`, `allow_join_result` and `disable_sync_subtasks` outside tests found one task-side wait,
DefectDojo `dojo/celery_dispatch.py:141` (`disable_sync_subtasks=False`, deliberately left alone) and a view polling
`AsyncResult` in indico (no task reaches it). So the rule has no corpus occurrence to measure, and the cases carry it.

**Overlap.** None. Celery's run-time guard is the only other check.

**Text.** Description (156 chars) fine. Message (193 chars) is within the limit but ends "keep a deliberate wait only
with disable_sync_subtasks=False": an agent told to fix or justify will add that argument, which removes Celery's own
guard and leaves the deadlock (Celery's API reference: "CAUTION do not enable this unless you must"). Steer safety
fails. Suggested: "`.{m}()` waits {where} on another task: Celery raises in the worker, and the pool can deadlock.
Chain the next step as a callback (chain or chord) instead of waiting."

**Verdict: rework.** (a) Narrow `cel_task_ref` (shared with 2.1, 2.5) so a non-Celery `apply_async`/`delay` is not a
result. (b) Drop the `disable_sync_subtasks=False` hint from the message. (c) Optional: read results kept in a list
or dict comprehension of sends, the common fan-out shape. Steer, on.

### 2.5 `celery-practice-task-arg-not-serializable`

**Value.** Tasks guide "State" (don't pass model objects; checked), Bertović No. 7, kombu `utils/json.py` (checked: the
encoder registers datetime, date, time, Decimal, UUID and handles bytes and `__json__`; a set falls through to
`json.JSONEncoder.default` and raises). Under JSON (the default) every reported shape raises EncodeError at the send,
the first time the line runs: a loud failure a test catches. The silent case, a stale model under pickle, is the
valuable one and is rare: the corpus has two pickle repositories, taiga-back (Django: the django pack's rule) and
indico (SQLAlchemy, whose `_CelerySAWrapper` turns models into identity keys; the rule exempts it). Value: medium.

**Logic read.** Arguments of task calls/sends that are a SQLAlchemy model instance, a session (names `session`,
`db_session`, `sa_session`, `dbsession`, `db_sess`, `db.session`, or a parameter annotated Session), a set, an open
file, a lambda/generator, or a run function by name. Pickled runs exempt sets and functions; Django documents exempt
the model shape (the django pack owns it).

**Counterexamples** (`/tmp/review-celery/targ`, run `review-celery-task-arg-not-serializable-cx`, 6 findings in 6
files):

| Shape | Expected | Result |
|---|---|---|
| `session = request.json["session"]; resume_chat.delay(session)` (a chat-session id) | silent | **reported: false positive** (`views.py:13`) |
| `session = str(uuid.uuid4()); resume_chat.delay(session)` | silent | **reported: false positive** (`views.py:15`) |
| `session = {"user": 1, "turns": []}; resume_chat.delay(session)` | silent | **reported: false positive** (`views.py:20`) |
| RQ `@job("default") def rq_tag`, `rq_tag.delay(set(tags))` (RQ pickles) | silent | **reported: false positive**, "the Celery task `rq_tag`" (`views.py:25`) |
| `notify.delay([user])` (model in a list) | reported | missed (documented gap) |
| `notify.delay(current_user)` (Flask-Login proxy of a model) | reported | missed |
| `notify.delay(**{"user": user})`, `notify.apply_async(kwargs=dict(user=user))` | reported | missed |
| `tag.delay(tags \| {"x"})` | reported | missed |
| `user = db.session.get(User, uid); notify.delay(user)` | reported | reported (`views.py:35`) |
| `def typed(db, user: User): notify.delay(user)` | reported | reported (`views.py:39`) |

The session shape reads only the name: its "last assignment" check, used for sets and files, is not applied to
sessions, so any local called `session` is a SQLAlchemy session.

**Corpus.** 0 matches in 20 repositories (dify through run `review-celery-ta-dify-api`, this rule alone on `api/`).
A text search for a task call with a `session` argument across 11 Celery repositories found
only `send_mentorship_session_survey.delay(session.id)` (apiv2, an attribute, correctly silent).

**Overlap.** The model shape is the SQLAlchemy twin of `django-practice-task-arg-model-instance` (django pack); the
two never fire on one file. mypy with celery-types accepts any argument; nothing checks serializability. Celery's own
EncodeError at the call is the run-time equivalent.

**Text.** Description (154 chars) fine. All five messages exceed 200 characters (290, 233, 220, 220, 230).
`TaskArgSession` says "SQLAlchemy session" for a Django `request.session` too (not gated on Django documents).

**Verdict: rework.** (a) Session shape: require evidence that the value is a session (annotated, `db.session`, bound
from `Session(...)`/`sessionmaker()()`/`SessionLocal()`/`get_session()`-style calls, or a `with … as session` from
one), and keep the bare-name fallback only when the name has no local assignment. (b) Narrow `cel_task_ref` so RQ's
and other libraries' `.delay` are not Celery sends. (c) Messages under 200 characters. (d) pack.json sets `"on": true`
explicitly with no corpus match behind it; leave it on only after (a) and (b), since the remaining shapes are all
deterministic EncodeErrors. Steer.

### 2.6 `celery-practice-unknown-task-name`

**Value.** Celery "Names" and the consumer's "Received unregistered task … ignored and discarded" (checked; the Stack
Overflow pages return 403 to a script but are well-known). A typo in a beat entry is a silent never-running job: real
when it happens, but the corpus has none (the README's hand check of 8 repositories agrees), and `celery inspect
registered` or the worker log surface it on first run.

**Logic read.** Task names at `send_task`, `signature`, and beat dicts (a `"task"` key plus `"schedule"` *or*
`"options"`), compared with registered names (explicit `name=`, implicit `<module>.<def>` with dotted-tail match,
Task subclasses' `name` attribute). Reported when the run "owns" the name: its module part is a module of the run, or
any document's module name *ends with* `.` + that part, or an explicit task name shares its first segment.

**Counterexamples** (`/tmp/review-celery/uk`, run `review-celery-unknown-task-name-cx`, 5 findings in 9 files):

| Shape | Expected | Result |
|---|---|---|
| `cel = make_celery()` (factory in another module); `@cel.task def ping`; beat `"proj.worker.ping"` | silent | **reported: false positive** (`celeryconfig.py:5`): `cel_app_ref` knows only module-level `Celery(...)` and the names `app`/`celery`/`celery_app` |
| `class Cleanup(Task): name = TASK_NAME` (a module constant); beat `"proj.cleanup"` | silent | **reported: false positive** (`celeryconfig.py:7`): the class `name` attribute is read only as a literal |
| `remote = Celery(broker="redis://other")`; `remote.send_task("tasks.process_upload")` while the run has `app/tasks.py` | silent (another service) | **reported: false positive** (`client.py:8`): `app.tasks` ends with `.tasks`, so the run "owns" `tasks.*` |
| `ML_JOBS = [{"task": "metrics.weekly", "options": {"window": 7}}]` with an `app/metrics.py` | silent (not a beat entry) | **reported: false positive** (`client.py:13`) |
| beat `"app.tasks.local_jbo"` (typo) | reported | reported (`celeryconfig.py:9`) |
| `beat_schedule["other"] = dict(task="app.tasks.missing", schedule=30.0)` | reported | missed |

**Corpus.** 0 matches in 20 repositories (dify through run `review-celery-a-dify-api`, this rule with countdown and
pickle, on `api/`, 4,921 of dify's 5,191 files). No false positive, and no real unknown name to find: the README's
hand check of the beat entries and string sends in 8 of these repositories found every name registered, and I did not
redo it. The rule's precision on real code is unmeasured; the counterexamples above are the evidence.

**Overlap.** None at edit time. Celery's worker log at run time.

**Text.** Description (163 chars) says "a task of the run", engine vocabulary a reader won't know; suggested: "Name
Celery tasks by strings that match a registered task; a worker discards messages for unknown names, so a mistyped
beat entry never runs." Message 271 characters (plus the moved-task hint). Its fix, "Use the task's registered name",
is the dangerous part: on each false-positive shape above the string is right, and an agent following the steer would
change it to a wrong one (for the factory-app case it would point at nothing at all). Steer safety fails until the
false-positive shapes are closed.

**Verdict: rework; start in log, off.** (a) Recognise a task decorator `@<x>.task` on any `x` whose `.task` is called
as a decorator in a module that imports celery, or bound from a function returning `Celery(...)`. (b) Resolve a Task
subclass's `name = CONST`. (c) Own a name only by an exact module of the run (drop the `endswith` clause) or an
explicit-name first segment. (d) Require `"schedule"` for a beat entry, or `"options"` only inside a dict assigned to
`beat_schedule`/`CELERY_BEAT_SCHEDULE`. After that, with no corpus match to measure, it can move to steer only on the
cases' evidence; until then log, off.

## 3. Pack-level findings

**Missing rules.** NOTES-celery.md §3 ranks ten rules to build; four were not built and nothing records why:

- `retry-swallowed` (ranked 7): `except Exception:` around `self.retry()`, or `raise self.retry()` inside an
  `autoretry_for=(Exception,)` task, defeats the retry accounting (Celery issue #4898, checked: such a task ran 2047
  times instead of 11, because `Retry` is an `Exception`). 25 files in 10 Celery repositories (the base corpus's 8,
  taiga-back and indico) call `self.retry(`; nothing in Ruff/Bandit ties the broad except to the retry. Single-file,
  cheap, high value: the strongest gap.
- `retry-non-idempotent` (8): `acks_late=True` tasks that send mail or webhooks; pretix has nine such decorators
  (`base/services/mail.py`, `api/webhooks.py` ×3, `plugins/sendmail/tasks.py` ×2, `base/services/notifications.py` ×2,
  `base/services/tickets.py`); dify's archive deletion task (`acks_late=True`, `autoretry_for=(Exception,)`) carries a
  docstring defending its idempotency. Expect log mode.
- `network-task-without-time-limit` (9): expect log mode; `bandit-b113` and `python-practice-requests-session-without-timeout`
  already cover requests calls without a timeout, so the rule's own value is the task-level `time_limit`.
- `async-def-task` (10): no corpus occurrence (text search for `async def` under a task decorator in 10 Celery
  repositories found none). Cheap, exact, low frequency.

`retry-swallowed` should be built before the pack is pitched; the other three can stay candidates with the reason
recorded in NOTES.

**Placement.** All six belong here. `dispatch-before-commit` spans Celery and SQLAlchemy; the brief puts it in celery,
and NOTES says so. No rule duplicates a django, sqlalchemy, python-practice or python-security rule
(`python-practice-retry-without-limit` exempts Celery's `self.retry`; `python-security-code-injection` mentions
Celery results only as trusted pickle).

**Library (`rules/lib/celery.rerail`).** Clean interface comment and good name resolution, but three heuristics leak
into every rule:

- `cel_task_ref`: any name that resolves through an import and is not an undecorated def. RQ's `@job`, a module-level
  `ThreadPool`, or any decorated callable with `.delay`/`.apply_async` becomes a Celery task (false positives in 2.1,
  2.4, 2.5). Fix once here: a `cel_task` def, a Celery signature, or a Task instance.
- `sa_session`: any name containing "session" (`http_session`, `chat_session`). `task-arg` narrows it locally;
  `dispatch-before-commit` does not.
- `cel_app_ref`: the names `app`, `celery`, `celery_app`, or a module-level `Celery(...)`; a factory-built app under
  another name is not an app (2.6).

**Cases.** Each rule's `.test` passes (`node dist/cli/cli.js rules test …`, all six). The cases are thorough for the
shapes the rules intend; none of them includes the false-positive shapes above (exclusive branches, an imported
thread pool, a string named `session`, RQ, a factory app).

**Can the pack be pitched alone?** Not yet. Over 20 Celery repositories the six rules reported 14 lines, 9 of
them two repositories' deliberate pickle settings; no rule found anything in paperless-ngx, plane, pretix,
addons-server, DefectDojo or saleor. For a Django + Celery team the django pack already owns dispatch and model
arguments, leaving pickle, countdown and result-get. For a Flask/FastAPI + SQLAlchemy + Celery team it is worth
installing for `dispatch-before-commit` (dify's and superset's findings are the pack's best evidence) and
`pickle-serializer`. Pitch it with the sqlalchemy pack, after the rework above and `retry-swallowed`.

## 4. Corpus

Commits from `~/rerail-bench/python-arch/corpus/MANIFEST.tsv`. No repository was cloned for this review. Used: the base
corpus's 8 Celery repositories, and the 12 other corpus repositories that use Celery (cloned earlier by the pack's
writers or other workers). Each ran all six rules at once (`review-celery-all-<repo>`) except dify, whose six-rule run
and a five-rule rerun hit the 15-minute limit under a load average near 40; it was rerun per rule group, as listed.

| Repository | Commit | .py files | Run | Findings |
|---|---|--:|---|--:|
| paperless-ngx/paperless-ngx | `faa6f09` | 424 | all six | 0 |
| makeplane/plane | `723bb17` | 784 | all six | 0 |
| pretix/pretix | `40ea909` | 1,228 | all six | 0 |
| mozilla/addons-server | `7e3374f` | 1,426 | all six | 0 |
| DefectDojo/django-DefectDojo | `cd0a448` | 2,601 | all six | 0 |
| saleor/saleor | `e1ccf4d` | 4,385 | all six | 0 |
| apache/superset | `fdf6172` | 3,191 | all six | 1 (dispatch) |
| langgenius/dify | `724ac15` | 5,191 | dispatch alone, whole repository (`review-celery-dbc-langgenius__dify`); countdown + pickle + unknown on `api/` (`review-celery-a-dify-api`); result-get alone and task-arg alone on `api/` (`review-celery-rg-dify-api`, `review-celery-ta-dify-api`); `api/` holds 4,921 of the 5,191 files and every Celery task | 2 (dispatch) |
| taigaio/taiga-back | `8ab6a4e` | 933 | all six | 6 (pickle) |
| indico/indico | `8e64139` | 1,301 | all six | 3 (pickle) |
| ecds/readux | `fcf0d36` | 374 | all six | 1 (countdown) |
| breatheco-de/apiv2 | `2055837` | 2,164 | all six | 1 (countdown) |
| lookit/lookit-api | `78562b5` | 329 | all six | 0 |
| UB-Mannheim/escriptorium | `7e06bd3` | 260 | all six | 0 |
| andreynovikov/django-shop | `2b8dc60` | 512 | all six | 0 |
| zenodo/zenodo-rdm | `b5bf82e` | 264 | all six | 0 |
| flaskbb/flaskbb | `63afaa4` | 242 | all six | 0 |
| Netflix/lemur | `3a3c2c2` | 337 | all six | 0 |
| Buuntu/fastapi-react | `af17f2d` | 30 | all six | 0 |
| wger-project/wger | `f903c66` | 766 | all six | 0 |

Counterexample repositories: `/tmp/review-celery/{dbc,pickle,cv/rabbit,cv/envbroker,cv/sqs,cv/misc,rget,targ,uk}`.
Findings of every run: `~/rerail-bench/python-arch/runs/review-celery-*/findings.tsv`.

## Applied

Fixes made by the pack's fixer, by root cause rather than by the counterexamples alone. Every rule's `.test` passes;
corpus figures are in the README's "Rule details".

- **Library (`rules/lib/celery.rerail`).** `cel_task_ref` now takes only resolved Celery tasks: a task def of the run
  (through aliases and re-exports), a module or local name bound to a task object (`app.task(fn)`, a Task subclass
  instance, a signature, also sent inline), `<app>.tasks[...]`, and signature calls on one; RQ's `@job`, thread pools
  and other `.delay`/`.apply_async` look-alikes are no longer tasks (2.1, 2.4, 2.5). Task decorators also resolve
  through project wrappers (a run function applying one), aliases (`task = app.task`), a Celery wrapper package's
  `task`/`shared_task` imported beside celery, and `@<x>.task` where x resolves to nothing in a module that imports
  celery. Apps resolve through factories (`cel = make_celery()`), locals and `current_app`; the `app`/`celery_app`/
  `celery` spelling is used only for a name that resolves to nothing and for attributes (2.6). `sa_session` is now
  proven by its binding (session class or subclass constructor, sessionmaker/scoped_session, Flask-SQLAlchemy's
  `db.session`, a run function returning or yielding one, `with ... as`, an annotation also inside `Annotated` or an
  alias, an imported module session, `self.x` set from one), with the name fallback (a name ending in "session" whose
  binding resolves to nothing, in a run that imports SQLAlchemy) overruled by any known non-session binding (2.5,
  section 3). `cel_task_arg` reads `**{...}`/`*[...]` spreads and `kwargs=dict(...)`.
- **`dispatch-before-commit`** (rework): writes, sends and commits pair only on one path (if/elif/else, match and
  try except/else arms; return/raise exits); attribute writes to mapped columns of rows loaded through the session
  are writes (the review's commonest missed shape); a caller's commit counts only on the session carried from the
  callee's parameter; RQ, pools, aliases, re-exports, task objects and HTTP clients in the cases; message 184
  characters. Steer, on.
- **`pickle-serializer`** (text): new description and a 158-character message without the unrelated advice; a send's
  `serializer=` is read only on Celery sends (a pool's `apply_async` is silent). Steer, on.
- **`countdown-past-visibility-timeout`** (rework): the limit follows the broker (RabbitMQ consumer_timeout 1800 s or
  a queue's `x-consumer-timeout`, with its own message; SQS 1800 s; else visibility_timeout or 3600 s); values follow
  names through every live assignment (`eta=later`); delay calls need a resolved task receiver, which closes the
  `.set(countdown=...)` gap; messages name the cost of a longer timeout. The lookit-api and addons-server misses are
  caught. Steer, on.
- **`result-get-in-task`** (rework): the `disable_sync_subtasks=False` advice is gone from the message (it now says
  Celery raises in the worker and to chain instead); a thread pool's `apply_async().get()` is silent through the
  library; results kept in lists, dicts and comprehensions (`[r.get() for r in results]`, `pending[i].get()`) are
  read. Steer, on.
- **`task-arg-not-serializable`** (rework): the session shape needs a proven session, or an unannotated parameter
  ending in "session" that its function never rebinds; strings, dicts and HTTP clients named `session` are silent;
  RQ's `.delay` is not a Celery send; models inside list, tuple and dict literals, spreads and `dict(...)` kwargs,
  Flask-Login's `current_user`, and set operators are read; the wrapper exemption resolves Task subclasses and
  `self.Task`; all five messages under 200 characters. Steer, on (the explicit `"on": true` is gone; on is the
  default).
- **`unknown-task-name`** (rework): `send_task`/`signature` need a resolved app; one value resolver reads every name
  (constants through imports and re-exports, class attributes, locals); `app.task(fn)` registers; ownership is by
  exact module only (the library-name list is gone except `celery.*`); an options-only beat entry counts only when it
  reaches the beat setting (data flow, including paperless-ngx's `parse_beat_schedule()`); new description and two
  messages that tell the agent to check the name, not rename it. Log, off.
- **Pack:** `pack.json` and the README list the rules in this review's reading order; the README's placeholders are
  filled and its rule details rewritten. Missing rules (`retry-swallowed`, `retry-non-idempotent`,
  `network-task-without-time-limit`, `async-def-task`) are out of this fix's scope and were not built.
