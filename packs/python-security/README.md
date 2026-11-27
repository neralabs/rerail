# Rerail rule pack: Python security

Rerail's own Python security rules, from OWASP and the frameworks' security docs, for untrusted input that reaches SQL, shells, URLs, files or deserializers.

Two of its seven rules start on, both in steer mode: `python-security-code-injection` and
`python-security-jwt-unverified`. The other five start off in log mode (`"on": false`): fewer than 90% of their judged
matches on #1120's engine were real. Each rule's details give its numbers and where to turn it on.

## Rules

Listed by worst outcome: the injection rules first, then authentication, files, outgoing requests and ORM lookups.

| Rule | Mode | Catches |
|---|---|---|
| `python-security-sql-injection` | Log, starts off | Don't build SQL from request data |
| `python-security-command-injection` | Log, starts off | Don't run shell commands or programs built from request data |
| `python-security-code-injection` | Steer | Don't run request data as code: eval, pickle, unsafe YAML or template source |
| `python-security-jwt-unverified` | Steer | Don't trust a JWT you didn't verify |
| `python-security-path-traversal` | Log, starts off | Don't build file paths from request data |
| `python-security-ssrf` | Log, starts off | Don't let request data pick where the server sends HTTP requests |
| `python-security-orm-lookup-injection` | Log, starts off | Don't let the client choose ORM lookup names |

Every rule reads the request through the pack's shared library, `rules/lib/security.rerail` (jwt-unverified uses its
own small copy): Django, DRF, Flask and Starlette request attributes, FastAPI and Flask route parameters, Django URL
kwargs captured as text, followed through assignments, string building, byte decoders and the project's own functions
across files. A value the framework already validated into a number, a date, a UUID or a fixed choice carries no
client text. A check is a call whose name starts with a checking verb (`is_`, `validate`, `check_`, …) or an
allow-list `in` test; a function that itself reaches the rule's sink, a format-only validator and a block list are
not checks.

## Rule details

### `python-security-sql-injection`

Rerail's own rule, not a port. It reports a query call whose SQL text holds client request data, in the same
function or through the project's helpers in other files. The client data is what `lib/security` calls a source:
Django/DRF `request.GET`, `.POST`, `.data`, `.query_params`, `.body`, `.headers`, `.META`, `.COOKIES`; Flask
`request.args`, `.form`, `.json`, `.values`; Starlette `request.query_params`, `.json()`; FastAPI and Flask route
parameters that carry text; and Django URL kwargs captured as text (`path("p/<str:name>/", view)`, a non-digit
`re_path()` group, `self.kwargs` in a class-based view routed so). Two messages:

- **SqlInjection:** the value is spliced into SQL text by an f-string, `%`, `+`, `.format()` or `.join()`. The sink is
  the SQL-text argument only, never the params argument:
  - Django: `<qs/manager>.raw(sql, …)`, the string items of `.extra(select=…, where=[…], tables=[…], order_by=[…])`,
    `django.db.models.expressions.RawSQL(sql, …)`;
  - SQLAlchemy: `text(sql)` and `literal_column(sql)` (resolved through imports, so a `text()` of an HTML builder is not
    one), `exec_driver_sql(sql)`, `<session|conn|engine>.execute(<str>)`;
  - DB-API and async drivers: `.execute()`, `.executemany()`, `.executescript()`, and asyncpg's and `databases`'
    `fetch`, `fetchrow`, `fetchval`, `fetch_all`, `fetch_one`, `fetch_val`, `iterate`, on a database handle: a
    `….cursor()`, `….connect(…)`, `asyncpg.connect/create_pool(…)`, `pool.acquire()` or `databases.Database(…)` call,
    a name bound to one (awaited, or `with`/`async with … as c`), a parameter annotated with a connection, cursor,
    pool or session class (sqlite3, psycopg, asyncpg, databases, SQLAlchemy), or a receiver named like a handle
    (`cursor`, `cur`, `conn`, `connection`, `session`, `engine`, `db`, `database`, `pool`). The name list stays only
    for an unannotated parameter or attribute, whose type the engine cannot decide;
  - pandas `read_sql` / `read_sql_query`.
- **SqlFragment:** a client string used whole as a fragment inside a server-built query: `text(x)` or
  `literal_column(x)` given to `order_by`, `where`, `filter`, `having`, `group_by`, a join or a column list, an
  `.extra(…)` item, `RawSQL(x)`. This is the sort-column injection (`select(User).order_by(text(sort))`).

The message names the source and its file and line, which for a helper is the view that read the request:
`reports.run(request.GET["q"])` in `views.py` and `cursor.execute(f"… WHERE author = '{term}'")` in `reports.py` is
reported at the execute call, naming `views.py`'s line.

```python
def search(request):
    q = request.GET.get("q", "")
    Product.objects.raw(f"SELECT * FROM p WHERE name LIKE '%{q}%'")         # reported
    Product.objects.raw("SELECT * FROM p WHERE name LIKE %s", [f"%{q}%"])   # not reported: q is a parameter
    session.execute(select(User).order_by(text(request.GET["sort"])))      # reported (SqlFragment)
```

It leaves alone:

- parameterised calls: `cursor.execute("… %s", [request.GET["q"]])`, `raw("… %s", [q])`,
  `text("… :n").bindparams(n=q)`, `session.execute(text("… :n"), {"n": q})`, `extra(where=["name = %s"], params=[q])`,
  asyncpg's `fetchrow("… $1", name)`;
- the placeholder list `f"… IN ({','.join(['%s'] * len(ids))})"` with the values in params;
- values the framework already reduced to a number, date, UUID or fixed choice (`lib/security`'s typed values): a
  DRF serializer's or Django form's `IntegerField`, `DecimalField`, `DateField`, `UUIDField`, `ChoiceField` or
  `TypedChoiceField` read from `validated_data`/`cleaned_data` (the class's own fields and its bases', also through
  `self.get_serializer()` and the view's `serializer_class`); a pydantic body field typed `Literal`, an `Enum`, `int`,
  `UUID` or a date, or `Field(pattern="^(a|b)$")`; a FastAPI parameter typed so, or with `Query/Path(pattern=…)` that
  admits fixed words or digits only; `int()`, `float()` and `UUID()` casts. A free-text field (`CharField`) and a
  pattern such as `^[a-z_]+$` (any identifier) are still reported;
- identifiers picked through a mapping (`col = SORTS[request.GET["sort"]]`), or a name the same function checks: an
  allow-list `in` test (`if order not in ALLOWED_COLUMNS: raise`) or a call whose name starts with a checking verb
  (`validate_name(q)`, `is_safe_column(c)`). A block list (`if q in BLOCKED: raise`), a call that merely contains a
  checking word (`run_spellcheck(q)`), and a "check" that runs the query itself (`check_exists(q)`) are no checks;
- a whole client statement run as written (`cursor.execute(request.POST["sql"])`, `session.execute(text(sql))`: an
  "SQL lab" feature such as Superset's or Dify's);
- a built string that never reaches a query (a log line), ORM lookups (`filter(name=request.GET["q"])`), and test
  files.

**Gaps:** values stored on objects or read back from the database are not followed; calls the call graph cannot
resolve pass nothing on; captures of an `include()`d URLconf's prefix are not read; a check done in the caller does
not clear a helper's parameter; SQL built by appending to a list (`parts.append(…)`) and joined later;
`",".join(map(str, ids))`; `Func(template=…)`; a client query wrapped in fixed SQL (`f"SELECT * FROM ({sql}) LIMIT
10"`) is reported although running the client's SQL is the feature; a ModelSerializer's or ModelForm's fields
generated from the model are not typed.

**Overlap:** Ruff S608 and Bandit B608 flag every SQL-looking string built by formatting, whatever the value's source,
and the safe placeholder-list shape; S610/S611 and B610/B611 flag every non-literal `extra()` and `RawSQL`. The
`python` pack ports none of B608/B611. In this lineup, `sqlalchemy-practice-text-interpolation` (sqlalchemy pack,
log) reports every `text()`/`exec_driver_sql()` built from any non-constant value, a superset of this rule's
SQLAlchemy sinks; both fire on `text(f"…{request_value}…")`. This rule adds the request source (a security signal
where the other is a hygiene rule), the Django, DB-API and async-driver sinks, the whole-fragment shape and the walk
across files. Keep both.

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

**Corpus:** re-measured after the rework, the three injection rules and ssrf run together at the commits of
`~/rerail-bench/python-arch/corpus/MANIFEST.tsv`: 0 matches on rachelos/we-mp-rss `8fe5bf93` (FastAPI),
sissbruecker/linkding `27b7303b` (Django), lookit/lookit-api `78562b50` (Django) and open-webui/open-webui
`0666e59d` (FastAPI). The review measured 0 on 23 web repositories with the rule before the rework. With no match,
precision had no denominator there: the rule was silent in mature code. The other repositories of the review were not
re-run: the machine's local engine runs were stopped during this fix.

**On #1120's engine (benchmark).** 0 of 3 new matches real (0%), all on Case211/remnawave-admin; with none earlier,
0 of 3 overall. The main false-positive shape is a conditional allow-list not read as one: `valid_order = order if
order in ('asc', 'desc') else 'desc'`, then `ORDER BY … {valid_order}` (`shared/db/violations.py:264`). The other is
a check before a project SQL builder (`web/backend/core/admin_accounts.py:415`). It starts off (Log): below 90%; turn
it on where SQL identifiers are cleared by a mapping or an `if x not in ALLOWED` check, not a conditional expression.

### `python-security-command-injection`

Rerail's own rule, not a port. It reports a call that runs an OS command when client request data reaches the
command, in the same function or through the project's own functions across files. The sources are the pack's: Django
and DRF `request.GET`, `.POST`, `.data`, `.FILES`, `.headers`, `.META`, …; Flask `request.args`, `.form`, …; Starlette
`request.query_params`, `.json()`, …; FastAPI and Flask route parameters; Django URL kwargs captured as text
(`path("convert/<str:name>/", views.convert)`). Two messages:

- **Shell command.** The command reaches a shell:
  - the first argument (or `args=`) of `subprocess.run`, `call`, `check_call`, `check_output` or `Popen` when the shell
    is on. That means `shell=True`; a `shell=` name that is True (a local, a parameter defaulting to True, or a
    parameter some call of the run passes True, as motionEye's `call_subprocess(cmd, shell=True)` does); or a `**kw`
    dict holding `"shell": True`. The rule finds that dict in a literal, `dict(shell=True)`, `kw["shell"] = True`
    (Modoboa's `exec_cmd`), `kw.setdefault("shell", True)` or `kw.update(shell=True)`. It also takes a function's own
    `**kw` when a call passes it `shell=True`;
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

```python
os.system(f"ping -c1 {request.GET['host']}")                  # reported
subprocess.Popen([request.POST["scanner"], "--scan", path])   # reported: the client picks the program
subprocess.run(["git", "log", request.GET["ref"]])            # not reported: fixed program, no shell
subprocess.run(f"ls {shlex.quote(p)}", shell=True)            # not reported: quoted
```

It leaves alone:
- list-form calls with a fixed program and request data in a later argument (`subprocess.run(["git", "log", ref])`);
- values passed through `shlex.quote()` or `shlex.join()`, `int()` or any other library call;
- values the framework already reduced to a fixed choice or a number: a serializer's or form's `ChoiceField`
  (`subprocess.run([s.validated_data["tool"], "-h"])` with `choices=["ls", "df"]`), an `IntegerField`, a pydantic
  `Literal` or `Enum` field;
- values picked by a dict lookup (`TOOLS[op]`, `TOOLS.get(op)`);
- a name its function checks against an allow-list: tested with `in` or `not in` (`if zone not in ALLOWED_ZONES:
  raise`), or passed to a call whose name starts with a checking verb (`is_valid_host(host)`, `validate_ref(ref)`);
  a function so named returning the value also counts. A function that itself runs a command is never a check, whatever
  its name (`check_disk(path)` running `df`, `run_healthcheck(host)` running `ping`), and a name that merely contains a
  checking word is none either: the review's `host = request.GET["host"]; git.run_healthcheck(host)` is reported in
  the helper;
- request data only in `env=`, `cwd=` or `input=`;
- fully literal commands; test files; code with no request source (management commands, `sys.argv`).

Hand-written quotes (`f"v4l2-ctl -d '{device}'"`, motionEye) are not quoting and are reported. A block list (`if host
in BLOCKED: raise`) is no check and is reported.

**Gaps:**
- Argument injection into a fixed program (`["git", "fetch", ref]` with `ref = "--upload-pack=…"`, as in GitPython's
  GHSA-rpm5-65cw-6hj4) is out of scope.
- Checks are flow-insensitive: a name checked anywhere in its function is trusted everywhere in it.
- `shlex.quote` is trusted on every platform, but it quotes only for POSIX shells.
- These are not followed: values stored on objects, in the session or in the database (second-order, as OctoPrint's
  stored filename); calls the call graph cannot resolve; `**kw` dicts built over more than one step;
  `loop.subprocess_shell`; captures of an `include()`d URLconf's prefix.
- A wrapper's shell flag is per function, not per call: when one caller passes `shell=True`, every call into the
  wrapper is read with the shell on.

**Overlap:** Ruff S602/S604/S605 and Bandit B602/B604/B605 flag every non-literal shell command, whatever its source;
the `python` pack ports none of them (it ports B606 and B609). They also flag `shlex.quote`d values and miss `shell`
set through `**kwargs` (Modoboa). This rule reports only request data, trusts the quoting, and follows wrappers across
files. Semgrep's `subprocess-injection` rules are taint rules too, but their only sanitiser is a dict lookup.

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

**Corpus:** re-measured after the rework, the three injection rules and ssrf run together at the commits of
`~/rerail-bench/python-arch/corpus/MANIFEST.tsv`: 0 matches on rachelos/we-mp-rss `8fe5bf93` (FastAPI),
sissbruecker/linkding `27b7303b` (Django), lookit/lookit-api `78562b50` (Django) and open-webui/open-webui
`0666e59d` (FastAPI). The review measured 0 on 23 web repositories with the rule before the rework. With no match,
precision had no denominator there: the rule was silent in mature code. The other repositories of the review were not
re-run: the machine's local engine runs were stopped during this fix.

**On #1120's engine (benchmark).** 2 of 10 new matches real (20%), on osworld and Phoenix-Agent; with none earlier,
2 of 10 overall. The 8 false ones are endpoints whose purpose is running the client's command as written, where an
argument list or `shlex.quote` would break the feature: osworld's in-VM `/execute`
(`desktop_env/server/main.py:98`) and Phoenix-Agent's `/shell/exec` (`sandbox/app/services/shell.py:44`). It starts
off (Log): below 90%; turn it on where no route exists to run a client's command (no sandbox, VM-control or
remote-shell API).

### `python-security-code-injection`

Rerail's own rule, not a port. It reports a call that turns client request data into code the server runs, when the
value comes from a request in the same function or through the project's own helpers in other files (the pack's
`lib/security` taint: Django and DRF request attributes and uploaded files, Flask's `request`, Starlette's request,
FastAPI and Flask route parameters, Django URL kwargs captured as text, followed through assignments, string building,
byte decoders and the call graph). One rule, a message per sink family; the message names the source and its file and
line.

- **Interpreters:** builtins `eval`, `exec`, `compile`; `importlib.import_module` and `__import__` with a tainted
  name (an f-string around it counts: `import_module(f"app.exporters.{fmt}")`).
- **Deserializers:** `pickle`/`_pickle`/`cPickle`/`pickle5` `loads`/`load`/`Unpickler`, `cloudpickle` and `dill`
  `loads`/`load`, `joblib.load`, `pandas.read_pickle`, `marshal.loads`/`load`, `shelve.open`'s path,
  `jsonpickle.decode`/`loads`, `numpy.load(…, allow_pickle=True)`, `torch.load(…, weights_only=False)`,
  `yaml.unsafe_load(_all)`, `yaml.full_load(_all)`, and `yaml.load(_all)` whose `Loader` (keyword or second
  positional) is missing or not a safe one. Safe: `SafeLoader`, `CSafeLoader`, `BaseLoader`, `CBaseLoader`, a class of
  the run deriving from one, or an expression whose text names one (`getattr(yaml, "CSafeLoader", yaml.SafeLoader)`).
  `FullLoader`, `UnsafeLoader`, `Loader` and a missing loader are reported (a missing loader means `FullLoader` on
  PyYAML 5.1 to 5.4, and a `TypeError` from 6.0). The payload is followed through byte decoders, so
  `pickle.loads(zlib.decompress(request.body))`, `pickle.loads(bytes.fromhex(request.COOKIES["state"]))` and
  `pickle.load(io.BytesIO(request.body))` are reported (`base64`, `binascii`, `codecs.decode`, `zlib`/`gzip`/`bz2`/
  `lzma` decompression, `io.BytesIO`/`StringIO`).
- **Template source:** `flask`/`quart` `render_template_string`, `jinja2.Template`, `mako.template.Template` (first
  positional, or `source=`/`template_string=`/`text=`), and `<env>.from_string(src)` where `<env>` is a
  `jinja2.Environment` (or `NativeEnvironment`, or a subclass of the run), a name bound to one (locally, or at the top
  of the run's module it is imported from), or a receiver named like an environment (`app.jinja_env`, `self.env`,
  `templates.engine`). The message says a client can run code: Jinja2 and Mako templates can reach Python objects.
- **Django template source:** `django.template.Template(src)` and `from_string(src)` on a Django `Engine(...)`,
  `Engine.get_default()` or `engines["django"]` get their own message: the Django template language cannot call
  arbitrary code, but a client template reads whatever its context reaches (settings, related objects, the user).

Only the source argument of a template counts: render context is data.

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
itsdangerous `loads` (signed JSON; a library call ends the taint), a length or checksum of the payload, `eval` of a
constant, pickle of data the server wrote (cache, Redis, Celery results: no request source),
`SandboxedEnvironment`/`ImmutableSandboxedEnvironment` (and subclasses of the run, also when imported from another
module), an environment receiver it cannot resolve in a file that builds a sandbox, `MakoTemplate(filename=…)`,
`torch.load` without `weights_only=False` (its default is `True` from torch 2.6; the `python` pack's bandit-b614 flags
every call), `numpy.load` without `allow_pickle=True`, and test files. Sanitisers it trusts: the pack's
`sec_checked_expr` (the value, or a name it is built from, passes an allow-list `in` test or a call whose name starts
with a checking verb in its function: `if fmt not in ALLOWED: …` then `import_module(f"pkg.{fmt}")`; a function that
itself evaluates, such as `validate_expr(e)` running `eval`, is no check, and `safe_eval(e)` is no check by name), and,
for deserializers, a payload the same function also passes to an `hmac` call (`hmac.new(key, blob, …)`).

**Gaps:** a jinja2 sandbox stays a mitigation, not a fix (Jinja's sandbox docs; GHSA-cpwx-vrp4-4pq7 is a sandbox
escape), yet sandboxed environments are not reported; values stored on objects or in the database and read back later
are not followed (the changedetection.io and fides advisories were stored templates, and `request.session` is not a
source); an hmac check after the load is still trusted (the pgAdmin advisory's order); ruamel.yaml's unsafe `typ` and
Flask-Session's pickle backends are not matched; a DRF serializer's `validate(data)` and a form's own
`self.cleaned_data` inside `clean()` are not sources.

**Overlap:** Ruff S301/S506/S307 and Bandit B301/B506/B307 flag every pickle, unsafe YAML and `eval` call whatever the
source, and miss `cloudpickle`; the `python` pack ports B301, B302, B307, B102, B614 and B702, which do the same.
Pylint W0123/W0122 flag every `eval`/`exec`. Nothing in Ruff, Bandit or pylint covers template injection (S701/B701
are about autoescape). This rule adds the request source (so it stays quiet on the server's own pickles) and the
template sinks.

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

**Corpus:** re-measured after the rework, the three injection rules and ssrf run together at the commits of
`~/rerail-bench/python-arch/corpus/MANIFEST.tsv`: 0 matches on rachelos/we-mp-rss `8fe5bf93` (FastAPI),
sissbruecker/linkding `27b7303b` (Django), lookit/lookit-api `78562b50` (Django) and open-webui/open-webui
`0666e59d` (FastAPI). The review measured 0 on 23 web repositories with the rule before the rework. With no match,
precision has no denominator: the rule ships on its cases and sources, and is silent in mature code, which is what a
team wants from an insurance rule. The other repositories of the review were not re-run: the machine's local engine
runs were stopped during this fix.

**On #1120's engine (benchmark).** No matches on the benchmark's repositories either, so no precision yet. It stays on
(Steer).

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
  query or repository, `repo.get(claims["sub"])` included); they are assigned to an identity (`request.user`,
  `g.current_user`, `request.state.claims`, `session[...]`); or they are returned from a function nothing in the
  project calls. A function of the project that returns unverified claims is followed to its callers, across files,
  and each call is judged the same way, so a `peek_claims(token)` helper is reported where a view looks a user up with
  its result, not where it routes or logs.
- **`algorithms` from the token's header.** `algorithms=` (or the third positional argument) holds the `alg` of a
  header read without verification: `jwt.get_unverified_header(token)`, `json.loads` of a base64 decode of the first
  segment, a function of the project returning one (Lemur's `fetch_token_header`), or a parameter a caller passes
  one to. The sender then chooses the algorithm (`none`, or HS256 keyed with the public key).
- **Open algorithm lists.** A decode that pins no algorithm where the library then trusts the token's `alg`:
  python-jose `jwt.decode` without `algorithms` (or `algorithms=None`), and PyJWT 1 `jwt.decode(token, key)` when the
  project is on PyJWT 1 (it calls `.decode()` on what `jwt.encode` returns, bytes only in PyJWT 1, or `setup.py` pins
  PyJWT below 2); `algorithms` that list `"none"` or take `get_default_algorithms()` (CVE-2022-29217); and a list
  mixing HMAC (`HS*`) with public-key algorithms (`RS*`, `ES*`, `PS*`, `EdDSA`), where the token's `alg` picks how the
  key is used. The message asks for a pin and promises no exploit: python-jose's own key-type checks refuse some
  swaps.

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

It leaves alone a function that also verifies the token, so reading `iss` or `kid` first to pick the key is fine.
Verifying means: any decode with the signature on in the same function, whatever expression holds the token
(`token = creds.credentials` peeked, then `jwt.decode(creds.credentials, key, algorithms=["RS256"])`);
`super().authenticate(request)` or `Base.authenticate(self, request)` with `Base` a base of the class (DRF's
`JWTAuthentication`); simplejwt's `get_validated_token`; the issuer's userinfo or introspection endpoint; or, given the
same token (the decode's argument, or a name bound to it), a call named `verify*`/`validat*` or a function of the run
that verifies, directly or through its callees. It also leaves alone claims only logged, compared or truth-tested
(`return payload.get("owner") == "service"`), or turned into something else (an expiry time); a header `alg` tested
with `in`, `==` or `!=`, or passed to a check call, before use; PyJWT 2 `jwt.decode` without `algorithms` (it raises);
`verify_exp: False` and the other claim options alone; pinned lists from settings (`algorithms=[settings.JWT_ALGORITHM]`);
test files.

**Cost:** the rule includes only `python/ast` and `python/calls` and its own copies of the few helpers it needs, and
every relation starts from a decode, a header read or a value already found. The review's runs filled the engine's
content store on open-webui, freqtrade, pretix, netbox and addons-server: one clause paired every two nodes of a
function (167 million rows on Netflix/dispatch). It now finishes on open-webui in 46 s of CPU (798 s before) and on
freqtrade in 49 s.

**Gaps:** options built from a non-literal (`{"verify_signature": verify}`, as in taiga) or merged at runtime are not
read; claims stored on an object and read back later are not followed; PyJWT pins in `requirements*.txt`,
`pyproject.toml` and `setup.cfg` are not read (a Python rule gets no text of those files), only the code signal and
`setup.py`; a function that verifies one token and trusts the unverified claims of another (an access token verified,
an ID token peeked for its email) is silent; a check of a header `alg` inside a function the `alg` is passed to is not
read; `jose.jws.verify` and authlib are not read; the userinfo exception trusts any call so named in the function.

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

**Corpus:** re-measured after the rework, the rule alone:

| Repository | Commit | Stack | Matches | Real |
|---|---|---|--:|--:|
| Netflix/lemur | `3a3c2c20` | Flask | 1 | 1 |
| Netflix/dispatch | `dd2837e8` | FastAPI | 3 | 3 |
| open-webui/open-webui | `0666e59d` | FastAPI | 0 | |
| freqtrade/freqtrade | `7fcb509a` | FastAPI | 0 | |
| fastapi-users/fastapi-users | `e9c57ff7` | FastAPI | 0 | |
| HumanSignal/label-studio | `2f42caec` | Django, DRF | 0 | |

4 of 4 read matches are real (100%), the review's same four. Lemur `lemur/auth/views.py:132` (HeaderAlgorithm:
`jwt.decode(id_token, secret, algorithms=[algo])` with `algo` from `fetch_token_header`) is the code of
GHSA-r9gp-7f88-9r54 at the pinned commit. Dispatch `src/dispatch/plugins/dispatch_core/plugin.py:92`, `:140`, `:142`
(OpenAlgorithms: python-jose without `algorithms`) are real as missing pins. Silent where it should be: label-studio's
`token_format.py` format sniff and open-webui's three unverified decodes (after the provider's userinfo accepted the
token; reading `exp` only; peeking `iss` to pick the provider whose JWKS then verifies the token). pretix/pretix,
netbox-community/netbox and mozilla/addons-server (over 1,000 files) were not re-run: their runs were stopped with
every local engine run on this machine.

**On #1120's engine (benchmark).** Findings unchanged: 4 of 4 real (100%), including Lemur's published advisory
(GHSA-r9gp-7f88-9r54); no new repository had matches. It stays on (Steer).

### `python-security-path-traversal`

Rerail's own rule, not a port. It reports a call that uses a filesystem path when client request data that can climb
out of a directory reaches the path, in the same function or through the project's own functions across files, and
nothing reduced the value to a bare name or checked that the resolved path stays under its base. The sources are the
pack's: Django and DRF `request.GET`, `.POST`, `.data`, `.FILES`, …; Flask `request.args`, `.form`, `.files`, …;
Starlette `request.query_params`, `.json()`, …; FastAPI and Flask route parameters, `UploadFile` parameters included;
Django URL kwargs captured as text, and `self.kwargs` in a class-based view routed so. An upload's client filename
counts: `request.FILES["f"].name`, Flask `request.files["f"].filename`, `UploadFile.filename`.

The rule carries, with every client value, what its text can do in a path:

| Shape | Text | Climbs |
|---|---|---|
| free | may hold `/`: a query parameter, a body field, an upload filename, FastAPI `{x:path}`, Flask `<path:x>`, Django `<path:x>`, a `re_path()` group, `self.kwargs` | `../..` and absolute paths |
| one segment | a route parameter bound to one path segment: FastAPI `{x}`, Flask `<x>`/`<string:x>`, Django `<str:x>`/`<slug:x>`/`<x>` | only as the whole segment `..`, one level |
| prefixed | free text after fixed text of its own segment (`f"{user}_{date}_{upload.filename}"`) | only if the prefixed directory exists, or after a lexical normalisation, or where the sink creates directories |

The text around a value comes from the library's `sec_text_before`/`sec_text_after`. A one-segment value with fixed
text before or after it in the same segment (`f"{name}.json"`, `f"v_{name}"`, `name + ".csv"`, `"report-%s" % name`)
cannot be `..` and is not reported; `f"{name}."` still is. A one-segment value becomes free text through any step that
may add `/` (`unquote()`, a decoder, `.replace()`, any library call but a join or a normalisation). A prefixed value is
not reported at an open, send or remove, and is reported again after `normpath`/`abspath`/`realpath`/`.resolve()` (which
skip the kernel's check of the missing directory), or at `os.makedirs` / `.mkdir(parents=True)` (which create it) and
Django's `FileSystemStorage`. An extension cut off a client name (`rsplit(".", 1)[-1]`) can still hold `/`; it is the
server name before it in the same segment that makes `f"{uuid4().hex}.{ext}"` safe.

Two messages:

- **PathTraversal**, at a call that reads, writes, lists or deletes the path:
  - `open`, `io.open`, `codecs.open`, `os.open`, `aiofiles.open`;
  - `os.remove`, `unlink`, `rmdir`, `removedirs`, `mkdir`, `makedirs`, `listdir`, `scandir`, `os.walk`,
    `shutil.rmtree`, `glob.glob`/`iglob`, `shutil.make_archive` (archive name, `root_dir`, `base_dir`);
  - both paths of `os.rename`, `os.replace`, `shutil.copy`, `copy2`, `copyfile`, `copytree` and `move`;
  - a pathlib path's `.read_text()`, `.read_bytes()`, `.write_text()`, `.write_bytes()`, `.unlink()`, `.rmdir()`,
    `.mkdir()`, `.touch()`, `.iterdir()`, `.glob()`, `.rglob()`, and its `.open()`, `.rename()` and `.replace()` when the
    receiver is known to be a path (`Path(…)`, `a / b`, `.resolve()`, `.parent`, a name assigned one, a parameter
    annotated `Path`);
  - readers that take a path: `PIL.Image.open`, pandas `read_csv`/`read_json`/`read_excel`/`read_parquet`/… (not
    `read_pickle`, which is code-injection's), `zipfile.ZipFile`, `tarfile.open`/`TarFile`;
  - Flask/Werkzeug `send_file(path)` and the **directory** of `send_from_directory(directory, path)`, never its `path`
    (changedetection.io's GHSA-9jj8-v89v-xjvw); Starlette/FastAPI `FileResponse(path)`; Django
    `FileSystemStorage(location)` (Django's `FileResponse` takes an open file, so the `open()` inside it is the sink);
  - Werkzeug `FileStorage.save(dst)` on an upload from `request.files`, and Flask-Uploads' `UploadSet.save(upload,
    name=…, folder=…)` (Flask-Reuploaded's GHSA-65mp-fq8v-56jr).
- **PathProbe**, a weaker message, at a call that only tests the path: `os.path.exists`/`lexists`/`isfile`/`isdir`/
  `islink`/`getsize`/`get*time`, `os.stat`/`lstat`/`access`, a path's `.exists()`/`.is_file()`/`.is_dir()`/`.stat()`.
  It leaks existence and size, not content, and it is reported only where no full sink in the same function takes the
  same client data (`exists(p)` then `open(p)` is one finding, at the `open`).

A file-like argument is never a path: an upload object (`request.FILES["f"]`, an `UploadFile`, a `FileStorage`), its
`.file` or `.stream`, `io.BytesIO`/`StringIO`. So `Image.open(BytesIO(f.read()))` and `pd.read_csv(upload.file)` stay
silent.

A helper counts by itself. When a Flask route passes `<path:name>` to `files.read_note(name)` and that helper, in
another module, runs `open(os.path.join(NOTES, name + ".md"))`, the rule reports the `open` line, and the message names
the route parameter and its file and line.

```python
open(os.path.join(MEDIA, request.GET["name"]))                         # reported
f = request.files["file"]; f.save(os.path.join(UPLOADS, f.filename))   # reported
open(os.path.join(BASE, f"{name}.json"))   # name a FastAPI {name}     # not reported: cannot be `..`
open(os.path.join(MEDIA, os.path.basename(request.GET["name"])))       # not reported
```

It trusts these sanitisers:
- every library call the library stops at: `os.path.basename`, `werkzeug.utils.secure_filename`,
  `django.utils.text.get_valid_filename`, `werkzeug.security.safe_join` / `flask.safe_join`, a storage's
  `get_valid_name` / `generate_filename`, `int()`, `uuid4()`, `os.path.splitext(x)[1]`, …;
- `Path(x).name`, `.stem` and `.suffix`, and a name reassigned from any of these (`filename =
  secure_filename(filename)`);
- an upload's `.temporary_file_path()` and `.file` (a server temp file, not a client path);
- a pydantic model field the model validates (`@field_validator("locale")`, `@validator`, `Field(pattern=…)`), read
  off a parameter annotated with the model;
- a function of the run whose name starts with a checking verb (`is_safe_path`), unless it opens the path itself;
- a name its function checks:
  - an allow-list `in` / `not in` test, or a call whose name starts with a checking verb;
  - `x.isalnum()`, `isdigit()` and the other character-class tests;
  - a containment test on a **resolved** path that reads the name: `.is_relative_to(base)` or `.relative_to(base)`,
    `os.path.commonpath([...]) == base`, or `.startswith(base + os.sep)` (or a base ending in `/`, or a name assigned
    one, as Open WebUI's `cache_root`), where the tested path comes from `.resolve()`, `os.path.realpath`, `abspath`
    or `normpath`;
  - `x.parent.samefile(base)` and `os.path.samefile`;
  - a call passing it to a function of the run that tests its parameter in one of those ways (freqtrade's
    `is_file_in_dir(fn, base)` and `is_path_in_dir`);
  - for a one-segment value only, whose only climb is `..`: `startswith(base)` on its resolved path without a
    separator, and a `".." in x` / `x == ".."` test.

These are not sanitisers, and the rule still reports: `normpath` alone, `.replace("../", "")`,
`os.path.commonprefix`, `startswith(base)` without a separator on free text (langflow's GHSA-ph9w-r52h-28p7: an
absolute path or `base-x` passes), `".." in x` on free text, `base in str(target)`, `is_relative_to` on a path never
resolved, a block list (`if name in BLOCKED: raise`), and a function merely named like a check (`resolve_safe`).

It leaves alone:
- `send_from_directory(CONST_DIR, user_path)` (it applies `safe_join` itself);
- `default_storage.save(name, f)` and `FileField` uploads: Django's storages refuse `..`;
- an upload's content (`Image.open(BytesIO(f.read()))`, pretix's shape);
- server-generated names (`uuid4().hex`), paths read back from the database, `int()`-cast ids, and `<int:…>` / `int`
  route parameters;
- test files.

**Gaps:**
- Values stored on objects or in the database are not followed (second-order paths); calls the call graph cannot
  resolve pass nothing on; captures of an `include()`d URLconf's prefix and of a FastAPI router's `prefix` are not
  read (the latter is taken for free text: it over-reports).
- Checks are flow-insensitive: a name checked, or reassigned to a sanitised value, anywhere in its function is trusted
  everywhere in it. A check in a caller does not clear a helper's parameter.
- The text after a value is read for f-strings and `+` only, so a one-segment parameter in `"{}.json".format(name)`
  or `"%s.json" % name` is still reported.
- Fixed text is judged by its characters: a server value that may be empty (`f"{prefix}{name}"`) is trusted as a name.
- `os.path.dirname()` ends the taint, so `os.makedirs(os.path.dirname(p))` before `open(p)` with a prefixed upload name
  is not read.
- A custom Flask or Django converter makes the parameter free text (over-reports).
- A path parameter without a `Path` annotation is not known to be a path, so its `.open()` is not read. Symlinks
  inside the base are not considered.
- It cannot tell an API meant to expose the whole filesystem (an agent sandbox, an admin file browser) from a
  vulnerable one: a client-chosen absolute path is reported in both.

**Overlap:** none in Ruff or Bandit (the pack's research found no path-traversal check in either; the `python` pack's
`ruff-pth*` rules are pathlib style only). Semgrep's `django/security/injection/path-traversal/*`,
`request-data-fileresponse` and `flask/security/injection/path-traversal-open` are search-mode `request.$W.get(...)`
patterns inside one function. They do not cover FastAPI parameters, `UploadFile.filename`, `send_file`, Starlette
`FileResponse`, `shutil.rmtree`/`os.remove`, or the directory argument of `send_from_directory`. This rule follows the
value across the project's functions and files, knows which route parameters can only climb one level, and knows the
containment checks that clear a value.

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

**Corpus:** re-measured after the rework, this rule alone, at the commits of `~/rerail-bench/python-arch/corpus/MANIFEST.tsv`:

| Repository | Commit | Stack | Matches | Real |
|---|---|---|--:|--:|
| rachelos/we-mp-rss | `8fe5bf93` | FastAPI | 11 | 11 |
| mealie-recipes/mealie | `8dbb9280` | FastAPI | 0 | |
| open-webui/open-webui | `0666e59d` | FastAPI | 0 | |
| paperless-ngx/paperless-ngx | `faa6f09f` | Django, DRF | 0 | |
| inventree/InvenTree | `939df605` | Django, DRF | 0 | |

11 of 11 read matches are real (100%), against the review's 7 of 12 (58%). The review's 7 real matches stay:
`apis/tools.py:181` (`os.remove` of `./data/docs/{mp_id}/{zip_filename}`, both from the JSON body), and the same
`mp_id` directory reaching `core/exporter.py:223`, `:250`, `:291`, `tools/mdtools/html2doc.py:206` and
`tools/mdtools/pdf_extractor.py:104`, `:206`. Its 5 false positives are gone: `apis/tools.py:752` (an extension after
a server name), `:793`, `:807` (a one-segment path parameter, `abspath` and `startswith(temp_dir)`),
`apis/user.py:566` (an upload filename after `{username}_{date}_`), `core/rss.py:46` (a path parameter with `.json`
appended). Four new matches are real: `apis/tools.py:182` (`zipfile.ZipFile` writing the archive at the same
client path), `:183` (`os.walk` of the client's `mp_id` directory, whose files are zipped then removed), `:401`
(`os.walk` of a query `mp_id` joined under `safe_root`, checked with `startswith` and no separator: a sibling
directory starting with `docs` passes), and `core/exporter.py:109` (PathProbe: `os.path.exists` under the body's
`mp_id` directory). All these matches come from one repository; the benchmark below measured other codebases.

**On #1120's engine (benchmark).** 11 of 11 earlier, 1 of 21 matches gained on the baseline repositories, and 19 of a
30-match sample of 89 new ones on 9 repositories: 31 of 62 real (50%). The main false-positive shape is an API whose
purpose is reaching any path: Phoenix-Agent's agent sandbox (`sandbox/app/services/file.py`) and osworld's in-VM
`/list_directory` (`third_party/osworld/desktop_env/server/main.py:1102`). Next is a helper that contains the path but
whose name doesn't start with a checking verb: Case211/remnawave-admin's `_safe_backup_path`
(`web/backend/core/backup_service.py:208`). It starts off (Log): below 90%; turn it on where no API is meant to reach
any path and path guards are named with a checking verb (`is_safe_path`).

### `python-security-ssrf`

Rerail's own rule, not a port. It reports an outgoing HTTP request whose scheme or host comes from client request
data, in the same function or through the project's own functions across files (a `fetch(url)` wrapper around
`requests.get(url)` is followed into its parameter), at the call that sends the request. The sources are the pack's:
Django/DRF `request.GET`, `.POST`, `.data`, …; Flask `request.args`, `.get_json()`, …; Starlette
`request.query_params`, `.json()`, …; FastAPI and Flask route parameters; a pydantic body's fields; Django URL kwargs
captured as text.

The sinks, by their URL argument (the first positional, the second after the method, or `url=`):

- `requests.get/post/put/patch/delete/head/options/request`; `httpx.get/…/request/stream`; `urllib3.request`,
  `aiohttp.request`; `urllib.request.urlopen`, `urlretrieve` and `Request`;
- the same methods (and `ws_connect`, `stream`, `urlopen`) on a client: `requests.Session()`,
  `httpx.Client()`/`AsyncClient()`, `aiohttp.ClientSession()`, `urllib3.PoolManager()`, bound to a name (`s = …`,
  `with … as s`, `async with`), returned by a function of the run (`get_client().get(url)`), held in `self.<attr>` by
  the class, or a parameter annotated with the class.

Only the part of the URL that decides where the request goes counts. The value must be the whole URL (through
`str()`, `urljoin`, unquote, `.strip()`, `x or DEFAULT`, subscripts and attributes of it), or be spliced in while the
host is still open: the library's `sec_text_before` gives the literal text before the splice (with the file's string
constants resolved), and the host is open when that text is empty or has `://` with no `/`, `?` or `#` after it.

```python
requests.get(request.GET["url"])                                          # reported
requests.get(f"https://{request.GET['host']}/status")                     # reported
requests.get(API + request.GET["path"])                                   # reported: API = "https://api.example.com"
                                                                          # has no `/`, so `@evil.com` moves the host
requests.get(urljoin(API_BASE, target))                                   # reported: an absolute target replaces it
requests.get(f"https://{settings.API_HOST}/items/{request.GET['id']}")    # not reported: the host is fixed
requests.post(TOKEN_URL, data={"code": request.GET["code"]})              # not reported: an OAuth code exchange
```

It leaves alone:

- a client with a connection-time guard: an httpx `transport=` or aiohttp `connector=` built from a class of the run,
  or a requests session that mounts an adapter class of the run (`session.mount("https://", GuardedHTTPAdapter())`,
  in the same function or in the class's `__init__`). That is the fix for a product whose feature is fetching the URL
  a user gives (link previews, imports, webhooks), which cannot allow-list. A library transport or adapter
  (`httpx.HTTPTransport(retries=3)`, `HTTPAdapter(max_retries=3)`) guards nothing and is still reported;
- data sent by staff (`lib/security`'s admin-only code): a Django view decorated `@staff_member_required` or
  `@user_passes_test(lambda u: u.is_superuser)`, or one that rejects `not request.user.is_staff`; a `ModelAdmin` or
  `AdminSite` method; a DRF view whose `permission_classes` require `IsAdminUser` (not under `|`); a FastAPI route
  depending (`Depends(...)` or `dependencies=[...]`) on a function named for admins (`get_admin_user`);
- a name its function checks against an allow-list: an `in`/`not in` test, or a call whose name starts with a checking
  verb (`is_allowed_url(url)`), also through a chain of names assigned from it (`parsed_url = urlparse(path); host =
  parsed_url.netloc; if host not in HOSTS: return`); a function so named, whose return carries nothing;
- every library call the library stops at (`int()`, `urlparse()`…); client data only in `data=`, `json=`, `params=`,
  `headers=`; URLs from settings or the database; test files.

Not checks, so still reported: Django's `URLValidator`, `validators.url()`, pydantic `HttpUrl` and
`TypeAdapter(...).validate_python` (they check the format, not the destination: changedetection.io's and langflow's
advisories); a block list (`if url in BLOCKED: raise`); a "check" that fetches the URL itself (`check_url(url)`
running `requests.head`).

**Gaps:** values stored on objects or in the database are not followed; calls the call graph cannot resolve pass
nothing on. Checks are flow-insensitive and trusted even though resolve-then-check helpers are bypassable by redirects
and DNS rebinding (Label Studio's and Open WebUI's advisories). A check in a caller clears the value it passes on, not
a helper that re-reads the request. `BASE + client_text` with `BASE` a parameter, a setting or an import is left alone
(only the file's own constants are read). A guard is trusted for being the run's own adapter or transport class, not
for what it does, and an admin dependency is recognised by its name (what a dependency checks is not decidable in
general).

**Overlap:** Ruff S310 / Bandit B310 (ported in the `python` pack) audit only `urlopen`'s scheme, from any source;
nothing covers requests, httpx, aiohttp or urllib3. Semgrep's Django/Flask `ssrf-requests` rules are single-function
`requests.*` searches.

**Sources:** [OWASP SSRF Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)
(allow-list destinations; redirects and DNS rebinding bypass a one-time check);
[PortSwigger SSRF](https://portswigger.net/web-security/ssrf). Advisories:
[Label Studio GHSA-p59w-9gqw-wj8r](https://github.com/advisories/GHSA-p59w-9gqw-wj8r) (SSRF on import bypassing
`SSRF_PROTECTION_ENABLED`), [Open WebUI GHSA-h6x2-583h-x99r](https://github.com/advisories/GHSA-h6x2-583h-x99r) (DNS
rebinding bypass of a resolve-then-check helper).

**Corpus:** re-measured after the rework, at the commits of `~/rerail-bench/python-arch/corpus/MANIFEST.tsv`, on the
repositories where the review had matches and two more:

| Repository | Commit | Stack | Review | Now | Real |
|---|---|---|--:|--:|--:|
| rachelos/we-mp-rss | `8fe5bf93` | FastAPI | 3 (2 real) | 2 | 2 |
| open-webui/open-webui | `0666e59d` | FastAPI | 4 (0 real) | 0 | |
| sissbruecker/linkding | `27b7303b` | Django | 1 (0 real) | 0 | |
| mealie-recipes/mealie | `8dbb9280` | FastAPI | 1 (0 real) | 1, then fixed | 0 |
| lookit/lookit-api | `78562b50` | Django | 0 | 0 | |

- Real (2): we-mp-rss `apis/tools.py:843`, the `/image/proxy` route with no auth dependency fetching any `url` query
  value with `follow_redirects=True`; and `apis/tools.py:708`, the authenticated image-crop route downloading
  `image_url` from the form.
- Gone, as the review asked: we-mp-rss `apis/res.py:67` (an allow-list two assignments away from the URL); open-webui's
  four admin-only routes (`Depends(get_admin_user)`); linkding's `GuardedHTTPAdapter` session.
- mealie `mealie/pkgs/safehttp/fetch.py:186` was still reported on the run: its `AsyncClient(transport=transport)`
  gets the transport from `_build_transport()`, a function returning `AsyncSafeTransport(**kwargs)`. Transports and
  adapters built by a function of the run are now followed (case `hosts/guarded.py`, `guarded_factory`); mealie was
  not re-run after that change, because local engine runs were stopped.
- Netflix/dispatch `plugins/dispatch_core/plugin.py:188` (the review's f-string bug: a settings `region` before the
  host's end and the client's `kid` after its `/`) is the shape of case `hosts/aggregates.py`, `server_region`, now
  silent; dispatch was not re-run for the same reason.

On the runs, 2 of 3 read matches were real; with the factory fix, every read match is real (2 of 2). The benchmark
below measured the fixes on more repositories, as the review asked.

**On #1120's engine (benchmark).** 2 of 3 earlier and 14 of 22 new matches: 16 of 25 real (64%). The main
false-positive shape (7 of the 9 false) is an admin-only route whose dependency isn't named like `get_admin_user`:
Case211/remnawave-admin's `/finance/accounts/test`, behind `Depends(require_permission("finance", "edit"))`, sending
the admin body's `base_url` to `client.post(f"{base}/auth.php")` (`web/backend/core/finance/adapters/hostkey.py:130`).
It starts off (Log): below 90%; turn it on where every admin-only route depends on a function named for admins
(`get_admin_user`).

### `python-security-orm-lookup-injection`

Rerail's own rule, not a port. It reports a Django ORM call where the client chooses the lookup or field names, not
just the values, in the same function or through the project's helpers in other files. The client data is what
`lib/security` calls a source (Django/DRF `request.GET`, `.POST`, `.data`, `.query_params`, …; Flask and Starlette
request data; FastAPI/Flask route parameters that carry text; Django URL kwargs captured as text, such as
`path("books/<str:sort>/", views.sorted_books)`). Two shapes:

- **Client-chosen keys expanded with `**`** into `filter`, `exclude`, `get`, `get_or_create`, `update_or_create`,
  `annotate`, `alias`, `aggregate`, `values`, `order_by` (any receiver: the method name and a request-keyed splat
  are enough), into `update` on a queryset receiver, or into `django.db.models.Q(**…)`. A mapping is request-keyed
  when it is the request's own mapping or read off it (`request.GET.dict()`, `request.data`,
  `request.query_params.copy()`, `request.data["filters"]`), a dict literal or comprehension with a request-derived
  key (`{f"{field}__icontains": value}`, `{k: v for k, v in request.GET.items()}`), or a name filled by
  `d[key] = …` with such a key or by `d.update(request.GET)`. This is key-level taint: a dict with fixed keys and
  request values (`{"title__icontains": q}`) is not request-keyed.
- **A request string used as a field name**: a positional (or `*`-splatted) argument of `order_by`, `values`,
  `values_list`, `only` or `defer` on a queryset receiver, or the name of `F(name)`, the kind of `Trunc(…, kind)` and
  the lookup name of `Extract(…, lookup_name)`. A free-text value of a serializer's `validated_data` or a form's
  `cleaned_data` counts (`order_by(p.validated_data["ordering"])` with a `CharField`, `values(*fields)` with a
  `ListField(child=CharField())`): only their keys are declared, not their values.

Queryset receivers are resolved, not guessed from names: a chain from `.objects`/`_default_manager`,
`get_queryset()`, `filter_queryset()`, `.queryset`; a related manager of a model of the run (a ForeignKey's or
ManyToManyField's `related_name`, `<model>_set`, a ManyToManyField's own name); a name bound to one; the return of a
function of the run that returns one; a parameter that some call of the run passes one to, whatever its name
(`items`, `objects`, `books`); the queryset parameter of a DRF or django-filter hook in a subclass of the framework's
class.

The message names the source and its file and line, which for a helper is the view that read the request:
`helpers.apply_sort(Book.objects.all(), request.GET["sort"])` in `views.py` and `return items.order_by(sort)` in
`helpers.py` is reported at the `order_by` call, naming `views.py`'s line. For DRF views it names the framework fix,
`OrderingFilter` with `ordering_fields`.

Sanitisers it trusts: a name its function tests with `in`/`not in` or passes to a check call
(`if sort not in ALLOWED_SORTS: …`, `{k: v for k, v in request.GET.items() if k in ALLOWED}`,
`sort = sanitize_order_by(sort, ALLOWED)`), the same expression text checked in the function, a field-name sink
inside an `if`/`elif` that tests the name or one derived from it with `in` or `==`, a name resolved with
`Model._meta.get_field(name)` (it raises for anything but one of the model's own fields, so no `__` path), a dict
lookup (`SORTS[request.GET["sort"]]`, `SORTS.get(…)`), the return of a function named like a check, a typed validated
value (a `ChoiceField`, number, boolean, UUID or date field); and, for a `**` splat only, a form's `cleaned_data` or
a serializer's `validated_data` (their keys are the declared fields), and a check of every key of the same request
mapping in the same function (`for k in request.GET: if k not in ALLOWED: raise`, also over `.keys()`/`.items()`,
`set(request.GET) - ALLOWED`, `set(request.GET) <= ALLOWED`, `.issubset(ALLOWED)`).

It leaves alone:

- keyword lookups with request values, `filter(title__icontains=request.GET["q"])`, and fixed-key dicts with request
  values, `filter(**{"title__icontains": q})`: Django passes values as query parameters;
- `**kwargs` and the keys of `self.kwargs` (the URLconf's capture names), server-built dicts;
- `Book.objects.create(**request.data)`: mass assignment, the drf pack's concern;
- `order_by(x)` on a receiver not resolved to a queryset (a table widget's own `order_by`), and a plain dict's
  `update(**request.GET)`; a field name under `if "<text>" in name:` (only names holding that text pass);
- staff-only code, where the client is staff who already see and sort the admin's data (`lib/security`'s admin-only
  code): a view that rejects `not request.user.is_staff`/`is_superuser`, `@staff_member_required`,
  `@user_passes_test(lambda u: u.is_staff)`, `@permission_required(…)`, DRF `@permission_classes([IsAdminUser])` or
  `permission_classes = [IsAdminUser]` (not `IsAdminUser | IsAuthenticated`), a `method_decorator` of one of these,
  `ModelAdmin`/`AdminSite`/`InlineModelAdmin`/list-filter methods, a class with an admin-only `dispatch` or a
  staff-only `test_func`, `PermissionRequiredMixin`; a call under `if request.user.is_staff:`; and a helper every
  call of which comes from such a function. The review's django-shop admin view is now silent;
- test files.

**Gaps:** values stored on objects or read back from the database, such as a saved view's filters, are not followed;
calls the call graph cannot resolve pass nothing on; a check in a caller clears a helper's parameter only for the
argument it checked; the `in`, check-call and key-loop checks are flow-insensitive (a test anywhere in the function
counts); a queryset reached only through an unresolved call or an attribute that is not a related manager's name
(`self.items`) is not a receiver; admin and staff bases outside the run are a name list (Django's own, `braces`'
staff mixins, and any base from an `admin` module named `…Admin`/`…Inline`/`…ListFilter`); a helper is staff-only
only when the call graph sees every call of it; `QuerySet.extra(order_by=…)` is the sql-injection rule's.

**Overlap:** none in Ruff, Bandit, pylint, flake8-django or the drf pack. Semgrep's
`django/security/injection/mass-assignment` covers only `objects.create(**request.X)` and `update(**request.X)`
followed by `save()`; it has no lookup, ordering or `Q` rule. This rule follows the names across files into the
project's query helpers and knows which keys a dict has.

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

**Corpus:** re-measured after the rework, the rule alone:

| Repository | Commit | Stack | Matches | Real |
|---|---|---|--:|--:|
| lookit/lookit-api | `78562b50` | Django | 2 | 2 |
| andreynovikov/django-shop | `2b8dc600` | Django | 0 | |
| UB-Mannheim/escriptorium | `7e06bd31` | Django, DRF | 0 | |
| inventree/InvenTree | `939df605` | Django, DRF | 0 | |

2 of 2 read matches are real. lookit `exp/views/user.py:54`: the researcher-facing participant list runs
`qs.order_by(self.get_ordering())` with `get_ordering()` returning `request.GET["sort"]`, so `?sort=password` orders
participants by password hash (the DRF docs' exact warning). lookit `studies/queries.py:585`: `_apply_sort(queryset,
sort)` with `sort` from the study list's `request.GET` in `exp/views/study.py:290`, the queryset resolved through the
`_apply_*` helper chain. The review's third match, django-shop `shop/admin/product.py:534` (a `ModelAdmin` view that
raises `PermissionDenied` unless `request.user.is_staff`), is now silent. pretix/pretix gave 0 on an earlier state of
this rework; the final rule was not re-run there, because local engine runs were stopped.

**On #1120's engine (benchmark).** 2 of 2 earlier and 0 of 19 gained: 2 of 21 real (10%). All 19 are on plane:
`.order_by(self.kwargs.get("order_by", "-created_at"))` counted as client data, though no route of plane captures
`order_by`, so the value is always the default (`apps/api/plane/api/views/cycle.py:93`). It starts off (Log): below
90%; turn it on where views read only the `self.kwargs` keys their routes capture.
