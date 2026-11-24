# Rerail rule pack: Pydantic

Rerail's own Pydantic rules, from the Pydantic v2 docs and migration guide, for correct models, validators and serialization.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/pydantic`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `pydantic-practice-before-validator-raw-input` | Log | Pydantic practice: a before, wrap or plain validator that uses its raw input as one type without checking it |
| `pydantic-practice-default-factory` | Steer | Pydantic practice: field default computed once at import instead of a default_factory |
| `pydantic-practice-dump-mode-json` | Steer | Pydantic practice: `model_dump()` handed to json.dumps or `json=` while the model has a datetime, UUID, Decimal or other field json cannot encode |
| `pydantic-practice-secret-settings-str` | Log | Pydantic practice: secret on a settings class typed str instead of SecretStr |
| `pydantic-practice-v1-api-in-v2` | Steer | Pydantic practice: Pydantic 1 API used on the models of a project pinned to Pydantic 2 |
| `pydantic-practice-validator-returns-value` | Steer | Pydantic practice: a validator that can end without returning the value (or `self`) |

## Rule details

### `pydantic-practice-before-validator-raw-input`

A before, wrap or plain validator gets the raw input, "which in theory could be any arbitrary object" (Pydantic docs,
Validators). Pydantic 2 turns only `ValueError`, `AssertionError` and `PydanticCustomError` raised in a validator into
a `ValidationError`; a `TypeError` (no longer converted since 2.0, migration guide) or an `AttributeError` escapes. So a
validator that calls `v.strip()` or `data.get("x")` on whatever arrived turns a client sending `123` for a string field,
or a list for an object, into an unhandled exception: a 500 in FastAPI instead of a 422, a crashed task.

**Catches**, once per validator at its first unchecked use: in `@field_validator(..., mode="before" | "wrap" |
"plain")`, v1's `@validator(..., pre=True)`, `@model_validator(mode="before" | "wrap")` and v1's
`@root_validator(pre=True)`, the value parameter used as one type with no check of its type: the receiver of a method
call (`v.strip()`, `data.get("x")`, `data.setdefault(...)`, `values.items()`), subscripted (`data["x"]`, `data["x"] =
...`), unpacked (`{**data}`), passed to `len()`, or iterated (`for k in data`, a comprehension). The parameter's
annotation does not protect (`def f(cls, v: str)`: Pydantic does not check it). It reports only validators of models
that foreign input reaches, read across files: a web route's parameter annotation names the model (`body: M`,
`list[M]`, `M | None`; FastAPI, Starlette or Flask route decorators), code outside tests parses it with
`M.model_validate(...)`, `model_validate_json`, v1's `parse_obj` / `parse_raw` / `parse_file` / `validate`, or builds a
`TypeAdapter` over it, or it is (in) the type of a field of such a model; the validator may sit in the model, a base
class or a mixin in another module. Also, in any model, the v1-style slip `@model_validator(mode="after")` written as a
classmethod `(cls, values)` that calls `values.get(...)` (or `items`, `keys`, `setdefault`, `pop`, `update`, ...),
subscripts or unpacks it: Pydantic passes the model instance there, so it raises `AttributeError` or `TypeError` on
every validation. In a project pinned to Pydantic 1, which converts `TypeError`, only method calls (`AttributeError`)
are reported.

**Leaves alone**: a use after `isinstance(v, ...)`, `type(v)`, `hasattr(v, ...)` or `callable(v)` of the same name (an
early `if not isinstance(v, str): return v`, or an `if isinstance(...)` around the use), or sharing a conditional
expression with one (`v.strip() if isinstance(v, str) else v`); a use inside `match v:`; a use in a `try` whose handler
catches what it raises (`AttributeError` for a method call, `TypeError` otherwise), `Exception` or everything; a use
after `if v != expected: raise ...` (or `not in`); a use after the name is rebound (`v = str(v)`; `data = data or {}`
still counts as raw). After-mode field validators and v1 validators without `pre` (the value already has the field's
type); `str(v)`, `f"{v}"`, `v is None`, `v == ""`, `bool(v)`, `"k" in v`; dunder methods; calls on other names
(`cls.helper(v)`, `info.data.get(...)`); an after model validator written as an instance method; a validator def that
carries another decorator besides `classmethod` / `staticmethod` (a project wrapper between Pydantic and the def, such
as zenml's `@before_validator_handler`, which converts the input first); decorators of other libraries (attrs, a
project's own `validator`); validators from `pydantic.v1` (v1 converts `TypeError`, and its decorators are not
resolved); test code. And models the project fills only from its own values: built with keyword arguments
(`Config(host=settings.HOST)`), parsed with `from_attributes=True` (an ORM row or response source), from a dict
literal, or from a name its scope binds to a dict literal; and models only test code parses.

**Gaps**: any type check of the name counts, even one of the wrong type (`if isinstance(v, list): return v` then
`v.split()`); a check on another name, a helper that checks, `contextlib.suppress`, and a rebinding by a loop, `with`
or walrus are not seen, so those validators are reported. `"k" in data` (a `TypeError` on an int) and attribute reads
without a call are not uses. Validators attached outside a class body (`BeforeValidator(fn)`, `WrapValidator`) are
not read, and a wrapper decorator that does not convert the input hides a real case. Exposure misses models parsed by
a project helper that takes the class (`validate_request(Model)` calling `model.model_validate(raw)`, dify's pattern)
and constructors fed `**request.json`; it counts every other `model_validate` as foreign data (also of a dict the
project built elsewhere and passed in) and every route parameter (also a query model whose values are always strings).

**Overlap**: none. Ruff, pylint, Bandit and flake8-pydantic have no validator-aware check; mypy and the Pydantic mypy
plugin accept `v.strip()` on a parameter typed `Any`, or typed `str` (the annotation is what is wrong).

**Sources**: Pydantic docs, [Validators](https://docs.pydantic.dev/latest/concepts/validators/) (before validators
"have to deal with the raw input, which in theory could be any arbitrary object"; the model-before example guards with
`if isinstance(data, dict):`; "Raising validation errors": only `ValueError`, `AssertionError` and
`PydanticCustomError`); [Migration guide](https://docs.pydantic.dev/latest/migration/) ("`TypeError` is no longer
converted to `ValidationError` in validators"); [fastapi/fastapi#4974](https://github.com/fastapi/fastapi/issues/4974)
(a root validator's error surfaces as a 500 instead of a 422). Checked locally with Pydantic 2.14.4:
`A.model_validate({"x": 123})` with a before validator `v.strip()` raises `AttributeError`; a model before validator
`data.setdefault(...)` given `[1]`, `"s"` or `1` raises `AttributeError`; `v["k"]` raises `TypeError`; an after
classmethod `(cls, values)` gets the instance (with a deprecation warning since 2.12) and `values.get` raises
`AttributeError`.

**Corpus**: every match read. "Without exposure" is the same rule reporting every validator, whatever reaches its model.

| Repository | Commit | Raw validators outside tests | Matches without exposure (real) | Matches (real) |
|---|---|--:|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a73bb49dd2967622c5ed11bf7a49603c93` | 1 | 0 | 0 |
| fastapi-users/fastapi-users | `e9c57ff7ff2fd385bb0f6f795b524e9736b64290` | 0 | 0 | 0 |
| freqtrade/freqtrade | `7fcb509a434cfaa5e0943a7ccc005967cfd0b9d9` | 1 | 1 (1) | 1 (1) |
| open-webui/open-webui | `0666e59d26a4d056e0bacee41742486badd71cf8` | 19 | 0 | 0 |
| mealie-recipes/mealie | `8dbb92808d5a58efe203640198bbadbc00d64dd0` | 25 | 1 (1) | 1 (1) |
| Netflix/dispatch | `dd2837e82a0bf5565b1b4b4b91ea30b7262d4061` | 11 | 0 | 0 |
| polarsource/polar | `872272fbfc6e92500d7b0532e00fbaf99df52835` | 10 | 3 (1) | 1 (1) |
| saleor/saleor | `e1ccf4d580025b06c2ff01441a1e798dc4635f24` | 5 | 0 | 0 |
| langgenius/dify | `724ac15121ee3d7d7145d6b9c0ce777938530476` | 204 | 35 (0) | 0 |
| apache/superset | `fdf61721d0561b03aa8147b185140359abf4af54` | 82 | 0 | 0 |
| letta-ai/letta | `1131535716e8a31c9a437f8695e25ac98f203a24` | | 6 (4) | 4 (4) |
| cornserve-ai/cornserve | `6d04df9010d7fdef041ed8a996f20f5837b4b16c` | | 2 (2) | 2 (2) |
| zenml-io/zenml | `61569b1e08e71372c37effd94f7e1f17da4140c4` | | not run | 0 (12 before the wrapper check, 0 real) |

Raw validators are before/wrap/plain decorators counted by a CPython `ast` scan (Python 3.9, so files using newer
syntax are not counted). Precision: 9 of 9 read matches real (3 of 3 in the ten Pydantic repositories); without
exposure 9 of 48 (3 of 40). Eleven further repositories with Pydantic models (speaches, agentic, AgentCapture,
Phoenix-Agent, movie-narrator, cyberbro, pyeep, carefree-drawboard, themecp-backend-v2, MemoLite, defra-model-broker)
gave no match either way, and meadow none with the rule.

Real matches: `freqtrade/rpc/api_server/api_schemas.py:548` (`values.get` in a model before validator of
`DownloadDataPayload`, a FastAPI request body); `mealie/schema/recipe/recipe_ingredient.py:253` (iterates the raw
`households_with_ingredient_food` inside `try/except AttributeError`: an int raises `TypeError`, not caught; the food
model is nested in recipe update bodies); `server/polar/organization/schemas.py:424` (`data.get("url")` in
`OrganizationSocialLink`, a list field of the organization update body); letta `letta/schemas/llm_config.py:142, 168,
215, 254` (four model before validators of `LLMConfig` calling `values.get`, a field of the agent create body);
cornserve `python/cornserve/task/base.py:594, 871` (`data[...]` in model before validators of models a gateway route
takes; the `"k" not in data` before them already raises on an int).

What exposure removes: dify's 27 vector-store and storage config classes (`providers/vdb/*`, `configs/middleware/vdb/
iris_config.py`, `extensions/storage/clickzetta_volume/...`), built from `dify_config` with keyword arguments or a
dict literal; dify response models validated from ORM rows with `from_attributes=True` (`controllers/console/app/
workflow.py:369`, `controllers/console/explore/trial.py:444`, `controllers/console/evaluation.py:229`); dify plugin
daemon and marketplace entities (`core/plugin/entities/*.py`, `core/tools/entities/tool_entities.py:186`, data from
dify's own services); dify's `ConversationVariablesQuery` (`controllers/service_api/app/conversation.py:79`, a GET query
model whose values are always strings); polar's backoffice form (`backoffice/organizations_v2/forms.py:152`, always a
dict from the form parser) and downloadable subscriber properties (`benefit/strategies/downloadables/schemas.py:56`,
built from stored benefit properties); letta's chat completion request and Azure provider (built in code). One
earlier false positive, saleor `webhook/response_schemas/shipping.py:103` (`value.upper()` after `if value !=
and `vdb-elasticsearch/.../elasticsearch_vector.py:48` (`model_validate(config_dict)`, a dict literal), led to the
project's-own-input check. zenml, run after those fixes as a check on unseen code, gave 12 matches, none real: every
one a model before validator (`src/zenml/config/step_configurations.py`, `docker_settings.py`,
`artifacts/artifact_config.py`, `model/model.py`, `models/v2/core/project.py`, ...) decorated with zenml's
`@before_validator_handler`, which hands the def a dict built from any input; that led to the wrapper check.

**Mode**: log. The read matches are all real after the last fixes, but the denominator is small (9, of which 3 in the
ten Pydantic repositories), and each fix came from the sample it was measured on: before the own-input check 9 of 11
(82%), and on the first unseen repository (zenml) 0 of 12 until the wrapper check.

### `pydantic-practice-default-factory`

A Pydantic field whose default is computed by a call when the class body runs: `created_at: datetime =
datetime.now()`, `id: UUID = uuid4()`. The class body runs once, at import, so every instance built without that field
gets the same value: timestamps as old as the process, and for ids the same UUID on every row, so inserts collide on
the primary key or overwrite each other's records. Pydantic copies a default for each instance but never evaluates it
again; `Field(default_factory=...)` is the per-instance form.

**Catches** a default that is, or contains outside a lambda, a call whose result differs per call, resolved through
imports (`from datetime import datetime as dt; dt.now()` and `import uuid; uuid.uuid4()` both count):

- `datetime.now()`, `datetime.utcnow()`, `datetime.now(timezone.utc)`, `datetime.today()`, `date.today()`,
  `time.time()`, `time.time_ns()`; `uuid1/4/6/7()`, also inside `str(uuid4())` or `uuid4().hex`; `os.urandom()`;
  the `secrets` and `random` functions (`secrets.token_urlsafe()`, `random.random()`); Django's `timezone.now()`,
  pendulum's and arrow's `now()`, `shortuuid.uuid()`, `nanoid.generate()`, `ulid.new()`, uuid6/uuid_utils, and
  `bson.ObjectId()` with no argument; an expression built on one (`datetime.now() + timedelta(days=7)`);
- a function of the run, in any file, whose own `return` value contains such a call, followed one level:
  `seen_at: datetime = utcnow()` with `def utcnow(): return datetime.now(timezone.utc)` in `app/timeutil.py`;
- in the forms `x: T = <call>`, `x: T = Field(<call>)`, `x: T = Field(default=<call>)` (pydantic's or sqlmodel's
  `Field`), on a model deriving from `BaseModel`, `RootModel` or `sqlmodel.SQLModel` (a table model or not), directly or
  through the run's classes in any module;
- a private attribute's default: `_x: T = <call>` and `_x = PrivateAttr(<call>)` / `PrivateAttr(default=<call>)`,
  which Pydantic also copies to each instance unchanged (message: `PrivateAttr(default_factory=...)`);
- a field of a `pydantic.dataclasses.dataclass`, also with `dataclasses.field(default=<call>)`.

The message names the call and proposes `Field(default_factory=...)` (`default_factory=uuid4`, `default_factory=lambda:
datetime.now(timezone.utc)`), which is safe wherever the rule matches: the field keeps its type and stays optional.

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

**Gaps**: a default inside `Annotated[T, Field(default=...)]`; a model whose Pydantic base is a library class outside
the run that is not a known base; a helper more than one call deep, or a method (`self.clock.now()`); a per-call
function of a library not listed; `pydantic.v1` models, which have the same bug.

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
[73128975](https://stackoverflow.com/questions/73128975) "Pydantic created at and updated at fields" (answers move a
`datetime.now()` default to `default_factory`); this repository's django pack, `django-practice-default-called-once`
(the same idea for Django model fields, and the list of per-call functions this rule reuses).

**Corpus** (MANIFEST.tsv commits): 0 matches in 8 repositories, so precision has no denominator; no false positive.

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

### `pydantic-practice-dump-mode-json`

`model_dump()` defaults to `mode="python"`: a `datetime`, `UUID`, `Decimal`, `set`, plain `Enum`, `HttpUrl` or
`SecretStr` field stays that Python object in the dict. The standard library's json encoder accepts only dict, list,
tuple, str, int, float, bool and None, so `json.dumps(m.model_dump())` raises `TypeError: Object of type datetime is
not JSON serializable` every time it runs with such a model: the code path is down (a 500, a failed job, an error
handler that itself raises). `m.model_dump(mode="json")` gives a JSON-safe dict, the same dict for JSON-native fields.

**Catches** `o.model_dump(...)` (or v2's deprecated `o.dict(...)`) in Python mode (no `mode=`, or `mode="python"`)
reaching the standard library's encoder:

- `json.dumps(...)` / `json.dump(..., fp)`, resolved through imports, with no `default=`, `cls=` or `**kwargs`;
- the `json=` keyword of `requests.<verb>()`, `httpx.<verb>()`, and the methods of a `requests.Session()`,
  `httpx.Client()` or `httpx.AsyncClient()` the function builds (by assignment or `with ... as`); both libraries
  encode `json=` with the json module;
- as the argument itself, a value or `**` splat of a dict literal (`json.dumps({"data": m.model_dump()})`), an item
  of a list or tuple literal, the body of a list comprehension, or a name the function last assigned from one of these
  (`payload = m.model_dump(); json.dumps(payload)`).

The receiver must hold a known model: a constructor call, a name bound to one, `self` in a method of the model, a
parameter annotated with it (lib/pydantic's `pd_instance`), or the variable of a comprehension or `for` loop over a
parameter annotated `list[Model]`. The model's fields are read where it is defined, across files: its own, those it
inherits from the run's classes, its `@computed_field` return types, and the fields of the models its fields hold
(alone, `Optional`, in `list[...]` or `dict[str, ...]`, to any depth). A field is reported when its annotation names
`datetime`, `date`, `time`, `timedelta`, `UUID` (and pydantic's `UUID1`..`UUID8`), `Decimal`, `bytes`, `bytearray`,
`set`/`frozenset`, a `pathlib` path, an `ipaddress` type, `SecretStr`/`SecretBytes`, pydantic's URL and DSN types
(Url objects in Pydantic 2), pydantic's constrained (`condecimal`, `conset`, ...), past/future and aware/naive date
types, or a run `Enum` that is not also a `str`, `int` or `float`. The message names the model, the first such field
and its type, and the sink.

**Leaves alone**: `model_dump(mode="json")`, `model_dump_json()`, `json.dumps(..., default=...)` or `cls=...`,
orjson, FastAPI's `jsonable_encoder(...)`, Flask's `jsonify(...)`, a project's own json module (superset's `from
superset.utils import json`; also every `json.dumps` when the run has a module named `json`), a client the function
did not build (a TestClient fixture, `self.client`). A model whose fields are all JSON-native (str, int, float, bool,
None, `Literal`, `str`/`int` enums, lists and dicts of those, nested models of those): it encodes today and
`mode="json"` would change nothing, so only proven non-JSON fields are reported; this is the rule's main precision
decision, and on the corpus every `json.dumps(m.model_dump())` it found was of such a model. Also left alone: a dump
with `include=` or `exclude=` (not followed), a model that defines its own `model_dump` or `dict` (letta's
`LettaBase`, which converts before returning), has a `@model_serializer` or sets `json_encoders` (Pydantic 2 applies
`json_encoders` in JSON mode only, so such a model can still fail; it is left alone as a sign the author handles
encoding); a field with `Field(exclude=True)`, a `@field_serializer` for it (unless `when_used="json"` or
`"json-unless-none"`, which do not run in Python mode), or a `PlainSerializer`/`WrapSerializer` in its `Annotated`;
an enum field of a model with `use_enum_values=True`; a field the model's own code replaces after validation (a
method of the model or a run base assigns `self.x = ...`, as an after model validator's `self.host = str(self.host)`,
or an after/plain/wrap field validator for it returns a value; a before validator's result is validated into the
declared type again, so it is still reported); pydantic.v1 models; URL fields in a project pinned to Pydantic 1
(v1's URLs are str); a name the function changes after the dump (`payload["at"] = ...`, `payload.update(...)`); and
test code, where such a dump fails the first time the test runs.

**Gaps**: a receiver whose model is not known (a function's return value, an attribute such as
`self._entities`, an untyped or `Any` parameter, an `isinstance`-narrowed loop variable); a field typed `Any` or
`dict[str, Any]` that holds a datetime at runtime (the awslabs/mcp incident below is this shape and is not caught);
type aliases (`UTCDatetime = Annotated[datetime, ...]`) and types from other libraries (pydantic_extra_types); a
model from outside the run; a dump passed through a helper before it reaches the encoder; `requests` uses simplejson
when it is installed, which encodes `Decimal` (not datetime or UUID); an inherited field overridden two levels down;
a SQLAlchemy JSON column or a Celery JSON-serialized task argument fed a dump (both encode with json too; not sinks
here).

**Overlap**: none. mypy and pyright accept `json.dumps(m.model_dump())` (`model_dump` returns `dict[str, Any]`); Ruff,
Bandit, pylint, flake8-pydantic and the pydantic mypy plugin have no such check.

**Sources**: Pydantic docs, [Serialization](https://docs.pydantic.dev/latest/concepts/serialization/) (Python mode
output "may contain non-JSON-compatible Python objects"; `mode="json"` / `model_dump_json()` are JSON-safe); Python
docs, [json conversion table](https://docs.python.org/3/library/json.html#py-to-json-table) (anything else raises
`TypeError`); incidents: [awslabs/mcp#1990](https://github.com/awslabs/mcp/issues/1990) (`json.dumps(data.model_dump())`
broke every Glue/Athena/EMR operation returning a timestamp; fixed with `mode='json'`),
[crewAIInc/crewAI#7358](https://github.com/crewAIInc/crewAI/issues/7358) (flow persistence crashes on datetime, UUID,
set, Decimal and Path state fields), [google/adk-python#6747](https://github.com/google/adk-python/issues/6747)
(Python-mode dump of an output schema is not JSON-serializable).

**Corpus**: 0 matches in the 11 Pydantic repositories run: fastapi/full-stack-fastapi-template@77e550a73bb4,
fastapi-users/fastapi-users@e9c57ff7ff2f, freqtrade/freqtrade@7fcb509a434c, open-webui/open-webui@0666e59d26a4,
Netflix/dispatch@dd2837e82a0b, mealie-recipes/mealie@8dbb92808d5a, polarsource/polar@872272fbfc6e,
langgenius/dify@724ac15121ee, apache/superset@fdf61721d056, zenml-io/zenml@61569b1e08e7 and letta-ai/letta@1131535716e8.
Every `json.dumps(m.model_dump())` there was read and is of a JSON-native model (dify's `chunk.model_dump()` in
`api/core/agent/cot_agent_runner.py` dumps `AgentScratchpadUnit.Action`, str and dict fields; `text_store.py` and
`vector_store.py` dump `Identifier`/str scopes; `workflow_tools_manage_service.py` dumps str fields and a str enum;
`rag_pipeline_task_proxy.py` str and `dict[str, Any]`), superset's `mcp_core.py`/`instance_metadata.py` go through
superset's own json, and letta's `errors.py:404` dumps `Message`, whose base `LettaBase` overrides `model_dump`. To
read real matches, the rule also ran over 30 small public repositories found by GitHub code search for
`json.dumps(self.model_dump())`, `json.dumps(event.model_dump())`, `json=payload.model_dump()` and similar
(appended to `MANIFEST.tsv` under `_wild-pydantic-dump/`). The first version reported 5 in aruba/pyafc@720e5d37ec11:
3 were false (`SyslogEntry`, `SnmpTrapServer` and `DhcpRelay` hold an `IPv4Address` that an after model validator
turns into a str, `self.host = str(self.host)`), which led to the "field rewritten after validation" exclusion and
its cases. The final rule reports 3, all real (3/3):
`pyafc/route_policies/route_maps.py:131` and `:257` (`RouteMap` / `RouteMapEntry` with
`match_ipv4_next_hop_address: IPv4Address`, posted as `data=json.dumps(data.model_dump(exclude_none=True))`: creating
a route map entry with a next hop fails), and EcorRouge/rococo-email-parser@1d4232d5c551 `rococo/models/email.py:103`
(`Email.__repr__` is `json.dumps(self.model_dump())` while `date: Optional[datetime]` is set by the parser: `repr()`
of any parsed email raises). The other 28 repositories had 0 matches (their dumped models are JSON-native).
`pd_typed_instance` (python/types) as an extra receiver found nothing more on fastapi-template, freqtrade, mealie
and dify and cost 3 s vs 3 s, 28 s vs 12 s, 20 s vs 12 s and 599 s vs 131 s: it is not used.

### `pydantic-practice-secret-settings-str`

A secret on a Pydantic settings class typed as a plain `str`: `SECRET_KEY: str`, `SMTP_PASSWORD: str | None = None`,
`OPENAI_API_KEY: Annotated[str, Field(min_length=20)]`. The settings object is the one a process prints, logs or
dumps at start-up, and `repr()`, `str()`, `model_dump()`, `model_dump_json()` of it, and a `ValidationError` raised on
it, print a `str` field in clear. `SecretStr` prints `'**********'` in all of them until `.get_secret_value()`.

**Catches** a field of a `pydantic_settings.BaseSettings` class (through the run's classes in any module), inherited
ones included (a settings base or a `BaseModel` mixin in another file, reported once at its declaration), and a field
of a plain model a settings class holds as a field (`smtp: SmtpConfig`, nested settings), when:

- its annotation is made only of `str` and `None` through `Optional[...]`, `Union[...]`, `|` and `Annotated[..., ...]`;
- a word part of its name (split at `_` and at camel-case humps, case-insensitive) is `password`, `passwd`, `secret`,
  `token`, `api_key`/`apikey`, `private_key`, `client_secret` or `credential(s)`: `SECRET_KEY`,
  `STRIPE_WEBHOOK_SECRET`, `slackBotToken`, not `TOKENIZER` or `PASSWORDLESS_LOGIN`;
- its name's last part does not say the value is something else (`_url`, `_uri`, `_path`, `_file`, `_dir`, `_name`,
  `_header`, `_algorithm`, `_prefix`, `_ttl`, `_lifetime`, `_length`, `_type`, `_arn`, `_enabled`, `_env`, `_var`,
  `_id`), and no part is `expire(s)`, `expiry`, `expiration`, `min` or `max`.

The message names the field and counts the reads of `.NAME` in the run's files: changing the type to `SecretStr` breaks
every read that uses the value as a string (`f"postgresql://u:{settings.DB_PASSWORD}@..."` silently gets
`'**********'`, `hmac.new(settings.SECRET_KEY.encode(), ...)` raises), so the message says to change each one to
`.get_secret_value()`. That is why the rule ships in `log` mode: following it means touching every reader.

**Leaves alone**: `SecretStr`, `SecretBytes`, `Secret[...]` and project types deriving from them; types that are not
a plain string (`ACCESS_TOKEN_EXPIRE_MINUTES: int`, `SECRET_KEYS: list[str]`, `Literal[...]`, URL and DSN types); a
field its author masked (`Field(repr=False)`, `Annotated[str, PlainSerializer(...)]` or `WrapSerializer`, a
`@field_serializer` naming it, a project alias such as mealie's `MaskedNoneString`, which is not read through); DSN
fields (`DATABASE_URL: str`) even when the URL embeds a password, since the name says URL and many carry none (a gap,
not a judgement); request and response models (`password: str` on a login body is input; the fastapi pack owns
response leaks); non-Pydantic config classes and dataclasses; `pydantic.v1` settings; `ClassVar` and `_private`
attributes; test code; and settings classes in migrations (`alembic/`, `migrations/`), which read the environment once
for that migration.

**Gaps**: a quoted annotation (`X_PASSWORD: "str"`); a type alias of `str`; a secret whose name has none of the words
(`ENCRYPTION_KEY`, `SIGNING_KEY`, `PEPPER`); a Vault AppRole `SECRET_ID` (left alone with every `_id`); a settings base
outside the run that is not a known base; `pydantic.v1` settings classes, which have the same bug. The read count
matches `.NAME` on any object, so a lower-case name (`password`) counts reads of other objects' attributes too.

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
| mealie-recipes/mealie | `8dbb92808d5a` | 4 | 4 | `mealie/core/settings/settings.py`: `SECRET`, `SESSION_SECRET` (excluded from the start-up log by hand), `REVERSE_PROXY_AUTH_SECRET` (not excluded: logged in clear by `mealie/app.py`); `db_providers.py`: `POSTGRES_PASSWORD`. Its `MaskedNoneString` fields stay silent |
| polarsource/polar | `872272fbfc6e` | 35 | 34 | all `server/polar/config.py` (`STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `AWS_SECRET_ACCESS_KEY`, `GITHUB_CLIENT_SECRET`, `TINYBIRD_API_TOKEN`, ...); not real: `POSTHOG_PROJECT_API_KEY`, PostHog's public, write-only project key. `BACKOFFICE_GOOGLE_CLIENT_SECRET: SecretStr`, `CURRENT_HASH_SECRET_ID`, `AWS_HASH_SECRET_ARN` stay silent |
| zenml-io/zenml | `61569b1e08e7` | 0 | - | its `BaseSettings` is zenml's own class; its configs use `SecretField()` |
| Case211/remnawave-admin (wild) | `1ca135f7f82c` | 13 | 13 | `web/backend/core/config.py` (`secret_key`, `admin_password`, ...), `src/config.py`, `shared/config.py`, `node-agent/src/config.py` |
| yfyfyufeng/Phoenix-Agent (wild) | `b55404dbcf2a` | 12 | 12 | `backend/app/infrastructure/config.py`: `api_key`, `serp_api_key`, `email_password`, `credentials_secret_key`, ... |
| shaun17/MemoLite (wild) | `e56211913e53` | 1 | 1 | `src/memolite/common/config.py`: `mcp_api_key` |
| speaches-ai/speaches (wild) | `993994f7984b` | 0 | - | already `SecretStr` (`api_key`, `chat_completion_api_key`): the near miss |

68 matches read, all of them; 67 real (98.5%). Before the migration exclusion, mealie also matched
`OPENAI_API_KEY` on a settings class inside an Alembic migration
(`mealie/alembic/versions/2026-05-18-16.27.05_2187537c52b8_add_table_for_ai_providers.py`), now left alone.
fastapi-users, freqtrade, open-webui, Netflix/dispatch and apache/superset define no `pydantic_settings` class, so the
rule has nothing to read there. langgenius/dify (about 80 secret-named `str` fields under `api/configs/`) was not run:
the shared corpus queue held 130 to 155 runs; a first attempt was cut by its caller's 20-minute limit while running,
and a second did not leave the queue within 53 minutes. The rule ships in `log` despite its precision: the fix
changes every reader of the field, and a reader missed gets `'**********'` silently.

### `pydantic-practice-v1-api-in-v2`

A project pinned to Pydantic 2 still calls Pydantic 1's API on its v2 models. Some of it no longer works at all; the
rest is deprecated since 2.0 and gone from Pydantic 3 (pydantic's `main` branch has already deleted it). The rule
reads the project's manifests (`pyproject.toml`, `requirements*.txt`, `setup.cfg`, `uv.lock`, `poetry.lock`, `Pipfile`,
at the root and up to 3 levels below it) to tell v1 code from v2 code: in a directory whose nearest manifests pin
Pydantic 1, or with no pin and no v2-only import, it says nothing.

**Catches**, in two groups with their own messages:

1. Removed in v2. The risk here is availability, not maintenance: the module fails to import, a call raises, or a
   setting is silently dropped. Each shape was checked against pydantic 2.11.4 and 2.12.5:
   - `Field(regex=...)`, `Field(const=...)`, `Field(unique_items=...)`: `PydanticUserError` when the class is created
     (pydantic's own `Field` only: `sqlmodel.Field` still takes `regex=` and maps it);
   - `from pydantic import BaseSettings`, `pydantic.BaseSettings`: `PydanticImportError`;
   - `@root_validator` or `@root_validator()` without `pre=True` or `skip_on_failure=True`: `PydanticUserError` when the
     class is created;
   - in a nested `class Config:`, the keys v2 dropped (`fields`, whose aliases are then lost, `allow_mutation`,
     `getter_dict`, `smart_union`, `copy_on_model_validation`, `json_loads`, `json_dumps`,
     `underscore_attrs_are_private`, `error_msg_templates`, `post_init_call`) and the keys v2 renamed (`orm_mode`,
     `allow_population_by_field_name`, `schema_extra`, `validate_all`, `anystr_strip_whitespace`, `anystr_lower`,
     `anystr_upper`, `max_anystr_length`, `min_anystr_length`, `keep_untouched`). v2 warns about both at class creation
     and then ignores them: it does not read a renamed key under its old name, so `orm_mode = True` leaves
     `from_attributes` off, and `allow_population_by_field_name = True` leaves population by name off;
   - `.json(...)` with any argument the deprecated method does not take (`indent`, `sort_keys`, `ensure_ascii`,
     `default`, `separators`, `encoder`, `models_as_dict`, ...): `TypeError`;
   - `M.from_orm(obj)` where M's config, its run bases' included (in any file), does not set `from_attributes`:
     `PydanticUserError` (an `orm_mode = True` does not count, as above).
2. Deprecated since v2, removed in v3, on a v2 model class or instance: `.dict()`, `.json()`, `.parse_obj()`,
   `.parse_raw()`, `.parse_file()`, `.from_orm()` (on a model with `from_attributes`), `.copy()`, `.construct()`,
   `.schema()`, `.schema_json()`, `.update_forward_refs()`, `.validate()`, `__fields__`, `__fields_set__`;
   `@validator`; `@root_validator(pre=True)` and `@root_validator(skip_on_failure=True)`; a nested `class Config:`
   (reported once, on its `class` line); `Field(min_items=, max_items=)`; and the uses (not the imports) of
   `parse_obj_as`, `schema_of`, `schema_json_of`, `pydantic.json.pydantic_encoder` and `validate_arguments`.

A line gets one message: a `.json(...)` or `.from_orm(...)` reported in group 1 is not reported again in group 2, a
`Field` call with a removed argument is not reported for `min_items`, and a `class Config` key is reported on its own
line only when it is in group 1.

The receiver of a method must resolve to a v2 model: `M.parse_obj(...)` on the class, an instance built as `M(...)` or
`M.model_validate(...)` (or any constructor, v1's included) and the name the same scope binds to it, `self` in a
method of M, a parameter annotated with M (a FastAPI body: `def create(body: UserCreate)`). A model is a class deriving
from `pydantic.BaseModel`, `RootModel`, `pydantic_settings.BaseSettings` or `sqlmodel.SQLModel`, through the run's
classes in any file.

**Leaves alone**: `.dict()`, `.copy()` or `.json()` on anything not known to be a model (`resp.json()`,
`settings.copy()`, a SQLAlchemy row); a method the project defines itself on the model or a base in the run (a
`def dict(self)` override), and a field of that name (`validate: Callable[...]`); models deriving from
`pydantic.v1.BaseModel`, and `pydantic.v1`'s `Field`, `validator` and `root_validator`; a `validator` of another
library or of the project; the v2 API (`model_config = ConfigDict(...)`, `@field_validator`, `model_dump`,
`model_dump_json(indent=2)`); code under an `if` whose test names the Pydantic version or the v2 API (`if
PYDANTIC_V2:`, `if hasattr(m, "model_dump"):`), and code in a `try` that catches `ImportError`, `ModuleNotFoundError`
or `AttributeError` (a compatibility shim such as `try: from pydantic_settings import BaseSettings except ImportError:
from pydantic import BaseSettings`); a project pinned to Pydantic 1, or with no pin and no v2-only name. Tests
(`tests/`, `test_*.py`, `conftest.py`, ...) get group 1 only: a crash there is still a crash, while the deprecation
warnings are the suite's own business.

**Gaps**: an instance the rule cannot type (a function's return value, an attribute such as `self.form.dict()`, a
list item) is not reported, and neither is a model whose base is a library's model outside the run (fastapi-users'
`BaseUser`, LangChain's classes). Receivers typed by python/types' inference (`pd_typed_instance`) are not read: on
Netflix/dispatch a variant that read them found the same 161 matches and no other, for about 3 s more. Renamed or
removed keys inside `model_config = ConfigDict(...)` (which v2 ignores too) are not reported; `.dict()` and `.copy()`
with arguments v2 refuses are reported as deprecated, not as crashes; `json_encoders` (deprecated, still honoured) is
not reported. A `pydantic` requirement written as Poetry's bare string (`pydantic = "^1.10"`) is not read as a pin
(lib/pydantic's `pd_manifest_pin`), so a Poetry project pinned to v1 that way and holding a v2 pin elsewhere above it
would be taken for v2.

**Overlap**: bump-pydantic (archived by pydantic) is a one-shot codemod, not a linter: it rewrites imports, Config
classes, Field parameters and validators (BP002, BP003, BP004, BP007) but has no rule for the method renames
(`.dict()`, `.parse_obj()`, `.from_orm()`), and it does not know which project is on which major. mypy with
`enable_error_code = deprecated` and pyright's `reportDeprecated` (a hint by default) report pydantic's
`@deprecated` methods only where the receiver's type is known and only once configured; neither knows `class Config`
keys, `Field(regex=)`, the `root_validator` crash or `.json(indent=)`. Ruff has no Pydantic rules (its UP rules do not
touch pydantic) and flake8-pydantic has none of these. What this rule adds: the manifest-pinned version gate (v1 code
in a v1 project, or a v1 sub-project of a v2 monorepo, stays silent), the `pydantic.v1` model exclusion, and the crash
group, which v2 itself only reports when the code runs.

**Sources**: Pydantic migration guide, https://docs.pydantic.dev/latest/migration/ ("Changes to pydantic.BaseModel",
"Changes to pydantic.Field", "Changes to config", "@validator and @root_validator are deprecated", "BaseSettings has
moved to pydantic-settings"); Pydantic usage errors, https://docs.pydantic.dev/latest/errors/usage_errors/
(`removed-kwargs`, `root-validator-pre-skip`); pydantic 2.x `pydantic/main.py`, where each old method is
`@typing_extensions.deprecated(...)` and emits `PydanticDeprecatedSince20`, and `pydantic/_internal/_config.py`
(`V2_REMOVED_KEYS`, `V2_RENAMED_KEYS`: a warning, then the keys are ignored); bump-pydantic's README
(https://github.com/pydantic/bump-pydantic).

**Corpus** (commits from MANIFEST.tsv; every match checked, its receiver's type included; tests are left out of
group 2):

| Repository | Commit | Pydantic pin | Matches | Real |
|---|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | 77e550a7 | `pydantic>=2.13.5` | 0 | |
| fastapi-users/fastapi-users | e9c57ff7 | none in its manifests | 0 | |
| open-webui/open-webui | 0666e59d | `pydantic==2.13.4` | 1 | 1 |
| freqtrade/freqtrade | 7fcb509a | `pydantic==2.14.3` | 0 | |
| mealie-recipes/mealie | 8dbb9280 | `pydantic==2.14.3` | 0 | |
| Netflix/dispatch | dd2837e8 | `pydantic==2.11.4` | 161 (74 files) | 161 |
| letta-ai/letta | 11315357 | `pydantic==2.11.4` | 1 | 1 |

163 matches, 163 real (100%), all in group 2; no group-1 shape occurs in these repositories. Examples:
`src/dispatch/case/service.py:282` `case_in.dict(exclude_unset=True, ...)` on a `CaseUpdate` body (117 such `.dict()`
calls, every receiver a parameter annotated with a dispatch schema); `src/dispatch/case/views.py:319`
`CaseRead.from_orm(current_case)` (12, on schemas whose `DispatchBase` sets `from_attributes=True`);
`src/dispatch/case/views.py:136` `CaseExpandedPagination(**pagination).json()` (21);
`src/dispatch/plugins/dispatch_slack/middleware.py:91` `SubjectMetadata.parse_raw(...)` (4);
`src/dispatch/case/models.py:430` `CaseReadMinimal.update_forward_refs()`; `src/dispatch/plugin/models.py:28` and five
Alembic revisions, `pydantic_encoder(obj)`; open-webui `backend/open_webui/routers/ollama.py:1072`
`@validator('content', pre=True)`; letta `letta/schemas/letta_message.py:255`, a nested `class Config` holding
`json_encoders`. Near misses the corpus showed silent: dispatch's ~45 `x.dict()` calls on SQLAlchemy models (its
declarative `Base` defines `dict()`), letta's `try: model_cls.model_fields ... except AttributeError:
model_cls.__fields__` and `if hasattr(tool, "model_dump"): ... elif hasattr(tool, "dict"): tool.dict()` shims, and
open-webui's `Valves.schema()` on classes loaded at run time. Volume: dispatch, a v2 codebase that kept the v1 method
names, carries 161 deprecated calls in 74 files, so its own convention disagrees with group 2. The steer is still
safe to follow (each replacement is pydantic's own method, taking the same arguments), but a team that has not
planned the v3 move may prefer group 2 in `log`; group 1 is worth a steer every time.

### `pydantic-practice-validator-returns-value`

Pydantic keeps what a validator returns. A field validator's return value becomes the field's value, so a path that
ends without a `return` (or with a bare `return`) silently sets the field to None, even a non-Optional one: an after
validator's result is not validated again. A before model validator's return value replaces the input data, a wrap
model validator's is the validated model, and an after model validator must `return self`: without it
`Model.model_validate(data)` returns None (`Model(...)` still works, with only a UserWarning since 2.9), and a field of
that model nested in a parent becomes None.

**Catches**, in a def decorated with Pydantic's `field_validator` (any mode), `model_validator` (before, after, wrap),
or v1's `validator` / `root_validator` (pre or not; they still run under Pydantic 2): the def's name when its body can
reach its end, and each bare `return`. The decorator is resolved through imports, re-exports included, and the def may
sit in any class: a mixin in one module whose model is declared in another is checked where the validator is written.
"Can reach its end" is `django-practice-clean-field-returns-value`'s analysis: a return or raise closes a path; an
`if`, `try`, `with`, loop or `match` is open when one of its ways through is (an `if` without `else`, a handler that
logs and falls through, a `for` loop with no `return` after it, a `match` with no catch-all); `assert` is not a way
out. Shapes in the cases: the happy path of an `if` that only raises, an `if/else` whose other branch only logs, a
validating loop, `try/except` that logs, an `elif` chain without `else`, a lone `assert`, `@model_validator(mode="after")`
that raises or ends, `@model_validator(mode="before")` that mutates `data` in place, a wrap model validator's bare
`return` in its handler, v1 `@validator` and `@root_validator`.

**Leaves alone**: validators whose every path returns or raises (`raise` in every `except`; `while True` left only by
`return`; a `for/else` that raises; a `match` whose every case returns); `return None` written out (a deliberate
None); `field_serializer`, `model_serializer` and `computed_field` (not validators); decorators of other libraries
named like Pydantic's (attrs' `@x.validator`, marshmallow's `validates` / `validates_schema`, DRF's
`validate_<field>` methods, a project's own `validator` function); generators; stubs whose body is only a docstring,
`pass` or `...` (abstract or not) and `typing.overload` stubs (`raise NotImplementedError` raises, so it is closed);
`return self` inside a `with` block; and test code (`tests/`, `test_*.py`, `conftest.py`, ...): test modules build
throwaway models and sometimes leave a validator without a return on purpose to check what Pydantic does with it.

**Gaps**: a call that always raises (a helper, `sys.exit()`, a `NoReturn` function) is taken as completing, so a
validator ending in one is reported. An after model validator that returns `None` explicitly, or something other
than `self`, is not reported. Validators attached outside a class body (`field_validator(...)(fn)`, `Annotated[...,
AfterValidator(fn)]`, `BeforeValidator`, `WrapValidator`) are not looked at: their function is an ordinary def.

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

**Corpus**: 0 matches in 4 repositories, so precision has no denominator; no false positive.

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
