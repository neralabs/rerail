# python-practice: mining notes

What a reviewer of core Python code (async, exceptions, typing, resources, packaging) flags every time, and which of it
no linter the team already runs can see. The pack is framework-free: Django ORM, SQLAlchemy sessions, FastAPI
dependencies, Celery and pytest have their own packs (BRIEF.md, "Pack boundaries").

Linters checked for every candidate: the `python` pack's Ruff 0.16.9 and Bandit 1.9.4 ports (`packs/python/pack.json`:
ASYNC100/105/115/116/210/221/222/230/250/251, B, TRY, RUF006, SIM, TC005/006/010, S110/S112/S113 among them), Ruff's
full rule list (B012, BLE001, B006, B019, SIM115, TC004 and the rest not ported), mypy's default error codes, pylint
(W0150, R1732, E0712), flake8-async (ASYNC102-104, which Ruff does not implement) and deptry (DEP001-DEP004).

## 1. Sources read

| Source | URL | What it contributed |
|---|---|---|
| Python docs, Developing with asyncio | https://docs.python.org/3/library/asyncio-dev.html | "Running Blocking Code": blocking code must not be called directly, every task waits; "Detect never-awaited coroutines": a coroutine called and not awaited never runs, only a RuntimeWarning says so |
| Python docs, Coroutines and Tasks | https://docs.python.org/3/library/asyncio-task.html | create_task: save a reference or the task may be garbage collected; Task cancellation: TaskGroup and asyncio.timeout() "might misbehave if a coroutine swallows asyncio.CancelledError"; gather(return_exceptions=True) semantics |
| Python docs, Runners | https://docs.python.org/3/library/asyncio-runner.html | asyncio.run() and Runner.run() "cannot be called when another asyncio event loop is running in the same thread" |
| Python docs, Event loop | https://docs.python.org/3/library/asyncio-eventloop.html | get_event_loop() raises RuntimeError in 3.14 when no loop is current; get_running_loop() preferred in coroutines |
| What's New in Python 3.14 | https://docs.python.org/3/whatsnew/3.14.html | asyncio.get_event_loop() no longer creates a loop (gh-126353); PEP 765 warnings for return/break/continue in finally |
| PEP 765, Disallow return/break/continue that exit a finally block | https://peps.python.org/pep-0765/ | `finally` that masks an exception; already a SyntaxWarning, Ruff B012 and pylint W0150 |
| Python docs, contextlib | https://docs.python.org/3/library/contextlib.html | the generator-based context manager pattern: acquire, `try: yield ... finally: release`; code after a bare yield does not run when the with-body raises |
| asgiref sync.py (AsyncToSync.__call__) | https://github.com/django/asgiref/blob/main/asgiref/sync.py | async_to_sync raises "You cannot use AsyncToSync in the same thread as an async event loop" when called from a coroutine |
| apache/airflow issue 63760 | https://github.com/apache/airflow/issues/63760 | a real outage of that error: a trigger reached async_to_sync from async code |
| Ruff ASYNC210/230/251 (flake8-async ports) | https://docs.astral.sh/ruff/rules/blocking-http-call-in-async-function/ , https://docs.astral.sh/ruff/rules/blocking-sleep-in-async-function/ | the blocking calls Ruff knows, and that it checks only calls written directly in the async def |
| flake8-async rules | https://flake8-async.readthedocs.io/en/latest/rules.html | ASYNC103/104 no-reraise-cancelled; ASYNC102 await in finally; the list Ruff has not ported |
| Ruff issue 8451, Implement flake8-async rules | https://github.com/astral-sh/ruff/issues/8451 | confirms ASYNC102-104 are not in Ruff |
| Ruff RUF006 asyncio-dangling-task | https://docs.astral.sh/ruff/rules/asyncio-dangling-task/ | the single-statement form of a lost task, already ported |
| mypy error codes, unused-coroutine | https://mypy.readthedocs.io/en/stable/error_code_list.html | mypy reports an unused coroutine by default, but only in checked (annotated) functions and only where it runs |
| mypy issue 20570 | https://github.com/python/mypy/issues/20570 | the forgotten-await bug class as mypy's maintainers describe it |
| Tenacity API reference | https://tenacity.readthedocs.io/en/latest/api.html | `Retrying(stop=stop_never, wait=wait_none)`: the defaults retry forever with no wait |
| Tenacity docs | https://tenacity.readthedocs.io/en/latest/ | "the default behavior is to retry forever without waiting" |
| backoff configuration | https://backoff.readthedocs.io/en/latest/user-guide/configuration.html | max_tries / max_time are the only limits; without them on_exception retries until success |
| Ruff TC004 runtime-import-in-type-checking-block | https://docs.astral.sh/ruff/rules/runtime-import-in-type-checking-block/ | TYPE_CHECKING imports used at runtime, single-file: already enforced |
| Ruff B012, SIM115, B019 | https://docs.astral.sh/ruff/rules/jump-statement-in-finally/ , https://docs.astral.sh/ruff/rules/open-file-with-context-handler/ , https://docs.astral.sh/ruff/rules/cached-instance-method/ | finally masking, open without with, lru_cache on methods: already enforced |
| pylint consider-using-with (R1732) | https://pylint.readthedocs.io/en/stable/user_guide/messages/refactor/consider-using-with.html | resources (files, Popen, pools, lock acquire) not released on error: already enforced |
| deptry rules | https://deptry.com/rules-violations/ | DEP001 missing, DEP002 unused, DEP003 transitive, DEP004 dev dependency in runtime code: already enforced |
| SuperFastPython, asyncio common exceptions | https://superfastpython.com/asyncio-common-exceptions-and-warnings/ | "coroutine was never awaited" and "cannot reuse already awaited coroutine" are the top asyncio errors |
| Stack Overflow 34116942, how to cache asyncio coroutines | https://stackoverflow.com/questions/34116942/how-to-cache-asyncio-coroutines | lru_cache on an async def caches the coroutine object; the second await raises |
| requests docs, Timeouts | https://requests.readthedocs.io/en/latest/user/advanced/#timeouts | "By default, requests do not time out unless a timeout value is set explicitly. Without a timeout, your code may hang for minutes or more." The same holds for a Session |
| Bandit B113 request_without_timeout | https://bandit.readthedocs.io/en/latest/plugins/b113_request_without_timeout.html | the check, and its port `packs/python/rules/bandit-b113-request-without-timeout.json`, resolves only `requests.get/post/...` module functions, not calls on a Session instance |
| python/cpython issue 149160 and arq issue 512 | https://github.com/python/cpython/issues/149160 , https://github.com/python-arq/arq/issues/512 | real breakage from the 3.14 get_event_loop change |

## 2. Candidates

Corpus probe: `rules/python-practice-probe.rerail` (removed after mining) over jupyterhub, mealie, open-webui,
freqtrade, Netflix dispatch and polar with the engine, plus text searches over all 20 repositories
(`rg` over the 29,415 `.py` files). Counts below leave out test files.

| Id | Catches | Why it is a bug | Sources | Scope | Linter overlap | Detection | Verdict |
|---|---|---|---|---|---|---|---|
| blocking-call-reached-from-async | an `async def` that calls a sync helper (any module) which reaches time.sleep, a sync `requests`/`httpx` call, urlopen, subprocess, os.system, a socket connect; also asyncio.run / run_until_complete / async_to_sync reached the same way | stalls every task on the loop (outage under load); a nested loop start raises RuntimeError | asyncio-dev "Running Blocking Code"; asyncio-runner; asgiref; Ruff ASYNC2xx docs; airflow 63760 | cross-file | Ruff ASYNC210/221/222/251 see only calls written in the async def | python/calls call graph: sure targets from the async def's own frame through sync defs (lib: pp_async_start, pp_sync_reach, pp_blocking, pp_loop_entry) | **build**: jupyterhub `proxy.py:730` (start() reaches time.sleep in _check_previous_process), `app.py:3612` (subprocess in dbutil); mealie `app.py:68` (startup only) |
| coroutine-never-awaited | a call of an async def of the run whose coroutine is thrown away (`helper()` as a statement, in sync or async code), or an async def handed to a sync runner (Thread target, executor.submit, run_in_executor, to_thread) | the body never runs; the only sign is a RuntimeWarning at garbage collection: silent loss of work | asyncio-dev "Detect never-awaited coroutines"; mypy unused-coroutine; SuperFastPython | cross-file | mypy unused-coroutine, only in annotated (checked) callers and only when the team runs mypy; RUF006 is the task form; Ruff ASYNC105 is trio only | call_target all async, not open, result discarded; references resolved to async defs (pp_coroutine_call, pp_discarded, pp_async_fn_ref) | **build**: 0 matches in 6 repos probed: merged code rarely keeps it, agents write it; strong evidence (docs, mypy default error) |
| cancellation-swallowed | in an `async def`, `except asyncio.CancelledError`, `except BaseException` or a bare `except:` around an await that ends without re-raising | TaskGroup, asyncio.timeout and shutdown depend on CancelledError propagating; a swallowed one makes cancel() a no-op and timeouts hang | asyncio-task "Task cancellation"; flake8-async ASYNC103/104; Ruff issue 8451 | single-file | flake8-async ASYNC103/104 (not in Ruff); none in the `python` pack | handler kind and re-raise paths; exempt the drain idiom (`t.cancel()` then `await t` in the try) and handlers that check `cancelling()` | **build** (context-aware: the drain idiom, which ASYNC103 reports, is left alone): text search finds 43 CancelledError handlers, several swallowing ones (jupyterhub `app.py:4124`, freqtrade `exchange.py:3658`); the writer found two of the lead's first picks wrong (open-webui `main.py:1710` re-raises; polar `worker/_health.py:196` is a sync def) |
| retry-without-limit | tenacity `@retry`/`Retrying` without `stop=`, backoff `on_exception`/`on_predicate` without `max_tries`/`max_time`, and a `while True` loop that retries until its try body succeeds with no attempt or time bound | a permanent error (bad credentials, 4xx, deleted row) becomes an endless loop: a hung worker or a request that never ends; tenacity's default also has no wait | tenacity API (stop_never, wait_none); tenacity docs; backoff configuration | single-file | none (Ruff, pylint, Bandit have no retry rule) | decorator calls resolved through imports, keyword presence; loop shape: try body exits on success, handler without raise/return/break and no bound | **build**: Netflix dispatch `dispatch_google/drive/drive.py:117` (`@retry(wait=...)`, no stop); manual loops in the corpus are mostly bounded |
| contextmanager-cleanup-skipped | a `@contextmanager`/`@asynccontextmanager` generator whose release call (close, release, unlock, shutdown, rollback, restore) follows a `yield` that no try/finally or with guards | the with-body's exception is thrown in at the yield; the release never runs: locks stay held, files and connections leak, patched globals stay patched | contextlib docs; pylint R1732 rationale | single-file | none: SIM115/R1732 check open() and Popen, not generator context managers | pp_cm_yield, guard by enclosing try/with inside the def, release-like call after it | **build**, likely Log: the probe's first form (any call after yield) matched 7 in polar and 3 in FastAPI lifespans, mostly commit-on-success code where skipping is intended; the writer narrows it to release calls |
| requests-session-without-timeout | a call on a `requests.Session` instance (local, `self.x` set in another method, module global, imported) with no `timeout=` | requests has no default timeout, on a Session too: one stalled server hangs the worker forever | requests docs "Timeouts"; Bandit B113 docs | cross-method / cross-file | Bandit B113 (ported) checks only `requests.<verb>()` module calls | receiver bound to `requests.Session()` through assignments, attributes and imports; call without `timeout` keyword | **build**: 24 files construct a Session outside tests (label-studio `core/utils/io.py:370`, DefectDojo `api_*/api_client.py`, superset `utils/network.py:157`, pretix paypal client) |
| task-dropped-across-modules | a helper returning `asyncio.create_task(...)` whose caller drops the result | the task can be garbage collected mid-flight | asyncio-task; RUF006 | cross-file | RUF006 sees the direct statement | call to a def whose return is a spawn call, call discarded | **drop**: 0 matches in 6 repos and no reports of the cross-module form; RUF006 covers the common one |
| gather-exceptions-discarded | `await asyncio.gather(..., return_exceptions=True)` whose result is thrown away | every exception is swallowed | asyncio-task; BaoDev post | single-file | none | discarded call shape | **drop**: 36 sites, nearly all the deliberate "cancel then drain" idiom (open-webui tether.py:314, socket/apps.py:354); a steer would be noise |
| lru-cache-on-async-def | `@lru_cache`/`@cache` on an `async def` | caches the coroutine; the second await raises RuntimeError | SO 34116942; asyncstdlib docs; bpo-46622 | single-file | none | decorator on async def | **drop**: 0 occurrences in 29,415 files |
| get-event-loop-without-loop | `asyncio.get_event_loop()` outside a running loop | RuntimeError from 3.14 | What's New 3.14; cpython 149160; arq 512 | cross-file (who calls it) | none | call in a sync def no async def reaches | **drop**: 10 corpus uses, all inside a running loop (jupyterhub init_signal, open-webui loaders) |
| sync-lock-across-await | a `threading.Lock` held across an `await` | blocks the loop, can deadlock | asyncio-sync docs | cross-method (lock type) | none | with-lock body containing await | **drop**: the corpus uses (freqtrade exchange_ws.py, open-webui misc.py:1310) hold the lock only around sync code |
| finally-masks-exception | return/break/continue in finally | swallows the exception | PEP 765 | single-file | Ruff B012, pylint W0150, CPython 3.14 SyntaxWarning | — | **drop**: enforced already |
| broad-except-swallows | `except Exception: pass` / log-only handlers | errors vanish | — | single-file | Ruff BLE001, S110, S112 | — | **drop**: enforced; no context the engine adds that tells harmful from deliberate |
| type-checking-import-at-runtime | a TYPE_CHECKING-only import used at runtime | NameError at runtime | Ruff TC004 docs | single-file (cross-file in pydantic models) | Ruff TC004; the pydantic-model form belongs to the pydantic pack | — | **drop**: TC004 covers the single-file form; the runtime-evaluated annotation form is framework-specific |
| any-leak | `Any` crossing a module boundary | type errors pass unchecked | mypy docs | cross-file | mypy `--disallow-any-*`, `warn_return_any` | — | **drop**: a type checker's job; the engine has no inference to add |
| undeclared-or-dev-dependency-import | imports of packages pyproject.toml does not declare, or declares only as dev | ImportError in production | deptry docs | cross-file (manifest) | deptry DEP001/DEP003/DEP004 | — | **drop**: deptry enforces it; `.rerail` Python rules do not read pyproject.toml |
| entry-point-unresolved | `[project.scripts]` target that names no function | the console script crashes on start | packaging guide | cross-file (manifest) | none | — | **drop**: needs TOML facts the Python rules do not get; no corpus occurrence |
| mutable-default / open-without-with / lock-acquire-without-finally / subprocess-without-check | — | — | — | single-file | Ruff B006, SIM115, pylint R1732, PLW1510 | — | **drop**: enforced |

## 3. Build list, ranked

1. `python-practice-blocking-call-reached-from-async` (availability, critical): one blocking helper stalls the whole
   service; Ruff only sees the direct call.
2. `python-practice-coroutine-never-awaited` (correctness, high): work silently never happens.
3. `python-practice-cancellation-swallowed` (availability, high): shutdowns and timeouts hang.
4. `python-practice-retry-without-limit` (availability, high): an endless retry on a permanent error.
5. `python-practice-requests-session-without-timeout` (availability, high): a stalled peer hangs a worker forever.
6. `python-practice-contextmanager-cleanup-skipped` (availability, medium): locks and connections left held on error.

Six rules, fewer than the guide's eight: the other candidates are either enforced already or had no real occurrence in
the corpus and no cross-file angle that would add one.
