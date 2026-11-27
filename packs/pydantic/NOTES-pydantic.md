# Notes: pydantic (Rerail's own Pydantic v2 practice rules)

The mining behind `packs/pydantic/`: the sources read, every candidate with its verdict, and the build list. Each
shipped rule's own evidence (cases, corpus matches, precision) is in [README.md](README.md#rule-details).

## 1. Sources read

| Source | URL | What it contributed |
|---|---|---|
| Pydantic migration guide (V1 → V2) | https://docs.pydantic.dev/latest/migration/ | Renamed BaseModel methods, removed `Field` kwargs (`const`, `regex`, `unique_items`), removed/renamed config keys, `@validator`/`@root_validator` deprecation and the `skip_on_failure` crash, "TypeError is no longer converted to ValidationError", `Optional[T]` without default is required, BaseSettings moved, `.json()` kwargs errors |
| Pydantic docs, Validators | https://docs.pydantic.dev/latest/concepts/validators/ | A field validator returns the validated value; an after model validator must return the instance; before validators get "the raw input, which in theory could be any arbitrary object" (examples guard with `isinstance`); only ValueError, AssertionError, PydanticCustomError become validation errors |
| Pydantic docs, Models | https://docs.pydantic.dev/latest/concepts/models/ | `model_construct()` skips validation ("only ... with data which has already been validated, or that you definitely trust"); `model_copy(update=...)` updates are not validated; custom `__init__` is now called by every validation method (dropped a candidate) |
| Pydantic docs, Fields | https://docs.pydantic.dev/latest/concepts/fields/ | `default_factory` for per-instance defaults (`uuid4().hex` example); `Field` signature |
| Pydantic docs, Serialization | https://docs.pydantic.dev/latest/concepts/serialization/ | `model_dump()` python mode vs `mode="json"`; `by_alias` default False (and `serialize_by_alias` since 2.11) |
| Pydantic usage errors | https://docs.pydantic.dev/latest/errors/usage_errors/ | `removed-kwargs`, `validator-instance-method` (a field validator on `self` fails at class creation: dropped as self-revealing) |
| Pydantic docs, Secret types / Settings | https://docs.pydantic.dev/latest/api/types/#pydantic.types.SecretStr, https://docs.pydantic.dev/latest/concepts/pydantic_settings/ | `SecretStr` masks in repr/str/dumps; settings docs use it for credentials |
| Pydantic mypy plugin | https://docs.pydantic.dev/latest/integrations/mypy/ | What the plugin checks (`init_typed`, `init_forbid_extra`, `warn_required_dynamic_aliases`): typed `__init__` calls, not validator bodies, dumps, versions or secrets |
| pydantic 2.14.4 `pydantic/main.py` and `fields.py`; pydantic `main` branch `main.py` | https://github.com/pydantic/pydantic/blob/v2.14.4/pydantic/main.py | The v1 methods are `@typing_extensions.deprecated` in 2.14 (so mypy/pyright can report them when configured); `fields.py` raises for `regex`/`const`/`unique_items`; on `main` (V3) the v1 methods and `pydantic.v1` are gone |
| pydantic SECURITY.md | https://github.com/pydantic/pydantic/blob/main/SECURITY.md | Trust boundary: the data validated is untrusted, application code (validators) trusted; framing for the raw-input and construct candidates |
| pydantic/pydantic#9334 | https://github.com/pydantic/pydantic/issues/9334 | An after model validator without `return self` makes `model_validate` return None; maintainers added a warning (PR #10255); mentions flake8-pydantic |
| pydantic/pydantic#7252 | https://github.com/pydantic/pydantic/issues/7252 | The nested-model form of the same bug (`parent.child` is None) |
| fastapi/fastapi#4974 | https://github.com/fastapi/fastapi/issues/4974 | A validator exception that is not a validation error surfaces as a 500 instead of a 422 |
| bump-pydantic README (archived) | https://github.com/pydantic/bump-pydantic | BP001-BP010: a one-shot codemod (Config, Field params, imports, decorators, `Optional` defaults); no method renames (`.dict()`, `.from_orm()`), not a linter |
| flake8-pydantic README | https://github.com/Viicos/flake8-pydantic | PYD001-PYD010 (positional Field default, non-annotated attribute, redundant Field, Annotated default, name/annotation clash, `__pydantic_config__`): no overlap with any candidate here |
| Ruff RUF009, RUF012, B008 | https://docs.astral.sh/ruff/rules/function-call-in-dataclass-default-argument/ | RUF009 covers per-call defaults in dataclasses only; RUF012 exempts Pydantic models; B008 covers function parameter defaults |
| Ruff S105/S106/S107, Bandit B105-B107 (the `python` pack's ports) | `packs/python/pack.json` | Hard-coded password literals only; no type-based secret check |
| Python `json` docs | https://docs.python.org/3/library/json.html#py-to-json-table | What `json.dumps` encodes; TypeError otherwise |
| Stack Overflow 73128975 "Pydantic created at and updated at fields" | https://stackoverflow.com/questions/73128975 | Teams reach for `datetime.now()` defaults; answers move to `default_factory` |
| SQLModel docs, UUID | https://sqlmodel.tiangolo.com/advanced/uuid/ | `Field(default_factory=uuid.uuid4)` as the documented id default |
| pydantic discussions #11210, issues #8379, #10141 (aliases) | https://github.com/pydantic/pydantic/discussions/11210 | `model_dump()` drops aliases by default; users ask for a per-model default (now `serialize_by_alias`) |
| OWASP Logging Cheat Sheet, CWE-532 | https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html | Secrets must not reach logs |

## 2. Corpus probes

Before any rule was written the library (`rules/lib/pydantic.rerail`) was proven with a probe rule on a case
directory (models, inherited config, fields, defaults, validators and their modes, instances through constructors,
annotated parameters and `self`, the `pyproject.toml` pin) and on two corpus repositories (full-stack-fastapi-template:
21 models, 43/43 documents on v2; mealie: 344 models, 72 validators, 723/723 documents on v2). Then a candidate probe
(`corpus-run`, one clause per candidate) ran on the 10 Pydantic repositories (full-stack-fastapi-template,
fastapi-users, open-webui, freqtrade, dispatch, mealie, polar, dify, superset), and a CPython `ast` scan of 14
repositories (those plus jupyterhub, saleor, InvenTree, healthchecks, label-studio) checked the validator shapes.
Counts below are raw probe matches, before any rule's own filtering.

| Probe | Matches | Where |
|---|--:|---|
| v1 API on a v2 model (`.dict()`, `.json()`, `.from_orm()`, `.parse_raw()`, `update_forward_refs`, `@validator`, `class Config`) | 198 | dispatch 129 `.dict()` + 41 `.from_orm()`, open-webui, dify |
| `Optional[X]`/`X \| None` field with no default | 668 | dify 353, polar 227, dispatch 54, superset 31 |
| `model_copy(update=...)` | 516 | dify 424, polar 42, superset 34 |
| secret-named `str` field in a BaseSettings model | 139 | dify 94, mealie, polar, full-stack template |
| secret-named `str` field in a plain model | 172 | request bodies (`password: str`) |
| `model_dump()`/`.dict()` without `by_alias` on a model with aliases | 136 | mealie 106 (ORM writes by field name), dify, superset |
| before-mode validator calling a method on the raw value, no guard (engine) / (`ast`) | 72 / 61 | dify 71, freqtrade, saleor |
| `model_construct()` / `.construct()` | 68 | dify 60 (tests, internal values), mealie tests |
| `json=` / `json.dumps` of a python-mode dump | 46 | mealie tests 37, dify 9 |
| validator that falls off its end or returns bare | 0 (1 probe hit, a validator that always raises) | |
| `raise TypeError` in a validator | 1 (dify, deliberate: a programming-error guard) | |
| field default calling `now()`/`uuid4()`/... | 0 (model bodies; test locals only) | |
| removed v1 kwargs (`Field(regex=)`, `const=`), `from pydantic import BaseSettings`, `root_validator` without `skip_on_failure`, removed Config keys | 0 | |

Every corpus repository with Pydantic pins v2 (`pydantic==2.x`, `>=2`, `pydantic-settings`, or a `uv.lock` entry at
2.x), which the library reads from the manifests.

## 3. Candidates

| Id | Catches | Why it is a bug | Sources | Cross-file | Linter overlap | Detection | Verdict | Reason |
|---|---|---|---|---|---|---|---|---|
| validator-returns-value | a field/model validator (any mode, v1's too) that can end, or `return`s bare, without returning the value / `self` | the field silently becomes None; `model_validate` returns None; a nested model becomes None | validators docs; #9334; #7252 | base/mixin validators resolve through imports | RET503/R1710 only when another path returns a value; mypy only if annotated | `pd_validator` + the "can complete normally" analysis of django-practice-clean-field-returns-value | build | silent wrong data; 0 in corpus but two upstream issues and a runtime warning added for it |
| dump-mode-json | `json.dumps(m.model_dump())`, `requests/httpx(json=m.model_dump())` where m's model (read across files) has a datetime/UUID/Decimal/... field | `TypeError: Object of type datetime is not JSON serializable` | serialization docs; Python json docs | model, bases and nested models in other modules | none (`model_dump` returns `dict[str, Any]`) | `pd_instance`/`pd_typed_instance` + field annotation types | build | outage of a code path; type-aware; 46 raw sink matches to judge |
| before-validator-raw-input | a before/wrap/plain validator (v1 `pre=True`, `root_validator(pre=True)`) calling methods on / subscripting the raw value without a type guard; an after model validator written as `(cls, values)` | AttributeError/TypeError escape as 500s, not 422s (v2 converts only ValueError/AssertionError) | validators docs; migration guide; fastapi#4974 | optional: models reached by external input | none | `pd_validator`, `pd_value_param` + guard analysis | build | 61-72 corpus matches; precision depends on input exposure, likely log |
| default-factory | a model field default that calls `now()`, `uuid4()`, `token_urlsafe()`... once at import | every instance shares one timestamp / id (PK collisions) | fields docs; RUF009 rationale; SO 73128975; SQLModel UUID docs | inherited models | RUF009 (dataclasses only), B008 (parameters only) | `pd_field_default` + callee `sym_of` | build | data safety; 0 in corpus, but exact and cheap, and the Django pack ships the same rule |
| v1-api-in-v2 | v1 APIs (`.dict()`, `.from_orm()`, `@validator`, `class Config`, removed `Field` kwargs, `pydantic.BaseSettings`, removed Config keys) on v2 models in a project pinned to v2 | deprecated (removed on pydantic `main`/V3); some crash at import or are silently ignored (`Config.fields`) | migration guide; usage errors; pydantic 2.14 `main.py`; bump-pydantic | manifest pin (pyproject/requirements/lock) + model resolution across files | bump-pydantic (codemod, no method renames); mypy/pyright `deprecated` only when configured and typed | `pd_v2_doc`, `pd_model_call`, `pd_config_class`, `pd_validator_deco` | build | 198 real-looking matches (dispatch) |
| secret-settings-str | a BaseSettings field named like a secret typed `str` | the settings object is printed/logged/dumped; `str` prints in clear, `SecretStr` masks | SecretStr docs; pydantic-settings docs; OWASP; CWE-532 | settings read everywhere; optional: the object reaching a log | S105-S107 only for literal defaults | `pd_settings`, `pd_has_field`, name/type | build (log) | 139 matches; steer is unsafe (readers need `.get_secret_value()`) |
| optional-without-default | `Optional[X]` field with no default (v2: required) | clients omitting the field get 422 | migration guide table; bump-pydantic BP001 | — | mypy plugin (`init_typed`) at call sites | annotation text | drop | 668 matches, most deliberate: polar (v2-native) has 227 required-nullable response fields; intent cannot be told apart, even on request models (open-webui config forms send every key) |
| copy-update-unvalidated | `model_copy(update=...)` | updates are not validated | models docs | — | none | method call | drop | 516 matches, all internal values in the read sample; the documented API, not a bug by itself |
| construct-untrusted | `model_construct(**untrusted)` | validation bypass | models docs; SECURITY.md | — | none | method call + source | drop | 68 matches, all tests or already-validated internal values (dify `billing_client/transport.py` salvages fields one by one on purpose); no untrusted shape found |
| alias-dump-without-by-alias | `model_dump()` of a model with aliases, no `by_alias` | wire format loses aliases | serialization docs; discussions #11210, #10141 | model aliases defined elsewhere | none | `pd_model_call` + alias config | drop | 136 matches, nearly all ORM writes by field name (mealie's 106) or receivers that `populate_by_name`; only a JSON wire sink to a strict receiver is a bug, which the code cannot show |
| validator-raises-typeerror | `raise TypeError` in a validator | v2 no longer converts it: 500 not 422 | migration guide | — | none | raise in `pd_validator` | drop | 1 match (dify, deliberate programming-error guard); folded the real hazard into before-validator-raw-input |
| removed-v1-kwargs (separate rule) | `Field(regex=)`, `const=`, `from pydantic import BaseSettings`, `root_validator` w/o `skip_on_failure` | crash at import | migration guide; usage errors | pin | bump-pydantic BP003/BP004/BP007 | `pd_field_call`, `sym_of` | merged into v1-api-in-v2 | 0 matches alone (crashes are self-revealing); shares the version gate, so it is that rule's "removed" group |
| field-validator-on-self | `@field_validator` on an instance method | PydanticUserError at class creation | usage errors (`validator-instance-method`) | — | — | signature | drop | fails at import, every time; 0 matches |
| validate-assignment-assumed | assigning raw values to model attributes expecting validation | `validate_assignment` is off by default | models docs | — | — | — | drop | no static signal tells an assumption from a deliberate assignment |
| strict-vs-lax coercion | numbers passed to `str` fields, `"false"` to bool... | v2 no longer coerces int → str | conversion table | — | pydantic mypy plugin `init_typed` | call kwargs vs field types | drop | the mypy plugin covers typed calls; JSON input cannot be seen statically |
| custom-init-skipped | logic in `__init__` skipped by `model_validate` | — | models docs | — | — | — | drop | false premise in current v2: a custom `__init__` is called by every validation method |
| mutable-default | `items: list = []` | — | fields docs | — | RUF012 exempts models | — | drop | Pydantic deep-copies defaults: not a bug |
| validator-mutates-shared-state | validators writing module/class state | runs on failed validations too | validators docs (union note) | — | — | — | drop | no corpus shape; too vague to match |
| secret-in-request-model | `password: str` on a request model | `ValidationError` echoes input | — | — | — | — | drop | 172 matches are login/registration bodies: input, not leaks; response leaks belong to the fastapi pack |

Pack boundaries: FastAPI response models leaking fields, PATCH bodies dumped without `exclude_unset`, and
`Model(**schema.model_dump())` mass assignment belong to the fastapi and sqlalchemy packs (their users expect them
there); SQLAlchemy `mapped_column(default=uuid4())` belongs to the sqlalchemy pack.

## 4. Build list, by value

1. `pydantic-practice-dump-mode-json`: availability (TypeError at runtime), type-aware and cross-file.
2. `pydantic-practice-validator-returns-value`: silent wrong data (a field or a whole model becomes None).
3. `pydantic-practice-default-factory`: data safety (one id or timestamp shared by every instance).
4. `pydantic-practice-before-validator-raw-input`: availability (500s instead of 422s on bad input).
5. `pydantic-practice-secret-settings-str`: security (secrets printed with the settings object), log.
6. `pydantic-practice-v1-api-in-v2`: maintainability (removed on pydantic `main`), with a crash group; the
   manifest-pinned version gate is what no linter has.

Six rather than eight: the other candidates were either noise on the corpus (optional-without-default,
copy-update, construct, alias) or fail loudly at import (the removed kwargs, merged into v1-api-in-v2).
