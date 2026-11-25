# Rerail rule pack: Python practice

Rerail's own Python rules for async code, cancellation, timeouts, retries and resource cleanup.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/python-practice`.

Its core is async safety that no linter in a Python team's toolchain sees: a cancellation swallowed in a coroutine,
and a blocking call or a second event loop reached from one through helpers in other modules. Around it: HTTP calls on
a requests Session without a timeout, retries that never give up, coroutines that never run, and context managers
whose cleanup an exception skips. Typing and packaging rules were considered and dropped: mypy, pyright, Ruff and
deptry already enforce them (`NOTES-python-practice.md`).

## Rules

In reading order: by value and confidence. A rule starts on only when at least 90% of its read corpus matches were
real; the others start off, in `log`, until a re-measure clears that bar.

| Rule | Mode | On | Catches |
|---|---|---|---|
| `python-practice-cancellation-swallowed` | Steer | On | Python practice: asyncio.CancelledError caught or suppressed in an async def and not re-raised |
| `python-practice-blocking-call-reached-from-async` | Log | On | Python practice: an async def calls a sync helper, in any module, that sleeps, does network I/O, runs a subprocess or starts an event loop |
| `python-practice-requests-session-without-timeout` | Steer | On | Python practice: an HTTP call on a requests Session without a timeout |
| `python-practice-retry-without-limit` | Log | Off | Python practice: a tenacity, backoff, retrying or retry decorator without a limit |
| `python-practice-retry-loop-without-limit` | Log | Off | Python practice: a hand-written retry loop that never gives up |
| `python-practice-coroutine-never-awaited` | Steer | On | Python practice: coroutine of an async def created and never run |
| `python-practice-contextmanager-cleanup-skipped` | Log | Off | Python practice: release after an unguarded yield in a @contextmanager is skipped on error |

Corpus numbers below are from `~/rerail-bench/python-arch/corpus/` at the commits in its `MANIFEST.tsv`: the brief's
20 repositories plus the extras named per rule. A match counts as real when the harm the message names can happen at
that site.

## Rule details

### `python-practice-cancellation-swallowed`

Rerail's own rule, not a port. asyncio implements `Task.cancel()`, `asyncio.timeout()`, `wait_for()` and `TaskGroup` by
throwing `CancelledError` into the coroutine. A coroutine that catches it and goes on keeps running after it was
cancelled, and the coroutine that awaits it resumes as if the work had finished: a timeout no longer fires, a
TaskGroup waits on it, shutdown hangs.

It reports, in an `async def`'s own frame and outside test code:

- an `except` clause that catches cancellation (`asyncio.CancelledError`, also through `asyncio.exceptions`, an
  import alias or a tuple; `BaseException`; a bare `except:`) of a try whose body awaits (`await`, `async with`,
  `async for`), when some path through the handler ends without re-raising: it falls through, returns, breaks,
  continues, or raises another exception first;
- a `with contextlib.suppress(...)` naming one of those classes whose body awaits, when more of the function runs
  after it (a later statement, or the next turn of a loop around it).

A path re-raises when it ends in a bare `raise`, `raise <the caught name>`, `raise CancelledError()`, a raise of a
`BaseExceptionGroup` (builtin, the `exceptiongroup` backport, or a subclass of the run; through import aliases) whose
arguments carry the caught exception (directly, in a list or tuple, or through a name the handler builds from it),
an `if` whose every branch re-raises or tests `isinstance(e, T)` against a class a cancellation is not, a `with` or a
nested `try ... finally` that does, `if isinstance(e, CancelledError): raise` (when the handler also catches
`Exception`), or a call of a function of the run whose body always raises. `raise ExceptionGroup(..., [e])` is no
re-raise: an `ExceptionGroup` refuses a `CancelledError` and raises `TypeError`.

It leaves alone the idioms a reviewer accepts:

- the drain: `t.cancel()` (or a loop or comprehension cancelling each of `ts`) earlier in the function, or in a
  helper of the same class (or a base class) or module that the call graph resolves (`self._stop()`), and the try
  (or the suppress) awaits `t`, `asyncio.gather(*ts)`, `asyncio.wait_for(t, ...)` or `asyncio.shield(t)`. `t` is
  compared by its text, a dotted name or a subscript (`self.tasks[k]`). `asyncio.wait(ts)` is not a drain: it never
  raises a child's CancelledError;
- a handler that reads `asyncio.current_task().cancelling()` (or calls `uncancel()`) and raises;
- the coroutine an event-loop runner starts: `asyncio.run(main())`, `uvloop.run(main())`,
  `loop.run_until_complete(main())`, `asyncio.run_coroutine_threadsafe(main(), loop)`, `.run(main())` on an
  `asyncio.Runner` (also `with asyncio.Runner() as runner`), and the function `anyio.run(main)` or `trio.run(main)`
  takes (also inside `functools.partial`);
- a handler that ends its function when no code of the run resumes after it: the try is the function's last
  statement (also inside a final `with`/`async with`, or in a top-level loop the handler leaves with `break` or
  `return`), nothing follows but `return`/`return None`, the handler returns no value, and either the function only
  ever runs as a task (`create_task`, `loop.create_task`, `ensure_future`, or a spawn helper of the run such as
  `safe_create_task(pump())`), or no code of the run awaits it (it is handed to a framework,
  `StreamingResponse(events(q))`, passed uncalled, `scheduler.add_job(self.refresh)`, or registered by a decorator,
  `@router.websocket("/ws")`) and the handler only records or logs. A function that returns a partial result after
  the handler, an async generator that yields after it or in it, and a function some code of the run awaits are
  reported;
- a deferred re-raise: the handler sets `cancelled = True` and a later `if cancelled: raise asyncio.CancelledError`
  raises it after the bookkeeping;
- a handler or suppress nested in a cancellation handler that itself re-raises on every path;
- anyio and trio code: a handler that catches cancellation only as `BaseException` or bare `except:` (or a
  `suppress(BaseException)`) in a function that uses anyio or trio itself. Their cancel scopes deliver a swallowed
  cancellation again at the next checkpoint. An explicit `except asyncio.CancelledError` there is still reported;
- `except Exception` (CancelledError has been a BaseException since Python 3.8), sync defs (also one nested in an
  async def), a try body that does not await, an application class named `CancelledError`,
  `concurrent.futures.CancelledError`, and test files.

**Gaps:** a drain whose task travels through another name (`t = self._task; self._task.cancel(); await t`), or whose
cancel is in a helper the call graph cannot resolve, is reported; a CancelledError that an async generator or library
hands up from a child it cancelled is reported when the function does not cancel that child by name (jupyterhub's
progress stream); a handler that sets a flag a loop reads and then exits cleanly is reported; a coroutine only code
outside the run awaits (a library's public API) is reported unless it is handed off, and one handed to an outside API
that does await it and resume is taken as ended; a suppress that ends its function is left alone; an exception group
class bound by assignment (`Group = BaseExceptionGroup`) is not resolved; anyio is decided per function, so code that
runs in an anyio scope without naming anyio is read as asyncio code.

**Sources:** Python docs, [Task cancellation](https://docs.python.org/3/library/asyncio-task.html#task-cancellation)
("asyncio.TaskGroup and asyncio.timeout() are implemented using cancellation internally and might misbehave if a
coroutine swallows asyncio.CancelledError"); flake8-async
[ASYNC103/ASYNC104](https://flake8-async.readthedocs.io/en/latest/rules.html) (no-reraise-cancelled,
cancelled-not-raised); Ruff has not ported them ([ruff#8451](https://github.com/astral-sh/ruff/issues/8451)); anyio,
[Cancellation and timeouts](https://anyio.readthedocs.io/en/stable/cancellation.html) (level cancellation, shielding).

**Overlap:** flake8-async ASYNC103/104, a flake8 plugin that is not in Ruff or the `python` pack. ASYNC103 reports
every handler of the drain idiom, the `cancelling()` check, the runner's `main()`, a task's last handler and a
handler nested in one that re-raises, the shapes the corpus shows are deliberate. This rule leaves them alone, and
reads across files what a single-file linter cannot: whether a coroutine is only ever started as a task or handed to
a framework (also through a spawn helper in another module), whether the cancel of a drain sits in a helper, and
whether a handler's call goes to a helper that always raises.

**Corpus:** re-run after this change, every match read: 21 matches, 20 real (95%).

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| letta-ai/letta (extra) | 11 | 11 | |
| freqtrade/freqtrade | 3 | 3 | |
| open-webui/open-webui | 3 | 3 | |
| jupyterhub/jupyterhub | 2 | 1 | `apihandlers/users.py:1097`: the CancelledError comes from the spawn future `iterate_until` wraps, not from the handler's own task |
| polarsource/polar, `sdk/` only (292 files; the full repository is over the fleet's limit) | 0 | | |
| khoj, lemur, mealie, paperless-ngx, label-studio, dispatch, fastapi-users, full-stack-fastapi-template, linkding, healthchecks | 0 | | |

Real, by file: letta's streaming interfaces (`interfaces/openai_streaming_interface.py:264`, `:770`, `:1195`,
`anthropic_streaming_interface.py:251`, `:802`, `anthropic_parallel_tool_call_streaming_interface.py:260`,
`gemini_streaming_interface.py:175`: the stream keeps yielding after a cancel), `server/rest_api/streaming_response.py:248`
and `:358`, `services/tool_executor/tool_execution_manager.py:131` (a cancelled tool call returns an error result and
the agent loop goes on), `services/tool_sandbox/e2b_sandbox.py:103`; freqtrade `exchange/exchange.py:3658` and
`:3697` (a cancelled trade download returns the partial list as a success), `rpc/telegram.py:144`; open-webui
`main.py:1782`, `:1784`, `:1808`; jupyterhub `app.py:4124`. Against the review's run, the change removed exactly the
named not-real matches: letta `server/rest_api/routers/v1/tools.py:753` (an SSE generator handed to a streaming
response, ending in a logging handler) and polar's six in `sdk/python/polar/metering/` and its generator template
(`__init__.py:222`, a `BaseExceptionGroup` re-raise; `buffered.py:327` and `:361`, `except BaseException` in anyio
code). dify (`api/`, 4,921 files) is not run: over the fleet's limit, and the review's own runs did not finish.

### `python-practice-blocking-call-reached-from-async`

Rerail's own rule, not a port. A coroutine runs on the event loop's thread until its next `await`. A blocking call
anywhere below it, also in a plain helper in another module, stops every other task on the loop until it returns:
under load one slow upstream stalls the whole service. And a helper that starts an event loop (`asyncio.run()`,
`run_until_complete()`, asgiref's `async_to_sync`) raises RuntimeError when a coroutine reaches it, because a loop
already runs in that thread.

It reports, in an `async def`'s own frame (not a nested def, lambda or class body) and outside test files, a call
whose target, by the call graph across the files of the run, is a sync function that blocks its thread, itself or
through a chain of calls through other sync functions in any module. The message names the helper and the
nearest blocking site (`path:line`), one finding per call:

- **blocking calls:** `time.sleep`, the sync `requests` and `httpx` module functions (`requests.get`, `httpx.post`,
  `requests.request`, ...), `urllib.request.urlopen`, `subprocess.run`/`call`/`check_call`/`check_output`/
  `getoutput`/`getstatusoutput`, `os.system`, `os.popen`, `os.wait*`, `socket.create_connection`/`getaddrinfo`/
  `gethostbyname`, `smtplib.SMTP(...)`/`SMTP_SSL(...)`. Message: run the helper with
  `await asyncio.to_thread(helper, ...)` or make it async, and leave it when it runs only before the server serves;
- **loop starts:** `asyncio.run()`, `<loop>.run_until_complete()`, `async_to_sync(f)(...)`, with their own message
  (a crash, not a stall): await the coroutine instead. A loop start written directly in the `async def` is reported
  too: no linter checks it.

A step of the chain is a sure call target, or a helper under a decorator that keeps calling it in the caller's
thread: `functools.lru_cache`/`cache`, cachetools, tenacity's `@retry`, `backoff.on_exception`, `retrying`,
`stamina`, or a decorator of the run whose wrapper calls the wrapped function (`@functools.wraps(f) def wrapper(*a):
return f(*a)`, also from a factory such as `@retrying(3)`). So a `@retry`-wrapped `requests.get` helper is followed.
Names resolve through imports, so `from time import sleep` counts, and `from app import requests; requests.get(...)`
with `app/requests.py` a module of the run does not. FastAPI and Starlette routes are `async def`s like any other and
are reported.

It leaves alone:

- a blocking call written in the `async def` itself: the `python` pack's ASYNC210/221/222/230/251 ports report it;
- a helper run in a thread: `await asyncio.to_thread(helper, x)`, `await loop.run_in_executor(None, helper)`,
  `await sync_to_async(helper)()`, `anyio.to_thread.run_sync(helper)`, a helper decorated with
  `@sync_to_async`/`@database_sync_to_async`, or one under a decorator of the run that hands it to a thread or an
  executor;
- a chain through an `async def` (calling it only makes a coroutine), through a call whose target the call graph
  cannot settle (a parameter, a dict lookup, a method on an object of unknown type), or through a decorator imported
  from a library the rule does not know;
- a loop start that cannot raise, because it runs only where no loop runs: in the `except RuntimeError` of a `try`
  that calls `asyncio.get_running_loop()`, in the branch `if loop is None:` or `if not loop.is_running():` leads to
  (or the `else:` of `if loop and loop.is_running():`), or after `nest_asyncio.apply()`. Such a dual-mode helper
  starts a loop only when called from sync code;
- startup code: a lifespan (an `asynccontextmanager` passed as `lifespan=`), an `on_event("startup")`/
  `add_event_handler("startup", f)` handler (or shutdown), an `on_startup=[...]`/`on_shutdown=[...]` entry, and any
  coroutine that only startup code awaits, at any depth (the lifespan awaits `server.init_async()`, which awaits
  `self._prepare()`). It runs while no request is served. A coroutine also awaited from a route, spawned as a task
  (`asyncio.create_task(poll())` in the lifespan), handed on by name (`scheduler.add_job(job)`) or decorated by
  anything but the known decorators is not startup code. A loop start in startup code is still reported, since it
  raises all the same;
- a blocking call that runs once per process: under a once-guard (`if self._x is None:`, or a module global, whose
  body assigns that same name: the lazy `git --version` probe), or in a memoized helper with no argument but
  `self`/`cls` (`@lru_cache def git_sha()`). A memoized helper with arguments blocks for each new key and is reported;
- `self.m()` in a method of class `K` follows `K`'s own `m` (or the one it inherits), not a subclass's override: a
  base class's `async def` that calls `self.m()` is judged with the base's `m`, so a subclass that overrides both
  methods is not blamed on the base;
- look-alikes: `trio.sleep`, `asyncio.sleep`, `httpx.AsyncClient().get`, a module of the run named `requests`;
- a call in a nested def, lambda or class body inside the `async def`;
- test files.

**Gaps:** file I/O (`open()`, `Path.read_text()`) is left out on purpose: short local reads are common in async code
and rarely what stalls a loop. Sync clients reached through an object (`requests.Session().get`, `smtplib.SMTP().sendmail`,
a sync database driver) are not followed; the Django ORM is `django-practice-sync-orm-reached-from-async`'s, sync
SQLAlchemy sessions are the sqlalchemy and fastapi packs'. A class body nested in the `async def` runs at once, but
is skipped with defs and lambdas. A call the call graph resolves to several targets is followed only when none is
unknown. A subclass that inherits a base's `async def` and overrides only the sync method it calls is not reported,
nor is `self.m()` where `m` is under a decorator of the run. The decorators kept as same-thread are a name list: a
library decorator outside it is not followed. A loop start guarded by a test the rule does not read (a flag, a helper
that returns whether a loop runs) is reported. Startup code the rule cannot tie to a lifespan is reported: a main
coroutine run by `asyncio.run(main())` or tornado's `IOLoop.run_sync(...)`, whose first awaits run before the server
listens (jupyterhub's `initialize()`). The message says to leave it when it runs only before serving.

**Overlap:** Ruff's ASYNC210/221/222/230/251 (ported in the `python` pack) see only a call written in the
`async def`; the helper in `services/http.py` the route calls is invisible to them, and no Ruff rule checks a loop
start in a coroutine.

**Sources:**
- Python docs, [Developing with asyncio: Running Blocking Code](https://docs.python.org/3/library/asyncio-dev.html#running-blocking-code): "Blocking (CPU-bound) code should not be called directly. ... if a function performs a CPU-intensive calculation for 1 second, all concurrent asyncio Tasks and IO operations would be delayed by 1 second."
- Python docs, [Runners: asyncio.run](https://docs.python.org/3/library/asyncio-runner.html#asyncio.run): "This function cannot be called when another asyncio event loop is running in the same thread."
- asgiref, [`sync.py`](https://github.com/django/asgiref/blob/main/asgiref/sync.py): AsyncToSync raises "You cannot use AsyncToSync in the same thread as an async event loop - just await the async function directly."
- Ruff, [blocking-http-call-in-async-function (ASYNC210)](https://docs.astral.sh/ruff/rules/blocking-http-call-in-async-function/): the check and its limit to calls written in the async function.

**Corpus:** re-run after this change, every match read: 18 matches, 17 real (94%).

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| khoj-ai/khoj (extra) | 7 | 7 | |
| letta-ai/letta (extra) | 5 | 5 | |
| jupyterhub/jupyterhub | 2 | 1 | `app.py:3612`: `initialize()` runs the alembic upgrade before any task starts (startup code the rule cannot tie to a lifespan) |
| apache/superset, `superset/mcp_service/` with `superset/utils/version.py` (254 files; the full repository is over the fleet's limit) | 2 | 2 | |
| mealie-recipes/mealie | 1 | 1 | |
| paperless-ngx/paperless-ngx | 1 | 1 | |
| freqtrade, open-webui, lemur, label-studio, dispatch, fastapi-users, full-stack-fastapi-template, linkding, healthchecks | 0 | | |

Real, by file: khoj `processor/tools/online_search.py:74` (`requests.head("https://www.google.com")` with no timeout
before every online search), `routers/api_chat.py:220` (text-to-speech `requests.post` in the route),
`processor/conversation/anthropic/utils.py:208` and `google/utils.py:324` (message formatting downloads images with
`requests.get`), and three the change newly reaches: `processor/image/generate.py:119` (Replicate image generation
with `requests.post`/`get` in the async generator), `:123` and `routers/helpers.py:1703` (an image download with
`requests.get` in `send_message_to_model`); letta `routers/v1/sandbox_configs.py:146`/`:150` (`pip install` through
`subprocess.run` in a request), `services/mcp_manager.py:821` and `mcp_server_manager.py:992` (`socket.getaddrinfo`
per MCP client), `schemas/providers.py:907` (`requests.post` per model); superset
`mcp_service/system/tool/health_check.py:78` and `generate_bug_report.py:312` (`git` through
`subprocess.check_output`); mealie `pkgs/safehttp/transport.py:216` and paperless `src/paperless/network.py:178` (the
async SSRF-pinning transports resolve the host with `socket.getaddrinfo` per request); jupyterhub `proxy.py:730`
(`start()` may `time.sleep` up to 3 s while `init_spawners` runs in the background). Against the review's run, the
startup fixpoint removed letta `server/server.py:420` (`init_async`, awaited only from the lifespan). dify is not run.

### `python-practice-requests-session-without-timeout`

Rerail's own rule, not a port. requests has no default timeout, on a `Session` as on the module functions: a server
that accepts the connection and then stops answering hangs the calling worker, request handler or task forever.
Bandit B113 (the `python` pack's `bandit-b113-request-without-timeout`, Ruff S113) reads only `requests.get(...)` and
its siblings, so code that follows the advice to "use a Session" loses the check. This rule reads the calls on a
Session instance, and binds the instance across statements, methods and modules.

It reports, in a def, lambda or class body outside test code, `.get/.post/.put/.patch/.delete/.head/.options/.request(...)`
with no `timeout=` (**`SessionCallWithoutTimeout`**) or with `timeout=None` (**`SessionCallTimeoutNone`**, as B113
reports it), on a receiver that holds a Session made by `requests.Session()`, `requests.session()`, or a class of the
run deriving from `requests.Session` that sets no timeout itself. The message names where the Session was created. The
receiver is bound through:

- a local name: `s = requests.Session(); s.get(url)` (the last assignment before the read), `with requests.Session() as
  s: s.post(...)`, and a name copied from one (`other = s`);
- a name of the enclosing function read in a lambda or nested def (`map(lambda u: s.get(u), urls)`), through the
  bindings that survive to the end of that function;
- `self.x` set in any method of the class or of a base class (`self.session = requests.Session()` in `__init__`, read in
  another method or in a subclass's), and a class attribute `session = requests.Session()`;
- a module global, read in its own module or imported from another (`from .net import session`, `net.session`);
- a call of a function or method of the run that returns a new Session (`make_session().get(url)`,
  `self._session = self._new_session()`), through its return value, across files.

It leaves alone:

- a call with any `timeout=` other than `None` (also a variable, or `(5, None)` for a stream), a call passing `*args`
  or `**kwargs` (a timeout may be inside), and `request()` with a ninth positional argument (the timeout);
- a Session of a class of the run that overrides `request()` or `send()` with a body naming `timeout` (the
  `TimeoutSession` idiom);
- a Session whose `request` or `send` (or the called verb itself) is replaced on the same instance by a value that
  sets a timeout: `self.session.request = functools.partial(self.session.request, timeout=10)`, a partial imported
  under another name, a lambda passing `timeout=`, a wrapper factory or a def of the run whose body names `timeout`.
  The replacement must reach this instance: it is in the scope that creates the Session (the factory, `__init__`, the
  module of a global), or it uses the same local name in the same function, or the same `self.x`. A factory's Session
  patched by one caller does not exempt another caller's; `partial(..., timeout=None)` sets no timeout;
- a Session on which `mount()` installs an adapter of a class of the run deriving from `HTTPAdapter` whose `send()`
  names `timeout` (`TimeoutHTTPAdapter`), when the `mount()` receiver binds to the same Session: in the factory, the
  function, or any method of the class. A plain `HTTPAdapter(max_retries=...)` sets no timeout and does not count;
- `httpx.Client` (5 s default timeout), `aiohttp.ClientSession`, SQLAlchemy's `Session`, and `requests.get(...)`
  (B113's);
- receivers it cannot bind: a parameter (annotated or not), `self.x` set from a parameter, a dict of sessions, a
  property;
- a call at module level, outside every def and class: a script run by hand, whose hang is in front of its operator
  (zenodo-rdm `scripts/new_api_flow.py`, 9 calls), and example and documentation directories (jupyterhub
  `examples/server-api/start-stop-server.py`);
- test files.

Whether an override, a replacement or an adapter "sets a timeout" is read from its text naming `timeout`: whether the
value is finite is not decidable there.

**Gaps:** a Session subclass whose `__init__` sets `self.timeout` is reported, rightly: requests never reads that
attribute. A timeout adapter mounted through a helper (`configure(session)`) is not seen, so that Session is reported.
Sessions passed as parameters (also annotated `requests.Session`), and a module-level function taking `self` as a
plain parameter (lemur `lemur_azure_dest/plugin.py:81`), are not bound. `session.send(prepared)` is not read.

**Sources:** requests docs, [Timeouts](https://requests.readthedocs.io/en/latest/user/advanced/#timeouts) ("By
default, requests do not time out unless a timeout value is set explicitly. Without a timeout, your code may hang for
minutes or more"), and the quickstart ("Nearly all production code should use this parameter in nearly all
requests"); Bandit [B113 request_without_timeout](https://bandit.readthedocs.io/en/latest/plugins/b113_request_without_timeout.html)
and its port `packs/python/rules/bandit-b113-request-without-timeout.json`.

**Overlap:** Bandit B113 / Ruff S113 cover the `requests.<verb>()` module functions, and this rule leaves them alone.
Semgrep's registry rule `python.requests.best-practice.use-timeout` covers a Session made and used in one scope. What
this rule adds: `self.x` across methods and subclasses, module globals across files, factories, closures, and the
timeout-adapter, `TimeoutSession` and patched-`request` exemptions. The python-security pack owns SSRF on the same
calls; this rule is about timeouts only.

**Corpus:** re-run after this change, every match read: 30 matches, 30 real (100%), the same 30 as the review's run.

| Repository | Matches | Real |
|---|--:|--:|
| Netflix/lemur (extra) | 24 | 24 |
| khoj-ai/khoj (extra) | 5 | 5 |
| pretix/pretix, `src/pretix/plugins/paypal2/` only (the full repository is over the fleet's limit) | 1 | 1 |
| letta, freqtrade, open-webui, jupyterhub, mealie, paperless-ngx, label-studio, dispatch, fastapi-users, full-stack-fastapi-template, linkding, healthchecks | 0 | 0 |

Real, by file: pretix `plugins/paypal2/client/core/paypal_http_client.py:141` (every PayPal API call goes through
`self.session.request(...)`, set in `__init__` with a retry-only `HTTPAdapter`); lemur
`plugins/lemur_digicert/plugin.py` (11 calls to DigiCert, `self.session` set in `__init__`),
`lemur_entrust/plugin.py` (5), `lemur_verisign/plugin.py` (5), `lemur_cfssl/plugin.py` (2),
`lemur_azure_dest/plugin.py:187`; khoj `processor/content/notion/notion_to_entries.py` (4) and
`github/github_to_entries.py:179` (a `stream=True` download). The review read the Sessions the rule leaves alone in
the brief's repositories (linkding, plane, addons-server, label-studio, freqtrade, superset): each passes `timeout=`,
forwards `**kwargs`, or hands the Session to a library.

### `python-practice-retry-without-limit`

Rerail's own rule, not a port. A retry that never gives up turns a permanent failure (bad credentials, a 4xx, a deleted
row, a schema error) into an endless loop: a request that never returns, a worker that never takes the next job, a hot
loop hammering the dependency that already failed. This rule reads the retry libraries, whose unbounded default is a
documented fact; hand-written loops are `python-practice-retry-loop-without-limit`'s.

It reports, outside test code, one violation per library, names resolved through imports (`import tenacity`,
`from tenacity import retry as r`, `from retrying import retry`):

- **`TenacityWithoutStop`**: tenacity's `@retry`, `@retry(...)`, `retry(...)(f)`, `Retrying(...)` and
  `AsyncRetrying(...)` with no `stop=`. Tenacity's default is `stop_never` with `wait_none`: it retries forever,
  without waiting, on any exception. A `retry=` predicate alone does not bound it: it picks which errors (or results)
  are retried, not how often.
- **`BackoffWithoutLimit`**: `@backoff.on_exception(...)` and `@backoff.on_predicate(...)` with neither `max_tries=`
  nor `max_time=`. `max_value=` caps the wait between tries, not their number.
- **`RetryingWithoutStop`**: the `retrying` package's `@retry`, `@retry(...)` and `Retrying(...)` with none of
  `stop_max_attempt_number=`, `stop_max_delay=`, `stop_func=` or `stop=`: its default never stops.
- **`RetryWithoutTries`**: the `retry` package's `@retry`, `@retry(...)` and `retry_call(...)` with no `tries` (by
  keyword, or the second positional argument of `retry()`, the fifth of `retry_call()`): its default, -1, is forever.

It leaves alone a limit of any value (`stop_never`, `max_tries=None` and `tries=-1` say the forever is meant),
keywords passed through `**kwargs` (they may hold the limit), tenacity's positional `Retrying(sleep, stop)`, `retry`
names of other libraries (urllib3's `Retry`, Celery's `self.retry()`, which the celery pack owns, a function of the
run named `retry`), and test files.

**Gaps:** a tenacity `.retry_with(stop=...)` at the call site is not followed; a retry object configured after it is
built is read where it is built.

**Sources:** tenacity API reference, [`Retrying(stop=stop_never, wait=wait_none)`](https://tenacity.readthedocs.io/en/latest/api.html);
tenacity docs, ["the default behavior is to retry forever without waiting"](https://tenacity.readthedocs.io/en/latest/);
backoff [configuration](https://backoff.readthedocs.io/en/latest/user-guide/configuration.html) (`max_tries` and
`max_time` are its only limits); retrying's [README](https://github.com/rholder/retrying) ("the default behavior is to
retry forever without waiting"); retry's [README](https://github.com/invl/retry) (`tries`: "the maximum number of
attempts. default: -1 (infinite)").

**Overlap:** none. Ruff, pylint and Bandit have no retry rule; Celery task retries belong to the celery pack.

**Corpus:** re-run after this change: 1 match, 1 real. dispatch
`src/dispatch/plugins/dispatch_google/drive/drive.py:117` (`@retry(wait=wait_exponential(multiplier=1, max=10))`
around Google Drive calls: a revoked credential or a 404 is retried forever). The other 13 repositories of the
re-run (letta, khoj, lemur, freqtrade, open-webui, jupyterhub, mealie, paperless-ngx, label-studio, fastapi-users,
full-stack-fastapi-template, linkding, healthchecks) report nothing; every other tenacity and backoff use the
review found sets a stop (superset, dify, khoj), and lemur's `retrying` uses set `stop_max_attempt_number`. One
match is too few to start the rule on: it starts off, in `log`.

### `python-practice-retry-loop-without-limit`

Rerail's own rule, not a port, split from `python-practice-retry-without-limit`. The same harm as that rule's, in a
hand-written loop. Whether a loop is a retry is read from its shape, not its intent, so this rule stays in `log`:
each new large repository has shown a new shape of `while True: try:` that is not a retry.

It reports, outside test code, **`RetryLoopWithoutLimit`**: a loop that only ends from inside (`while True:`,
`while 1:`, `for _ in itertools.count():` also imported under another name, or `while not done:` where `done` is set
in the loop) with a `try` in its own body (a statement of it, or inside an `if`, `with` or another `try` there, not
inside a nested loop) that leaves the loop once its work is done (a `return`, a `break` of this loop, or `done = ...`,
as a statement of the try body or its `else`, or of a `with` there), when one of the try's `except` clauses never
raises, returns, leaves the loop or calls `sys.exit()`/`os._exit()`, and nothing else in the loop (a statement before
or after the try, its `finally`) does either: no attempt counter or deadline the loop checks. A raise in a def or
lambda nested in the loop does not end it.

It leaves alone:

- a `for attempt in range(n)` loop; a loop whose handler re-raises once a counter or deadline is exceeded, or whose
  loop checks one before or after the try (healthchecks `hc/api/transports.py:180`, mealie
  `mealie/repos/repository_group.py:25`), or whose handler sets the `done` flag;
- a worker, polling or reconnect loop whose exits are only conditional (`if msg is None: break`,
  `if stop.is_set(): break`: a stop signal, not a retry's success) or that has none, since it is meant to run until
  told to stop (jupyterhub `singleuser/extension.py:502`, open-webui `utils/automations.py:87`, the subscriber's
  reconnect loop in open-webui `socket/redis_room_channels.py:136`, dify `api/core/mcp/client/streamable_client.py:414`);
- a loop whose looping handlers catch only what is not a failure of the work: a wait's timeout (`q.get(timeout=1)`
  then `except queue.Empty: continue`, also `asyncio.QueueEmpty`; a redis `Lock(..., blocking_timeout=3)` then
  `except LockError`; dify `api/core/mcp/session/base_session.py:235`, breatheco-de/apiv2
  `breathecode/celery.py:182`), or a person's Ctrl-C (`except KeyboardInterrupt: continue` in an interactive shell:
  polar `server/scripts/shell.py:86`). A handler that also catches a real error (`except (KeyboardInterrupt,
  OSError)`) is still read;
- a handler of a name-conflict error (`EntityExistsError`, `IntegrityError`, a class named for a duplicate, conflict
  or unique error) that assigns a new value the try reads: each turn sends something new (zenml
  `src/zenml/zen_stores/sql_zen_store.py:11509`);
- a loop that prompts the person at the terminal on every turn (`input()`, `getpass()`, `click.prompt()`,
  `typer.prompt()`, rich's `Prompt.ask()`), in the loop or in a function of the run it calls, at any depth through
  sure call targets (superset `superset-extensions-cli/src/superset_extensions_cli/cli.py:732`). A prompt behind an
  `if`, an `and`/`or` or a conditional expression does not count, there or in the helper: `os.getenv("EMAIL") or
  input("Email: ")` prompts only while the variable is unset, so with it set the loop spins;
- a `while` with a real condition; test files.

The lists of wait, conflict and prompt names stay names: what an exception class means for the loop is not decidable
from the code. They resolve through imports.

**Gaps:** a loop whose bound sits in a helper its handler calls (`self._check_attempts()`, which raises) is reported;
a poll-until-ready loop (`try: r = get(); if r.ok: break`) is not read; a retry by recursion (`except OSError: return
fetch()`) is not read.

**Sources:** as `python-practice-retry-without-limit`; Google SRE book, [Handling Overload: Deciding to
Retry](https://sre.google/sre-book/handling-overload/) (per-request retry budgets, "a failure from a request that has
already been retried 3 times ... is not retried").

**Overlap:** none in Ruff, pylint, Bandit or mypy.

**Corpus:** re-run after this change, every match read: 2 matches, 2 real. khoj `src/khoj/utils/initialization.py:257`
and `:266` (start-up retries creating the admin user and the chat configuration on `except Exception`, with no sleep
and no bound; the helper prompts only when the environment variables are unset, so with them set a database error
spins start-up in a hot loop). The review's not-real match, polar `server/scripts/shell.py:86` (an asyncio REPL's
`except KeyboardInterrupt`), is gone (run over `server/scripts/`, 105 files). Before the review, the writer's runs
removed 12 loop matches one exclusion at a time (listed under "It leaves alone"); a shape-based claim with that
history and two matches starts off, in `log`.

### `python-practice-coroutine-never-awaited`

Rerail's own rule, not a port. Calling an `async def` runs none of its body: it returns a coroutine (an async
generator when the def yields), which runs only when it is awaited, scheduled as a task or iterated. When the
coroutine is thrown away, the work silently never happens; the only sign is `RuntimeWarning: coroutine '...' was
never awaited`, printed when the object is garbage collected, far from the call.

For typed code the team's type checker already reports the plain dropped call: pyright and basedpyright's
`reportUnusedCoroutine` is an error by default and needs no annotations, and mypy's `unused-coroutine` reports it in
the functions mypy checks. This rule's own value is the shapes they let through and code they do not check: an async
def handed to an API that calls it synchronously (no type checker reports it), `_ = f()` (mypy issue 20570), a
coroutine call inside a comprehension or `and`/`or` statement, and untyped code in projects without pyright.

It reports, outside test code:

- **a dropped coroutine** (`DroppedInAsync`, `DroppedInSync`, with advice for where the call is written, and
  `DroppedGenerator` for an `async def` that yields): a coroutine call whose value nothing can read: a statement of
  its own (`notify(user)` instead of `await notify(user)`), the element of a list, set or generator comprehension
  written as a statement (`[notify(u) for u in users]`), an operand of an `and`/`or` written as a statement
  (`ok and notify(user)`), or the value assigned to a name its function never reads (`_ = notify(user)`, also `_` at
  module level). A coroutine call is one whose every target by the call graph (across files, through imports,
  `self.m()` with every override in the run's subclasses) is an `async def` of the run, and which can call nothing
  else;
- **an async def handed to a sync caller** (`HandedToThread`, `HandedToExecutor`, `HandedToAtexit`, `HandedToSignal`,
  `HandedToLoopCallback`, each with the fix that fits the API): an async def of the run, named directly, through an
  import, as `self.m`, inside `functools.partial(...)`, or as a lambda whose body is a coroutine call
  (`target=lambda: notify(user)`), passed to an API that calls it as a plain function and ignores (or only stores)
  what it returns: `threading.Thread(target=...)`, `multiprocessing.Process(target=...)`,
  `threading.Timer(interval, fn)`, `<executor>.submit(fn, ...)`, `<loop>.run_in_executor(executor, fn, ...)`,
  `asyncio.to_thread(fn, ...)`, `atexit.register(fn)` (also as a decorator), `signal.signal(sig, fn)`, and an
  asyncio loop's `call_soon`, `call_soon_threadsafe`, `call_later` and `call_at` (asyncio refuses a coroutine
  function there only in debug mode).

It leaves alone: `await f()`, `asyncio.create_task(f())`, `ensure_future`, `TaskGroup.create_task`, `asyncio.run(f())`,
`return f()`, `x = f()` when the function reads `x` anywhere (also in a nested def), a module-level name other than
`_` (another module may import it), `f()` as an argument (`gather(f(), g())`, `asyncio.wait([f(u) for u in users])`);
a lambda that runs or schedules the coroutine (`lambda: asyncio.run(f())`); a call that may also reach a sync def (a
sync override in a subclass); a def under a decorator from outside the run (`@async_to_sync`: the call graph leaves
the call open) or a workspace decorator returning a sync wrapper; a method resolved through a base outside the run; a
reference to a decorated async def (only `staticmethod`, `classmethod` and `abstractmethod` are known to keep it a
coroutine function); `submit` on a receiver neither built nor named as an executor (`JobQueue().submit(fn)` may await
what it gets); `call_soon`/`call_later` on a receiver not built as an asyncio loop in the same def, module or class
(tornado's `IOLoop`, often named `loop`, runs the awaitable a callback returns); sync defs handed to any of these
APIs; and test code.

**Gaps:** an async def reached through a variable, a container or `getattr`; `executor.map(async_fn, ...)` and
`list(map(async_fn, ...))`; a conditional expression written as a statement; reading a name is not flow-sensitive
(`c = f()` read on one path counts as read); async defs outside the run (a library's coroutine function), which the
call graph does not see; an asyncio loop held in a parameter or another module's variable; `self.hook()` in a
framework base class when a subclass in the run has a base the call graph cannot follow (in fastapi-users, the
`examples/` and `docs/` subclasses `UserManager(UUIDIDMixin, BaseUserManager[User, uuid.UUID])` leave
`self.on_after_register(...)` in `BaseUserManager` open). `loop.add_signal_handler` is left out on purpose: asyncio
always raises `TypeError` for a coroutine function there, so the bug is loud.

**Overlap:** pyright `reportUnusedCoroutine` (default error) and mypy `unused-coroutine` (checked functions only)
report the plain dropped call; neither reports `_ = f()`, and neither reports the second shape. Ruff's RUF006 is about
tasks (`asyncio.create_task(...)` whose result is dropped), not coroutines; flake8-async ASYNC105 is trio only.

**Corpus:** 0 matches, re-run after this change over 14 repositories of under 1,000 `.py` files (letta, khoj, lemur,
freqtrade, open-webui, jupyterhub, mealie, paperless-ngx, label-studio, dispatch, fastapi-users,
full-stack-fastapi-template, linkding, healthchecks; commits in `MANIFEST.tsv`). The review's earlier run over the
other repositories (and polar, which this run did not repeat) found 0 too. Precision cannot be measured on merged
code, which rarely keeps this bug: the work visibly never happens. A seeded copy of fastapi-users (the writer's check:
a `Thread(target=self.on_after_login)` and a dropped cross-module `ping()`) reported both. The rule starts in
`steer` on the strength of its shape: every match is a coroutine that cannot run, and each message's fix only adds an
`await`, a task or a loop entry.

**Sources:**
- Python docs, [Developing with asyncio: Detect never-awaited coroutines](https://docs.python.org/3/library/asyncio-dev.html#detect-never-awaited-coroutines):
  "When a coroutine function is called, but not awaited (e.g. `coro()` instead of `await coro()`) or the coroutine
  is not scheduled with `asyncio.create_task()`, asyncio will emit a RuntimeWarning".
- mypy docs, [Check that coroutine return value is used [unused-coroutine]](https://mypy.readthedocs.io/en/stable/error_code_list.html#check-that-coroutine-return-value-is-used-unused-coroutine):
  "Mypy ensures that return values of async def functions are not ignored, as this is usually a programming error,
  as the coroutine won't be executed at the call site"; mypy issue 20570 (`_ = foo()` escapes the check).
- pyright docs, [Configuration: reportUnusedCoroutine](https://microsoft.github.io/pyright/#/configuration?id=reportunusedcoroutine)
  (an error in the default rule set).
- Python docs, [asyncio event loop: run_in_executor](https://docs.python.org/3/library/asyncio-eventloop.html#asyncio.loop.run_in_executor),
  [asyncio.to_thread](https://docs.python.org/3/library/asyncio-task.html#asyncio.to_thread),
  [threading.Thread](https://docs.python.org/3/library/threading.html#threading.Thread),
  [Executor.submit](https://docs.python.org/3/library/concurrent.futures.html#concurrent.futures.Executor.submit),
  [atexit.register](https://docs.python.org/3/library/atexit.html#atexit.register) and
  [signal.signal](https://docs.python.org/3/library/signal.html#signal.signal): each calls the function it is given
  as `fn(*args)` and keeps or drops the return value; none awaits it.

### `python-practice-contextmanager-cleanup-skipped`

Rerail's own rule, not a port. contextlib runs a generator context manager up to its `yield`, runs the with-block, and,
when the block raises, throws the exception in at the `yield`. Code after the yield then runs only if a handler around
the yield catches the exception and goes on: otherwise the lock stays held, the file descriptor, cursor or connection
leaks, the temp directory stays, and a patched attribute, thread-local, environment variable, signal handler, pending
alarm, `sys.path` entry or working directory stays changed for every later caller.

It reports, outside test code, a `yield` in the own frame of a def decorated with `contextlib.contextmanager` or
`asynccontextmanager` (resolved through imports and aliases) when a release follows it on the normal path: a later
statement of a block that holds the yield, with no `return` or `raise` between them and no handler between them that
swallows the exception. Only two things swallow it: an `except` that catches everything (bare, `Exception`,
`BaseException`) and neither re-raises nor returns, and `with contextlib.suppress(Exception)`. A `finally` runs its own
body and re-raises, a narrow or re-raising `except` and any other `with` let the exception through, so code after them
is skipped. A release is:

- `X.close()`, `aclose()`, `release()`, `unlock()`, `shutdown()`, `stop()`, `terminate()`, `kill()`,
  `disconnect()`, `dispose()`, `rollback()`, `cleanup()`, `unlink()`;
- `os.remove()`, `os.unlink()`, `os.rmdir()`, `shutil.rmtree()`;
- `X.reset(...)`/`X.restore(...)` of something the def touched before the yield (`token = var.set(v)`);
- `T = old` where the def saved `old = T` before the yield (T an attribute or subscript, by its text), and
  `T = False` where it set `T = True` before the yield;
- `del T` or `delattr(o, "a")` of an attribute or subscript the def set before the yield
  (`_context.call = Call(); yield; del _context.call`, `sys.modules[n] = m; yield; del sys.modules[n]`);
- writes to `os.environ` (`os.environ[k] = old`, `del os.environ[k]`, `pop()`, `update()`);
- a removal from a container the def added to before the yield, on the same receiver: an add verb before (`append`,
  `insert`, `add`, `extend`, `push`, `register`, `subscribe`, `attach`, `install`, or a name starting with one, such as
  `addHandler`), a remove verb after (`remove`, `pop`, `discard`, `clear`, `unregister`, `unsubscribe`, `detach`,
  `uninstall`, or a name starting with one, such as `removeHandler`): `sys.path.insert(0, p); yield;
  sys.path.remove(p)`; and `X.pop(k)` after `X[k] = v`;
- a paired call: a call statement after the yield to the same function as a call before it (by resolved name), with
  as many positional arguments, where the function is neither a builtin nor a class of the run and the call before is
  a statement of its own or the whole value of an assignment. Either it restores: it passes, at one position, a name
  holding the state read before the set, where the call before passed something else (`old = get_user();
  set_user(u); yield; set_user(old)`, `os.chdir(old)`, `signal.signal(sig, old)`); or it resets: every argument is a
  neutral constant (0, `None`, `False`, a named constant such as `logging.NOTSET` or `signal.SIG_DFL`) other than
  before (`signal.alarm(n); yield; signal.alarm(0)`, `logging.disable(CRITICAL); yield; logging.disable(NOTSET)`).

The message names the release and its line, and says to leave the code alone when the release must happen only on
success.

It leaves alone: `try: yield x finally: x.close()`; `try: yield except Exception: log()` followed by a release (the
handler swallows the error, so the release runs); `try: yield except: rollback(); raise` followed by `commit()`; code
after the yield that commits, flushes, enqueues, logs or records success (meant to run only on success); a call
computed before and after the yield (`label = str(key); yield label; log.info("%s", str(key))`); a paired call with
the same arguments, or whose later argument is neither saved state nor a neutral constant (`notify("start"); yield;
notify("end")`, `bar.update(0); yield; bar.update(100)`); an early `if x is None: yield; return` (code after it is
unreachable from that yield); `X.cleanup()` of a `tempfile.TemporaryDirectory`, whose finalizer removes the directory
as soon as the generator's frame drops it; `X.reset()` or `del T` of something the def did not touch before the yield;
ASGI lifespan functions (named `*lifespan*`, or passed as `lifespan=`), whose shutdown is the server's; pytest
fixtures (pytest runs the code after a fixture's yield also when the test fails); a yield in a nested def; a
`contextmanager` decorator that is not contextlib's; test files.

**Gaps:** a release in a helper called after the yield (`self._teardown()`) is not followed; a release that is meant
to be skipped on error (a marker file left behind on purpose so the next run retries) is reported, and the message
says to leave it; a named constant used as an event rather than a state (`emit(EVENT_START); yield; emit(EVENT_END)`)
reads as a reset; a logging call that logs a saved value where the call before logged another reads as a restore; a
`with` whose own exit swallows exceptions, other than `contextlib.suppress`, is not known to.

**Sources:** Python docs, [contextlib.contextmanager](https://docs.python.org/3/library/contextlib.html#contextlib.contextmanager)
("If an unhandled exception occurs in the block, it is reraised inside the generator at the point where the yield
occurred. Thus, you can use a try…except…finally statement to trap the error (if any), or ensure that some cleanup
takes place"); pylint
[R1732 consider-using-with](https://pylint.readthedocs.io/en/stable/user_guide/messages/refactor/consider-using-with.html)
("the release of the allocated resources is ensured even in the case of an exception").

**Overlap:** none. Ruff SIM115 and pylint R1732 check `open()`, `Popen`, pools and lock acquires used without `with`;
pylint W0135 `contextmanager-generator-missing-cleanup` warns at a generator that uses such a context manager; none
reads what a generator context manager runs after its `yield`.

**Corpus:** re-run after this change, every match read: 9 matches, 8 real (89%), so the rule stays off.

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| mozilla/addons-server, `src/olympia/core/` and `src/olympia/amo/` | 3 | 3 | |
| HumanSignal/label-studio | 2 | 2 | |
| saleor/saleor, `saleor/webhook/` | 2 | 2 | |
| pretix/pretix, `src/pretix/control/` | 1 | 1 | |
| paperless-ngx/paperless-ngx | 1 | 0 | `src/documents/search/_schema.py:294`: the docstring says the marker is left behind on error on purpose (the case the message tells the reader to leave) |
| polarsource/polar, `server/polar/billing_entry/` | 0 | | |
| letta, khoj, lemur, freqtrade, open-webui, jupyterhub, mealie, dispatch, fastapi-users, full-stack-fastapi-template, linkding, healthchecks | 0 | | |

The large repositories were run over the subtree that holds the review's matches (the fleet refuses 1,000 files or
more). Real: addons-server `olympia/amo/utils.py:329` (`no_jinja_autoescape`: Jinja autoescaping stays off for the
process after an exception in the block), `olympia/core/__init__.py:59` (`override_user`: the thread-local current
user stays overridden) and `:72`; label-studio `core/utils/io.py:74` (fd and temp file leak) and `:81` (temp dir
left); saleor `webhook/observability/utils.py:161` and `:172` (`del _context.api_call` / `gql_operation` after the
yield: on an exception the thread-local stays set for the next request on that thread), newly caught; pretix
`control/views/users.py:71` (session expiry not restored after a failed hijack). The polar false positive
(`billing_entry/service.py:134`, `str(meter_id)` read as a restore) is gone.
