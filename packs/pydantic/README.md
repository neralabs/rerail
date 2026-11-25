# Rerail rule pack: Pydantic

Rerail's own Pydantic rules, from the Pydantic v2 docs and migration guide, for correct models, validators and serialization.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/pydantic`.

## Rules

| Rule | Mode | On | Catches |
|---|---|---|---|
| `pydantic-practice-v1-api-in-v2` | Steer | On | Pydantic practice: Pydantic 1 API that Pydantic 2 removed or ignores, in a project pinned to Pydantic 2 |
| `pydantic-practice-dump-mode-json` | Steer | On | Pydantic practice: `model_dump()` handed to json.dumps, a JSONResponse or `json=` while the model has a datetime, UUID, Decimal or other field json cannot encode |
| `pydantic-practice-validator-returns-value` | Steer | On | Pydantic practice: a validator that can end without returning the value (or `self`) |
| `pydantic-practice-default-factory` | Steer | On | Pydantic practice: field default computed once at import instead of a default_factory |
| `pydantic-practice-before-validator-raw-input` | Log | On | Pydantic practice: a before, wrap or plain validator that uses its raw input as one type without checking it |
| `pydantic-practice-secret-settings-str` | Log | On | Pydantic practice: secret on a settings class typed str instead of SecretStr |
| `pydantic-practice-deprecated-v1-api` | Log | Off | Pydantic practice: Pydantic 1 API that Pydantic 2 deprecates and Pydantic 3 removes, in a project pinned to Pydantic 2 |

The rules are listed in the order to read them, by what they prevent: an import-time crash or a silently ignored
setting, a code path that raises every time, a field or model silently None, one value shared by every instance, a
500 instead of a 422 on malformed input, a secret printed when the settings are logged, and last the Pydantic 3
upgrade list.

## Rule details

### `pydantic-practice-v1-api-in-v2`

A project pinned to Pydantic 2 uses a Pydantic 1 API that Pydantic 2 removed: the model fails to import, a call
raises, or a setting is silently dropped. The rule reads the project's manifests (`pyproject.toml`,
`requirements*.txt`, `setup.cfg`, `uv.lock`, `poetry.lock`, `Pipfile`, at the root and up to 3 levels below it) to tell
v1 code from v2 code: in a directory whose nearest manifests pin Pydantic 1, or with no pin and no v2-only import, it
says nothing. What Pydantic 2 deprecated but still runs (`.dict()`, `@validator`, a nested `class Config:`) is
`pydantic-practice-deprecated-v1-api`'s, off by default.

**Catches**, each checked against pydantic 2.11.4 and 2.12.5: `Field(regex=...)`, `Field(const=...)`,
`Field(unique_items=...)` (pydantic's own `Field`, under any import alias: `PydanticUserError` when the class is
created); `from pydantic import BaseSettings` (aliased or not) and `pydantic.BaseSettings` (`PydanticImportError`);
`@root_validator` or `@root_validator()` without `pre=True` or `skip_on_failure=True`, under any alias
(`PydanticUserError` when the class is created); in a nested `class Config:`, the keys v2 dropped (`fields`, whose
aliases are then lost, `allow_mutation`, `getter_dict`, `smart_union`, `copy_on_model_validation`, `json_loads`,
`json_dumps`, `underscore_attrs_are_private`, `error_msg_templates`, `post_init_call`) and the keys v2 renamed
(`orm_mode`, `allow_population_by_field_name`, `schema_extra`, `validate_all`, `anystr_strip_whitespace`,
`anystr_lower`, `anystr_upper`, `max_anystr_length`, `min_anystr_length`, `keep_untouched`): v2 warns once and
ignores them, so `orm_mode = True` leaves `from_attributes` off; `.json(...)` with an argument the deprecated method
does not take (`indent`, `sort_keys`, `ensure_ascii`, `default`, ...: `TypeError`); and `M.from_orm(obj)` when
`from_attributes` is not on for M (`PydanticUserError`). Tests are read too: a crash there is still a crash.

`from_attributes` is resolved the way Pydantic merges config. M's own setting wins (`model_config =
ConfigDict(from_attributes=...)`, a dict literal, a class keyword `class M(BaseModel, from_attributes=True)`, or the v2
key in a nested `Config`); with none, it is on when it is on for one of M's bases, read the same way through the run's
modules and re-exports. Of the bases outside the run, `sqlmodel.SQLModel` sets it, so every SQLModel model has it
(`HeroRead(HeroBase)`, `HeroBase(SQLModel)`: `HeroRead.from_orm(row)` works, with a deprecation warning). A subclass
that sets `from_attributes=False` turns it off for itself and its own subclasses. `orm_mode=True` never counts, in a
nested `Config` or in `ConfigDict(...)`: v2 ignores the old name.

The receiver of `.json(...)` or `.from_orm(...)` must resolve to a v2 model: the class, an instance built as `M(...)`
or by a constructor, the name the same scope binds to it, `self` in a method of M, or a parameter annotated with M.

**Leaves alone**: `.json(...)` and `.from_orm(...)` on anything not known to be a model (`resp.json()`, a SQLAlchemy
row); a method the project defines itself on the model or a run base; models deriving from `pydantic.v1.BaseModel`,
and `pydantic.v1`'s `Field`, `validator` and `root_validator`; `sqlmodel.Field(regex=...)`, which SQLModel still maps;
`@root_validator(pre=True)` and `@root_validator(skip_on_failure=True)`, `.json()` with only the arguments it still
takes, and `from_orm()` on a model with `from_attributes` (all deprecated, not broken: the other rule's); and code
written to run under both majors. A guard is a version shim when it reads Pydantic, resolved rather than read as text:
pydantic's version symbol through imports (`pydantic.VERSION`, `from pydantic.version import VERSION as PV`),
`importlib.metadata.version("pydantic")`, `hasattr(x, "model_dump")` or `getattr(x, "model_validate", None)`, or a
variable assigned one of those, followed through modules, imports and further variables (`PYDANTIC_V2 =
VERSION.startswith("2.")` in `app/compat.py`, imported elsewhere as `IS_V2`). A flag defined outside the run
(FastAPI's `fastapi._compat.PYDANTIC_V2`) cannot be read, so there its name decides (`PYDANTIC_V2`,
`IS_PYDANTIC_V1`, `PYDANTIC_VERSION`). The guard is an `if` or `elif` condition or a conditional expression; a `try`
whose handler catches `ImportError`, `ModuleNotFoundError` or `AttributeError` is a shim too.
`if path.startswith(settings.API_V1_STR):` and `if version >= 2:` are not shims.

**Gaps**: an instance the rule cannot type (a function's return value, an attribute such as `self.form.json(indent=2)`,
a list item) is not reported, and neither is a model whose base is a library's model outside the run (fastapi-users'
`BaseUser`). Renamed or removed keys inside `model_config = ConfigDict(...)` (which v2 ignores too) are not reported,
though `orm_mode` there does leave `from_attributes` off for `from_orm()`. `.dict()` and `.copy()` with arguments v2
refuses are not reported. A model with two bases that disagree on `from_attributes` is read as on. A Poetry bare-string
pin (`pydantic = "^1.10"`) is not read as a pin.

**Sources**: Pydantic migration guide, https://docs.pydantic.dev/latest/migration/ ("Changes to pydantic.Field",
"Changes to config", "@validator and @root_validator are deprecated", "BaseSettings has moved to pydantic-settings");
Pydantic usage errors, https://docs.pydantic.dev/latest/errors/usage_errors/ (`removed-kwargs`,
`root-validator-pre-skip`); pydantic 2.x `pydantic/_internal/_config.py` (`V2_REMOVED_KEYS`, `V2_RENAMED_KEYS`: a
warning, then the keys are ignored) and `pydantic/main.py` (`from_orm` raises unless `from_attributes`); SQLModel's
`sqlmodel/main.py`, whose `SQLModel.model_config` sets `from_attributes=True`.

**Corpus**: 0 matches over Netflix/dispatch, khoj, open-webui, letta, zenml, full-stack-fastapi-template, mealie and
dify, so precision has no denominator and there is no false positive; a text search of full-stack-template, mealie
and open-webui for the removed shapes finds none either (a v2 test run would fail on most of them); the 173 matches
the single rule had here were all deprecated uses, now `pydantic-practice-deprecated-v1-api`'s. The rule guards new
code: an edit that brings v1 code into a v2 project, and the silently ignored `Config` keys, which no test run
reports.

### `pydantic-practice-dump-mode-json`

`model_dump()` defaults to `mode="python"`: a `datetime`, `UUID`, `Decimal`, `set`, plain `Enum`, `HttpUrl` or
`SecretStr` field stays that Python object in the dict. The standard library's json encoder accepts only dict, list,
tuple, str, int, float, bool and None, so `json.dumps(m.model_dump())` raises `TypeError: Object of type datetime is
not JSON serializable` every time it runs with such a model: the code path is down (a 500, a failed job, an error
handler that itself raises). Starlette's `JSONResponse` renders its content with `json.dumps` too, so a FastAPI route
returning `JSONResponse(content=m.model_dump())` answers 500 on every call. `m.model_dump(mode="json")` gives a
JSON-safe dict, the same dict for JSON-native fields.

**Catches** `o.model_dump(...)` (or v2's deprecated `o.dict(...)`) in Python mode (no `mode=`, or `mode="python"`)
reaching an encoder that uses the standard library's json with no default. The sinks are resolved symbols and
objects, never a method name alone:

- `json.dumps(...)` / `json.dump(..., fp)`, resolved through imports, with no `default=`, `cls=` or `**kwargs`;
- Starlette's `JSONResponse` (`starlette.responses.JSONResponse`, re-exported as `fastapi.responses.JSONResponse`),
  through any import, alias, module attribute or re-export of the run, and every run subclass of it (at any depth)
  that overrides neither `render` nor `__init__`: its first argument or `content=`;
- aiohttp's `web.json_response(data)` with no `dumps=`;
- the `json=` keyword of `requests.<verb>()`, `httpx.<verb>()`, `aiohttp.request()`, and of the HTTP methods of a
  client object: a `requests.Session()`, `httpx.Client()`, `httpx.AsyncClient()` or `aiohttp.ClientSession()` (or a
  run subclass of one that overrides neither `__init__` nor a request method), built in place, assigned to a name,
  bound by `with ... as`, or stored in `self.x` when the class's methods (and those of its run bases and subclasses)
  assign `self.x` only such a client or None;
- as the argument itself, a value or `**` splat of a dict literal (`json.dumps({"data": m.model_dump()})`), an item
  of a list or tuple literal, the body of a list comprehension, either branch of a conditional expression
  (`m.model_dump() if m else {}`), an operand of `or` (the right one of `and`), or a name the function last assigned
  from one of these (`payload = m.model_dump(); json.dumps(payload)`).

The receiver must hold a known model: a constructor call, a name bound to one, `self` in a method of the model, a
parameter annotated with it (`Event`, `Event | None`; lib/pydantic's `pd_instance`), or the variable of a
comprehension or `for` loop over a parameter annotated `list[Model]`. The model's fields are read where it is
defined, across files: its own, those it inherits from the run's classes, its `@computed_field` return types, and the
fields of the models its fields hold (alone, `Optional`, in `list[...]` or `dict[str, ...]`, to any depth). A field
is reported when its annotation names `datetime`, `date`, `time`, `timedelta`, `UUID` (and pydantic's
`UUID1`..`UUID8`), `Decimal`, `bytes`, `bytearray`, `set`/`frozenset`, a `pathlib` path, an `ipaddress` type,
`SecretStr`/`SecretBytes`, pydantic's URL and DSN types (Url objects in Pydantic 2), pydantic's constrained
(`condecimal`, `conset`, ...), past/future and aware/naive date types, or a run `Enum` that is not also a `str`,
`int` or `float`. The message names the model, the first such field and its type, and the sink.

**Leaves alone**: `model_dump(mode="json")`, `model_dump_json()`, `json.dumps(..., default=...)` or `cls=...`,
orjson, FastAPI's `ORJSONResponse` and `UJSONResponse` (they encode datetime and UUID), a run `JSONResponse` subclass
with its own `render` (and its subclasses) or its own `__init__`, FastAPI's `jsonable_encoder(...)`, Flask's
`jsonify(...)`, Django's `JsonResponse` (its encoder takes datetime, UUID and Decimal), a project's own json module
(superset's `from superset.utils import json`; also every `json.dumps` when the run has a module named `json`), a
project class that only shares a name (`JSONResponse`, `ClientSession`), an aiohttp session built with
`json_serialize=` or `**kwargs`, `json_response(..., dumps=...)`, a client the rule cannot resolve (a TestClient
fixture, a parameter, a function's return value, `self.client = client or httpx.Client()`, a `self.client` a subclass
replaces), a client subclass that overrides `request` or `send`. A model whose fields are all JSON-native (str, int,
float, bool, None, `Literal`, `str`/`int` enums, lists and dicts of those, nested models of those): it encodes today
and `mode="json"` would change nothing, so only proven non-JSON fields are reported; this is the rule's main precision
decision, and on the corpus every resolved `JSONResponse`/`json.dumps`/`json=` dump it found unreported was of such a
model (fastapi-users' `JSONResponse(bearer_response.model_dump())`, str fields only). Also left alone: a dump with
`include=` or `exclude=` (not followed), a model that defines its own `model_dump` or `dict` (letta's `LettaBase`;
srneha24/Movie-Collection-Backend's `APIResponse`, which isoformats its UUID and datetimes there), has a
`@model_serializer` or sets `json_encoders` (Pydantic 2 applies `json_encoders` in JSON mode only, so such a model can
still fail; it is left alone as a sign the author handles encoding); a field with `Field(exclude=True)`, a
`@field_serializer` for it (unless `when_used="json"` or `"json-unless-none"`, which do not run in Python mode), or a
`PlainSerializer`/`WrapSerializer` in its `Annotated`; an enum field of a model with `use_enum_values=True`; a field
the model's own code replaces after validation (a method of the model or a run base assigns `self.x = ...`, as an
after model validator's `self.host = str(self.host)`, or an after/plain/wrap field validator for it returns a value;
a before validator's result is validated into the declared type again, so it is still reported); pydantic.v1 models;
URL fields in a project pinned to Pydantic 1 (v1's URLs are str); a name the function changes after the dump
(`payload["at"] = ...`, `payload.update(...)`); and test code, where such a dump fails the first time the test runs.

**Gaps**: a receiver whose model is not known (a function's return value, as in most real
`JSONResponse(content=result.model_dump())` code; an attribute such as `self._entities` or
`request.interaction`; a dict `.get(...)`; a tuple unpacking; an untyped or `Any` parameter; an
`isinstance`-narrowed loop variable); a field typed `Any` or `dict[str, Any]` that holds a datetime at runtime (the
awslabs/mcp incident below is this shape and is not caught); type aliases (`UTCDatetime = Annotated[datetime, ...]`)
and types from other libraries (pydantic_extra_types); a model from outside the run; a dump passed through a helper
before it reaches the encoder; a client held in a module global or a class attribute, or passed in as a parameter
(it may have been built with its own encoder); a `JSONResponse` subclass with its own `__init__` (it may convert the
content); `requests` uses simplejson when it is installed, which encodes `Decimal` (not datetime or UUID); an
inherited field overridden two levels down; a SQLAlchemy JSON column or a Celery JSON-serialized task argument fed a
dump (both encode with json too; not sinks here).

**Overlap**: none. mypy and pyright accept `json.dumps(m.model_dump())` and `JSONResponse(content=m.model_dump())`
(`model_dump` returns `dict[str, Any]`, `content` is `Any`); Ruff, Bandit, pylint, flake8-pydantic and the pydantic
mypy plugin have no such check.

**Sources**: Pydantic docs, [Serialization](https://docs.pydantic.dev/latest/concepts/serialization/) (Python mode
output "may contain non-JSON-compatible Python objects"; `mode="json"` / `model_dump_json()` are JSON-safe); Python
docs, [json conversion table](https://docs.python.org/3/library/json.html#py-to-json-table) (anything else raises
`TypeError`); Starlette, [Responses: JSONResponse](https://www.starlette.io/responses/#jsonresponse) and FastAPI,
[Return a Response Directly](https://fastapi.tiangolo.com/advanced/response-directly/) (a returned `JSONResponse` is
sent as is, without `jsonable_encoder`); aiohttp, [Client reference](https://docs.aiohttp.org/en/stable/client_reference.html)
(`json_serialize=json.dumps` by default); incidents: [awslabs/mcp#1990](https://github.com/awslabs/mcp/issues/1990)
(`json.dumps(data.model_dump())` broke every Glue/Athena/EMR operation returning a timestamp; fixed with
`mode='json'`), [crewAIInc/crewAI#7358](https://github.com/crewAIInc/crewAI/issues/7358) (flow persistence crashes on
datetime, UUID, set, Decimal and Path state fields), [google/adk-python#6747](https://github.com/google/adk-python/issues/6747)
(Python-mode dump of an output schema is not JSON-serializable).

**Corpus**: 0 matches in the Pydantic repositories run with the reworked rule: fastapi-users/fastapi-users@e9c57ff7ff2f,
open-webui/open-webui@0666e59d26a4, mealie-recipes/mealie@8dbb92808d5a, Netflix/dispatch@dd2837e82a0b,
khoj-ai/khoj@ae229ca894c0 and letta-ai/letta@1131535716e8 (the earlier version also had 0 in polarsource/polar,
fastapi/full-stack-fastapi-template, freqtrade, dify, superset and zenml, whose dumps are of JSON-native
models). fastapi-users' `JSONResponse(bearer_response.model_dump())` (`authentication/transport/bearer.py:26`) is the
new sink on a str-only model and stays silent. Every `json.dumps(m.model_dump())` in the corpus was read and is of a
JSON-native model (dify's `chunk.model_dump()` in `api/core/agent/cot_agent_runner.py` dumps
`AgentScratchpadUnit.Action`, str and dict fields; `text_store.py` and `vector_store.py` dump `Identifier`/str scopes;
`workflow_tools_manage_service.py` dumps str fields and a str enum; `rag_pipeline_task_proxy.py` str and
`dict[str, Any]`), superset's `mcp_core.py`/`instance_metadata.py` go through superset's own json, and letta's
`errors.py:404` dumps `Message`, whose base `LettaBase` overrides `model_dump`. To read real matches, the rule also
ran over 55 small public repositories found by GitHub code search (appended to `MANIFEST.tsv` under
`_wild-pydantic-dump/`): 30 for `json.dumps(self.model_dump())`, `json.dumps(event.model_dump())`,
`json=payload.model_dump()` and similar, and 25 for `JSONResponse(content=...model_dump())` and aiohttp
`json=...model_dump()`. The first version reported 5 in aruba/pyafc@720e5d37ec11: 3 were false (`SyslogEntry`,
`SnmpTrapServer` and `DhcpRelay` hold an `IPv4Address` that an after model validator turns into a str,
`self.host = str(self.host)`), which led to the "field rewritten after validation" exclusion and its cases. The
current rule reports 3, all real (3/3): `pyafc/route_policies/route_maps.py:131` and `:257` (`RouteMap` /
`RouteMapEntry` with `match_ipv4_next_hop_address: IPv4Address`, posted as
`data=json.dumps(data.model_dump(exclude_none=True))`: creating a route map entry with a next hop fails), and
EcorRouge/rococo-email-parser@1d4232d5c551 `rococo/models/email.py:103` (`Email.__repr__` is
`json.dumps(self.model_dump())` while `date: Optional[datetime]` is set by the parser: `repr()` of any parsed email
raises). The 25 JSONResponse/aiohttp repositories hold 58 non-test sites of those sinks and give 0 matches: where the
sink and model resolve, the model is JSON-native (cyberark/FuzzyAI's 7 `self._session.post(url,
json=request.model_dump())` on an `aiohttp.ClientSession` built in `__init__`; taipei-day-trip's
`JSONResponse(content=response_data.model_dump())`) or converts its own fields (Movie-Collection-Backend's
`APIResponse.model_dump` override); the rest dump a function's return value, which the rule does not type.
`pd_typed_instance` (python/types) as an extra receiver found nothing more on fastapi-template, freqtrade, mealie
and dify and cost 3 s vs 3 s, 28 s vs 12 s, 20 s vs 12 s and 599 s vs 131 s: it is not used.

### `pydantic-practice-validator-returns-value`

Pydantic keeps what a validator returns. A field validator's return value becomes the field's value, so a path that
ends without a `return` (or with a bare `return`) silently sets the field to None, even a non-Optional one: an after
validator's result is not validated again. A before model validator's return value replaces the input data, a wrap
model validator's is the validated model, and an after model validator must `return self`: without it
`Model.model_validate(data)` returns None (`Model(...)` still works, with only a UserWarning since 2.9), and a field of
that model nested in a parent becomes None.

**Catches**, in a def decorated with Pydantic's `field_validator` (any mode), `model_validator` (before, after, wrap),
or v1's `validator` / `root_validator` (pre or not; they still run under Pydantic 2), and in a def a field's
annotation attaches with `AfterValidator(fn)`, `BeforeValidator(fn)`, `WrapValidator(fn)` or `PlainValidator(fn)`
(directly in `Annotated[...]` or through a module-level alias such as `Upper = Annotated[str, AfterValidator(fn)]`,
in the same module or imported): the def's name when its body can reach its end, and each bare `return`; and, in an
after model validator, a `return None` too (there None is never right: `model_validate()` returns it). The decorator
is resolved through imports, re-exports included, and the def may sit in any class: a mixin in one module whose model
is declared in another is checked where the validator is written. "Can reach its end" is
`django-practice-clean-field-returns-value`'s analysis: a return or raise closes a path; an `if`, `try`, `with`, loop
or `match` is open when one of its ways through is (an `if` without `else`, a handler that logs and falls through, a
`for` loop with no `return` after it, a `match` with no catch-all); `assert` is not a way out; a call statement whose
callee never returns closes a path: `sys.exit()`, `os._exit()`, `typing.assert_never()`, a def of the run annotated
`-> NoReturn` / `-> Never`, or a def of the run whose last statement raises or calls another such def, followed
through the call graph to any depth and resolved through imports (an aliased import, `errors.bail()`, a static
method `Errors.invalid()`); a helper that only logs, whatever its name (`fail_soft()`), returns. Shapes in the
cases: the happy path of an `if` that only raises, an `if/else` whose other branch only logs, a validating loop,
`try/except` that logs, an `elif` chain without `else`, a lone `assert`, `@model_validator(mode="after")` that
raises or ends or returns None,
`@model_validator(mode="before")` that mutates `data` in place, a wrap model validator's bare `return` in its
handler, v1 `@validator` and `@root_validator`, an `AfterValidator` function reached through an alias, a
`BeforeValidator` function's bare `return`.

The messages name the value to return (`v`, `handler(v)`'s result, `self`); the bare-`return` message adds "or write
`return None` if None is intended", since a bare `return` on an `Optional` field (`if v == "": return`) often means
None on purpose.

**Leaves alone**: validators whose every path returns or raises (`raise` in every `except`; `while True` left only by
`return`; a `for/else` that raises; a `match` whose every case returns; a last statement that calls a helper which
always raises); `return None` written out in any validator but an after model validator (a deliberate
None); `field_serializer`, `model_serializer` and `computed_field` (not validators); decorators of other libraries
named like Pydantic's (attrs' `@x.validator`, marshmallow's `validates` / `validates_schema`, DRF's
`validate_<field>` methods, a project's own `validator` function); generators; stubs whose body is only a docstring,
`pass` or `...` (abstract or not) and `typing.overload` stubs (`raise NotImplementedError` raises, so it is closed);
`return self` inside a `with` block; and test code (`tests/`, `test_*.py`, `conftest.py`, ...): test modules build
throwaway models and sometimes leave a validator without a return on purpose to check what Pydantic does with it.

**Gaps**: a helper counts as never returning only when its last statement raises or calls such a helper, or it is
annotated `NoReturn`; one that raises in both branches of a final `if`, or a method called on an instance
(`self.fail()`), is taken as completing, so a validator ending in it is reported. An after model validator that
returns something other than `self` or None is not reported. A validator attached by calling the decorator outside a
class body (`field_validator(...)(fn)`) is not looked at.

**Overlap**: no linter reads a validator's decorator. Ruff's RET503 (implicit-return, not in the `python` pack's
default set) fires only when another path of the function returns a value; pylint's R1710
(inconsistent-return-statements) only when some returns have a value and others not. On this rule's 16 cases, Ruff
0.16.9 `--select RET503` reports 2 (the `if/else` whose else only logs, the `elif` chain without else); pylint 4
`--enable=R1710` reports 6 (those two, the three bare returns, and the `try/except` that logs), plus one near miss
(a nested helper's bare return, not the validator's). Neither reports the 10 validators that never return: the
common bug, a field validator that only raises, or an after model validator without `return self`. mypy and pyright
report a missing return only when the validator is annotated with a non-None return type. Bandit and flake8-pydantic
have no such check.

**Sources**: Pydantic docs, [Validators](https://docs.pydantic.dev/latest/concepts/validators/) ("a callable taking
the value to be validated as an argument and returning the validated value"; after model validators: "the validated
instance should be returned"); [pydantic#9334](https://github.com/pydantic/pydantic/issues/9334) (an after validator
without `return self` makes `model_validate` return None while `Model(**d)` works; the maintainers added the
"A custom validator is returning a value other than `self`" warning in
[#10255](https://github.com/pydantic/pydantic/pull/10255)); [pydantic#7252](https://github.com/pydantic/pydantic/issues/7252)
(a nested model's field becomes None).

**Corpus**: 0 matches in 4 repositories, so precision has no denominator; no false positive. The review's run of 14
repositories also gave 0. After the rework (Annotated validator functions, `return None` in after model validators,
helpers that never return followed through the call graph) the rule was re-run over freqtrade, mealie, letta,
open-webui, Netflix/dispatch and polar (whose 48 files with `BeforeValidator` / `AfterValidator` are now read): still 0.

| Repository | Commit | Validators outside tests | Matches |
|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a73bb49dd2967622c5ed11bf7a49603c93` | 3 | 0 |
| mealie-recipes/mealie | `8dbb92808d5a58efe203640198bbadbc00d64dd0` | 83 | 0 |
| polarsource/polar | `872272fbfc6e92500d7b0532e00fbaf99df52835` | 63 | 0 |
| langgenius/dify | `724ac15121ee3d7d7145d6b9c0ce777938530476` | 652 | 0 |

Validator counts are decorator lines found by text search; in mealie a probe of the rule confirmed that all 83
resolve to Pydantic and are examined (73 field, 8 after model, 2 before model validators). A separate Python `ast`
scan of the same repositories, Netflix/dispatch and open-webui, tests included, found no validator that can end
without returning either: teams that use Pydantic heavily already write `return v` / `return self`. The rule is a
guard for new code, where the two upstream issues show the bug is easy to write.

### `pydantic-practice-default-factory`

A Pydantic field whose default is computed by a call when the class body runs: `created_at: datetime =
datetime.now()`, `id: UUID = uuid4()`. The class body runs once, at import, so every instance built without that field
gets the same value: timestamps as old as the process, and for ids the same UUID on every row, so inserts collide on
the primary key or overwrite each other's records. Pydantic copies a default for each instance but never evaluates it
again; `Field(default_factory=...)` is the per-instance form.

**Catches** a default that is, or contains outside a lambda, a call whose result differs per call, resolved through
imports (`from datetime import datetime as dt; dt.now()` and `import uuid; uuid.uuid4()` both count):

- `datetime.now()`, `datetime.utcnow()`, `datetime.now(timezone.utc)`, `datetime.today()`, `date.today()`,
  `time.time()`, `time.time_ns()`, `time.monotonic()`, `time.perf_counter()`; `uuid1/4/6/7()`, also inside
  `str(uuid4())`, `uuid4().hex` or an f-string; `os.urandom()`;
  the `secrets` and `random` functions (`secrets.token_urlsafe()`, `random.random()`); Django's `timezone.now()`,
  pendulum's and arrow's `now()`, `shortuuid.uuid()`, `nanoid.generate()`, `ulid.new()`, uuid6/uuid_utils, and
  `bson.ObjectId()` with no argument; an expression built on one (`datetime.now() + timedelta(days=7)`);
- a def of the run, in any file (a module function, or a static or class method called through its class), whose own
  `return` value contains such a call or a call of another such def, followed through the call graph to any depth:
  `seen_at: datetime = utcnow()` with `def utcnow(): return datetime.now(timezone.utc)` in `app/timeutil.py`, and
  `at: str = stamp()` with `def stamp(): return utcnow().isoformat()`; names resolve through imports and aliases
  (`from uuid import uuid4 as make_uuid`), and a project class named like a library one (`clock.datetime.now()`)
  is the project's, not the library's;
- in the forms `x: T = <call>`, `x: T = Field(<call>)`, `x: T = Field(default=<call>)` (pydantic's or sqlmodel's
  `Field`), and `x: Annotated[T, Field(<call>)]` / `Annotated[T, Field(default=<call>)]`, also through a
  module-level alias in any module (`RequestId = Annotated[str, Field(default=request_id())]`, reported where the
  alias is built), on a model deriving from `BaseModel`, `RootModel` or `sqlmodel.SQLModel` (a table model or not),
  directly or through the run's classes in any module;
- a private attribute's default: `_x: T = <call>` and `_x = PrivateAttr(<call>)` / `PrivateAttr(default=<call>)`,
  which Pydantic also copies to each instance unchanged (message: `PrivateAttr(default_factory=...)`);
- a field of a `pydantic.dataclasses.dataclass`, also with `dataclasses.field(default=<call>)`.

The message names the call and proposes `Field(default_factory=...)` (`uuid4`, `lambda: datetime.now(timezone.utc)`),
which is safe wherever the rule matches: the field keeps its type and stays optional. It also says to keep the call
when every instance must share the value (a process start time, say).

**Leaves alone**: `default_factory=` of any form and an uncalled function as a value; calls meant to give a fixed
value (`datetime(2020, 1, 1)`, `timedelta(days=1)`, `UUID("...")`, `uuid5(...)`, `Decimal("0")`, `frozenset(...)`,
`Path(...)`, `os.getenv(...)`, `Enum.X`, `Field(...)`), and a project helper that returns a fixed value or returns a
function rather than calling one; mutable literals (`tags: list[str] = []`), which Pydantic copies per instance, so
they are safe on a model; `ClassVar` annotations; an unannotated `_x = datetime.now()` (Pydantic makes it a private
attribute too, but it reads as a class constant); settings models (`pydantic_settings.BaseSettings` and subclasses):
a settings object is the process's configuration, read once by design, and a factory would make two `Settings()`
disagree (with `SECRET_KEY: str = secrets.token_urlsafe(32)`, a token signed under one instance would fail to verify
under another), so following the message there could break working code; plain classes, stdlib and attrs dataclasses
(Ruff RUF009), SQLAlchemy `mapped_column(default=...)` (the sqlalchemy pack), `pydantic.v1` models, function
parameter defaults (Ruff B008); and test code (`tests/`, `test_*.py`, `conftest.py`, ...).

**Gaps**: the per-call library functions are a name list, since nothing in the code says a function returns a fresh
value (a library function not listed is missed); a model whose Pydantic base is a library class outside the run that
is not a known base; a method called on an instance (`self.clock.now()`), whose class is not resolved; a value
returned through a variable (`t = datetime.now(); return t`); `pydantic.v1` models, which have the same bug.
`created: datetime = Field(datetime.now)` (the function itself as the default, so the field holds a function) is a
different bug and is not reported.

**Overlap**: Ruff's RUF009 (function-call-in-dataclass-default-argument) reports the same call in a stdlib or attrs
dataclass only. Run with ruff 0.17.6 (`--select RUF009,B008`) on this rule's cases, it reported the stdlib and attrs
dataclasses and the function parameter default (B008) and none of the 27 Pydantic model and Pydantic dataclass lines
this rule reports. RUF012 exempts Pydantic models; flake8-pydantic (PYD001-PYD010) has no such check. What this rule
adds: Pydantic model and Pydantic dataclass awareness through the run's base classes in any module, private
attributes, `Field(default=...)`, and helpers followed across files.

**Sources**: Pydantic docs, Fields, [Default values](https://docs.pydantic.dev/latest/concepts/fields/#default-values)
(`default_factory` for values computed per instance; its example is `Field(default_factory=lambda: uuid4().hex)`);
Ruff [RUF009](https://docs.astral.sh/ruff/rules/function-call-in-dataclass-default-argument/) (the same bug in
dataclasses: the call runs once, when the class is defined); SQLModel docs, [UUID](https://sqlmodel.tiangolo.com/advanced/uuid/)
(`Field(default_factory=uuid.uuid4, primary_key=True)`); Stack Overflow
[73128975](https://stackoverflow.com/questions/73128975) "Pydantic created at and updated at fields" (the accepted
answer itself writes `created_at: datetime = datetime.now()`; a later answer points out that every instance then
shares one timestamp and moves it to `default_factory`); this repository's django pack, `django-practice-default-called-once`
(the same idea for Django model fields, and the list of per-call functions this rule reuses).

**Corpus** (MANIFEST.tsv commits): 0 matches in 8 repositories, so precision has no denominator; no false positive.
The review's run of 14 repositories also gave 0. After the rework (Annotated and alias defaults, helpers followed to
any depth, `time.monotonic` / `perf_counter`) the rule was re-run over freqtrade, mealie, letta, open-webui,
Netflix/dispatch and polar (polar has `Annotated[...]` fields in 48 files): still 0 matches.

| Repository | Commit | Files importing pydantic/sqlmodel | Matches |
|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a73bb49dd2967622c5ed11bf7a49603c93` | 18 | 0 |
| fastapi-users/fastapi-users | `e9c57ff7ff2fd385bb0f6f795b524e9736b64290` | 10 | 0 |
| freqtrade/freqtrade | `7fcb509a434cfaa5e0943a7ccc005967cfd0b9d9` | 6 | 0 |
| khoj-ai/khoj | `ae229ca894c0b80ad84664afcfdde523b5e87057` | 14 | 0 |
| open-webui/open-webui | `0666e59d26a4d056e0bacee41742486badd71cf8` | 84 | 0 |
| Netflix/dispatch | `dd2837e82a0bf5565b1b4b4b91ea30b7262d4061` | 117 | 0 |
| letta-ai/letta | `1131535716e8a31c9a437f8695e25ac98f203a24` | 129 | 0 |
| mealie-recipes/mealie | `8dbb92808d5a58efe203640198bbadbc00d64dd0` | 178 | 0 |

To show the rule reaches these models, a probe copy of it reporting every default that contains any call (not only
a per-call one) found 2 in full-stack-fastapi-template (`items: list[Item] = Relationship(...)` on its SQLModel table
models, whose `id` and `created_at` already use `default_factory=uuid.uuid4` / `default_factory=get_datetime_utc`) and
12 in mealie (its `UpdatedAtField(...)` wrapper of `Field`, nested model instances such as
`GroupEventNotifierOptions()`, a `dataclasses.field(default_factory=list)`); none is a per-call value. A Python `ast`
scan of the class bodies of 14 repositories (the 9 Pydantic repositories of the corpus, zenml, letta, khoj,
we-mp-rss and fastapi-react) found such calls only outside Pydantic: SQLAlchemy `func.now()` server defaults in dify
and letta, and `uuid.uuid4()` on two `unittest.TestCase` classes in dify's tests. Teams on these repositories already
write `default_factory`; the rule is a guard for new code, where the Stack Overflow question and the SQLModel docs show
the bug is easy to write.

### `pydantic-practice-before-validator-raw-input`

A before, wrap or plain validator gets the raw input, "which in theory could be any arbitrary object" (Pydantic docs,
Validators). Pydantic 2 turns only `ValueError`, `AssertionError` and `PydanticCustomError` raised in a validator into
a `ValidationError`; a `TypeError` (no longer converted since 2.0, migration guide) or an `AttributeError` escapes. So a
validator that calls `v.strip()` or `data.get("x")` on whatever arrived turns a client sending `123` for a string field,
or a list for an object, into an unhandled exception: a 500 in FastAPI instead of a 422, a crashed task.

**Catches**, once per validator at its first unchecked use, in `@field_validator(..., mode="before" | "wrap" |
"plain")`, v1's `@validator(..., pre=True)`, `@model_validator(mode="before" | "wrap")`, v1's
`@root_validator(pre=True)`, and a function a field's annotation attaches with `BeforeValidator(fn)`,
`WrapValidator(fn)` or `PlainValidator(fn)` (a def of the run, by name or imported, or a lambda; directly in
`Annotated[...]` or through a module-level alias such as `Slug = Annotated[str, BeforeValidator(lower_slug)]` in
another module): the value parameter used as one type: the receiver of a method call (`v.strip()`, `data.get("x")`),
subscripted (`data["x"]`), unpacked (`{**data}`), passed to `len()`, iterated (`for k in data`, a comprehension), or
the right side of `in` (`"kind" not in data`: a `TypeError` on an int). The parameter's annotation does not protect
(`def f(cls, v: str)`: Pydantic does not check it). Also, in any model, the v1-style slip
`@model_validator(mode="after")` written as a classmethod `(cls, values)` that calls `values.get(...)` (or `items`,
`keys`, `setdefault`, ...), subscripts or unpacks it: Pydantic passes the model instance there, so it raises on every
validation. In a project pinned to Pydantic 1, which converts `TypeError`, only method calls (`AttributeError`) count.

A use counts as checked by what the check means for the path it is on, not by where the check sits: in the body of an
`if` / `elif` on `isinstance(v, ...)`, `type(v)`, `hasattr(v, ...)` or `callable(v)`; after an `if` on a negated check
that returns or raises (`if not isinstance(v, str): return v`, `if type(v) is not str: raise ...`); after `assert
isinstance(...)`; on the true side of a conditional expression or `and` on a check, or the false side / `or` of a
negated one. An early exit on a positive check leaves every other type: `if isinstance(v, dict): return v` followed by
`{x.key_name: x.value for x in v}` (mealie's shape) is reported, since `5` reaches the loop.

It reports only validators of models that foreign input reaches, on evidence read across files:

- a web route names the model in a parameter annotation (`body: M`, `list[M]`, `M | None`), the route resolved through
  its router object: a module-level object built by FastAPI's `FastAPI()` / `APIRouter()`, flask-openapi3's
  `OpenAPI()` / `APIBlueprint()`, a project subclass of one (mealie's `UserAPIRouter`), or a class a project factory
  returns (polar's `APIRouter = get_api_router_class(APIRoute)`), here or imported. A `.get` / `.post` decorator of
  any other object (a cache) is not a route. A parameter FastAPI builds from query, path, header or cookie values
  (`Depends()`, `Query()`, `Header()`, `Cookie()`, `Path()`, `Security()`, as its default or in its `Annotated`
  metadata) is left out: each value arrives already validated as its declared type (review counterexample: an
  `ItemQuery` with `Annotated[ItemQuery, Depends()]`);
- it is parsed from raw text (`model_validate_json`, v1's `parse_raw` / `parse_file`, a TypeAdapter's
  `validate_json`), or from a value the code shows is foreign by `model_validate`, `parse_obj`, a TypeAdapter's
  `validate_python` or `M(**value)`: decoded JSON, YAML, TOML or msgpack (`json.loads`, `orjson.loads`,
  `yaml.safe_load`, ...), the body of the request a handler receives (`await request.json()`, `request.form()`, with
  the request a parameter annotated with Starlette's, FastAPI's, Flask's, aiohttp's or Django's request class, or
  Flask's `request.get_json()`), an HTTP response's `.json()` (`requests` / `httpx` calls), a route's raw parameter, a
  key or `.get()` of one, or a name bound to one. A TypeAdapter is followed from where it is built to where it is used,
  across modules;
- or it is (in) the type of a field of such a model; the validator may sit in the model, a base class or a mixin in
  another module, or be attached through `Annotated`.

Each mode gets its own message, since the safe fix differs: a before validator returns other input unchanged so the
field's own validation reports it; a wrap validator passes it to its handler unchanged; a plain validator replaces
Pydantic's validation, so returning other input unchanged would store it unchecked, and its message says to raise
`ValueError` instead.

**Leaves alone**: checked uses (above), a use inside `match v:`, in a `try` whose handler catches what it raises
(`AttributeError` for a method call, `TypeError` otherwise), `Exception` or everything, after `if v != expected:
raise ...` (or `not in`), or after the name is rebound (`v = str(v)`; `data = data or {}` still counts as raw).
After-mode field validators and `AfterValidator` functions, and v1 validators without `pre` (the value already has the
field's type); `str(v)`, `f"{v}"`, `v is None`, `v == ""`, `bool(v)`, `v in ("a", "b")`; dunder methods; calls on
other names (`cls.helper(v)`, `info.data.get(...)`); an after model validator written as an instance method; a
validator def that carries another decorator besides `classmethod` / `staticmethod` (a project wrapper between
Pydantic and the def, such as zenml's `@before_validator_handler`, which converts the input first); decorators of other
libraries (attrs, a project's own `validator`); validators from `pydantic.v1`; test code, and models only test code
parses. And models parsed only from the project's own objects: an ORM row (polar's response schemas validated from
query results), another model's dump (`M.model_validate(other.model_dump())`), the project's settings, keyword
arguments, or a value whose origin the function does not show.

**Gaps**: a check of the wrong type in the right place still counts (`if isinstance(v, int): return v.strip()`); a
check on another name (`s = v; if isinstance(s, str)`), a helper that checks, `contextlib.suppress`, and a rebinding by
a loop, `with` or walrus are not seen, so those validators are reported; attribute reads without a call are not uses.
A validator attached through an alias of an alias, a `type X = ...` statement, or to a route parameter rather than a
model field is not read, and a wrapper decorator that does not convert the input hides a real case. Exposure needs
evidence, so it misses a model parsed from a parameter whose caller passes foreign data (a webhook handler's `payload:
dict`, a Celery task's argument, a project helper that takes the class, dify's `validate_request(Model)`), an HTTP
client object's response (`await client.get(...)`), and a router held in a local variable or an attribute.

**Overlap**: none. Ruff, pylint, Bandit and flake8-pydantic have no validator-aware check; mypy and the Pydantic mypy
plugin accept `v.strip()` on a parameter typed `Any`, or typed `str` (the annotation is what is wrong).

**Sources**: Pydantic docs, [Validators](https://docs.pydantic.dev/latest/concepts/validators/) (before validators
"have to deal with the raw input, which in theory could be any arbitrary object"; the model-before example guards with
`if isinstance(data, dict):`; "Raising validation errors": only `ValueError`, `AssertionError` and
`PydanticCustomError`); [Migration guide](https://docs.pydantic.dev/latest/migration/) ("`TypeError` is no longer
converted to `ValidationError` in validators"); [fastapi/fastapi#4974](https://github.com/fastapi/fastapi/issues/4974)
(a root validator's error surfaces as a 500 instead of a 422). Checked with Pydantic 2.14.4: `A.model_validate({"x":
123})` with a before validator `v.strip()` raises `AttributeError`; `v["k"]` raises `TypeError`; through FastAPI the
same model as a body returns 500 for `{"name": 123}`; a plain validator returning a non-str `v` unchanged stores `123`
in a `str` field, and so does a wrap validator that skips its handler; an after classmethod `(cls, values)` gets the
instance and `values.get` raises `AttributeError`.

**Corpus**: every match read. Commits from MANIFEST.tsv; the review's baseline was 7 matches, 7 real (freqtrade 1,
mealie 1, letta 4, polar 1).

| Repository | Commit | Matches | Real | Notes |
|---|---|--:|--:|---|
| freqtrade/freqtrade | `7fcb509a434c` | 1 | 1 | `freqtrade/rpc/api_server/api_schemas.py:548`, `values.get` in a model before validator of `DownloadDataPayload`, a request body |
| mealie-recipes/mealie | `8dbb92808d5a` | 5 | 5 | `mealie/schema/recipe/recipe_ingredient.py:85`, `:253`, `mealie/schema/household/group_shopping_list.py:75`, `:187`, `mealie/schema/recipe/recipe.py:226`: `extras` converters that return `v` when it is a dict and otherwise iterate it (`{x.key_name: x.value for x in v}`), so `"extras": 5` in a recipe or shopping-list body raises `TypeError`; the review had only `:253` (its old guard test took the positive `isinstance` exit as a check) |
| letta-ai/letta | `1131535716e8` | 4 | 4 | `letta/schemas/llm_config.py:142, 168, 215, 254`, `values.get` in four model before validators of `LLMConfig`, a field of the agent create body |
| open-webui/open-webui | `0666e59d26a4` | 0 | | |
| Netflix/dispatch | `dd2837e82a0b` | 0 | | |
| polarsource/polar | `872272fbfc6e` | not re-run (review: 1) | (review: 1) | 2,522 files: too large for the corpus hosts; see below |

10 matches, all read, 10 real in the five repositories measured with the final rule (runs `pydantic-final-<repo>`,
all seven rules together; review: 7 of 7). polar (2,522 `.py` files) was not
re-run with the final rule; the review's numbers stand (1 match, 1 real: `server/polar/organization/schemas.py:424`).
Intermediate versions gave, on polar: with Annotated validators added and the old exposure test, 3 matches,
`server/polar/agent/schemas.py:104` (`normalize_claim_code`, `value.strip()` on `AgentClaimConfirm.code`, the review's
extra real case), `server/polar/organization/schemas.py:424` (real) and `server/polar/product/schemas.py:1231`
(`_filter_benefit_public_list` on `BenefitPublicList`, a response field of schemas validated from ORM rows: false,
now left alone by the evidence-based exposure); with the evidence-based exposure but routes resolved only through a
router class's qualified name, 0, because polar builds its router class with a factory (`APIRouter =
get_api_router_class(APIRoute)`). The final rule resolves such factories (cased in `app/routing.py`); its result on
polar is not measured.

**Mode**: log, on. Every read match is real, but the exposure test is evidence-based and new shapes of foreign input
(webhook payloads, task arguments) are gaps rather than matches, so the denominator stays small; earlier rounds showed
each new codebase bringing a new exposure shape (zenml: 0 of 12 before the wrapper check).

### `pydantic-practice-secret-settings-str`

A secret on a Pydantic settings class typed as a plain `str`: `SECRET_KEY: str`, `SMTP_PASSWORD: str | None = None`,
`OPENAI_API_KEY: Annotated[str, Field(min_length=20)]`. The settings object is the one a process prints, logs or
dumps at start-up, and `repr()`, `str()`, `model_dump()`, `model_dump_json()` of it, and a `ValidationError` raised on
it, print a `str` field in clear. `SecretStr` prints `'**********'` in all of them until `.get_secret_value()`.

**Catches** a field of a `pydantic_settings.BaseSettings` class (through the run's classes in any module, a base
named as `config.Settings` included), inherited ones included (a settings base or a `BaseModel` mixin in another file,
reported once at its declaration), and a field of a plain model a settings class holds as a field (`smtp:
SmtpConfig`, nested settings), when:

- its type is a plain string, alone or with `None`: the builtin `str`, Pydantic's `StrictStr` or `constr(...)`,
  through `Optional[...]`, `Union[...]`, `|` and `Annotated[..., ...]`, and through a module-level alias of the run
  (`PlainStr = str`, `NonEmpty = Annotated[str, Field(min_length=1)]`, `OptionalText = Optional[PlainStr]`), in the
  same module or imported. Names resolve through imports, not by their text;
- its name, or a string its `Field` gives as `alias=` or `validation_alias=` (`AliasChoices(...)` included), ends in
  a secret. The name is cut into word parts at `_` and at camel-case humps, case-insensitive, a trailing number
  dropped, and the last part is `password`, `passwd`, `secret`, `token`, `credential(s)`, `apikey` or `pepper`
  (`SMTP_PASSWORD`, `STRIPE_WEBHOOK_SECRET`, `slackBotToken`, `SENDGRID_APIKEY`, `PASSWORD_PEPPER`), or it is `key`
  after one of those words, `api` or `private`, or after `access` that follows one of them (`SECRET_KEY`,
  `OPENAI_API_KEY`, `GITHUB_APP_PRIVATE_KEY`, `privateKey`, `AWS_SECRET_ACCESS_KEY`). `openai: str =
  Field(alias="OPENAI_API_KEY")` is caught by its alias.

The name is the only signal the engine can decide: the type is `str` whatever the value, and the value comes from the
environment. So the rule reads the name's last word, which says what the value is. A name that mentions a secret and
ends in something else is about the secret, not the secret.

The message names the field and counts the reads of `.NAME` in the run's files: changing the type to `SecretStr` breaks
every read that uses the value as a string (`f"postgresql://u:{settings.DB_PASSWORD}@..."` silently gets
`'**********'`, `hmac.new(settings.SECRET_KEY.encode(), ...)` raises), so the message says to change each one to
`.get_secret_value()`. That is why the rule ships in `log` mode: following it means touching every reader.

**Leaves alone**: names whose last part is not a secret, whatever they mention (`JWT_TOKEN_AUDIENCE`, `TOKEN_ISSUER`,
`PASSWORD_HASH_SCHEME`, `AUTH_TOKEN_COOKIE`, `TOKEN_ENDPOINT`, `GITHUB_TOKEN_SCOPES`, `TOKEN_URL`, `SECRET_KEY_FILE`,
`API_KEY_NAME`, `CURRENT_HASH_SECRET_ID`, `PUBLIC_KEY`), and names with no secret part (`TOKENIZER`,
`PASSWORDLESS_LOGIN`); an access key id (`S3_ACCESS_KEY`, `minioAccessKey`: the public half of a pair whose
`S3_SECRET_KEY` is caught; dify has eight); `SecretStr`, `SecretBytes`, `Secret[...]` and project types deriving from
them; types that are not a plain string (`ACCESS_TOKEN_EXPIRE_MINUTES: int`, `SECRET_KEYS: list[str]`, `Literal[...]`,
URL and DSN types); a field its author masked (`Field(repr=False)`, `Annotated[str, PlainSerializer(...)]` or
`WrapSerializer`, in place or through an alias such as mealie's `MaskedNoneString`, or a `@field_serializer` naming
it); DSN fields (`DATABASE_URL: str`) even when the URL embeds a password, since the name says URL and many carry none
(a gap, not a judgement);
request and response models (`password: str` on a login body is input; the fastapi pack owns response leaks);
non-Pydantic config classes and dataclasses; `pydantic.v1` settings; `ClassVar` and `_private` attributes; test code;
and settings classes in migrations (`alembic/`, `migrations/`), which read the environment once for that migration.

**Gaps**: a last word that abbreviates a secret (`DB_PASS`: `_pass` also ends `FIRST_PASS` and `COMPILER_PASS`); a
key with no secret word before it (`FERNET_KEY`, `OPENAI_KEY`, `ENCRYPTION_KEY`, `SIGNING_KEY`: `_key` alone also ends
`CACHE_KEY`, `SORT_KEY`, `IDEMPOTENCY_KEY`); a secret word followed by a qualifier (dify's
`INNER_API_KEY_FOR_PLUGIN`); `SALT`; a Vault AppRole `SECRET_ID`; a vendor's public key named like a secret (PostHog's
`POSTHOG_PROJECT_API_KEY`, reported: nothing in the code says it is public); a quoted annotation (`X_PASSWORD:
"str"`); a PEP 695 `type` alias; a settings base outside the run that is not a known base; `pydantic.v1` settings
classes, which have the same bug. The read count matches `.NAME` on any object, so a lower-case name (`password`)
counts reads of other objects' attributes too.

**Cross-file reach to a log**: the narrower shape (the settings object passed to `print`/a logger, or its
`model_dump()` logged or returned) was looked for in the corpus and appears once among the repositories that have
settings classes: mealie's start-up `logger.info(settings.model_dump_json(exclude={"SECRET", "SESSION_SECRET", ...}))`
(`mealie/app.py`), which shows the hazard: the exclude list is kept by hand, and `REVERSE_PROXY_AUTH_SECRET: str |
None`, added later, is not in it, so it is logged in clear at every start. One site is too few to measure a separate
shape, so the rule reports the field, whatever reaches it.

**Overlap**: Bandit B105/B106/B107 and Ruff S105-S107 report a hard-coded password literal (`SECRET_KEY: str =
"changeme"`); this rule is about the type, whatever the default, so both may fire on a field with a literal default.
mypy, pylint, flake8-pydantic and the rest have no such check.

**Sources**: Pydantic docs, [SecretStr](https://docs.pydantic.dev/latest/api/types/#pydantic.types.SecretStr) ("will
be formatted as `'**********'` when converted to a string or dumped to JSON") and
[Secrets](https://docs.pydantic.dev/latest/examples/secrets/); pydantic-settings docs,
[Settings Management](https://docs.pydantic.dev/latest/concepts/pydantic_settings/), which type credentials as
`SecretStr`; OWASP [Logging Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html)
(never log passwords, access tokens, encryption keys); [CWE-532](https://cwe.mitre.org/data/definitions/532.html),
insertion of sensitive information into a log file.

**Corpus** (MANIFEST.tsv commits):

| Repository | Commit | Matches | Real | Notes |
|---|---|--:|--:|---|
| fastapi/full-stack-fastapi-template | `77e550a73bb4` | 3 | 3 | `backend/app/core/config.py`: `SECRET_KEY`, `SMTP_PASSWORD`, `FIRST_SUPERUSER_PASSWORD` |
| mealie-recipes/mealie | `8dbb92808d5a` | 4 | 4 | `mealie/core/settings/settings.py`: `SECRET`, `SESSION_SECRET` (excluded from the start-up log by hand), `REVERSE_PROXY_AUTH_SECRET` (not excluded: logged in clear by `mealie/app.py`); `db_providers.py`: `POSTGRES_PASSWORD`. Its `MaskedNoneString` fields stay silent: the alias's `PlainSerializer` is read through |
| letta-ai/letta | `1131535716e8` | 25 | 25 | `letta/settings.py`: 20 `*_api_key`, `aws_secret_access_key`, `modal_token_secret`, `pg_password`, `clickhouse_password`, `default_token` |
| polarsource/polar | `872272fbfc6e` | 35 | 34 | all `server/polar/config.py` (`STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `AWS_SECRET_ACCESS_KEY`, `GITHUB_CLIENT_SECRET`, `TINYBIRD_API_TOKEN`, ...); not real: `POSTHOG_PROJECT_API_KEY`, PostHog's public, write-only project key. `BACKOFFICE_GOOGLE_CLIENT_SECRET: SecretStr`, `CURRENT_HASH_SECRET_ID`, `AWS_HASH_SECRET_ARN` stay silent |
| langgenius/dify | `724ac15121ee` | 92 | 92 | `api/configs/**` (78: `SECRET_KEY`, `DB_PASSWORD`, `REDIS_PASSWORD`, `S3_SECRET_KEY`, `PASSWORD_PEPPER`, 30-odd vector-store passwords and API keys, ...) and `dify-agent/**` (14: `server_secret_key`, `api_token`, `*_auth_token`, `e2b_api_key`, ...). Its eight `*_ACCESS_KEY` fields (access key ids) stay silent; missed: `INNER_API_KEY_FOR_PLUGIN` |
| open-webui/open-webui | `0666e59d26a4` | 0 | - | no `BaseSettings` class |
| Netflix/dispatch | `dd2837e82a0b` | 0 | - | no `BaseSettings` class |

159 matches read, all of them; 158 real (99.4%). The name test reads the last word of the name: the review's six
counterexamples (`JWT_TOKEN_AUDIENCE`, `TOKEN_ISSUER`, `PASSWORD_HASH_SCHEME`, `AUTH_TOKEN_COOKIE`, `TOKEN_ENDPOINT`,
`GITHUB_TOKEN_SCOPES`), all reported by the first version, are left alone, and every corpus match of the first version
but one (`INNER_API_KEY_FOR_PLUGIN`) is kept. The rule ships in `log` despite its precision: the fix changes every
reader of the field, and a reader missed gets `'**********'` silently.

### `pydantic-practice-deprecated-v1-api`

A project pinned to Pydantic 2 still calls Pydantic 1's API where Pydantic 2 keeps it working with a deprecation
warning; Pydantic 3 removes it (pydantic's `main` branch already has). Nothing breaks today, so the rule logs and
ships off: it is the upgrade list for a team planning the move to Pydantic 3. The project's major is read from its
manifests as for `pydantic-practice-v1-api-in-v2`, which reports what Pydantic 2 already broke; a line gets one
message when both run.

**Catches**, on a v2 model class or instance: `.dict()`, `.json()`, `.parse_obj()`, `.parse_raw()`, `.parse_file()`,
`.from_orm()` (on a model for which `from_attributes` is on), `.copy()`, `.construct()`, `.schema()`, `.schema_json()`,
`.update_forward_refs()`, `.validate()`, `__fields__`, `__fields_set__`; `@validator` and
`@root_validator(pre=True)` / `@root_validator(skip_on_failure=True)`, under any import alias; a nested
`class Config:` (once, on its `class` line); `Field(min_items=, max_items=)`; and the uses (not the imports) of
`parse_obj_as`, `schema_of`, `schema_json_of`, `pydantic_encoder` and `validate_arguments`. SQLModel models count:
their `from_orm()`, `dict()` and `parse_obj()` are deprecated wrappers too, and SQLModel turns `from_attributes` on, so
`HeroRead.from_orm(row)` on `HeroRead(HeroBase)`, `HeroBase(SQLModel)` is reported here, not as a crash. A receiver is
the class, an instance built as `M(...)` or by a constructor, the name the same scope binds to it, `self` in a method
of M, or a parameter annotated with M (a FastAPI body).

**Leaves alone**: what Pydantic 2 removed (a post `@root_validator` without `skip_on_failure`, `Field(regex=)`,
`.json(indent=...)`, `from_orm()` without `from_attributes`), left to `pydantic-practice-v1-api-in-v2`; `.dict()`,
`.copy()` or `.json()` on anything not known to be a model (`resp.json()`, `settings.copy()`, a SQLAlchemy row); a
method the project defines itself on the model or a run base, and a field of that name (`validate: Callable`);
`pydantic.v1` models and validators; `sqlmodel.Field(min_items=)`; tests, whose deprecation warnings are the suite's own
business; a project pinned to Pydantic 1; and code written to run under both majors: a guard that reads Pydantic,
resolved rather than read as text (pydantic's version symbol through imports, `importlib.metadata.version("pydantic")`,
`hasattr(x, "model_dump")`, or a variable assigned one of those, followed through modules and imports; a flag from
outside the run, such as FastAPI's `PYDANTIC_V2`, by its name), or a `try` catching `ImportError`,
`ModuleNotFoundError` or `AttributeError`. `if path.startswith(settings.API_V1_STR): return body.dict()` is reported:
that condition tests an API path, not Pydantic.

**Gaps**: an instance the rule cannot type (a function's return value, an attribute such as `self.form.dict()`, a list
item) is not reported, nor a model whose base is a library's model outside the run. `.dict()` and `.copy()` with
arguments v2 refuses are reported as deprecated, not as crashes; `json_encoders` (deprecated, still honoured) is not
reported on its own line (its `class Config` is).

**Overlap**: mypy with `enable_error_code = deprecated` and pyright's `reportDeprecated` report pydantic's
`@deprecated` methods where the receiver's type is known, once configured; on this rule's former cases mypy 2.4.1
reported every method call on a typed receiver, `@root_validator` and `parse_obj_as`, and missed the nested
`class Config`, `@validator` at its use, `pydantic_encoder` and `validate_arguments`. bump-pydantic is a one-shot
codemod with no rule for the method renames. What this rule adds: the manifest-pinned version gate, the `pydantic.v1`
exclusion, the shim test, and coverage for a team that does not run mypy with that code.

**Sources**: Pydantic migration guide, https://docs.pydantic.dev/latest/migration/ ("Changes to pydantic.BaseModel",
"@validator and @root_validator are deprecated", "Changes to config"); pydantic 2.x `pydantic/main.py`, where each old
method is `@typing_extensions.deprecated(...)` and emits `PydanticDeprecatedSince20`; SQLModel's `sqlmodel/main.py`
(deprecated `from_orm`, `parse_obj`, `dict`).

**Corpus** (commits from MANIFEST.tsv; tests left out):

| Repository | Matches | Read | Real |
|---|--:|--:|--:|
| Netflix/dispatch | 161 (74 files) | 41 | 41 |
| khoj-ai/khoj | 8 | 8 | 8 |
| open-webui/open-webui | 1 | 1 | 1 |
| letta-ai/letta | 1 | 1 | 1 |
| langgenius/dify | 1 | 1 | 1 |
| zenml-io/zenml | 1 | 1 | 1 |
| fastapi/full-stack-fastapi-template | 0 | | |
| mealie-recipes/mealie | 0 | | |

173 matches, 53 read (every fourth of dispatch's by path, all the others), 53 real deprecated uses (100%). Examples:
`src/dispatch/case/service.py:282` `case_in.dict(exclude_unset=True, ...)` on a `CaseUpdate` body (117 such `.dict()`
calls); `src/dispatch/incident/views.py:200` `IncidentRead.from_orm(current_incident)` (12, on schemas whose
`DispatchBase` sets `from_attributes=True`); `src/dispatch/case/messaging.py:407` `SubjectMetadata(...).json()` (21);
`src/dispatch/plugins/dispatch_slack/middleware.py:132` `EngagementMetadata.parse_raw(...)` (4); six
`pydantic_encoder` uses, five of them in Alembic revisions; khoj's `Field(..., min_items=1)` (5); open-webui's
`@validator('content', pre=True)`; nested `class Config` in letta, dify and zenml's example code. Volume is why the
rule logs and ships off: dispatch, a v2 codebase that kept the v1 names, carries 161 of them in 74 files.
