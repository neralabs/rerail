# python-practice: review

Reviewer's audit of the six rules merged into `packs/python-practice/`. Nothing in the pack was changed; the verdicts
below are the output. Every number here was measured for this review (commands in each section and in "Corpus");
the README's corpus tables are the writers' own and were not copied.

Read the rules in this order (by value and confidence, not `pack.json`'s alphabetical order):

1. `python-practice-cancellation-swallowed`
2. `python-practice-blocking-call-reached-from-async`
3. `python-practice-requests-session-without-timeout`
4. `python-practice-retry-without-limit`
5. `python-practice-coroutine-never-awaited`
6. `python-practice-contextmanager-cleanup-skipped`

## 1. Summary

| Rule | Verdict | Proposed mode / on | Corpus matches (repos) | Read precision | Reason |
|---|---|---|---|---|---|
| `python-practice-cancellation-swallowed` | rework | `log` / off; `steer` / on once re-measured | 26 (5) | 69% (18/26) | Real, unlinted bug; on asyncio code at the bar, but anyio code and exception-group re-raises in polar are counted as swallows |
| `python-practice-blocking-call-reached-from-async` | rework | `log` / off; on after the startup fix | 16 (7) | 88% (14/16) | Real stalls in six codebases; startup code reached through one more coroutine is the only miss-fire, plus a `get_running_loop()`-guarded loop start |
| `python-practice-requests-session-without-timeout` | rework (text, one exemption) | `steer` / on | 30 (3) | 100% (30/30) | Every match real; messages twice the cap; `functools.partial(session.request, timeout=...)` is reported |
| `python-practice-retry-without-limit` | rework (text, one exemption) | `log` / off | 4 (3) | 75% (3/4) | Real but rare; the loop form still finds a non-retry `while True` in each new large repository |
| `python-practice-coroutine-never-awaited` | rework (text, two shapes) | `steer` / on | 0 | not measurable | Precise by construction, finds nothing in merged code; pyright/mypy already cover the dropped-call half; misses `_ = f()` |
| `python-practice-contextmanager-cleanup-skipped` | rework | `log` / off | 8 (5) | 75% (6/8) | Finds real leaks (Jinja autoescape left off, thread-local user left set) but fires on `str()`, and misses `del` releases and `signal.alarm(0)` |

Match counts cover the brief's 20 repositories plus three extras the writers had added to `MANIFEST.tsv` (letta,
khoj, lemur), because the 20 alone give too few matches for most rules; each section splits them. Precision counts a
match as real when the harm the message names can happen at that site.

Verdict counts: keep 0, rework 6, cut 0.

Shared findings that apply to every rule:

- **Messages exceed the 200-character cap** (docs/pack-format.md, "Message", criterion 6). Measured with placeholders as
  written: 11 of the 14 messages are over it (blocking 173/173/142 pass; cancellation 356; contextmanager 352;
  coroutine 230/248/233/246; session 390/374; retry 335/273/415). `Dropped` and `HandedToSync` also splice a `{fix}`
  sentence of about 150 characters on top. No check reads messages, so the pack validation passes regardless.
- **Descriptions**: three are not one imperative sentence starting with a verb (criterion 7): cancellation ("An async
  def must re-raise ..."), contextmanager ("A @contextmanager generator must ..."), retry ("A retry must give up: ...").
  The retry description carries code (`stop=`, `max_tries=`, `max_time=`; criterion 3). The blocking description
  leaves out the loop-start clauses (criterion 6).
- **pack.json groups are inconsistent**: two rules are in group `Async`, four in `Python practice`, although
  cancellation-swallowed is as much an async rule as the other two.
- **README**: the contextmanager section ends in the literal placeholder `**Corpus** (pinned commits from
  MANIFEST.tsv): CORPUS_TABLE`. Check 5 (`npx vitest run test/engine/catalog/packs.test.ts -t "pack python-practice"`,
  run for this review: 15 passed) does not catch it.
- Every rule's own `.test` block passes (`node dist/cli/cli.js rules test packs/python-practice/rules/<rule>.rerail`,
  run for each of the six: 1 passed, 0 failed).

## 2. `python-practice-cancellation-swallowed`

**Value.** Real and under-linted. asyncio implements `Task.cancel()`, `asyncio.timeout()`, `wait_for()` and
`TaskGroup` by throwing `CancelledError` into the coroutine; the Python docs say outright that TaskGroup and
`asyncio.timeout()` "might misbehave if a coroutine swallows asyncio.CancelledError" (asyncio-task.html, "Task
Cancellation", checked). flake8-async ASYNC103/104 exist for this (rules page checked) and Ruff has not ported them
(Ruff's rule list, checked for this review: ASYNC100, 105, 109, 110, 115, 116, 119, 210, 212, 220, 221, 222, 230, 240,
250, 251, 401; no 103/104). Frequency: the corpus has 42 non-test `except ...CancelledError` handlers in 7 of the 20
repositories (text count), plus `except BaseException`/bare `except:` handlers around awaits; the rule found swallowing
ones in 4 of the brief's 20 repositories, plus letta (below).

**Correctness.** The logic is the most careful in the pack: path-sensitive re-raise analysis, the drain idiom, the
`cancelling()` check, runner entry points, task-only coroutines found across files through spawn helpers, deferred
re-raise, and nested handlers. Its own cases pass. Counterexamples (`/tmp/review-python-practice/cancel/`, `cancel2/`):

| Shape | Expected | Result |
|---|---|---|
| `async def main()` swallowing at its end, started by `uvloop.run(main())` | silent (same as `asyncio.run`, which is exempt) | **reported** (FP): the runner list is `asyncio.run`, `run_until_complete`, `run_coroutine_threadsafe` only; `uvloop.run`, `asyncio.Runner().run`, `anyio.run` are missing |
| drain where the cancel is in a helper: `self._stop()` (which calls `self._task.cancel()`), then `try: await self._task except CancelledError: pass` | silent | **reported** (FP; the README lists the "other name" form as a gap) |
| drain through a subscript: `self.tasks[k].cancel(); try: await self.tasks[k] ...` | silent | **reported** (FP): `dotted_expr` does not cover subscripts |
| `except CancelledError as e: log; raise e from None` | silent | silent (correct) |
| `except (KeyboardInterrupt, BaseException): return None` | reported | reported (correct) |
| `from asyncio import CancelledError` handler returning | reported | reported (correct) |
| `async with asyncio.timeout(5): try: return await c.get() except CancelledError: return None` | reported | reported (correct: this is the shape that breaks `timeout()`) |
| handler with `if asyncio.current_task().cancelling(): raise` | silent | silent (correct) |
| FastAPI websocket handler ending in `except (WebSocketDisconnect, asyncio.CancelledError): log.info(...)` | debatable | reported: the function ends right after the handler, so "keeps running after it was cancelled" is false; the caller sees a normal return. Low harm under Starlette/anyio, which re-deliver cancellation at the next checkpoint |
| SSE async generator ending in `except CancelledError: log.info("client went away")` | debatable | reported (same shape as the websocket one) |
| `with contextlib.suppress(asyncio.CancelledError): await c.get()` then more code | reported | **missed** (README gap) |
| `finally: return None` around an await | — | missed; Ruff B012 and pylint W0150 already report it |

**Corpus.** Run over all 20 repositories of the brief plus letta (an extra the writers added to MANIFEST.tsv), with
all six rules in one run per repository (superset and dify split, see "Corpus" at the end). Every match read:

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| letta-ai/letta (extra) | 12 | 11 | `server/rest_api/routers/v1/tools.py:753`: the OAuth SSE generator's handler is the last thing it does (the "function ends anyway" shape) |
| freqtrade/freqtrade | 3 | 3 | |
| open-webui/open-webui | 3 | 3 | |
| jupyterhub/jupyterhub | 2 | 1 | `apihandlers/users.py:1097`: the CancelledError comes from the spawn future `iterate_until` wraps, not from this handler's own task |
| polarsource/polar | 6 | 0 | all in the anyio-based metering SDK, three handlers each in `sdk/python/polar/metering/` and its generator template copy: `__init__.py:222` re-raises the caught error inside a `BaseExceptionGroup` (it is not swallowed); `buffered.py:327` records the worker's error and ends the task (anyio re-delivers a scope's cancellation at the next checkpoint, so nothing hangs); `buffered.py:361` catches inside a shielded cleanup scope |
| langgenius/dify, apache/superset | 0 (superset); dify did not finish (900 s limit, and once the content-store limit) in three runs | | |
| the other 14 | 0 | | |

Total 26 matches in 5 repositories (4 of the brief's 20 plus letta), 18 real. Real, sampled: letta `interfaces/openai_streaming_interface.py:264` (and six siblings: "Don't raise the
exception here", the stream keeps yielding after a cancel), `services/tool_executor/tool_execution_manager.py:131`
(a cancelled tool call returns an error result and the agent loop goes on); freqtrade `exchange/exchange.py:3658` and
`:3697` (a cancelled trade download `break`s and returns the partial list as a success), `rpc/telegram.py:144`
(`except BaseException` around every command); open-webui `main.py:1782`, `:1784`, `:1808`; jupyterhub
`app.py:4124` (`asyncio.wait()` never raises a child's CancelledError, so the one caught is the shutdown task's own;
real, low impact). The polar matches are the finding the writer's corpus missed: on anyio code, and with
exception groups, `except BaseException` is not the asyncio swallow the rule describes.

**Overlap.** flake8-async ASYNC103/104 (a flake8 plugin, not Ruff, not the `python` pack). This rule adds what a
single-file linter cannot: the drain idiom and `cancelling()` exemption (ASYNC103 reports them), and the cross-file
"only ever started as a task" exemption. No mypy, pylint, Bandit or framework linter covers it.

**Text.** Description: not imperative ("An async def must ..."; criterion 7); suggest "Re-raise asyncio.CancelledError
from every handler in an async def that catches it; swallowing it breaks cancel(), timeouts and task groups." (131
characters). Message: 356 characters, over the 200 cap; it also asserts "the coroutine keeps running after it was
cancelled", which is false when the handler is the last thing the function does (websocket/SSE shape above). Suggested
(≤200): "Re-raise the CancelledError at the end of this handler (`raise` after the cleanup); swallowing it breaks
cancel(), asyncio.timeout() and TaskGroup. Leave it if this function cancelled the task it awaits."

**Verdict: rework; `log`, off until re-measured, then `steer`, on.** The bug is real and no linter the team runs
catches it, and on plain asyncio code the rule is at the bar (18 of 20 real outside polar's anyio SDK, 90%). Across the corpus it is not:
18 of 26 read matches real, 69%, below the 90% a rule needs to start on, and every miss-fire is a shape that can be named:
(1) count `raise SomeGroup(..., [..., <the caught name>, ...])` (an exception group carrying the caught error) as a
re-raise; (2) extend `pcs_task_end`'s "the handler ends the function" exemption from spawned tasks to any coroutine
whose try is the last statement and whose handler only records or logs (polar `buffered.py:327`, letta `tools.py:753`,
the websocket/SSE counterexamples): the message's "keeps running" is false there; (3) add `uvloop.run`,
`asyncio.Runner(...).run` and `anyio.run` to `pcs_runner_entry`; (4) let the drain compare subscripts
(`self.tasks[k]`) by text; (5) shorten the message and fix the description. Then re-run polar, jupyterhub and letta:
(1) and (2) remove most of the not-real matches above (polar `__init__.py:222`, `buffered.py:327`, letta `tools.py:753`);
jupyterhub `users.py:1097` and polar `buffered.py:361` would stay, so measure before taking `steer`.

## 3. `python-practice-blocking-call-reached-from-async`

**Value.** Real. Python's asyncio-dev "Running Blocking Code" (checked) and the 2xx family of flake8-async/Ruff
establish the bug; Ruff only sees a call written in the `async def`. The airflow issue cited in the notes (#63760,
checked) is a genuine outage of the loop-start variant: a sync `cached_property` reached `async_to_sync` from a
trigger's coroutine, several frames down. The django pack's `sync-orm-reached-from-async` and the fastapi pack's
`sync-session-in-async-route` are the same reach analysis for other sinks.

**Correctness.** Reads well: sure call targets only, offload wrappers (`to_thread`, `run_in_executor`,
`sync_to_async`, decorators), lifespans, once-guards, `self.m()` bound to the class. Counterexamples
(`/tmp/review-python-practice/blocking/`):

| Shape | Expected | Result |
|---|---|---|
| `await asyncio.to_thread(fetch(url))` (the helper is called in the loop, its result handed to the thread) | reported | reported (correct, a nice catch) |
| `c = SyncClient(); c.fetch(url)` with `fetch` doing `time.sleep` | reported | reported (correct: a local instance's method is followed) |
| dual-mode helper: `try: asyncio.get_running_loop() except RuntimeError: return asyncio.run(coro)` then `ensure_future` | silent | **reported** (FP, `LoopStartReached`): the helper only starts a loop when none runs, so it can never raise here |
| helper under `nest_asyncio.apply()` then `run_until_complete` | silent | **reported** (FP, rare) |
| `async def main(): setup_env()` (subprocess) in a CLI run by `asyncio.run(main())` | silent (nothing else on the loop) | reported; the message's "leave it if it runs only before serving" covers it |
| `@functools.lru_cache` helper running `git rev-parse` | silent | silent: the decorator leaves the call open. The same applies to *every* decorated helper, so a tenacity `@retry`-wrapped `requests.get` helper (a common shape) is **missed** |
| helper calling `requests.Session().get(...)` | reported | **missed** (README gap) |
| helper doing `open(path).read()` | — | missed (deliberate, README) |

**Corpus.** All matches read:

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| letta-ai/letta (extra) | 6 | 5 | `server/server.py:420`: `prepare_local_sandbox()` in `init_async`, which the lifespan awaits (startup) |
| khoj-ai/khoj (extra) | 4 | 4 | |
| jupyterhub/jupyterhub | 2 | 1 | `app.py:3612`: `initialize()` runs the alembic upgrade before any task starts |
| mealie-recipes/mealie | 1 | 1 | |
| paperless-ngx/paperless-ngx | 1 | 1 | |
| apache/superset, langgenius/dify | 2 (superset, both real); dify not run (see "Corpus") | | |
| the other 15 | 0 | | |

Total 16 matches in 7 repositories (5 of the brief's 20 plus letta and khoj), 14 real. Real, by file: letta `routers/v1/sandbox_configs.py:146`/`:150` (`pip install` through `subprocess.run`
in a request), `services/mcp_manager.py:821` and `mcp_server_manager.py:992` (`socket.getaddrinfo` per MCP client),
`schemas/providers.py:907` (`requests.post` per model, no timeout); khoj `processor/tools/online_search.py:74`
(`is_internet_connected()`: `requests.head("https://www.google.com")` with no timeout before every online search),
`routers/api_chat.py:220` (text-to-speech `requests.post` in the route), `processor/conversation/anthropic/utils.py:208`
and `google/utils.py:324` (message formatting downloads images with `requests.get`); mealie
`pkgs/safehttp/transport.py:216` and paperless `src/paperless/network.py:178` (the async SSRF-pinning transports
resolve the host with `socket.getaddrinfo` per request); jupyterhub `proxy.py:730` (`start()` may `time.sleep` up to 3 s
while `init_spawners` runs in the background). The khoj matches are new (the writer did not run khoj for this rule).

**Overlap.** Ruff ASYNC210/212/220/221/222/230/251 (the `python` pack ports 210/221/222/230/251) see only calls written
in the `async def`. flake8-async ASYNC200 lets a team configure extra blocking names, still per function. Nothing
follows a helper across modules, and nothing reports `asyncio.run()`/`async_to_sync` reached from a coroutine.

**Text.** Description omits the loop-start half (criterion 6); suggest "Don't call a sync helper that sleeps, does
network I/O, runs a subprocess or starts an event loop from an async def; run it in a thread or make it async." (148).
Messages are within 200 characters; `LoopStartReached`'s advice "give the helper an async twin" is right; the
`BlockingCallReached` advice is safe to follow (moving a helper to a thread never breaks it, short of thread-safety
of the helper's own state, which the message cannot know).

**Verdict: rework; `log`, off as it is, on after change (1).** Read precision is 14 of 16, 88%, under the 90% bar, so it starts off as it is; both not-real matches are startup code, and change (1) below removes the letta one. Changes: (1) extend the startup exemption one level:
a coroutine whose only callers are a lifespan/startup handler (letta `server.py:420`, `init_async` awaited from the
lifespan) is startup code too; (2) exempt a loop start guarded by `asyncio.get_running_loop()` in a `try` whose
`except RuntimeError` holds the loop start; (3) the description fix. Keep `log` until the precision is re-measured
above 90%.

## 4. `python-practice-requests-session-without-timeout`

**Value.** Real. requests' docs: "Nearly all production code should use this parameter in nearly all requests.
Failure to do so can cause your program to hang indefinitely" (`docs/user/quickstart.rst`, checked) and the
advanced page's "By default, requests do not time out unless a timeout value is set explicitly". Bandit B113 and its
port (`packs/python/rules/bandit-b113-request-without-timeout.json`, checked) resolve only `requests.get/options/
head/post/put/patch/delete` imports, so the Session form is unchecked.

**Correctness.** Session binding through locals, `with`, `self.x` across methods and bases, module globals, imports
and factories works on everything I tried. Counterexamples (`/tmp/review-python-practice/session/`):

| Shape | Expected | Result |
|---|---|---|
| `self.session.request = functools.partial(self.session.request, timeout=10)` in `__init__`, then `self.session.get(url)` | silent (the partial supplies a timeout) | **reported** (FP): a known idiom for a session-wide default |
| `self.session.headers.get("X-Token")` | silent | silent (correct) |
| `make().post("/x")` where `make()` returns a Session | reported | reported (correct) |
| `def f(session: requests.Session): session.get(...)` | reported | missed (README gap: parameters) |
| `s.send(prepared)` | reported | missed (README gap) |
| `s.get(u)` inside a lambda in the function that made `s` | reported | **missed**: the local binding is not followed into the lambda's scope |

**Corpus.** Over the 20 repositories of the brief, 1 match (pretix
`plugins/paypal2/client/core/paypal_http_client.py:141`, real: every PayPal API call goes through a Session with a
retry-only adapter). A text search finds 25 `requests.Session()` constructions in non-test code of the 20; I read the
ones the rule left alone (linkding `bookmarks/services/http_client.py:289`, plane `utils/url_security.py:122`,
addons-server `scanners/tasks.py:572` and `check_locales_completion_rate.py:109`, label-studio `ml/api_connector.py:70`,
`data_export/functions.py:97` and `core/utils/io.py:370`, freqtrade `ft_rest_client.py:36`, superset
`utils/network.py:157` and `db_engine_specs/trino.py:306`): each passes `timeout=`, forwards `**kwargs`, or hands the
Session to a library; silence is right. The extras give the volume: Netflix/lemur 24 matches (DigiCert, Entrust,
VeriSign, CFSSL, Azure plugins; `self.session` set in `__init__` with a retry-only adapter; read all 24, all real) and
khoj 5 (Notion and GitHub indexers; all real). Total 30 matches in 3 repositories (1 of the brief's 20 plus lemur and khoj), 30 real; superset and dify (`api/`) 0.

**Overlap.** Bandit B113 / Ruff S113 cover module functions only. Semgrep's registry rule
`python.requests.best-practice.use-timeout` (checked) also covers `$S = requests.Session(...) ... $S.get(...)` in one
scope; this rule adds `self.x` across methods and subclasses, module globals across files, factories, and the
timeout-adapter/`TimeoutSession` exemptions. Not in Ruff, pylint, mypy.

**Text.** Description fine apart from `timeout=` (code; criterion 3): "Pass a timeout to every HTTP call on a
requests Session; a Session has no default timeout, so a stalled server hangs the caller." Messages are 390 and 374
characters. Suggested (≤200): "Pass `timeout=(connect, read)` to this Session call, e.g. `(3.05, 30)`, with the read
timeout above the server's slowest normal answer; requests never times out by default." The steer is safe: a read
timeout counts between bytes, not for the whole download.

**Verdict: rework (text and one exemption); `steer`, on.** Precision is 30 of 30 read matches real; changes are the message length, the description,
and leaving alone a Session whose `request` is replaced by a `functools.partial(..., timeout=...)`.

## 5. `python-practice-retry-without-limit`

**Value.** Real. tenacity's docs: "the default behavior is to retry forever without waiting" (checked); backoff's
README documents `max_tries`/`max_time` as the limits and has no default (checked). Low frequency: tenacity is
imported in 2 of the 20 repositories (dispatch, dify), backoff in 1 (superset); `retrying` in none.

**Correctness.** The tenacity and backoff forms are exact. The hand-written loop form is the risky one: the writer's
README lists 12 loop matches removed one exemption at a time (queue waits, stop signals, prompts, name searches,
lock waits). Counterexamples (`/tmp/review-python-practice/retry/`):

| Shape | Expected | Result |
|---|---|---|
| `@retry(wait=wait_fixed(1), retry=retry_if_result(lambda r: r is None))` | reported | reported (correct) |
| module-level `tenacity.Retrying(wait=wait_fixed(2))` | reported | reported (correct) |
| `while True: try: conn = connect(); break except OSError: time.sleep(1)` (a container's wait-for-db) | reported | reported (correct; the message's "leave it if it must run until it succeeds" fits) |
| reconnect loop `while True: try: client.run_forever(); return except ConnectionError: sleep(5)` | debatable | reported; this is the long-lived-connection case the message tells the reader to leave |
| `done = False; while not done: try: ...; done = True except OSError: sleep(1)` | reported | **missed** (only `while True`/`while 1` are read) |
| `for _ in itertools.count(): try: return connect() except OSError: sleep(1)` | reported | **missed** |
| `@retrying.retry(wait_fixed=1000)` (also forever by default) | reported | missed (README gap) |
| recursion: `except OSError: return recursive(connect)` | reported | missed |
| try nested in an `if` of the loop body | reported | missed (README gap) |

**Corpus.** Every match read:

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| Netflix/dispatch | 1 | 1 | |
| khoj-ai/khoj (extra) | 2 | 2 | |
| polarsource/polar | 1 | 0 | `server/scripts/shell.py:86`: an asyncio REPL's `while True: try: loop.run_forever() except KeyboardInterrupt: ...; continue else: break`; Ctrl-C handling, not a retry |
| apache/superset, langgenius/dify | 0 (superset, dify `api/`) | | |
| the other 16 | 0 | | |

Total 4 matches in 3 repositories, 3 real. Real: dispatch `plugins/dispatch_google/drive/drive.py:117` (`@retry(wait=wait_exponential(multiplier=1,
max=10))` around Google Drive uploads: a revoked credential retries forever); khoj `utils/initialization.py:257` and
`:266` (start-up retries creating the admin user and chat configuration on `except Exception`, no sleep, no bound;
with the admin credentials in the environment a database error spins start-up hot). Note that `_create_admin_user()`
calls `input()` when the environment variables are missing, which the rule's prompt exemption would accept if it were
in the loop body; it is one call down, so the exemption does not see it.

**Overlap.** None in Ruff, pylint, Bandit, mypy. Celery's own `max_retries` defaults to 3; the celery pack owns
`self.retry()`.

**Text.** Description is not imperative and carries code; suggest "Give every retry a limit: an attempt count or a
deadline; an unbounded retry turns a permanent error into an endless loop." (123). Messages are 335, 273 and 415
characters. Suggested: `TenacityWithoutStop` "Add a `stop=` to this tenacity retry, e.g. `stop_after_attempt(5)`;
without one it retries forever. Pass `stop=stop_never` if forever is meant." (≈150); similar for backoff and the loop.
The steer is safe: adding a stop changes only the endless case.

**Verdict: rework (text and one exemption); `log`, off until it has more matches.** Read precision 3 of 4 (75%) on
a sample too small to start it on. Changes: leave alone a loop whose only looping handler catches `KeyboardInterrupt`
(polar's REPL; a person's Ctrl-C, like the prompt exemption); follow the prompt exemption one call down (khoj's
`_create_admin_user()`); shorten the three messages and fix the description. The tenacity/backoff half alone has no
false positive in the corpus and would qualify for `steer`; if the loop form keeps needing per-repository exemptions,
split it into its own rule in `log`.

## 6. `python-practice-coroutine-never-awaited`

**Value.** The bug is real (asyncio-dev "Detect never-awaited coroutines", checked; mypy's `unused-coroutine`, and mypy
issue 20570, checked, which is about `_ = foo()` escaping that check). But it is rare in merged code and already caught
by the type checker most of this stack runs: mypy reports the dropped-call shape in checked functions, and
**pyright/basedpyright's `reportUnusedCoroutine` is an error by default and needs no annotations** (not mentioned in
the notes or README). The second shape (an async def handed to `Thread(target=)`, `executor.submit`, `to_thread`,
`atexit`, `signal`, `call_soon`) is not covered by any of them, and is the rule's unique value.

**Correctness.** Precise by construction (every call target must be an async def of the run, call graph closed).
Counterexamples (`/tmp/review-python-practice/coro/`):

| Shape | Expected | Result |
|---|---|---|
| `m = Mailer(); m.deliver(user)` (async method of a local instance) | reported | reported (correct) |
| `_ = send(user)` | reported | **missed** (the exact case of mypy issue 20570; the rule treats any assignment as "someone may await it") |
| `[send(u) for u in users]` as a statement | reported | **missed** |
| `ok and send(user)` as a statement | reported | **missed** |
| `list(map(send, users))` | reported | missed |
| `Thread(target=lambda: send(user))` | reported | missed (README gap) |

**Corpus.** 0 matches in every repository run (the brief's 20 except dify, which was not run for this rule; plus letta, khoj and lemur).
A cruder single-file probe for the same bug (a bare-name call statement of a function that exists only as an
`async def` in the repository, over fastapi-users, jupyterhub, open-webui, freqtrade, dispatch, mealie, polar, letta)
found nothing either: merged code does not keep this bug, because the work visibly never happens. Precision cannot be
measured; the writer's seeded-bug check on fastapi-users is the only positive evidence.

**Overlap.** mypy `unused-coroutine` (checked functions only), pyright `reportUnusedCoroutine` (default error),
CPython's RuntimeWarning, Ruff RUF006 (tasks, not coroutines), flake8-async ASYNC105 (trio only). The cross-file
resolution adds nothing over a type checker for the first shape; the "handed to a sync caller" shape is new.

**Text.** Description fine. Messages 230 to 248 characters before `{fix}` expands, about 380 after; over the cap.
Each `{fix}` is accurate per API.

**Verdict: rework (text and two shapes); `steer`, on.** Every match it can make is a coroutine that never runs, so `steer` is safe, and the
cost of a silent rule is nil. Changes: catch `_ = f()` and a comprehension/boolean expression statement whose
element is a coroutine call; shorten the messages; say in the README that pyright and mypy cover the dropped-call
shape for typed code, so the pitch rests on the `HandedToSync` shape and on untyped code.

## 7. `python-practice-contextmanager-cleanup-skipped`

**Value.** Real but medium: contextlib's docs state that an exception in the with-block is re-raised at the `yield`
and recommend try/finally (checked). The worst corpus case is a security-relevant one: mozilla
`olympia/core/__init__.py:59` leaves a thread-local "current user" overridden for the rest of the thread when the
block raises. Others are temp-file and descriptor leaks.

**Correctness.** The release vocabulary (close/release/.../restore of a saved value) is reasonable and the "commit
only on success" exemption is right. Counterexamples (`/tmp/review-python-practice/cm/`):

| Shape | Expected | Result |
|---|---|---|
| `cur = conn.cursor(); yield cur; conn.commit(); cur.close()` | reported | reported (correct) |
| `profiler.start(); yield; profiler.stop()` | reported | reported (real: the profiler keeps running) |
| `status = console.status(...); status.start(); yield; status.stop()` (a rich spinner) | debatable | reported (real but cosmetic) |
| `signal.signal(SIGALRM, h); signal.alarm(n); yield; signal.alarm(0)` (the classic timeout manager) | reported | **missed**: a pending alarm fires later in unrelated code |
| `sys.path.insert(0, p); yield; sys.path.remove(p)` | reported | **missed** |
| `logging.disable(CRITICAL); yield; logging.disable(NOTSET)` | reported | **missed** (the "same callee before and after" form needs a name argument) |
| `try: yield except ValueError: raise` then `_lock.release()` | reported | missed (README gap: any except counts as a guard) |
| saleor-style `_context.x = X(); yield; del _context.x` | reported | missed (README gap: `del X.attr`), and the corpus has two real ones (below) |

**Corpus.** Every match read:

| Repository | Matches | Real | Not real |
|---|--:|--:|---|
| mozilla/addons-server | 3 | 3 | |
| HumanSignal/label-studio | 2 | 2 | |
| pretix/pretix | 1 | 1 | |
| paperless-ngx/paperless-ngx | 1 | 0 | `src/documents/search/_schema.py:294`: the docstring says the marker is left behind on error on purpose (the case the message tells the reader to leave) |
| polarsource/polar | 1 | 0 | `server/polar/billing_entry/service.py:134`: "release" `str(meter_id)` at line 197; `str()` was called before the yield and `meter_id` bound before it, so the "puts back a saved value" form fires on a builtin. The message then reads "`str(meter_id)` ... what it releases or restores stays held" |
| apache/superset, langgenius/dify | 0 (superset, dify `api/`) | | |
| the other 13 | 0 | | |

Total 8 matches in 5 repositories, 6 real. Real: addons-server `olympia/amo/utils.py:329` (`no_jinja_autoescape`: Jinja autoescaping stays **off** for
the process after an exception in the block), `olympia/core/__init__.py:59` (`override_user`: the thread-local current
user stays overridden) and `:72`; label-studio `core/utils/io.py:74` (fd and temp file leak) and `:81` (temp dir left);
pretix `control/views/users.py:71` (session expiry not restored after a failed hijack; low impact). Reproduced the
polar false positive in `/tmp/review-python-practice/round3/app/m.py` (`label = str(key); yield label;
log.info("done %s", str(key))` is reported).

Recall check: a single-file AST scan of the 20 repositories for contextmanager generators with statements after a
top-level unguarded `yield` found 17 sites. Besides the ones above it found saleor
`saleor/webhook/observability/utils.py:161` and `:172` (`del _context.api_call` / `del _context.gql_operation` after
the yield; on an exception from the view the thread-local stays set, and the next request on that thread finds it,
is not the root, and neither reports nor clears it): real, and missed (the README's `del X.attr` gap). The rest are
lifespans (excluded by design), commit-on-success code, or logging.

**Overlap.** None for this shape. pylint W0135 `contextmanager-generator-missing-cleanup` (checked) is a different
check: it warns at a generator that *uses* such a context manager. Ruff SIM115 and pylint R1732 cover `open()`/`Popen`
without `with`.

**Text.** Description not imperative ("A @contextmanager generator must ..."); suggest "Release what a
@contextmanager generator acquired in a try/finally around its yield; an exception in the with-block skips code after
the yield." Message 352 characters. README section ends in the `CORPUS_TABLE` placeholder.

**Verdict: rework; `log`, off until re-measured.** Read precision 6 of 8 (75%), and its best real cases (saleor) are
missed. Changes: (1) restrict the "same function called before and after the yield" form to calls whose earlier call
did not just compute a value: exclude builtins and pure conversions (`str`, `int`, `len`, `Decimal`, ...) and require
the argument name to have been assigned from a getter-like call before the first call (`old = get_user();
set_user(u); ...; set_user(old)`), which fixes polar; (2) count `del X.attr`/`del X[k]` of something set before the
yield as a release (saleor); (3) add `signal.alarm(0)`, `sys.path.remove`/`pop`, and a same-callee call with a
constant argument after a different constant before (`logging.disable(NOTSET)`); (4) shorten the message; (5) replace
the README's `CORPUS_TABLE` placeholder.

## 8. Pack-level findings

**Lineup and scope.** `pack.json`'s `covers` says "correct async code, exceptions, typing and packaging", and the
notes' scope adds resource handling. What ships is four async rules, one resource rule and one HTTP/retry pair:
no typing and no packaging rule. The notes drop each typing/packaging candidate for a sound reason (mypy, Ruff TC004
and deptry already enforce them; `.rerail` Python rules do not read `pyproject.toml`), so the fix is to the
description, not the lineup: "Rerail's own Python rules for async code, cancellation, timeouts, retries and resource
cleanup."

**Missing rules, with evidence from this review.**

- `del X.attr` / thread-local cleanup after a contextmanager's yield: not a new rule, a gap of
  contextmanager-cleanup-skipped, but the corpus's clearest real cases are this shape (saleor
  `saleor/webhook/observability/utils.py:161` and `:172`: `_context.api_call` is set before the yield and deleted
  after it; on an exception from the view the thread-local stays set, and the next request on that worker thread
  finds it, is not the root, and neither reports nor clears it).
- Blocking calls reached through objects (`requests.Session().get`, `smtplib.SMTP(...).sendmail`, a sync `httpx.Client`
  method, `subprocess.Popen(...).communicate()`) from an `async def`: a gap of blocking-call-reached-from-async. The
  session rule already binds Session receivers across files; the two could share that binding.
- No other candidate the notes dropped looks wrongly dropped. Spot checks: `lru_cache` on an `async def` (0 corpus
  occurrences, notes' count), `asyncio.get_event_loop()` outside a loop (notes found 10 uses, all in a running loop),
  `gather(return_exceptions=True)` discarded (the drain idiom dominates).

**Rules that belong in another pack.** None. All six are framework-free. `requests-session-without-timeout` borders
python-security (which owns SSRF on the same calls) and is correctly about timeouts only. `blocking-call-reached-from-
async` shares its analysis with `django-practice-sync-orm-reached-from-async` and
`fastapi-practice-sync-session-in-async-route`; the three should stay in their packs (the sinks are framework
APIs) but share one reach implementation (below).

**Library quality.** `rules/lib/python_practice.rerail` is well documented (interface table, background, worked
examples). Issues:

- Dead predicates: `pp_awaited` and `pp_log_call`/`pp_log_name` are declared and defined but used by no rule.
- `pp_test_doc` is the tenth copy of the same test-file predicate (`al_`, `cel_`, `dj_` twice, `fa_`, `pd_`, `pt_`,
  `sec_`, `sa_`): it belongs in core (`python/ast` or a `python/files` lib), so a team's test layout is decided once.
- The async reach (`pp_async_start`/`pp_sync_reach`) is the third implementation of "a sync function reached from an
  `async def` through sure call targets" (django's `sora_*`, fastapi's sync-session rule). One core lib with an
  extensible sink predicate would give the same offload/lifespan/`self.m()` behaviour to all three; today each has
  its own exemptions (only this one knows lifespans and once-guards).
- `pp_blocking_q` restates the blocking-call list that core `misc2-async` already has for the Ruff ports, with
  differences (this one adds sockets and smtplib, leaves out `open`). One list with a flag for "file I/O" would keep
  them aligned.
- `pp_spawn_call` recognises `x.create_task()` on any receiver named `*loop*`, or on any non-identifier receiver;
  `TaskGroup.create_task` on `self._tg` counts as a spawn, which is fine for the cancellation rule but worth a comment.

**Can it be pitched to a team on its own?** Partly. Cancellation-swallowed and blocking-call-reached-from-async are
the pitch: async bugs no linter in the team's toolchain sees, with real outages in the corpus (letta's request path
running `pip install`, freqtrade returning a partial download as success). Session-timeout and retry-without-limit are
solid but narrow (one and two matches across the brief's 20 repositories). Coroutine-never-awaited mostly repeats the
team's type checker. The contextmanager rule has the most real bugs it does not see. As a standalone pack it reads
like an async-safety pack; titled that way it is pitchable, titled "core Python practice" it underdelivers on its own
description.

## 9. Corpus repositories and commits

Corpus root `~/rerail-bench/python-arch/corpus/`, commits from `MANIFEST.tsv` (shallow clones, read-only). Each
repository was run with all six rules in one run:

```
~/rerail-bench/python-arch/bin/corpus-run <repo> review-python-practice-all-<repo> \
  packs/python-practice/rules/python-practice-{blocking-call-reached-from-async,cancellation-swallowed,\
  contextmanager-cleanup-skipped,coroutine-never-awaited,requests-session-without-timeout,retry-without-limit}.rerail
```

Findings are in `~/rerail-bench/python-arch/runs/review-python-practice-*/findings.tsv`. The machine's load average
was about 45 to 55 throughout (other packs' reviews running), which matters for the time-outs below.

| Repository | Commit | .py files | Result |
|---|---|--:|---|
| sissbruecker/linkding | `27b7303baf41` | 229 | 0 |
| healthchecks/healthchecks | `c1039ad23395` | 658 | 0 |
| paperless-ngx/paperless-ngx | `faa6f09f289b` | 424 | 2 (blocking 1, contextmanager 1) |
| HumanSignal/label-studio | `2f42caecbf6c` | 775 | 2 (contextmanager 2) |
| makeplane/plane | `723bb178f97b` | 784 | 0 |
| inventree/InvenTree | `939df605f43d` | 731 | 0 |
| pretix/pretix | `40ea9097d0b2` | 1,228 | 2 (contextmanager 1, session 1) |
| mozilla/addons-server | `7e3374fe5383` | 1,426 | 3 (contextmanager 3) |
| DefectDojo/django-DefectDojo | `cd0a4483a224` | 2,601 | 0 |
| saleor/saleor | `e1ccf4d58002` | 4,385 | 0 |
| fastapi-users/fastapi-users | `e9c57ff7ff2f` | 88 | 0 |
| fastapi/full-stack-fastapi-template | `77e550a73bb4` | 43 | 0 |
| jupyterhub/jupyterhub | `3ca37442a18e` | 158 | 4 (blocking 2, cancellation 2) |
| open-webui/open-webui | `0666e59d26a4` | 330 | 3 (cancellation 3) |
| freqtrade/freqtrade | `7fcb509a434c` | 487 | 3 (cancellation 3) |
| Netflix/dispatch | `dd2837e82a0b` | 717 | 1 (retry 1) |
| mealie-recipes/mealie | `8dbb92808d5a` | 723 | 1 (blocking 1) |
| polarsource/polar | `872272fbfc6e` | 2,522 | 8 (cancellation 6, contextmanager 1, retry 1) |
| apache/superset | `fdf61721d056` | 3,191 | 2 (blocking 2). The six-rule run hit the engine's content-store limit; re-run as three pairs of rules, and the cancellation/contextmanager pair (timed out at 900 s) as single rules: every other rule 0 |
| langgenius/dify (`api/`, 4,921 files) | `724ac15121ee` | 5,191 | retry 0, session 0, contextmanager 0 (each run alone or in a pair). Cancellation-swallowed did not finish: it hit the 900 s limit alone and, in a pair, the content-store limit. Blocking and coroutine have no result: the six-rule run and their pair both timed out at 900 s, and their single-rule runs were still queued when local benchmark runs were stopped on the laptop, so they were **not run**. Those three rules have no dify numbers |
| letta-ai/letta (extra, in MANIFEST.tsv) | `1131535716e8` | 878 | 18 (blocking 6, cancellation 12) |
| khoj-ai/khoj (extra, in MANIFEST.tsv) | `ae229ca894c0` | 252 | 11 (blocking 4, session 5, retry 2) |
| Netflix/lemur (extra, in MANIFEST.tsv) | `3a3c2c201d42` | 337 | 24 (session 24) |

The three extras were already cloned and listed in `MANIFEST.tsv` by the rule writers; I added no repository.
Scratch counterexamples are in `/tmp/review-python-practice/{blocking,cancel,cancel2,cm,coro,session,retry,round3}/`,
run as `review-python-practice-<rule>-cx` (and `-cx2`, `round3-cx`). Recall probes (`cmscan.py`, `coroscan.py`) and
text counts (`probe.txt`) are in the same directory.

## Applied

Applied by the pack fixer; this review's text above is unchanged. Every rule's `.test` block passes
(`nice -n 19 node dist/cli/cli.js rules test packs/python-practice/rules/<rule>.rerail`, 1 passed, 0 failed each).
Every message is at most 200 characters with placeholders as written, and no message splices a `{fix}` sentence.
`pack.json` lists the rules in this review's reading order, with explicit `on`; groups are `Async`,
`Timeouts and retries` and `Resources`; `covers` is the section 8 wording. The README was rebuilt in the same order;
the `CORPUS_TABLE` placeholder is gone.

- **cancellation-swallowed** (`steer`, on): (1) a raise of a `BaseExceptionGroup` (builtin, backport, or a subclass of
  the run, through aliases) that carries the caught exception, directly, in a list/tuple or through a name the handler
  builds, is a re-raise; `ExceptionGroup` is not (it refuses a cancellation). (2) The task-only "handler ends the
  function" exemption now also covers a coroutine no code of the run awaits (handed to a framework, passed uncalled,
  registered by a decorator) whose handler only records or logs, bounded so that a function returning a value after
  the handler, an async generator yielding after it, or one the run awaits stays reported (freqtrade's partial
  download stays). (3) Runners: `uvloop.run`, `asyncio.Runner().run` (also bound by `with`/assignment), `anyio.run`
  and `trio.run` taking the function. (4) Drains compare subscripts by text and follow the cancel into helpers of the
  class, its bases or the module. Also: anyio/trio code (a `BaseException`/bare handler in a function using anyio or
  trio) is left alone, since their scopes re-deliver; `contextlib.suppress(CancelledError)` followed by more code is
  a new violation `CancellationSuppressed`; description and messages as proposed. Not fixed: jupyterhub
  `apihandlers/users.py:1097` (its try also awaits the handler's own work, so whose cancellation it is stays
  undecidable). Re-measured: 21 matches, 20 real (95%): letta 11/11, freqtrade 3/3, open-webui 3/3, jupyterhub 1/2,
  polar `sdk/` 0 (its six not-real matches are gone), letta `tools.py:753` gone; so `steer`, on.
- **blocking-call-reached-from-async** (`log`, on): (1) startup is a fixpoint: a coroutine only startup code awaits,
  at any depth, is startup code; roots gained `add_event_handler` and `on_startup`/`on_shutdown`. (2) A loop start
  behind a "no loop runs here" guard (the `get_running_loop()` try, `is None`/`is_running()` tests, after
  `nest_asyncio.apply()`) is not reported. Also: helpers under same-thread decorators (functools caches, cachetools,
  tenacity, backoff, retrying, stamina) and under wraps-style wrappers of the run are followed, so the review's
  `@retry`-wrapped `requests.get` miss is caught; a memoized helper with no argument counts as once per process.
  (3) Description names the loop-start half. Not fixed: jupyterhub `app.py:3612` (its main coroutine is handed on by
  `partial`; an exemption would also drop the real `proxy.py:730`). `pp_blocking_q` stays: core `misc2-async` exposes
  no list to reuse. Re-measured: 18 matches, 17 real (94%): khoj 7/7 (three newly reached image and model helpers),
  letta 5/5 (`server.py:420` gone), superset `mcp_service/` 2/2, mealie 1/1, paperless 1/1, jupyterhub 1/2; so on.
- **requests-session-without-timeout** (`steer`, on): a Session whose `request`/`send` (or the called verb) is replaced
  on the same instance by a value that sets a timeout (`functools.partial(..., timeout=10)`, an aliased partial, a
  lambda, a wrapper factory or a def of the run naming `timeout`) is left alone; the patch must reach that instance
  (the creating scope, the same local, or the same `self.x`), so one caller patching a factory's Session does not
  exempt another. Also: a closure (`lambda u: s.get(u)`) now binds the enclosing function's Session (the review's
  lambda miss). Description and messages as proposed. Re-measured: 30 matches, 30 real (lemur 24, khoj 5, pretix
  `plugins/paypal2/` 1), the review's 30.
- **retry-without-limit** (`log`, off), split: this id keeps the library forms and adds the `retrying` package
  (`RetryingWithoutStop`) and the `retry` package (`RetryWithoutTries`), the review's `@retrying.retry` miss. New
  **retry-loop-without-limit** (`log`, off) takes the hand-written loop form, with: `KeyboardInterrupt` joining the
  wait timeouts as "not a failure of the work" (polar's REPL); the prompt exemption followed into helpers at any depth,
  but only for a prompt that runs on every turn (khoj's `os.getenv(...) or input(...)` runs only with the variable
  unset, so khoj's two real matches stay reported); and the review's misses `while not done`, `for _ in
  itertools.count()` and a try inside an `if`/`with` of the loop. Recursion stays a gap. Descriptions and messages as
  proposed. Re-measured: the library rule 1/1 (dispatch `drive.py:117`), too few to start on; the loop rule 2/2
  (khoj `initialization.py:257`, `:266`), polar `server/scripts/shell.py:86` gone; both stay off.
- **coroutine-never-awaited** (`steer`, on): `_ = f()` and an assigned name its function never reads, a coroutine
  call as a comprehension element or an `and`/`or` operand written as a statement, and a lambda whose body is a
  coroutine call handed to a sync caller are now reported. `Dropped` and `HandedToSync` were split per context and
  per API (`DroppedInAsync`/`InSync`/`Generator`, `HandedToThread`/`Executor`/`Atexit`/`Signal`/`LoopCallback`) so
  each message is under 200 characters without a `{fix}` splice. The README says pyright and mypy cover the plain
  dropped call for typed code. `list(map(f, ...))` stays a gap. Re-measured: 0 matches in 14 repositories.
- **contextmanager-cleanup-skipped** (`log`, off): (1) the restore form is now a real save/restore: both calls made
  for their effect, the function neither a builtin nor a class of the run, and the restored argument holding state
  read before the set (polar's `str(meter_id)` is silent). (2) `del T`/`delattr` of what was set before the yield is
  a release (saleor). (3) A reset to a neutral constant (`signal.alarm(0)`, `logging.disable(NOTSET)`) and an
  add/remove pair on one receiver (`sys.path`) are releases. Also: only a catch-all `except` that neither re-raises
  nor returns, or `suppress(Exception)`, guards code after the yield (the review's `except ValueError: raise` miss).
  Message shortened; README placeholder replaced. Re-measured: 9 matches, 8 real (89%): addons-server 3/3,
  label-studio 2/2, saleor `webhook/` 2/2 (newly caught), pretix `control/` 1/1, paperless 0/1 (intentional marker),
  polar `billing_entry/` 0 (the `str()` match is gone); just under the bar, so it stays off.

Corpus re-run: all seven rules in one fleet run per repository over the 14 repositories under 1,000 `.py` files
(letta, khoj, lemur, freqtrade, open-webui, jupyterhub, mealie, paperless-ngx, label-studio, dispatch, fastapi-users,
full-stack-fastapi-template, linkding, healthchecks; runs `python-practice-fix-all-<repo>`), and over the subtrees
of the larger ones that hold the review's matches (polar `sdk/`, `server/scripts/`, `server/polar/billing_entry/`;
addons-server `src/olympia/core/`, `src/olympia/amo/`; pretix `src/pretix/control/`, `src/pretix/plugins/paypal2/`;
saleor `saleor/webhook/`; superset `superset/mcp_service/` with `superset/utils/version.py`; runs
`python-practice-fix-sub-*`). Not re-run: dify, DefectDojo, and the rest of those large repositories, where the
review found no match.

Not in scope, from section 8: sharing one async-reach library with the django and fastapi packs, moving
`pp_test_doc` into core (`docs/rule-syntax/lib/python`), and blocking calls through objects
(`requests.Session().get`) are core or cross-pack work this pack cannot do alone. The dead predicates `pp_awaited`
and `pp_log_call`/`pp_log_name` were removed from the library.
