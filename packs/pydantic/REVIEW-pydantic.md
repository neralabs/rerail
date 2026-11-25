# Review: the pydantic pack

An audit of the six rules in `packs/pydantic/` by a reviewer who did not write them. The rules, cases, `pack.json`,
README and `NOTES-pydantic.md` were read as merged; nothing in them was changed. Every number below was measured for
this review (the README's corpus numbers are the writers' and were not copied).

Runtime claims were checked against pydantic 2.14.4, pydantic-settings, sqlmodel 0.0.50 and FastAPI in a scratch venv
(`/tmp/review-pydantic/venv`, scripts under `/tmp/review-pydantic/runtime/`). Counterexamples are under
`/tmp/review-pydantic/<rule>/`, corpus runs under `~/rerail-bench/python-arch/runs/review-pydantic-*`. Each rule's own
`.test` block passes (`node dist/cli/cli.js rules test packs/pydantic/rules/<rule>.rerail`: 6 of 6 pass).

## Summary

| Rule | Verdict | Proposed mode / on | Corpus matches (repos) | Read precision | Reason |
|---|---|---|--:|--:|---|
| `pydantic-practice-v1-api-in-v2` | rework | split: removed-in-v2 group steer / on; deprecated group log / off | 173 (6) | 53 read, 53 real deprecated uses; 0 crash-group matches | One id mixes two hazards: a real crash/silent-ignore group and a deprecation group that mypy's `deprecated` code already reports on typed code and that fires 161 times in one repo. SQLModel `from_orm` is a false crash report. |
| `pydantic-practice-dump-mode-json` | rework | steer / on | 0 (0 of 14) | no denominator | Sound and precise, but misses its stack's own sink (Starlette/FastAPI `JSONResponse(content=m.model_dump())`, which raises the same TypeError), aiohttp, and a client stored on `self`; message is 346 chars. |
| `pydantic-practice-validator-returns-value` | keep (text fix) | steer / on | 0 (0 of 14) | no denominator | Exact, cross-file; adds the 10 of 16 case shapes RET503 cannot see. One false-positive shape (a validator ending in a helper that always raises). Bare-`return` message would undo a deliberate None. |
| `pydantic-practice-default-factory` | keep (text fix) | steer / on | 0 (0 of 14) | no denominator | Exact bug with a real source (the accepted SO answer has it); RUF009 does not cover models. Message is 376 chars. |
| `pydantic-practice-before-validator-raw-input` | rework | log / on | 7 (4) | 7/7 | Real mechanism (a 500 instead of a 422, reproduced), but FastAPI `Depends()` query models are a false-positive shape, the message's fix is wrong for `plain` and `wrap` validators, and `Annotated[..., BeforeValidator(fn)]` (48 files in polar) is unread. |
| `pydantic-practice-secret-settings-str` | rework | log / on | 160 (5) | 159/160 | Corpus matches are real, but a name deny-list lets `TOKEN_AUDIENCE`, `TOKEN_ISSUER`, `PASSWORD_HASH_SCHEME`, `AUTH_TOKEN_COOKIE`, `TOKEN_ENDPOINT`, `*_TOKEN_SCOPES` through (6 of 6 counterexamples reported); message is 512 chars. |

Verdicts: keep 2, rework 4, cut 0.

**Reading order.** `pack.json` lists the rules alphabetically. Read them by what they prevent: (1) `v1-api-in-v2`'s
removed-in-v2 group (an import-time crash, or `orm_mode`/`Config.fields` silently ignored), (2) `dump-mode-json` (a
code path that raises every time), (3) `validator-returns-value` (a field or a whole model silently None),
(4) `default-factory` (one id or timestamp shared by every instance), (5) `before-validator-raw-input` (a 500 instead
of a 422 on malformed input), (6) `secret-settings-str` (a secret printed if the settings object is ever logged), and
last the deprecation half of `v1-api-in-v2`.

## `pydantic-practice-before-validator-raw-input`

### Value

The mechanism is real and reproduced with pydantic 2.14.4: a `mode="before"` validator doing `v.strip()` raises
`AttributeError` out of `A.model_validate({"x": 123})` and `{"x": None}`; `v["k"]` on an int raises `TypeError`; neither
becomes a `ValidationError`. Through FastAPI the same model as a body returns **500** for `{"name": 123}`
(`runtime/fa.py`). The classmethod after-validator slip is also real: the `(cls, values)` def receives the model
instance (with `PydanticDeprecatedSince212`). Sources hold up: the validators page's "raw input, which in theory could
be any arbitrary object", the migration guide's "TypeError is no longer converted", and
[fastapi#4974](https://github.com/fastapi/fastapi/issues/4974) (opened: "Model with a root validator in `Depends`
leads to 500 error instead of 422").

How often it hurts: only when a client sends the wrong JSON type, and the outcome is one failed request with a 500
and a stack trace, not data loss. A lead would flag it on a public API; many teams treat it as robustness polish.
Severity medium at most.

### Correctness: counterexamples (`/tmp/review-pydantic/bvr`, run `review-pydantic-before-validator-raw-input-cx`)

| # | Shape | Expected | Result |
|---|---|---|---|
| CX1 | `def list_items(q: Annotated[ItemQuery, Depends()])`, before validator `v.strip() if v else v` | silent: FastAPI validates each query parameter as `str` before calling `ItemQuery(...)` (checked: the validator got `str` for `?q=123`) | **reported** (`app/schemas.py:11`): false positive |
| CX2 | `if isinstance(v, list): return v` then `v.split(",")` | reported (int raises) | silent: miss (documented gap) |
| CX3 | `slug: Annotated[str, BeforeValidator(lambda v: v.strip().lower())]` on a route body | reported | silent: miss (documented gap) |
| CX4 | `mode="plain"` validator `return v.strip()` | reported | reported (`:35`), but see the message problem below |
| CX5 | before model validator on a model only built as `Internal.model_validate(src.model_dump())` | silent (project's own dict) | **reported** (`:44`): false positive (documented as an exposure gap) |
| CX6 | `mode="wrap"` validator `handler(v.upper())` | reported | reported (`:54`), message wrong for wrap |
| CX7 | `if "kind" not in data:` in a before model validator | reported (`in` on an int raises TypeError) | silent: miss (documented gap) |

**Message is unsafe for two modes.** It says "Return `{x}` unchanged unless `isinstance({x}, ...)`, so Pydantic's own
validation reports it". That is true for `before` only. Checked at runtime: a `plain` validator that returns a
non-str `v` unchanged gives `P.model_validate({"x": 123}).x == 123` on a `str` field (plain validators replace
Pydantic's validation, so nothing reports it: a silent wrong type); a `wrap` validator that returns `v` unchanged
skips `handler` and gives the same `123`. Following the steer turns a loud 500 into silent bad data.

### Corpus

| Repository | Matches | Real | Note |
|---|--:|--:|---|
| freqtrade | 1 | 1 | `freqtrade/rpc/api_server/api_schemas.py:548`, `values.get` in `DownloadDataPayload` (request body) |
| mealie | 1 | 1 | `mealie/schema/recipe/recipe_ingredient.py:253`, iterates raw `v` inside `except AttributeError`; an int raises `TypeError` |
| letta | 4 | 4 | `letta/schemas/llm_config.py:142, 168, 215, 254`, `values.get` in four before model validators of `LLMConfig` |
| polar | 1 | 1 | `server/polar/organization/schemas.py:424`, `data.get("url", "")` in `OrganizationSocialLink.validate_url` (a list field of the organization update body); the `isinstance` check comes one line too late |
| the other 10 run | 0 | | full-stack-template, fastapi-users, fastapi-react, open-webui, khoj, we-mp-rss, dispatch, dify, superset, zenml |

All matches read. The misses matter more than the false positives here: in polar, `Annotated[..., BeforeValidator(fn)]`
appears in 48 non-test files against 63 decorator validators, and one of those functions uses raw input unguarded on a
request body: `polar/agent/schemas.py:102` `normalize_claim_code` (`value.strip()`, on `AgentClaimConfirm.code`, the
body of `confirm_claim` at `polar/agent/endpoints.py:138`). The rule cannot see it. (`scope_to_list`, also unguarded,
feeds only the `AuthorizeResponseBase.scopes` response field, so it is not foreign input; most of polar's other
`BeforeValidator` functions already guard with `isinstance`.)

### Overlap

None found. Ruff 0.17.6 has no Pydantic-aware rule (only FAST001 and annotation-style rules mention Pydantic),
flake8-pydantic's PYD001-PYD010 are style checks, mypy and the pydantic mypy plugin accept `v.strip()` on an `Any` or
`str` parameter, pylint and Bandit have nothing.

### Text

- Description (180 chars): accurate, one imperative sentence; fine.
- `RawInputUnchecked`: 322 chars (criterion 6: at most 200), and wrong advice for `plain`/`wrap` (criteria 2 and 4).
- `AfterValidatorGetsInstance`: 274 chars; advice correct.

### Verdict: rework, log / on

1. Exclude route parameters whose default or `Annotated` metadata is `Depends()` (FastAPI builds those models from
   already-typed query/path values).
2. One message per mode: `before` keeps "return the value unchanged unless isinstance"; `wrap` says "call
   `handler(v)` unchanged unless isinstance"; `plain` says "raise ValueError unless isinstance" (never return it
   unchanged). Keep each under 200 characters.
3. Read `BeforeValidator(fn)` / `WrapValidator(fn)` / `PlainValidator(fn)` in a field's `Annotated[...]` (the
   function's first parameter is the value), at least for named functions; the extra real case found in polar during
   this review (`normalize_claim_code`) is of this form.
4. Stay in `log`: 7 of 7 real is too small a denominator, and the earlier rounds (README: 0 of 12 on zenml before the
   wrapper fix) show each new codebase brings a new exposure shape.

## `pydantic-practice-default-factory`

### Value

Real and easy to write. The Stack Overflow source holds up and is stronger than the README says: the *accepted*
answer to [73128975](https://stackoverflow.com/questions/73128975) uses `created_at: datetime = datetime.now()`, and
only a later, lower-voted answer points out every instance shares the timestamp. For ids it means duplicate primary
keys. The Pydantic Fields docs and SQLModel's UUID page use `default_factory`. Frequency in mature code is low: 0
corpus matches (below), consistent with the writer's AST scan.

### Correctness: counterexamples (`/tmp/review-pydantic/dflt`, run `review-pydantic-default-factory-cx`)

| # | Shape | Result |
|---|---|---|
| CX1 | `BootInfo.started_at: datetime = datetime.now(timezone.utc)` (a deliberate process-start time) | reported (`app/models.py:9`); the message's last sentence covers this ("Keep the call only if every instance must share the import-time value") |
| CX2 | `id: Annotated[str, Field(default=str(uuid4()))]` | silent: miss (documented gap) |
| CX3 | `queued: float = time.monotonic()` | silent: miss (`time.monotonic`, `time.perf_counter` are not in the per-call list) |
| CX4 | `run_id: str = f"run-{uuid4().hex[:8]}"` | reported (`:15`): the call inside an f-string is found |
| CX5 | `created: datetime = Field(datetime.now)` (the function object as the default) | silent; a different bug (the field holds a function), out of scope |

No false positive. The settings-class exclusion is right (a factory there would make two `Settings()` disagree).

### Corpus

0 matches in every repository run (table at the end). No denominator; the rule is a guard for new code.

### Overlap

Ruff 0.17.6 `--select RUF009,B008` on the rule's own cases: RUF009 reports 3 stdlib-dataclass lines and B008 1
parameter default, all near misses the rule leaves alone; none of the 27 Pydantic lines it reports. flake8-pydantic
has no such check. What the rule adds is real.

### Text

- Description (149 chars): fine.
- `DefaultEvaluatedOnce`: 376 chars, over the 200 cap. Content is right (names the call, the factory form, and when to
  keep it). Suggested: "{fn} runs once at import, so every instance shares its value. Use `{maker}(default_factory=...)`
  (e.g. `uuid4`, `lambda: datetime.now(timezone.utc)`) unless all instances must share it." (about 180 with
  placeholders as written).

### Verdict: keep, steer / on

Shorten the message. Optional additions: `time.monotonic`/`perf_counter`, and a default inside `Annotated[...,
Field(default=...)]`.

## `pydantic-practice-dump-mode-json`

### Value

Real: `json.dumps(E(...).model_dump())` with a `datetime` raises `TypeError: Object of type datetime is not JSON
serializable` every time. The three incident sources open and match the README's description:
[awslabs/mcp#1990](https://github.com/awslabs/mcp/issues/1990) ("datetime serialization error"),
[crewAI#7358](https://github.com/crewAIInc/crewAI/issues/7358) ("crashes with TypeError when Flow state contains
datetime, UUID, or set fields"), [google/adk-python#6747](https://github.com/google/adk-python/issues/6747). The
decision to report only models with a proven non-JSON field is the right precision call.

### Correctness: counterexamples (`/tmp/review-pydantic/dump`, run `review-pydantic-dump-mode-json-cx`)

| # | Shape | Result |
|---|---|---|
| CX1 | `return JSONResponse(content=e.model_dump())`, `Event.at: datetime` | silent: **miss**. Starlette's `JSONResponse.render` is `json.dumps`; checked: it raises the same TypeError. This is the FastAPI-stack form of the bug, and the stack this pack is for. |
| CX2 | `async with aiohttp.ClientSession() as s: await s.post(url, json=e.model_dump())` | silent: miss (aiohttp's default `json_serialize` is `json.dumps`) |
| CX3 | `self.client = httpx.Client()` in `__init__`, `self.client.post(json=p.model_dump())` with `Decimal` | silent: miss (documented: only clients built in the same function) |
| CX4 | `data = {"event": e.model_dump()}; json.dumps(data)` | reported (`app/views.py:30`) |
| CX5 | `json.dumps(e.model_dump(), default=str)` | silent, correct |
| CX6 | `json.dumps(e.model_dump() if e else {})` | silent: miss (conditional expression not followed) |

No false positive among them.

### Corpus

0 matches in every repository run. The writer's 3 real matches are in repositories found by searching GitHub for the
pattern (`_wild-pydantic-dump/`), not in the brief's corpus; they were not re-run here. In the corpus, fastapi-users'
`JSONResponse(bearer_response.model_dump())` (`fastapi_users/authentication/transport/bearer.py:26`) is the
`JSONResponse` shape on a JSON-native model (two `str` fields), so a `JSONResponse` sink would stay silent there too.

### Overlap

None: `model_dump` returns `dict[str, Any]`, so mypy/pyright accept every sink; Ruff, Bandit, pylint and
flake8-pydantic have nothing.

### Text

- Description (139 chars): fine.
- `DumpNotJson`: 346 chars, over the cap; content correct. Suggested: "`{model}` field `{field}` is not
  JSON-encodable in Python mode, so {sink} raises TypeError. Use `.model_dump(mode=\"json\")`, unless the encoder's
  `default=` handles it." (about 170 as written).

### Verdict: rework, steer / on

Add Starlette/FastAPI `JSONResponse(content=...)` (positional or `content=`) as a sink, and aiohttp's
`ClientSession` methods' `json=`; shorten the message. The precision design stays.

## `pydantic-practice-secret-settings-str`

### Value

`repr(Settings())` and `model_dump()` print a `str` secret in clear and a `SecretStr` as `'**********'` (checked).
The hazard needs a second step (the settings object reaching a log, an error report or a response), which the rule
does not check; the README's one concrete case (mealie's start-up `model_dump_json(exclude=...)` missing
`REVERSE_PROXY_AUTH_SECRET`) is a good one. Sources (SecretStr docs, pydantic-settings docs, OWASP, CWE-532) are
general guidance, not incident reports. Value: real hygiene, medium at most; the fix touches every reader, so `log`
is right.

### Correctness: counterexamples (`/tmp/review-pydantic/secret`, run `review-pydantic-secret-settings-str-cx`)

| # | Field | Result |
|---|---|---|
| CX1 | `JWT_TOKEN_AUDIENCE: str = "api"` | **reported**: false positive |
| CX2 | `TOKEN_ISSUER: str = "https://auth"` | **reported**: false positive |
| CX3 | `PASSWORD_HASH_SCHEME: str = "argon2"` | **reported**: false positive |
| CX4 | `AUTH_TOKEN_COOKIE: str = "access_token"` | **reported**: false positive |
| CX5 | `TOKEN_ENDPOINT: str = "/oauth/token"` | **reported**: false positive |
| CX6 | `GITHUB_TOKEN_SCOPES: str = "repo"` | **reported**: false positive |
| CX7-9 | `DB_PASS`, `FERNET_KEY`, `OPENAI_KEY` | silent: misses (abbreviations and keys without a listed word) |
| CX10-11 | `SECRET_KEY: str`, `SMTP_PASSWORD: str \| None` | reported, correct |

The cause is the shape of the name test: any secret word anywhere in the name, minus a deny-list of last parts (`_url`,
`_name`, `_ttl`, ...). Every unlisted last part (`audience`, `issuer`, `scheme`, `cookie`, `endpoint`, `scopes`,
`policy`, `template`, `location`, `backend`) is reported. The corpus repositories happen not to have such names; the
next one will.

### Corpus

| Repository | Matches | Real | Note |
|---|--:|--:|---|
| full-stack-fastapi-template | 3 | 3 | `backend/app/core/config.py:23, 47, 66`: `SECRET_KEY`, `SMTP_PASSWORD`, `FIRST_SUPERUSER_PASSWORD` |
| mealie | 4 | 4 | `mealie/core/settings/settings.py:185, 186, 504`, `db_providers.py:38` |
| letta | 25 | 25 | `letta/settings.py`: 20 `*_api_key`, `modal_token_secret`, `aws_secret_access_key`, `pg_password`, `clickhouse_password`, `default_token` ("Default token for Letta client") |
| polar | 35 | 34 | all `server/polar/config.py` (`STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `GITHUB_CLIENT_SECRET`, `AWS_SECRET_ACCESS_KEY`, `DISCORD_BOT_TOKEN`, `POLAR_ACCESS_TOKEN`, ...); not real: `:373` `POSTHOG_PROJECT_API_KEY`, PostHog's public write-only project key |
| dify | 93 | 93 | `api/configs/**` (79: `SECRET_KEY`, `DB_PASSWORD`, `REDIS_PASSWORD`, `S3_SECRET_KEY`, 30-odd vector-store passwords and API keys, `PASSWORD_PEPPER`, ...) and `dify-agent/src/dify_agent/server/settings.py` / `runtime_backend/profile.py` (14: `server_secret_key`, `api_token`, `*_auth_token`, `e2b_api_key`, ...); every one a credential |
| the other 9 run | 0 | | fastapi-users, fastapi-react, open-webui, freqtrade, khoj, we-mp-rss, dispatch, superset, zenml |

### Overlap

Ruff S105 on the rule's cases reports 13 lines: 2 overlap with the rule's 14 (`POSTGRES_PASSWORD`,
`FIRST_SUPERUSER_PASSWORD`, both literal defaults); the other 11 are S105's own false positives (`TOKEN_TYPE`,
`JWT_SECRET_ALGORITHM`, `TOKEN_URL`, ...) or test lines. The type-based check is not covered by any linter.

### Text

- Description (162 chars): fine.
- `PlaintextSecretSetting`: 512 chars, two and a half times the cap. It carries the right content (the read count and
  the `'**********'` trap). Suggested: "Declare `{name}` as `SecretStr` and read it with `.get_secret_value()` at each
  of its {reads} uses; a read left as is gets `'**********'`. Keep `str` if it is not a secret." (about 170 as written).

### Verdict: rework, log / on

Replace the deny-list with an allow-list of final parts: report when the last word part is a secret word
(`password`, `passwd`, `secret`, `token`, `credential(s)`, `apikey`, `pepper`), or the last part is `key` and a
secret word or `api`/`private`/`access` precedes it (`SECRET_KEY`, `API_KEY`, `AWS_SECRET_ACCESS_KEY`). Simulated with
a Python regex over the names (not the engine): of the 160 corpus matches in full-stack-template, mealie, letta, polar
and dify, 159 keep matching (dropped: dify's `INNER_API_KEY_FOR_PLUGIN`); all 14 `# caught` names in the rule's
cases keep matching; CX1-CX6 drop out. Shorten the message.

## `pydantic-practice-v1-api-in-v2`

### Value

Two different hazards in one rule.

- **Group 1 (removed in v2)** is real and checked at runtime: `Field(regex=)` raises `PydanticUserError`, a post
  `@root_validator` without `skip_on_failure` raises at class creation, `.json(indent=2)` raises `TypeError`,
  `from_orm` without `from_attributes` raises, and `orm_mode = True` in a nested `Config` is renamed-and-ignored
  (warning only), so `from_attributes` stays off. The renamed/removed `Config` keys are the most valuable part: they
  fail silently.
- **Group 2 (deprecated, removed in v3)** is maintenance. Every call still works on pydantic 2.x.

### Correctness: counterexamples (`/tmp/review-pydantic/v1`, run `review-pydantic-v1-api-in-v2-cx`)

| # | Shape | Result |
|---|---|---|
| CX1 | `HeroRead.from_orm(row)`, `HeroRead(HeroBase)`, `HeroBase(SQLModel)` | **reported as `FromOrmCrash`** (`app/services.py:6`): false. SQLModel sets `from_attributes=True` on its base config (checked on a direct `SQLModel` subclass: `model_config["from_attributes"] is True`, and `from_orm` returns the instance with only SQLModel's deprecation warning). The crash message would send an agent to add config that is already there. |
| CX2 | `UserRead` with `model_config = ConfigDict(orm_mode=True)`, `UserRead.from_orm(row)` | reported `FromOrmCrash` (`:10`), correct |
| CX3 | `if path.startswith(settings.API_V1_STR): return body.dict()` | silent: the shim test (`if` text matching `v1`, `v2`, `version`, `pydantic`, ... case-insensitively) takes any condition mentioning `API_V1_STR` (the full-stack template's own setting name) as a compatibility shim |
| CX4 | `load().dict()` (instance from a function's return) | silent: documented gap |

### Corpus

| Repository | Matches | Group | Real |
|---|--:|---|--:|
| Netflix/dispatch | 161 | 2 (117 `.dict()`, 21 `.json()`, 12 `.from_orm()`, 6 `pydantic_encoder`, 4 `.parse_raw()`, 1 `.update_forward_refs()`) | 161 deprecated uses |
| khoj | 8 | 2 (5 `Field(min_items=)`, 2 `.json()`, 1 `class Config`) | 8 |
| open-webui | 1 | 2 (`@validator`, `backend/open_webui/routers/ollama.py:1072`) | 1 |
| letta | 1 | 2 (`class Config`, `letta/schemas/letta_message.py:255`) | 1 |
| dify | 1 | 2 (`class Config` with `populate_by_name`, `api/core/workflow/nodes/knowledge_retrieval/retrieval.py:42`) | 1 |
| zenml | 1 | 2 (`class Config` with `arbitrary_types_allowed`, `examples/agent_comparison/materializers/prompt.py:31`, example code) | 1 |
| the other 8 run | 0 | | full-stack-template, fastapi-users, fastapi-react, freqtrade, we-mp-rss, mealie, polar, superset |

Read: every non-dispatch match, and every fourth dispatch match by path (41), 53 in all. Each receiver resolves to a
v2 model (dispatch's are parameters annotated with schemas deriving from `DispatchBase(BaseModel)`, which sets
`from_attributes=True`). All are real deprecated uses; none is a crash. Six dispatch matches are `pydantic_encoder`
in Alembic revisions (`src/dispatch/database/revisions/tenant/versions/2021-10-27_d1dc160533c7.py:35`, ...): real
(the import fails on Pydantic 3), but editing old migrations is a choice a team makes once, not per steer.

Precision is not the problem; volume and steer value are. In dispatch, a codebase whose convention is the v1 names,
the rule fires 161 times across 74 files; as a steer it asks for a rename in every file an agent touches.

### Overlap

mypy 2.4.1 on the rule's own cases (`/tmp/review-pydantic/ovl/pydantic-practice-v1-api-in-v2`,
`--enable-error-code deprecated`) reports 28 of the 38 caught lines:

- group 1 with mypy's **default** settings: `Field(regex=/const=/unique_items=)` and `Field(min_items=)` as
  `call-overload` (schemas.py:10-13, tests:7), the post `@root_validator` as `call-overload` (:33), `pydantic.BaseSettings`
  as an invalid base (settings.py:14);
- group 2 with `deprecated` enabled: every method on a typed receiver (services.py:11-38, 44), `@root_validator`
  (:37, :41), `parse_obj_as`.

mypy does not report: the nested `class Config` and its renamed/removed keys (schemas.py:22-26), `@validator` at its
use (:29), `pydantic_encoder` (:46), `validate_arguments` (:50, a different error), `from pydantic import BaseSettings`
(settings.py:2), `.json(sort_keys=True)` in tests (:13). mypy is version-correct by construction (it reads the
installed pydantic), so the manifest gate is not an advantage over a team that runs mypy; it is over a team that does
not. pyright's `reportDeprecated` reads the same `@deprecated` markers (pyright was not run here). Of the corpus,
zenml's `pyproject.toml` already configures mypy with `plugins = ["pydantic.mypy"]`.

What the rule adds over mypy: the `Config` key group (silent), `.json(...)` with dumps arguments, `from_orm` on a
model without `from_attributes`, and coverage for untyped teams.

### Text

All group-1 and group-2 messages are under 200 characters and name the replacement. `FromOrmCrash` is wrong for
SQLModel models (CX1). `DeprecatedMethod` for `.copy()` correctly warns that `model_copy` takes no `include`/`exclude`
(checked: `model_copy(include=...)` raises TypeError).

### Verdict: rework

1. Split into two rules: the removed-in-v2 group (`RemovedFieldArg`, `RemovedBaseSettings`, `RootValidatorCrash`,
   `RemovedConfigKey`, `RenamedConfigKey`, `JsonDumpsArgs`, `FromOrmCrash`) in steer / on; the deprecation group in
   log / off by default (on for a team planning the v3 move). The pack is private, so the ids can still change.
2. Treat `sqlmodel.SQLModel` as setting `from_attributes=True` in `v1i_from_attributes`.
3. Narrow the shim test to conditions naming `pydantic`, `PYDANTIC_V*`, `VERSION` of pydantic, or `hasattr(..., "model_*")`,
   not any text containing `v1`/`v2`/`version`.

## `pydantic-practice-validator-returns-value`

### Value

Reproduced: a field validator that only raises on the bad path sets the field to `None` (`F(x="ok").x is None`); an
after model validator without `return self` makes `M.model_validate(...)` return `None` and a nested field `None`,
while `M(...)` works with only a UserWarning. Sources open and match:
[pydantic#9334](https://github.com/pydantic/pydantic/issues/9334) ("Confusing handling of badly defined
`model_validator`"), [pydantic#7252](https://github.com/pydantic/pydantic/issues/7252). A before model validator
that mutates `data` and returns nothing is loud (`ValidationError: Input should be a valid dictionary`), so that
kind is lower value than the message's "validates None" suggests, but it is still a bug.

### Correctness: counterexamples (`/tmp/review-pydantic/vrv`, run `review-pydantic-validator-returns-value-cx`)

| # | Shape | Result |
|---|---|---|
| CX1 | `def must_be_upper(v)` that raises or falls off its end, attached as `Annotated[str, AfterValidator(must_be_upper)]` | silent: miss (documented gap); the field becomes None the same way |
| CX2 | `if v == "": return` then `return v`, on `email: Optional[str] = None` (None meant) | reported (`app/models.py:26`); the message says "Return `v` ... instead", which would keep `""`: see Text |
| CX3 | a field validator whose last statement is `fail("bad email")`, a module helper that always raises | **reported** (`:31`): false positive (documented gap); Ruff RET503 has the same blind spot |
| CX4 | after model validator ending `return None` | silent: miss (documented: an explicit None is taken as deliberate, but for an after *model* validator it is never right: `model_validate` returns None) |

CX3 is the one false-positive shape. Following a run function one level (its body's last statement is a `raise`,
or it is annotated `-> NoReturn` / `Never`) would close it; `default-factory` already follows helpers one level.
CX4 should be reported: for `mode="after"` model validators, `return None` has no legitimate use.

### Corpus

0 matches in every repository run. Consistent with the README: teams that use Pydantic heavily already write `return v`.

### Overlap

Ruff 0.17.6 `--select RET503` on the rule's cases reports 2 of its 16 lines (`app/schemas.py:39`, `:73`); the 10
validators that never return anything are outside RET503's definition (it needs another path that returns a value),
and pylint R1710 has the same precondition (pylint was not run here). The README's claim holds.

### Text

- Description (137 chars): fine.
- `ValidatorFallsOff` / `ValidatorBareReturn`: under 200 as written; clear.
- `ValidatorBareReturn` says "Return {fix} instead, or raise ValueError". A bare `return` on an `Optional` field
  (`if v == "": return`) usually means None on purpose; following the message keeps `""`. Add "or write `return
  None` if None is intended" (the rule already leaves explicit `return None` alone).

### Verdict: keep, steer / on

Small fixes, none blocking: the bare-return message (above); follow a helper that always raises one level (CX3);
report `return None` in an after model validator (CX4). Optional: read functions passed to `AfterValidator(fn)` in
`Annotated[...]` (polar uses the `Annotated` form in 48 files).

## Pack-level findings

### Missing rules (with evidence)

- **Annotated validators.** Both validator rules read decorators only. polar has 48 non-test files with
  `BeforeValidator(`/`AfterValidator(`/`WrapValidator(`/`PlainValidator(` against 63 decorator validators; freqtrade 1
  and dify 7 files. The one real raw-input bug found in polar during this review is of this form. This is a shared
  library gap (`pd_validator` should yield `(fn, field, mode)` for `Annotated` metadata), not a new rule.
- **`pydantic.v1` imports in a v2 project.** The v1 rule deliberately leaves `pydantic.v1` alone, but `pydantic.v1`
  is the next upgrade blocker: on Python 3.14 pydantic warns "Core Pydantic V1 functionality isn't compatible with
  Python 3.14 or greater" ([pydantic#12618](https://github.com/pydantic/pydantic/issues/12618),
  [#12363](https://github.com/pydantic/pydantic/issues/12363)), and the pack notes record that pydantic's `main`
  (V3) branch has removed it. The corpus has 1 non-test file importing it (Netflix/dispatch). A log-mode candidate,
  low priority.
- **Serialization sinks for `dump-mode-json`**: `JSONResponse`, aiohttp, Celery JSON task arguments, SQLAlchemy JSON
  columns (the last two documented as gaps by the writer). Extending the existing rule is enough.

### Rules in the wrong pack

None. `before-validator-raw-input`'s exposure test reads FastAPI/Flask routes, which fits here (the hazard is in the
model). `secret-settings-str` could be read as a python-security rule, but its test and its fix are Pydantic types
(`SecretStr`, `pydantic_settings`), so a Pydantic user expects it here. The candidates the notes moved out
(response-model leaks, PATCH without `exclude_unset`, mass assignment) are correctly left to the fastapi and
sqlalchemy packs.

### Library quality

`rules/lib/pydantic.rerail` is careful and well documented: imports resolve through `sym_of`, bases close linearly
across files, the manifest pin reader handles requirements, pyproject, Poetry, uv and lock files, and `pydantic.v1`
models are kept apart. Weak points found: `pd_instance` cannot type a function's return value or an attribute
(`self.x`), which bounds every rule that needs a receiver (dump-mode-json, v1-api-in-v2); `pd_config_of` does not know
library bases' config (SQLModel's `from_attributes`); validators attached through `Annotated` are not modelled.

### Can it be pitched on its own?

Not yet. After the reworks it is a small, precise pack (one crash/silent-ignore steer, three steers for runtime bugs,
two log rules) whose matches are nearly all real; but on the 14 repositories run, its steer rules produced 0 matches
outside the v1 deprecation group, so a team's first impression would be either silence or 161 rename requests. It
sells as part of the FastAPI + Pydantic + SQLAlchemy bundle, with the v1 crash group and `dump-mode-json` (once it
reads `JSONResponse`) as the headline.

## Corpus repositories and commits

From `~/rerail-bench/python-arch/corpus/MANIFEST.tsv`. One run per repository with all six rules
(`review-pydantic-all-<repo>`), every one finished within the 15-minute limit. These are the brief's Pydantic
repositories, superset, and the five FastAPI/Pydantic repositories already appended to the manifest (fastapi-react,
khoj, we-mp-rss, letta, zenml). The `_wild-pydantic-dump/` repositories the dump-mode-json writer found by code search
were not re-run. No repository was cloned for this review.

| Repository | Commit | Files | Result |
|---|---|--:|---|
| fastapi/full-stack-fastapi-template | `77e550a73bb49dd2967622c5ed11bf7a49603c93` | 43 | 3 (secret 3) |
| fastapi-users/fastapi-users | `e9c57ff7ff2fd385bb0f6f795b524e9736b64290` | 88 | 0 |
| Buuntu/fastapi-react | `af17f2d422d0ebc5f173e83f2c6931045ab055b0` | 30 | 0 |
| open-webui/open-webui | `0666e59d26a4d056e0bacee41742486badd71cf8` | 330 | 1 (v1 1) |
| freqtrade/freqtrade | `7fcb509a434cfaa5e0943a7ccc005967cfd0b9d9` | 484 | 1 (raw-input 1) |
| khoj-ai/khoj | `ae229ca894c0b80ad84664afcfdde523b5e87057` | 252 | 8 (v1 8) |
| rachelos/we-mp-rss | `8fe5bf93874383b5956026f3747885e3674b1857` | 221 | 0 |
| Netflix/dispatch | `dd2837e82a0bf5565b1b4b4b91ea30b7262d4061` | 717 | 161 (v1 161) |
| mealie-recipes/mealie | `8dbb92808d5a58efe203640198bbadbc00d64dd0` | 723 | 5 (raw-input 1, secret 4) |
| letta-ai/letta | `1131535716e8a31c9a437f8695e25ac98f203a24` | 878 | 30 (raw-input 4, secret 25, v1 1) |
| polarsource/polar | `872272fbfc6e92500d7b0532e00fbaf99df52835` | 2522 | 36 (raw-input 1, secret 35) |
| langgenius/dify | `724ac15121ee3d7d7145d6b9c0ce777938530476` | 5191 | 94 (secret 93, v1 1) |
| apache/superset | `fdf61721d0561b03aa8147b185140359abf4af54` | 3190 | 0 |
| zenml-io/zenml | `61569b1e08e71372c37effd94f7e1f17da4140c4` | 2518 | 1 (v1 1) |

## Applied

Applied by the pack fixer (with three sub-workers for dump-mode-json, secret-settings-str and the v1 split). Each
root cause was fixed in the pack library when more than one rule leans on it. `pack.json`, the README and
`dimensions.json` list the rules in the reading order above. All 7 rule tests and the pack contract pass.

- `pydantic-practice-v1-api-in-v2` (split): keeps the removed-in-v2 group only (steer, on;
  availability/high, hazard `deps.removed-api`). `from_attributes` is resolved as Pydantic merges config, with
  `sqlmodel.SQLModel` setting it (CX1 silent, CX2 still `FromOrmCrash`), and the shim test resolves pydantic's version
  symbols, `hasattr`/`getattr` of `model_*` and variables holding them instead of matching text (CX3 reported); both
  moved to the library (`pd_from_attributes`, `pd_version_guarded`). Corpus: 0 matches in 8 repositories.
- `pydantic-practice-deprecated-v1-api` (new, the deprecation half): log, off; maintainability/low. Corpus: 173 matches,
  53 read, 53 real; no location changed from the single rule.
- `pydantic-practice-dump-mode-json`: sinks resolved as symbols and objects: Starlette/FastAPI `JSONResponse` and its run
  subclasses (not `ORJSONResponse`/`UJSONResponse` or a subclass with its own `render`), aiohttp sessions and
  `json_response`, clients stored on `self`, conditional and boolean expressions (CX1, CX2, CX3, CX6 caught); message
  167 characters. Steer, on. Corpus: 0 in 6 corpus repositories, 3 of 3 real in the code-search repositories; polar
  not re-run (local runs were stopped).
- `pydantic-practice-validator-returns-value`: bare-return message names `return None`; helpers that never return are
  followed through the call graph (last statement raises or calls one, or `NoReturn`/`Never`; CX3 silent); `return
  None` in an after model validator is reported (CX4); `Annotated` validator functions are read through the library's
  new `pd_fn_validator` (CX1 caught). Steer, on. Corpus: 0 in 6 repositories.
- `pydantic-practice-default-factory`: message 185 characters; `Annotated[..., Field(default=...)]` and module-level
  aliases read in the library's `pd_field_default` (CX2 caught); helpers followed to any depth;
  `time.monotonic`/`perf_counter` added (CX3 caught). Steer, on. Corpus: 0 in 6 repositories.
- `pydantic-practice-before-validator-raw-input`: one message per mode (before, wrap, plain), each under 200
  characters; `BeforeValidator`/`WrapValidator`/`PlainValidator` functions and lambdas read through the library,
  aliases across modules included (CX3 and polar's `normalize_claim_code` caught); FastAPI `Depends()`/`Query()`/...
  parameters resolved and left out (CX1); guards read by meaning, so a positive `isinstance` early exit no longer
  guards (CX2 caught) and `in` on raw input is a use (CX7 caught); routes resolved through their router object
  (FastAPI, flask-openapi3, project subclasses, router-class factories: library `pd_route`) and parse-call exposure
  needs evidence of foreign input (decoders, request bodies, HTTP responses, raw route parameters), so a model fed
  another model's dump (CX5) or ORM rows is left alone. Log, on. Corpus: 10 matches, 10 real in freqtrade, mealie,
  letta, open-webui and dispatch (mealie's four new `extras` converters are the CX2 shape); polar was not re-run with
  the final router resolution (local runs were stopped).
- `pydantic-practice-secret-settings-str`: the name deny-list is replaced by a last-word test (CX1-CX6 silent), with
  `Field` aliases and string-type aliases resolved; message 186 characters. Log, on. Corpus: 159 matches, 158 real
  (99.4%); CX7-CX9 documented as gaps.

Not applied (missing rules, out of scope): a `pydantic.v1` import rule for v2 projects; SQLAlchemy JSON columns and
Celery JSON task arguments as `dump-mode-json` sinks.
