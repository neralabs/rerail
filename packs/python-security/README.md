# Rerail rule pack: Python security

Rerail's own Python security rules, from OWASP and the frameworks' security docs, for untrusted input that reaches SQL, shells, URLs, files or deserializers.

This pack is private while its rules are written and reviewed: it is not published to npm, and is installed from
a checkout with `rerail packs add packs/python-security`.

## Rules

| Rule | Mode | Catches |
|---|---|---|
| `python-security-code-injection` | Steer | Don't run request data as code: eval, pickle, unsafe YAML or template source |
| `python-security-command-injection` | Steer | Don't run shell commands or programs built from request data |
| `python-security-jwt-unverified` | Steer | Don't trust a JWT you didn't verify |
| `python-security-orm-lookup-injection` | Steer | Don't let the client choose ORM lookup names |
| `python-security-path-traversal` | Log | Don't build file paths from request data |
| `python-security-sql-injection` | Steer | Don't build SQL from request data |
| `python-security-ssrf` | Log | Don't let request data pick where the server sends HTTP requests |

## Rule details

### `python-security-code-injection`

Rerail's own rule, not a port. It reports a call that turns client request data into code the server runs, when the
value comes from a request in the same function or through the project's own helpers in other files (the pack's
`lib/security` taint: Django and DRF request attributes and uploaded files, Flask's `request`, Starlette's request,
FastAPI and Flask route parameters, followed through assignments, string building and the call graph). One rule, a
message per sink family; the message names the source and its file and line.

- **Interpreters:** builtins `eval`, `exec`, `compile`; `importlib.import_module` and `__import__` with a tainted
  name (an f-string around it counts: `import_module(f"app.exporters.{fmt}")`).
- **Deserializers:** `pickle`/`_pickle`/`cPickle`/`pickle5` `loads`/`load`/`Unpickler`, `cloudpickle` and `dill`
  `loads`/`load`, `joblib.load`, `pandas.read_pickle`, `marshal.loads`/`load`, `shelve.open`'s path,
  `jsonpickle.decode`/`loads`, `yaml.unsafe_load(_all)`, `yaml.full_load(_all)`, and `yaml.load(_all)` whose `Loader`
  (keyword or second positional) is missing or not a safe one. Safe: `SafeLoader`, `CSafeLoader`, `BaseLoader`,
  `CBaseLoader`, a class of the run deriving from one, or an expression whose text names one
  (`getattr(yaml, "CSafeLoader", yaml.SafeLoader)`). `FullLoader`, `UnsafeLoader`, `Loader` and a missing loader are
  reported (a missing loader means `FullLoader` on PyYAML 5.1 to 5.4, and a `TypeError` from 6.0).
- **Template source:** `flask`/`quart` `render_template_string`, `jinja2.Template`, `django.template.Template`,
  `mako.template.Template` (first positional, or `source=`/`template_string=`/`text=`), and `<env>.from_string(src)`
  where `<env>` is a `jinja2.Environment` (or `NativeEnvironment`, or a subclass of the run), a Django `Engine(...)`,
  `Engine.get_default()` or `engines["django"]`, a name bound to one (locally, or at the top of the run's module it is
  imported from), or a receiver named like an environment (`app.jinja_env`, `self.env`, `templates.engine`). Only the
  source argument: render context is data.

```python
def restore(request):
    state = pickle.loads(request.body)                                   # reported
    page = render_template_string("<p>" + request.GET["n"] + "</p>")     # reported
    page = render_template_string("<p>{{ n }}</p>", n=request.GET["n"])  # not reported: context, not source
    conf = yaml.load(request.body, Loader=yaml.SafeLoader)               # not reported
```

Cross-file: a view passes `request.body` to `codec.decode(blob)` in another module, which calls `pickle.loads(blob)`
(reported in the codec, naming `request.body` in the view); a FastAPI route's `template: str` body field reaches
`_env.from_string(template)` in a service module.

It leaves alone: `json.loads`, `yaml.safe_load`, safe `Loader`s, `ast.literal_eval`, `django.core.signing.loads` and
itsdangerous `loads` (signed JSON; a library call ends the taint), `eval` of a constant, pickle of data the server
wrote (cache, Redis, Celery results: no request source), `SandboxedEnvironment`/`ImmutableSandboxedEnvironment` (and
subclasses of the run, also when imported from another module), an environment receiver it cannot resolve in a file
that builds a sandbox, `MakoTemplate(filename=…)`, and test files. Sanitisers it trusts: the pack's
`sec_checked_expr` (the value, or a name it is built from, passes through a validator-named call or an `in`
allow-list test in its function: `if fmt not in ALLOWED: …` then `import_module(f"pkg.{fmt}")`), and, for
deserializers, a payload the same function also passes to an `hmac` call (`hmac.new(key, blob, …)`).

**Gaps:** a jinja2 sandbox stays a mitigation, not a fix (Jinja's sandbox docs; GHSA-cpwx-vrp4-4pq7 is a sandbox
escape), yet sandboxed environments are not reported; values stored on objects or in the database and read back
later are not followed (the changedetection.io and fides advisories were stored templates); an hmac check after the
load is still trusted (the pgAdmin advisory's order); `torch.load`, `numpy.load(allow_pickle=True)`, ruamel.yaml's
unsafe `typ` and Flask-Session's pickle backends are not matched; Django URL keyword arguments are not sources, nor
are a DRF serializer's `validate(data)` or a form's own `self.cleaned_data` inside `clean()`.

**Overlap:** Ruff S301/S506/S307 and Bandit B301/B506/B307 flag every pickle, unsafe YAML and `eval` call whatever the
source, and miss `cloudpickle`; nothing in Ruff or Bandit covers template injection (S701/B701 are about
autoescape). This rule adds the request source (so it stays quiet on the server's own pickles) and the template
sinks.

**Sources:** [pickle docs](https://docs.python.org/3/library/pickle.html) ("Never unpickle data that could have come
from an untrusted source"); [PyYAML docs](https://pyyaml.org/wiki/PyYAMLDocumentation) ("yaml.load is as powerful as
pickle.load"); [jsonpickle docs](https://jsonpickle.github.io/);
[OWASP Deserialization Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Deserialization_Cheat_Sheet.html);
[PortSwigger SSTI](https://portswigger.net/web-security/server-side-template-injection);
[Django templates](https://docs.djangoproject.com/en/5.1/topics/templates/) ("isn't safe against untrusted template
authors"); [Flask API](https://flask.palletsprojects.com/en/stable/api/) (`render_template_string`'s source is
template code); [Jinja sandbox](https://jinja.palletsprojects.com/en/stable/sandbox/). Advisories:
[fugue GHSA-xv5p-fjw5-vrj6](https://github.com/advisories/GHSA-xv5p-fjw5-vrj6) (`cloudpickle.loads` of a request
body), [docling-core GHSA-vqxf-v2gg-x3hc](https://github.com/advisories/GHSA-vqxf-v2gg-x3hc) (`FullLoader`),
[litellm GHSA-6wvf-77m9-58rm](https://github.com/advisories/GHSA-6wvf-77m9-58rm) (a request's Jinja template),
[changedetection.io GHSA-4r7v-whpg-8rx3](https://github.com/advisories/GHSA-4r7v-whpg-8rx3) (`from_string`),
[fides GHSA-c34r-238x-f7qx](https://github.com/advisories/GHSA-c34r-238x-f7qx).

**Corpus:** 0 matches in 14 repositories, so there is no precision figure; the rule ships on its cases and the
advisories above. Django/DRF: sissbruecker/linkding `27b7303baf41`, babybuddy/babybuddy `1638cfb02e26`,
paperless-ngx/paperless-ngx `faa6f09f289b`, healthchecks/healthchecks `c1039ad23395`, wger-project/wger
`f903c66d3c0c`, pretix/pretix `40ea9097d0b2`, netbox-community/netbox `69978099811c`. FastAPI:
fastapi/full-stack-fastapi-template `77e550a73bb4`, fastapi-users/fastapi-users `e9c57ff7ff2f`, open-webui/open-webui
`0666e59d26a4`, mealie-recipes/mealie `8dbb92808d5a`, Netflix/dispatch `dd2837e82a0b`. Flask: apache/superset
`fdf61721d056`. langgenius/dify `724ac15121ee` did not complete: the engine's content store hit its hard limit twice.
The sinks these repositories do have stay silent: paperless-ngx's `pickle.loads` reads its own classifier file, and
its `signed_pickle_loads` checks an HMAC first; its `_template_environment.from_string` is a `SandboxedEnvironment`
subclass imported from another module; Superset's `env.from_string(sql)` runs in a file that builds its
`SupersetSandboxedEnvironment`; pretix's `Engine().from_string(fh.read())` reads a server file. One silent sink is a
gap: netbox's `validate_script_content` `exec`s an uploaded script (`file.read()` in a DRF serializer's
`validate(data)`, `self.cleaned_data["upload_file"].read()` in a form's `clean()`); this is netbox's permission-gated
custom-script upload, by design, and the library does not treat `validate()`'s `data` or a form's own `cleaned_data`
as request sources.

### `python-security-command-injection`

Rerail's own rule, not a port. It reports a call that runs an OS command when client request data reaches the
command, in the same function or through the project's own functions across files. The sources are the pack's: Django
and DRF `request.GET`, `.POST`, `.data`, `.FILES`, `.headers`, `.META`, …; Flask `request.args`, `.form`, …; Starlette
`request.query_params`, `.json()`, …; FastAPI and Flask route parameters. Two messages:

- **Shell command.** The command reaches a shell:
  - the first argument (or `args=`) of `subprocess.run`, `call`, `check_call`, `check_output` or `Popen` when the shell
    is on. That means `shell=True`; a `shell=` name that is True (a local, a parameter defaulting to True, or a
    parameter some call of the run passes True, as motionEye's `call_subprocess(cmd, shell=True)` does); or a `**kw`
    dict holding `"shell": True`. The rule finds that dict in a literal, `dict(shell=True)`, `kw["shell"] = True` (Modoboa's
    `exec_cmd`), `kw.setdefault("shell", True)` or `kw.update(shell=True)`. It also takes a function's own `**kw` when a
    call passes it `shell=True`;
  - the command of `os.system`, `os.popen`, `subprocess.getoutput` and `getstatusoutput`, `commands.getoutput` and
    `getstatusoutput`, and `asyncio.create_subprocess_shell`;
  - the item after `-c` in an argument list that starts a shell by name (`["sh", "-c", cmd]`,
    `create_subprocess_exec("bash", "-c", cmd)`).

  A list command under `shell=True` counts by its first item only.
- **Program.** The client picks the executable: the first item of a `subprocess.*` argument list without a shell. The
  rule reads that list from a literal, `[*argv]`, `prefix + args`, a name assigned a list, a parameter given one, a
  project function returning one, or `shlex.split(…)`. The other program sinks are `executable=`, the first argument
  of `os.exec*`, `os.posix_spawn*`, `os.startfile` and `asyncio.create_subprocess_exec`, the second of `os.spawn*`, and
  the argv of `pty.spawn` (pyLoad's client-set executable).

A project wrapper counts by itself. In `net.ping(request.POST["host"])` running
`subprocess.run(f"ping -c1 {host}", shell=True)` in another module, the rule reports the `subprocess.run` line, and
the message names `request.POST` and its file and line.

It leaves alone:
- list-form calls with a fixed program and request data in a later argument (`subprocess.run(["git", "log", ref])`);
- values passed through `shlex.quote()` or `shlex.join()`, `int()` or any other library call;
- values picked by a dict lookup (`TOOLS[op]`, `TOOLS.get(op)`);
- a name its function checks: passed to a call named like a validator (`is_valid_host(host)`), or tested with `in` or
  `not in` (`if zone not in ALLOWED_ZONES`). A function named like a validator returning the value also counts.
  `subprocess.check_output` and `check_call` are not validators;
- request data only in `env=`, `cwd=` or `input=`;
- fully literal commands;
- test files;
- code with no request source (management commands, `sys.argv`).

Hand-written quotes (`f"v4l2-ctl -d '{device}'"`, motionEye) are not quoting and are reported.

**Gaps:**
- Argument injection into a fixed program (`["git", "fetch", ref]` with `ref = "--upload-pack=…"`, as in GitPython's
  GHSA-rpm5-65cw-6hj4) is out of scope.
- Checks are flow-insensitive: a name checked anywhere in its function is trusted everywhere in it. A blocklist test
  (`if host in BLOCKED: raise`) counts as a check.
- `shlex.quote` is trusted on every platform, but it quotes only for POSIX shells.
- These are not followed: values stored on objects or in the database (second-order, as OctoPrint's stored filename);
  calls the call graph cannot resolve; `**kw` dicts built over more than one step; `loop.subprocess_shell`.
- A wrapper's shell flag is per function, not per call: when one caller passes `shell=True`, every call into the
  wrapper is read with the shell on.

**Corpus:** 0 matches in 10 repositories at their pinned commits:

| Repository | Commit | Stack | Process calls (text search, outside tests) | Matches |
|---|---|---|--:|--:|
| fastapi/full-stack-fastapi-template | `77e550a7` | FastAPI | 0 | 0 |
| sissbruecker/linkding | `27b7303b` | Django | 2 | 0 |
| open-webui/open-webui | `0666e59d` | FastAPI | 8 | 0 |
| paperless-ngx/paperless-ngx | `faa6f09f` | Django, DRF | 2 | 0 |
| Netflix/dispatch | `dd2837e8` | FastAPI | 5 | 0 |
| mealie-recipes/mealie | `8dbb9280` | FastAPI | 5 | 0 |
| inventree/InvenTree | `939df605` | Django, DRF | 6 | 0 |
| letta-ai/letta | `11315357` | FastAPI | 13 | 0 |
| pretix/pretix | `40ea9097` | Django | 6 | 0 |
| mozilla/addons-server | `7e3374fe` | Django, DRF | 18 | 0 |

With no matches, precision cannot be measured; the rule ships on its cases and the advisories below. Every process
call in these repositories runs a fixed program, a build or developer script, or settings. The closest shape is
linkding's `bookmarks/services/monolith.py:23`, an unused helper. It hand-quotes a bookmark URL into a `shell=True`
command (motionEye's pattern), but the URL comes from the database, so it is second-order and outside this rule (see
Gaps). A Flask repository (apache/superset) was queued but not run.

**Overlap:** Ruff S602/S604/S605 and Bandit B602/B604/B605 flag every non-literal shell command, whatever its source.
They also flag `shlex.quote`d values and miss `shell` set through `**kwargs` (Modoboa). This rule reports only request
data, trusts the quoting, and follows wrappers across files. Semgrep's `subprocess-injection` rules are taint rules
too, but their only sanitiser is a dict lookup.

**Sources:**
- Python docs, [subprocess, Security Considerations](https://docs.python.org/3/library/subprocess.html#security-considerations):
  with `shell=True`, "it is the application's responsibility to ensure that all whitespace and metacharacters are
  quoted appropriately to avoid shell injection vulnerabilities."
- Python docs, [shlex.quote](https://docs.python.org/3/library/shlex.html#shlex.quote): it returns "a shell-escaped
  version of the string", and it is "only designed for Unix shells".
- [OWASP OS Command Injection Defense Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/OS_Command_Injection_Defense_Cheat_Sheet.html).
- Advisories:
  - [GHSA-g5mq-prx7-c588](https://github.com/advisories/GHSA-g5mq-prx7-c588) (motionEye: a hand-quoted device name
    under `shell=True`, through a wrapper);
  - [GHSA-wwv8-cqpr-vx3m](https://github.com/advisories/GHSA-wwv8-cqpr-vx3m) (Modoboa: `shell` set through `**kwargs`);
  - [GHSA-6hw7-j4jw-wpff](https://github.com/advisories/GHSA-6hw7-j4jw-wpff) (galaxy_ng: a git ref in a shell command);
  - [GHSA-79h8-gxhq-q3jg](https://github.com/advisories/GHSA-79h8-gxhq-q3jg) (lollms, FastAPI: an environment name
    under `shell=True`);
  - [GHSA-9fpx-fr6j-2vh6](https://github.com/advisories/GHSA-9fpx-fr6j-2vh6) (GeoNode: an uploaded file's layer name);
  - [GHSA-w48f-wwwf-f5fr](https://github.com/advisories/GHSA-w48f-wwwf-f5fr) (pyLoad: a client-set executable in a list
    form call).

### `python-security-jwt-unverified`

Rerail's own rule, not a port. It reads PyJWT (`jwt.decode`, `jwt.decode_complete`) and python-jose
(`jose.jwt.decode`, `get_unverified_claims`) calls, resolved through imports, so `from jose import jwt` is python-jose
and `authlib.jose` or a module of the project named `jwt` is neither. Outside test files it reports three shapes, a
message each.

- **Unverified claims that are trusted.** Claims read without checking the signature
  (`options={"verify_signature": False}` as a dict literal or a name bound to one, `dict(verify_signature=False)`,
  PyJWT 1's `verify=False`, python-jose `get_unverified_claims`) when the code trusts them: the function is an
  authentication hook and reads them (a method named `authenticate`, as in DRF authentication classes and Django
  authentication backends; a function a FastAPI `Depends(...)`/`Security(...)` names; a name like `get_current_user`,
  `authenticate`, `user_from_token`, `verify_token`, `login_required`); they reach a lookup (a callee naming a user,
  account, member, principal, identity or customer, a Django manager, a `get`/`filter`/`where` on a session, `db`,
  query or repository); they are assigned to an identity (`request.user`, `g.current_user`, `request.state.claims`,
  `session[...]`); or they are returned from a function nothing in the project calls. A function of the project
  that returns unverified claims is followed to its callers, across files, and each call is judged the same way, so
  a `peek_claims(token)` helper is reported where a view looks a user up with its result, not where it routes or
  logs.
- **`algorithms` from the token's header.** `algorithms=` (or the third positional argument) holds the `alg` of a
  header read without verification: `jwt.get_unverified_header(token)`, `json.loads` of a base64 decode of the first
  segment, a function of the project returning one (Lemur's `fetch_token_header`), or a parameter a caller passes
  one to. The sender then chooses the algorithm (`none`, or HS256 keyed with the public key).
- **Open algorithm lists.** python-jose `jwt.decode` without `algorithms` (or with `algorithms=None`), which accepts
  any `alg` the header names; and `algorithms` that list `"none"` or take `get_default_algorithms()`
  (CVE-2022-29217).

```python
class BearerAuthentication(BaseAuthentication):
    def authenticate(self, request):
        payload = jwt.decode(token, options={"verify_signature": False})     # reported
        return User.objects.get(pk=payload["user_id"]), token

def validate_id_token(id_token, secret):
    header = jwt.get_unverified_header(id_token)
    return jwt.decode(id_token, secret, algorithms=[header["alg"]])          # reported

def is_jwt_formatted(token):
    try:
        jwt.decode(token, options={"verify_signature": False})               # not reported: a format sniff
    except jwt.PyJWTError:
        return False
    return True
```

It leaves alone: a function that also verifies the same token (a decode with the signature on, or a call named
`verify*`/`validat*` given it), so reading `iss` or `kid` first to pick the key is fine; a function that sends the
token to the issuer's userinfo or introspection endpoint; claims only logged, compared or truth-tested (`return
payload.get("owner") == "service"`), or turned into something else (an expiry time); a header `alg` tested with
`in`, `==` or `!=`, or passed to a validator-named call, before use; PyJWT 2 `jwt.decode` without `algorithms`
(it raises); `verify_exp: False` and the other claim options alone; pinned lists from settings
(`algorithms=[settings.JWT_ALGORITHM]`); test files.

**Gaps:** options built from a non-literal (`{"verify_signature": verify}`, as in taiga) or merged at runtime are not
read; claims stored on an object and read back later are not followed; a lookup through a callee whose names say
nothing about users (`repo.get(...)`) is missed outside authentication hooks; an algorithm list mixing HS* and RS*
families (the other half of the key-confusion advice) is not reported; `jose.jws.verify` and authlib are not read;
the userinfo exception trusts any call so named in the function.

**Overlap:** nothing in Ruff or Bandit. Semgrep's `unverified-jwt-decode` flags every `verify_signature: False`
(including format sniffs and read-then-verify), and `jwt-python-none-alg` a literal `"none"`; neither reads where
the claims go, the header's `alg` reaching `algorithms`, python-jose without `algorithms`, or helpers across files.

**Sources:** [PyJWT usage](https://pyjwt.readthedocs.io/en/stable/usage.html) ("Reading the Claimset without
Validation … generally ill-advised"); [PyJWT changelog](https://pyjwt.readthedocs.io/en/stable/changelog.html)
(2.0 requires `algorithms`; `verify` removed in favour of `options={"verify_signature": False}`);
[python-jose `jws.py`](https://raw.githubusercontent.com/mpdavis/python-jose/master/jose/jws.py) (with `algorithms`
omitted, any header `alg` is accepted); [Auth0, Critical vulnerabilities in JSON Web Token
libraries](https://auth0.com/blog/critical-vulnerabilities-in-json-web-token-libraries/);
[PortSwigger, algorithm confusion](https://portswigger.net/web-security/jwt/algorithm-confusion). Advisories:
[CVE-2022-29217 / GHSA-ffqj-6fqr-9h24](https://github.com/jpadilla/pyjwt/security/advisories/GHSA-ffqj-6fqr-9h24)
(PyJWT, `get_default_algorithms()`), [CVE-2024-33663](https://github.com/advisories/GHSA-6c5p-j8vq-pqhj)
(python-jose algorithm confusion), [GHSA-r9gp-7f88-9r54](https://github.com/advisories/GHSA-r9gp-7f88-9r54)
(Netflix Lemur, `algorithms=[header_data["alg"]]`), [GHSA-mcgx-2gcr-p3hp](https://github.com/advisories/GHSA-mcgx-2gcr-p3hp)
(jupyterhub-ltiauthenticator, signatures not validated).

**Corpus:** CORPUS_PLACEHOLDER

### `python-security-orm-lookup-injection`

Rerail's own rule, not a port. It reports a Django ORM call where the client chooses the lookup or field names, not
just the values, in the same function or through the project's helpers in other files. The client data is what
`lib/security` calls a source (Django/DRF `request.GET`, `.POST`, `.data`, `.query_params`, …; Flask and Starlette
request data; FastAPI/Flask route parameters that carry text). Two shapes:

- **Client-chosen keys expanded with `**`** into `filter`, `exclude`, `get`, `get_or_create`, `update_or_create`,
  `annotate`, `alias`, `aggregate`, `values`, `order_by` (any receiver: the method name and a request-keyed splat
  are enough), into `update` on a queryset-looking receiver, or into `django.db.models.Q(**…)`. A mapping is
  request-keyed when it is the request's own mapping or read off it (`request.GET.dict()`, `request.data`,
  `request.query_params.copy()`, `request.data["filters"]`), a dict literal or comprehension with a request-derived
  key (`{f"{field}__icontains": value}`, `{k: v for k, v in request.GET.items()}`), or a name filled by
  `d[key] = …` with such a key or by `d.update(request.GET)`. This is key-level taint: a dict with fixed keys and
  request values (`{"title__icontains": q}`) is not request-keyed.
- **A request string used as a field name**: a positional (or `*`-splatted) argument of `order_by`, `values`,
  `values_list`, `only` or `defer` on a queryset-looking receiver (a chain from `.objects`/`_default_manager`,
  `get_queryset()`, `filter_queryset()`, `.queryset`, a name like `qs`/`*_qs`/`*queryset*`, or a name bound to one),
  or the name of `F(name)`, the kind of `Trunc(…, kind)` and the lookup name of `Extract(…, lookup_name)`.

The message names the source and its file and line, which for a helper is the view that read the request:
`queries.apply_sort(Book.objects.all(), request.GET["sort"])` in `views.py` and `return qs.order_by(sort)` in
`queries.py` is reported at the `order_by` call, naming `views.py`'s line.

Sanitisers it trusts: a name its function tests with `in`/`not in` or passes to a validator-named call
(`if sort not in ALLOWED_SORTS: …`, `{k: v for k, v in request.GET.items() if k in ALLOWED}`,
`sort = sanitize_order_by(sort, ALLOWED)`), the same expression text checked in the function, a name resolved with
`Model._meta.get_field(name)` (it raises for anything but one of the model's own fields, so no `__` path), a dict
lookup (`SORTS[request.GET["sort"]]`, `SORTS.get(…)`), the return of a validator-named helper, and a form's
`cleaned_data` or a serializer's `validated_data` (their keys are the declared fields).

It leaves alone:

- keyword lookups with request values, `filter(title__icontains=request.GET["q"])`, and fixed-key dicts with request
  values, `filter(**{"title__icontains": q})`: Django passes values as query parameters;
- `**kwargs` of URLconf captures (`def get(self, request, **kwargs): Book.objects.get(**kwargs)`), server-built dicts;
- `Book.objects.create(**request.data)`: mass assignment, the drf pack's concern;
- `order_by(x)` on a receiver that does not look like a queryset (a table widget's own `order_by`), and a plain
  dict's `update(**request.GET)`;
- test files.

**Overlap:** none in Ruff or Bandit. Semgrep's `django/security/injection/mass-assignment` covers only
`objects.create(**request.X)` and `update(**request.X)` followed by `save()`; it has no lookup, ordering or `Q` rule.
This rule follows the names across files into the project's query helpers and knows which keys a dict has.

**Gaps:** `lib/security`'s (Django URL keyword arguments and `self.kwargs` are not sources; values stored on objects
or read back from the database, such as a saved view's filters, are not followed; calls the call graph cannot
resolve pass nothing on); a check of the whole mapping's keys in a loop (`for k in request.GET: if k not in ALLOWED:
raise`) is not seen, so that shape would be reported; a check in a caller clears a helper's parameter only for the
argument it checked; related managers (`author.book_set.order_by(x)`) and querysets under other names are not
receivers; the checks are flow-insensitive (a name tested with `in` anywhere in its function counts as checked).

CORPUS

**Sources:**

- elttam, [plORMbing your Django ORM](https://www.elttam.com/blog/plormbing-your-django-orm/) (2024):
  `Model.objects.filter(**request.data)` lets the client send `{"created_by__user__password__startswith": "a"}` and
  leak a hidden field one character at a time.
- Django REST framework, [filtering guide, OrderingFilter](https://www.django-rest-framework.org/api-guide/filtering/):
  specify the ordering fields "to prevent unexpected data leakage, such as allowing users to order against a
  password hash field".
- Django security releases: [CVE-2025-64459](https://www.djangoproject.com/weblog/2025/nov/05/security-releases/)
  (SQL injection through a dictionary expanded into `filter()`, `exclude()`, `get()`, `Q()`),
  [CVE-2025-57833](https://www.djangoproject.com/weblog/2025/sep/03/security-releases/) (`annotate()`/`alias()`
  with an expanded dictionary), [CVE-2021-35042](https://www.djangoproject.com/weblog/2021/jul/01/security-releases/)
  (unsanitised input to `order_by()`), [CVE-2024-42005](https://www.djangoproject.com/weblog/2024/aug/06/security-releases/)
  (`values()`/`values_list()` with a crafted key), CVE-2022-34265 (`Trunc(kind)`/`Extract(lookup_name)`).
- Label Studio, [GHSA-6hjj-gq77-j4qw / CVE-2023-47117](https://github.com/HumanSignal/label-studio/security/advisories/GHSA-6hjj-gq77-j4qw):
  a user-supplied filter key (`…__active_users__password`) leaked password hashes.

### `python-security-path-traversal`

Rerail's own rule, not a port. It reports a call that uses a filesystem path when client request data reaches the
path, in the same function or through the project's own functions across files, and nothing reduced the value to a
bare name or checked that the resolved path stays under its base. The sources are the pack's: Django and DRF
`request.GET`, `.POST`, `.data`, `.FILES`, …; Flask `request.args`, `.form`, `.files`, …; Starlette
`request.query_params`, `.json()`, …; FastAPI and Flask route parameters, `UploadFile` parameters included. An
upload's client filename counts: `request.FILES["f"].name`, Flask `request.files["f"].filename`,
`UploadFile.filename`.

The sinks:
- `open`, `io.open`, `codecs.open`, `os.open` and `aiofiles.open`;
- `os.remove`, `unlink`, `rmdir`, `removedirs`, `mkdir`, `makedirs`, `listdir`, `scandir` and `shutil.rmtree`;
- both paths of `os.rename`, `os.replace`, `shutil.copy`, `copy2`, `copyfile`, `copytree` and `move`;
- a pathlib path's `.read_text()`, `.read_bytes()`, `.write_text()`, `.write_bytes()`, `.unlink()`, `.rmdir()`,
  `.mkdir()` and `.touch()`, and its `.open()`, `.rename()` and `.replace()` when the receiver is known to be a path
  (`Path(…)`, `a / b`, `.resolve()`, `.parent`, a name assigned one, a parameter annotated `Path`), so an upload's own
  `.open()` and `str.replace()` stay apart;
- Flask/Werkzeug `send_file(path)`, and the **directory** of `send_from_directory(directory, path)`, never its `path`
  (changedetection.io's GHSA-9jj8-v89v-xjvw);
- Starlette/FastAPI `FileResponse(path)` and Django `FileSystemStorage(location)`. Django's `FileResponse` takes an
  open file, so the `open()` inside it is the sink;
- Werkzeug `FileStorage.save(dst)` on an upload from `request.files`, and Flask-Uploads' `UploadSet.save(upload,
  name=…, folder=…)` (Flask-Reuploaded's GHSA-65mp-fq8v-56jr).

A helper counts by itself. When a Flask route passes `<name>` to `files.read_note(name)` and that helper, in another
module, runs `open(os.path.join(NOTES, name + ".md"))`, the rule reports the `open` line, and the message names the
route parameter and its file and line. A FastAPI `UploadFile` passed to a helper that writes `dest / upload.filename`
is reported in the helper the same way.

It trusts these sanitisers:
- every library call lib/security stops at: `os.path.basename`, `werkzeug.utils.secure_filename`,
  `django.utils.text.get_valid_filename`, `werkzeug.security.safe_join` / `flask.safe_join`, a storage's
  `get_valid_name` / `generate_filename`, `int()`, `uuid4()`, …;
- `Path(x).name`, `.stem` and `.suffix`, and a name reassigned from any of these (`filename =
  secure_filename(filename)`);
- an upload's `.temporary_file_path()` and `.file` (a server temp file, not a client path);
- a pydantic model field the model validates (`@field_validator("locale")`, `@validator`, `Field(pattern=…)`), read
  off a parameter annotated with the model;
- a function of the run named like a validator (`resolve_safe`), as lib/security defines it;
- a name its function checks:
  - lib/security's `sec_checked`: passed to a validator-named call, or tested with `in` / `not in`;
  - `x.isalnum()`, `isdigit()` and the other character-class tests;
  - a containment test on a **resolved** path that reads the name: `.is_relative_to(base)` or `.relative_to(base)`,
    `os.path.commonpath([...]) == base`, or `.startswith(base + os.sep)` (or a base ending in `/`, or a name assigned
    one, as Open WebUI's `cache_root`), where the tested path comes from `.resolve()`, `os.path.realpath`, `abspath`
    or `normpath`;
  - `x.parent.samefile(base)` and `os.path.samefile`;
  - a call passing it to a function of the run that tests its parameter in one of those ways (freqtrade's
    `is_file_in_dir(fn, base)` and `is_path_in_dir`).

These are not sanitisers, and the rule still reports: `normpath` alone, `.replace("../", "")`,
`os.path.commonprefix`, `startswith(base)` without a separator (langflow's GHSA-ph9w-r52h-28p7), `base in
str(target)`, and `is_relative_to` on a path never resolved.

It leaves alone:
- `send_from_directory(CONST_DIR, user_path)` (it applies `safe_join` itself);
- `default_storage.save(name, f)` and `FileField` uploads: Django's storages refuse `..`;
- an upload's content (`Image.open(BytesIO(f.read()))`, pretix's shape);
- server-generated names (`uuid4().hex`), paths read back from the database, `int()`-cast ids, and `<int:…>` / `int`
  route parameters;
- test files.

**Gaps:**
- lib/security's gaps: Django URL kwargs and `self.kwargs` are not sources; values stored on objects or in the
  database are not followed (second-order paths); calls the call graph cannot resolve pass nothing on.
- Checks are flow-insensitive: a name checked, or reassigned to a sanitised value, anywhere in its function is
  trusted everywhere in it. A blocklist test (`if name in BLOCKED`) counts as a check, and a validator-named function
  is trusted whatever it does.
- A check in a caller does not clear a helper's parameter.
- A path parameter without a `Path` annotation is not known to be a path, so its `.open()` is not read.
- Symlinks inside the base are not considered.
- It does not know which values a route's framework already constrains. A FastAPI `{name}` or Flask `<name>` path
  parameter cannot hold `/`, so it can only climb one level (`..`); the rule reports it like any other client text.
- It cannot tell an API meant to expose the whole filesystem (an agent sandbox, an admin file browser) from a
  vulnerable one: a client-chosen absolute path is reported in both.

**Corpus:** 25 matches in 41 repositories at their pinned commits (`MANIFEST.tsv`), every one read. All 25 come from
three small FastAPI applications. The other 38 repositories have none: their file routes reduce the name or check
containment, and the rule recognises those checks.

| Repository | Commit | Stack | Matches | Real |
|---|---|---|--:|--:|
| rachelos/we-mp-rss | `8fe5bf93` | FastAPI | 12 | 11 |
| Tcotl/AgentCapture | `728ffa00` | FastAPI | 4 | 4 |
| yfyfyufeng/Phoenix-Agent | `b55404db` | FastAPI (agent sandbox) | 9 | 0 |
| freqtrade/freqtrade | `7fcb509a` | FastAPI | 0 | |
| open-webui/open-webui | `0666e59d` | FastAPI | 0 | |
| mealie-recipes/mealie | `8dbb9280` | FastAPI | 0 | |
| Netflix/dispatch | `dd2837e8` | FastAPI | 0 | |
| fastapi/full-stack-fastapi-template | `77e550a7` | FastAPI | 0 | |
| fastapi-users/fastapi-users | `e9c57ff7` | FastAPI | 0 | |
| letta-ai/letta | `11315357` | FastAPI | 0 | |
| khoj-ai/khoj | `ae229ca8` | FastAPI, Django | 0 | |
| Buuntu/fastapi-react | `af17f2d4` | FastAPI | 0 | |
| sissbruecker/linkding | `27b7303b` | Django | 0 | |
| healthchecks/healthchecks | `c1039ad2` | Django | 0 | |
| paperless-ngx/paperless-ngx | `faa6f09f` | Django, DRF | 0 | |
| HumanSignal/label-studio | `2f42caec` | Django, DRF | 0 | |
| inventree/InvenTree | `939df605` | Django, DRF | 0 | |
| makeplane/plane | `723bb178` | Django, DRF | 0 | |
| wger-project/wger | `f903c66d` | Django, DRF | 0 | |
| babybuddy/babybuddy | `1638cfb0` | Django | 0 | |
| TandoorRecipes/recipes | `22b5841a` | Django, DRF | 0 | |
| lookit/lookit-api | `78562b50` | Django | 0 | |
| ecds/readux | `fcf0d36d` | Django | 0 | |
| UB-Mannheim/escriptorium | `7e06bd31` | Django | 0 | |
| andreynovikov/django-shop | `2b8dc600` | Django | 0 | |
| jupyterhub/jupyterhub | `3ca37442` | Tornado | 0 | |
| flaskbb/flaskbb | `63afaa41` | Flask | 0 | |
| Netflix/lemur | `3a3c2c20` | Flask | 0 | |
| zenodo/zenodo-rdm | `b5bf82ec` | Flask (Invenio) | 0 | |
| pretix/pretix | `40ea9097` | Django, DRF | 0 | |
| 11 small FastAPI/Flask apps (10xGraph, remnawave-admin, carefree-drawboard, cornserve, MemoLite, speaches, cyberbro, agentic, Glimpse, themecp-backend-v2, notify-server) | see `MANIFEST.tsv` | FastAPI, Flask | 0 | |

mozilla/addons-server was queued behind other packs' runs for over 30 minutes, then cancelled, so it was not run.
Its probe lead was read by hand (below).

Precision is 15 of 25 (60%) when a weak check counts as real, and 7 of 25 (28%) when only a path exploitable as
written counts. So the rule starts in `log` mode.
- **Exploitable as written (7).** we-mp-rss `apis/tools.py:181` (`os.remove` of `./data/docs/{mp_id}/{zip_filename}`
  from the JSON body). The same `mp_id` reaches `core/exporter.py:223`, `250` and `291`,
  `tools/mdtools/html2doc.py:206`, and `tools/mdtools/pdf_extractor.py:104` and `206`, through four modules.
- **Weak check or constrained value (8):**
  - we-mp-rss `apis/tools.py:793` and `807`, and `core/rss.py:46`: `startswith(base)` without a separator, on a path
    parameter that cannot hold `/`;
  - AgentCapture `app/services/deployed_server.py:190` and `215`: `str(base) in str(target)`, a substring test that
    a sibling directory passes;
  - we-mp-rss `apis/user.py:566` and AgentCapture `app/services/c2_artifact_builder.py:172` and `379`: an upload's
    filename, or a body field, glued after a fixed prefix in the same path segment. That escapes on Windows, or where
    the prefix exists as a directory.
- **Not real (10):**
  - Phoenix-Agent's `sandbox/app/services/file.py`: 9 matches in an agent sandbox whose file API takes absolute paths
    by design;
  - we-mp-rss `apis/tools.py:752`: `output_format` reaches the file name only after PIL has accepted it as an image
    format.

Dropping client paths used whole, with no base joined, would remove the sandbox's 9 matches. It would also remove
the classic `send_file(request.args["path"])` shape, so the rule keeps them.

Matches fixed during the corpus pass, and now in the cases:
- freqtrade's 4 routes check through the project's `is_file_in_dir` helper (`file.parent.samefile(directory)`);
- Open WebUI's cache route tests `startswith(cache_root)`, where `cache_root = abspath(CACHE_DIR) + os.sep`;
- mealie's registration `locale` is checked by a pydantic `@field_validator`.

The probe leads came out as follows:
- freqtrade `bt_fileutils.py` and `misc.py`, and `web_ui.py:49`: checked, silent;
- mealie `media_user.py:22`: resolved and `is_relative_to`, silent;
- Open WebUI `main.py:3214`: separator check, silent;
- addons-server `files/utils.py:86-91`: opens through Django storage with the upload's server path, not a client
  path, and is not a sink.

**Overlap:** none in Ruff or Bandit (the pack's research found no path-traversal check in either).
Semgrep's `django/security/injection/path-traversal/*`, `request-data-fileresponse` and
`flask/security/injection/path-traversal-open` are search-mode `request.$W.get(...)` patterns inside one function.
They do not cover FastAPI parameters, `UploadFile.filename`, `send_file`, Starlette `FileResponse`,
`shutil.rmtree`/`os.remove`, or the directory argument of `send_from_directory`. This rule follows the value across
the project's functions and files, and knows the containment checks that clear it.

**Sources:**
- Flask docs, [`send_file`](https://flask.palletsprojects.com/en/stable/api/#flask.send_file): "Never pass file paths
  provided by a user … Use `send_from_directory()` to safely serve user-requested paths".
- Python docs, [`os.path.join`](https://docs.python.org/3/library/os.path.html#os.path.join): "If a segment is an
  absolute path … all previous segments are ignored". And [pathlib
  `is_relative_to`](https://docs.python.org/3/library/pathlib.html): string-based, it "neither accesses the
  filesystem nor treats '..' segments specially".
- [OWASP Path Traversal Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Path_Traversal_Prevention_Cheat_Sheet.html):
  the `realpath` + `commonpath` recipe; character filters fail on decoded input.
- Werkzeug [`secure_filename` and `safe_join`](https://werkzeug.palletsprojects.com/en/stable/utils/), Django
  [file storage](https://docs.djangoproject.com/en/5.1/ref/files/storage/) and FastAPI
  [`UploadFile.filename`](https://fastapi.tiangolo.com/reference/uploadfile/) ("the original file name").
- Advisories:
  - [GHSA-9jj8-v89v-xjvw](https://github.com/advisories/GHSA-9jj8-v89v-xjvw) (changedetection.io, Flask: a route
    parameter in the directory of `send_from_directory`);
  - [GHSA-ph9w-r52h-28p7](https://github.com/advisories/GHSA-ph9w-r52h-28p7) (langflow, FastAPI: path parameters
    joined under a base, checked with `startswith` and no separator);
  - [GHSA-9whx-c884-c68q](https://github.com/advisories/GHSA-9whx-c884-c68q) (langflow: request names in a path
    passed to `shutil.rmtree`);
  - [GHSA-wqmh-qccw-pqwf](https://github.com/advisories/GHSA-wqmh-qccw-pqwf) (mlflow, Flask: a trace ID from the body
    in a FileStore path, arbitrary write);
  - [GHSA-65mp-fq8v-56jr](https://github.com/advisories/GHSA-65mp-fq8v-56jr) (Flask-Reuploaded: client input as
    `save(name=…)`).

### `python-security-sql-injection`

Rerail's own rule, not a port. It reports a query call whose SQL text holds client request data spliced in by an
f-string, `%`, `+`, `.format()` or `.join()`, in the same function or through the project's helpers in other files.
The client data is what `lib/security` calls a source: Django/DRF `request.GET`, `.POST`, `.data`, `.query_params`,
`.body`, `.headers`, `.META`, `.COOKIES`; Flask `request.args`, `.form`, `.json`, `.values`; Starlette
`request.query_params`, `.json()`; and FastAPI/Flask route parameters that carry text. The sink is the SQL-text
argument only, never the params argument:

- Django: `<qs/manager>.raw(sql, …)`, the string items of `.extra(select=…, where=[…], tables=[…], order_by=[…])`,
  `django.db.models.expressions.RawSQL(sql, …)`;
- SQLAlchemy: `text(sql)` (resolved through imports: `sqlalchemy.text`, `sqlalchemy.sql.text`, `sqlmodel.text`, so a
  `text()` of an HTML builder is not one), `exec_driver_sql(sql)`, `<session|conn|engine>.execute(<str>)`;
- DB-API: `.execute()`, `.executemany()`, `.executescript()` on a database handle: a `….cursor()` call or a name bound
  to one (`cursor = connection.cursor()`, `with connections["replica"].cursor() as c`), a name bound to a
  `….connect(…)` call (sqlite3, psycopg), or a receiver named like a handle (`cursor`, `cur`, `conn`, `connection`,
  `session`, `engine`, `db`, `database`, `db.session`, `self.session`).

The message names the source and its file and line, which for a helper is the view that read the request:
`reports.run(request.GET["q"])` in `views.py` and `cursor.execute(f"… WHERE author = '{term}'")` in `reports.py` is
reported at the execute call, naming `views.py`'s line. A string built in one helper, returned, and executed in a
third function is followed the same way.

It leaves alone:

- parameterised calls: `cursor.execute("… %s", [request.GET["q"]])`, `raw("… %s", [q])`,
  `text("… :n").bindparams(n=q)`, `session.execute(text("… :n"), {"n": q})`, `extra(where=["name = %s"], params=[q])`;
- the placeholder list `f"… IN ({','.join(['%s'] * len(ids))})"` with the values in params: only `%s` markers are
  spliced in (Ruff S608 and Bandit B608 flag it);
- values cast by `int()`, `float()` or `UUID()` before they are spliced (`f"… LIMIT {int(request.GET['n'])}"`);
- identifiers picked through a mapping (`col = SORTS[request.GET["sort"]]`), or a name the same function checks
  (`if order not in ALLOWED_COLUMNS: raise`, or passes to a validator-named call: `lib/security`'s `sec_checked`);
- a client query run as written (`cursor.execute(request.POST["sql"])`, an "SQL lab" feature such as Superset's or
  Dify's): nothing is spliced, so the client is meant to write the SQL;
- a built string that never reaches a query (a log line), ORM lookups (`filter(name=request.GET["q"])`), and test
  files.

**Overlap:** Ruff S608 and Bandit B608 flag every SQL-looking string built by formatting, whatever the value's source,
and the safe placeholder-list shape; S610/S611 and B610/B611 flag every non-literal `extra()` and `RawSQL`. This rule
reports only SQL built from request data, so server-built SQL (table names from settings, migrations, maintenance
commands) stays quiet, and it follows the value across files into the project's query helpers, which a single-call
linter cannot.

**Gaps:** `lib/security`'s (Django URL keyword arguments and `self.kwargs` are not sources; values stored on objects
or read back from the database are not followed; calls the call graph cannot resolve pass nothing on); a check done
in the caller does not clear a helper's parameter; SQL built by appending to a list (`parts.append(…)`) and joined
later; `",".join(map(str, ids))`; pandas `read_sql` and `Func(template=…)`; a client query wrapped in fixed SQL
(`f"SELECT * FROM ({sql}) LIMIT 10"`) is reported although running the client's SQL is the feature.

**Corpus:** no match in 21 repositories, so precision has no denominator; the rule ships on its cases and sources,
which is what a mature codebase should give for this check (langgenius/dify `724ac15121ee` did not complete: the
engine's content store hit its hard limit, twice). Django/DRF: sissbruecker/linkding
`27b7303baf41`, babybuddy/babybuddy `1638cfb02e26`, healthchecks/healthchecks `c1039ad23395`,
paperless-ngx/paperless-ngx `faa6f09f289b`, HumanSignal/label-studio `2f42caecbf6c`, inventree/InvenTree
`939df605f43d`, makeplane/plane `723bb178f97b`, wger-project/wger `f903c66d3c0c`, TandoorRecipes/recipes
`22b5841a7828`, taigaio/taiga-back `8ab6a4e3b5f6`, pretix/pretix `40ea9097d0b2`, netbox-community/netbox
`69978099811c`, mozilla/addons-server `7e3374fe5383`; FastAPI/SQLAlchemy: fastapi/full-stack-fastapi-template
`77e550a73bb4`, fastapi-users/fastapi-users `e9c57ff7ff2f`, jupyterhub/jupyterhub `3ca37442a18e`,
open-webui/open-webui `0666e59d26a4`, mealie-recipes/mealie `8dbb92808d5a`, Netflix/dispatch `dd2837e82a0b`,
freqtrade/freqtrade `7fcb509a434c`; Flask: apache/superset `fdf61721d056` (its SQL Lab runs the client's query as
written, and stays quiet). The sinks do match real code: a probe of the same sinks without the request-source
condition found 9 query calls with built SQL text in paperless-ngx (`src/paperless_ai/vector_store.py`, e.g.
`conn.execute(f"PRAGMA page_size={DB_PAGE_SIZE}")` and `"DROP TABLE IF EXISTS " + DEFAULT_TABLE_NAME`, and its
`m0001_v1_to_v2.py` migration), all built from constants: the S608/B608 shape this rule leaves alone. healthchecks
has no query call with built SQL text at all.

**Sources:**

- Django, [Performing raw SQL queries](https://docs.djangoproject.com/en/5.1/topics/db/sql/): "Do not use string
  formatting on raw queries or quote placeholders in your SQL strings"; pass values in `params`.
- Django, [QuerySet API, `extra()`](https://docs.djangoproject.com/en/5.1/ref/models/querysets/) and
  [query expressions, `RawSQL`](https://docs.djangoproject.com/en/5.1/ref/models/expressions/): escape user values
  with `params`.
- SQLAlchemy, [working with transactions and the DBAPI](https://docs.sqlalchemy.org/en/20/tutorial/dbapi_transactions.html):
  "a Python literal value … should never be stringified into SQL string directly; a parameter should always be
  used"; [`text()`](https://docs.sqlalchemy.org/en/20/core/sqlelement.html) and
  [`exec_driver_sql`](https://docs.sqlalchemy.org/en/20/core/connections.html).
- [OWASP SQL Injection Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/SQL_Injection_Prevention_Cheat_Sheet.html):
  parameterise values; map identifiers through an allow-list.
- Advisories: [GHSA-pwh4-6r3m-j2rf](https://github.com/advisories/GHSA-pwh4-6r3m-j2rf) (pyload-ng, request values
  joined into an f-string `IN ('{…}')`), [GHSA-r75f-5x8p-qvmc](https://github.com/advisories/GHSA-r75f-5x8p-qvmc)
  (litellm, a header value in the query text), [GHSA-h7j7-3rx6-xvcg](https://github.com/advisories/GHSA-h7j7-3rx6-xvcg)
  (CKAN), [GHSA-8ppf-x4gr-2x7g](https://github.com/advisories/GHSA-8ppf-x4gr-2x7g) (calibreweb).

### `python-security-ssrf`

Don't let request data choose the scheme or host of an outgoing HTTP request; check the host against an allow-list first.

Rerail's own rule, not a port. Client request data (lib/security's sources: Django/DRF `request.GET`/`.POST`/
`.data`/…, Flask `request.args`/`.get_json()`/…, Starlette `request.query_params`/`.json()`/…, FastAPI and Flask
route parameters, a pydantic body's fields) that decides where the server sends an HTTP request, in the same
function or through the project's own functions across files (a `fetch(url)` wrapper around `requests.get(url)`
is followed into its parameter), reported at the call that sends the request. The sinks, by their URL argument
(the first positional, or the second after the method, or `url=`):
- `requests.get/post/put/patch/delete/head/options/request`; `httpx.get/…/request/stream`; `urllib3.request`,
`aiohttp.request`; `urllib.request.urlopen`, `urlretrieve` and `Request`;
- the same methods (and `ws_connect`, `stream`, `urlopen`) on a client: `requests.Session()`,
`httpx.Client()`/`AsyncClient()`, `aiohttp.ClientSession()`, `urllib3.PoolManager()`, bound to a name (`s =
…`, `with … as s`, `async with`), held in `self.<attr>` by the class, or a parameter annotated with the class.
The value must control the scheme and host: the whole URL is client text (through `str()`, `urljoin`, unquote,
`.strip()`, `x or DEFAULT`, subscripts and attributes of it), or client text is spliced in before the host ends:
an f-string interpolation, `+` right operand, `%` or `.format()` placeholder whose preceding literal text has no
`/`, `?` or `#` after its `://`, and either sits first or follows a literal `://`.
requests.get(request.GET["url"])                                  reported
requests.get(f"https://{request.GET['host']}/status")             reported
requests.get(urljoin(API_BASE, target))                           reported: an absolute target replaces the host
requests.get(f"{settings.API}/items/{item_id}")                   not reported: the host is fixed
requests.post(TOKEN_URL, data={"code": request.GET["code"]})      not reported: an OAuth code exchange
Sanitisers it trusts (for precision; see gaps): a name its function checks (lib/security's sec_checked: passed
to a validator-named call such as `validate_url(url)` or `is_allowed_host(urlparse(url).hostname)`, or tested
with `in`/`not in`), also through a name assigned from it (`parsed = urlparse(url)`, then `parsed.hostname in
ALLOWED`); a function of the run named like a validator, whose return carries nothing; and every library call
lib/security stops at (`int()`, `urlparse()`…). Not sanitisers: `validators.url()` and pydantic `HttpUrl`, which
check the format only (changedetection.io's and langflow's advisories).
Left alone: client data only in `data=`, `json=`, `params=`, `headers=`; a fixed host with a client path or
query, a `BASE + path` concatenation whose left side is not a literal; URLs from settings or the database; test
files.
Overlap: Ruff S310 / Bandit B310 audit only `urlopen`'s scheme, from any source; nothing covers requests, httpx,
aiohttp or urllib3. Semgrep's Django/Flask `ssrf-requests` rules are single-function `requests.*` searches.
Gaps: lib/security's (Django URL kwargs and `self.kwargs` are not sources; values stored on objects or in the
database are not followed; calls the call graph cannot resolve). Checks are flow-insensitive and trusted even
though resolve-then-check helpers are bypassable by redirects and DNS rebinding (Label Studio's and Open WebUI's
advisories); any `x in …` counts, a blocklist too. A check in a caller clears the value it passes on, not a helper
that re-reads the request. `BASE + client_text` with BASE a name is left alone, though `@evil` after a host
without a trailing slash moves it. Clients reached through other attributes or factories are not recognised.
A product whose feature is fetching the URL a user gives (bookmark checks, RAG loaders) is reported: blocking
private addresses is then the fix, and the rule cannot see one that is done in a transport adapter.
