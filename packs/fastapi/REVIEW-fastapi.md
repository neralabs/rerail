# FastAPI pack: review

Reviewer's audit of the nine `fastapi-practice-*` rules, written without the lead's own review. Every number here was
re-measured; the README's corpus numbers were not copied. Commands, counterexamples and corpus repositories are listed
per rule and in section 4.

Read the rules in this order (by the harm a hit prevents and by how much the verdict changes them), not in
`pack.json`'s alphabetical order: sync-session-in-async-route, request-session-in-background-task,
yield-dependency-cleanup, cors-wildcard-with-credentials, response-exposes-secret-field, route-missing-sibling-auth,
route-shadowed, on-event-handler, http-client-per-request.

## 1. Summary

"Matches (repos)" is the sum over the FastAPI corpus repositories that finished (section 4); "read precision" counts
matches whose shape holds and whose advice applies, over matches read (all were read).

| Rule | Verdict | Proposed mode / on | Corpus matches (repos) | Read precision | Reason |
|---|---|---|--:|--:|---|
| `sync-session-in-async-route` | keep | steer / on | 54 (5) | 53/54 (98%) | Real event-loop stalls in five apps, invisible to Ruff's ASYNC rules; one harmless `rollback()` on a fresh scoped session. |
| `request-session-in-background-task` | rework | steer / on, after the change | 4 (1) | 4/4 | All four hits are middleware-owned sessions (dispatch); but on FastAPI ≥ 0.118 a default-scope yield dependency's session is still open in the task, so the rule's main shape and its message are wrong for current FastAPI. |
| `yield-dependency-cleanup` | keep (text fix) | steer / on | 0 (0) | n/a | Exact shape, 4 of 5 counterexample hits right; the "server logs nothing" claim is stale since FastAPI raises `FastAPIError` for a swallowed error. |
| `cors-wildcard-with-credentials` | keep | steer / on | 2 (2) | 2/2 | A breach by configuration; both hits real (one low-impact, bearer-only API); misses `or`-defaults and `**kwargs`, no false positive found. |
| `response-exposes-secret-field` | rework | steer / on | 0 (0) | n/a | Silent on the corpus; counterexamples show two false-positive shapes (an intentional `password` field, a field masked by `@field_serializer`) and narrow name coverage (`password_digest`, `hashed_pw` missed). |
| `route-missing-sibling-auth` | keep | log / on | 0 (0) | n/a | Narrow by design (one outlier write route on a router of ≥ 3 authenticated ones); counterexamples: 1 real, 2 public forms reported, two unauthenticated writes on one router missed. |
| `route-shadowed` | keep | steer / on | 5 (3) | 5/5 | Live bugs: dispatch `GET /cases/minimal`, letta `GET /metadata` ×2 and a duplicated `/{run_id}/metrics`. |
| `on-event-handler` | rework | NeverRuns: steer / on; Deprecated: log / on | 4 (1) | 4/4 as deprecated usage, 0 dead handlers | The "never runs" branch is the value; the "deprecated" message is wrong for `add_event_handler`/`on_startup=` on current FastAPI, `on_event`'s deprecation is already reported by pyright/mypy, and a third-party emitter with a literal `"startup"` is reported. |
| `http-client-per-request` | rework | log / on | 42 (6) | 39/42 (93%) | Real per-request pools in five apps; but an `@lru_cache` dependency that returns a client (the shared-client idiom the message recommends) is reported, and polar's three hits are a template copy and per-credential clients the advice does not fit. |

Verdicts: keep 5 (sync-session, yield-cleanup with a text fix, cors, sibling-auth, route-shadowed), rework 4
(request-session-in-background-task, response-exposes-secret-field, on-event-handler, http-client-per-request), cut 0.

All nine rules' own `.test` blocks pass (`nice -n 19 node dist/cli/cli.js rules test packs/fastapi/rules/<rule>.rerail`,
each `1 passed, 0 failed`).

## 2. Per rule

Counterexamples live in `/tmp/review-fastapi/<dir>/` and were run with
`~/rerail-bench/python-arch/bin/corpus-run /tmp/review-fastapi/<dir> review-fastapi-<rule>-cx packs/fastapi/rules/<rule>.rerail`.
Corpus runs ran all nine rules at once per repository:
`~/rerail-bench/python-arch/bin/corpus-run <repo> review-fastapi-all-<repo> packs/fastapi/rules/fastapi-practice-*.rerail`
(findings in `~/rerail-bench/python-arch/runs/review-fastapi-all-<repo>/findings.tsv`).

### 2.1 `fastapi-practice-sync-session-in-async-route` — keep, steer, on

**Value.** FastAPI runs `async def` routes and dependencies on the event loop; a sync SQLAlchemy query there blocks
every concurrent request for its duration. The primary source holds: FastAPI's
[Concurrency and async / await](https://fastapi.tiangolo.com/async/) says `def` path operations run in a threadpool
"instead of being called directly (as it would block the server)", and
[fastapi-best-practices](https://github.com/zhanymkanov/fastapi-best-practices) says the same. The README's
[discussion #12089](https://github.com/fastapi/fastapi/discussions/12089) is a weak source: it is about an
`AsyncSession` that seemed to block (answer: pool size, aiosqlite), and only uses a `def` route with a sync session as
its control; it shows nothing about a sync session in an `async def`. The BaoDev post (S25) is a small freelance blog;
its latency anecdote is plausible but unverified. Two independent sources remain (FastAPI docs, best-practices guide).
Frequency: five of twelve FastAPI apps in the corpus do it.

**Correctness.** Logic read in full. Counterexamples (`/tmp/review-fastapi/sync-session/`):

| Line | Shape | Result |
|---|---|---|
| `routes.py:10` | `await crud.aget_user(db, uid)`, an `async def` helper doing `db.query()` | missed (documented gap) |
| `routes.py:15` | `db = next(get_db())` then `db.query(...)` in an async route | reported at `next(get_db())`, naming `db.close()` in `get_db`'s `finally`, which does not run there; the real query on line 16 is not reported: calling a generator function is treated as running its body |
| `routes.py:21` | untyped helper `get_user(db, uid)` passed the request session | reported, correct |
| `routes.py:26` | `await run_in_threadpool(lambda: db.query(...))` | silent, correct |
| `routes.py:34` | `async def` yield dependency with a sync session, `db.close()` in `finally` | reported; real (close resets the connection on the loop), low cost |
| `routes.py:43` | `request.app.state.db.query(...)` | missed (only `request.state` is read) |

**Corpus.** 54 matches in 5 repositories, all read: Buuntu 9/9 (`routers/users.py:31,59,76,92,107`,
`routers/auth.py:16,43`, `core/auth.py:29`, the middleware's `request.state.db.close()` at `main.py:23`); dispatch 8/8
(`main.py:156,164,182,184` in `db_session_middleware`, Slack endpoints `plugins/dispatch_slack/endpoints.py:85,110,135,144`);
mealie 11/11 (`routes/auth/auth.py:322,328,399,405,440` OIDC and reverse-proxy logins, `routes/spa/__init__.py:246,251,264`,
`OpenAIService(self.repos)` at `routes/admin/admin_debug.py:38` and `controller_group_ai_providers.py:64,87`);
we-mp-rss 23/23 (`apis/filter_rule.py` ×13, `apis/tags.py` ×10: `async def` CRUD routes on `Depends(get_db)` doing
`db.query(...)`); freqtrade 2/3 (`rpc/api_server/deps.py:33,34` `session.remove()` real; `:29` `Trade.rollback()` on a
scoped session the request has just keyed, no connection checked out, no IO: false). Silent where it should be: the
template (`def` routes), fastapi-users, open-webui, khoj, letta, polar (`AsyncSession`) and zenml (its middleware
offloads the store update with `await to_thread.run_sync(...)`).

Noise note: a route gets one finding per session call (`filter_rule.py:183,184`, `:256,257`); in steer mode that is
several lines for one fix ("make it `def`"). Acceptable, since each line is a separate blocking call.

**Overlap.** Ruff ASYNC210/230/251 (ported in `packs/python`) flag HTTP, `open` and `sleep` written directly in an
`async def`, never a session method and never through a helper; `python-practice-blocking-call-reached-from-async`
leaves SQLAlchemy to this rule (coordinator split, its header line 37); the `sqlalchemy` pack has no sync/async rule;
SQLAlchemy itself warns nothing. mypy, Bandit, pylint: nothing. This rule adds the FastAPI-specific join (which
functions run on the loop, which parameter a `Depends` fills with a sync session) and the cross-file helper walk.

**Text.** Description 165 characters, imperative, accurate. Messages (388 and 390 characters) exceed the 200-character
criterion, as most of the published django pack's do; they name the change and when not to make it (an `async def`
that awaits or sets a ContextVar). Fine otherwise.

**Verdict: keep**, steer, on. Optional follow-ups: do not follow a call of a generator function as if its body ran
(`next(get_db())`), and skip `rollback()` immediately after a scoped session is created.

### 2.2 `fastapi-practice-request-session-in-background-task` — rework

**Value, checked against the source.** The rule's premise is "the yield dependency that owns the session commits,
rolls back and closes it around the response (before the task in 0.106–0.117, racing it since 0.118)". The second half
is wrong for FastAPI's default dependency scope. FastAPI 0.165.0's `fastapi/routing.py` `request_response` (lines
157–163) runs `await response(scope, receive, send)` inside the request's `AsyncExitStack`, and Starlette's
`Response.__call__` runs the background tasks after sending the body, inside that call; so the exit code of a
default-scope (`scope="request"`) yield dependency runs after the background tasks finish. FastAPI's
[Advanced Dependencies](https://fastapi.tiangolo.com/advanced/advanced-dependencies/) confirms 0.118 moved the exit
code back to "after the response is sent" and 0.121 added `Depends(scope="function")` for the early exit. On
FastAPI ≥ 0.118 a task handed the session of a plain `def get_db(): with SessionLocal() as db: yield db` therefore
sees an open session, not a closed one. The FastAPI docs still recommend ids plus a new session in the task as
cleaner code, so the advice is defensible as style, not as the bug the message describes.

Where the bug is real: a session owned by HTTP middleware that commits and closes after `call_next` (dispatch's
`db_session_middleware`, `src/dispatch/main.py`: commit after `call_next`, `close()` and `remove()` in `finally`), a
dependency declared `scope="function"`, a session opened with `with` in the route itself, and apps pinned to FastAPI
0.106–0.117.

**Counterexamples** (`/tmp/review-fastapi/bg-session/`):

| Line | Shape | Result |
|---|---|---|
| `routes.py:11` | default-scope `SessionDep` passed to `add_task` | reported "closed … before or while the task runs": false on FastAPI ≥ 0.118 |
| `routes.py:16` | `Depends(get_db_fn, scope="function")` session passed to `add_task` | reported, correct |
| `routes.py:19-23` | ORM object loaded in `with SessionLocal() as s:` handed to the task (detached) | missed |
| `routes.py:31` | helper `schedule(bg, db)` called from the route | missed (documented gap) |
| `routes.py:40` | `bg.add_task(svc.run)` with `svc` from a dependency built on the session | missed (documented gap) |

**Corpus.** 4 matches, 1 repository, all real: dispatch `src/dispatch/case/views.py:229,277,458,482`, each passing
`db_session` (from `request.state.db`, the middleware's session) to a flow run as a background task; the middleware
commits and closes it right after `call_next` returns, before the task runs. All four are the middleware shape, none
the yield-dependency shape the message describes first.

**Overlap.** None (Ruff, mypy, Bandit, pylint know nothing of background tasks or session ownership).

**Text.** Description fine (145 characters). The message ("closed by its dependency or middleware around the
response, before or while the task runs") is false for default-scope yield dependencies on current FastAPI.

**Verdict: rework.** Split the "request" kind: report in steer only when the session's owner closes it before the
task (an HTTP middleware that assigns `request.state.<x>`, a `Depends(..., scope="function")`, or a `with` block in the
route), with the current message; for a default-scope yield dependency either drop it or report in log with a message
that says what is true ("the task runs inside the request's transaction and holds its connection until it finishes;
on FastAPI 0.106–0.117 the session is already closed: pass ids and open a session in the task"). Then steer / on.

### 2.3 `fastapi-practice-yield-dependency-cleanup` — keep (text fix)

**Value.** Holds: [Dependencies with yield](https://fastapi.tiangolo.com/tutorial/dependencies/dependencies-with-yield/)
uses `try/finally` and says to re-raise in `except`; the 0.110.0 release note made the exception reach the generator.
A close after `yield` outside `finally` is skipped on every failing request. Corpus apps all already use `finally` or
`with` (Buuntu, mealie, freqtrade, template, fastapi-users), so this guards new code (an agent writing
`db = SessionLocal(); yield db; db.close()`), not old code.

One claim is stale. Every cached FastAPI from 0.124.2 to 0.165.0 (`~/.cache/uv/archive-v0/*/fastapi/routing.py`) raises
`FastAPIError("Response not awaited. … a dependency with yield has a block with a bare except … and is not raising the
exception again")` when the error is swallowed, so the server does log a 500 with that hint. What is lost is the
route's own exception and traceback, and an `HTTPException(404)` becomes a 500. The SwallowedError message ("the client
gets a 500 with no log") overstates.

**Counterexamples** (`/tmp/review-fastapi/yield/app/deps.py`):

| Line | Shape | Result |
|---|---|---|
| `:9` | `db = SessionLocal(); yield db; db.close()` | reported, correct |
| `:16` | `except Exception: db.rollback()` without raise | reported, correct |
| `:27` | `except AppError: pass` (project class) | missed (documented) |
| `:37-39` | `except Exception as e: …; raise HTTPException(500) from e` | silent, correct |
| `:53` | `db.close()` in the `else:` of the try | reported, correct (else runs only on success) |
| `:60` | release in a helper `_release(db)` after the yield | missed (documented) |
| `:68` | handler ends in `abort(500)`, a helper that always raises | reported: false positive (documented) |
| `:78` | class dependency `DBDep.__call__` used as `dbdep = DBDep(); Depends(dbdep)` | missed: the README says class `__call__` dependencies are read, but lib/fastapi's `fa_depends_fn` only resolves `Depends(DBDep())` written inline, not a name bound to an instance (FastAPI's own docs use the bound-name form) |

**Corpus.** 0 matches in all 12 repositories.

**Overlap.** None in Ruff, mypy, Bandit or pylint. `python-practice-contextmanager-cleanup-skipped` checks the same
shape for `@contextmanager` generators only, so there is no double report; but the two disagree on one shape: it leaves a
release written after a guarded `try` alone (its gaps), this rule reports it. Pick one behaviour for both.

**Text.** Description 189 characters, fine. UnprotectedCleanup (184) is good. SwallowedError (179): replace "the client
gets a 500 with no log" with "the route's own error and traceback are lost, and an HTTPException turns into a 500".

**Verdict: keep**, steer, on, with the message fix; fix `fa_depends_fn` for instance names (it serves every rule).

### 2.4 `fastapi-practice-cors-wildcard-with-credentials` — keep

**Value.** Verified in Starlette 1.10.0 `starlette/middleware/cors.py:163-164`: with `"*"` and `allow_credentials`,
`send` echoes the request's `Origin` (`allow_explicit_origin`), and `preflight_explicit_allow_origin` is true whenever
credentials are on (`:37`). Any site can then make credentialed reads, as
[PortSwigger](https://portswigger.net/web-security/cors) describes. Sources hold. Frequency: 2 of 12 apps.

**Counterexamples** (`/tmp/review-fastapi/cors/app/main.py`):

| Line | Shape | Result |
|---|---|---|
| `:7` | `allow_origin_regex=r".*"` (raw string) | reported, correct |
| `:11` | `(os.getenv("ORIGINS") or "*").split(",")` | missed |
| `:15` | `add_middleware(CORSMiddleware, **cors_kwargs)` with the wildcard and credentials in a dict | missed |
| `:19` | wildcard + credentials under `if os.getenv("ENV") == "dev":` | reported; real configuration, dev only |
| `:23` | `settings["origins"]` dict lookup holding `["*"]` | missed (documented: dict lookups not settled) |
| `:26` | `allow_origin_regex="https?://.*\\..*"` | missed (only listed regex strings count; acceptable) |

**Corpus.** 2 matches, 2 repositories, both read: open-webui `backend/open_webui/main.py:803`
(`CORS_ALLOW_ORIGIN = os.getenv('CORS_ALLOW_ORIGIN', '*').split(';')` default, real); we-mp-rss `web.py:74`
(`allow_origins=["*"], allow_credentials=True`; real configuration, its API authenticates with a bearer header, so the
practical impact is small). Precision 2/2.

**Overlap.** Semgrep `python.fastapi.security.wildcard-cors` flags the literal `["*"]` regardless of credentials;
flake8-fastapi CF008 is middleware order; Ruff, Bandit, mypy, pylint have nothing. This rule adds the credentials
condition, cross-module values and environment/settings defaults.

**Text.** Description 143 characters, fine. Both messages name the change and the public-API exception; over 200
characters (391, 439).

**Verdict: keep**, steer, on. Cheap recall additions: `x or "*"` defaults, and a `**kwargs` dict literal.

### 2.5 `fastapi-practice-response-exposes-secret-field` — rework

**Value.** The FastAPI docs' own headline for response models
([Response Model](https://fastapi.tiangolo.com/tutorial/response-model/), [Extra Models](https://fastapi.tiangolo.com/tutorial/extra-models/))
and OWASP API3; a breach when it fires. The corpus shows the idiom is followed everywhere (template `UserPublic`,
dispatch `UserRead`, mealie `UserOut`), so the rule guards new code.

**Counterexamples** (`/tmp/review-fastapi/secret/app/`):

| Line | Shape | Result |
|---|---|---|
| `routes.py:9` | `response_model=User`, a table with `password_digest` and `hashed_pw` | missed: names outside the whole-word list |
| `routes.py:12-14` | no response model, returns `crud.get_account(db, id)` (untyped helper returning `db.get(Account, id)`) | missed |
| `routes.py:17-19` | `return jsonable_encoder(db.get(Account, id))` | missed |
| `routes.py:22` | `response_model=Account` (ORM, `hashed_password`, `webhook_secret`) | reported for `hashed_password`, correct; `webhook_secret` not in the list |
| `routes.py:26` | `WifiOut(BaseModel)` with `password: str`, a device password the owner asked for | reported: false positive (intentional) |
| `routes.py:30` | `MaskedOut` whose `password` is masked by `@field_serializer("password")` | reported: false positive |
| `routes.py:34-36` | `return {"account": db.get(Account, id)}` | missed (documented) |

**Corpus.** 0 matches in all 12 repositories. Precision cannot be measured.

**Overlap.** None: Ruff, Bandit, mypy, pylint, pydantic's mypy plugin do not relate a route to its serialized fields.
`pydantic-practice-secret-settings-str` is about settings fields, not responses.

**Text.** Description 147 characters, fine. Messages (277, 302) name the change; they assert "never for clients", which
the two false-positive shapes contradict.

**Verdict: rework**, then steer / on: leave a field alone when the model has a `field_serializer`/`model_serializer`
naming it (or `Field(repr=False, exclude=…)` variants already handled); on ORM models add `password_digest`,
`hashed_pw`, `pwd_hash`, `*_secret` and `*_api_key` suffix names, which are stored values by construction; and add one
sentence to the message for intentional fields ("if this route exists to show the value to its owner, say so and leave
it").

### 2.6 `fastapi-practice-route-missing-sibling-auth` — keep, log

**Value.** OWASP API5 holds; a forgotten `user: CurrentUser` on one write route is a real, common review comment. The
rule is deliberately narrow: POST/PUT/PATCH/DELETE only, at least 3 authenticated siblings, and no other unauthenticated
non-public route on the router.

**Counterexamples** (`/tmp/review-fastapi/auth/app/`):

| File:line | Shape | Result |
|---|---|---|
| `routes.py:13` | `delete_item` without auth beside 3 authenticated routes | reported, correct |
| `routes2.py:12,14` | two new writes (`DELETE`, `PATCH`) without auth beside 3 authenticated routes | both missed: the "no other open route" condition makes each one excuse the other; an agent adding a CRUD pair forgets both at once |
| `routes3.py:11` | delete guarded by `dependencies=[Depends(ensure_org_member)]`, a neutral name whose own dependency is `get_current_user` | silent, correct (the closure is followed) |
| `routes3.py:15` | public `submit_feedback` | reported: false positive (documented) |
| `routes4.py:12` | public `submit_contact_form` with a rate-limit dependency | reported: false positive |

**Corpus.** 0 matches in all 12 repositories.

**Overlap.** None in any linter listed.

**Text.** Description 150 characters, fine. Message (272) names the change and the public exception; good.

**Verdict: keep**, log, on. Precision unmeasured on the corpus, 1 of 3 counterexample hits real; log is right. To
raise recall without noise: allow the outlier condition up to 2 open writes when the router has ≥ 5 authenticated
routes.

### 2.7 `fastapi-practice-route-shadowed` — keep, steer

**Value.** FastAPI's [Order matters](https://fastapi.tiangolo.com/tutorial/path-params/#order-matters) and Starlette's
route priority; the later route is dead code that looks served.

**Corpus.** 5 matches in 3 repositories, all read, all real: dispatch `src/dispatch/case/views.py:152` `GET /minimal`
after `GET /{case_id}` (`:78`); letta `server/rest_api/routers/v1/folders.py:97` and `sources.py:90` `GET /metadata`
after `GET /{folder_id}` / `/{source_id}` (typed `FolderId`, so `/metadata` gets a validation error);
letta `runs.py:217` a second `GET /{run_id}/metrics` handler duplicating `:203`; we-mp-rss `web.py:144` `GET /` after the
SPA catch-all `GET /{path:path}` (`:130`; `.*` matches the empty path, so `serve_root` never runs, though both serve
`index.html`). Silent where it should be: polar's `POST /organizations/` declared twice
(`server/polar/organization/endpoints.py:307` and `:329`, the second with `name="create"`, picked by API version).

**Counterexamples** (`/tmp/review-fastapi/shadowed/app/main.py`):

| Line | Shape | Result |
|---|---|---|
| `:9` | `GET /users/me` after `GET /users/{user_id}` | reported, correct |
| `:13` | app-level `GET /{full_path:path}` SPA catch-all declared before `app.include_router(api, prefix="/api")` | missed: different routers; this shadows the whole API |
| `:23` | `GET /things/latest` at module level after `GET /things/{id}` under `if os.getenv("X"):` | missed (documented: blocks compared separately) |
| `:29` | `GET /items/latest` after `GET /items/{item_id}` with `item_id: int` only in the signature | reported, correct (the path converter is `str`; FastAPI answers 422) |
| `:35` | `@api.get("/x")` after `@api.api_route("/x", methods=["GET", "HEAD"])` | reported as a GET duplicate, correct |

**Overlap.** None (Ruff FAST001–003 do not compare paths; no linter does).

**Text.** Description 164 characters, fine. Messages (246, 210) name the fix.

**Verdict: keep**, steer, on. Worth adding: an app's catch-all `{x:path}` route declared before an `include_router`
of the same app.

### 2.8 `fastapi-practice-on-event-handler` — rework

**Value, checked against the source.** The NeverRuns case is real: FastAPI 0.165.0 `routing.py:2703-2712` uses the
given `lifespan` as the app router's lifespan context, and `_DefaultLifespan` (which runs `on_startup`) only when no
lifespan is given, so `@app.on_event` on such an app never runs. The OnEventDeprecated message overstates for FastAPI:
FastAPI 0.165.0 keeps its own `add_event_handler`, `on_startup` and `on_shutdown` ("Handle on_startup/on_shutdown
locally since Starlette removed support", `routing.py:2731-2740`), without a deprecation marker; only `on_event` is
`@typing_extensions.deprecated`. So "deprecated (Starlette 0.26 deprecated them, Starlette 1.0 removed them)" is right
for a plain Starlette app and wrong for `FastAPI.add_event_handler`/`FastAPI(on_startup=...)`.

**Counterexamples** (`/tmp/review-fastapi/on-event/app/`):

| File:line | Shape | Result |
|---|---|---|
| `extra.py:6` | `@app.on_event("startup")`, `app` imported from a module that builds it with `lifespan=` | reported NeverRuns, correct |
| `extra.py:13` | `app.router.on_startup.append(warm2)` | missed (documented) |
| `extra.py:17` | `pyee.EventEmitter().on_event("startup", fn)`, a library emitter | reported OnEventDeprecated: false positive (the README calls this "none in the corpus") |
| `plain.py:4` | `FastAPI().add_event_handler("startup", ...)` | reported "deprecated … Starlette 1.0 removed them": the call works and is not deprecated in FastAPI 0.165 |

**Corpus.** 4 matches, 1 repository: letta `otel/sqlalchemy_instrumentation.py:547,551` and
`otel/sqlalchemy_instrumentation_integration.py:118,122`, all OnEventDeprecated. The mining notes say these register on
"an app whose constructor passes `lifespan=`", but neither `setup_fastapi_instrumentation` nor
`setup_fastapi_db_instrumentation` is called anywhere in letta (`rg` over the repository), so they are deprecated
`on_event` use in dead helpers: real as deprecated usage, not a dead handler. The NeverRuns branch has no corpus hit.

**Overlap.** `on_event` carries `@deprecated`, so pyright (`reportDeprecated`, Pylance's strike-through) and mypy
(`--enable-error-code deprecated`) already flag every `on_event` the deprecated message covers. Nothing flags a handler
dead under `lifespan=`; that is this rule's value.

**Text.** Description 147 characters, fine. NeverRuns message (342) is accurate and actionable. OnEventDeprecated (412)
needs to drop "deprecated" for FastAPI's `add_event_handler`/`on_startup=` and say what is true: "FastAPI stops calling
these as soon as the app gets a `lifespan=`; prefer the lifespan".

**Verdict: rework.** NeverRuns: steer / on as is. OnEventDeprecated: log / on, with the message fix; drop the
"any receiver with a literal event" branch unless the receiver is an unannotated parameter or a name the run cannot
resolve to an imported class (pyee, socketio and other emitters resolve to their library).

### 2.9 `fastapi-practice-http-client-per-request` — rework, log

**Value.** [aiohttp](https://docs.aiohttp.org/en/stable/client_quickstart.html) ("Don't create a session per request")
and [HTTPX](https://www.python-httpx.org/advanced/clients/) hold. Performance, not breakage; log is right.

**Counterexamples** (`/tmp/review-fastapi/http-client/app/main.py`):

| Line | Shape | Result |
|---|---|---|
| `:9` | `httpx.get(...)` in a route (builds a `Client` per call, the case HTTPX's docs warn about first) | missed |
| `:14` | `aiohttp.request("GET", ...)` in a route | missed |
| `:19` | factory `make_client()` reached from a route | reported, correct |
| `:29` | one `httpx.AsyncClient` per websocket connection, reused for the socket's lifetime | reported; debatable (one per long-lived connection is reasonable) |
| `:37` | client built in `Svc.__init__`, `Svc()` returned by an `@lru_cache` dependency `get_svc` used as `Depends(get_svc)` | reported naming `get_svc`: false positive. `hc_cached` stops the walk at a cached callee but a cached def that is itself a dependency is still a start (`hc_start` = `fa_per_request`); `@lru_cache def get_client(): return httpx.AsyncClient()` + `Depends(get_client)` is the shared-client idiom the message recommends |

**Corpus.** 42 matches in 6 repositories, all read; 39 real per-request pools: open-webui 19 (`main.py:997,1039,2755`,
`routers/auths.py:977,1586`, `configs.py:387,436`, `functions.py:126`, `openai.py:94,844,1038`,
`pipelines.py:246,316,366,409,449,496,537`, `tools.py:491`; aiohttp sessions per call beside its own pool); letta 7
(`routers/v1/anthropic.py:145,201,314`, `zai.py:141,197,310`, `git_http.py:315`); khoj 6
(`processor/tools/online_search.py:577,604,622,640`, `run_code.py:113,328`, reached from the chat websocket; `:113` is
an error path that never closes its session); we-mp-rss 6 (`apis/proxy.py:300,455`, `apis/res.py:61`,
`apis/tools.py:707,842`, `core/cascade.py:316`); mealie 1 (`pkgs/safehttp/flaresolverr.py:44`, a rare escalation path).
Many sit on low-traffic admin routes (open-webui pipelines, version check): real shape, small cost.

Polar's 3 are not real:

- `sdk/python/polar/base.py:257` (`AsyncClientBase.__init__`), reached from `delete_payment_method`
  (`server/polar/organization/endpoints.py:1356`) through `PolarSelfClient._create_portal_sdk`
  (`server/polar/integrations/polar/client.py:933`), which builds an SDK client per call carrying a customer-session
  token in its default headers and closes it with `async with`. The SDK takes no client to share, so the fix needs an
  SDK change: the case the message tells the agent to leave.
- `sdk/generator/python/template/polar/base.py:255`: the same construction, reported again in the SDK generator's
  template, which never runs. The template's package is also named `polar`, so the call graph resolves the import to
  both copies. False positive.
- `server/polar/merchant_migration/adapters/lemon_squeezy.py:93`: `LemonSqueezyAdapter.__init__` builds a client with
  the merchant's own API key in its headers (it accepts `client=` to share one), reached from the migration's `create`
  route (`endpoints.py:138`): one call per migration set-up, per-merchant credentials. Shape holds, advice does not
  pay; counted false.

**Overlap.** None: Ruff/Bandit `S113`/`B113` are about timeouts; `python-practice-requests-session-without-timeout`
likewise.

**Text.** Description 151 characters, fine. Messages are the longest in the pack (515, 571 characters); they carry the
cookie-jar caveat, which matters (open-webui passes per-user cookies), but should be cut to the change plus that caveat.

**Verdict: rework**, log, on: exclude cached defs (`hc_cached`) from `hc_start`; add `httpx.get/post/...` and
`aiohttp.request` as per-call constructions; shorten the messages. The polar hits add two shapes to name in the
gaps (a client whose default headers carry a per-request credential; duplicate package names such as generator
templates); log mode absorbs them, so they do not change the mode.

## 3. Pack-level findings

**Missing rules, with the evidence level.**

- Unauthenticated GET and websocket routes beside authenticated siblings: `route-missing-sibling-auth` leaves both out
  on purpose (GET outliers were public reads). The we-mp-rss miss the README names (`apis/tools.py:321`
  `download_export_file`, its `Depends(get_current_user)` commented out) is a data-export GET. A GET variant gated on
  the route returning a model or file (not a status string) is the next step; no measurement done here.
- A write route on a session whose provider never commits (the route `db.add(x)`s, neither it nor the dependency
  commits): silent data loss, cross-file, FastAPI-specific in where the session comes from. Not measured on the
  corpus; candidate only.
- An app-level catch-all `{x:path}` route declared before `include_router` (section 2.7): `route-shadowed` compares
  routes of one router only.

**Rules that belong elsewhere.** None need to move. `yield-dependency-cleanup` and
`python-practice-contextmanager-cleanup-skipped` should agree on the "release after a guarded try" shape (2.3).
`http-client-per-request` is framework-free in spirit, but its per-request reach needs FastAPI's routes, so it fits here.

**Library quality.** `rules/lib/fastapi.rerail` is documented clause by clause and its interface is reused well across
the nine rules. Gaps that cost recall in several rules:

- `fa_depends_fn` does not resolve `Depends(name)` where `name` is bound to an instance (`checker = Checker(...)`), the
  form FastAPI's docs teach for class dependencies (missed in 2.3; the same hole hides such dependencies from every rule).
- Namespace packages (directories without `__init__.py`, open-webui's `routers/`, dispatch's subpackages) are handled
  per rule by suffix matching (`cors_class_expr`, `ra_ref`), not in the library; `http-client-per-request` misses
  helpers there (README gaps).
- Calls of generator functions are followed as if the body ran (2.1).
- Two packages with the same dotted name (polar's `sdk/python/polar` and its generator template
  `sdk/generator/python/template/polar`) both answer an import, so a cross-file walk reports in code that never runs
  (2.9). Rules could skip a target whose module name has a second definition outside the importer's source root.
- README: three corpus sections are placeholders, not text: `CORPUS_TABLE` under http-client-per-request (line 173) and
  on-event-handler (line 267), `CORPUS_PLACEHOLDER` under route-shadowed (line 581). The NOTES' claim that letta's
  `on_event` helpers sit on a lifespan app is wrong (2.8).
- Messages: 14 of 16 exceed the 200-character criterion of `docs/pack-format.md` ("Message", criterion 6); the
  published django pack does the same (30 of 41), so this is a house-wide drift rather than this pack's alone.

**Can it be pitched on its own?** Yes, after the four reworks. On real FastAPI apps it finds blocking sync sessions
in async routes (54 hits, 98% real, in five apps), shadowed and duplicated routes (5, all real), credentialed wildcard
CORS (2, real) and request sessions handed to background work behind middleware (4, real), none of which Ruff, mypy,
Bandit or pylint report. The other rules are quiet on well-kept code, which is the right default for steer rules that
guard new code.

## 4. Corpus

Shallow clones under `~/rerail-bench/python-arch/corpus/`, commits from `MANIFEST.tsv` (checked with
`git rev-parse HEAD`). The seven FastAPI repositories of BRIEF.md's table, plus the five FastAPI repositories the pack
lead added to the manifest; no repository was added for this review.

| Repository | Commit | Run | Matches |
|---|---|---|--:|
| `fastapi-users/fastapi-users` | `e9c57ff7ff2f` | finished | 0 |
| `fastapi/full-stack-fastapi-template` | `77e550a73bb4` | finished | 0 |
| `open-webui/open-webui` | `0666e59d26a4` | finished | 20 (cors 1, http-client 19) |
| `freqtrade/freqtrade` | `7fcb509a434c` | finished | 3 (sync-session) |
| `Netflix/dispatch` | `dd2837e82a0b` | finished | 13 (sync-session 8, bg-session 4, shadowed 1) |
| `mealie-recipes/mealie` | `8dbb92808d5a` | finished | 12 (sync-session 11, http-client 1) |
| `polarsource/polar` | `872272fbfc6e` | finished | 3 (http-client) |
| `Buuntu/fastapi-react` (lead-added) | `af17f2d422d0` | finished | 9 (sync-session) |
| `letta-ai/letta` (lead-added) | `1131535716e8` | finished | 14 (http-client 7, on-event 4, shadowed 3) |
| `khoj-ai/khoj` (lead-added) | `ae229ca894c0` | finished | 6 (http-client) |
| `zenml-io/zenml` (lead-added) | `61569b1e08e7` | finished | 0 |
| `rachelos/we-mp-rss` (lead-added) | `8fe5bf938743` | finished | 31 (sync-session 23, http-client 6, cors 1, shadowed 1) |

Polar and zenml first sat unstarted in the big-repository queue (3 slots); after the pool grew to 5 slots they were run
with `~/rerail-bench/python-arch/bin/corpus-run <repo> review-fastapi-all-<repo> packs/fastapi/rules/fastapi-practice-*.rerail`
(polar: 3 findings in 2,522 files; zenml: 0 findings in 2,518 files). Zenml's silence includes the gaps the README
names (its example pipeline's `CORSConfig(allow_origins=["*"], allow_credentials=True)` and the server config
validator that fills in `"*"`).

Source versions read: FastAPI 0.165.0 (`fastapi/routing.py`, and the `Response not awaited` error in every cached
version 0.124.2–0.165.0) and Starlette 1.10.0 (`starlette/middleware/cors.py`), from `~/.cache/uv/archive-v0/`.

## Applied

Applied by the pack fixer; corpus numbers re-measured with `corpus-run` over the twelve FastAPI repositories of
section 4 (all rules per repository, runs `fastapi-fix-all-<repo>`), counterexamples re-run from
`/tmp/review-fastapi/<dir>` (runs `fastapi-fix-cx-<dir>`).

- **Library (`rules/lib/fastapi.rerail`).** `fa_depends_fn` now resolves `Depends(name)` where the name is bound to an
  instance in any module (`checker = Checker()`, `Depends(checker)`), so every rule sees class-based dependencies in
  FastAPI's documented form. New `rules/lib/fastapi_events.rerail` holds the event-registration logic both event rules
  share, with receiver resolution (`oe_foreign`) in place of the "any receiver with a literal event" heuristic.
- **sync-session-in-async-route** (keep): messages cut to 193/186 characters, description reworded. Logic unchanged;
  steer / on.
- **request-session-in-background-task** (rework): the rule now resolves who closes the session instead of treating
  every request session alike: an HTTP middleware's `request.state.<x>` (also through a dependency that returns it), a
  `Depends(..., scope="function")` dependency, or a `with` block; a default-scope yield dependency's session (open in
  the task on FastAPI 0.118+) is left alone. One message per owner. Cases: function-scope, middleware (decorator and
  `BaseHTTPMiddleware`), alias re-export, default-scope near misses, `.state` set by a dependency, and the review's
  three missed shapes as documented gaps. Steer / on.
- **yield-dependency-cleanup** (keep, text fix): SwallowedError message now says the route's own error and traceback
  are lost and an HTTPException turns into a 500; a handler ending in a call of a def that always raises (or is
  `NoReturn`/`Never`) counts as raising (`abort(500)` no longer reported); class dependencies bound to a name are read
  (library fix). The "release after a guarded try" shape stays reported (the raise skips it); the disagreement with
  `python-practice-contextmanager-cleanup-skipped` is documented, not changed (another pack). Steer / on.
- **cors-wildcard-with-credentials** (keep): added `x or "*"` / `x or ["*"]` fallbacks, `.split()` on any wildcard
  value, and options passed as `**kw` (dict literal or `dict(...)`); cases for each, plus the dev-only branch and the
  review's two misses as near misses. Messages 196/192. Steer / on.
- **response-exposes-secret-field** (rework): names are read as words (password and hash words in either order, with
  prefixes; `_secret`, `_api_key`, `_secret_key`, `_private_key`, `_password` suffixes on ORM models) and bool fields are
  flags by type; what pydantic never sends is resolved through imports (`SecretStr`, `ClassVar`, `PlainSerializer`,
  `Field(exclude=True)`, `@field_serializer` naming the field, `@model_serializer`), so aliases count and a look-alike
  decorator from another library does not. Messages say when to leave an intentional field. Steer / on.
- **route-missing-sibling-auth** (keep): message 179 characters, description reworded. Log / on.
- **route-shadowed** (keep): messages 190/179. Steer / on.
- **on-event-handler** (rework, split): keeps the id for the never-runs half (steer / on); receivers resolved through
  the new library, so another library's emitter (`pyee`, also aliased) is never an app. Message no longer claims a
  deprecation.
- **lifecycle-event-handler** (new id, the other half): every other registration, log / on, with a message that makes
  no deprecation claim for `add_event_handler`/`on_startup=`.
- **http-client-per-request** (rework): a cached def (`@lru_cache`, resolved through imports and aliases) is no start
  even when it is a dependency; `httpx.get/post/...` and `aiohttp.request` count as per-call clients (also aliased);
  messages 192/197; the polar shapes, duplicate package names and per-websocket clients are named in the gaps. Log / on.
- **Corpus after the fixes** (all twelve repositories finished): sync-session 54 (5 repos, 53 real, 98%);
  request-session 4 (dispatch, all the middleware's session, 4/4); cors 2 (2/2); route-shadowed 5 (5/5);
  lifecycle-event-handler 4 (letta, 4/4 as event-handler usage); http-client 42 (6 repos, 39/42, 93%); yield, secret,
  sibling-auth, on-event-handler 0. Counterexamples: cors 4 hits (raw regex, `or` fallback, `dict(...)` kwargs, dev
  branch); secret 5 hits (`password_digest`, `hashed_pw`, `hashed_password`, `webhook_secret`, the intentional
  `WifiOut.password`), `MaskedOut` silent; yield 4 hits including the bound class instance, `abort(500)` silent;
  bg-session 1 hit (the function-scoped dependency), the default-scope one silent.
- **Pack-level.** README: placeholders replaced, sections and rules table in the reading order above (same order in
  `pack.json`), every rule `"on": true` with the mode proposed. NOTES: the letta `on_event` claim corrected.
  Not done (missing rules, out of scope): unauthenticated GET/websocket routes beside authenticated siblings; a write
  route on a session nobody commits; an app-level `{x:path}` catch-all before `include_router`. Optional follow-ups
  not done: sync-session's `next(get_db())` and post-create `rollback()` shapes; sibling-auth's "two open writes on a
  router of five" recall extension; a library-level fix for two packages with one dotted name and for namespace
  packages.
